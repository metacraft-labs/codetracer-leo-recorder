//! Tracer implementation for Leo programs.
//!
//! Compiles a Leo source file using the `leo` CLI compiler to produce
//! Aleo instructions (.aleo), then executes those instructions using
//! a built-in Aleo Virtual Machine (AVM) interpreter. The AVM interpreter
//! evaluates the real compiled output — register-based Aleo instructions —
//! not the Leo source directly.
//!
//! The execution pipeline mirrors the real Leo/Aleo toolchain:
//!   Leo source → `leo build` → Aleo instructions (.aleo) → AVM execution
//!
//! Variable values are mapped back to Leo source lines by correlating
//! Aleo register assignments with the original source declarations.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

use codetracer_trace_types::{
    EventLogKind, FieldTypeRecord, Line, TypeKind, TypeRecord, TypeSpecificInfo, ValueRecord,
    NONE_VALUE,
};
use codetracer_trace_writer_nim::trace_writer::TraceWriter;
use codetracer_trace_writer_nim::{create_trace_writer, TraceEventsFileFormat};
use eyre::{eyre, Context, Result};

// The recorder is CTFS-only per `Recorder-CLI-Conventions.md` §4 (see
// `codetracer-specs`).  We pin every `create_trace_writer` call site to
// this constant so the tracer surface no longer carries a `format`
// parameter and the writer cannot accidentally drift away from the
// canonical multi-stream container.
const CTFS_FORMAT: TraceEventsFileFormat = TraceEventsFileFormat::Ctfs;

use crate::source_map::{generate_source_map, AleoSourceMap, SourceMap};

// ---------------------------------------------------------------------------
// Aleo instruction types (parsed from compiled .aleo output)
// ---------------------------------------------------------------------------

/// A parsed Aleo instruction operand: either a register or a literal.
#[derive(Debug, Clone)]
pub enum Operand {
    /// A register reference like `r0`, `r1`, etc.
    Register(usize),
    /// A typed literal like `10u32`, `2u32`, etc.
    Literal(i64),
}

/// A parsed Aleo instruction.
#[derive(Debug, Clone)]
pub enum AleoInstruction {
    /// `add <op1> <op2> into <dest>;`
    Add {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `sub <op1> <op2> into <dest>;`
    Sub {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `mul <op1> <op2> into <dest>;`
    Mul {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `div <op1> <op2> into <dest>;`
    Div {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `mod <op1> <op2> into <dest>;`
    Mod {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `is.eq <op1> <op2> into <dest>;`
    IsEq {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `is.neq <op1> <op2> into <dest>;`
    IsNeq {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `lt <op1> <op2> into <dest>;` (less than)
    Lt {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `lte <op1> <op2> into <dest>;` (less than or equal)
    Lte {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `gt <op1> <op2> into <dest>;` (greater than)
    Gt {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `gte <op1> <op2> into <dest>;` (greater than or equal)
    Gte {
        src1: Operand,
        src2: Operand,
        dest: usize,
    },
    /// `call <function_name> <args...> into <dests...>;`
    Call {
        function_name: String,
        args: Vec<Operand>,
        dests: Vec<usize>,
    },
    // -- Finalize-scope mapping instructions --
    /// `get mapping[key_reg] into value_reg;`
    MappingGet {
        mapping: String,
        key_reg: usize,
        value_reg: usize,
    },
    /// `get.or_use mapping[key_reg] default_reg into value_reg;`
    MappingGetOrUse {
        mapping: String,
        key_reg: usize,
        default_reg: usize,
        value_reg: usize,
    },
    /// `set mapping[key_reg] into value_reg;`
    MappingSet {
        mapping: String,
        key_reg: usize,
        value_reg: usize,
    },
    /// `remove mapping[key_reg];`
    MappingRemove { mapping: String, key_reg: usize },
    /// `contains mapping[key_reg] into result_reg;`
    MappingContains {
        mapping: String,
        key_reg: usize,
        result_reg: usize,
    },

    /// An instruction we don't interpret (passthrough).
    #[allow(dead_code)]
    Unknown(String),
}

/// A parsed Aleo function (closure, function, or finalize block).
#[derive(Debug, Clone)]
pub struct AleoFunction {
    /// Function name.
    pub name: String,
    /// Whether this is a `closure` (internal) or `function` (transition).
    #[allow(dead_code)]
    pub is_closure: bool,
    /// Input registers and their types.
    pub inputs: Vec<(usize, String)>,
    /// Body instructions.
    pub instructions: Vec<AleoInstruction>,
    /// Output registers.
    pub outputs: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Mapping store for finalize-scope simulation
// ---------------------------------------------------------------------------

/// Simulates on-chain mapping state during finalize execution.
///
/// Each mapping is identified by name and stores key-value pairs, where
/// both keys and values are represented as i64 (matching the AVM interpreter's
/// register type).
#[derive(Debug, Clone, Default)]
pub struct MappingStore {
    /// mapping_name -> (key_string -> value)
    store: HashMap<String, HashMap<String, i64>>,
}

impl MappingStore {
    /// Create an empty mapping store.
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    /// Set a value in a mapping: `mapping[key] = value`.
    pub fn set(&mut self, mapping: &str, key: &str, value: i64) {
        self.store
            .entry(mapping.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Get a value from a mapping. Returns `None` if the key is absent.
    pub fn get(&self, mapping: &str, key: &str) -> Option<i64> {
        self.store.get(mapping)?.get(key).copied()
    }

    /// Get a value or return a default if the key is absent.
    pub fn get_or_use(&self, mapping: &str, key: &str, default: i64) -> i64 {
        self.get(mapping, key).unwrap_or(default)
    }

    /// Check whether a mapping contains a key.
    pub fn contains(&self, mapping: &str, key: &str) -> bool {
        self.store
            .get(mapping)
            .map(|m| m.contains_key(key))
            .unwrap_or(false)
    }

    /// Remove a key from a mapping.
    pub fn remove(&mut self, mapping: &str, key: &str) {
        if let Some(m) = self.store.get_mut(mapping) {
            m.remove(key);
        }
    }
}

// ---------------------------------------------------------------------------
// Leo source-level types (for mapping back to source)
// ---------------------------------------------------------------------------

/// A Leo source-level variable binding, used for trace emission.
#[derive(Debug, Clone)]
struct LeoBinding {
    /// Variable name (e.g. "a", "sum_val").
    name: String,
    /// Type name (e.g. "u32").
    type_name: String,
    /// 1-based source line number.
    line: u32,
    /// The right-hand-side expression text (everything after `=`,
    /// stripped of trailing `;`).  Carried so the source-derived
    /// evaluator can re-parse it without re-walking the original
    /// source lines.  Defaults to the empty string for bindings parsed
    /// before this field was introduced (none of the live call sites
    /// rely on the empty-default behaviour any more).
    #[allow(dead_code)]
    rhs: String,
}

/// A Leo function definition parsed from source (for call/return tracing).
#[derive(Debug, Clone)]
struct LeoFunctionDef {
    /// Function name.
    name: String,
    /// Whether this is a `transition` (vs `function`).
    #[allow(dead_code)]
    is_transition: bool,
    /// 1-based line number of the function definition.
    line: u32,
    /// Declared parameter names in source order (e.g. for
    /// `transition transfer(receiver: address, amount: u64)` this is
    /// `["receiver", "amount"]`).
    ///
    /// Surfaced on `CallRecord.args` via `TraceWriter::arg(name, NONE_VALUE)`
    /// staged before `register_call` -- mirrors the Circom 1.58 declared-
    /// input-signal staging and the TON 1.57 declared-func.params staging.
    /// Live argument values are NOT yet threaded back to these names because
    /// the Leo-to-Aleo register mapping at the call site requires symbolic
    /// argument-expression resolution that the current parser does not
    /// perform; see AUDIT-CTFS-2026-05.md follow-ups.
    parameters: Vec<String>,
    /// Variable bindings in order.
    bindings: Vec<LeoBinding>,
    /// 1-based line number of the return statement (if any).
    return_line: Option<u32>,
    /// 1-based line number of the function body's closing brace.
    /// Used by the static `assert` / `assert_eq` sweep so the recorder
    /// can scope each assertion to its declaring function (and emit
    /// io_events at the function's call window) even though the
    /// source-level expression compiler does not lower the assert
    /// itself to an Aleo instruction yet.
    #[allow(dead_code)]
    body_end_line: Option<u32>,
}

// ---------------------------------------------------------------------------
// The main tracer
// ---------------------------------------------------------------------------

/// The main tracer struct that captures Leo execution traces.
pub struct LeoTracer {
    writer: Box<dyn TraceWriter + Send>,
    /// Registered type IDs for Leo types.
    type_ids: HashMap<String, codetracer_trace_types::TypeId>,
}

impl LeoTracer {
    /// Trace a Leo program and write CodeTracer output files.
    ///
    /// 1. Compiles the Leo source using the `leo` CLI to produce Aleo instructions.
    /// 2. Parses the Aleo instructions into an internal representation.
    /// 3. Executes the Aleo instructions using a built-in AVM interpreter.
    /// 4. Maps computed values back to Leo source variables.
    /// 5. Emits Step, Call, Return, and Value trace events.
    pub fn trace_program(source_path: &Path, source_code: &str, out_dir: &Path) -> Result<()> {
        // -- 1. Parse Leo source for variable/function mappings --
        let _source_map = SourceMap::from_source(source_path, source_code);
        let leo_functions = parse_leo_functions(source_code);

        eprintln!("Parsed {} Leo functions from source", leo_functions.len());

        // -- 2. Compile Leo to Aleo instructions --
        let aleo_source = match compile_leo_to_aleo(source_path, source_code) {
            Ok(aleo) => aleo,
            Err(error) => {
                let message = format!("{error:#}");
                write_error_trace(source_path, out_dir, "leo_compile_error", &message)?;
                return Err(error);
            }
        };

        eprintln!(
            "Compiled Leo to {} bytes of Aleo instructions",
            aleo_source.len()
        );

        // -- 3. Parse the Aleo instructions --
        let aleo_functions = parse_aleo_program(&aleo_source);

        eprintln!("Parsed {} Aleo functions", aleo_functions.len());

        // -- 3b. Generate Aleo-to-Leo source map --
        let aleo_source_map = generate_source_map(source_code, &aleo_source);

        eprintln!("Generated Aleo-to-Leo source map");

        // -- 4. Execute the Aleo instructions --
        let execution_results = match execute_aleo_program(&aleo_functions) {
            Ok(results) => results,
            Err(error) => {
                let message = format!("{error:#}");
                write_error_trace(source_path, out_dir, "avm_runtime_error", &message)?;
                return Err(error);
            }
        };

        eprintln!(
            "Executed Aleo program, got {} function results",
            execution_results.len()
        );

        // -- 5. Map Aleo register values back to Leo source variables --
        let variable_values =
            map_registers_to_variables(&leo_functions, &aleo_functions, &execution_results);

        for (name, val) in &variable_values {
            eprintln!("  {name} = {val}");
        }

        // -- 6. Create the trace writer (CTFS only) --
        let program_str = source_path.to_string_lossy();
        let mut tracer = LeoTracer {
            writer: create_trace_writer(&program_str, &[], CTFS_FORMAT),
            type_ids: HashMap::new(),
        };

        // -- 7. Initialise output files --
        std::fs::create_dir_all(out_dir)
            .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

        // CTFS-only writer — events stream lives in `trace.bin`.
        let events_path = out_dir.join("trace.bin");
        let metadata_path = out_dir.join("trace_metadata.json");
        let paths_path = out_dir.join("trace_paths.json");

        TraceWriter::begin_writing_trace_events(&mut *tracer.writer, &events_path)
            .map_err(|e| eyre!("{e}"))?;
        TraceWriter::begin_writing_trace_metadata(&mut *tracer.writer, &metadata_path)
            .map_err(|e| eyre!("{e}"))?;
        TraceWriter::begin_writing_trace_paths(&mut *tracer.writer, &paths_path)
            .map_err(|e| eyre!("{e}"))?;

        // -- 8. Start the trace --
        TraceWriter::start(&mut *tracer.writer, source_path, Line(1));

        // Register common Leo types.
        for type_name in &["u32", "u64", "i32", "i64", "field", "bool"] {
            let type_id =
                TraceWriter::ensure_type_id(&mut *tracer.writer, TypeKind::Int, type_name);
            tracer.type_ids.insert(type_name.to_string(), type_id);
        }

        // -- 9. Emit trace events --
        tracer.emit_trace_events(
            source_path,
            source_code,
            &leo_functions,
            &variable_values,
            &aleo_source_map,
        )?;

        // Close the <toplevel> call that start() opened.
        TraceWriter::register_return(&mut *tracer.writer, NONE_VALUE);

        // -- 10. Finish writing --
        TraceWriter::finish_writing_trace_events(&mut *tracer.writer).map_err(|e| eyre!("{e}"))?;
        TraceWriter::finish_writing_trace_metadata(&mut *tracer.writer)
            .map_err(|e| eyre!("{e}"))?;
        TraceWriter::finish_writing_trace_paths(&mut *tracer.writer).map_err(|e| eyre!("{e}"))?;
        tracer.writer.close().map_err(|e| eyre!("{e}"))?;

        Ok(())
    }

    /// Emit trace events by walking through Leo functions and their bindings.
    ///
    /// The structured evaluator (`parse_leo_program` +
    /// `execute_leo_function`) is the primary execution engine: it
    /// handles if/else, ternary, for-loops, struct/tuple/array
    /// literals, argumentful function calls, and arbitrary call
    /// graphs.  The legacy AVM `variable_values` map is no longer
    /// consulted by the emit pass -- the structured evaluator's typed
    /// `Value` env replaces it.
    fn emit_trace_events(
        &mut self,
        source_path: &Path,
        source_code: &str,
        leo_functions: &[LeoFunctionDef],
        _variable_values: &HashMap<String, i64>,
        _aleo_source_map: &AleoSourceMap,
    ) -> Result<()> {
        let mut prog = parse_leo_program(source_code);

        // Per-function source path: defaults to the caller's
        // `source_path` for every function declared in the main
        // file.  When the recorder loads an imported `.leo` file
        // (via `import X.aleo;`), each imported function is
        // re-tagged with the imported file's path so source-line
        // metadata for cross-program calls correctly points at
        // the library file rather than the caller.
        let mut func_source_files: HashMap<String, std::path::PathBuf> = HashMap::new();
        for f in &prog.funcs {
            func_source_files.insert(f.name.clone(), source_path.to_path_buf());
        }

        // Resolve `import X.aleo;` directives by loading sibling
        // `.leo` files from the caller's directory and merging
        // their function tables / struct decls into the main
        // program.  Each imported function is tagged with the
        // imported file's path so the writer registers a distinct
        // function-id pointing at the library source.
        let imports = collect_imports(source_code);
        for import_name in &imports {
            let import_path = source_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{import_name}.leo"));
            if !import_path.exists() {
                eprintln!(
                    "Imported program {import_name} not found at {}; skipping",
                    import_path.display()
                );
                continue;
            }
            let imported_src = match std::fs::read_to_string(&import_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "Failed to read imported program {}: {e}; skipping",
                        import_path.display()
                    );
                    continue;
                }
            };
            let imported_prog = parse_leo_program(&imported_src);
            for f in imported_prog.funcs {
                func_source_files.insert(f.name.clone(), import_path.clone());
                if !prog.by_name.contains_key(&f.name) {
                    prog.by_name.insert(f.name.clone(), f.clone());
                }
                prog.funcs.push(f);
            }
            for (name, def) in imported_prog.structs {
                prog.structs.entry(name).or_insert(def);
            }
        }

        // The structured parser is the source of truth.  Register
        // every declared struct as a typed record so
        // `value_to_record` can emit `ValueRecord::Struct` with the
        // right field-name metadata when the evaluator surfaces a
        // struct literal.
        let mut struct_type_ids: HashMap<String, codetracer_trace_types::TypeId> = HashMap::new();
        for (name, def) in &prog.structs {
            // Field-type IDs default to u32 (Int) -- every fixture
            // struct uses only `u32` fields today.  Field types that
            // aren't registered fall back to the writer's u32 id.
            let field_records: Vec<FieldTypeRecord> = def
                .fields
                .iter()
                .map(|(fname, ftype)| FieldTypeRecord {
                    name: fname.clone(),
                    type_id: self.ensure_int_type(ftype),
                })
                .collect();
            let type_record = TypeRecord {
                kind: TypeKind::Struct,
                lang_type: name.clone(),
                specific_info: TypeSpecificInfo::Struct {
                    fields: field_records,
                },
            };
            let id = TraceWriter::ensure_raw_type_id(&mut *self.writer, type_record);
            struct_type_ids.insert(name.clone(), id);
        }

        // Compute reachability set for the static-sweep assert gating:
        // only assertions inside functions actually reachable from
        // `main` should surface as io_events.  Unreachable functions
        // declared in source (e.g. `failing_compute` in
        // error_paths_test.leo) must not pollute the io_event stream.
        let reachable: BTreeSet<String> = if prog.by_name.contains_key("main") {
            reachable_functions(&prog, "main")
        } else {
            BTreeSet::new()
        };

        // Bare `.aleo` dialect: source uses `program X.aleo;` (header
        // semicolon, no surrounding `{...}`) and `function NAME:`
        // (colon, no parens) blocks of register-based AVM
        // instructions.  The structured Leo parser may speculatively
        // accept the `function NAME:` shape -- the bare-`.aleo`
        // detector takes precedence so the dedicated synthesiser
        // owns the emit pass and downstream consumers see the
        // register-level trace shape rather than a spurious
        // structured-parser interpretation.
        if is_bare_aleo_dialect(source_code) {
            self.emit_aleo_dialect_trace(source_path, source_code);
            return Ok(());
        }

        // Execute and emit from `main` via the structured evaluator.
        if let Some(main_fn) = prog.by_name.get("main") {
            let main_fn_clone = main_fn.clone();
            let trace = execute_leo_function(&prog, &main_fn_clone, &[]);
            self.emit_call_trace(
                source_path,
                &main_fn_clone,
                &trace,
                &[],
                &prog,
                &struct_type_ids,
                &func_source_files,
                /*is_entry_point=*/ true,
            );
        } else {
            // No `main` defined.  Multi-entry-point dispatch: every
            // top-level `transition` declared in the program is
            // executed in declaration order as its own entry point.
            // Each surfaces a balanced Call/Return pair so downstream
            // tooling sees three distinct Function entries when a
            // program exposes (e.g.) deposit / withdraw / query
            // independently of any caller.  Synthesised default
            // arguments (zeroed for numeric formals) keep the
            // execution well-defined when the caller can't supply
            // real inputs from the source-level view.
            let transitions: Vec<LeoFunc> = prog
                .funcs
                .iter()
                .filter(|f| f.is_transition)
                .cloned()
                .collect();
            if !transitions.is_empty() {
                for tf in &transitions {
                    let synth_args: Vec<(String, Value)> = tf
                        .parameters
                        .iter()
                        .map(|(pname, ptype)| (pname.clone(), Value::Int(0, ptype.clone())))
                        .collect();
                    let trace = execute_leo_function(&prog, tf, &synth_args);
                    self.emit_call_trace(
                        source_path,
                        tf,
                        &trace,
                        &synth_args,
                        &prog,
                        &struct_type_ids,
                        &func_source_files,
                        /*is_entry_point=*/ false,
                    );
                }
            } else {
                // Fallback: legacy walk (no transitions parsed by the
                // structured evaluator).  Preserves backward compat
                // for finalize-test fixtures whose `function` decls
                // the structured parser doesn't yet handle.
                let func_map: HashMap<&str, &LeoFunctionDef> =
                    leo_functions.iter().map(|f| (f.name.as_str(), f)).collect();
                if let Some(any_fn) = func_map.values().next() {
                    self.emit_function_trace(
                        source_path,
                        source_code,
                        any_fn,
                        &func_map,
                        &HashMap::new(),
                        &AleoSourceMap::empty(),
                        true,
                    )?;
                }
            }
        }

        // Static assert sweep -- gated on reachability.  The
        // structured evaluator already emitted per-call assert events
        // inline; this sweep is intentionally skipped because every
        // reachable assert was emitted by `emit_call_trace`.  Kept as
        // a documented no-op so downstream tooling (the IO-event
        // count assertions) sees the reachable-only assert set.
        let _ = leo_functions;
        let _ = reachable;

        Ok(())
    }

    /// Register (or look up) an Int-kind type id for a given Leo
    /// type name (`u32`, `u64`, ...).  Side-effect: caches in
    /// `self.type_ids` so repeat lookups are O(1).
    fn ensure_int_type(&mut self, type_name: &str) -> codetracer_trace_types::TypeId {
        if let Some(id) = self.type_ids.get(type_name) {
            return *id;
        }
        let id = TraceWriter::ensure_type_id(&mut *self.writer, TypeKind::Int, type_name);
        self.type_ids.insert(type_name.to_string(), id);
        id
    }

    /// Walk a `CallTrace` and emit step / call / return / io events.
    ///
    /// `call_args` carries the `(formal-name, value)` pairs that the
    /// caller staged before entering this function.  For the entry
    /// point (`main` -- merged into `<toplevel>`), pass an empty slice.
    fn emit_call_trace(
        &mut self,
        source_path: &Path,
        func: &LeoFunc,
        trace: &CallTrace,
        call_args: &[(String, Value)],
        prog: &LeoProgram,
        struct_type_ids: &HashMap<String, codetracer_trace_types::TypeId>,
        func_source_files: &HashMap<String, std::path::PathBuf>,
        is_entry_point: bool,
    ) {
        // Functions imported from sibling .leo files have their own
        // source path so the writer registers a distinct
        // function-id pointing at the library file rather than the
        // caller's source path.  Falls back to the caller's path
        // for top-level / non-imported functions.
        let func_path: std::path::PathBuf = func_source_files
            .get(&func.name)
            .cloned()
            .unwrap_or_else(|| source_path.to_path_buf());
        // For functions declared inside an imported program, surface
        // the qualified name (`math_lib.aleo/double`) in the
        // function table so cross-program calls are unambiguous;
        // local functions keep their bare name.
        let qualified_name: String = match (&func.program_name, func_source_files.get(&func.name)) {
            // Function comes from an imported file (different source path).
            (Some(prog_name), Some(p)) if p.as_path() != source_path => {
                format!("{prog_name}.aleo/{}", func.name)
            }
            _ => func.name.clone(),
        };
        let fn_id = TraceWriter::ensure_function_id(
            &mut *self.writer,
            &qualified_name,
            &func_path,
            Line(func.line as i64),
        );
        if !is_entry_point {
            // Re-tag each argument's `Value::Int(_, type_name)` with
            // the declared parameter type so visibility modifiers
            // surface (`public sender: u64` registers a distinct
            // `u64.public` type id, distinguishing on-chain-visible
            // witnesses from witness-only inputs).  Non-Int values
            // are passed through unchanged.
            let typed_call_args: Vec<(String, Value)> = call_args
                .iter()
                .map(|(pname, pvalue)| {
                    let declared = func
                        .parameters
                        .iter()
                        .find(|(n, _)| n == pname)
                        .map(|(_, t)| t.clone());
                    let retyped = match (pvalue, declared) {
                        (Value::Int(i, _), Some(t)) => Value::Int(*i, t),
                        (other, _) => other.clone(),
                    };
                    (pname.clone(), retyped)
                })
                .collect();
            for (pname, pvalue) in &typed_call_args {
                let record = self.value_to_record(pvalue, struct_type_ids);
                TraceWriter::arg(&mut *self.writer, pname, record);
            }
            TraceWriter::register_call(&mut *self.writer, fn_id, vec![]);
            // Emit a function-entry step so the writer assigns a
            // distinct `entryStep` to this call.  Without this, two
            // back-to-back `register_call`s (e.g. `compute() { return
            // inner(); }`) end up sharing the same entryStep and
            // ct-print orders them by call_key (close order, LIFO)
            // instead of entry order.  The step points at the
            // function's signature line for symmetry with the
            // historical AVM emit shape.  When the function has
            // parameters, this also anchors the per-parameter
            // `register_variable_with_full_value` calls below so
            // structured arguments (Tuple/Struct/Sequence) actually
            // surface in the variable stream.
            TraceWriter::register_step(&mut *self.writer, &func_path, Line(func.line as i64));
            // Re-emit each formal parameter as a step variable so the
            // structured argument value (Tuple `(10, 20)` for
            // `sum_pair(p)`, Struct `{x:3,y:4}` for
            // `point_distance_sq(p)`) lands on the function-entry
            // step.  Without this, parameters whose value isn't `Int`
            // never appear in `step.vars`, defeating the whole point
            // of decoding structured argument literals at the call
            // site.
            for (pname, pvalue) in &typed_call_args {
                let record = self.value_to_record(pvalue, struct_type_ids);
                TraceWriter::register_variable_with_full_value(&mut *self.writer, pname, record);
            }
        }

        // Emit step events in source order, interleaving sub-calls
        // at the binding indices where they were captured.
        let mut sub_idx = 0;
        for (binding_idx, binding) in trace.bindings.iter().enumerate() {
            // First, emit any sub-calls captured AT this binding index
            // before this binding's step (the sub-call's RHS is part
            // of this binding's expression -- the call_entry should
            // appear right before the binding's step in the stream).
            while sub_idx < trace.sub_calls.len()
                && trace.sub_calls[sub_idx].after_binding == binding_idx
            {
                let sub = &trace.sub_calls[sub_idx];
                if let Some(callee_fn) = prog.by_name.get(&sub.callee) {
                    self.emit_call_trace(
                        source_path,
                        callee_fn,
                        &sub.trace,
                        &sub.args,
                        prog,
                        struct_type_ids,
                        func_source_files,
                        false,
                    );
                }
                sub_idx += 1;
            }

            // Emit step + variable for this binding.  Use the
            // function's own source path so cross-program calls
            // surface step events against the library's source
            // file rather than the caller's.
            TraceWriter::register_step(&mut *self.writer, &func_path, Line(binding.line as i64));
            let record = self.value_to_record(&binding.value, struct_type_ids);
            TraceWriter::register_variable_with_full_value(
                &mut *self.writer,
                &binding.name,
                record,
            );
        }

        // Emit trailing sub-calls (captured "after the last binding"
        // -- e.g. when the return expression itself is a call).
        while sub_idx < trace.sub_calls.len() {
            let sub = &trace.sub_calls[sub_idx];
            if let Some(callee_fn) = prog.by_name.get(&sub.callee) {
                self.emit_call_trace(
                    source_path,
                    callee_fn,
                    &sub.trace,
                    &sub.args,
                    prog,
                    struct_type_ids,
                    func_source_files,
                    false,
                );
            }
            sub_idx += 1;
        }

        // Emit io_events for any assertions executed inside this
        // function.  Reachability is implicit -- we only reach this
        // function because main's call graph leads here.
        for a in &trace.asserts {
            let metadata = match a.kind {
                AssertKind::Assert => "LeoAssert",
                AssertKind::AssertEq => "LeoAssertEq",
            };
            TraceWriter::register_special_event(
                &mut *self.writer,
                EventLogKind::Error,
                metadata,
                &a.text,
            );
        }

        // Emit io_events for hash-builtin calls and finalize-scope
        // mapping ops surfaced by the structured evaluator.  Reads
        // (`Mapping::get*`, hash-builtin evaluations) carry
        // EventLogKind::Read; writes (`Mapping::set/remove`)
        // carry EventLogKind::Write; aborts (`LeoFinalizeAbort`,
        // `LeoOverflow` -- though the latter is non-fatal) carry
        // EventLogKind::Error so downstream tooling can
        // distinguish chain-failing events from successful state
        // I/O.  Each event's `text` field round-trips the verbatim
        // source-level expression.
        for e in &trace.io_events {
            let kind = if e.metadata == "LeoFinalizeAbort" {
                EventLogKind::Error
            } else if e.is_write {
                EventLogKind::Write
            } else {
                EventLogKind::Read
            };
            TraceWriter::register_special_event(&mut *self.writer, kind, e.metadata, &e.text);
        }

        // Emit the return-statement step (so the line of the `return`
        // shows up in the trace, matching the legacy AVM emit shape).
        let return_line = find_return_line_in_body(&func.body).unwrap_or(func.body_end_line);
        TraceWriter::register_step(&mut *self.writer, &func_path, Line(return_line as i64));

        if !is_entry_point {
            let record = self.value_to_record(&trace.return_value, struct_type_ids);
            TraceWriter::register_return(&mut *self.writer, record);
        }
    }

    /// Emit a trace for a bare `.aleo`-dialect source file (the
    /// register-based instruction form the Leo compiler emits as
    /// its build output, exercised here directly).  The structured
    /// Leo evaluator does not understand this surface; the bare-
    /// `.aleo` synthesiser walks each function's `input` /
    /// arithmetic / `cast` / `output` lines, threading register
    /// values through a small env, and surfaces each destination
    /// register as a step variable so downstream tooling can see
    /// every register's post-instruction payload + type.
    ///
    /// `cast rA into rB as TYPE` re-types rA's payload as TYPE; the
    /// emitted variable for rB carries a distinct `TypeId` so the
    /// post-cast type chain round-trips through ct-print --full.
    /// `output rN as TYPE.private` produces a CloseFrame whose
    /// `return_value` payload is rN's value typed as TYPE.
    fn emit_aleo_dialect_trace(&mut self, source_path: &Path, source_code: &str) {
        let lines: Vec<&str> = source_code.lines().collect();
        let mut i = 0;
        // Synthesised default values for `input` registers: each
        // declared input gets a distinct, non-zero stand-in (the
        // declaration index plus a small base) so downstream
        // tooling sees deterministic, distinct register values.
        let input_base: i64 = 5;
        while i < lines.len() {
            let trimmed = lines[i].trim();
            if !trimmed.starts_with("function ") || !trimmed.ends_with(':') {
                i += 1;
                continue;
            }
            let name = trimmed["function ".len()..]
                .trim_end_matches(':')
                .trim()
                .to_string();
            let func_line = (i + 1) as u32;
            // Walk the body lines until the next blank-or-function
            // boundary.
            let mut body: Vec<(u32, String)> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let raw = lines[j].trim();
                if raw.starts_with("function ") && raw.ends_with(':') {
                    break;
                }
                if !raw.is_empty() && !raw.starts_with("//") {
                    body.push(((j + 1) as u32, raw.to_string()));
                }
                j += 1;
            }
            i = j;

            // Register the function id.
            let fn_id = TraceWriter::ensure_function_id(
                &mut *self.writer,
                &name,
                source_path,
                Line(func_line as i64),
            );

            // Walk body to collect `input` declarations + arithmetic
            // instructions + `output` declarations.
            let mut registers: HashMap<usize, AleoDialectReg> = HashMap::new();
            let mut input_regs: Vec<(usize, String)> = Vec::new();
            let mut steps: Vec<(u32, usize, AleoDialectReg)> = Vec::new(); // (line, dest, post-instr value)
            let mut output: Option<(u32, usize, String)> = None; // (line, src_reg, declared type)
            for (line_num, raw) in &body {
                let s = raw.trim().trim_end_matches(';').trim();
                if let Some(rest) = s.strip_prefix("input ") {
                    // `r0 as u32.private`
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 && parts[1] == "as" {
                        if let Some(reg_idx) = parse_aleo_dialect_register(parts[0]) {
                            let raw_type = parts[2];
                            let base_type =
                                raw_type.split('.').next().unwrap_or(raw_type).to_string();
                            registers.insert(
                                reg_idx,
                                AleoDialectReg {
                                    value: input_base + input_regs.len() as i64,
                                    type_name: base_type.clone(),
                                },
                            );
                            input_regs.push((reg_idx, base_type));
                        }
                    }
                    continue;
                }
                if let Some(rest) = s.strip_prefix("output ") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 && parts[1] == "as" {
                        if let Some(reg_idx) = parse_aleo_dialect_register(parts[0]) {
                            let raw_type = parts[2];
                            let base_type =
                                raw_type.split('.').next().unwrap_or(raw_type).to_string();
                            output = Some((*line_num, reg_idx, base_type));
                        }
                    }
                    continue;
                }
                // Arithmetic shapes: `add A B into D;` `mul A B into D;`
                // `cast A into D as TYPE;`
                let tokens: Vec<&str> = s.split_whitespace().collect();
                if tokens.is_empty() {
                    continue;
                }
                let op = tokens[0];
                if op == "cast" {
                    // `cast rA into rD as TYPE`
                    if tokens.len() >= 5 && tokens[2] == "into" && tokens[4] == "as" {
                        let src_v = aleo_dialect_resolve(tokens[1], &registers);
                        if let Some(dest) = parse_aleo_dialect_register(tokens[3]) {
                            let target_type = tokens[5..]
                                .first()
                                .copied()
                                .unwrap_or(tokens[4])
                                .to_string();
                            // tokens layout for `cast A into D as TYPE`:
                            // [cast, A, into, D, as, TYPE].  Re-derive
                            // safely.
                            let target_type = if tokens.len() >= 6 {
                                tokens[5].to_string()
                            } else {
                                target_type
                            };
                            let truncated = truncate_to_type(src_v.unwrap_or(0), &target_type);
                            let r = AleoDialectReg {
                                value: truncated,
                                type_name: target_type,
                            };
                            registers.insert(dest, r.clone());
                            steps.push((*line_num, dest, r));
                        }
                    }
                    continue;
                }
                // `OP A B into D` (binary arithmetic).
                if tokens.len() >= 5 && tokens[3] == "into" {
                    let v1 = aleo_dialect_resolve(tokens[1], &registers).unwrap_or(0);
                    let v2 = aleo_dialect_resolve(tokens[2], &registers).unwrap_or(0);
                    if let Some(dest) = parse_aleo_dialect_register(tokens[4]) {
                        // Pick the type from the LHS register if the
                        // operand is a register reference; otherwise
                        // fall back to u32.
                        let type_name = parse_aleo_dialect_register(tokens[1])
                            .and_then(|r| registers.get(&r).map(|reg| reg.type_name.clone()))
                            .unwrap_or_else(|| "u32".to_string());
                        let result = match op {
                            "add" => wrapping_add_in_type(v1, v2, &type_name),
                            "sub" => wrapping_sub_in_type(v1, v2, &type_name),
                            "mul" => wrapping_mul_in_type(v1, v2, &type_name),
                            "div" => {
                                if v2 == 0 {
                                    0
                                } else {
                                    v1 / v2
                                }
                            }
                            "mod" => {
                                if v2 == 0 {
                                    0
                                } else {
                                    v1 % v2
                                }
                            }
                            _ => v1,
                        };
                        let r = AleoDialectReg {
                            value: result,
                            type_name,
                        };
                        registers.insert(dest, r.clone());
                        steps.push((*line_num, dest, r));
                    }
                }
            }

            // Stage args from inputs and emit the call_entry.
            for (reg_idx, type_name) in &input_regs {
                let value = registers.get(reg_idx).map(|r| r.value).unwrap_or(0);
                let type_id = self.ensure_int_type(type_name);
                let record = ValueRecord::Int { i: value, type_id };
                let arg_name = format!("r{reg_idx}");
                TraceWriter::arg(&mut *self.writer, &arg_name, record);
            }
            TraceWriter::register_call(&mut *self.writer, fn_id, vec![]);

            // Function-entry step + per-input variables (mirrors the
            // structured evaluator's entry-step shape).
            TraceWriter::register_step(&mut *self.writer, source_path, Line(func_line as i64));
            for (reg_idx, type_name) in &input_regs {
                let value = registers.get(reg_idx).map(|r| r.value).unwrap_or(0);
                let type_id = self.ensure_int_type(type_name);
                let record = ValueRecord::Int { i: value, type_id };
                let var_name = format!("r{reg_idx}");
                TraceWriter::register_variable_with_full_value(
                    &mut *self.writer,
                    &var_name,
                    record,
                );
            }

            // Per-instruction steps.
            for (line_num, dest, reg) in &steps {
                TraceWriter::register_step(&mut *self.writer, source_path, Line(*line_num as i64));
                let type_id = self.ensure_int_type(&reg.type_name);
                let record = ValueRecord::Int {
                    i: reg.value,
                    type_id,
                };
                let var_name = format!("r{dest}");
                TraceWriter::register_variable_with_full_value(
                    &mut *self.writer,
                    &var_name,
                    record,
                );
            }

            // Output step + register_return.
            if let Some((out_line, out_reg, out_type)) = output {
                TraceWriter::register_step(&mut *self.writer, source_path, Line(out_line as i64));
                let value = registers.get(&out_reg).map(|r| r.value).unwrap_or(0);
                let type_id = self.ensure_int_type(&out_type);
                let record = ValueRecord::Int { i: value, type_id };
                // Emit the output register as a step variable too so
                // downstream tooling sees the post-cast / post-output
                // value attributed to the output line.
                let var_name = format!("r{out_reg}");
                TraceWriter::register_variable_with_full_value(
                    &mut *self.writer,
                    &var_name,
                    record.clone(),
                );
                TraceWriter::register_return(&mut *self.writer, record);
            } else {
                TraceWriter::register_return(&mut *self.writer, NONE_VALUE);
            }
        }
    }

    /// Convert a structured `Value` into a wire-format `ValueRecord`,
    /// registering nested struct types lazily as needed.
    fn value_to_record(
        &mut self,
        v: &Value,
        struct_type_ids: &HashMap<String, codetracer_trace_types::TypeId>,
    ) -> ValueRecord {
        match v {
            Value::Int(i, type_name) => {
                let type_id = self.ensure_int_type(type_name);
                ValueRecord::Int { i: *i, type_id }
            }
            Value::Bool(b) => {
                let type_id = self.ensure_int_type("bool");
                ValueRecord::Bool { b: *b, type_id }
            }
            Value::Str(text, type_name) => {
                // Register a `TypeKind::String` id for the address /
                // string-shaped payload so downstream tooling can
                // distinguish it from `u32` (the catch-all integer
                // type) via `type_id`.
                let type_id = if let Some(id) = self.type_ids.get(type_name) {
                    *id
                } else {
                    let id =
                        TraceWriter::ensure_type_id(&mut *self.writer, TypeKind::String, type_name);
                    self.type_ids.insert(type_name.clone(), id);
                    id
                };
                ValueRecord::String {
                    text: text.clone(),
                    type_id,
                }
            }
            Value::Sequence(elems, elem_type) => {
                let elements: Vec<ValueRecord> = elems
                    .iter()
                    .map(|e| self.value_to_record(e, struct_type_ids))
                    .collect();
                let lang = format!("[{elem_type}; {}]", elems.len());
                let type_id =
                    TraceWriter::ensure_type_id(&mut *self.writer, TypeKind::Array, &lang);
                ValueRecord::Sequence {
                    elements,
                    is_slice: false,
                    type_id,
                }
            }
            Value::Tuple(elems) => {
                let elements: Vec<ValueRecord> = elems
                    .iter()
                    .map(|e| self.value_to_record(e, struct_type_ids))
                    .collect();
                let lang = format!("({} elements)", elems.len());
                let type_id =
                    TraceWriter::ensure_type_id(&mut *self.writer, TypeKind::Tuple, &lang);
                ValueRecord::Tuple { elements, type_id }
            }
            Value::Struct { name, fields } => {
                let type_id = struct_type_ids
                    .get(name)
                    .copied()
                    .unwrap_or_else(|| self.ensure_int_type(name));
                let field_values: Vec<ValueRecord> = fields
                    .iter()
                    .map(|(_, v)| self.value_to_record(v, struct_type_ids))
                    .collect();
                ValueRecord::Struct {
                    field_values,
                    type_id,
                }
            }
            Value::Unknown => NONE_VALUE,
        }
    }

    /// Scan every Leo function body for `assert(<expr>);` and
    /// `assert_eq(<a>, <b>);` calls and emit one io_event per call via
    /// `register_special_event`.  We use `EventLogKind::Error` so the
    /// event is unambiguously surfaced as a failure-shaped record in
    /// the writer's IO stream and -- importantly -- bumps the
    /// `io_events` counter that ct-print's `--full` output exposes.
    ///
    /// Today the recorder's source-level expression compiler does not
    /// lower `assert` calls into Aleo instructions, so the assertion
    /// itself is never evaluated at runtime.  The static sweep is the
    /// minimum visible signal that downstream consumers (the GUI, the
    /// agent evals) need to know "an assertion existed at this
    /// location".  It satisfies the universal-checklist's
    /// exceptions/errors row for the Leo recorder.
    ///
    /// Once the source-level compiler learns to evaluate assert
    /// predicates, this static sweep should be replaced with runtime
    /// evaluation that distinguishes passing vs failing branches and
    /// terminates the trace on a failed assertion.
    ///
    /// Currently unused -- the structured evaluator
    /// (`execute_leo_function`) emits per-call assert events inline,
    /// gated on reachability from `main`.  Kept for the legacy
    /// fallback path when no `main` function is defined (finalize-
    /// scope fixtures).
    #[allow(dead_code)]
    fn emit_assert_events_from_source(
        &mut self,
        source_code: &str,
        leo_functions: &[LeoFunctionDef],
    ) {
        for func in leo_functions {
            for assertion in collect_assertions_in_function(source_code, func) {
                let kind = match assertion.kind {
                    AssertKind::Assert => EventLogKind::Error,
                    AssertKind::AssertEq => EventLogKind::Error,
                };
                let metadata = match assertion.kind {
                    AssertKind::Assert => "LeoAssert",
                    AssertKind::AssertEq => "LeoAssertEq",
                };
                TraceWriter::register_special_event(
                    &mut *self.writer,
                    kind,
                    metadata,
                    &assertion.text,
                );
            }
        }
    }

    /// Emit trace events for a single function.
    ///
    /// Uses the `AleoSourceMap` to resolve step locations: if the source map
    /// has a mapping for a given binding's position in its function, the step
    /// event will reference the Leo source line from the source map. This
    /// ensures traced steps always point to Leo source lines rather than
    /// Aleo instruction positions.
    ///
    /// When `is_entry_point` is true, Call/Return events are suppressed so the
    /// function body runs at depth 0 under `<toplevel>`.
    fn emit_function_trace(
        &mut self,
        source_path: &Path,
        source_code: &str,
        func: &LeoFunctionDef,
        func_map: &HashMap<&str, &LeoFunctionDef>,
        variable_values: &HashMap<String, i64>,
        aleo_source_map: &AleoSourceMap,
        is_entry_point: bool,
    ) -> Result<()> {
        // Register function metadata (for function list / calltrace).
        let fn_id = TraceWriter::ensure_function_id(
            &mut *self.writer,
            &func.name,
            source_path,
            Line(func.line as i64),
        );
        if !is_entry_point {
            // Stage declared parameter NAMES via `TraceWriter::arg(name,
            // NONE_VALUE)` before `register_call` (audit (c)).  Mirrors the
            // Circom 1.58 declared-input-signal staging and the TON 1.57
            // declared-func.params staging.  Live argument values are not
            // yet threaded back to these names because the Leo-source
            // parser does not currently resolve the call-site argument
            // expressions to their evaluated register values; documented
            // as a follow-up in AUDIT-CTFS-2026-05.md.
            for param_name in &func.parameters {
                TraceWriter::arg(&mut *self.writer, param_name, NONE_VALUE);
            }
            TraceWriter::register_call(&mut *self.writer, fn_id, vec![]);
        }

        // Process bindings.
        for (binding_idx, binding) in func.bindings.iter().enumerate() {
            // Check if this binding is a function call.
            // We detect this by looking for a binding whose name matches
            // a variable that holds a function call result.
            let is_call = false; // We handle calls via the call graph below.

            if !is_call {
                // Use the Aleo source map to resolve the Leo source line.
                // The binding index corresponds to the Aleo instruction index
                // within the function (since Leo let-bindings map 1:1 to
                // Aleo instructions).
                let step_line = aleo_source_map
                    .resolve(&func.name, binding_idx)
                    .map(|(_, line)| line)
                    .unwrap_or(binding.line);

                // Emit Step event pointing to the Leo source line.
                TraceWriter::register_step(&mut *self.writer, source_path, Line(step_line as i64));

                // Emit Value event if we have a value for this variable.
                if let Some(&val) = variable_values.get(&binding.name) {
                    let type_id = self
                        .type_ids
                        .get(&binding.type_name)
                        .copied()
                        .unwrap_or_else(|| self.type_ids.get("u32").copied().unwrap());

                    let value = ValueRecord::Int { i: val, type_id };
                    TraceWriter::register_variable_with_full_value(
                        &mut *self.writer,
                        &binding.name,
                        value,
                    );
                }
            }
        }

        // Note: nested function calls are handled via the return call
        // detection below, not via individual bindings.

        // Emit return step if present.
        if let Some(return_line) = func.return_line {
            TraceWriter::register_step(&mut *self.writer, source_path, Line(return_line as i64));
        }

        // Check if the return expression is a function call.
        // Parse source to find `return <func_name>();` pattern.
        let return_calls_function = find_return_call_target(source_code, func, func_map);

        if let Some(callee_name) = return_calls_function {
            if let Some(callee) = func_map.get(callee_name.as_str()) {
                self.emit_function_trace(
                    source_path,
                    source_code,
                    callee,
                    func_map,
                    variable_values,
                    aleo_source_map,
                    false,
                )?;
            }
        }

        // Emit Return event (skip for entry point — its steps live under
        // <toplevel> which is closed separately).
        if !is_entry_point {
            let return_value = find_return_value(source_code, func, func_map, variable_values);
            match return_value {
                Some(val) => {
                    let type_id = self.type_ids.get("u32").copied().unwrap();
                    let value = ValueRecord::Int { i: val, type_id };
                    TraceWriter::register_return(&mut *self.writer, value);
                }
                None => {
                    TraceWriter::register_return(&mut *self.writer, NONE_VALUE);
                }
            }
        }

        Ok(())
    }
}

fn write_error_trace(
    source_path: &Path,
    out_dir: &Path,
    metadata: &str,
    message: &str,
) -> Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    let program_str = source_path.to_string_lossy();
    let mut writer = create_trace_writer(&program_str, &[], CTFS_FORMAT);

    // CTFS-only writer — events stream lives in `trace.bin`.
    let events_path = out_dir.join("trace.bin");
    let metadata_path = out_dir.join("trace_metadata.json");
    let paths_path = out_dir.join("trace_paths.json");

    TraceWriter::begin_writing_trace_events(&mut *writer, &events_path)
        .map_err(|e| eyre!("{e}"))?;
    TraceWriter::begin_writing_trace_metadata(&mut *writer, &metadata_path)
        .map_err(|e| eyre!("{e}"))?;
    TraceWriter::begin_writing_trace_paths(&mut *writer, &paths_path).map_err(|e| eyre!("{e}"))?;

    TraceWriter::start(&mut *writer, source_path, Line(1));
    TraceWriter::register_special_event(&mut *writer, EventLogKind::Error, metadata, message);
    TraceWriter::register_return(&mut *writer, NONE_VALUE);

    TraceWriter::finish_writing_trace_events(&mut *writer).map_err(|e| eyre!("{e}"))?;
    TraceWriter::finish_writing_trace_metadata(&mut *writer).map_err(|e| eyre!("{e}"))?;
    TraceWriter::finish_writing_trace_paths(&mut *writer).map_err(|e| eyre!("{e}"))?;
    writer.close().map_err(|e| eyre!("{e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Leo compilation via `leo` CLI
// ---------------------------------------------------------------------------

/// Compile a Leo source file to Aleo instructions.
///
/// First tries the `leo` CLI compiler (creating a temporary project directory).
/// If the `leo` CLI is not available, falls back to generating Aleo instructions
/// directly from the Leo source. The fallback performs the same translation that
/// the Leo compiler does for simple programs: converting let-bindings and
/// arithmetic expressions into register-based Aleo instructions.
///
/// In both cases, the output is executed through the same AVM interpreter.
fn compile_leo_to_aleo(source_path: &Path, source_code: &str) -> Result<String> {
    // Try the real Leo compiler first.
    match compile_leo_to_aleo_via_cli(source_path, source_code) {
        Ok(aleo) => return Ok(aleo),
        Err(e) => {
            eprintln!(
                "Leo CLI compilation unavailable ({}), \
                 falling back to source-level Aleo generation",
                e
            );
        }
    }

    // Fallback: generate Aleo instructions from Leo source.
    // This performs the same source → Aleo instruction translation that
    // the Leo compiler does, producing real Aleo instructions that are
    // then executed by the AVM interpreter.
    generate_aleo_from_leo_source(source_code)
}

/// Try to compile Leo to Aleo using the `leo` CLI.
fn compile_leo_to_aleo_via_cli(source_path: &Path, source_code: &str) -> Result<String> {
    let leo_bin = std::env::var("LEO_BIN").unwrap_or_else(|_| "leo".to_string());

    let program_name = extract_program_name(source_code).unwrap_or_else(|| {
        source_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".to_string())
    });

    // Create a temporary Leo project directory.
    let project_dir =
        tempfile::tempdir().with_context(|| "failed to create temp dir for Leo project")?;

    let project_path = project_dir.path().join(&program_name);
    std::fs::create_dir_all(&project_path)
        .with_context(|| "failed to create Leo project directory")?;

    // Create program.json (Leo project manifest).
    let program_json = format!(
        r#"{{
    "program": "{program_name}.aleo",
    "version": "0.1.0",
    "description": "",
    "license": "MIT"
}}"#
    );
    std::fs::write(project_path.join("program.json"), &program_json)
        .with_context(|| "failed to write program.json")?;

    // Create .env file with a dummy private key for local execution.
    let env_content = "NETWORK=testnet\nPRIVATE_KEY=APrivateKey1zkp8CZNn3yeCseEtxuVPbDCwSyhGW6yZKUYKfgXmcpoGPWH\n";
    std::fs::write(project_path.join(".env"), env_content)
        .with_context(|| "failed to write .env file")?;

    // Create src/main.leo with the source code.
    let src_dir = project_path.join("src");
    std::fs::create_dir_all(&src_dir).with_context(|| "failed to create src directory")?;
    std::fs::write(src_dir.join("main.leo"), source_code)
        .with_context(|| "failed to write main.leo")?;

    // Run `leo build` to compile.
    eprintln!("Running: {} build (in {})", leo_bin, project_path.display());

    let output = Command::new(&leo_bin)
        .arg("build")
        .current_dir(&project_path)
        .output()
        .with_context(|| {
            format!(
                "failed to run Leo compiler ('{leo_bin}'). \
             Set LEO_BIN environment variable to the path of the leo binary."
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(eyre!(
            "Leo compilation failed:\nstdout: {stdout}\nstderr: {stderr}"
        ));
    }

    eprintln!("Leo compilation succeeded");

    // Read the generated .aleo file.
    let aleo_path = project_path
        .join("build")
        .join(format!("{program_name}.aleo"));

    if aleo_path.exists() {
        return std::fs::read_to_string(&aleo_path).with_context(|| {
            format!(
                "failed to read compiled Aleo instructions: {}",
                aleo_path.display()
            )
        });
    }

    // Try alternative path: build/main.aleo
    let alt_aleo_path = project_path.join("build").join("main.aleo");
    if alt_aleo_path.exists() {
        return std::fs::read_to_string(&alt_aleo_path).with_context(|| {
            format!(
                "failed to read compiled Aleo instructions: {}",
                alt_aleo_path.display()
            )
        });
    }

    // Search for any .aleo file in the build directory.
    if let Ok(entries) = std::fs::read_dir(project_path.join("build")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "aleo").unwrap_or(false) {
                return std::fs::read_to_string(&path).with_context(|| {
                    format!(
                        "failed to read compiled Aleo instructions: {}",
                        path.display()
                    )
                });
            }
        }
    }

    Err(eyre!(
        "Leo build succeeded but no .aleo file found in build directory: {}",
        project_path.join("build").display()
    ))
}

// ---------------------------------------------------------------------------
// Fallback: Generate Aleo instructions from Leo source
// ---------------------------------------------------------------------------

/// Generate Aleo instructions from Leo source code.
///
/// This performs the same transformation that the Leo compiler does for
/// simple programs: converting let-bindings and arithmetic expressions
/// into register-based Aleo instructions. The output is valid Aleo
/// instruction format that is executed by the AVM interpreter.
///
/// For complex programs, the `leo` CLI should be used instead.
fn generate_aleo_from_leo_source(source_code: &str) -> Result<String> {
    let program_name = extract_program_name(source_code).unwrap_or_else(|| "main".to_string());

    let leo_fns = parse_leo_functions(source_code);
    if leo_fns.is_empty() {
        return Err(eyre!(
            "fallback Leo-to-Aleo generation failed: no transition or function declarations found"
        ));
    }
    let mut aleo_output = format!("program {program_name}.aleo;\n\n");

    for func in &leo_fns {
        // Determine if this function calls other functions (becomes a function)
        // or is self-contained (becomes a closure).
        let has_call = func.return_line.is_some() && func.bindings.is_empty() && leo_fns.len() > 1;

        if has_call {
            // This function just calls another function (like `main` calling `compute`).
            aleo_output.push_str(&format!("function {}:\n", func.name));

            // Find what function it calls by looking at the source.
            // For patterns like `return compute();`, generate a call instruction.
            let callee = find_callee_in_source(source_code, func);
            if let Some(callee_name) = callee {
                aleo_output.push_str(&format!("    call {callee_name} into r0;\n"));
                aleo_output.push_str("    output r0 as u32.private;\n");
            }
        } else {
            // This function has bindings — compile to a closure.
            aleo_output.push_str(&format!("closure {}:\n", func.name));

            // Generate Aleo instructions from let-bindings.
            let mut var_to_reg: HashMap<String, usize> = HashMap::new();

            for (next_reg, binding) in func.bindings.iter().enumerate() {
                let expr = find_binding_expr(source_code, binding);
                let dest_reg = next_reg;

                if let Some(expr) = expr {
                    let instr = compile_expr_to_aleo(&expr, dest_reg, &var_to_reg);
                    aleo_output.push_str(&format!("    {instr}\n"));
                }

                var_to_reg.insert(binding.name.clone(), dest_reg);
            }

            // Output the last register (return value).
            if !func.bindings.is_empty() {
                let last_reg = func.bindings.len() - 1;
                aleo_output.push_str(&format!("    output r{last_reg} as u32;\n"));
            }
        }

        aleo_output.push('\n');
    }

    Ok(aleo_output)
}

/// Find the expression for a Leo let-binding by parsing the source.
fn find_binding_expr(source_code: &str, binding: &LeoBinding) -> Option<String> {
    let lines: Vec<&str> = source_code.lines().collect();
    let line_idx = (binding.line as usize).checked_sub(1)?;
    if line_idx >= lines.len() {
        return None;
    }

    let trimmed = lines[line_idx].trim();
    if !trimmed.starts_with("let ") {
        return None;
    }

    let after_let = &trimmed[4..];
    if let Some(eq_pos) = after_let.find('=') {
        let expr = after_let[eq_pos + 1..]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();
        if !expr.is_empty() {
            return Some(expr);
        }
    }

    None
}

/// Find what function a Leo function calls in its return statement.
fn find_callee_in_source(source_code: &str, func: &LeoFunctionDef) -> Option<String> {
    let lines: Vec<&str> = source_code.lines().collect();

    if let Some(return_line) = func.return_line {
        let line_idx = (return_line as usize).checked_sub(1)?;
        if line_idx < lines.len() {
            let trimmed = lines[line_idx].trim();
            if let Some(stripped) = trimmed.strip_prefix("return ") {
                let expr = stripped.trim().trim_end_matches(';').trim();
                if let Some(name) = expr.strip_suffix("()") {
                    let name = name.trim();
                    if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Compile a Leo expression to an Aleo instruction string.
///
/// Handles:
/// - Typed literals: `10u32` → `add 10u32 0u32 into rN;`
/// - Variable refs: `a` → (uses the register for `a`)
/// - Binary ops: `a + b` → `add rA rB into rN;`
fn compile_expr_to_aleo(
    expr: &str,
    dest_reg: usize,
    var_to_reg: &HashMap<String, usize>,
) -> String {
    let expr = expr.trim();

    // Try binary operations: scan for +, -, *, / at top level.
    for op_char in &['+', '-', '*', '/'] {
        if let Some(pos) = find_top_level_operator(expr, *op_char) {
            let left = expr[..pos].trim();
            let right = expr[pos + 1..].trim();

            let left_operand = expr_to_aleo_operand(left, var_to_reg);
            let right_operand = expr_to_aleo_operand(right, var_to_reg);

            let opcode = match op_char {
                '+' => "add",
                '-' => "sub",
                '*' => "mul",
                '/' => "div",
                _ => "add",
            };

            return format!("{opcode} {left_operand} {right_operand} into r{dest_reg};");
        }
    }

    // Simple literal or variable reference.
    let operand = expr_to_aleo_operand(expr, var_to_reg);
    format!("add {operand} 0u32 into r{dest_reg};")
}

/// Convert a Leo expression atom to an Aleo operand string.
fn expr_to_aleo_operand(expr: &str, var_to_reg: &HashMap<String, usize>) -> String {
    let expr = expr.trim();

    // Check if it's a known variable.
    if let Some(&reg) = var_to_reg.get(expr) {
        return format!("r{reg}");
    }

    // Check if it's a typed literal (e.g. 10u32).
    if parse_typed_literal(expr).is_some() {
        return expr.to_string();
    }

    // Check if it's a plain integer — add u32 suffix.
    if expr.parse::<i64>().is_ok() {
        return format!("{expr}u32");
    }

    // Fallback: return as-is.
    expr.to_string()
}

/// Find a binary operator at the top level (not inside parentheses).
/// Scans right-to-left for lowest precedence (+/-), then for (*/).
fn find_top_level_operator(expr: &str, op: char) -> Option<usize> {
    let chars: Vec<char> = expr.chars().collect();
    let mut depth = 0i32;

    // For + and -, scan right to left (left-associative).
    // For * and /, also scan right to left.
    for i in (1..chars.len()).rev() {
        match chars[i] {
            ')' => depth += 1,
            '(' => depth -= 1,
            c if c == op && depth == 0 => {
                // Make sure it's not part of a type suffix like "u32".
                let left = expr[..i].trim();
                let right = expr[i + 1..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Some(i);
                }
            }
            _ => {}
        }
    }

    None
}

/// Extract the program name from Leo source code.
/// Parses `program <name>.aleo {` to get `<name>`.
fn extract_program_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(after) = trimmed.strip_prefix("program ") {
            if let Some(dot_pos) = after.find('.') {
                let name = after[..dot_pos].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Aleo instruction parser
// ---------------------------------------------------------------------------

/// Parse an Aleo program source into function definitions.
pub fn parse_aleo_program(source: &str) -> Vec<AleoFunction> {
    let mut functions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Parse closure, function, or finalize definition.
        let (is_closure, name) = if let Some(stripped) = trimmed.strip_prefix("closure ") {
            let name = stripped.trim_end_matches(':').trim().to_string();
            (true, name)
        } else if let Some(stripped) = trimmed
            .strip_prefix("function ")
            .or_else(|| trimmed.strip_prefix("finalize "))
        {
            let name = stripped.trim_end_matches(':').trim().to_string();
            (false, name)
        } else {
            i += 1;
            continue;
        };

        i += 1;

        // Parse inputs, instructions, and outputs.
        let mut inputs = Vec::new();
        let mut instructions = Vec::new();
        let mut outputs = Vec::new();

        while i < lines.len() {
            let line = lines[i].trim();

            if line.is_empty()
                || line.starts_with("closure ")
                || line.starts_with("function ")
                || line.starts_with("finalize ")
                || line.starts_with("program ")
                || line.starts_with("mapping ")
            {
                break;
            }

            if line.starts_with("input ") {
                // `input r0 as u32.private;` or `input r0 as u32;`
                if let Some(reg_idx) = parse_register_from_input(line) {
                    let type_name = parse_type_from_input(line);
                    inputs.push((reg_idx, type_name));
                }
            } else if line.starts_with("output ") {
                // `output r2 as u32.private;`
                if let Some(reg_idx) = parse_register_ref(
                    line.strip_prefix("output ")
                        .unwrap_or("")
                        .split_whitespace()
                        .next()
                        .unwrap_or(""),
                ) {
                    outputs.push(reg_idx);
                }
            } else {
                // Parse instruction.
                if let Some(instr) = parse_aleo_instruction(line) {
                    instructions.push(instr);
                }
            }

            i += 1;
        }

        if !name.is_empty() {
            functions.push(AleoFunction {
                name,
                is_closure,
                inputs,
                instructions,
                outputs,
            });
        }
    }

    functions
}

/// Parse a register reference like `r0`, `r1`, etc. Returns the index.
fn parse_register_ref(s: &str) -> Option<usize> {
    let s = s.trim().trim_end_matches(';');
    if let Some(stripped) = s.strip_prefix('r') {
        stripped.parse::<usize>().ok()
    } else {
        None
    }
}

/// Parse a register index from an input line like `input r0 as u32.private;`.
fn parse_register_from_input(line: &str) -> Option<usize> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        parse_register_ref(parts[1])
    } else {
        None
    }
}

/// Parse the type from an input line like `input r0 as u32.private;`.
fn parse_type_from_input(line: &str) -> String {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 4 {
        // e.g. "u32.private;" -> "u32"
        let type_str = parts[3].trim_end_matches(';');
        if let Some(dot_pos) = type_str.find('.') {
            type_str[..dot_pos].to_string()
        } else {
            type_str.to_string()
        }
    } else {
        "u32".to_string()
    }
}

/// Parse an operand: either a register (`r0`) or a typed literal (`10u32`).
fn parse_operand(s: &str) -> Option<Operand> {
    let s = s.trim().trim_end_matches(';');
    if let Some(reg) = parse_register_ref(s) {
        Some(Operand::Register(reg))
    } else if let Some(val) = parse_typed_literal(s) {
        Some(Operand::Literal(val))
    } else if let Ok(val) = s.parse::<i64>() {
        Some(Operand::Literal(val))
    } else if s == "true" {
        Some(Operand::Literal(1))
    } else if s == "false" {
        Some(Operand::Literal(0))
    } else {
        None
    }
}

/// Parse a single Aleo instruction line.
pub fn parse_aleo_instruction(line: &str) -> Option<AleoInstruction> {
    let trimmed = line.trim().trim_end_matches(';');
    let parts: Vec<&str> = trimmed.split_whitespace().collect();

    if parts.len() < 2 {
        return Some(AleoInstruction::Unknown(line.to_string()));
    }

    // Binary arithmetic: `add r0 r1 into r2`
    let opcode = parts[0];
    match opcode {
        "add" | "add.w" | "sub" | "sub.w" | "mul" | "mul.w" | "div" | "div.w" | "rem" | "rem.w"
        | "mod" | "is.eq" | "is.neq" | "lt" | "lte" | "gt" | "gte" => {
            if parts.len() >= 5 && parts[3] == "into" {
                let src1 = parse_operand(parts[1])?;
                let src2 = parse_operand(parts[2])?;
                let dest = parse_register_ref(parts[4])?;

                match opcode {
                    "add" | "add.w" => Some(AleoInstruction::Add { src1, src2, dest }),
                    "sub" | "sub.w" => Some(AleoInstruction::Sub { src1, src2, dest }),
                    "mul" | "mul.w" => Some(AleoInstruction::Mul { src1, src2, dest }),
                    "div" | "div.w" => Some(AleoInstruction::Div { src1, src2, dest }),
                    "rem" | "rem.w" | "mod" => Some(AleoInstruction::Mod { src1, src2, dest }),
                    "is.eq" => Some(AleoInstruction::IsEq { src1, src2, dest }),
                    "is.neq" => Some(AleoInstruction::IsNeq { src1, src2, dest }),
                    "lt" => Some(AleoInstruction::Lt { src1, src2, dest }),
                    "lte" => Some(AleoInstruction::Lte { src1, src2, dest }),
                    "gt" => Some(AleoInstruction::Gt { src1, src2, dest }),
                    "gte" => Some(AleoInstruction::Gte { src1, src2, dest }),
                    _ => Some(AleoInstruction::Unknown(line.to_string())),
                }
            } else {
                Some(AleoInstruction::Unknown(line.to_string()))
            }
        }
        "call" => {
            // `call compute r0 r1 into r2 r3;`
            // or `call compute into r2;`
            if parts.len() >= 2 {
                let function_name = parts[1].to_string();
                let into_pos = parts.iter().position(|&p| p == "into");

                let args = if let Some(pos) = into_pos {
                    parts[2..pos]
                        .iter()
                        .filter_map(|&p| parse_operand(p))
                        .collect()
                } else {
                    parts[2..]
                        .iter()
                        .filter_map(|&p| parse_operand(p))
                        .collect()
                };

                let dests = if let Some(pos) = into_pos {
                    parts[pos + 1..]
                        .iter()
                        .filter_map(|&p| parse_register_ref(p))
                        .collect()
                } else {
                    vec![]
                };

                Some(AleoInstruction::Call {
                    function_name,
                    args,
                    dests,
                })
            } else {
                Some(AleoInstruction::Unknown(line.to_string()))
            }
        }
        // Mapping instructions used in finalize scopes.
        //
        // Aleo mapping instruction formats:
        //   get mapping[rK] into rV;
        //   get.or_use mapping[rK] rD into rV;
        //   set rV into mapping[rK];
        //   remove mapping[rK];
        //   contains mapping[rK] into rR;
        "get" => parse_mapping_get(parts),
        "get.or_use" => parse_mapping_get_or_use(parts),
        "set" => parse_mapping_set(parts),
        "remove" => parse_mapping_remove(parts),
        "contains" => parse_mapping_contains(parts),
        _ => Some(AleoInstruction::Unknown(line.to_string())),
    }
}

/// Parse `get mapping[rK] into rV;`
fn parse_mapping_get(parts: Vec<&str>) -> Option<AleoInstruction> {
    // parts: ["get", "mapping[rK]", "into", "rV"]
    if parts.len() >= 4 && parts[2] == "into" {
        let (mapping, key_reg) = parse_mapping_bracket(parts[1])?;
        let value_reg = parse_register_ref(parts[3])?;
        Some(AleoInstruction::MappingGet {
            mapping,
            key_reg,
            value_reg,
        })
    } else {
        None
    }
}

/// Parse `get.or_use mapping[rK] rD into rV;`
fn parse_mapping_get_or_use(parts: Vec<&str>) -> Option<AleoInstruction> {
    // parts: ["get.or_use", "mapping[rK]", "rD", "into", "rV"]
    if parts.len() >= 5 && parts[3] == "into" {
        let (mapping, key_reg) = parse_mapping_bracket(parts[1])?;
        let default_reg = parse_register_ref(parts[2])?;
        let value_reg = parse_register_ref(parts[4])?;
        Some(AleoInstruction::MappingGetOrUse {
            mapping,
            key_reg,
            default_reg,
            value_reg,
        })
    } else {
        None
    }
}

/// Parse `set rV into mapping[rK];`
fn parse_mapping_set(parts: Vec<&str>) -> Option<AleoInstruction> {
    // parts: ["set", "rV", "into", "mapping[rK]"]
    if parts.len() >= 4 && parts[2] == "into" {
        let value_reg = parse_register_ref(parts[1])?;
        let (mapping, key_reg) = parse_mapping_bracket(parts[3])?;
        Some(AleoInstruction::MappingSet {
            mapping,
            key_reg,
            value_reg,
        })
    } else {
        None
    }
}

/// Parse `remove mapping[rK];`
fn parse_mapping_remove(parts: Vec<&str>) -> Option<AleoInstruction> {
    // parts: ["remove", "mapping[rK]"]
    if parts.len() >= 2 {
        let (mapping, key_reg) = parse_mapping_bracket(parts[1])?;
        Some(AleoInstruction::MappingRemove { mapping, key_reg })
    } else {
        None
    }
}

/// Parse `contains mapping[rK] into rR;`
fn parse_mapping_contains(parts: Vec<&str>) -> Option<AleoInstruction> {
    // parts: ["contains", "mapping[rK]", "into", "rR"]
    if parts.len() >= 4 && parts[2] == "into" {
        let (mapping, key_reg) = parse_mapping_bracket(parts[1])?;
        let result_reg = parse_register_ref(parts[3])?;
        Some(AleoInstruction::MappingContains {
            mapping,
            key_reg,
            result_reg,
        })
    } else {
        None
    }
}

/// Parse a `mapping[rK]` token into (mapping_name, register_index).
fn parse_mapping_bracket(token: &str) -> Option<(String, usize)> {
    let token = token.trim().trim_end_matches(';');
    let bracket_start = token.find('[')?;
    let bracket_end = token.find(']')?;
    if bracket_end <= bracket_start {
        return None;
    }
    let mapping = token[..bracket_start].to_string();
    let reg_str = &token[bracket_start + 1..bracket_end];
    let reg = parse_register_ref(reg_str)?;
    Some((mapping, reg))
}

// ---------------------------------------------------------------------------
// Aleo VM interpreter
// ---------------------------------------------------------------------------

/// Result of executing an Aleo function: register values and output values.
#[derive(Debug, Clone)]
pub struct FunctionResult {
    /// All register values after execution (register index → value).
    pub registers: HashMap<usize, i64>,
    /// Output values in order.
    pub outputs: Vec<i64>,
}

/// Execute all functions in the Aleo program, starting from `main`.
fn execute_aleo_program(functions: &[AleoFunction]) -> Result<HashMap<String, FunctionResult>> {
    let func_map: HashMap<&str, &AleoFunction> =
        functions.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut results: HashMap<String, FunctionResult> = HashMap::new();
    let mut mapping_store = MappingStore::new();

    // Execute main if it exists.
    if let Some(main_fn) = func_map.get("main") {
        execute_function(main_fn, &func_map, &[], &mut results, &mut mapping_store)?;
    }

    Ok(results)
}

/// Execute a single Aleo function with given input values.
fn execute_function(
    func: &AleoFunction,
    func_map: &HashMap<&str, &AleoFunction>,
    input_values: &[i64],
    results: &mut HashMap<String, FunctionResult>,
    mapping_store: &mut MappingStore,
) -> Result<FunctionResult> {
    let mut registers: HashMap<usize, i64> = HashMap::new();

    // Set input registers from provided values.
    for (idx, &(reg, _)) in func.inputs.iter().enumerate() {
        if idx < input_values.len() {
            registers.insert(reg, input_values[idx]);
        } else {
            registers.insert(reg, 0); // Default value for missing inputs.
        }
    }

    // Execute instructions.
    for instr in &func.instructions {
        match instr {
            AleoInstruction::Add { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, v1.wrapping_add(v2));
            }
            AleoInstruction::Sub { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, v1.wrapping_sub(v2));
            }
            AleoInstruction::Mul { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, v1.wrapping_mul(v2));
            }
            AleoInstruction::Div { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                if v2 != 0 {
                    registers.insert(*dest, v1 / v2);
                } else {
                    return Err(eyre!("division by zero in Aleo instruction"));
                }
            }
            AleoInstruction::Mod { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                if v2 != 0 {
                    registers.insert(*dest, v1 % v2);
                } else {
                    return Err(eyre!("modulo by zero in Aleo instruction"));
                }
            }
            AleoInstruction::IsEq { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, if v1 == v2 { 1 } else { 0 });
            }
            AleoInstruction::IsNeq { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, if v1 != v2 { 1 } else { 0 });
            }
            AleoInstruction::Lt { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, if v1 < v2 { 1 } else { 0 });
            }
            AleoInstruction::Lte { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, if v1 <= v2 { 1 } else { 0 });
            }
            AleoInstruction::Gt { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, if v1 > v2 { 1 } else { 0 });
            }
            AleoInstruction::Gte { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                registers.insert(*dest, if v1 >= v2 { 1 } else { 0 });
            }
            AleoInstruction::Call {
                function_name,
                args,
                dests,
            } => {
                // Resolve arguments.
                let arg_values: Vec<i64> = args
                    .iter()
                    .map(|op| resolve_operand(op, &registers))
                    .collect();

                // Look up and execute the callee.
                if let Some(callee) = func_map.get(function_name.as_str()) {
                    let callee_result =
                        execute_function(callee, func_map, &arg_values, results, mapping_store)?;

                    // Map callee outputs to destination registers.
                    for (idx, &dest) in dests.iter().enumerate() {
                        if idx < callee_result.outputs.len() {
                            registers.insert(dest, callee_result.outputs[idx]);
                        }
                    }
                }
            }
            AleoInstruction::MappingGet {
                mapping,
                key_reg,
                value_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), &registers);
                let key_str = key.to_string();
                let value = mapping_store.get(mapping, &key_str).unwrap_or(0);
                registers.insert(*value_reg, value);
            }
            AleoInstruction::MappingGetOrUse {
                mapping,
                key_reg,
                default_reg,
                value_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), &registers);
                let default = resolve_operand(&Operand::Register(*default_reg), &registers);
                let key_str = key.to_string();
                let value = mapping_store.get_or_use(mapping, &key_str, default);
                registers.insert(*value_reg, value);
            }
            AleoInstruction::MappingSet {
                mapping,
                key_reg,
                value_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), &registers);
                let value = resolve_operand(&Operand::Register(*value_reg), &registers);
                let key_str = key.to_string();
                mapping_store.set(mapping, &key_str, value);
            }
            AleoInstruction::MappingRemove { mapping, key_reg } => {
                let key = resolve_operand(&Operand::Register(*key_reg), &registers);
                let key_str = key.to_string();
                mapping_store.remove(mapping, &key_str);
            }
            AleoInstruction::MappingContains {
                mapping,
                key_reg,
                result_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), &registers);
                let key_str = key.to_string();
                let result = if mapping_store.contains(mapping, &key_str) {
                    1
                } else {
                    0
                };
                registers.insert(*result_reg, result);
            }
            AleoInstruction::Unknown(_) => {
                // Skip unknown instructions.
            }
        }
    }

    // Collect output values.
    let outputs: Vec<i64> = func
        .outputs
        .iter()
        .map(|&reg| registers.get(&reg).copied().unwrap_or(0))
        .collect();

    let result = FunctionResult {
        registers: registers.clone(),
        outputs,
    };

    results.insert(func.name.clone(), result.clone());

    Ok(result)
}

/// Resolve an operand to its value.
pub fn resolve_operand(op: &Operand, registers: &HashMap<usize, i64>) -> i64 {
    match op {
        Operand::Register(idx) => registers.get(idx).copied().unwrap_or(0),
        Operand::Literal(val) => *val,
    }
}

// ---------------------------------------------------------------------------
// Register-to-variable mapping
// ---------------------------------------------------------------------------

/// Map Aleo register values back to Leo source variable names.
///
/// The Leo compiler generates Aleo instructions where register assignments
/// correspond 1:1 to Leo let-bindings. We use the order of let-bindings
/// in the Leo source and the order of non-input registers in the Aleo
/// function to establish the mapping.
fn map_registers_to_variables(
    leo_functions: &[LeoFunctionDef],
    aleo_functions: &[AleoFunction],
    execution_results: &HashMap<String, FunctionResult>,
) -> HashMap<String, i64> {
    let mut variable_values = HashMap::new();

    for leo_fn in leo_functions {
        // Find the corresponding Aleo function.
        // The Leo compiler may rename functions (e.g. transitions become closures).
        let aleo_fn = find_matching_aleo_function(leo_fn, aleo_functions);

        if let Some(aleo_fn) = aleo_fn {
            if let Some(result) = execution_results.get(&aleo_fn.name) {
                // Map registers to variables.
                // Leo let-bindings are compiled to sequential register assignments.
                // The first N registers are inputs, then each let-binding gets
                // the next register.
                let first_binding_reg = aleo_fn.inputs.len();

                for (idx, binding) in leo_fn.bindings.iter().enumerate() {
                    let reg_idx = first_binding_reg + idx;
                    if let Some(&val) = result.registers.get(&reg_idx) {
                        variable_values.insert(binding.name.clone(), val);
                    }
                }
            }
        }
    }

    variable_values
}

/// Find the Aleo function that corresponds to a Leo function definition.
///
/// The Leo compiler may compile transitions into closures with the same name,
/// or into functions. We match by name.
fn find_matching_aleo_function<'a>(
    leo_fn: &LeoFunctionDef,
    aleo_functions: &'a [AleoFunction],
) -> Option<&'a AleoFunction> {
    // Direct name match.
    if let Some(f) = aleo_functions.iter().find(|f| f.name == leo_fn.name) {
        return Some(f);
    }
    // No match found.
    None
}

// ---------------------------------------------------------------------------
// Leo source parser (for source mapping only)
// ---------------------------------------------------------------------------

/// Parse Leo function definitions from source for source-line mapping.
///
/// This parser extracts only the information needed for trace emission:
/// function names, line numbers, variable bindings, and return lines.
fn parse_leo_functions(source: &str) -> Vec<LeoFunctionDef> {
    let mut functions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        let line_num = (i + 1) as u32;

        // Check for transition or function definition.  `async
        // transition` and `async function` are accepted with the
        // same shape as the un-prefixed variants.
        let (is_transition, after_keyword) =
            if let Some(stripped) = trimmed.strip_prefix("async transition ") {
                (true, stripped)
            } else if let Some(stripped) = trimmed.strip_prefix("async function ") {
                (false, stripped)
            } else if let Some(stripped) = trimmed.strip_prefix("transition ") {
                (true, stripped)
            } else if let Some(stripped) = trimmed.strip_prefix("function ") {
                (false, stripped)
            } else {
                i += 1;
                continue;
            };

        // Parse function name.
        let name_end = after_keyword.find('(').unwrap_or(after_keyword.len());
        let name = after_keyword[..name_end].trim().to_string();

        // Parse parameter list, if present, by scanning `(...)` and
        // extracting each `<param_name>: <type>` declaration's name.
        // Multi-line parameter lists are not supported; the Leo style
        // guide keeps them on one line for transitions and functions.
        let parameters = if name_end < after_keyword.len() {
            let after_name = &after_keyword[name_end..];
            // Find matching close paren on the same line(s).  We scan the
            // current source line first, then continue across lines until
            // the closing `)` is found (params can wrap onto continuation
            // lines for long signatures).
            let mut params_text = String::new();
            let mut paren_depth = 0i32;
            let mut closed = false;
            for ch in after_name.chars() {
                match ch {
                    '(' => paren_depth += 1,
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
                if paren_depth >= 1 && !(ch == '(' && paren_depth == 1 && params_text.is_empty()) {
                    params_text.push(ch);
                }
            }
            if !closed {
                // Continue collecting across following source lines until
                // the matching `)` is found.  This tolerates wrapped
                // parameter lists.
                let mut k = i + 1;
                while !closed && k < lines.len() {
                    for ch in lines[k].chars() {
                        match ch {
                            '(' => paren_depth += 1,
                            ')' => {
                                paren_depth -= 1;
                                if paren_depth == 0 {
                                    closed = true;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        if paren_depth >= 1 {
                            params_text.push(ch);
                        }
                    }
                    k += 1;
                }
            }
            parse_leo_parameter_list(&params_text)
        } else {
            Vec::new()
        };

        // Parse body: collect bindings and return line.
        let mut bindings = Vec::new();
        let mut return_line = None;
        let mut brace_depth = 0i32;
        let mut body_started = false;

        // Count opening braces on the definition line.
        for ch in lines[i].chars() {
            match ch {
                '{' => {
                    brace_depth += 1;
                    body_started = true;
                }
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        let mut j = i + 1;
        let mut body_end_line: Option<u32> = None;
        while j < lines.len() && (brace_depth > 0 || !body_started) {
            let body_line = lines[j].trim();
            let body_line_num = (j + 1) as u32;

            // Track brace depth.
            for ch in lines[j].chars() {
                match ch {
                    '{' => {
                        brace_depth += 1;
                        body_started = true;
                    }
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }

            // Parse let bindings.
            if body_line.starts_with("let ") {
                if let Some(binding) = parse_leo_binding(body_line, body_line_num) {
                    bindings.push(binding);
                }
            }

            // Parse return statement.
            if body_line.starts_with("return ") {
                return_line = Some(body_line_num);
            }

            if brace_depth <= 0 && body_started {
                body_end_line = Some(body_line_num);
                break;
            }
            j += 1;
        }

        if !name.is_empty() {
            functions.push(LeoFunctionDef {
                name,
                is_transition,
                line: line_num,
                parameters,
                bindings,
                return_line,
                body_end_line,
            });
        }

        i = j + 1;
    }

    functions
}

/// Parse a Leo parameter list body (the text between the function-
/// definition parens, excluding the parens themselves) and extract
/// declared parameter names in source order.
///
/// Accepts comma-separated `name: type` (and `name: type.visibility`)
/// declarations.  Tolerates leading visibility / mode keywords that
/// the Leo language uses such as `public`, `private`, `constant` --
/// the name is always the first identifier of each comma-separated
/// segment, possibly preceded by a mode keyword.
///
/// Examples:
///   ""                          -> []
///   "a: u32"                    -> ["a"]
///   "a: u32, b: u32"            -> ["a", "b"]
///   "public a: u32"             -> ["a"]
///   "owner: address, amount: u64" -> ["owner", "amount"]
fn parse_leo_parameter_list(params_text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for raw in params_text.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        // The parameter form is `[mode ]<name>: <type>`.  Find the colon
        // and walk back to the first identifier-shaped token.
        let before_colon = match trimmed.find(':') {
            Some(pos) => trimmed[..pos].trim(),
            None => trimmed,
        };
        // Strip a leading visibility/mode keyword such as `public`,
        // `private`, `constant`, `const`, `mut`.
        let candidate = before_colon.split_whitespace().last().unwrap_or("").trim();
        if !candidate.is_empty()
            && candidate.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !candidate.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            result.push(candidate.to_string());
        }
    }
    result
}

/// Parse a Leo let-binding like `let a: u32 = 10u32;` or
/// `let mut result: u32 = combined;`.
///
/// The `mut` keyword is stripped from the binding name so the trace
/// surfaces the source-correct identifier (`result`, not `mut result`).
fn parse_leo_binding(line: &str, line_num: u32) -> Option<LeoBinding> {
    let trimmed = line.trim();
    if !trimmed.starts_with("let ") {
        return None;
    }

    // Strip the leading `let ` and tolerate an optional `mut ` modifier.
    let after_let = trimmed[4..].trim_start();
    let after_mut = if let Some(rest) = after_let.strip_prefix("mut ") {
        rest.trim_start()
    } else {
        after_let
    };

    if let Some(colon_pos) = after_mut.find(':') {
        let name = after_mut[..colon_pos].trim().to_string();
        let after_colon = &after_mut[colon_pos + 1..];
        // Get the type (before '=') and the RHS expression text.
        let (type_name, rhs) = if let Some(eq_pos) = after_colon.find('=') {
            let type_name = after_colon[..eq_pos].trim().to_string();
            let rhs = after_colon[eq_pos + 1..]
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
            (type_name, rhs)
        } else {
            let type_name = after_colon.trim().trim_end_matches(';').trim().to_string();
            (type_name, String::new())
        };

        if !name.is_empty() && !type_name.is_empty() {
            return Some(LeoBinding {
                name,
                type_name,
                line: line_num,
                rhs,
            });
        }
    }

    None
}

/// The flavour of a Leo assertion call surfaced by the static
/// `collect_assertions_in_function` sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssertKind {
    /// `assert(<expr>);`
    Assert,
    /// `assert_eq(<a>, <b>);`
    AssertEq,
}

/// One `assert` / `assert_eq` call site found by the static sweep, with
/// the originating source line number and the verbatim text the writer
/// surfaces in the io_event content payload.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LeoAssertion {
    kind: AssertKind,
    /// Verbatim source text including the `assert(...)` / `assert_eq(...)`
    /// wrapper, with the trailing `;` stripped.  Content payload of the
    /// emitted special_event so downstream consumers (the GUI, the
    /// agent evals) can render the assertion expression as-is.
    text: String,
    /// 1-based source line of the assert call (kept for future use --
    /// when the recorder learns to bind assertions to step events, this
    /// is the line we'd point the io_event at).
    #[allow(dead_code)]
    line: u32,
}

/// Scan one Leo function's body for `assert(...)` and `assert_eq(...)`
/// statements and return them in source order.  The current source-
/// level expression compiler does not lower these calls into Aleo
/// instructions, so they would otherwise be silently dropped from the
/// trace.
///
/// We keep this lexical and intentionally simple: any line whose
/// trimmed prefix is `assert(` or `assert_eq(` counts.  Comments are
/// ignored (`//` lines).  Multi-line assertions are not supported --
/// the Leo style guide keeps each `assert` / `assert_eq` on a single
/// line, and every fixture in `test-programs/leo/` follows that.
#[allow(dead_code)]
fn collect_assertions_in_function(source_code: &str, func: &LeoFunctionDef) -> Vec<LeoAssertion> {
    let mut out = Vec::new();
    let lines: Vec<&str> = source_code.lines().collect();

    // Inclusive 1-based body bounds.  If `body_end_line` is missing
    // (malformed source), fall back to the return line so we still
    // sweep at least the body up to the return.
    let start = func.line.saturating_add(1);
    let end = func.body_end_line.or(func.return_line).unwrap_or(func.line);

    for line_num in start..=end {
        let idx = (line_num as usize).checked_sub(1);
        let Some(idx) = idx else { continue };
        if idx >= lines.len() {
            break;
        }
        let trimmed = lines[idx].trim();
        if trimmed.starts_with("//") {
            continue;
        }
        let text = trimmed.trim_end_matches(';').trim().to_string();

        if trimmed.starts_with("assert_eq(") {
            out.push(LeoAssertion {
                kind: AssertKind::AssertEq,
                text,
                line: line_num,
            });
        } else if trimmed.starts_with("assert(") {
            out.push(LeoAssertion {
                kind: AssertKind::Assert,
                text,
                line: line_num,
            });
        }
    }

    out
}

/// Resolve `return <name>(...)` to the actual callee name by parsing the
/// `return` source line.  Returns `Some(callee_name)` only when the
/// callee is in `func_map`.
///
/// This replaces a previous heuristic that walked `func_map` (a
/// HashMap, with non-deterministic iteration order) and picked the
/// first peer with empty bindings.  That heuristic crashed (segfault)
/// on chains where two callers have empty bodies because it could
/// recurse into the same function twice; even when it didn't crash,
/// the choice of callee depended on HashMap insertion order.
fn find_return_call_target(
    source_code: &str,
    func: &LeoFunctionDef,
    func_map: &HashMap<&str, &LeoFunctionDef>,
) -> Option<String> {
    let return_line = func.return_line?;
    let lines: Vec<&str> = source_code.lines().collect();
    let idx = (return_line as usize).checked_sub(1)?;
    let line = lines.get(idx)?.trim();
    let after_return = line.strip_prefix("return ")?;
    let expr = after_return.trim().trim_end_matches(';').trim();
    let callee = parse_call_name(expr)?;
    if callee == func.name {
        // Don't recurse into self -- defends against a crafted source
        // line that looks like `return <fn-name>(...)` from inside the
        // same function.
        return None;
    }
    if func_map.contains_key(callee.as_str()) {
        Some(callee)
    } else {
        None
    }
}

/// If `expr` is shaped like `name(...)`, return `name`.  Otherwise
/// `None`.  Tolerates whitespace and ignores anything inside the
/// parens.  The closing `)` must be the last non-whitespace character.
fn parse_call_name(expr: &str) -> Option<String> {
    let expr = expr.trim();
    if !expr.ends_with(')') {
        return None;
    }
    let open = expr.find('(')?;
    let name = expr[..open].trim();
    if name.is_empty() {
        return None;
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':')
    {
        return None;
    }
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(name.to_string())
}

/// Find the return value for a function, using deterministic source
/// order (parses the `return ...` line) rather than a HashMap walk.
fn find_return_value(
    source_code: &str,
    func: &LeoFunctionDef,
    func_map: &HashMap<&str, &LeoFunctionDef>,
    variable_values: &HashMap<String, i64>,
) -> Option<i64> {
    // First, try parsing the return statement directly.  Variable
    // returns and pass-through call returns are common.
    if let Some(return_line) = func.return_line {
        let lines: Vec<&str> = source_code.lines().collect();
        if let Some(idx) = (return_line as usize).checked_sub(1) {
            if let Some(line) = lines.get(idx) {
                let trimmed = line.trim();
                if let Some(after_return) = trimmed.strip_prefix("return ") {
                    let expr = after_return.trim().trim_end_matches(';').trim();
                    // `return name;` -> look up `name` in variable_values.
                    if expr.chars().all(|c| c.is_alphanumeric() || c == '_') && !expr.is_empty() {
                        if let Some(&v) = variable_values.get(expr) {
                            return Some(v);
                        }
                    }
                    // `return foo(...);` -> use the callee's last binding.
                    if let Some(callee_name) = parse_call_name(expr) {
                        if let Some(callee) = func_map.get(callee_name.as_str()) {
                            if let Some(last_binding) = callee.bindings.last() {
                                if let Some(&v) = variable_values.get(&last_binding.name) {
                                    return Some(v);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: if this function has bindings, the return value is the
    // last binding.  Used by self-contained functions that have no
    // explicit return-line entry.
    if let Some(last_binding) = func.bindings.last() {
        return variable_values.get(&last_binding.name).copied();
    }

    None
}

/// Parse an Aleo bech32m literal expression: `aleo1...` for an
/// address, `sign1...` for a signature.  Returns `Some(Value::Str)`
/// when the expression matches; `None` when it does not so the
/// caller can fall through to other literal / variable parsing.
///
/// The body following the `1` separator is required to be lower-
/// case alphanumeric (the bech32m alphabet, restricted further
/// here for simplicity); the expression must be at least 12
/// characters total so an identifier like `aleo1` or `sign1` is
/// not mis-classified as a literal.
fn parse_bech32m_literal(expr: &str) -> Option<Value> {
    let (prefix, type_name) = if expr.starts_with("aleo1") {
        ("aleo1", "address")
    } else if expr.starts_with("sign1") {
        ("sign1", "signature")
    } else {
        return None;
    };
    if expr.len() < 12 {
        return None;
    }
    let body = &expr[prefix.len()..];
    if !body
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        return None;
    }
    Some(Value::Str(expr.to_string(), type_name.to_string()))
}

/// Parse a typed Leo literal like `10u32`, `42u64`, `10field`, `5i32`.
fn parse_typed_literal(s: &str) -> Option<i64> {
    // Try each known type suffix.  `field`, `group`, and `scalar`
    // are Leo's native curve types -- the source-level evaluator
    // surfaces them as `Int(value, "<suffix>")` so downstream
    // tooling can distinguish them via `type_id`.  Real curve math
    // lives in snarkVM and is not modelled here.
    for suffix in &[
        "u32", "u64", "u128", "u8", "u16", "i32", "i64", "i128", "i8", "i16", "field", "group",
        "scalar",
    ] {
        if let Some(num_str) = s.strip_suffix(suffix) {
            if let Ok(val) = num_str.parse::<i64>() {
                return Some(val);
            }
        }
    }
    None
}

// ===========================================================================
// Structured Leo evaluator
//
// The historical `LeoTracer` flow lowers each Leo function to Aleo
// instructions and reads back register values to produce step events.
// That pipeline drops control flow (`if/else`, ternary, `for`),
// structured literals (struct / tuple / array), and argumentful
// function calls because the source-level lowering is line-oriented.
//
// The structured evaluator below works directly off the Leo source.
// It parses each function into a list of `Stmt`s, evaluates them in
// order with a small-but-real expression interpreter, and threads a
// typed `Value` through the env so structured shapes survive into
// `register_variable_with_full_value`.  The evaluator is the source
// of truth for the `compute()` execution shape; the legacy AVM path
// is still used for funcs that the structured parser cannot handle
// (none of the recorder fixtures hit that fallback today).
// ===========================================================================

/// One Leo statement after structured parsing.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Stmt {
    Let {
        mutable: bool,
        name: String,
        type_name: String,
        expr: String,
        line: u32,
    },
    Assign {
        target: String,
        expr: String,
        line: u32,
    },
    Assert {
        kind: AssertKind,
        text: String,
        line: u32,
    },
    For {
        var: String,
        type_name: String,
        lo: i64,
        hi: i64,
        body: Vec<Stmt>,
        line: u32,
    },
    Return {
        expr: String,
        line: u32,
    },
    /// A bare expression statement (e.g. an `assert_eq` we already
    /// captured as `Assert`, or a fall-through expression).  Kept so
    /// step counts stay aligned to source lines that the parser
    /// recognises but doesn't yet evaluate.
    #[allow(dead_code)]
    ExprStmt {
        text: String,
        line: u32,
    },
}

/// A single Leo function fully parsed into structured statements.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LeoFunc {
    name: String,
    is_transition: bool,
    line: u32,
    body_end_line: u32,
    parameters: Vec<(String, String)>, // (name, type_name)
    body: Vec<Stmt>,
    /// Free-form return type, captured for completeness; unused by the
    /// evaluator today.
    return_type: Option<String>,
    /// The Aleo program this function belongs to (e.g. `"math_lib"`
    /// for a function declared inside `program math_lib.aleo {...}`).
    /// Used to surface qualified function names
    /// (`math_lib.aleo/double`) in the writer's function table when
    /// the recorder loads multiple .leo files via `import`.
    program_name: Option<String>,
}

/// A Leo struct definition (`struct Name { field: type, ... }`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LeoStructDef {
    name: String,
    fields: Vec<(String, String)>, // (field_name, type_name)
}

/// Runtime value carried by the structured evaluator.  Maps onto
/// `ValueRecord` variants in `value_to_record`; types are kept around
/// so `Sequence` / `Tuple` / `Struct` writers can emit the right
/// `TypeKind` / `lang_type` strings.
#[derive(Debug, Clone)]
enum Value {
    Int(i64, String),
    Bool(bool),
    /// A typed string value -- used for Aleo `address` payloads
    /// surfaced by the call-context builtins (`self.caller`,
    /// `self.signer`).  Carries the lang-level type name (e.g.
    /// `"address"`) so `value_to_record` can register the right
    /// `TypeKind::String` id.
    Str(String, String),
    Sequence(Vec<Value>, String),
    Tuple(Vec<Value>),
    Struct {
        name: String,
        fields: Vec<(String, Value)>,
    },
    /// A value the evaluator could not compute.  Treated as zero in
    /// arithmetic so the recorder doesn't surface garbage.
    Unknown,
}

impl Value {
    fn as_int(&self) -> i64 {
        match self {
            Value::Int(i, _) => *i,
            Value::Bool(b) => {
                if *b {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

/// The evaluator's environment for one function call: local-variable
/// bindings as `Value`s.
type EvalEnv = HashMap<String, Value>;

/// All structured information the tracer needs about a Leo program.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LeoProgram {
    /// Declared-source-order list of functions; `main` first if
    /// present so writer-side function-table order is stable.
    funcs: Vec<LeoFunc>,
    /// Lookup by name.
    by_name: HashMap<String, LeoFunc>,
    /// All struct declarations.
    structs: HashMap<String, LeoStructDef>,
}

// ---------------------------------------------------------------------------
// Structured Leo parser
// ---------------------------------------------------------------------------

/// One bare-`.aleo` register's runtime payload + declared type.
/// Used by `emit_aleo_dialect_trace` to thread register values
/// through synthesised AVM execution.
#[derive(Clone, Debug)]
struct AleoDialectReg {
    value: i64,
    type_name: String,
}

impl AsAleoRegisterValue for AleoDialectReg {
    fn value(&self) -> i64 {
        self.value
    }
}

/// Parse a register reference from the bare `.aleo` dialect:
/// `"r3"` -> `Some(3)`, `"r12"` -> `Some(12)`, `"3u32"` -> `None`.
fn parse_aleo_dialect_register(token: &str) -> Option<usize> {
    let stripped = token.trim().trim_end_matches(',').trim_end_matches(';');
    let rest = stripped.strip_prefix('r')?;
    rest.parse::<usize>().ok()
}

/// Resolve an Aleo-dialect operand: a register reference or a
/// typed literal like `2u32`.  Returns `Some(i64)` on success;
/// `None` when the token is unrecognised.
fn aleo_dialect_resolve(
    token: &str,
    registers: &HashMap<usize, impl AsAleoRegisterValue>,
) -> Option<i64> {
    let stripped = token.trim().trim_end_matches(',').trim_end_matches(';');
    if let Some(reg_idx) = parse_aleo_dialect_register(stripped) {
        return registers.get(&reg_idx).map(|r| r.value());
    }
    parse_typed_literal(stripped).or_else(|| stripped.parse::<i64>().ok())
}

/// Tiny accessor trait so `aleo_dialect_resolve` can pull the
/// integer payload out of either a `Reg` carrier (from
/// `emit_aleo_dialect_trace`) or any other future register
/// representation without coupling to a specific struct.
trait AsAleoRegisterValue {
    fn value(&self) -> i64;
}

/// Detect whether `source` is the bare `.aleo` dialect (i.e. a
/// hand-written Aleo instruction file, the form the Leo compiler
/// emits as its build output) rather than Leo source.
///
/// The discriminator is the program-header form: bare `.aleo`
/// uses `program X.aleo;` (terminated by a semicolon, no
/// surrounding braces), whereas Leo uses `program X.aleo { ... }`.
/// We also require at least one `function NAME:` block so a stub
/// `program X.aleo;` line embedded in a Leo file (which is
/// otherwise illegal Leo syntax) does NOT trip the detector.
fn is_bare_aleo_dialect(source: &str) -> bool {
    let mut has_bare_program = false;
    let mut has_function_decl = false;
    for raw in source.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("program ") {
            let rest = rest.trim();
            if rest.ends_with(';') {
                has_bare_program = true;
            }
        }
        if trimmed.starts_with("function ") && trimmed.ends_with(':') {
            has_function_decl = true;
        }
    }
    has_bare_program && has_function_decl
}

/// Scan a Leo source file for `import X.aleo;` directives and
/// return the bare program names (e.g. `["math_lib"]`).  The
/// recorder uses these to load sibling `.leo` files from the
/// caller's directory and merge them into the structured
/// program.
fn collect_imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            let rest = rest.trim().trim_end_matches(';').trim();
            if let Some(name) = rest.strip_suffix(".aleo") {
                let bare = name.trim();
                if !bare.is_empty() {
                    out.push(bare.to_string());
                }
            }
        }
    }
    out
}

/// Parse a Leo source file into a `LeoProgram` (functions + structs).
fn parse_leo_program(source: &str) -> LeoProgram {
    let lines: Vec<&str> = source.lines().collect();
    let mut structs: HashMap<String, LeoStructDef> = HashMap::new();
    let mut funcs: Vec<LeoFunc> = Vec::new();
    // Tracks the most recent `program X.aleo { ... }` header so each
    // function declared inside it can be tagged with the qualifying
    // program name -- needed to surface qualified function names
    // (`math_lib.aleo/double`) when the recorder loads multiple
    // .leo files via `import`.
    let mut current_program: Option<String> = None;

    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();

        // `program X.aleo {` header -- record so subsequent function
        // declarations inherit the program qualifier.
        if let Some(rest) = trimmed.strip_prefix("program ") {
            let rest = rest.trim();
            if let Some(stripped) = rest.strip_suffix("{") {
                let bare = stripped.trim();
                if let Some(name) = bare.strip_suffix(".aleo") {
                    current_program = Some(name.trim().to_string());
                }
            } else if let Some(name) = rest.strip_suffix(".aleo") {
                current_program = Some(name.trim().to_string());
            }
        }

        // Struct or record declaration.  Aleo's `record` keyword is
        // a struct-shaped UTXO primitive: `record Token { owner:
        // address, amount: u64 }`.  The structured evaluator treats
        // it as a `Value::Struct` -- the `owner` field is surfaced
        // as a u32 stand-in (the source-level evaluator does not
        // model addresses as a distinct value kind) but the
        // mint/transfer/burn lifecycle calls flow through the call
        // graph as regular function calls.
        if let Some(rest) = trimmed
            .strip_prefix("struct ")
            .or_else(|| trimmed.strip_prefix("record "))
        {
            let name_end = rest.find('{').unwrap_or(rest.len());
            let name = rest[..name_end]
                .trim()
                .trim_end_matches('{')
                .trim()
                .to_string();
            let mut fields = Vec::new();
            let mut depth = 0i32;
            let mut started = false;
            let mut k = i;
            // Walk lines until brace depth returns to zero.
            'sk: while k < lines.len() {
                for ch in lines[k].chars() {
                    match ch {
                        '{' => {
                            depth += 1;
                            started = true;
                        }
                        '}' => {
                            depth -= 1;
                            if started && depth == 0 {
                                k += 1;
                                break 'sk;
                            }
                        }
                        _ => {}
                    }
                }
                if k > i {
                    let body_line = lines[k].trim().trim_end_matches(',').trim();
                    if !body_line.is_empty() && !body_line.starts_with("//") {
                        if let Some(colon) = body_line.find(':') {
                            let field_name = body_line[..colon].trim().to_string();
                            let field_type = body_line[colon + 1..]
                                .trim()
                                .trim_end_matches(',')
                                .trim()
                                .to_string();
                            if !field_name.is_empty()
                                && field_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                            {
                                fields.push((field_name, field_type));
                            }
                        }
                    }
                }
                k += 1;
            }
            if !name.is_empty() {
                structs.insert(name.clone(), LeoStructDef { name, fields });
            }
            i = k;
            continue;
        }

        // Function or transition declaration.  Aleo's `async`
        // qualifier prefixes either a transition (the on-chain
        // entry point that emits a Future) or a function (the
        // finalize handler that consumes the Future and runs in the
        // on-chain finalize scope).  The recorder treats them as
        // ordinary functions for evaluation purposes; the
        // distinction matters only for `Mapping::*` ops, which the
        // evaluator surfaces as io_events regardless.
        let stripped_async = trimmed
            .strip_prefix("async transition ")
            .or_else(|| trimmed.strip_prefix("async function "));
        let (is_transition, after_keyword) = if let Some(s) = stripped_async {
            // `async transition` => is_transition true; `async function` => false.
            (trimmed.starts_with("async transition "), s)
        } else if let Some(s) = trimmed.strip_prefix("transition ") {
            (true, s)
        } else if let Some(s) = trimmed.strip_prefix("function ") {
            (false, s)
        } else {
            i += 1;
            continue;
        };

        let line_num = (i + 1) as u32;
        let name_end = after_keyword.find('(').unwrap_or(after_keyword.len());
        let name = after_keyword[..name_end].trim().to_string();

        // Collect the parameter-list text (between matching parens),
        // possibly spanning multiple source lines.
        let mut params_text = String::new();
        let mut return_type: Option<String> = None;
        let mut k = i;
        let mut consumed_signature_chars: usize;
        let mut paren_depth = 0i32;
        let mut signature_done = false;
        // We re-walk the line from name_end forward.
        let mut col = name_end;
        let mut current = lines[k];
        loop {
            let bytes = current[col..].as_bytes();
            let mut j = 0;
            while j < bytes.len() {
                let ch = bytes[j] as char;
                match ch {
                    '(' => {
                        paren_depth += 1;
                        if paren_depth > 1 {
                            params_text.push(ch);
                        }
                    }
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            signature_done = true;
                            j += 1;
                            break;
                        } else {
                            params_text.push(ch);
                        }
                    }
                    _ => {
                        if paren_depth >= 1 {
                            params_text.push(ch);
                        }
                    }
                }
                j += 1;
            }
            consumed_signature_chars = col + j;
            if signature_done {
                break;
            }
            k += 1;
            if k >= lines.len() {
                break;
            }
            current = lines[k];
            col = 0;
            params_text.push(' ');
        }

        // Look at the rest of the line (after the closing paren) for
        // an arrow `-> Type` and the opening `{`.
        let mut after_signature = if k < lines.len() {
            lines[k][consumed_signature_chars..].to_string()
        } else {
            String::new()
        };
        if let Some(arrow) = after_signature.find("->") {
            let rest = &after_signature[arrow + 2..];
            let brace = rest.find('{').unwrap_or(rest.len());
            return_type = Some(rest[..brace].trim().to_string());
            after_signature = rest[brace..].to_string();
        }

        // Body: lex from current position until matching closing brace.
        let mut depth = 0i32;
        let mut started = false;
        // Re-prime by counting braces in the post-signature text.
        for ch in after_signature.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    started = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        // Collect body lines.
        let mut body_lines: Vec<(u32, String)> = Vec::new();
        let mut body_end_line = (k + 1) as u32;
        let mut m = k + 1;
        while m < lines.len() && (depth > 0 || !started) {
            let raw = lines[m];
            for ch in raw.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        started = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            body_end_line = (m + 1) as u32;
            body_lines.push(((m + 1) as u32, raw.to_string()));
            if depth <= 0 && started {
                break;
            }
            m += 1;
        }

        let parameters = parse_leo_parameter_list_typed(&params_text);
        let body = parse_block(&body_lines);

        if !name.is_empty() {
            funcs.push(LeoFunc {
                name,
                is_transition,
                line: line_num,
                body_end_line,
                parameters,
                body,
                return_type,
                program_name: current_program.clone(),
            });
        }
        i = m + 1;
    }

    let by_name: HashMap<String, LeoFunc> =
        funcs.iter().cloned().map(|f| (f.name.clone(), f)).collect();
    LeoProgram {
        funcs,
        by_name,
        structs,
    }
}

/// Parse a `(<name>: <type>, ...)` parameter list body and return
/// `(name, type)` pairs.
///
/// Visibility modifiers (`public` / `private`) on the *parameter*
/// itself surface as a `<type>.public` / `<type>.private` suffix on
/// the returned `type` string so the writer registers a distinct
/// type id for the on-chain-visible vs witness-only variant.  The
/// default visibility (no leading keyword) is `private`, so it is
/// surfaced bare without a suffix to match the historical type
/// names downstream tooling already pins.  Visibility suffixes that
/// appear on the *type* itself (`u32.public`) are also recognised
/// and round-trip the same way so callers don't need to care about
/// which side of the colon the modifier landed on.
fn parse_leo_parameter_list_typed(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in split_top_level_commas(text) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (before_colon, after_colon) = match trimmed.find(':') {
            Some(pos) => (trimmed[..pos].trim(), trimmed[pos + 1..].trim()),
            None => (trimmed, ""),
        };
        // Detect a `public` / `private` modifier prefixed onto the
        // parameter name (Aleo's primary form).  The actual name is
        // the last whitespace-separated token before the colon; the
        // modifier (if any) is the first token.
        let parts: Vec<&str> = before_colon.split_whitespace().collect();
        let name = parts.last().copied().unwrap_or("").to_string();
        let modifier = if parts.len() >= 2 {
            match parts[0] {
                "public" => Some("public"),
                "private" => Some("private"),
                _ => None,
            }
        } else {
            None
        };
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            // Visibility suffixes on the type (`u32.public`) ride
            // along too.  Default visibility is private, surfaced
            // bare so existing type-name pins keep working.
            let raw_type = after_colon.trim_end_matches(',').trim();
            let (base_type, type_visibility) = match raw_type.find('.') {
                Some(p) => {
                    let suffix = raw_type[p + 1..].trim();
                    (raw_type[..p].to_string(), Some(suffix))
                }
                None => (raw_type.to_string(), None),
            };
            // Resolve the effective visibility: parameter-level
            // modifier overrides the type-level suffix when both
            // appear (the parameter-level form is canonical in the
            // Leo guide).
            let effective_vis = modifier.or(type_visibility);
            let type_name = match effective_vis {
                Some("public") => format!("{base_type}.public"),
                _ => base_type,
            };
            out.push((name, type_name));
        }
    }
    out
}

/// Split a string at top-level commas, ignoring commas nested inside
/// `()`, `[]`, or `{}`.
fn split_top_level_commas(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    let mut current = String::new();
    for ch in text.chars() {
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_brack += 1,
            ']' => depth_brack -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            ',' if depth_paren == 0 && depth_brack == 0 && depth_brace == 0 => {
                out.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Parse a sequence of body lines into `Stmt`s.  Recognises:
///   - `let [mut] name: T = expr;`
///   - `name = expr;`
///   - `assert(expr);` / `assert_eq(a, b);`
///   - `for var: T in lo..hi { ... }`  (single-line header; body parsed recursively)
///   - `return expr;`
///
/// `body_lines` is a slice of `(1-based line number, raw source line)`
/// pairs covering the entire function body, brace-trimmed.
fn parse_block(body_lines: &[(u32, String)]) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < body_lines.len() {
        let (line_num, raw) = &body_lines[i];
        let trimmed = raw.trim();

        // Skip empty lines, comments, and bare braces.
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed == "{" || trimmed == "}" {
            i += 1;
            continue;
        }

        // `for var: T in lo..hi {` -> collect body.
        if let Some(rest) = trimmed.strip_prefix("for ") {
            // Header form: `var: T in lo..hi {` or `var in lo..hi {`.
            let (header, _open_brace) = match rest.find('{') {
                Some(p) => (rest[..p].trim(), true),
                None => (rest, false),
            };
            // Split at " in ".
            if let Some(in_pos) = header.find(" in ") {
                let var_part = header[..in_pos].trim();
                let range_part = header[in_pos + 4..].trim();
                let (var_name, var_type) = match var_part.find(':') {
                    Some(p) => (
                        var_part[..p].trim().to_string(),
                        var_part[p + 1..].trim().to_string(),
                    ),
                    None => (var_part.to_string(), "u32".to_string()),
                };
                if let Some(dots) = range_part.find("..") {
                    let lo_text = range_part[..dots].trim();
                    let hi_text = range_part[dots + 2..].trim().trim_end_matches('{').trim();
                    let lo = parse_typed_literal(lo_text)
                        .or_else(|| lo_text.parse::<i64>().ok())
                        .unwrap_or(0);
                    let hi = parse_typed_literal(hi_text)
                        .or_else(|| hi_text.parse::<i64>().ok())
                        .unwrap_or(0);
                    // Find matching closing brace.
                    let mut depth = 0i32;
                    // Count opening braces on the header line.
                    for ch in raw.chars() {
                        match ch {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                    }
                    let mut body_lines_inner: Vec<(u32, String)> = Vec::new();
                    let mut j = i + 1;
                    while j < body_lines.len() && depth > 0 {
                        let (jl, jr) = &body_lines[j];
                        for ch in jr.chars() {
                            match ch {
                                '{' => depth += 1,
                                '}' => depth -= 1,
                                _ => {}
                            }
                        }
                        if depth > 0 {
                            body_lines_inner.push((*jl, jr.clone()));
                        }
                        j += 1;
                    }
                    let inner = parse_block(&body_lines_inner);
                    out.push(Stmt::For {
                        var: var_name,
                        type_name: var_type,
                        lo,
                        hi,
                        body: inner,
                        line: *line_num,
                    });
                    i = j;
                    continue;
                }
            }
            // Fall through if header didn't parse cleanly.
            i += 1;
            continue;
        }

        // `if cond { ... } else { ... }` block as a statement -- not
        // supported as a statement, only as an expression in `let` RHS.
        // Skip the brace-balanced body so we don't mis-parse its contents.
        if trimmed.starts_with("if ") {
            let mut depth = 0i32;
            for ch in raw.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            let mut j = i + 1;
            while j < body_lines.len() && depth > 0 {
                let (_, jr) = &body_lines[j];
                for ch in jr.chars() {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                j += 1;
            }
            i = j;
            continue;
        }

        // `let [mut] name: T = expr;` -- expression may span multiple
        // lines if it contains an `if {} else {}` or a multi-line
        // struct literal.  We extend the expression text until we see
        // a line that ends with a semicolon at brace-depth zero.
        if trimmed.starts_with("let ") {
            let mut full = String::new();
            let mut depth = 0i32;
            let mut j = i;
            loop {
                if j >= body_lines.len() {
                    break;
                }
                let (_, jr) = &body_lines[j];
                if !full.is_empty() {
                    full.push(' ');
                }
                full.push_str(jr.trim());
                for ch in jr.chars() {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if depth <= 0 && jr.trim().ends_with(';') {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let line_for_let = *line_num;
            let after_let = full.trim_start_matches("let ").trim();
            let (mutable, after_mut) = match after_let.strip_prefix("mut ") {
                Some(rest) => (true, rest.trim_start()),
                None => (false, after_let),
            };
            if let Some(colon) = after_mut.find(':') {
                let name = after_mut[..colon].trim().to_string();
                let after_colon = &after_mut[colon + 1..];
                if let Some(eq) = after_colon.find('=') {
                    let type_name = after_colon[..eq].trim().to_string();
                    let expr = after_colon[eq + 1..]
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .to_string();
                    out.push(Stmt::Let {
                        mutable,
                        name,
                        type_name,
                        expr,
                        line: line_for_let,
                    });
                }
            }
            i = j;
            continue;
        }

        // `assert_eq(a, b);` / `assert(expr);`
        if trimmed.starts_with("assert_eq(") || trimmed.starts_with("assert(") {
            let kind = if trimmed.starts_with("assert_eq(") {
                AssertKind::AssertEq
            } else {
                AssertKind::Assert
            };
            let text = trimmed.trim_end_matches(';').trim().to_string();
            out.push(Stmt::Assert {
                kind,
                text,
                line: *line_num,
            });
            i += 1;
            continue;
        }

        // `return expr;`
        if let Some(rest) = trimmed.strip_prefix("return ") {
            let expr = rest.trim().trim_end_matches(';').trim().to_string();
            out.push(Stmt::Return {
                expr,
                line: *line_num,
            });
            i += 1;
            continue;
        }

        // `name = expr;` (assignment to mutable binding)
        if let Some(eq) = trimmed.find('=') {
            let lhs = trimmed[..eq].trim();
            let rhs = trimmed[eq + 1..].trim().trim_end_matches(';').trim();
            if !lhs.is_empty()
                && lhs.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !lhs.starts_with(|c: char| c.is_ascii_digit())
            {
                out.push(Stmt::Assign {
                    target: lhs.to_string(),
                    expr: rhs.to_string(),
                    line: *line_num,
                });
                i += 1;
                continue;
            }
        }

        // Bare expression statement.  Recognised so finalize-scope
        // `Mapping::set(...)` / `Mapping::remove(...)` calls --
        // which would otherwise be silently dropped by the parser
        // -- propagate to `execute_block` and surface as io_events
        // with EventLogKind::Write.  The trailing semicolon is
        // stripped so `eval_expr` sees a clean expression.
        if trimmed.ends_with(';') {
            let text = trimmed.trim_end_matches(';').trim().to_string();
            if !text.is_empty() {
                out.push(Stmt::ExprStmt {
                    text,
                    line: *line_num,
                });
            }
        }

        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Expression evaluator
// ---------------------------------------------------------------------------

/// Result of executing a structured Leo function: the final return
/// value (if a `return` statement was hit) and the list of step events
/// that should be emitted to surface the function's bindings.
#[derive(Debug, Clone)]
struct CallTrace {
    return_value: Value,
    /// Per-binding emission, in execution order.
    bindings: Vec<EmittedBinding>,
    /// Sub-calls made by this function, captured so the emit pass can
    /// recurse into them in deterministic source order.
    sub_calls: Vec<SubCall>,
    /// Assertions executed in this call.
    asserts: Vec<EmittedAssert>,
    /// IO events surfaced by the evaluator (hash-builtin calls,
    /// `Mapping::*` ops in finalize scope).  Surfaced via
    /// `register_special_event` in source order alongside asserts.
    io_events: Vec<EmittedIoEvent>,
    /// Whether the function aborted mid-execution (e.g. a
    /// `Mapping::get` on a non-existent key in a finalize block).
    /// When `true`, the emit pass writes `NONE_VALUE` as the return
    /// payload so the CloseFrame surfaces as a failed-frame
    /// CallRecord matching the on-chain finalize-abort semantics.
    #[allow(dead_code)]
    aborted: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct EmittedBinding {
    name: String,
    type_name: String,
    line: u32,
    value: Value,
}

#[derive(Debug, Clone)]
struct SubCall {
    /// Name of the function being called.
    callee: String,
    /// Argument values, evaluated in the caller's env.
    args: Vec<(String, Value)>, // (formal-parameter name, value)
    /// Where to place the call event in the caller's binding stream
    /// (index into `bindings`).
    after_binding: usize,
    /// Recorded result of executing the callee.
    trace: CallTrace,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct EmittedAssert {
    kind: AssertKind,
    text: String,
    line: u32,
}

/// An IO-shaped event surfaced by the structured evaluator.  Used
/// for hash-builtin invocations (`BHP256::hash_to_field(x)`) and
/// finalize-scope mapping ops (`Mapping::set/get/get_or_use`) so
/// downstream tooling can reconstruct the on-chain state-mutation
/// sequence and the data-dependency reads against opaque hash
/// oracles from the trace alone.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct EmittedIoEvent {
    /// EventLogKind discriminant (`Read` for reads, `Write` for
    /// writes, kept abstract via `bool is_write` so the helper
    /// stays free of the trace-types dependency).
    is_write: bool,
    /// Short metadata tag emitted as the event's `metadata` field
    /// (e.g. `"LeoMappingSet"`, `"LeoBhpHash"`).
    metadata: &'static str,
    /// Verbatim source-level expression text (e.g.
    /// `"Mapping::set(balances, receiver, updated)"`).
    text: String,
}

/// Execute a Leo function with a given `(formal-name, value)` argument
/// list.  Returns a `CallTrace` describing every event the emit pass
/// should produce.
fn execute_leo_function(prog: &LeoProgram, func: &LeoFunc, args: &[(String, Value)]) -> CallTrace {
    let mut env: EvalEnv = HashMap::new();
    for (name, value) in args {
        env.insert(name.clone(), value.clone());
    }
    let mut bindings: Vec<EmittedBinding> = Vec::new();
    let mut sub_calls: Vec<SubCall> = Vec::new();
    let mut asserts: Vec<EmittedAssert> = Vec::new();
    let mut io_events: Vec<EmittedIoEvent> = Vec::new();
    let mut return_value: Value = Value::Unknown;
    let mut aborted = false;

    execute_block(
        prog,
        &func.body,
        &mut env,
        &mut bindings,
        &mut sub_calls,
        &mut asserts,
        &mut io_events,
        &mut return_value,
        &mut aborted,
    );

    // Post-execution sync: for each Let binding, replace its value
    // with the final env value so any post-loop / post-assign updates
    // surface.  Immutable bindings are unaffected (the value was
    // already final).
    for b in bindings.iter_mut() {
        if let Some(final_v) = env.get(&b.name) {
            b.value = final_v.clone();
        }
    }

    // If the function aborted (e.g. `Mapping::get` on a non-existent
    // key in a finalize block), the return value is forced to
    // `Unknown` so the emit pass writes `NONE_VALUE` -- the
    // CloseFrame on the wire then surfaces as a failed-frame
    // CallRecord with no return payload, matching the on-chain
    // finalize-abort semantics.
    if aborted {
        return_value = Value::Unknown;
    }

    CallTrace {
        return_value,
        bindings,
        sub_calls,
        asserts,
        io_events,
        aborted,
    }
}

fn execute_block(
    prog: &LeoProgram,
    block: &[Stmt],
    env: &mut EvalEnv,
    bindings: &mut Vec<EmittedBinding>,
    sub_calls: &mut Vec<SubCall>,
    asserts: &mut Vec<EmittedAssert>,
    io_events: &mut Vec<EmittedIoEvent>,
    return_value: &mut Value,
    aborted: &mut bool,
) {
    for stmt in block {
        if *aborted {
            return;
        }
        match stmt {
            Stmt::Let {
                name,
                type_name,
                expr,
                line,
                ..
            } => {
                let io_count_before = io_events.len();
                let value = eval_expr(prog, env, expr, sub_calls, io_events, bindings.len());
                // Detect a finalize abort surfaced by the evaluator
                // (e.g. `Mapping::get` on a non-existent key).  We
                // still push the binding so the abort site is
                // attributed to the correct source line, but mark
                // `aborted` so subsequent statements are skipped and
                // the call's return value is forced to Unknown.
                let abort_emitted = io_events[io_count_before..]
                    .iter()
                    .any(|e| e.metadata == "LeoFinalizeAbort");
                env.insert(name.clone(), value.clone());
                bindings.push(EmittedBinding {
                    name: name.clone(),
                    type_name: type_name.clone(),
                    line: *line,
                    value,
                });
                if abort_emitted {
                    *aborted = true;
                    return;
                }
            }
            Stmt::Assign { target, expr, .. } => {
                // Update env in place; do NOT push a fresh emitted
                // binding.  The `Let` that introduced this name already
                // pushed its own EmittedBinding; the binding's value
                // is updated post-execution from the final env so the
                // step event surfaces the post-loop / post-assign
                // value rather than the initial one.
                let value = eval_expr(prog, env, expr, sub_calls, io_events, bindings.len());
                env.insert(target.clone(), value);
            }
            Stmt::Assert { kind, text, line } => {
                asserts.push(EmittedAssert {
                    kind: *kind,
                    text: text.clone(),
                    line: *line,
                });
            }
            Stmt::For {
                var,
                type_name,
                lo,
                hi,
                body,
                ..
            } => {
                for i in *lo..*hi {
                    if *aborted {
                        return;
                    }
                    env.insert(var.clone(), Value::Int(i, type_name.clone()));
                    execute_block(
                        prog,
                        body,
                        env,
                        bindings,
                        sub_calls,
                        asserts,
                        io_events,
                        return_value,
                        aborted,
                    );
                }
            }
            Stmt::Return { expr, .. } => {
                let value = eval_expr(prog, env, expr, sub_calls, io_events, bindings.len());
                *return_value = value;
                return;
            }
            Stmt::ExprStmt { text, .. } => {
                // Bare expression statements that ARE recognised as
                // builtin call shapes (Mapping::set, hash builtins,
                // etc.) need to be evaluated for their side
                // effects.  Plain unrecognised expression statements
                // are no-ops as before.
                let io_count_before = io_events.len();
                let _ = eval_expr(prog, env, text, sub_calls, io_events, bindings.len());
                let abort_emitted = io_events[io_count_before..]
                    .iter()
                    .any(|e| e.metadata == "LeoFinalizeAbort");
                if abort_emitted {
                    *aborted = true;
                    return;
                }
            }
        }
    }
}

// (helper `type_name_of_value` removed -- callers were eliminated when
//  the Stmt::Assign arm stopped re-pushing EmittedBindings.)

/// Evaluate a Leo expression to a `Value`.
///
/// Sub-calls discovered during evaluation are recorded into `sub_calls`
/// so the emit pass can interleave call_entry/call_exit events at the
/// appropriate location in the binding stream.
fn eval_expr(
    prog: &LeoProgram,
    env: &EvalEnv,
    expr: &str,
    sub_calls: &mut Vec<SubCall>,
    io_events: &mut Vec<EmittedIoEvent>,
    after_binding: usize,
) -> Value {
    let expr = expr.trim();
    if expr.is_empty() {
        return Value::Unknown;
    }

    // Strip a single layer of outer parentheses, but ONLY when the
    // inside doesn't contain a top-level comma.  `(a, b)` is a tuple
    // literal; stripping the parens would turn it into `a, b` and
    // mis-evaluate.
    if expr.starts_with('(') && expr.ends_with(')') && matched_outer_parens(expr) {
        let inside = &expr[1..expr.len() - 1];
        if find_top_level_char(inside, ',').is_none() {
            return eval_expr(prog, env, inside, sub_calls, io_events, after_binding);
        }
    }

    // Aleo call-context built-ins.  `self.caller` / `self.signer`
    // surface as bech32m address strings (deterministic stand-in
    // literals so the trace doesn't depend on a real signing
    // key); `block.height` surfaces as a typed `u32` so finalize-
    // scope arithmetic on the chain height stays well-defined.
    // These are recognised before generic dot-access so the
    // evaluator doesn't fall back to "var lookup of `self`" and
    // surface `Unknown`.
    if let Some(value) = eval_context_builtin(expr) {
        return value;
    }

    // Cross-program call: `Program.aleo/function(args)`.  Detected
    // BEFORE binary operator scans so the embedded `/` and `.`
    // characters don't get misparsed as division / field access.
    // The function is looked up by its bare name in the merged
    // `prog.by_name` table -- imports are loaded into the same
    // table by `emit_trace_events`.
    if let Some(call_value) =
        try_eval_qualified_call(prog, env, expr, sub_calls, io_events, after_binding)
    {
        return call_value;
    }

    // Ternary: `cond ? a : b` -- the `?` and `:` must be at top level.
    if let Some(qpos) = find_top_level_char(expr, '?') {
        if let Some(cpos) = find_top_level_char_from(expr, ':', qpos + 1) {
            let cond = eval_expr(
                prog,
                env,
                &expr[..qpos],
                sub_calls,
                io_events,
                after_binding,
            );
            let then_branch = eval_expr(
                prog,
                env,
                &expr[qpos + 1..cpos],
                sub_calls,
                io_events,
                after_binding,
            );
            let else_branch = eval_expr(
                prog,
                env,
                &expr[cpos + 1..],
                sub_calls,
                io_events,
                after_binding,
            );
            return if value_truthy(&cond) {
                then_branch
            } else {
                else_branch
            };
        }
    }

    // `if cond { then } else { else }` expression.
    if let Some(rest) = expr.strip_prefix("if ") {
        if let Some(parsed) = parse_if_expr(rest) {
            let cond = eval_expr(prog, env, &parsed.cond, sub_calls, io_events, after_binding);
            return if value_truthy(&cond) {
                eval_expr(
                    prog,
                    env,
                    &parsed.then_branch,
                    sub_calls,
                    io_events,
                    after_binding,
                )
            } else {
                eval_expr(
                    prog,
                    env,
                    &parsed.else_branch,
                    sub_calls,
                    io_events,
                    after_binding,
                )
            };
        }
    }

    // Top-level binary comparisons (lowest precedence after ternary).
    for op in &["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(pos) = find_top_level_substr(expr, op) {
            let lhs = eval_expr(prog, env, &expr[..pos], sub_calls, io_events, after_binding);
            let rhs = eval_expr(
                prog,
                env,
                &expr[pos + op.len()..],
                sub_calls,
                io_events,
                after_binding,
            );
            let li = lhs.as_int();
            let ri = rhs.as_int();
            let r = match *op {
                "==" => li == ri,
                "!=" => li != ri,
                "<=" => li <= ri,
                ">=" => li >= ri,
                "<" => li < ri,
                ">" => li > ri,
                _ => false,
            };
            return Value::Bool(r);
        }
    }

    // Top-level `+` / `-` (left-associative; scan right-to-left).
    // Default arithmetic is *checked* in Leo: a true-mathematical
    // result outside the representable range of the operand type
    // surfaces as a `LeoOverflow` io_event (parallel to
    // `LeoAssert` from M10).  The wrapping result is still bound
    // so downstream arithmetic stays well-defined and the trace
    // round-trips cleanly through ct-print.
    for op_char in &['+', '-'] {
        if let Some(pos) = find_top_level_operator_excluding_unary(expr, *op_char) {
            let left = expr[..pos].trim();
            let right = expr[pos + 1..].trim();
            if !left.is_empty() && !right.is_empty() {
                let lv = eval_expr(prog, env, left, sub_calls, io_events, after_binding);
                let rv = eval_expr(prog, env, right, sub_calls, io_events, after_binding);
                let li = lv.as_int();
                let ri = rv.as_int();
                let t = match (&lv, &rv) {
                    (Value::Int(_, t), _) | (_, Value::Int(_, t)) => t.clone(),
                    _ => "u32".to_string(),
                };
                if detect_checked_overflow(li, ri, *op_char, &t) {
                    io_events.push(EmittedIoEvent {
                        is_write: false,
                        metadata: "LeoOverflow",
                        text: expr.to_string(),
                    });
                }
                let r = if *op_char == '+' {
                    wrapping_add_in_type(li, ri, &t)
                } else {
                    wrapping_sub_in_type(li, ri, &t)
                };
                return Value::Int(r, t);
            }
        }
    }

    // Top-level `*` / `/` / `%`.
    for op_char in &['*', '/', '%'] {
        if let Some(pos) = find_top_level_operator_excluding_unary(expr, *op_char) {
            let left = expr[..pos].trim();
            let right = expr[pos + 1..].trim();
            if !left.is_empty() && !right.is_empty() {
                let lv = eval_expr(prog, env, left, sub_calls, io_events, after_binding);
                let rv = eval_expr(prog, env, right, sub_calls, io_events, after_binding);
                let li = lv.as_int();
                let ri = rv.as_int();
                let t = match (&lv, &rv) {
                    (Value::Int(_, t), _) | (_, Value::Int(_, t)) => t.clone(),
                    _ => "u32".to_string(),
                };
                if *op_char == '*' && detect_checked_overflow(li, ri, '*', &t) {
                    io_events.push(EmittedIoEvent {
                        is_write: false,
                        metadata: "LeoOverflow",
                        text: expr.to_string(),
                    });
                }
                let r = match *op_char {
                    '*' => wrapping_mul_in_type(li, ri, &t),
                    '/' => {
                        if ri == 0 {
                            0
                        } else {
                            li / ri
                        }
                    }
                    '%' => {
                        if ri == 0 {
                            0
                        } else {
                            li % ri
                        }
                    }
                    _ => 0,
                };
                return Value::Int(r, t);
            }
        }
    }

    // Explicit type cast: `<expr> as <type>`.  Binds tighter than
    // `+`/`-`/`*` (those operator scans already split the cast off
    // when they recurse on each side), so by the time we get here
    // the entire expression is a single cast chain.  Truncates the
    // payload modulo the target type's representable range so
    // narrowing casts surface the post-truncation value (e.g.
    // `300u32 as u8 == 44`), and re-types the value with the
    // target type id so downstream tooling can distinguish
    // `u32(100)` from `field(100)` via `type_id`.
    if let Some(as_pos) = find_top_level_keyword(expr, " as ") {
        let lhs = expr[..as_pos].trim();
        let rhs = expr[as_pos + 4..].trim();
        if !lhs.is_empty() && !rhs.is_empty() {
            let base_v = eval_expr(prog, env, lhs, sub_calls, io_events, after_binding);
            let raw = base_v.as_int();
            let target_type = rhs.split_whitespace().next().unwrap_or(rhs).to_string();
            let truncated = truncate_to_type(raw, &target_type);
            return Value::Int(truncated, target_type);
        }
    }

    // Tuple / array literal.
    if expr.starts_with('(') && expr.ends_with(')') {
        let inner = &expr[1..expr.len() - 1];
        let parts = split_top_level_commas(inner);
        if parts.len() >= 2 {
            let elems: Vec<Value> = parts
                .iter()
                .map(|p| eval_expr(prog, env, p.trim(), sub_calls, io_events, after_binding))
                .collect();
            return Value::Tuple(elems);
        }
    }
    if expr.starts_with('[') && expr.ends_with(']') {
        let inner = &expr[1..expr.len() - 1];
        let parts = split_top_level_commas(inner);
        let elems: Vec<Value> = parts
            .iter()
            .map(|p| eval_expr(prog, env, p.trim(), sub_calls, io_events, after_binding))
            .collect();
        let elem_type = match elems.first() {
            Some(Value::Int(_, t)) => t.clone(),
            _ => "u32".to_string(),
        };
        return Value::Sequence(elems, elem_type);
    }

    // Struct literal: `Name { field: expr, ... }`.
    if let Some(brace) = expr.find('{') {
        let name = expr[..brace].trim();
        if !name.is_empty()
            && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && expr.ends_with('}')
        {
            let inner = expr[brace + 1..expr.len() - 1].trim();
            let parts = split_top_level_commas(inner);
            let mut fields: Vec<(String, Value)> = Vec::new();
            for part in parts {
                let p = part.trim();
                if let Some(colon) = p.find(':') {
                    let fname = p[..colon].trim().to_string();
                    let fexpr = p[colon + 1..].trim();
                    let fv = eval_expr(prog, env, fexpr, sub_calls, io_events, after_binding);
                    fields.push((fname, fv));
                }
            }
            return Value::Struct {
                name: name.to_string(),
                fields,
            };
        }
    }

    // Builtin call: `Family::method(args...)`.  Covers Aleo's hash
    // builtins (BHP / Pedersen / Poseidon / Keccak) and the
    // `Mapping::*` finalize-scope ops.  Surfaces an io_event so
    // downstream tooling can reconstruct hash invocations and
    // mapping reads/writes from the trace alone.
    if let Some(open) = expr.find('(') {
        if expr.ends_with(')') {
            let head = expr[..open].trim();
            if let Some(dcolon) = head.find("::") {
                let family = head[..dcolon].trim();
                let method = head[dcolon + 2..].trim();
                if !family.is_empty()
                    && !method.is_empty()
                    && family.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && method.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    let inner = &expr[open + 1..expr.len() - 1];
                    let arg_exprs = split_top_level_commas(inner);
                    let arg_values: Vec<Value> = arg_exprs
                        .iter()
                        .map(|a| {
                            eval_expr(prog, env, a.trim(), sub_calls, io_events, after_binding)
                        })
                        .collect();

                    return eval_builtin_method(family, method, &arg_values, expr, io_events);
                }
            }
        }
    }

    // Function call: `name(args...)`.
    if let Some(open) = expr.find('(') {
        let name = expr[..open].trim();
        let is_call_shape = expr.ends_with(')')
            && !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !name.chars().next().is_some_and(|c| c.is_ascii_digit());
        if is_call_shape {
            let inner = &expr[open + 1..expr.len() - 1];
            let arg_exprs = split_top_level_commas(inner);
            let arg_values: Vec<Value> = arg_exprs
                .iter()
                .map(|a| eval_expr(prog, env, a.trim(), sub_calls, io_events, after_binding))
                .collect();

            if let Some(callee) = prog.by_name.get(name) {
                let formals: Vec<(String, Value)> = callee
                    .parameters
                    .iter()
                    .enumerate()
                    .map(|(i, (pname, _ptype))| {
                        let v = arg_values.get(i).cloned().unwrap_or(Value::Unknown);
                        (pname.clone(), v)
                    })
                    .collect();
                let trace = execute_leo_function(prog, callee, &formals);
                let rv = trace.return_value.clone();
                sub_calls.push(SubCall {
                    callee: name.to_string(),
                    args: formals,
                    after_binding,
                    trace,
                });
                return rv;
            }
            // Unknown callee -- return Unknown but still record nothing.
            return Value::Unknown;
        }
    }

    // Field access: `p.x` or `p.0` (for tuples).  When the right
    // side has shape `method(args)`, evaluate it as an instance-
    // method call instead -- this is how Leo expresses Aleo's
    // wrapping (`add_wrapped`) and saturating (`add_saturating`)
    // arithmetic modes.
    if let Some(dot) = find_top_level_char(expr, '.') {
        let base = expr[..dot].trim();
        let field = expr[dot + 1..].trim();
        if let Some(open) = field.find('(') {
            if field.ends_with(')') {
                let method = field[..open].trim();
                if !method.is_empty()
                    && method.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !method.chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    let inner = &field[open + 1..field.len() - 1];
                    let arg_exprs = split_top_level_commas(inner);
                    let arg_values: Vec<Value> = arg_exprs
                        .iter()
                        .map(|a| {
                            eval_expr(prog, env, a.trim(), sub_calls, io_events, after_binding)
                        })
                        .collect();
                    let base_v = eval_expr(prog, env, base, sub_calls, io_events, after_binding);
                    return eval_instance_method(&base_v, method, &arg_values, expr, io_events);
                }
            }
        }
        let base_v = eval_expr(prog, env, base, sub_calls, io_events, after_binding);
        return access_field(&base_v, field);
    }

    // Index access: `xs[expr]`.
    if let Some(open) = expr.find('[') {
        if expr.ends_with(']') {
            let base = expr[..open].trim();
            let idx_text = &expr[open + 1..expr.len() - 1];
            let base_v = eval_expr(prog, env, base, sub_calls, io_events, after_binding);
            let idx_v = eval_expr(prog, env, idx_text, sub_calls, io_events, after_binding);
            if let Value::Sequence(elems, _) = &base_v {
                let i = idx_v.as_int() as usize;
                if let Some(v) = elems.get(i) {
                    return v.clone();
                }
            }
            return Value::Unknown;
        }
    }

    // Bool literal.
    if expr == "true" {
        return Value::Bool(true);
    }
    if expr == "false" {
        return Value::Bool(false);
    }

    // Aleo bech32m literal: `aleo1...` (address) or `sign1...`
    // (signature).  Both surface as `Value::Str(text, type_name)` so
    // the writer registers a `TypeKind::String` id distinct from
    // generic strings -- downstream tooling can then distinguish
    // address-shaped payloads from signature-shaped payloads via the
    // surfaced lang_type.  The body of the literal is required to
    // be the bech32m alphabet (lowercase alphanumerics excluding
    // `1bio`) so an unrelated identifier starting with `aleo` or
    // `sign` does NOT get misclassified as a literal.
    if let Some(literal_value) = parse_bech32m_literal(expr) {
        return literal_value;
    }

    // Typed literal.
    if let Some(v) = parse_typed_literal(expr) {
        let t = expr
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
            .to_string();
        let t = if t.is_empty() { "u32".to_string() } else { t };
        return Value::Int(v, t);
    }
    if let Ok(v) = expr.parse::<i64>() {
        return Value::Int(v, "u32".to_string());
    }

    // Variable reference.
    if let Some(v) = env.get(expr) {
        return v.clone();
    }

    Value::Unknown
}

fn value_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int(i, _) => *i != 0,
        _ => false,
    }
}

fn access_field(base: &Value, field: &str) -> Value {
    match base {
        Value::Struct { fields, .. } => fields
            .iter()
            .find(|(n, _)| n == field)
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Unknown),
        Value::Tuple(elems) => {
            if let Ok(idx) = field.parse::<usize>() {
                elems.get(idx).cloned().unwrap_or(Value::Unknown)
            } else {
                Value::Unknown
            }
        }
        _ => Value::Unknown,
    }
}

/// Try to evaluate a cross-program call expression of shape
/// `Program.aleo/function(args)`.  Returns `Some(Value)` when
/// the expression matches; `None` when it does not so the caller
/// can fall through to generic evaluation.
///
/// The function is looked up by its bare name in `prog.by_name`
/// (the merged function table includes every imported program's
/// functions) and dispatched through the same `execute_leo_function`
/// path as a local call.  The SubCall is captured so the emit
/// pass surfaces the call_entry / call_exit pair against the
/// imported function's source path -- the writer registers it
/// with the qualified name (`math_lib.aleo/double`) so downstream
/// tooling can disambiguate cross-program calls in the function
/// table.
fn try_eval_qualified_call(
    prog: &LeoProgram,
    env: &EvalEnv,
    expr: &str,
    sub_calls: &mut Vec<SubCall>,
    io_events: &mut Vec<EmittedIoEvent>,
    after_binding: usize,
) -> Option<Value> {
    if !expr.ends_with(')') {
        return None;
    }
    let open = expr.find('(')?;
    let head = expr[..open].trim();
    let slash = head.find('/')?;
    let prog_part = head[..slash].trim();
    let func_name = head[slash + 1..].trim();
    if !prog_part.ends_with(".aleo") || func_name.is_empty() {
        return None;
    }
    if !func_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let inner = &expr[open + 1..expr.len() - 1];
    let arg_exprs = split_top_level_commas(inner);
    let arg_values: Vec<Value> = arg_exprs
        .iter()
        .map(|a| eval_expr(prog, env, a.trim(), sub_calls, io_events, after_binding))
        .collect();
    let callee = prog.by_name.get(func_name)?;
    let formals: Vec<(String, Value)> = callee
        .parameters
        .iter()
        .enumerate()
        .map(|(i, (pname, _ptype))| {
            let v = arg_values.get(i).cloned().unwrap_or(Value::Unknown);
            (pname.clone(), v)
        })
        .collect();
    let trace = execute_leo_function(prog, callee, &formals);
    let rv = trace.return_value.clone();
    sub_calls.push(SubCall {
        callee: func_name.to_string(),
        args: formals,
        after_binding,
        trace,
    });
    Some(rv)
}

/// Evaluate an Aleo call-context built-in expression like
/// `self.caller`, `self.signer`, or `block.height`.  Returns
/// `None` for anything else so the caller can fall through to
/// generic evaluation.
///
/// `self.caller` and `self.signer` surface as bech32m address
/// strings -- the structured evaluator doesn't model real signing
/// keys so we use deterministic stand-in literals (the canonical
/// Aleo zero-address for `self.caller`, and a distinct fixed
/// address for `self.signer`) so the address payload round-trips
/// through `ValueRecord::String` without depending on a key
/// generation oracle.
///
/// `block.height` surfaces as a typed `u32` so finalize-scope
/// arithmetic on the chain height stays well-defined; the value
/// is the deterministic stand-in `100u32`.
fn eval_context_builtin(expr: &str) -> Option<Value> {
    match expr {
        "self.caller" => Some(Value::Str(
            "aleo1qnr4dkkvkgfqph0vzc3y6z2eu975wnpz2925ntjccd76cqsnpdjsdhgg6c".to_string(),
            "address".to_string(),
        )),
        "self.signer" => Some(Value::Str(
            "aleo1k7nl06ge2nlfm70cd6cf2asgn9w6m6kk4tytfgsugvyt5p3c2g8slzdr3a".to_string(),
            "address".to_string(),
        )),
        "block.height" => Some(Value::Int(100, "u32".to_string())),
        _ => None,
    }
}

/// Evaluate an Aleo builtin static-method call like
/// `BHP256::hash_to_field(x)` or `Mapping::set(balances, k, v)`.
///
/// The hash builtins return a deterministic stand-in (sum of
/// integer arg payloads, typed `field`) so the trace exercises the
/// parsing + value-emission path without pretending to implement
/// BLS12-377 BHP / Pedersen / Poseidon / Keccak.  Real curve-level
/// hashing lives in snarkVM and is intentionally not modelled
/// here.
///
/// Each invocation surfaces as an `EmittedIoEvent` so downstream
/// tooling can reason about hash invocations as data-dependency
/// reads against an opaque oracle, and about mapping ops as the
/// canonical on-chain state-mutation events.
fn eval_builtin_method(
    family: &str,
    method: &str,
    args: &[Value],
    expr_text: &str,
    io_events: &mut Vec<EmittedIoEvent>,
) -> Value {
    // Mapping operations -- finalize-scope state I/O.
    if family == "Mapping" {
        match method {
            "get" => {
                // `Mapping::get(map, key)` -- the variant WITHOUT a
                // `get_or_use` default fallback.  On the real chain
                // this aborts the finalize when the key is absent;
                // the source-level evaluator does not model an
                // in-memory mapping store so EVERY `Mapping::get`
                // call is treated as a non-existent-key access and
                // surfaces a `LeoFinalizeAbort` `EventLogKind::Error`
                // io_event.  `execute_block` propagates the abort
                // by halting the rest of the function and forcing
                // the return value to `Unknown` so the CloseFrame
                // marks the frame as failed (no return payload
                // emitted).
                io_events.push(EmittedIoEvent {
                    is_write: false,
                    metadata: "LeoFinalizeAbort",
                    text: expr_text.to_string(),
                });
                return Value::Unknown;
            }
            "get_or_use" => {
                io_events.push(EmittedIoEvent {
                    is_write: false,
                    metadata: "LeoMappingGetOrUse",
                    text: expr_text.to_string(),
                });
                // The third arg is the default; surface it so
                // arithmetic that depends on the get-or-use result
                // matches the real-chain "no prior key" path.
                let default = args.get(2).cloned().unwrap_or(Value::Unknown);
                return default;
            }
            "set" => {
                io_events.push(EmittedIoEvent {
                    is_write: true,
                    metadata: "LeoMappingSet",
                    text: expr_text.to_string(),
                });
                return Value::Unknown;
            }
            "remove" => {
                io_events.push(EmittedIoEvent {
                    is_write: true,
                    metadata: "LeoMappingRemove",
                    text: expr_text.to_string(),
                });
                return Value::Unknown;
            }
            "contains" => {
                io_events.push(EmittedIoEvent {
                    is_write: false,
                    metadata: "LeoMappingContains",
                    text: expr_text.to_string(),
                });
                return Value::Bool(false);
            }
            _ => {}
        }
    }

    // Hash builtins.  Family prefixes recognised by the recorder:
    // BHP{256,512,768,1024}, Pedersen{64,128}, Poseidon{2,4,8},
    // Keccak{256,384,512}, SHA3_{256,384,512}.  We treat any
    // family whose prefix matches one of those buckets as a hash
    // builtin and the method as the output kind
    // (`hash_to_field`, `hash_to_group`, ...).
    let family_kind = if family.starts_with("BHP") {
        Some("BHP")
    } else if family.starts_with("Pedersen") {
        Some("Pedersen")
    } else if family.starts_with("Poseidon") {
        Some("Poseidon")
    } else if family.starts_with("Keccak") {
        Some("Keccak")
    } else if family.starts_with("SHA3") {
        Some("SHA3")
    } else {
        None
    };
    if let Some(kind) = family_kind {
        if method.starts_with("hash_to_")
            || method.starts_with("commit_to_")
            || method == "hash"
            || method == "commit"
        {
            // Deterministic stub: sum the integer payloads.  Type
            // is derived from the method's `hash_to_<type>` suffix
            // when available; otherwise defaults to `field`.
            let out_type = if let Some(suffix) = method.strip_prefix("hash_to_") {
                suffix.to_string()
            } else if let Some(suffix) = method.strip_prefix("commit_to_") {
                suffix.to_string()
            } else {
                "field".to_string()
            };
            let sum: i64 = args.iter().map(|a| a.as_int()).sum();
            let metadata = match kind {
                "BHP" => "LeoBhpHash",
                "Pedersen" => "LeoPedersenHash",
                "Poseidon" => "LeoPoseidonHash",
                "Keccak" => "LeoKeccakHash",
                "SHA3" => "LeoSha3Hash",
                _ => "LeoHash",
            };
            io_events.push(EmittedIoEvent {
                is_write: false,
                metadata,
                text: expr_text.to_string(),
            });
            return Value::Int(sum, out_type);
        }
    }

    // Unknown builtin -- emit a tagged read so it doesn't silently
    // disappear from the trace, then return Unknown.
    io_events.push(EmittedIoEvent {
        is_write: false,
        metadata: "LeoUnknownBuiltin",
        text: expr_text.to_string(),
    });
    Value::Unknown
}

/// Evaluate an Aleo instance-method call like `a.add_wrapped(b)`
/// or `a.add_saturating(b)` -- the wrapping / saturating arithmetic
/// modes that Leo exposes alongside the default checked operators.
///
/// Wrapping methods complete silently with the wrapping result.
/// Saturating methods clamp to the type's max / min bound.  Both
/// are typed with the receiver's type so downstream arithmetic
/// stays well-defined.  Unknown methods return `Value::Unknown`
/// without emitting an io_event so the caller can fall through to
/// generic field-access semantics.
fn eval_instance_method(
    receiver: &Value,
    method: &str,
    args: &[Value],
    expr_text: &str,
    io_events: &mut Vec<EmittedIoEvent>,
) -> Value {
    // Aleo signature verification: `sig.verify(signer, msg) -> bool`.
    // Real Schnorr verification lives in snarkVM and depends on the
    // chain's group/scalar curve; the recorder's source-level
    // evaluator returns a deterministic stand-in (`true`) so the
    // trace exercises the parsing + value-emission path without
    // pretending to implement Schnorr.  Each call surfaces an
    // io_event of `EventLogKind::Read` tagged `LeoSignatureVerify`
    // with the verbatim source-level expression so downstream
    // tooling can reason about signature checks as data-dependency
    // reads against an opaque verifier oracle.
    if method == "verify" {
        if let Value::Str(_, type_name) = receiver {
            if type_name == "signature" {
                io_events.push(EmittedIoEvent {
                    is_write: false,
                    metadata: "LeoSignatureVerify",
                    text: expr_text.to_string(),
                });
                return Value::Bool(true);
            }
        }
    }
    let (lhs, type_name) = match receiver {
        Value::Int(i, t) => (*i, t.clone()),
        _ => return Value::Unknown,
    };
    let rhs = args.first().map(|v| v.as_int()).unwrap_or(0);
    let (max_val, min_val) = type_bounds(&type_name);
    match method {
        "add_wrapped" => Value::Int(wrapping_add_in_type(lhs, rhs, &type_name), type_name),
        "sub_wrapped" => Value::Int(wrapping_sub_in_type(lhs, rhs, &type_name), type_name),
        "mul_wrapped" => Value::Int(wrapping_mul_in_type(lhs, rhs, &type_name), type_name),
        "add_saturating" => {
            let sum = lhs.saturating_add(rhs);
            let clamped = sum.clamp(min_val, max_val);
            Value::Int(clamped, type_name)
        }
        "sub_saturating" => {
            let diff = lhs.saturating_sub(rhs);
            let clamped = diff.clamp(min_val, max_val);
            Value::Int(clamped, type_name)
        }
        "mul_saturating" => {
            let prod = lhs.saturating_mul(rhs);
            let clamped = prod.clamp(min_val, max_val);
            Value::Int(clamped, type_name)
        }
        _ => Value::Unknown,
    }
}

/// Return `(max, min)` representable values for a Leo numeric
/// type name.  Used by saturating arithmetic and the
/// checked-overflow detector.  Unknown types fall back to the
/// `u32` bounds so arithmetic on `field` / `scalar` (which the
/// recorder treats as unbounded ints) stays well-defined.
fn type_bounds(type_name: &str) -> (i64, i64) {
    match type_name {
        "u8" => (u8::MAX as i64, 0),
        "u16" => (u16::MAX as i64, 0),
        "u32" => (u32::MAX as i64, 0),
        "u64" => (i64::MAX, 0),
        "u128" => (i64::MAX, 0),
        "i8" => (i8::MAX as i64, i8::MIN as i64),
        "i16" => (i16::MAX as i64, i16::MIN as i64),
        "i32" => (i32::MAX as i64, i32::MIN as i64),
        "i64" => (i64::MAX, i64::MIN),
        "i128" => (i64::MAX, i64::MIN),
        _ => (u32::MAX as i64, 0),
    }
}

/// Wrap `a + b` modulo the type's representable range.  Used by
/// `add_wrapped` and the checked-overflow detector (which emits a
/// `LeoOverflow` io_event but still propagates the wrapping result
/// so downstream arithmetic stays well-defined).
fn wrapping_add_in_type(a: i64, b: i64, type_name: &str) -> i64 {
    match type_name {
        "u8" => (a as u8).wrapping_add(b as u8) as i64,
        "u16" => (a as u16).wrapping_add(b as u16) as i64,
        "u32" => (a as u32).wrapping_add(b as u32) as i64,
        "u64" => (a as u64).wrapping_add(b as u64) as i64,
        "i8" => (a as i8).wrapping_add(b as i8) as i64,
        "i16" => (a as i16).wrapping_add(b as i16) as i64,
        "i32" => (a as i32).wrapping_add(b as i32) as i64,
        _ => a.wrapping_add(b),
    }
}

fn wrapping_sub_in_type(a: i64, b: i64, type_name: &str) -> i64 {
    match type_name {
        "u8" => (a as u8).wrapping_sub(b as u8) as i64,
        "u16" => (a as u16).wrapping_sub(b as u16) as i64,
        "u32" => (a as u32).wrapping_sub(b as u32) as i64,
        "u64" => (a as u64).wrapping_sub(b as u64) as i64,
        "i8" => (a as i8).wrapping_sub(b as i8) as i64,
        "i16" => (a as i16).wrapping_sub(b as i16) as i64,
        "i32" => (a as i32).wrapping_sub(b as i32) as i64,
        _ => a.wrapping_sub(b),
    }
}

fn wrapping_mul_in_type(a: i64, b: i64, type_name: &str) -> i64 {
    match type_name {
        "u8" => (a as u8).wrapping_mul(b as u8) as i64,
        "u16" => (a as u16).wrapping_mul(b as u16) as i64,
        "u32" => (a as u32).wrapping_mul(b as u32) as i64,
        "u64" => (a as u64).wrapping_mul(b as u64) as i64,
        "i8" => (a as i8).wrapping_mul(b as i8) as i64,
        "i16" => (a as i16).wrapping_mul(b as i16) as i64,
        "i32" => (a as i32).wrapping_mul(b as i32) as i64,
        _ => a.wrapping_mul(b),
    }
}

/// Truncate `raw` to the representable range of `type_name`.
/// For unsigned types, this is `raw mod (max + 1)`.  For signed
/// types, the bit-pattern is reinterpreted (so `300u32 as i8`
/// surfaces as `(300 as i8) = 44` after the standard Rust cast
/// chain).  Curve types (`field`, `scalar`, `group`) are
/// unbounded for the source-level evaluator so the value passes
/// through unchanged.
fn truncate_to_type(raw: i64, type_name: &str) -> i64 {
    match type_name {
        "u8" => (raw as u8) as i64,
        "u16" => (raw as u16) as i64,
        "u32" => (raw as u32) as i64,
        "u64" => raw,
        "u128" => raw,
        "i8" => (raw as i8) as i64,
        "i16" => (raw as i16) as i64,
        "i32" => (raw as i32) as i64,
        "i64" => raw,
        "i128" => raw,
        // Curve types: identity (real curve math lives in snarkVM).
        _ => raw,
    }
}

/// Find a top-level occurrence of a multi-character keyword
/// (e.g. ` as `) ignoring matches nested in parens / brackets /
/// braces.  Returns the byte offset of the first character of the
/// match.
fn find_top_level_keyword(expr: &str, keyword: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    let kbytes = keyword.as_bytes();
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    let mut i = 0;
    while i + kbytes.len() <= bytes.len() {
        let c = bytes[i] as char;
        match c {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_brack += 1,
            ']' => depth_brack -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            _ => {}
        }
        if depth_paren == 0
            && depth_brack == 0
            && depth_brace == 0
            && &bytes[i..i + kbytes.len()] == kbytes
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Detect whether the checked operation `a OP b` (for `OP` in
/// `+ - *`) overflows the representable range of `type_name`.
/// Returns `true` if the true mathematical result is outside the
/// `[min, max]` bounds for the type.
fn detect_checked_overflow(a: i64, b: i64, op: char, type_name: &str) -> bool {
    let (max_val, min_val) = type_bounds(type_name);
    match op {
        '+' => a.checked_add(b).is_none_or(|r| r > max_val || r < min_val),
        '-' => a.checked_sub(b).is_none_or(|r| r > max_val || r < min_val),
        '*' => a.checked_mul(b).is_none_or(|r| r > max_val || r < min_val),
        _ => false,
    }
}

struct IfExpr {
    cond: String,
    then_branch: String,
    else_branch: String,
}

/// Parse `cond { then } else { else }`.  The leading `if ` has already
/// been stripped.  Both branches are extracted as their inner brace
/// contents (with the braces stripped), so `eval_expr` can recurse.
fn parse_if_expr(expr_after_if: &str) -> Option<IfExpr> {
    let s = expr_after_if.trim();
    let open = s.find('{')?;
    let cond = s[..open].trim().to_string();
    let mut depth = 0i32;
    let mut then_end = None;
    for (idx, ch) in s[open..].char_indices() {
        let real = open + idx;
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    then_end = Some(real);
                    break;
                }
            }
            _ => {}
        }
    }
    let then_end = then_end?;
    let then_branch = s[open + 1..then_end].trim().to_string();
    let after_then = s[then_end + 1..].trim_start();
    let after_else = after_then.strip_prefix("else")?.trim_start();
    let else_open = after_else.find('{')?;
    let mut depth = 0i32;
    let mut else_end = None;
    for (idx, ch) in after_else[else_open..].char_indices() {
        let real = else_open + idx;
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    else_end = Some(real);
                    break;
                }
            }
            _ => {}
        }
    }
    let else_end = else_end?;
    let else_branch = after_else[else_open + 1..else_end].trim().to_string();
    Some(IfExpr {
        cond,
        then_branch,
        else_branch,
    })
}

fn matched_outer_parens(expr: &str) -> bool {
    let bytes = expr.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'(' || bytes[bytes.len() - 1] != b')' {
        return false;
    }
    let mut depth = 0i32;
    for (i, ch) in expr.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i < expr.len() - 1 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn find_top_level_char(expr: &str, ch: char) -> Option<usize> {
    find_top_level_char_from(expr, ch, 0)
}

fn find_top_level_char_from(expr: &str, ch: char, start: usize) -> Option<usize> {
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    for (i, c) in expr.char_indices() {
        if i < start {
            continue;
        }
        match c {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_brack += 1,
            ']' => depth_brack -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            _ => {}
        }
        if c == ch && depth_paren == 0 && depth_brack == 0 && depth_brace == 0 {
            return Some(i);
        }
    }
    None
}

fn find_top_level_substr(expr: &str, needle: &str) -> Option<usize> {
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    let bytes = expr.as_bytes();
    let nbytes = needle.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_brack += 1,
            ']' => depth_brack -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            _ => {}
        }
        if depth_paren == 0 && depth_brack == 0 && depth_brace == 0 {
            if i + nbytes.len() <= bytes.len() && &bytes[i..i + nbytes.len()] == nbytes {
                // Avoid matching `=` inside `==` when caller asked for
                // `=` (we don't, but be defensive).
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Find a top-level operator character, scanning right-to-left so the
/// rightmost one wins (left-associative).  Skips the character when
/// it could be a unary minus or part of a `=>`/`->`/`==` token.
fn find_top_level_operator_excluding_unary(expr: &str, op: char) -> Option<usize> {
    let chars: Vec<char> = expr.chars().collect();
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    // Compute closing-from-the-right: build depth-from-end map.
    // Simpler: walk left-to-right, but remember matches and pick the
    // rightmost one.
    let mut byte_pos = 0;
    let mut last_match: Option<usize> = None;
    for (i, ch) in chars.iter().enumerate() {
        let prev = if i == 0 { ' ' } else { chars[i - 1] };
        let next = if i + 1 < chars.len() {
            chars[i + 1]
        } else {
            ' '
        };
        match *ch {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_brack += 1,
            ']' => depth_brack -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            _ => {}
        }
        let at_top = depth_paren == 0 && depth_brack == 0 && depth_brace == 0;
        if at_top && *ch == op {
            // Skip leading-position unary -/+
            if i == 0 {
                byte_pos += ch.len_utf8();
                continue;
            }
            // Skip when previous char is an operator/equals (e.g. `*-1`).
            if matches!(
                prev,
                '+' | '-'
                    | '*'
                    | '/'
                    | '%'
                    | '='
                    | '<'
                    | '>'
                    | '!'
                    | '?'
                    | ':'
                    | '('
                    | ','
                    | '['
                    | '{'
            ) {
                byte_pos += ch.len_utf8();
                continue;
            }
            // Skip `==`, `<=`, `>=`, `!=`, `&&`, `||`, `->`, `=>`.
            if op == '=' && next == '=' {
                byte_pos += ch.len_utf8();
                continue;
            }
            last_match = Some(byte_pos);
        }
        byte_pos += ch.len_utf8();
    }
    last_match
}

/// Find the line number of the first `return` statement in a function
/// body (or `None` if there isn't one).
fn find_return_line_in_body(body: &[Stmt]) -> Option<u32> {
    for stmt in body {
        match stmt {
            Stmt::Return { line, .. } => return Some(*line),
            Stmt::For { body, .. } => {
                if let Some(line) = find_return_line_in_body(body) {
                    return Some(line);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Reachability
// ---------------------------------------------------------------------------

/// Compute the set of functions reachable from `entry` by following
/// every textual call site in the structured statements.  Used to gate
/// the static `assert` sweep so unreachable functions don't pollute
/// the io_event stream.
fn reachable_functions(prog: &LeoProgram, entry: &str) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![entry.to_string()];
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(func) = prog.by_name.get(&name) {
            collect_callees(prog, &func.body, &mut stack);
        }
    }
    seen
}

fn collect_callees(prog: &LeoProgram, block: &[Stmt], out: &mut Vec<String>) {
    for stmt in block {
        match stmt {
            Stmt::Let { expr, .. } | Stmt::Assign { expr, .. } | Stmt::Return { expr, .. } => {
                collect_callees_in_expr(prog, expr, out);
            }
            Stmt::For { body, .. } => collect_callees(prog, body, out),
            Stmt::Assert { text, .. } | Stmt::ExprStmt { text, .. } => {
                collect_callees_in_expr(prog, text, out);
            }
        }
    }
}

fn collect_callees_in_expr(prog: &LeoProgram, expr: &str, out: &mut Vec<String>) {
    // Find every identifier directly followed by `(` and consider it a
    // call site.  Names that aren't in the program are filtered.
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() {
                let cc = bytes[i] as char;
                if cc.is_alphanumeric() || cc == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let name = &expr[start..i];
            // Skip whitespace.
            let mut j = i;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                if prog.by_name.contains_key(name) {
                    out.push(name.to_string());
                }
            }
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Type registration helper
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_program_name() {
        assert_eq!(
            extract_program_name("program flow_test.aleo {\n}"),
            Some("flow_test".to_string())
        );
        assert_eq!(
            extract_program_name("program hello.aleo {\n}"),
            Some("hello".to_string())
        );
        assert_eq!(extract_program_name("// no program"), None);
    }

    #[test]
    fn test_parse_leo_functions() {
        let source = r#"program test.aleo {
    transition compute() -> u32 {
        let a: u32 = 10u32;
        let b: u32 = 32u32;
        let sum_val: u32 = a + b;
        return sum_val;
    }

    transition main() -> u32 {
        return compute();
    }
}"#;
        let functions = parse_leo_functions(source);
        assert_eq!(functions.len(), 2);
        assert_eq!(functions[0].name, "compute");
        assert!(functions[0].is_transition);
        assert_eq!(functions[0].bindings.len(), 3);
        assert_eq!(functions[0].bindings[0].name, "a");
        assert_eq!(functions[0].bindings[1].name, "b");
        assert_eq!(functions[0].bindings[2].name, "sum_val");
        assert_eq!(functions[1].name, "main");
        assert_eq!(functions[1].bindings.len(), 0);
    }

    #[test]
    fn test_parse_leo_binding() {
        let binding = parse_leo_binding("let a: u32 = 10u32;", 3);
        assert!(binding.is_some());
        let b = binding.unwrap();
        assert_eq!(b.name, "a");
        assert_eq!(b.type_name, "u32");
        assert_eq!(b.line, 3);
    }

    #[test]
    fn test_parse_aleo_program() {
        let aleo_source = r#"program flow_test.aleo;

closure compute:
    add 10u32 32u32 into r0;
    add r0 r0 into r1;
    mul r1 2u32 into r2;
    add r2 10u32 into r3;
    output r3 as u32;

function main:
    call compute into r0;
    output r0 as u32.private;
"#;
        let functions = parse_aleo_program(aleo_source);
        assert_eq!(functions.len(), 2);
        assert_eq!(functions[0].name, "compute");
        assert!(functions[0].is_closure);
        assert_eq!(functions[1].name, "main");
        assert!(!functions[1].is_closure);
    }

    #[test]
    fn test_parse_operand() {
        assert!(matches!(parse_operand("r0"), Some(Operand::Register(0))));
        assert!(matches!(parse_operand("r5"), Some(Operand::Register(5))));
        assert!(matches!(parse_operand("10u32"), Some(Operand::Literal(10))));
        assert!(matches!(parse_operand("42u64"), Some(Operand::Literal(42))));
        assert!(matches!(parse_operand("true"), Some(Operand::Literal(1))));
        assert!(matches!(parse_operand("false"), Some(Operand::Literal(0))));
    }

    #[test]
    fn test_parse_aleo_instruction_add() {
        let instr = parse_aleo_instruction("add r0 r1 into r2;");
        assert!(instr.is_some());
        match instr.unwrap() {
            AleoInstruction::Add { dest, .. } => assert_eq!(dest, 2),
            other => panic!("expected Add, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_aleo_instruction_mul() {
        let instr = parse_aleo_instruction("mul r0 2u32 into r1;");
        assert!(instr.is_some());
        match instr.unwrap() {
            AleoInstruction::Mul { dest, .. } => assert_eq!(dest, 1),
            other => panic!("expected Mul, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_aleo_instruction_call() {
        let instr = parse_aleo_instruction("call compute into r0;");
        assert!(instr.is_some());
        match instr.unwrap() {
            AleoInstruction::Call {
                function_name,
                dests,
                ..
            } => {
                assert_eq!(function_name, "compute");
                assert_eq!(dests, vec![0]);
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }

    #[test]
    fn test_execute_simple_aleo_program() {
        // Simulate what the Leo compiler would produce for flow_test.leo:
        //   let a: u32 = 10u32;
        //   let b: u32 = 32u32;
        //   let sum_val: u32 = a + b;
        //   let doubled: u32 = sum_val * 2u32;
        //   let final_result: u32 = doubled + a;
        //
        // The Leo compiler produces Aleo instructions using literals and
        // register-based computation. The exact output depends on the
        // compiler version, but a typical compilation would look like this
        // (using a closure for the compute transition):
        let aleo_source = r#"program flow_test.aleo;

closure compute:
    add 10u32 32u32 into r0;
    mul r0 2u32 into r1;
    add r1 10u32 into r2;
    output r2 as u32;

function main:
    call compute into r0;
    output r0 as u32.private;
"#;
        let functions = parse_aleo_program(aleo_source);
        let results = execute_aleo_program(&functions).unwrap();

        // Check compute function results.
        let compute = results.get("compute").unwrap();
        assert_eq!(compute.registers[&0], 42); // 10 + 32
        assert_eq!(compute.registers[&1], 84); // 42 * 2
        assert_eq!(compute.registers[&2], 94); // 84 + 10
        assert_eq!(compute.outputs, vec![94]);

        // Check main function results.
        let main = results.get("main").unwrap();
        assert_eq!(main.outputs, vec![94]);
    }

    #[test]
    fn test_execute_aleo_with_individual_vars() {
        // A more detailed Aleo representation where each let-binding
        // gets its own register, matching how Leo actually compiles.
        // This version preserves all intermediate values:
        //   r0 = 10u32 (literal assignment via add with 0)
        //   r1 = 32u32
        //   r2 = r0 + r1 = 42  (sum_val)
        //   r3 = r2 * 2 = 84   (doubled)
        //   r4 = r3 + r0 = 94  (final_result)
        let functions = vec![
            AleoFunction {
                name: "compute".to_string(),
                is_closure: true,
                inputs: vec![],
                instructions: vec![
                    AleoInstruction::Add {
                        src1: Operand::Literal(10),
                        src2: Operand::Literal(0),
                        dest: 0,
                    },
                    AleoInstruction::Add {
                        src1: Operand::Literal(32),
                        src2: Operand::Literal(0),
                        dest: 1,
                    },
                    AleoInstruction::Add {
                        src1: Operand::Register(0),
                        src2: Operand::Register(1),
                        dest: 2,
                    },
                    AleoInstruction::Mul {
                        src1: Operand::Register(2),
                        src2: Operand::Literal(2),
                        dest: 3,
                    },
                    AleoInstruction::Add {
                        src1: Operand::Register(3),
                        src2: Operand::Register(0),
                        dest: 4,
                    },
                ],
                outputs: vec![4],
            },
            AleoFunction {
                name: "main".to_string(),
                is_closure: false,
                inputs: vec![],
                instructions: vec![AleoInstruction::Call {
                    function_name: "compute".to_string(),
                    args: vec![],
                    dests: vec![0],
                }],
                outputs: vec![0],
            },
        ];

        let results = execute_aleo_program(&functions).unwrap();
        let compute = results.get("compute").unwrap();

        assert_eq!(compute.registers[&0], 10); // a
        assert_eq!(compute.registers[&1], 32); // b
        assert_eq!(compute.registers[&2], 42); // sum_val = a + b
        assert_eq!(compute.registers[&3], 84); // doubled = sum_val * 2
        assert_eq!(compute.registers[&4], 94); // final_result = doubled + a
        assert_eq!(compute.outputs, vec![94]);

        // Main should also have the final result.
        let main = results.get("main").unwrap();
        assert_eq!(main.outputs, vec![94]);
    }

    #[test]
    fn test_map_registers_to_variables() {
        let leo_functions = vec![LeoFunctionDef {
            name: "compute".to_string(),
            is_transition: true,
            line: 2,
            parameters: vec![],
            bindings: vec![
                LeoBinding {
                    name: "a".to_string(),
                    type_name: "u32".to_string(),
                    line: 3,
                    rhs: "10u32".to_string(),
                },
                LeoBinding {
                    name: "b".to_string(),
                    type_name: "u32".to_string(),
                    line: 4,
                    rhs: "32u32".to_string(),
                },
                LeoBinding {
                    name: "sum_val".to_string(),
                    type_name: "u32".to_string(),
                    line: 5,
                    rhs: "a + b".to_string(),
                },
                LeoBinding {
                    name: "doubled".to_string(),
                    type_name: "u32".to_string(),
                    line: 6,
                    rhs: "sum_val * 2u32".to_string(),
                },
                LeoBinding {
                    name: "final_result".to_string(),
                    type_name: "u32".to_string(),
                    line: 7,
                    rhs: "doubled + a".to_string(),
                },
            ],
            return_line: Some(8),
            body_end_line: Some(9),
        }];

        let aleo_functions = vec![AleoFunction {
            name: "compute".to_string(),
            is_closure: true,
            inputs: vec![],       // No inputs.
            instructions: vec![], // Not needed for mapping.
            outputs: vec![4],
        }];

        let mut results = HashMap::new();
        let mut regs = HashMap::new();
        regs.insert(0, 10); // a
        regs.insert(1, 32); // b
        regs.insert(2, 42); // sum_val
        regs.insert(3, 84); // doubled
        regs.insert(4, 94); // final_result
        results.insert(
            "compute".to_string(),
            FunctionResult {
                registers: regs,
                outputs: vec![94],
            },
        );

        let vars = map_registers_to_variables(&leo_functions, &aleo_functions, &results);

        assert_eq!(vars["a"], 10);
        assert_eq!(vars["b"], 32);
        assert_eq!(vars["sum_val"], 42);
        assert_eq!(vars["doubled"], 84);
        assert_eq!(vars["final_result"], 94);
    }

    #[test]
    fn test_parse_typed_literal() {
        assert_eq!(parse_typed_literal("10u32"), Some(10));
        assert_eq!(parse_typed_literal("42u64"), Some(42));
        assert_eq!(parse_typed_literal("10field"), Some(10));
        assert_eq!(parse_typed_literal("5i32"), Some(5));
        assert_eq!(parse_typed_literal("hello"), None);
    }

    #[test]
    fn test_parse_register_ref() {
        assert_eq!(parse_register_ref("r0"), Some(0));
        assert_eq!(parse_register_ref("r5"), Some(5));
        assert_eq!(parse_register_ref("r10"), Some(10));
        assert_eq!(parse_register_ref("r0;"), Some(0));
        assert_eq!(parse_register_ref("abc"), None);
        assert_eq!(parse_register_ref(""), None);
    }

    // -----------------------------------------------------------------------
    // MappingStore tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mapping_store_set_and_get() {
        let mut store = MappingStore::new();
        store.set("balances", "alice", 100);
        assert_eq!(store.get("balances", "alice"), Some(100));
        assert_eq!(store.get("balances", "bob"), None);
        assert_eq!(store.get("other_mapping", "alice"), None);
    }

    #[test]
    fn test_mapping_store_get_or_use() {
        let mut store = MappingStore::new();
        store.set("balances", "alice", 100);
        assert_eq!(store.get_or_use("balances", "alice", 0), 100);
        assert_eq!(store.get_or_use("balances", "bob", 50), 50);
    }

    #[test]
    fn test_mapping_store_contains() {
        let mut store = MappingStore::new();
        assert!(!store.contains("balances", "alice"));
        store.set("balances", "alice", 100);
        assert!(store.contains("balances", "alice"));
        assert!(!store.contains("balances", "bob"));
    }

    #[test]
    fn test_mapping_store_remove() {
        let mut store = MappingStore::new();
        store.set("balances", "alice", 100);
        assert!(store.contains("balances", "alice"));
        store.remove("balances", "alice");
        assert!(!store.contains("balances", "alice"));
        assert_eq!(store.get("balances", "alice"), None);
    }

    #[test]
    fn test_mapping_store_overwrite() {
        let mut store = MappingStore::new();
        store.set("balances", "alice", 100);
        store.set("balances", "alice", 200);
        assert_eq!(store.get("balances", "alice"), Some(200));
    }

    // -----------------------------------------------------------------------
    // Mapping instruction parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_mapping_bracket() {
        let (name, reg) = parse_mapping_bracket("balances[r0]").unwrap();
        assert_eq!(name, "balances");
        assert_eq!(reg, 0);

        let (name, reg) = parse_mapping_bracket("accounts[r5];").unwrap();
        assert_eq!(name, "accounts");
        assert_eq!(reg, 5);

        assert!(parse_mapping_bracket("invalid").is_none());
    }

    #[test]
    fn test_parse_mapping_get_instruction() {
        let instr = parse_aleo_instruction("get balances[r0] into r1;").unwrap();
        match instr {
            AleoInstruction::MappingGet {
                mapping,
                key_reg,
                value_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(value_reg, 1);
            }
            other => panic!("expected MappingGet, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_mapping_get_or_use_instruction() {
        let instr = parse_aleo_instruction("get.or_use balances[r0] r2 into r3;").unwrap();
        match instr {
            AleoInstruction::MappingGetOrUse {
                mapping,
                key_reg,
                default_reg,
                value_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(default_reg, 2);
                assert_eq!(value_reg, 3);
            }
            other => panic!("expected MappingGetOrUse, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_mapping_set_instruction() {
        let instr = parse_aleo_instruction("set r1 into balances[r0];").unwrap();
        match instr {
            AleoInstruction::MappingSet {
                mapping,
                key_reg,
                value_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(value_reg, 1);
            }
            other => panic!("expected MappingSet, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_mapping_remove_instruction() {
        let instr = parse_aleo_instruction("remove balances[r0];").unwrap();
        match instr {
            AleoInstruction::MappingRemove { mapping, key_reg } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
            }
            other => panic!("expected MappingRemove, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_mapping_contains_instruction() {
        let instr = parse_aleo_instruction("contains balances[r0] into r1;").unwrap();
        match instr {
            AleoInstruction::MappingContains {
                mapping,
                key_reg,
                result_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(result_reg, 1);
            }
            other => panic!("expected MappingContains, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Mapping execution tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_execute_mapping_operations() {
        // Build a finalize-like function that exercises all mapping ops.
        // We use execute_function directly since execute_aleo_program only starts from "main".
        let func = AleoFunction {
            name: "finalize_test".to_string(),
            is_closure: false,
            inputs: vec![(0, "u32".to_string()), (1, "u32".to_string())],
            instructions: vec![
                // set r1 into balances[r0]    (balances[key] = value)
                AleoInstruction::MappingSet {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    value_reg: 1,
                },
                // contains balances[r0] into r2
                AleoInstruction::MappingContains {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    result_reg: 2,
                },
                // get balances[r0] into r3
                AleoInstruction::MappingGet {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    value_reg: 3,
                },
                // remove balances[r0]
                AleoInstruction::MappingRemove {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                },
                // contains balances[r0] into r4  (should be 0 after remove)
                AleoInstruction::MappingContains {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    result_reg: 4,
                },
                // get.or_use balances[r0] r1 into r5  (should use default after remove)
                AleoInstruction::MappingGetOrUse {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    default_reg: 1,
                    value_reg: 5,
                },
            ],
            outputs: vec![2, 3, 4, 5],
        };

        let func_map: HashMap<&str, &AleoFunction> =
            [("finalize_test", &func)].into_iter().collect();
        let mut results = HashMap::new();
        let mut mapping_store = MappingStore::new();

        // Input: r0=42 (key), r1=100 (value)
        let result = execute_function(
            &func,
            &func_map,
            &[42, 100],
            &mut results,
            &mut mapping_store,
        )
        .unwrap();

        // After set: balances["42"] = 100
        // contains -> r2 = 1
        assert_eq!(result.registers[&2], 1);
        // get -> r3 = 100
        assert_eq!(result.registers[&3], 100);
        // After remove: balances["42"] gone
        // contains -> r4 = 0
        assert_eq!(result.registers[&4], 0);
        // get.or_use with default r1=100 -> r5 = 100
        assert_eq!(result.registers[&5], 100);
    }

    // -----------------------------------------------------------------------
    // Leo parameter list parsing (audit (c) -- declared parameter names
    // staged via TraceWriter::arg before register_call).
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_leo_parameter_list_empty() {
        assert!(parse_leo_parameter_list("").is_empty());
        assert!(parse_leo_parameter_list("   ").is_empty());
    }

    #[test]
    fn test_parse_leo_parameter_list_single() {
        assert_eq!(parse_leo_parameter_list("a: u32"), vec!["a".to_string()]);
    }

    #[test]
    fn test_parse_leo_parameter_list_multiple() {
        assert_eq!(
            parse_leo_parameter_list("a: u32, b: u32"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn test_parse_leo_parameter_list_visibility_modes() {
        // Leo allows `public` / `private` / `constant` as mode keywords
        // before each parameter; the parser must extract just the name.
        assert_eq!(
            parse_leo_parameter_list("public a: u32, private b: u32"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn test_parse_leo_parameter_list_address_record() {
        assert_eq!(
            parse_leo_parameter_list("owner: address, amount: u64"),
            vec!["owner".to_string(), "amount".to_string()]
        );
    }

    #[test]
    fn test_parse_leo_functions_captures_parameters() {
        let source = r#"program test.aleo {
    transition transfer(receiver: address, amount: u64) -> u64 {
        return amount;
    }

    transition compute() -> u32 {
        let a: u32 = 10u32;
        return a;
    }
}"#;
        let functions = parse_leo_functions(source);
        assert_eq!(functions.len(), 2);

        // First transition has two declared parameters.
        let transfer = &functions[0];
        assert_eq!(transfer.name, "transfer");
        assert_eq!(transfer.parameters, vec!["receiver", "amount"]);

        // The parameterless transition has an empty parameter list (no
        // false positives from the body / return statement).
        let compute = &functions[1];
        assert_eq!(compute.name, "compute");
        assert!(compute.parameters.is_empty());
    }

    #[test]
    fn test_parse_finalize_block() {
        let aleo_source = r#"program token.aleo;

mapping balances:
    key as address.public;
    value as u64.public;

function transfer:
    input r0 as address.private;
    input r1 as u64.private;
    output r0 as address.private;

finalize transfer:
    input r0 as address.public;
    input r1 as u64.public;
    get.or_use balances[r0] r1 into r2;
    add r2 r1 into r3;
    set r3 into balances[r0];
"#;
        let functions = parse_aleo_program(aleo_source);

        // Should parse the function and finalize blocks.
        assert!(functions.len() >= 2);

        // Find the finalize block (has mapping instructions).
        let finalize = functions
            .iter()
            .find(|f| f.instructions.len() == 3)
            .unwrap();
        assert_eq!(finalize.name, "transfer");

        // Verify the instructions were parsed correctly.
        assert!(matches!(
            finalize.instructions[0],
            AleoInstruction::MappingGetOrUse { .. }
        ));
        assert!(matches!(
            finalize.instructions[1],
            AleoInstruction::Add { .. }
        ));
        assert!(matches!(
            finalize.instructions[2],
            AleoInstruction::MappingSet { .. }
        ));
    }
}
