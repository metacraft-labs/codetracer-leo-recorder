//! On-chain program replay via Aleo RPC (M7 milestone).
//!
//! Provides infrastructure to fetch deployed Aleo programs from the network
//! and replay their execution locally through the AVM interpreter, producing
//! CodeTracer trace output.
//!
//! The replay pipeline:
//!   1. Fetch the deployed program source (Aleo Instructions) from an Aleo node
//!      via its REST API (`/testnet/program/{program_id}`).
//!   2. Parse the Aleo instructions using the existing AVM parser.
//!   3. Execute the specified function with the given inputs.
//!   4. Trace the execution with source mapping and write CodeTracer output.

use std::collections::HashMap;
use std::path::Path;

use codetracer_trace_types::{Line, TypeKind, ValueRecord, NONE_VALUE};
use codetracer_trace_writer_nim::trace_writer::TraceWriter;
use codetracer_trace_writer_nim::{create_trace_writer, TraceEventsFileFormat};
use eyre::{eyre, Context, Result};

use crate::tracer::{
    parse_aleo_program, resolve_operand, AleoFunction, AleoInstruction, FunctionResult,
    MappingStore, Operand,
};

// ---------------------------------------------------------------------------
// RPC client
// ---------------------------------------------------------------------------

/// Client for the Aleo node REST API.
///
/// Connects to a snarkOS node's REST endpoint to fetch deployed programs
/// and other on-chain data.
pub struct AleoRpcClient {
    /// The base URL of the Aleo node REST API (e.g. `https://api.explorer.aleo.org/v1`).
    pub endpoint: String,
}

impl AleoRpcClient {
    /// Create a new RPC client pointing at the given endpoint.
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for replaying a deployed Aleo program.
pub struct ReplayConfig {
    /// The on-chain program ID (e.g. `credits.aleo`).
    pub program_id: String,
    /// The function to execute within the program.
    pub function_name: String,
    /// Input values for the function (as string literals, e.g. `"10u32"`).
    pub inputs: Vec<String>,
    /// The Aleo node REST API endpoint.
    pub endpoint: String,
}

// ---------------------------------------------------------------------------
// Deployed program
// ---------------------------------------------------------------------------

/// A deployed Aleo program fetched from the network.
pub struct DeployedProgram {
    /// The program ID (e.g. `credits.aleo`).
    pub program_id: String,
    /// The Aleo Instructions source text.
    pub source: String,
}

// ---------------------------------------------------------------------------
// Program fetching
// ---------------------------------------------------------------------------

/// Fetch a deployed program's source from the Aleo network.
///
/// Calls the node's REST API at `GET {endpoint}/testnet/program/{program_id}`
/// to retrieve the Aleo Instructions text of a deployed program.
///
/// In test/development mode (when the `ALEO_RPC_MOCK` environment variable
/// is set), returns mock program data instead of making a real HTTP request.
pub fn fetch_program(client: &AleoRpcClient, program_id: &str) -> Result<DeployedProgram> {
    // Check for mock mode (used in tests and development).
    if std::env::var("ALEO_RPC_MOCK").is_ok() {
        return fetch_mock_program(program_id);
    }

    let url = format!("{}/testnet/program/{}", client.endpoint, program_id);

    // Use a simple blocking HTTP request via std.
    // In production this would use reqwest or similar, but to avoid adding
    // a heavy dependency we shell out to curl for now.
    let output = std::process::Command::new("curl")
        .args(["-s", "-f", &url])
        .output()
        .with_context(|| format!("failed to fetch program from {url}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!(
            "failed to fetch program {program_id} from {url}: {stderr}"
        ));
    }

    let source =
        String::from_utf8(output.stdout).with_context(|| "program source is not valid UTF-8")?;

    if source.trim().is_empty() {
        return Err(eyre!("empty response for program {program_id} from {url}"));
    }

    Ok(DeployedProgram {
        program_id: program_id.to_string(),
        source,
    })
}

/// Return mock program data for testing.
///
/// Provides a small set of well-known program IDs with hardcoded Aleo
/// Instructions source, so tests can run without a live network.
fn fetch_mock_program(program_id: &str) -> Result<DeployedProgram> {
    let source = match program_id {
        "hello.aleo" => MOCK_HELLO_PROGRAM,
        "arithmetic.aleo" => MOCK_ARITHMETIC_PROGRAM,
        _ => {
            return Err(eyre!(
                "mock program not found: {program_id} (set ALEO_RPC_MOCK=1 for mock mode)"
            ));
        }
    };

    Ok(DeployedProgram {
        program_id: program_id.to_string(),
        source: source.to_string(),
    })
}

/// Mock: a simple hello/addition program.
const MOCK_HELLO_PROGRAM: &str = r#"program hello.aleo;

function main:
    input r0 as u32.public;
    input r1 as u32.public;
    add r0 r1 into r2;
    output r2 as u32.public;
"#;

/// Mock: an arithmetic program with multiple operations.
const MOCK_ARITHMETIC_PROGRAM: &str = r#"program arithmetic.aleo;

function compute:
    input r0 as u32.public;
    input r1 as u32.public;
    add r0 r1 into r2;
    mul r2 2u32 into r3;
    sub r3 r0 into r4;
    output r4 as u32.public;

function main:
    input r0 as u32.public;
    input r1 as u32.public;
    call compute r0 r1 into r2;
    output r2 as u32.public;
"#;

// ---------------------------------------------------------------------------
// Input parsing
// ---------------------------------------------------------------------------

/// Parse a string input value into an i64.
///
/// Accepts Aleo-style typed literals like `10u32`, `42u64`, `5i32`,
/// or plain integers like `10`.
fn parse_input_value(input: &str) -> Result<i64> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(eyre!("empty input value"));
    }

    // Strip Aleo type suffix if present (e.g. "10u32" -> "10", "-5i64" -> "-5").
    // Type suffixes match the pattern: [ui](8|16|32|64|128) or "field", "scalar", etc.
    let numeric = if let Some(pos) = trimmed.rfind(['u', 'i']) {
        // Check if everything after 'u'/'i' is digits (a type suffix).
        let suffix = &trimmed[pos + 1..];
        let prefix = &trimmed[..pos];
        if !prefix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) && !suffix.is_empty() {
            prefix
        } else {
            trimmed
        }
    } else {
        // Also handle "field", "scalar", "group" suffixes.
        let suffixes = ["field", "scalar", "group"];
        let mut result = trimmed;
        for s in &suffixes {
            if let Some(stripped) = trimmed.strip_suffix(s) {
                if !stripped.is_empty() {
                    result = stripped;
                    break;
                }
            }
        }
        result
    };

    numeric
        .parse::<i64>()
        .with_context(|| format!("invalid input value: {trimmed}"))
}

// ---------------------------------------------------------------------------
// Replay execution
// ---------------------------------------------------------------------------

/// Replay a deployed Aleo program's execution and produce a CodeTracer trace.
///
/// This is the main entry point for the M7 replay pipeline:
/// 1. Fetches the program from the Aleo network via RPC.
/// 2. Parses the Aleo Instructions source.
/// 3. Executes the specified function through the AVM interpreter.
/// 4. Writes CodeTracer trace output files to `out_dir`.
pub fn replay_program(
    config: &ReplayConfig,
    out_dir: &Path,
    format: TraceEventsFileFormat,
) -> Result<()> {
    // 1. Fetch the deployed program.
    let client = AleoRpcClient::new(&config.endpoint);
    let deployed = fetch_program(&client, &config.program_id)
        .with_context(|| format!("failed to fetch program {}", config.program_id))?;

    replay_deployed_program(&deployed, config, out_dir, format)
}

/// Replay a pre-fetched deployed program (used internally and in tests).
pub fn replay_deployed_program(
    deployed: &DeployedProgram,
    config: &ReplayConfig,
    out_dir: &Path,
    format: TraceEventsFileFormat,
) -> Result<()> {
    eprintln!(
        "Fetched program {} ({} bytes of Aleo Instructions)",
        deployed.program_id,
        deployed.source.len()
    );

    // 2. Parse the Aleo instructions.
    let functions = parse_aleo_program(&deployed.source);

    eprintln!(
        "Parsed {} functions from {}",
        functions.len(),
        deployed.program_id
    );

    // Find the target function.
    let func_map: HashMap<&str, &AleoFunction> =
        functions.iter().map(|f| (f.name.as_str(), f)).collect();

    if !func_map.contains_key(config.function_name.as_str()) {
        let available: Vec<&str> = func_map.keys().copied().collect();
        return Err(eyre!(
            "function '{}' not found in program {}. Available: {:?}",
            config.function_name,
            config.program_id,
            available
        ));
    }

    // 3. Parse input values.
    let input_values: Vec<i64> = config
        .inputs
        .iter()
        .map(|s| parse_input_value(s))
        .collect::<Result<Vec<_>>>()?;

    // 4. Execute the function.
    let mut results: HashMap<String, FunctionResult> = HashMap::new();
    let mut mapping_store = MappingStore::new();

    execute_function_for_replay(
        func_map[config.function_name.as_str()],
        &func_map,
        &input_values,
        &mut results,
        &mut mapping_store,
    )?;

    eprintln!(
        "Executed function '{}', got {} function results",
        config.function_name,
        results.len()
    );

    // 5. Write the trace output.
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    // Use the program source as a synthetic source file for tracing.
    let source_filename = format!("{}.aleo", config.program_id.trim_end_matches(".aleo"));
    let source_path = out_dir.join(&source_filename);
    std::fs::write(&source_path, &deployed.source)
        .with_context(|| format!("failed to write source file: {}", source_path.display()))?;

    // Create trace writer.
    let program_str = source_path.to_string_lossy().to_string();
    let mut writer = create_trace_writer(&program_str, &[], format);

    let events_filename = match format {
        TraceEventsFileFormat::Json => "trace.json",
        TraceEventsFileFormat::Binary
        | TraceEventsFileFormat::BinaryV0
        | TraceEventsFileFormat::Ctfs => "trace.bin",
    };
    let events_path = out_dir.join(events_filename);
    let metadata_path = out_dir.join("trace_metadata.json");
    let paths_path = out_dir.join("trace_paths.json");

    TraceWriter::begin_writing_trace_events(&mut *writer, &events_path)
        .map_err(|e| eyre!("{e}"))?;
    TraceWriter::begin_writing_trace_metadata(&mut *writer, &metadata_path)
        .map_err(|e| eyre!("{e}"))?;
    TraceWriter::begin_writing_trace_paths(&mut *writer, &paths_path).map_err(|e| eyre!("{e}"))?;

    TraceWriter::start(&mut *writer, &source_path, Line(1));

    // Register common types.
    let mut type_ids = HashMap::new();
    for type_name in &["u32", "u64", "i32", "i64", "field", "bool"] {
        let type_id = TraceWriter::ensure_type_id(&mut *writer, TypeKind::Int, type_name);
        type_ids.insert(type_name.to_string(), type_id);
    }

    let u32_type_id = type_ids["u32"];

    // Walk through the Aleo source lines to find function/instruction positions.
    let source_lines: Vec<&str> = deployed.source.lines().collect();

    // Find the function's start line in the source.
    let func_start_line = source_lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            let trimmed = line.trim();
            trimmed.starts_with("function ")
                && trimmed
                    .strip_prefix("function ")
                    .map(|rest| rest.trim_end_matches(':').trim() == config.function_name)
                    .unwrap_or(false)
        })
        .map(|(i, _)| (i + 1) as i64) // 1-based
        .unwrap_or(1);

    // Step into the function.
    let fn_id = TraceWriter::ensure_function_id(
        &mut *writer,
        &config.function_name,
        &source_path,
        Line(func_start_line),
    );
    TraceWriter::register_call(&mut *writer, fn_id, vec![]);

    // Emit value events for registers from the target function's result.
    if let Some(result) = results.get(&config.function_name) {
        let mut reg_indices: Vec<usize> = result.registers.keys().copied().collect();
        reg_indices.sort();

        // Find instruction lines within the function (lines after the function
        // declaration that contain instructions, not input/output declarations).
        for (i, &reg_idx) in reg_indices.iter().enumerate() {
            let instr_line = func_start_line + 1 + i as i64;
            let value = result.registers[&reg_idx];

            // Step to the instruction line.
            TraceWriter::register_step(&mut *writer, &source_path, Line(instr_line));

            // Emit the register value.
            let var_name = format!("r{}", reg_idx);
            let value_record = ValueRecord::Int {
                i: value,
                type_id: u32_type_id,
            };
            TraceWriter::register_variable_with_full_value(&mut *writer, &var_name, value_record);
        }
    }

    // Return from the function.
    TraceWriter::register_return(&mut *writer, NONE_VALUE);

    // Finish writing.
    TraceWriter::finish_writing_trace_events(&mut *writer).map_err(|e| eyre!("{e}"))?;
    TraceWriter::finish_writing_trace_metadata(&mut *writer).map_err(|e| eyre!("{e}"))?;
    TraceWriter::finish_writing_trace_paths(&mut *writer).map_err(|e| eyre!("{e}"))?;
    writer.close().map_err(|e| eyre!("{e}"))?;

    eprintln!("Trace files written to {}", out_dir.display());

    Ok(())
}

/// Execute a single Aleo function for replay, recursively handling calls.
fn execute_function_for_replay(
    func: &AleoFunction,
    func_map: &HashMap<&str, &AleoFunction>,
    input_values: &[i64],
    results: &mut HashMap<String, FunctionResult>,
    mapping_store: &mut MappingStore,
) -> Result<FunctionResult> {
    let mut registers: HashMap<usize, i64> = HashMap::new();

    // Set input registers.
    for (idx, &(reg, _)) in func.inputs.iter().enumerate() {
        if idx < input_values.len() {
            registers.insert(reg, input_values[idx]);
        } else {
            registers.insert(reg, 0);
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
                    return Err(eyre!("division by zero"));
                }
            }
            AleoInstruction::Mod { src1, src2, dest } => {
                let v1 = resolve_operand(src1, &registers);
                let v2 = resolve_operand(src2, &registers);
                if v2 != 0 {
                    registers.insert(*dest, v1 % v2);
                } else {
                    return Err(eyre!("modulo by zero"));
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
                let arg_values: Vec<i64> = args
                    .iter()
                    .map(|op| resolve_operand(op, &registers))
                    .collect();

                if let Some(callee) = func_map.get(function_name.as_str()) {
                    let callee_result = execute_function_for_replay(
                        callee,
                        func_map,
                        &arg_values,
                        results,
                        mapping_store,
                    )?;
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

    // Collect outputs.
    let outputs: Vec<i64> = func
        .outputs
        .iter()
        .map(|&reg| registers.get(&reg).copied().unwrap_or(0))
        .collect();

    let result = FunctionResult { registers, outputs };

    results.insert(func.name.clone(), result.clone());

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Config construction --

    #[test]
    fn test_replay_config_construction() {
        let config = ReplayConfig {
            program_id: "hello.aleo".to_string(),
            function_name: "main".to_string(),
            inputs: vec!["3u32".to_string(), "5u32".to_string()],
            endpoint: "https://api.explorer.aleo.org/v1".to_string(),
        };

        assert_eq!(config.program_id, "hello.aleo");
        assert_eq!(config.function_name, "main");
        assert_eq!(config.inputs.len(), 2);
        assert_eq!(config.endpoint, "https://api.explorer.aleo.org/v1");
    }

    #[test]
    fn test_replay_config_empty_inputs() {
        let config = ReplayConfig {
            program_id: "test.aleo".to_string(),
            function_name: "run".to_string(),
            inputs: vec![],
            endpoint: "http://localhost:3030".to_string(),
        };

        assert!(config.inputs.is_empty());
    }

    // -- RPC client --

    #[test]
    fn test_aleo_rpc_client_new() {
        let client = AleoRpcClient::new("https://api.explorer.aleo.org/v1");
        assert_eq!(client.endpoint, "https://api.explorer.aleo.org/v1");
    }

    #[test]
    fn test_aleo_rpc_client_trims_trailing_slash() {
        let client = AleoRpcClient::new("https://api.explorer.aleo.org/v1/");
        assert_eq!(client.endpoint, "https://api.explorer.aleo.org/v1");
    }

    // -- Input parsing --

    #[test]
    fn test_parse_input_value_typed() {
        assert_eq!(parse_input_value("10u32").unwrap(), 10);
        assert_eq!(parse_input_value("42u64").unwrap(), 42);
        assert_eq!(parse_input_value("-5i32").unwrap(), -5);
        assert_eq!(parse_input_value("0u32").unwrap(), 0);
    }

    #[test]
    fn test_parse_input_value_plain() {
        assert_eq!(parse_input_value("10").unwrap(), 10);
        assert_eq!(parse_input_value("0").unwrap(), 0);
        assert_eq!(parse_input_value("-3").unwrap(), -3);
    }

    #[test]
    fn test_parse_input_value_whitespace() {
        assert_eq!(parse_input_value("  10u32  ").unwrap(), 10);
    }

    #[test]
    fn test_parse_input_value_invalid() {
        assert!(parse_input_value("abc").is_err());
        assert!(parse_input_value("").is_err());
    }

    // -- Mock program fetch --

    #[test]
    fn test_fetch_mock_program_hello() {
        let program = fetch_mock_program("hello.aleo").unwrap();

        assert_eq!(program.program_id, "hello.aleo");
        assert!(program.source.contains("program hello.aleo"));
        assert!(program.source.contains("function main"));
    }

    #[test]
    fn test_fetch_mock_program_arithmetic() {
        let program = fetch_mock_program("arithmetic.aleo").unwrap();

        assert_eq!(program.program_id, "arithmetic.aleo");
        assert!(program.source.contains("function compute"));
        assert!(program.source.contains("function main"));
    }

    #[test]
    fn test_fetch_mock_program_not_found() {
        let result = fetch_mock_program("nonexistent.aleo");
        assert!(result.is_err());
    }

    // -- Deployed program struct --

    #[test]
    fn test_deployed_program_fields() {
        let program = DeployedProgram {
            program_id: "test.aleo".to_string(),
            source: "program test.aleo;\n".to_string(),
        };

        assert_eq!(program.program_id, "test.aleo");
        assert!(program.source.starts_with("program"));
    }

    // -- Replay pipeline with hardcoded Aleo instructions --

    #[test]
    fn test_replay_hello_program() {
        let deployed = DeployedProgram {
            program_id: "hello.aleo".to_string(),
            source: MOCK_HELLO_PROGRAM.to_string(),
        };

        let config = ReplayConfig {
            program_id: "hello.aleo".to_string(),
            function_name: "main".to_string(),
            inputs: vec!["3u32".to_string(), "5u32".to_string()],
            endpoint: "http://localhost:3030".to_string(),
        };

        let out_dir = tempfile::tempdir().unwrap();
        let result = replay_deployed_program(
            &deployed,
            &config,
            out_dir.path(),
            TraceEventsFileFormat::Json,
        );

        assert!(result.is_ok(), "replay failed: {:?}", result.err());

        // Verify .ct output with CTFS magic bytes.
        let ct_files: Vec<_> = std::fs::read_dir(out_dir.path())
            .expect("read output dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "ct"))
            .collect();
        assert!(!ct_files.is_empty(), "expected at least one .ct file in output dir");
        let content = std::fs::read(&ct_files[0]).expect("read .ct file");
        assert!(content.len() >= 5, ".ct file too small");
        assert_eq!(&content[..5], &[0xC0u8, 0xDE, 0x72, 0xAC, 0xE2], "CTFS magic bytes mismatch");

        // Verify the source file was written.
        assert!(out_dir.path().join("hello.aleo").exists());
    }

    #[test]
    fn test_replay_arithmetic_program() {
        let deployed = DeployedProgram {
            program_id: "arithmetic.aleo".to_string(),
            source: MOCK_ARITHMETIC_PROGRAM.to_string(),
        };

        let config = ReplayConfig {
            program_id: "arithmetic.aleo".to_string(),
            function_name: "main".to_string(),
            inputs: vec!["10u32".to_string(), "20u32".to_string()],
            endpoint: "http://localhost:3030".to_string(),
        };

        let out_dir = tempfile::tempdir().unwrap();
        let result = replay_deployed_program(
            &deployed,
            &config,
            out_dir.path(),
            TraceEventsFileFormat::Json,
        );

        assert!(result.is_ok(), "replay failed: {:?}", result.err());

        // Check that the source file was saved.
        let source_content =
            std::fs::read_to_string(out_dir.path().join("arithmetic.aleo")).unwrap();
        assert!(source_content.contains("function compute"));
    }

    #[test]
    fn test_replay_function_not_found() {
        let deployed = DeployedProgram {
            program_id: "hello.aleo".to_string(),
            source: MOCK_HELLO_PROGRAM.to_string(),
        };

        let config = ReplayConfig {
            program_id: "hello.aleo".to_string(),
            function_name: "nonexistent".to_string(),
            inputs: vec![],
            endpoint: "http://localhost:3030".to_string(),
        };

        let out_dir = tempfile::tempdir().unwrap();
        let result = replay_deployed_program(
            &deployed,
            &config,
            out_dir.path(),
            TraceEventsFileFormat::Json,
        );

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.err().unwrap());
        assert!(err_msg.contains("nonexistent"));
    }

    #[test]
    fn test_replay_execution_correctness() {
        // Verify that the AVM interpreter produces correct results
        // when replaying the hello.aleo mock program (3 + 5 = 8).
        let source = MOCK_HELLO_PROGRAM;
        let functions = parse_aleo_program(source);
        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        let mut results = HashMap::new();
        let mut mapping_store = MappingStore::new();

        let result = execute_function_for_replay(
            func_map["main"],
            &func_map,
            &[3, 5],
            &mut results,
            &mut mapping_store,
        )
        .unwrap();

        // r0=3, r1=5, r2=8 (3+5)
        assert_eq!(result.registers[&0], 3);
        assert_eq!(result.registers[&1], 5);
        assert_eq!(result.registers[&2], 8);
        assert_eq!(result.outputs, vec![8]);
    }

    #[test]
    fn test_replay_execution_arithmetic_correctness() {
        // Verify the arithmetic.aleo mock: compute(10, 20)
        //   r0=10, r1=20
        //   r2 = r0 + r1 = 30
        //   r3 = r2 * 2 = 60
        //   r4 = r3 - r0 = 50
        //   output r4
        let source = MOCK_ARITHMETIC_PROGRAM;
        let functions = parse_aleo_program(source);
        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        let mut results = HashMap::new();
        let mut mapping_store = MappingStore::new();

        let result = execute_function_for_replay(
            func_map["main"],
            &func_map,
            &[10, 20],
            &mut results,
            &mut mapping_store,
        )
        .unwrap();

        // main calls compute(10, 20), which returns 50.
        assert_eq!(result.outputs, vec![50]);

        // The compute function's result should also be recorded.
        let compute_result = results.get("compute").unwrap();
        assert_eq!(compute_result.registers[&2], 30); // 10 + 20
        assert_eq!(compute_result.registers[&3], 60); // 30 * 2
        assert_eq!(compute_result.registers[&4], 50); // 60 - 10
        assert_eq!(compute_result.outputs, vec![50]);
    }
}
