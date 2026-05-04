//! CTFS audit regression tests for the Leo recorder.
//!
//! Closes audit gaps documented in AUDIT-CTFS-2026-05.md:
//!
//! - (a) `--format ctfs` is the default; the CLI advertises it and
//!   produces the canonical multi-stream `.ct` container by default.
//! - (c) Transition parameter names are surfaced on `CallRecord.args`
//!   via `TraceWriter::arg(name, NONE_VALUE)` staging so the
//!   calltrace pane's `.call-arg` rows match the source.
//! - (d) Compile/generation errors are surfaced as `EventLogKind::Error`
//!   records with `leo_compile_error` metadata before the recorder aborts.

use std::path::{Path, PathBuf};

use codetracer_trace_types::{ValueRecord, NONE_VALUE};
use codetracer_trace_writer_nim::{NimTraceReaderHandle, TraceEventsFileFormat};

/// CTFS magic bytes: C0 DE 72 AC E2.
///
/// See `codetracer-trace-format-spec/` for the canonical schema.
const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

fn test_programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/leo")
}

fn first_ct_file(out_dir: &Path) -> PathBuf {
    let mut ct_files: Vec<_> = std::fs::read_dir(out_dir)
        .expect("read output dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();
    assert!(
        !ct_files.is_empty(),
        "expected at least one .ct file in {}",
        out_dir.display()
    );
    ct_files.sort();
    ct_files[0].clone()
}

fn open_ctfs_reader(out_dir: &Path) -> NimTraceReaderHandle {
    let ct_path = first_ct_file(out_dir);
    NimTraceReaderHandle::open(&ct_path.to_string_lossy()).unwrap_or_else(|e| {
        panic!(
            "failed to open Nim CTFS reader for {}: {e}",
            ct_path.display()
        )
    })
}

fn bytes_from_json_array(value: &serde_json::Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected byte array JSON, got {value:#}"))
        .iter()
        .map(|byte| {
            byte.as_u64()
                .unwrap_or_else(|| panic!("expected byte value, got {byte:#}")) as u8
        })
        .collect()
}

fn decode_value_record(value: &serde_json::Value) -> ValueRecord {
    let bytes = bytes_from_json_array(value);
    cbor4ii::serde::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("failed to decode ValueRecord from {bytes:?}: {e}"))
}

fn string_from_json_byte_array(value: &serde_json::Value) -> String {
    String::from_utf8(bytes_from_json_array(value))
        .unwrap_or_else(|e| panic!("expected UTF-8 byte array, got error: {e}"))
}

fn read_events(out_dir: &Path) -> Vec<serde_json::Value> {
    let reader = open_ctfs_reader(out_dir);
    (0..reader.event_count())
        .map(|index| {
            let json = reader.event_json(index).expect("read event JSON");
            serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("invalid event JSON: {e}: {json}"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Audit (a) -- default-Ctfs CLI + canonical multi-stream `.ct` container
// ---------------------------------------------------------------------------

/// Recording with `TraceEventsFileFormat::Ctfs` produces a `.ct` container
/// whose first 5 bytes match the canonical CTFS magic.
#[test]
fn ctfs_writer_produces_ct_container() {
    let tmp_dir = tempfile::tempdir().expect("temp dir");
    let out_dir = tmp_dir.path().join("ctfs-traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    codetracer_leo_recorder::recorder::record(&source_path, &out_dir, TraceEventsFileFormat::Ctfs)
        .expect("record should succeed");

    let ct_path = first_ct_file(&out_dir);
    let bytes = std::fs::read(&ct_path).expect("read .ct file");
    assert!(
        bytes.len() > CTFS_MAGIC.len(),
        ".ct container should have a header + body, got {} bytes",
        bytes.len()
    );
    assert_eq!(
        &bytes[..CTFS_MAGIC.len()],
        &CTFS_MAGIC,
        ".ct container should start with the canonical CTFS magic"
    );
}

/// `record --help` advertises the Ctfs format and lists it as the default.
///
/// Same idiom as the audited recorders (Flow 1.52 / Fuel 1.53 / PolkaVM 1.55
/// / Miden 1.56 / TON 1.57 / Circom 1.58).
#[test]
fn ctfs_format_advertised_in_record_help() {
    let output = std::process::Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--", "record", "--help"])
        .output()
        .expect("failed to run record --help");

    assert!(
        output.status.success(),
        "record --help should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ctfs"),
        "record --help should advertise the `ctfs` format value, got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("[default: ctfs]"),
        "record --help should advertise `ctfs` as the DEFAULT format, got:\n{}",
        stdout
    );
}

/// Invoking `record` without `--format` produces a CTFS-magic `.ct`
/// container, proving that `ctfs` is the wired-through default.
#[test]
fn ctfs_is_the_default_record_format() {
    let tmp_dir = tempfile::tempdir().expect("temp dir");
    let out_dir = tmp_dir.path().join("default-format-traces");
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
        ])
        .output()
        .expect("failed to run record");

    assert!(
        output.status.success(),
        "default-format record should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let ct_path = first_ct_file(&out_dir);
    let bytes = std::fs::read(&ct_path).expect("read .ct file");
    assert!(bytes.len() >= CTFS_MAGIC.len(), ".ct file too small");
    assert_eq!(
        &bytes[..CTFS_MAGIC.len()],
        &CTFS_MAGIC,
        "default-format .ct should be a canonical CTFS container"
    );
}

// ---------------------------------------------------------------------------
// Audit (c) -- transition parameter staging via TraceWriter::arg
// ---------------------------------------------------------------------------

/// Read the produced CTFS container back and assert the parameter-staging
/// path reaches `CallRecord.args`.  Source-level Leo values are still the
/// canonical `NONE_VALUE` placeholder; this test pins the read-side
/// content contract without expanding recorder semantics.
#[test]
fn ctfs_reader_sees_staged_call_args() {
    // Author a tiny Leo program with a parameterised transition.
    // The staging path runs for every non-entry-point function, which
    // the recorder maps to the callees of `main`.
    let tmp_src = tempfile::tempdir().expect("temp dir");
    let leo_src = r#"program staging_test.aleo {
    transition compute(a: u32, b: u32) -> u32 {
        let sum_val: u32 = a + b;
        return sum_val;
    }

    transition main() -> u32 {
        return compute();
    }
}"#;
    let leo_path = tmp_src.path().join("staging.leo");
    std::fs::write(&leo_path, leo_src).expect("write leo source");

    let tmp_out = tempfile::tempdir().expect("temp dir");
    let out_dir = tmp_out.path().join("staging-traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    codetracer_leo_recorder::recorder::record(&leo_path, &out_dir, TraceEventsFileFormat::Ctfs)
        .expect("record should succeed");

    let ct_path = first_ct_file(&out_dir);
    let bytes = std::fs::read(&ct_path).expect("read .ct file");
    assert!(
        bytes.len() > CTFS_MAGIC.len() + 16,
        "parameterised-transition .ct container should have substantial content, got {} bytes",
        bytes.len()
    );
    assert_eq!(
        &bytes[..CTFS_MAGIC.len()],
        &CTFS_MAGIC,
        "staging-test .ct should be a canonical CTFS container"
    );

    let reader = open_ctfs_reader(&out_dir);
    let calls: Vec<serde_json::Value> = (0..reader.call_count())
        .map(|key| {
            let json = reader.call_json(key).expect("read call JSON");
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("invalid call JSON: {e}: {json}"))
        })
        .collect();
    assert!(!calls.is_empty(), "expected at least one call record");

    let staged_call = calls
        .iter()
        .find(|call| call["args"].as_array().is_some_and(|args| args.len() == 2))
        .unwrap_or_else(|| panic!("missing call with two staged args: {calls:#?}"));
    let args = staged_call["args"]
        .as_array()
        .expect("staged call args should be an array");
    let arg_names: Vec<_> = args
        .iter()
        .map(|arg| {
            let id = arg["varname_id"]
                .as_u64()
                .unwrap_or_else(|| panic!("arg missing varname_id: {arg:#}"));
            reader.varname(id).expect("read arg varname")
        })
        .collect();
    assert_eq!(arg_names, vec!["a", "b"]);

    for arg in args {
        assert_eq!(
            decode_value_record(&arg["value"]),
            NONE_VALUE,
            "source-level Leo parameter values should decode as ValueRecord::None"
        );
    }
}

// ---------------------------------------------------------------------------
// Audit (d) -- compile errors via register_special_event(Error, ...)
// ---------------------------------------------------------------------------

/// Even when the Leo compiler/fallback generator rejects the source, the
/// recorder should leave a debuggable partial CTFS trace with a canonical
/// Error event before returning the CLI/library error.
#[test]
fn ctfs_reader_sees_leo_compile_error_event() {
    let tmp_src = tempfile::tempdir().expect("temp dir");
    let leo_path = tmp_src.path().join("invalid.leo");
    std::fs::write(
        &leo_path,
        "this is not a Leo program and declares no transition or function\n",
    )
    .expect("write invalid leo source");

    let tmp_out = tempfile::tempdir().expect("temp dir");
    let out_dir = tmp_out.path().join("compile-error-traces");

    let err =
        codetracer_leo_recorder::recorder::record(&leo_path, &out_dir, TraceEventsFileFormat::Ctfs)
            .expect_err("invalid Leo source should fail recording");
    assert!(
        format!("{err:#}").contains("no transition or function declarations found"),
        "unexpected compile/generation error: {err:#}"
    );

    let ct_path = first_ct_file(&out_dir);
    let bytes = std::fs::read(&ct_path).expect("read compile-error .ct file");
    assert_eq!(
        &bytes[..CTFS_MAGIC.len()],
        &CTFS_MAGIC,
        "compile-error trace should still be a canonical CTFS container"
    );

    let events = read_events(&out_dir);
    let error_event = events
        .iter()
        .find(|event| event["kind"].as_str() == Some("error"))
        .unwrap_or_else(|| panic!("missing CTFS error event: {events:#?}"));
    let content = string_from_json_byte_array(&error_event["data"]);
    assert!(
        content.contains("no transition or function declarations"),
        "unexpected compile-error content: {content}"
    );
}
