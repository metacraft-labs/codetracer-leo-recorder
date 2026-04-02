//! Integration tests for the Leo tracer.
//!
//! These tests parse and evaluate real Leo program files through the
//! source-level evaluator and verify the resulting CodeTracer trace output.
//!
//! Tests verify actual trace content with specific computed values,
//! not just file existence or non-emptiness.

use std::path::{Path, PathBuf};

use codetracer_leo_recorder::source_map::generate_source_map;
use codetracer_trace_writer::TraceEventsFileFormat;

/// Helper: path to the test-programs directory.
fn test_programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/leo")
}

/// Helper: run the tracer on a Leo source file and return the output directory.
fn run_tracer_on_file(source_path: &Path, out_dir: &Path) {
    codetracer_leo_recorder::recorder::record(source_path, out_dir, TraceEventsFileFormat::Json)
        .expect("trace_program should succeed");
}

/// Helper: parse the trace events JSON from the output directory.
fn load_trace_events(out_dir: &Path) -> Vec<serde_json::Value> {
    let events_path = out_dir.join("trace.json");
    let content = std::fs::read_to_string(&events_path).expect("failed to read trace events");
    let events: serde_json::Value =
        serde_json::from_str(&content).expect("trace events should be valid JSON");
    events
        .as_array()
        .expect("events should be an array")
        .clone()
}

/// Helper: parse trace_metadata.json from the output directory.
fn load_trace_metadata(out_dir: &Path) -> serde_json::Value {
    let metadata_path = out_dir.join("trace_metadata.json");
    let content =
        std::fs::read_to_string(&metadata_path).expect("failed to read trace_metadata.json");
    serde_json::from_str(&content).expect("trace_metadata.json should be valid JSON")
}

/// Helper: collect all Int values from Value events in the trace.
/// Returns a vec of (variable_id, i64_value) pairs.
fn collect_int_values(events: &[serde_json::Value]) -> Vec<(i64, i64)> {
    events
        .iter()
        .filter_map(|e| {
            let val = e.get("Value")?;
            let variable_id = val.get("variable_id")?.as_i64()?;
            let value = val.get("value")?;
            if value.get("kind").and_then(|k| k.as_str()) == Some("Int") {
                let i = value.get("i").and_then(|v| v.as_i64())?;
                Some((variable_id, i))
            } else {
                None
            }
        })
        .collect()
}

/// Helper: collect all VariableName events and return the names in order.
fn collect_variable_names(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| {
            e.get("VariableName")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Helper: find all Int values for a given variable name across the trace.
fn find_variable_values(events: &[serde_json::Value], var_name: &str) -> Vec<i64> {
    let var_names = collect_variable_names(events);
    let var_id = var_names.iter().position(|name| name == var_name);

    match var_id {
        Some(id) => {
            let int_values = collect_int_values(events);
            int_values
                .iter()
                .filter(|(vid, _)| *vid == id as i64)
                .map(|(_, v)| *v)
                .collect()
        }
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// Test 1: Record flow_test.leo, verify 3-file output
// ---------------------------------------------------------------------------

#[test]
fn test_leo_compile_and_run() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    // Verify the three output files exist and are non-empty.
    for filename in &["trace.json", "trace_metadata.json", "trace_paths.json"] {
        let path = out_dir.join(filename);
        assert!(path.exists(), "{} should exist", filename);
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 0, "{} should be non-empty", filename);
    }

    // trace.json should be valid JSON containing an array of events.
    let events = load_trace_events(&out_dir);
    assert!(!events.is_empty(), "trace should have at least one event");

    // There should be Step events (actual execution was recorded).
    let step_count = events.iter().filter(|e| e.get("Step").is_some()).count();
    assert!(
        step_count > 0,
        "trace should contain at least one Step event, got none"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Verify trace contains value 94 (final_result = doubled + a)
// ---------------------------------------------------------------------------

#[test]
fn test_leo_compute_value() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let events = load_trace_events(&out_dir);

    // Collect all integer values from the trace.
    let int_values = collect_int_values(&events);
    let all_values: Vec<i64> = int_values.iter().map(|(_, v)| *v).collect();

    // The program calculates: a=10, b=32, sum_val=a+b=42, doubled=sum_val*2=84,
    // final_result=doubled+a=94. This value should appear in the trace.
    assert!(
        all_values.contains(&94),
        "trace should contain value 94 (final_result = doubled + a = 84 + 10), got values: {:?}",
        {
            let mut unique: Vec<i64> = all_values.clone();
            unique.sort();
            unique.dedup();
            unique
        }
    );
}

// ---------------------------------------------------------------------------
// Test 3: Verify variable values a=10, b=32, sum_val=42, doubled=84,
//         final_result=94
// ---------------------------------------------------------------------------

#[test]
fn test_leo_variable_values() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let events = load_trace_events(&out_dir);

    // Check that VariableName events exist.
    let var_names = collect_variable_names(&events);
    assert!(
        !var_names.is_empty(),
        "trace should contain VariableName events"
    );

    // Verify specific variable names appear.
    assert!(
        var_names.contains(&"a".to_string()),
        "variable 'a' should appear in trace, got names: {:?}",
        var_names
    );
    assert!(
        var_names.contains(&"b".to_string()),
        "variable 'b' should appear in trace, got names: {:?}",
        var_names
    );
    assert!(
        var_names.contains(&"sum_val".to_string()),
        "variable 'sum_val' should appear in trace, got names: {:?}",
        var_names
    );
    assert!(
        var_names.contains(&"doubled".to_string()),
        "variable 'doubled' should appear in trace, got names: {:?}",
        var_names
    );
    assert!(
        var_names.contains(&"final_result".to_string()),
        "variable 'final_result' should appear in trace, got names: {:?}",
        var_names
    );

    // Verify variable values.
    let a_values = find_variable_values(&events, "a");
    assert!(
        a_values.contains(&10),
        "variable 'a' should have value 10, got: {:?}",
        a_values
    );

    let b_values = find_variable_values(&events, "b");
    assert!(
        b_values.contains(&32),
        "variable 'b' should have value 32, got: {:?}",
        b_values
    );

    let sum_values = find_variable_values(&events, "sum_val");
    assert!(
        sum_values.contains(&42),
        "variable 'sum_val' should have value 42 (a + b = 10 + 32), got: {:?}",
        sum_values
    );

    let doubled_values = find_variable_values(&events, "doubled");
    assert!(
        doubled_values.contains(&84),
        "variable 'doubled' should have value 84 (sum_val * 2 = 42 * 2), got: {:?}",
        doubled_values
    );

    let final_values = find_variable_values(&events, "final_result");
    assert!(
        final_values.contains(&94),
        "variable 'final_result' should have value 94 (doubled + a = 84 + 10), got: {:?}",
        final_values
    );
}

// ---------------------------------------------------------------------------
// Test 4: Verify Step events at correct source lines
// ---------------------------------------------------------------------------

#[test]
fn test_leo_step_events() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let events = load_trace_events(&out_dir);

    // Count Step events.
    let step_events: Vec<&serde_json::Value> =
        events.iter().filter(|e| e.get("Step").is_some()).collect();

    // flow_test.leo has 5 let bindings + 1 return in compute() + 1 return in main()
    // but main()'s return calls compute() which adds its own steps.
    // At minimum: 5 let + 1 return in compute = 6, plus 1 return in main = 7.
    assert!(
        step_events.len() >= 6,
        "should have at least 6 step events for flow_test.leo, got {}",
        step_events.len()
    );

    // Verify step events have valid structure.
    for event in &step_events {
        let step = event.get("Step").unwrap();
        assert!(
            step.get("path_id").is_some(),
            "Step event should have path_id field"
        );
        let line = step["line"]
            .as_i64()
            .expect("Step line should be an integer");
        assert!(line > 0, "Step line should be positive, got {}", line);
        assert!(
            line <= 20,
            "Step line should be within source file range, got {}",
            line
        );
    }

    // Verify that step events include the let binding lines.
    let step_lines: Vec<i64> = step_events
        .iter()
        .map(|e| e.get("Step").unwrap()["line"].as_i64().unwrap())
        .collect();

    // Line 3: let a: u32 = 10u32;
    assert!(
        step_lines.contains(&3),
        "step events should include line 3 (let a), got lines: {:?}",
        step_lines
    );
    // Line 4: let b: u32 = 32u32;
    assert!(
        step_lines.contains(&4),
        "step events should include line 4 (let b), got lines: {:?}",
        step_lines
    );
    // Line 5: let sum_val: u32 = a + b;
    assert!(
        step_lines.contains(&5),
        "step events should include line 5 (let sum_val), got lines: {:?}",
        step_lines
    );
    // Line 6: let doubled: u32 = sum_val * 2u32;
    assert!(
        step_lines.contains(&6),
        "step events should include line 6 (let doubled), got lines: {:?}",
        step_lines
    );
    // Line 7: let final_result: u32 = doubled + a;
    assert!(
        step_lines.contains(&7),
        "step events should include line 7 (let final_result), got lines: {:?}",
        step_lines
    );
}

// ---------------------------------------------------------------------------
// Test 5: Verify Call/Return events for compute() function call
// ---------------------------------------------------------------------------

#[test]
fn test_leo_function_calls() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let events = load_trace_events(&out_dir);

    // There should be Call events (function entries were recorded).
    let call_count = events.iter().filter(|e| e.get("Call").is_some()).count();
    assert!(
        call_count >= 2,
        "trace should contain at least 2 Call events (main + compute), got {}",
        call_count
    );

    // There should be Return events.
    let return_events: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e.get("Return").is_some())
        .collect();
    assert!(
        return_events.len() >= 2,
        "trace should contain at least 2 Return events, got {}",
        return_events.len()
    );

    // The last Return event should be after the last Step.
    let last_return_idx = events
        .iter()
        .rposition(|e| e.get("Return").is_some())
        .expect("should have a Return event");

    let steps_after_return = events[last_return_idx + 1..]
        .iter()
        .filter(|e| e.get("Step").is_some())
        .count();
    assert_eq!(
        steps_after_return, 0,
        "no Step events should appear after the final Return"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Verify metadata JSON structure
// ---------------------------------------------------------------------------

#[test]
fn test_leo_metadata_structure() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let metadata = load_trace_metadata(&out_dir);

    // TraceMetadata must contain "program", "args", and "workdir" fields.
    assert!(
        metadata.get("program").is_some(),
        "metadata should have 'program' field, got: {}",
        metadata
    );
    assert!(
        metadata["program"].is_string(),
        "metadata 'program' should be a string"
    );
    let program_str = metadata["program"].as_str().unwrap();
    assert!(
        program_str.contains("flow_test.leo"),
        "metadata 'program' should reference the leo source file, got: {}",
        program_str
    );

    assert!(
        metadata.get("args").is_some(),
        "metadata should have 'args' field, got: {}",
        metadata
    );
    assert!(
        metadata["args"].is_array(),
        "metadata 'args' should be an array"
    );

    assert!(
        metadata.get("workdir").is_some(),
        "metadata should have 'workdir' field, got: {}",
        metadata
    );
    assert!(
        metadata["workdir"].is_string(),
        "metadata 'workdir' should be a string"
    );
}

// ---------------------------------------------------------------------------
// Test 7: CLI record end-to-end test
// ---------------------------------------------------------------------------

#[test]
fn test_leo_cli_record() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("cli-traces");
    let source_path = test_programs_dir().join("flow_test.leo");

    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "run",
            "--quiet",
            "--",
            "record",
            source_path.to_str().unwrap(),
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("failed to run");

    assert!(
        output.status.success(),
        "record should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify output files exist.
    assert!(out_dir.join("trace.json").exists());
    assert!(out_dir.join("trace_metadata.json").exists());
    assert!(out_dir.join("trace_paths.json").exists());

    // Verify the CLI-produced trace has actual content.
    let events = load_trace_events(&out_dir);
    assert!(!events.is_empty(), "CLI trace should have events");

    let step_count = events.iter().filter(|e| e.get("Step").is_some()).count();
    assert!(step_count > 0, "CLI trace should contain Step events");

    // Verify values are present in the CLI-produced trace too.
    let int_values = collect_int_values(&events);
    let all_values: Vec<i64> = int_values.iter().map(|(_, v)| *v).collect();
    assert!(
        all_values.contains(&94),
        "CLI trace should contain value 94, got values: {:?}",
        all_values
    );
}

// ---------------------------------------------------------------------------
// Test 8: AleoSourceMap generation for flow_test program
// ---------------------------------------------------------------------------

#[test]
fn test_source_map_flow_test_program() {
    // Leo source for flow_test.
    let leo_source = r#"program flow_test.aleo {
    transition compute() -> u32 {
        let a: u32 = 10u32;
        let b: u32 = 32u32;
        let sum_val: u32 = a + b;
        let doubled: u32 = sum_val * 2u32;
        let final_result: u32 = doubled + a;
        return final_result;
    }

    transition main() -> u32 {
        return compute();
    }
}"#;

    // Corresponding Aleo output (as the Leo compiler would produce).
    let aleo_source = r#"program flow_test.aleo;

closure compute:
    add 10u32 0u32 into r0;
    add 32u32 0u32 into r1;
    add r0 r1 into r2;
    mul r2 2u32 into r3;
    add r3 r0 into r4;
    output r4 as u32;

function main:
    call compute into r0;
    output r0 as u32.private;
"#;

    let source_map = generate_source_map(leo_source, aleo_source);

    // All 5 instructions in compute should map to Leo lines 3-7.
    assert_eq!(source_map.instruction_count("compute"), 5);

    let expected_lines = [3u32, 4, 5, 6, 7];
    for (idx, &expected_line) in expected_lines.iter().enumerate() {
        let (file, line) = source_map
            .resolve("compute", idx)
            .unwrap_or_else(|| panic!("instruction {} should resolve", idx));
        assert_eq!(
            file, "flow_test.leo",
            "instruction {} should map to flow_test.leo",
            idx
        );
        assert_eq!(
            line, expected_line,
            "instruction {} should map to Leo line {}, got {}",
            idx, expected_line, line
        );
    }
}

// ---------------------------------------------------------------------------
// Test 9: Traced steps reference Leo source lines (not Aleo lines)
// ---------------------------------------------------------------------------

#[test]
fn test_traced_steps_reference_leo_lines() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let events = load_trace_events(&out_dir);

    // Collect step lines.
    let step_lines: Vec<i64> = events
        .iter()
        .filter_map(|e| {
            e.get("Step")
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_i64())
        })
        .collect();

    // All step lines should be within the Leo source range (1-14 for flow_test.leo).
    for &line in &step_lines {
        assert!(
            (1..=14).contains(&line),
            "step line {} should be within Leo source range 1-14",
            line
        );
    }

    // Specifically, the let-binding lines (3, 4, 5, 6, 7) should be present.
    // These are Leo source lines, not Aleo instruction indices.
    for expected in &[3i64, 4, 5, 6, 7] {
        assert!(
            step_lines.contains(expected),
            "step events should include Leo source line {}, got lines: {:?}",
            expected,
            step_lines
        );
    }
}

// ---------------------------------------------------------------------------
// Test 10: Source map edge case — empty function (no let-bindings)
// ---------------------------------------------------------------------------

#[test]
fn test_source_map_empty_function() {
    let leo_source = r#"program empty_test.aleo {
    transition main() -> u32 {
        return 0u32;
    }
}"#;

    let aleo_source = r#"program empty_test.aleo;

function main:
    output 0u32 as u32.private;
"#;

    let source_map = generate_source_map(leo_source, aleo_source);

    // main has no instructions (only output line), so instruction_count is 0.
    assert_eq!(source_map.instruction_count("main"), 0);
    // Resolving any index should return None.
    assert!(
        source_map.resolve("main", 0).is_none(),
        "empty function should have no source map entries"
    );
}

// ---------------------------------------------------------------------------
// Test 11: Source map edge case — function with no let-bindings but with
//          instructions (e.g. a wrapper that just calls another function)
// ---------------------------------------------------------------------------

#[test]
fn test_source_map_call_only_function() {
    let leo_source = r#"program call_only.aleo {
    transition compute() -> u32 {
        let x: u32 = 42u32;
        return x;
    }

    transition main() -> u32 {
        return compute();
    }
}"#;

    let aleo_source = r#"program call_only.aleo;

closure compute:
    add 42u32 0u32 into r0;
    output r0 as u32;

function main:
    call compute into r0;
    output r0 as u32.private;
"#;

    let source_map = generate_source_map(leo_source, aleo_source);

    // compute has 1 instruction mapping to line 3 (let x: u32 = 42u32;).
    assert_eq!(source_map.instruction_count("compute"), 1);
    let (file, line) = source_map.resolve("compute", 0).unwrap();
    assert_eq!(file, "call_only.leo");
    assert_eq!(line, 3);

    // main has 1 instruction (call) but no let-bindings, so no mapping.
    assert_eq!(source_map.instruction_count("main"), 1);
    assert!(
        source_map.resolve("main", 0).is_none(),
        "main has no let-bindings so call instruction should not map"
    );
}
