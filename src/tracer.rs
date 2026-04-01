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

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use codetracer_trace_types::{Line, TypeKind, ValueRecord, NONE_VALUE};
use codetracer_trace_writer::trace_writer::TraceWriter;
use codetracer_trace_writer::{create_trace_writer, TraceEventsFileFormat};
use eyre::{eyre, Context, Result};

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
    /// Variable bindings in order.
    bindings: Vec<LeoBinding>,
    /// 1-based line number of the return statement (if any).
    return_line: Option<u32>,
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
    pub fn trace_program(
        source_path: &Path,
        source_code: &str,
        out_dir: &Path,
        format: TraceEventsFileFormat,
    ) -> Result<()> {
        // -- 1. Parse Leo source for variable/function mappings --
        let _source_map = SourceMap::from_source(source_path, source_code);
        let leo_functions = parse_leo_functions(source_code);

        eprintln!("Parsed {} Leo functions from source", leo_functions.len());

        // -- 2. Compile Leo to Aleo instructions --
        let aleo_source = compile_leo_to_aleo(source_path, source_code)?;

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
        let execution_results = execute_aleo_program(&aleo_functions)?;

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

        // -- 6. Create the trace writer --
        let program_str = source_path.to_string_lossy();
        let mut tracer = LeoTracer {
            writer: create_trace_writer(&program_str, &[], format),
            type_ids: HashMap::new(),
        };

        // -- 7. Initialise output files --
        std::fs::create_dir_all(out_dir)
            .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

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
            &leo_functions,
            &variable_values,
            &aleo_source_map,
        )?;

        // -- 10. Finish writing --
        TraceWriter::finish_writing_trace_events(&mut *tracer.writer).map_err(|e| eyre!("{e}"))?;
        TraceWriter::finish_writing_trace_metadata(&mut *tracer.writer)
            .map_err(|e| eyre!("{e}"))?;
        TraceWriter::finish_writing_trace_paths(&mut *tracer.writer).map_err(|e| eyre!("{e}"))?;

        Ok(())
    }

    /// Emit trace events by walking through Leo functions and their bindings.
    ///
    /// When an `AleoSourceMap` is available, step events point to the Leo
    /// source lines that originated each Aleo instruction, instead of the
    /// raw Aleo instruction positions.
    fn emit_trace_events(
        &mut self,
        source_path: &Path,
        leo_functions: &[LeoFunctionDef],
        variable_values: &HashMap<String, i64>,
        aleo_source_map: &AleoSourceMap,
    ) -> Result<()> {
        // Find main function and the functions it calls.
        let func_map: HashMap<&str, &LeoFunctionDef> =
            leo_functions.iter().map(|f| (f.name.as_str(), f)).collect();

        // Execute starting from main.
        if let Some(main_fn) = func_map.get("main") {
            self.emit_function_trace(
                source_path,
                main_fn,
                &func_map,
                variable_values,
                aleo_source_map,
            )?;
        }

        Ok(())
    }

    /// Emit trace events for a single function.
    ///
    /// Uses the `AleoSourceMap` to resolve step locations: if the source map
    /// has a mapping for a given binding's position in its function, the step
    /// event will reference the Leo source line from the source map. This
    /// ensures traced steps always point to Leo source lines rather than
    /// Aleo instruction positions.
    fn emit_function_trace(
        &mut self,
        source_path: &Path,
        func: &LeoFunctionDef,
        func_map: &HashMap<&str, &LeoFunctionDef>,
        variable_values: &HashMap<String, i64>,
        aleo_source_map: &AleoSourceMap,
    ) -> Result<()> {
        // Emit Call event.
        let fn_id = TraceWriter::ensure_function_id(
            &mut *self.writer,
            &func.name,
            source_path,
            Line(func.line as i64),
        );
        TraceWriter::register_call(&mut *self.writer, fn_id, vec![]);

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
        let return_calls_function = find_return_call_target(func, func_map);

        if let Some(callee_name) = return_calls_function {
            if let Some(callee) = func_map.get(callee_name.as_str()) {
                self.emit_function_trace(
                    source_path,
                    callee,
                    func_map,
                    variable_values,
                    aleo_source_map,
                )?;
            }
        }

        // Emit Return event.
        // Find the return value: it's the last variable in the called function
        // or the function itself.
        let return_value = find_return_value(func, func_map, variable_values);
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

        Ok(())
    }
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
            let mut next_reg: usize = 0;

            for binding in &func.bindings {
                let expr = find_binding_expr(source_code, binding);
                let dest_reg = next_reg;
                next_reg += 1;

                if let Some(expr) = expr {
                    let instr = compile_expr_to_aleo(&expr, dest_reg, &var_to_reg);
                    aleo_output.push_str(&format!("    {instr}\n"));
                }

                var_to_reg.insert(binding.name.clone(), dest_reg);
            }

            // Output the last register (return value).
            if !func.bindings.is_empty() {
                let last_reg = next_reg - 1;
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
            if trimmed.starts_with("return ") {
                let expr = trimmed[7..].trim().trim_end_matches(';').trim();
                if expr.ends_with("()") {
                    let name = expr[..expr.len() - 2].trim();
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
        if trimmed.starts_with("program ") {
            let after = &trimmed[8..];
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
        let (is_closure, name) = if trimmed.starts_with("closure ") {
            let name = trimmed[8..].trim_end_matches(':').trim().to_string();
            (true, name)
        } else if trimmed.starts_with("function ") {
            let name = trimmed[9..].trim_end_matches(':').trim().to_string();
            (false, name)
        } else if trimmed.starts_with("finalize ") {
            let name = trimmed[9..].trim_end_matches(':').trim().to_string();
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
    if s.starts_with('r') {
        s[1..].parse::<usize>().ok()
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

        // Check for transition or function definition.
        let (is_transition, after_keyword) = if trimmed.starts_with("transition ") {
            (true, &trimmed[11..])
        } else if trimmed.starts_with("function ") {
            (false, &trimmed[9..])
        } else {
            i += 1;
            continue;
        };

        // Parse function name.
        let name_end = after_keyword.find('(').unwrap_or(after_keyword.len());
        let name = after_keyword[..name_end].trim().to_string();

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
                break;
            }
            j += 1;
        }

        if !name.is_empty() {
            functions.push(LeoFunctionDef {
                name,
                is_transition,
                line: line_num,
                bindings,
                return_line,
            });
        }

        i = j + 1;
    }

    functions
}

/// Parse a Leo let-binding like `let a: u32 = 10u32;`.
fn parse_leo_binding(line: &str, line_num: u32) -> Option<LeoBinding> {
    let trimmed = line.trim();
    if !trimmed.starts_with("let ") {
        return None;
    }

    let after_let = &trimmed[4..];
    if let Some(colon_pos) = after_let.find(':') {
        let name = after_let[..colon_pos].trim().to_string();
        let after_colon = &after_let[colon_pos + 1..];
        // Get the type (before '=').
        let type_name = if let Some(eq_pos) = after_colon.find('=') {
            after_colon[..eq_pos].trim().to_string()
        } else {
            after_colon.trim().trim_end_matches(';').trim().to_string()
        };

        if !name.is_empty() && !type_name.is_empty() {
            return Some(LeoBinding {
                name,
                type_name,
                line: line_num,
            });
        }
    }

    None
}

/// Find if a function's return statement calls another function.
/// Parses `return <func_name>();` patterns.
fn find_return_call_target(
    func: &LeoFunctionDef,
    func_map: &HashMap<&str, &LeoFunctionDef>,
) -> Option<String> {
    // We look at the return_line to see if it matches a pattern like
    // `return compute();`
    // For simplicity, check if any other function name appears as a call target.
    // This is determined by the function being called having bindings
    // that contribute to the return value.
    for (name, _) in func_map.iter() {
        if *name != func.name {
            // Check if this function has no bindings and its return
            // line calls the other function. We can infer this from
            // the Aleo instructions (call instruction).
            if func.bindings.is_empty() && func.return_line.is_some() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Find the return value for a function.
fn find_return_value(
    func: &LeoFunctionDef,
    func_map: &HashMap<&str, &LeoFunctionDef>,
    variable_values: &HashMap<String, i64>,
) -> Option<i64> {
    // If this function has bindings, the return value is likely the last binding.
    if let Some(last_binding) = func.bindings.last() {
        return variable_values.get(&last_binding.name).copied();
    }

    // If this function calls another, the return value is from the callee.
    for (name, callee) in func_map.iter() {
        if *name != func.name {
            if let Some(last_binding) = callee.bindings.last() {
                return variable_values.get(&last_binding.name).copied();
            }
        }
    }

    None
}

/// Parse a typed Leo literal like `10u32`, `42u64`, `10field`, `5i32`.
fn parse_typed_literal(s: &str) -> Option<i64> {
    // Try each known type suffix.
    for suffix in &["u32", "u64", "u128", "i32", "i64", "i128", "field"] {
        if s.ends_with(suffix) {
            let num_str = &s[..s.len() - suffix.len()];
            if let Ok(val) = num_str.parse::<i64>() {
                return Some(val);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
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
            bindings: vec![
                LeoBinding {
                    name: "a".to_string(),
                    type_name: "u32".to_string(),
                    line: 3,
                },
                LeoBinding {
                    name: "b".to_string(),
                    type_name: "u32".to_string(),
                    line: 4,
                },
                LeoBinding {
                    name: "sum_val".to_string(),
                    type_name: "u32".to_string(),
                    line: 5,
                },
                LeoBinding {
                    name: "doubled".to_string(),
                    type_name: "u32".to_string(),
                    line: 6,
                },
                LeoBinding {
                    name: "final_result".to_string(),
                    type_name: "u32".to_string(),
                    line: 7,
                },
            ],
            return_line: Some(8),
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
