# Leo / Aleo Recorder — CTFS Audit (2026-05)

This memo records the CTFS audit of `codetracer-leo-recorder` against
the canonical pipeline and the audit checklist in section 5.6 of
`/tmp/isonim-migration.txt`.  The Leo recorder is the SIXTEENTH
recorder audited (after Ruby, Python, JavaScript, EVM, PHP, Solana,
Move, Cardano, Cairo, Flow/Cadence, Fuel/Sway, PolkaVM, Miden,
TON/Tolk, and Circom — entries 1.21–1.58).

## Architecture

Single-process Rust crate.  Two CLI subcommands:

  * `record <source.leo>` — compiles the Leo source via the upstream
    `leo` CLI (or, when unavailable, via an in-crate Leo→Aleo
    fallback generator), parses the generated Aleo Instructions, runs
    them through a built-in AVM interpreter, and emits canonical
    CodeTracer events through the Rust-native `NimTraceWriter`
    (`codetracer_trace_writer_nim` sibling-path dep).
  * `replay --program-id ... --function ... --input ...` — fetches an
    on-chain Aleo program via the snarkOS REST API (or `ALEO_RPC_MOCK`
    fixtures), parses its Aleo Instructions, executes the requested
    function in the AVM interpreter, and writes a trace.

Not an FFI consumer — every canonical TraceWriter entry point is
reachable.  Like Circom 1.58, this is a zk-rollup-domain recorder
(Aleo programs are SNARK-proven on the Aleo L1).  Unlike Circom, Leo
DOES have a stack-machine layer (the AVM) and is closer in shape to
TON/Tolk (1.57) and Move (1.46) than to Circom (1.58):

  * Operand layer: Aleo register file (`r0`, `r1`, …), not a stack.
  * Call boundary: Leo `transition` and `function` declarations
    compile to Aleo `function` and `closure` definitions; both are
    canonical "Function call" boundaries for `register_call` /
    `register_return`.
  * Source-level parameter list: yes (Leo source has `transition
    foo(a: u32, b: u32) -> u32 { ... }`).  This is staged as
    `TraceWriter::arg(param_name, NONE_VALUE)` per declared
    parameter, mirroring the TON 1.57 `func.params` and Circom 1.58
    `signal input` shapes.

## Findings vs. section 5.6 checklist

### (a) default-Ctfs CLI — GAP CLOSED

Pre-fix: `record` and `replay` exposed an `OutputFormat { Binary,
Json }` enum with `Binary` (legacy CBOR + Zstd) as the default; the
canonical multi-stream CTFS container — the format the Nim
`ct_reader_*` FFI and the db-backend's `CTFSTraceReader` consume
directly — was not selectable from the CLI at all.

Post-fix: typed `clap::ValueEnum OutputFormat { Ctfs, Binary, Json }`
with `Ctfs` listed first and `default_value = "ctfs"` for both
`RecordArgs.format` and `ReplayArgs.format`.  An `impl From<OutputFormat>
for TraceEventsFileFormat` collapses each dispatch site to
`args.format.into()`.  An `OutputFormat::as_str` helper
(`#[allow(dead_code)]`) is wired for future `trace_metadata.json`
`format` field emission, mirroring the Fuel 1.53 / PolkaVM 1.55 /
Miden 1.56 / TON 1.57 / Circom 1.58 pattern.

Verified post-fix by `tests/test_ctfs_audit.rs::ctfs_writer_produces_
ct_container`, `ctfs_format_advertised_in_record_help`, and
`ctfs_is_the_default_record_format`.

### (b) `register_call` for each call — OK (with caveat)

The recorder calls `TraceWriter::register_call` at every transition
boundary: once in `tracer.rs::emit_function_trace` (for non-entry-
point Leo functions, after parameter staging) and once in
`replay.rs::replay_deployed_program` (for the replay subcommand's
entry function).  The main transition's body is intentionally merged
into `<toplevel>` (so its body lives at depth 0 in the calltrace
pane), matching the Circom 1.58 main-template pattern.

Caveat: nested call detection in `tracer.rs::find_return_call_target`
is heuristic — it assumes a function whose body is empty AND whose
return line exists must call SOME other function in the program.
This works for the existing two-function fixtures but will break on
programs with three or more transitions where multiple are pure call-
forwards.  The replacement is a real AST-based call-graph extraction;
documented as an open follow-up.

### (c) Call args via `TraceWriter::arg` — GAP CLOSED (declared names)

Pre-fix: every `register_call` site used `vec![]` (no args) and there
was no `writer.arg(...)` staging anywhere in the recorder, so the
calltrace pane's `.call-arg` rows were always empty.

Post-fix:

  * `tracer.rs`: `LeoFunctionDef` gains a `parameters: Vec<String>`
    field; `parse_leo_functions` extracts the parameter list from the
    source `transition <name>(<params>) -> ...` / `function
    <name>(<params>) -> ...` syntax.  Multi-line parameter lists are
    tolerated.  A new `parse_leo_parameter_list` helper extracts each
    parameter name from a comma-separated `name: type` body and
    correctly skips visibility-mode keywords (`public`, `private`,
    `constant`, `const`, `mut`).  `emit_function_trace` iterates
    `func.parameters` and stages each through
    `TraceWriter::arg(param_name, NONE_VALUE)` immediately before
    `register_call` for every non-entry-point function.

  * `replay.rs`: `replay_deployed_program` stages the live input
    register set on the Aleo function being entered.  The argument
    NAME is the register identifier (`r0`, `r1`, …) and the VALUE is
    the live `input_values[i]` pushed by the caller (resolved
    through the typed-literal parser to a `ValueRecord::Int { i,
    type_id }` matching the parsed input type).  This mirrors the
    Miden 1.56 operand-stack staging (`s0..s3`) and the PolkaVM 1.55
    Ecalli A0..A5 staging.

Live arg values on the recorder side (Leo source, not replay):

  * The Leo→Aleo register binding map is not yet threaded back to
    the source-level parameter names, so `tracer.rs` stages
    `NONE_VALUE` (declared name only).  Same shape as Circom 1.58
    declared-input-signal staging and TON 1.57 declared-func.params
    staging — those recorders also stage names without live values
    pending a parser extension.

  * The infrastructure to resolve them exists: each Aleo function
    that the parser produces has `inputs: Vec<(usize, String)>`
    (register index + type) and `execution_results` already
    contains every register's final value.  The remaining work is
    to look up the matching `AleoFunction` for each
    `LeoFunctionDef` (which the recorder already does in
    `find_matching_aleo_function` for `map_registers_to_variables`)
    and use the inputs list to convert each `LeoFunctionDef.parameters[i]`
    name into a live `ValueRecord::Int { i: input_value, type_id }`.
    Tracked in the open-follow-ups section below.

Verified post-fix by:

  * `src/tracer.rs` unit tests: `test_parse_leo_parameter_list_*`
    (5 cases for the parameter-list parser),
    `test_parse_leo_functions_captures_parameters` (end-to-end
    parser test with `transition transfer(receiver: address,
    amount: u64)`).
  * `tests/test_ctfs_audit.rs::ctfs_reader_sees_staged_call_args`
    (read-side CTFS assertion that a parameterised transition's
    staged names appear in `CallRecord.args` as `a` / `b` with the
    canonical `NONE_VALUE` placeholder).

### (d) `Write` / `Error` / `EvmEvent` special-event routing — OPEN

  * **Write**: N/A.  The Leo→Aleo→AVM pipeline has no native
    stdout / stderr; the recorder's `eprintln!` calls are diagnostics
    on the recorder's own stderr, not on the traced program's.  The
    Aleo program model has no `print` opcode (unlike Cairo / Cadence
    / EVM `console_log`).  Same shape as Circom 1.58, Tolk 1.57,
    Miden 1.56, Cairo 1.50, Cardano 1.48.

  * **Error**: PARTIAL.  Three error paths can fail today:

    1. `compile_leo_to_aleo_via_cli` — `leo build` failure (compiler
       error, missing project layout, bad `.env`).  Falls back to
       the in-crate `generate_aleo_from_leo_source` generator, but
       that path itself can fail on unsupported syntax.  This final
       compile/generation failure path is now routed through
       `register_special_event(EventLogKind::Error,
       "leo_compile_error", msg)` before the recorder returns the
       error.  The partial trace is finalised, and
       `tests/test_ctfs_audit.rs::ctfs_reader_sees_leo_compile_error_event`
       opens it through `NimTraceReaderHandle` and asserts that a CTFS
       `error` event with the failure message is readable.
    2. AVM interpreter (`tracer.rs::execute_function`) — division
       by zero, modulo by zero, unknown function name in `call`,
       recursion overflow (no recursion check today).
    3. `replay.rs::fetch_program` — RPC fetch failure (curl
       non-zero, empty response, malformed UTF-8).

    The AVM and RPC paths still bubble up via `?` before a debuggable
    Error event is finalised.  Concrete remaining fix shape (mirrors
    Cardano 1.48 / Flow 1.52 / Tolk 1.57 closed pattern): route
    AVM/RPC failures through `register_special_event(EventLogKind::Error,
    "<kind>", msg)` with metadata `"avm_runtime_error"` /
    `"aleo_rpc_error"`, and finalise the partial trace via the existing
    `finish_writing_*` calls.  The current CTFS reader projection exposes
    the Error kind and message bytes, but collapses the metadata string
    (`leo_compile_error`) into the known multi-stream/event projection
    boundary documented below.

  * **EvmEvent**: OPEN (two distinct cases).

    1. **Mapping operations**.  Aleo's on-chain mapping commands
       (`get`, `get.or_use`, `set`, `remove`, `contains`) are
       structured state-mutation events analogous to EVM `SSTORE`
       / `SLOAD` and Cairo `StorageWrite` / `StorageRead`.  The
       recorder's AVM interpreter handles them functionally
       (`MappingStore`) but does not surface them as
       `EventLogKind::EvmEvent` records.  Concrete fix shape:
       inside each `AleoInstruction::Mapping{Get,GetOrUse,Set,
       Remove,Contains}` branch in `execute_function` (or in a
       new `FinalizeTracer`-driven pass), emit
       `register_special_event(EventLogKind::EvmEvent,
       "aleo_mapping_<op>", "<mapping>[<key>] = <value>")` so the
       calltrace / event-log panes can render each mapping mutation
       as a structured event.

    2. **Output records / `Future::Async`**.  Aleo `function`
       declarations emit `output ... as record` and `output ... as
       <future>` lines.  Records are private value containers;
       futures are deferred-finalize handles that move the on-
       chain mapping mutations from the proving phase to the
       consensus phase.  Both are structured-event analogues of EVM
       `LOG` opcodes and Cairo `StarknetEvent` / `Cadence event` /
       Fuel `LogData`.  Concrete fix shape: at the end of
       `execute_function`, walk the function's `outputs` list and
       emit `register_special_event(EventLogKind::EvmEvent,
       "aleo_record_output" or "aleo_future_async",
       "<reg>=<value>")` per output.

### (e) Thread events — N/A

Aleo program execution is single-threaded by construction (the AVM
interpreter is a sequential register machine, the witness generator
is deterministic, and the on-chain `finalize` block runs serially in
each block's transition list).  No `register_thread_*` calls needed.
Same status as Circom 1.58, Tolk 1.57, Miden 1.56, all stack-VM
recorders.

### (f) Step records — OK

`tracer.rs::emit_function_trace` calls `TraceWriter::register_step`
on each Leo source binding (using the `AleoSourceMap` to translate
the Aleo instruction position back to its Leo source line) and on
the return line.  `replay.rs::replay_deployed_program` calls
`register_step` on each instruction line in the executed function.

Granularity is "source-line per let-binding" today.  Per-Aleo-
instruction stepping (the analogue of Miden 1.56's `execute_iter`
iterator) is a future enhancement — would require driving the AVM
interpreter from a step-by-step iterator and using `AleoSourceMap`
to translate each instruction back to a Leo source line.  Tracked as
a follow-up below.

### (g) CTFS schema match — GAP CLOSED via (a)

The pre-fix `tests/test_tracer.rs::test_leo_cli_record` asserted
CTFS magic on a file produced under `--format json`, which worked
only because the Nim writer happened to treat Json's writer-side
dispatch identically to Ctfs for the multi-stream container path.
Post-fix the test passes `--format ctfs` explicitly so any future
divergence between Json and Ctfs in the Nim writer cannot silently
break the recorder.  The same fix is applied to
`run_tracer_on_file` (now passes `TraceEventsFileFormat::Ctfs`).

### (h) Obsolete `add_event` calls — OK

No `add_event` references in `src/`.  Every event is emitted through
the dedicated entry points (`register_call`, `register_return`,
`register_step`, `register_variable_with_full_value`,
`ensure_function_id`, `ensure_type_id`, `arg`).

### (i) `#[no_mangle]` stubs — OK

The recorder uses the Rust API directly via the
`codetracer_trace_writer_nim` crate.  No C FFI exposure, no
`#[no_mangle]` stubs.

## Concrete fixes applied

  1. **`src/main.rs`** — `OutputFormat` gains a `Ctfs` variant
     (listed first) with doc-comments on each option;
     `default_value = "ctfs"` for `RecordArgs.format` and
     `ReplayArgs.format`; `impl From<OutputFormat> for
     TraceEventsFileFormat` collapses both dispatch sites to
     `args.format.into()`.  Adds `OutputFormat::as_str` helper
     (`#[allow(dead_code)]`) for future `trace_metadata.json`
     `format` field emission.

  2. **`src/tracer.rs`** — `LeoFunctionDef` gains a `parameters:
     Vec<String>` field; `parse_leo_functions` is extended to scan
     the parameter list with multi-line tolerance and call a new
     `parse_leo_parameter_list` helper that extracts each
     parameter name (skipping leading visibility / mode keywords).
     `emit_function_trace` stages each `func.parameters` entry
     through `TraceWriter::arg(name, NONE_VALUE)` immediately
     before `register_call` for every non-entry-point function.

  3. **`src/replay.rs`** — `replay_deployed_program` stages each
     of the target Aleo function's input registers via
     `TraceWriter::arg(format!("r{}", reg_idx), live_value)`
     before `register_call`.  The argument's `type_id` is
     resolved from the parsed type name (with `u32` as the
     fallback for unknown types), so live integer values reach
     the calltrace pane's `.call-arg` rows.

  4. **`tests/test_tracer.rs`** — both helpers
     (`run_tracer_on_file` and `test_leo_cli_record`) now use the
     `Ctfs` format explicitly so any eventual divergence between
     `Json` and `Ctfs` in the Nim writer cannot silently break
     the recorder.

  5. **`tests/test_ctfs_audit.rs`** — five audit tests:
     `ctfs_writer_produces_ct_container`,
     `ctfs_format_advertised_in_record_help`,
     `ctfs_is_the_default_record_format`,
     `ctfs_reader_sees_staged_call_args`,
     `ctfs_reader_sees_leo_compile_error_event`.

  6. **`src/tracer.rs` unit tests** — six new tests for the
     parameter-list parser plus an end-to-end
     `test_parse_leo_functions_captures_parameters` test;
     existing `test_map_registers_to_variables` updated to set
     the new `parameters: vec![]` field.

  7. **`src/tracer.rs` compile-error routing** — the record path now
     catches final Leo compile/fallback-generation failures, writes a
     partial trace with `register_special_event(EventLogKind::Error,
     "leo_compile_error", msg)`, closes the implicit `<toplevel>` call,
     finalises the CTFS container, then returns the original error.
     The fallback generator now returns a concrete error when no Leo
     `transition` or `function` declarations are found instead of
     silently producing an empty Aleo program.

## Open follow-ups (not blocking; documented for future work)

### Live argument values for source-level transition calls (audit (c))

Today the source-side staging passes `NONE_VALUE` because the Leo
parser does not resolve call-site argument expressions back to the
called function's parameter list.  Concrete fix shape:

  1. Look up the matching `AleoFunction` for each
     `LeoFunctionDef` (already done by
     `find_matching_aleo_function` for
     `map_registers_to_variables`).
  2. Use `aleo_fn.inputs` to pair `LeoFunctionDef.parameters[i]`
     with the input register at index `i`.
  3. Look up the live value of that register in
     `execution_results[fn_name].registers[reg_idx]`.
  4. Stage `TraceWriter::arg(param_name, ValueRecord::Int { i:
     val, type_id })` instead of `arg(param_name, NONE_VALUE)`.

Same shape as the Circom 1.58 "per-instance input-signal value
resolution" follow-up and the TON 1.57 "Tolk parser extension for
arg-passing call sites" follow-up.

### Compile / AVM / RPC error routing (audit (d), Error) — PARTIAL

Compile/generation errors are now routed through
`register_special_event(EventLogKind::Error, "leo_compile_error", msg)`
before returning the recorder error, and the partial CTFS trace is
finalised.  `tests/test_ctfs_audit.rs::ctfs_reader_sees_leo_compile_error_event`
records invalid Leo source, opens the `.ct` through
`NimTraceReaderHandle`, and asserts a readable CTFS `error` event whose
message contains the compile/generation failure.  The reader-visible IO
event projection exposes kind + data bytes but not the original metadata
string, so metadata readback remains covered by the known multi-stream
projection boundary below.

AVM runtime failures and Aleo RPC fetch failures remain open.  Their next
layer fix is the same finalise-on-error helper shape, with
`"avm_runtime_error"` and `"aleo_rpc_error"` metadata respectively.

### Mapping operations as structured events (audit (d), EvmEvent)

In each `AleoInstruction::Mapping{Get,GetOrUse,Set,Remove,Contains}`
branch in `execute_function`, emit
`register_special_event(EventLogKind::EvmEvent, "aleo_mapping_<op>",
"<mapping>[<key>] = <value>")` after the functional operation.  This
is the Aleo analogue of EVM `SSTORE` / `SLOAD` and Cairo
`StorageWrite` / `StorageRead`.

### Output records and `Future::Async` as structured events (audit (d), EvmEvent)

Walk the function's `outputs` list at the end of `execute_function`
and emit `register_special_event(EventLogKind::EvmEvent,
"aleo_record_output" or "aleo_future_async", "<reg>=<value>")` per
output — this is the Aleo analogue of EVM `LOG` opcodes, Cairo
`StarknetEvent`, Cadence `event`, and Fuel `LogData`.

### Per-Aleo-instruction step emission

The existing `FinalizeTracer` (in `src/finalize.rs`) already
implements the per-instruction tracing pass for finalize blocks.
Generalise it to the main-execution path (currently only
`LeoTracer::emit_function_trace` is wired into `record`) and use
the `AleoSourceMap` to translate each Aleo instruction back to its
Leo source line, parallel to Miden 1.56's `execute_iter` step-by-
step iterator.

### Real Leo→Aleo CLI integration into `ct record`

The Leo recorder is not yet wired into the codetracer monorepo's
`ct record` command (similar gating to TON 1.57 `tolk` pipeline,
Circom 1.58 circom pipeline, and PolkaVM 1.55 polkatool pipeline).
Once wired, the targeted Playwright spec
`src/tests/gui/tests/program_specific_tests/leo_example.spec.ts`
can exercise the recorder-side fixes end-to-end.

### Reader-side end-to-end content assertions — CLOSED for source-level args

`tests/test_ctfs_audit.rs::ctfs_reader_sees_staged_call_args` now
opens the produced `.ct` container through `NimTraceReaderHandle`,
walks `CallRecord` JSON, resolves each arg `varname_id`, and asserts
that the parameterised source-level transition carries `a` / `b` in
`CallRecord.args` with the canonical `NONE_VALUE` placeholder.  Replay-
path live register arg readback remains a separate optional assertion;
the recorder semantics were already covered by 1.59 and this follow-up
only closes the source-level read-side assertion gap.

### Multi-stream IO event collapse

Cross-cutting issue documented in 1.39 / 1.41 / 1.44 / 1.46 / 1.48 /
1.50 / 1.52 / 1.53 / 1.55 / 1.56 / 1.57 / 1.58.  Once the recorder
emits Error / EvmEvent records (open gaps above), they will collapse
onto stderr and lose their metadata in the multi-stream pane.  Out
of scope for any single recorder audit.

### AST-based call-graph extraction (replaces heuristic)

Replace the heuristic in `tracer.rs::find_return_call_target` with a
real AST-based call-graph extraction so programs with three or more
transitions trace correctly.  Track each `return <callee>(<args>)`
site explicitly and follow the call edge.

## Cross-cutting findings — Leo / Aleo specifics

The Leo recorder shares structure with both stack-VM recorders
(TON 1.57, Move 1.46, Miden 1.56) and zk-rollup recorders (Circom
1.58):

  * **Like stack-VM recorders**: there is a real operand layer (the
    AVM register file), `register_call` / `register_return` map to
    function/closure boundaries, `register_step` granularity can
    drop to per-instruction.
  * **Like zk-rollup recorders**: the program is proven, not just
    executed; output values are `record` / `future` types
    representing on-chain state effects rather than direct mutation;
    constraint-violation errors are a class distinct from the
    runtime error class.

The SOURCE-level parameter list IS available (Leo's `transition foo(a:
u32) -> u32 { ... }` syntax), so audit (c) staging is by declared
parameter NAME (mirroring TON 1.57 `func.params` and Circom 1.58
`signal input` patterns), with live values pending a one-step parser
extension.  The COMPILED Aleo level has typed input registers
(`input rN as u32.private`) so the replay-side staging uses live
values from the start.

Future zk-rollup-language recorder audits (Halo2-based circuits,
Plonk / Plonky2 frontends, Risc0 / SP1 zkVMs) should expect this
hybrid shape and stage call args at BOTH the source-language level
(declared parameter names with NONE_VALUE) AND the compiled-bytecode
level (live register values), choosing the layer that matches the
trace-emission boundary.
