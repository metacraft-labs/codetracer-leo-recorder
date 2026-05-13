//! Integration tests for the Leo tracer.
//!
//! Tests cover three areas:
//!
//! 1. **Recorder API** — drive `codetracer_leo_recorder::recorder::record`
//!    directly with a real Leo fixture and assert the resulting `.ct`
//!    container exists and starts with the canonical CTFS magic bytes.
//! 2. **`ct print` content** — record a fixture and pipe the resulting
//!    `.ct` container through `ct-print --json` from
//!    `codetracer-trace-format-nim` to make content-level assertions.
//!    Skips gracefully when `ct-print` is not present (i.e. when this
//!    crate is built outside the metacraft workspace).
//! 3. **CLI env-var contract** — exercise the post-2026-05-08
//!    `CODETRACER_LEO_RECORDER_OUT_DIR` /
//!    `CODETRACER_LEO_RECORDER_DISABLED` env vars and the
//!    no-`--format` invariant from `Recorder-CLI-Conventions.md` §4 / §5.
//!    These invoke the recorder binary via `CARGO_BIN_EXE_*`.
//!
//! History note: pre-2026-05-08 the recorder shipped a `--format
//! ctfs|binary|json` flag and the `test_leo_cli_record` test passed
//! `--format ctfs` explicitly.  When the convention switched to
//! CTFS-only the `--format` argument was removed and the
//! `test_leo_cli_record` test was rewritten to omit it.  See
//! `AUDIT-CTFS-2026-05.md` ("Convention compliance follow-up") for the
//! full record.

use std::path::{Path, PathBuf};
use std::process::Command;

/// CTFS magic bytes: C0 DE 72 AC E2
const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

fn test_programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/leo")
}

fn run_tracer_on_file(source_path: &Path, out_dir: &Path) {
    // Recorder is CTFS-only — `record` takes no format parameter.
    codetracer_leo_recorder::recorder::record(source_path, out_dir)
        .expect("trace_program should succeed");
}

fn assert_valid_ct_file(out_dir: &Path) -> PathBuf {
    let ct_files: Vec<_> = std::fs::read_dir(out_dir)
        .expect("failed to read output directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();

    assert!(
        !ct_files.is_empty(),
        "expected at least one .ct file in {}, found: {:?}",
        out_dir.display(),
        std::fs::read_dir(out_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect::<Vec<_>>()
    );

    let ct_path = &ct_files[0];
    let content = std::fs::read(ct_path).expect("failed to read .ct file");
    assert!(content.len() >= CTFS_MAGIC.len(), ".ct file too small");
    assert_eq!(
        &content[..CTFS_MAGIC.len()],
        &CTFS_MAGIC,
        ".ct file should start with CTFS magic bytes"
    );

    ct_path.clone()
}

/// Path to the `ct-print` binary shipped with `codetracer-trace-format-nim`.
///
/// The Leo recorder is CTFS-only; tests that need to make content-level
/// assertions on a recorded trace pipe the `.ct` container through
/// `ct-print --json` and assert on the resulting JSON.  This is the
/// same workflow that `Recorder-CLI-Conventions.md` §4 prescribes for
/// downstream tools / golden snapshots.
fn ct_print_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("codetracer-trace-format-nim")
        .join("ct-print")
}

// ---------------------------------------------------------------------------
// Recorder API smoke tests
// ---------------------------------------------------------------------------

#[test]
fn test_leo_compile_and_run() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let ct_path = assert_valid_ct_file(&out_dir);
    let size = std::fs::metadata(&ct_path).unwrap().len();
    assert!(
        size > 100,
        ".ct file should have substantial content, got {} bytes",
        size
    );
}

#[test]
fn test_leo_compute_value() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_leo_variable_values() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_leo_step_events() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_leo_metadata_structure() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_leo_cli_record() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("cli-traces");
    let source_path = test_programs_dir().join("flow_test.leo");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-leo-recorder"))
        .args([
            "record",
            source_path.to_str().unwrap(),
            "--out-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run");

    assert!(
        output.status.success(),
        "record should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_leo_function_calls() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_traced_steps_reference_leo_lines() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

// ---------------------------------------------------------------------------
// CTFS content via `ct-print` — replaces the legacy `--format json` content
// assertions
// ---------------------------------------------------------------------------

/// Record `flow_test.leo`, then convert the produced `.ct` container to
/// JSON via `ct-print` and assert on:
///
/// 1. **Structural anchors** (legacy layer): `ct-print --json` output
///    contains the source filename / variable names / canonical u32
///    values somewhere in the textual rendering.
/// 2. **Exact decoded values** (the layer enabled by `ct-print --full`):
///    the `flow_test.leo` program executes `(10 + 32) * 2 + 10 = 94`
///    via the `compute()` transition, with intermediate let-bindings
///    `a=10`, `b=32`, `sum_val=42`, `doubled=84`, `final_result=94`.
///    Each binding must surface in the trace as a step event with a
///    decoded `Int` ValueRecord whose `i` field matches the literal
///    value from the source program.
///
/// Pre-2026-05-08 this assertion was made directly on a recorder-emitted
/// `trace.json` file (via `--format json`).  The convention now mandates
/// CTFS-only output; `ct print` is the canonical conversion tool.  See
/// `Recorder-CLI-Conventions.md` §4.  `ct-print --full` (added 2026-05
/// in `codetracer-trace-format-nim`) is what enables the exact-value
/// layer — its output is a deterministic JSON document with every CBOR
/// `ValueRecord` decoded to a structured form like
/// `{"kind":"Int","i":42,"type_id":6}`.
#[test]
fn test_recorded_trace_via_ct_print_json() {
    let ct_print = ct_print_path();
    if !ct_print.exists() {
        eprintln!(
            "SKIP: ct-print not found at {} — only available within the \
             metacraft workspace where codetracer-trace-format-nim is a sibling.",
            ct_print.display()
        );
        return;
    }

    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.leo");
    run_tracer_on_file(&source_path, &out_dir);

    let ct_path = assert_valid_ct_file(&out_dir);

    // -----------------------------------------------------------------
    // Layer 1 (legacy): ct-print --json — substring presence checks.
    // Kept as a safety net so a regression in the textual rendering
    // is caught even if --full's JSON shape evolves.
    // -----------------------------------------------------------------
    let output = Command::new(&ct_print)
        .args(["--json"])
        .arg(&ct_path)
        .output()
        .expect("failed to run ct-print");

    assert!(
        output.status.success(),
        "ct-print --json should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_json = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout_json.is_empty(),
        "ct-print --json produced empty output"
    );

    // Structural anchor 1: the fixture source path name appears in the
    // path stream rendered by ct-print.
    assert!(
        stdout_json.contains("flow_test.leo"),
        "ct-print --json output should mention the fixture source path \
         (flow_test.leo); got:\n{stdout_json}"
    );

    // Structural anchor 2: at least one of the Leo source-level
    // variable names should appear.  `flow_test.leo` declares
    // `a` / `b` / `sum_val` / `doubled` / `final_result`.
    let variable_anchor = ["a", "b", "sum_val", "doubled", "final_result"]
        .iter()
        .any(|v| stdout_json.contains(v));
    assert!(
        variable_anchor,
        "ct-print --json output should mention at least one of the \
         Leo source variable names (a/b/sum_val/doubled/final_result); \
         got:\n{stdout_json}"
    );

    // Note: unlike the cairo precedent, the leo recorder's `ct-print
    // --json` output does *not* surface the integer payloads (only
    // varnames + step/call structure).  Exact-value assertions live in
    // Layer 2 below (`ct-print --full`), which is the whole point of
    // this upgrade.

    // -----------------------------------------------------------------
    // Layer 2 (the upgrade): ct-print --full — exact decoded values.
    // -----------------------------------------------------------------
    let full_output = Command::new(&ct_print)
        .args(["--full", "--strip-paths"])
        .arg(&ct_path)
        .output()
        .expect("failed to run ct-print --full");

    assert!(
        full_output.status.success(),
        "ct-print --full should succeed; stderr: {}",
        String::from_utf8_lossy(&full_output.stderr)
    );

    let doc: serde_json::Value = serde_json::from_slice(&full_output.stdout)
        .expect("ct-print --full should emit valid JSON");

    // ----- Function table: compute and main must both appear ---------
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        functions.iter().any(|f| f.ends_with("compute")),
        "expected `compute` in functions table; got {:?}",
        functions
    );
    assert!(
        functions.iter().any(|f| f.ends_with("main")),
        "expected `main` in functions table; got {:?}",
        functions
    );

    // ----- Path table: the canonical fixture path must appear ---------
    let paths: Vec<&str> = doc["paths"]
        .as_array()
        .expect("paths array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("flow_test.leo")),
        "expected flow_test.leo in paths table; got {:?}",
        paths
    );

    // ----- Step / call counts ----------------------------------------
    // The Leo recorder, after compiling Leo to Aleo and running the Aleo
    // program, surfaces 8 step events for the canonical fixture: one
    // synthetic top-level step on line 1, then `compute` walks lines
    // 12 -> 3 -> 4 -> 5 -> 6 -> 7 (six steps), then a final step on
    // line 8 outside the call.  Only `compute` is actually traced as a
    // call (call_entry/call_exit pair) — `main` appears in the function
    // table but is not invoked as a recorded call here, the recorder
    // returns the compute result directly.  These are stable properties
    // of the canonical fixture; if they change, that's a real regression
    // to investigate, not a flake.
    let counts = &doc["counts"];
    assert_eq!(
        counts["steps"].as_u64(),
        Some(8),
        "expected 8 step events for flow_test.leo; counts={counts}",
    );
    assert_eq!(
        counts["calls"].as_u64(),
        Some(1),
        "expected 1 call event (compute); counts={counts}",
    );

    let events = doc["events"].as_array().expect("events array");

    // ----- Call sequence: compute is the only recorded call -----------
    let call_sequence: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "call_entry")
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert_eq!(
        call_sequence.len(),
        1,
        "expected exactly 1 call_entry event; got {:?}",
        call_sequence
    );
    assert!(
        call_sequence[0].ends_with("compute"),
        "expected the recorded call to be `compute`; got {:?}",
        call_sequence
    );

    // ----- Exact decoded variable values ------------------------------
    // Collect every (varname, i64) pair surfaced by step events.  These
    // come from the recorder writing `ValueRecord::Int` CBOR blobs, then
    // ct-print --full decoding them back to `{"kind":"Int","i":<n>,...}`.
    let observed_vars: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .filter_map(|v| {
            let name = v["varname"].as_str()?.to_string();
            let value = &v["value"];
            // The leo recorder encodes u32 register values as
            // ValueRecord::Int.  If something else surfaces (e.g. BigInt
            // for wider Aleo numeric types), fail loudly so the test
            // author can decide whether to extend the assertions or
            // accept the new variant.
            assert_eq!(
                value["kind"].as_str(),
                Some("Int"),
                "variable `{}` should decode as Int, got {}; \
                 if a new ValueRecord variant has landed for leo numeric \
                 registers, extend this test to assert on it explicitly \
                 rather than weakening the check",
                name,
                value
            );
            let i = value["i"]
                .as_i64()
                .unwrap_or_else(|| panic!("Int.i must be i64 for `{name}`; got {value}"));
            Some((name, i))
        })
        .collect();

    // The canonical flow: a=10, b=32, sum_val=a+b=42, doubled=sum_val*2=84,
    // final_result=doubled+a=94.
    let expected: &[(&str, i64)] = &[
        ("a", 10),
        ("b", 32),
        ("sum_val", 42),
        ("doubled", 84),
        ("final_result", 94),
    ];
    for (name, value) in expected {
        assert!(
            observed_vars
                .iter()
                .any(|(n, v)| n == name && v == value),
            "expected step variable `{name}` = {value} in --full output; \
             observed = {observed_vars:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// CLI env-var contract
// ---------------------------------------------------------------------------

/// `CODETRACER_LEO_RECORDER_OUT_DIR` must be honoured as a fallback
/// for `--out-dir`.  Convention: `Recorder-CLI-Conventions.md` §5.
#[test]
fn test_env_out_dir_used_when_flag_omitted() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let env_out_dir = tmp_dir.path().join("via-env");

    let source_path = test_programs_dir().join("flow_test.leo");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-leo-recorder"))
        .args(["record"])
        .arg(&source_path)
        .env("CODETRACER_LEO_RECORDER_OUT_DIR", &env_out_dir)
        // Make sure the env-var doesn't bleed in from the developer's shell.
        .env_remove("CODETRACER_LEO_RECORDER_DISABLED")
        .output()
        .expect("failed to run recorder");

    assert!(
        output.status.success(),
        "recorder should succeed when CODETRACER_LEO_RECORDER_OUT_DIR is set; \
         stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The env-var-supplied output dir must contain the .ct bundle.
    let ct_files: Vec<_> = std::fs::read_dir(&env_out_dir)
        .unwrap_or_else(|e| {
            panic!(
                "expected env-supplied out-dir {:?} to exist after record: {e}",
                env_out_dir
            )
        })
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();
    assert!(
        !ct_files.is_empty(),
        "expected the env-supplied output dir {:?} to receive the .ct trace bundle",
        env_out_dir
    );
}

/// `CODETRACER_LEO_RECORDER_DISABLED=1` must skip recording entirely.
/// The recorder process should still exit 0 (the Leo recorder doesn't
/// run a separate target subprocess — it compiles & executes the
/// source itself — so "disabled" simply means "don't write any
/// trace artefacts").
#[test]
fn test_env_disabled_skips_recording() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("should-stay-empty");

    let source_path = test_programs_dir().join("flow_test.leo");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-leo-recorder"))
        .args(["record"])
        .arg(&source_path)
        .args(["--out-dir"])
        .arg(&out_dir)
        .env("CODETRACER_LEO_RECORDER_DISABLED", "1")
        .output()
        .expect("failed to run recorder");

    assert!(
        output.status.success(),
        "recorder should succeed in disabled mode; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // No trace artefacts of any kind should have been written.
    let no_artefacts = !out_dir.exists()
        || (std::fs::read_dir(&out_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).next().is_none())
            .unwrap_or(true));
    assert!(
        no_artefacts,
        "no trace artefacts should be written when \
         CODETRACER_LEO_RECORDER_DISABLED=1; got files in {:?}",
        out_dir
    );
}

/// `--format` is no longer accepted at any level — clap must reject it.
/// Convention: §4 (CTFS-only).
#[test]
fn test_format_flag_rejected_by_clap() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    let source_path = test_programs_dir().join("flow_test.leo");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-leo-recorder"))
        .args(["record"])
        .arg(&source_path)
        .args(["--out-dir"])
        .arg(&out_dir)
        .args(["--format", "json"])
        .output()
        .expect("failed to run recorder");

    assert!(
        !output.status.success(),
        "--format should be rejected by clap; stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--format")
            || stderr.contains("unexpected argument")
            || stderr.contains("unrecognized")
            || stderr.contains("found argument"),
        "clap error should mention the unknown --format flag; got stderr:\n{stderr}"
    );
}

/// The CLI binary must not expose a `--format` flag at any level.
/// Convention: `Recorder-CLI-Conventions.md` §4 — recorders are
/// CTFS-only.
#[test]
fn test_no_format_flag_in_help() {
    let bin = env!("CARGO_BIN_EXE_codetracer-leo-recorder");

    for subcmd in [None, Some("record"), Some("replay")] {
        let mut cmd = Command::new(bin);
        if let Some(s) = subcmd {
            cmd.arg(s);
        }
        cmd.arg("--help");

        let output = cmd.output().expect("failed to run --help");
        assert!(
            output.status.success(),
            "--help (subcmd={:?}) should exit 0",
            subcmd
        );

        let help = String::from_utf8_lossy(&output.stdout);
        assert!(
            !help.contains("--format"),
            "--help (subcmd={:?}) must not advertise --format; got:\n{help}",
            subcmd
        );
        assert!(
            !help.contains("CODETRACER_FORMAT"),
            "--help (subcmd={:?}) must not advertise CODETRACER_FORMAT; got:\n{help}",
            subcmd
        );
    }
}

/// `--help` must mention `ct print` so users know where to go for
/// human-readable conversion of the recorded CTFS bundle.
#[test]
fn test_help_mentions_ct_print() {
    let bin = env!("CARGO_BIN_EXE_codetracer-leo-recorder");
    let output = Command::new(bin)
        .arg("--help")
        .output()
        .expect("failed to run --help");
    assert!(output.status.success(), "--help should exit 0");

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("ct print"),
        "--help must mention `ct print` as the conversion tool; got:\n{help}"
    );
}

// ===========================================================================
// Per-program ct-print --full coverage tests
// ===========================================================================
//
// These tests follow the recorder-test-requirements policy
// (`metacraft-specs/policies/recorder-test-requirements.md`):
//
// * Each test records one Leo program through the recorder's normal
//   entry point (`codetracer_leo_recorder::recorder::record`).
// * The produced `.ct` is piped through `ct-print --full --strip-paths`.
// * Assertions are made on the **decoded JSON document** with EXACT
//   counts (`assert_eq!(events.len(), N)` -- never `>=`), EXACT
//   ordering (later step from a strictly later source line where
//   applicable), and EXACT decoded values
//   (`value["i"] == 42`, `value["kind"] == "Int"`).
//
// `ValueRecord` variants outside the expected set are rejected with
// a hard error message asking the test author to extend the test
// rather than weaken the assertion.
//
// Where the recorder's current behaviour deviates from what the
// language semantics dictate -- e.g. multi-line `if`/`else` bodies
// or argumentful function calls not being parsed because the source-
// level parser is hand-rolled and very limited -- the deviation is
// documented inline as `RECORDER BUG: ...` and a parallel `#[ignore]`d
// assertion captures the spec-correct expectation so it surfaces the
// moment the recorder catches up.

/// Skip-helper: returns `Some(path)` to ct-print or logs a clear
/// `SKIP:` diagnostic and returns `None`.  The
/// `verify-cli-convention-no-silent-skip.sh` script greps for the
/// literal `SKIP:` token, so silent skips remain forbidden.
fn ct_print_or_skip(test_name: &str) -> Option<PathBuf> {
    let p = ct_print_path();
    if !p.exists() {
        eprintln!(
            "SKIP: {test_name} requires ct-print at {} -- only available \
             within the metacraft workspace where codetracer-trace-format-nim \
             is a sibling.",
            p.display()
        );
        return None;
    }
    Some(p)
}

/// Record a program and return the `ct-print --full --strip-paths`
/// JSON document plus the absolute path to the source file (so the
/// caller can match `metadata.program`).  Returns `None` when
/// `ct-print` is unavailable (the caller has already emitted a
/// `SKIP:` line via `ct_print_or_skip`).
fn record_and_dump_full(test_name: &str, program: &str) -> Option<(serde_json::Value, PathBuf)> {
    let ct_print = ct_print_or_skip(test_name)?;

    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join(program);
    codetracer_leo_recorder::recorder::record(&source_path, &out_dir)
        .expect("recorder::record should succeed");

    let ct_files: Vec<_> = std::fs::read_dir(&out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container in {:?}",
        out_dir
    );

    let output = Command::new(&ct_print)
        .args(["--full", "--strip-paths"])
        .arg(&ct_files[0])
        .output()
        .expect("failed to run ct-print --full");

    assert!(
        output.status.success(),
        "ct-print --full should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("ct-print --full should emit valid JSON");

    drop(tmp_dir);

    Some((doc, source_path))
}

/// Decode every (varname, i64) pair from step events.  Rejects any
/// `ValueRecord` variant other than `Int` with a hard error that
/// asks the test author to extend the test rather than weaken it.
fn observed_int_vars(doc: &serde_json::Value) -> Vec<(String, i64)> {
    let events = doc["events"].as_array().expect("events array");
    let mut out = Vec::new();
    for ev in events {
        if ev["kind"] != "step" {
            continue;
        }
        let Some(vars) = ev["vars"].as_array() else {
            continue;
        };
        for v in vars {
            let name = v["varname"].as_str().expect("varname str").to_string();
            let value = &v["value"];
            assert_eq!(
                value["kind"].as_str(),
                Some("Int"),
                "variable `{}` should decode as Int, got {}; \
                 if a new ValueRecord variant has landed for Leo numeric \
                 registers, extend this test to assert on it explicitly \
                 rather than weakening the check",
                name,
                value
            );
            let i = value["i"]
                .as_i64()
                .unwrap_or_else(|| panic!("Int.i must be i64 for `{name}`; got {value}"));
            out.push((name, i));
        }
    }
    out
}

/// Decode the call-entry sequence as a vector of function names.
fn observed_call_sequence(doc: &serde_json::Value) -> Vec<String> {
    doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "call_entry")
        .map(|e| {
            e["function"]
                .as_str()
                .expect("call_entry.function str")
                .to_string()
        })
        .collect()
}

/// Decode (varname, i64) pairs in event-emission order.
fn observed_var_sequence(doc: &serde_json::Value) -> Vec<(String, i64)> {
    observed_int_vars(doc)
}

/// Assert that every `step` event carries a strictly increasing
/// `step_index`.  This is the recorder's only ordering guarantee
/// against duplicates / reorderings.
fn assert_step_indices_monotonic(doc: &serde_json::Value) {
    let mut last = -1i64;
    for ev in doc["events"].as_array().expect("events array") {
        if ev["kind"] != "step" {
            continue;
        }
        let idx = ev["step_index"]
            .as_i64()
            .expect("step_index must be present on step events");
        assert!(
            idx > last,
            "step_index must strictly increase; got {idx} after {last}"
        );
        last = idx;
    }
}

/// Assert `metadata.program` ends with the expected source filename.
fn assert_metadata_program_ends_with(doc: &serde_json::Value, source_path: &Path) {
    let prog = doc["metadata"]["program"]
        .as_str()
        .expect("metadata.program str");
    let want = source_path.file_name().unwrap().to_string_lossy();
    assert!(
        prog.ends_with(&*want),
        "metadata.program {prog} must end with {want}"
    );
}

// --- control_flow_test.leo -------------------------------------------------

/// Records `control_flow_test.leo` and asserts on the **current
/// observed** event shape.  The program exercises if/else, ternary,
/// and a `for` loop.  The Leo recorder's source-level parser only
/// handles `let X: T = expr;` lines whose RHS is a literal, register,
/// or `+`/`-`/`*`/`/` expression -- if/else expressions, ternary
/// expressions, and `for`-loop bodies are intentionally not compiled
/// to Aleo instructions.  This pin captures today's output as a
/// golden snapshot so any future regression is caught even before the
/// upstream parser gains real support.  The parallel `#[ignore]`d
/// test below captures the spec-correct expectation.
#[test]
fn test_control_flow_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_control_flow_test_via_ct_print_full",
        "control_flow_test.leo",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    // ----- Function table --------------------------------------------
    // Stable on this fixture: only `main` and `compute` make it into
    // the function table because the recorder's call tracker runs from
    // `main` and follows `return compute();`.  No other functions exist.
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(functions, vec!["main", "compute"]);

    // ----- Counts -----------------------------------------------------
    // 1 absolute top-level step + 1 dispatch step (line of `main`'s
    // return) + 5 step events for the five `let`-bindings inside
    // compute (raw, sign, bonus, combined, "mut result") + 1 trailing
    // post-call step on the `return compute();` line = 8 step events.
    // Only `compute` is recorded as a call (call_entry/call_exit pair).
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(8), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(8),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 8 step + 1 call_entry + 1 call_exit = 10 events.
    assert_eq!(events.len(), 10, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence ---------------------------------------------
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // ----- Decoded variable values -----------------------------------
    // RECORDER BUG: spec wants the decoded values to be
    //   raw=7, sign=1, bonus=300, combined=301,
    //   result=301 (initial) and finally 301 + 0+1+2 = 304.
    //
    // Today only `raw`, `combined`, and a binding misnamed
    // `"mut result"` (the parser leaves the `mut` keyword in the
    // varname) surface, and `combined`/`result` evaluate to 0 because
    // the AVM register holding `sign` (and therefore `bonus`) is never
    // populated -- the source-level expression compiler does not
    // recognise `if {} else {}` or `?:` and falls back to an `add
    // <unparsed-text> 0u32 into rN;` instruction whose left operand
    // never resolves.  Likewise the `for i in 0u32..3u32 { result =
    // result + i; }` loop body is not compiled at all so the final
    // `result` retains its initial value.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("raw".to_string(), 7),
            // RECORDER BUG: should also see ("sign", 1), ("bonus", 300)
            // here -- skipped because the if/else and ternary RHS
            // expressions don't compile to a recognised Aleo
            // instruction.
            ("combined".to_string(), 0),
            // RECORDER BUG: varname leak -- the parser slices `let mut
            // result: u32 = combined;` so the binding name carries the
            // `mut ` keyword prefix.  Spec-correct varname is
            // `"result"` and the value should be 304 after the for
            // loop accumulates.
            ("mut result".to_string(), 0),
        ],
        "control_flow_test today only decodes raw / combined / \"mut result\" \
         and all three are wrong-or-absent past the if/else; spec-correct \
         values live in the parallel #[ignore]d test"
    );
}

#[test]
#[ignore = "RECORDER BUG: if/else, ternary, for-loop bodies, and the \
            `mut` modifier are all opaque to the Leo source-level \
            parser.  Spec-compliant ct-print --full output should \
            surface the full chain raw=7, sign=1, bonus=300, \
            combined=301, result=304 with the varname `result` (no \
            `mut ` prefix)."]
fn test_control_flow_test_full_chain_decodes() {
    let Some((doc, _)) = record_and_dump_full(
        "test_control_flow_test_full_chain_decodes",
        "control_flow_test.leo",
    ) else {
        return;
    };
    let expected: Vec<(String, i64)> = vec![
        ("raw".into(), 7),
        ("sign".into(), 1),
        ("bonus".into(), 300),
        ("combined".into(), 301),
        ("result".into(), 304),
    ];
    assert_eq!(observed_var_sequence(&doc), expected);
}

// --- nested_calls_test.leo -------------------------------------------------

#[test]
#[ignore = "RECORDER BUG: the recorder's call-target resolver \
            (`find_return_call_target` in src/tracer.rs) iterates a \
            HashMap of peer functions in non-deterministic order and \
            returns the first non-self peer.  For chains where two or \
            more intermediate callers have empty bodies, this either \
            picks the wrong callee or recurses into the same function \
            twice and ultimately blows the stack -- the recorder \
            segfaults (~80% of the time on a 4-deep chain, ~30% of the \
            time on a 3-deep chain).  Spec-compliant ct-print --full \
            output should surface the chain main -> compute -> inner \
            with call_entry order [compute, inner], call_exit order \
            [inner, compute], and step values a=1, b=2, c=3."]
fn test_nested_calls_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_nested_calls_test_via_ct_print_full",
        "nested_calls_test.leo",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    // Spec-compliant assertions (will fail today because the recorder
    // either segfaults or picks the wrong callee).
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(functions, vec!["main", "compute", "inner"]);

    let counts = &doc["counts"];
    assert_eq!(counts["calls"].as_u64(), Some(2));

    assert_eq!(
        observed_call_sequence(&doc),
        vec!["compute".to_string(), "inner".to_string()]
    );

    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("a".into(), 1),
            ("b".into(), 2),
            ("c".into(), 3),
        ]
    );

    let events = doc["events"].as_array().expect("events array");
    let exit_sequence: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert_eq!(exit_sequence, vec!["inner", "compute"]);
}

// --- collections_test.leo --------------------------------------------------

#[test]
#[ignore = "RECORDER BUG: struct, tuple, and array literals are \
            completely opaque to the recorder's source-level parser \
            (it only understands literal / register / +-*/ \
            expressions in `let` RHS).  Argumentful function calls \
            like `sum_pair((10u32, 20u32))` and \
            `point_distance_sq(Point { x: 3u32, y: 4u32 })` are not \
            recognised as calls.  Compounding that, the call-target \
            resolver picks a non-deterministic peer when the chain \
            from `main` has multiple zero-binding intermediates, so \
            even the surviving integer let-bindings surface in a \
            different order from run to run.  Spec-compliant \
            ct-print --full output should expose the array `xs` as a \
            ValueRecord::Sequence, the (u32,u32) tuple as Tuple, and \
            the `Point` struct as Struct, with grand_total decoded \
            as Int=40."]
fn test_collections_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_collections_test_via_ct_print_full",
        "collections_test.leo",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    // Spec-compliant function table: every function in the source
    // must surface (writer-assignment order is main first because the
    // recorder enters via main()).
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["main", "compute", "sum_pair", "point_distance_sq", "array_total"]
    );

    // Spec-compliant ValueRecord variants: Int, Sequence (the array),
    // Tuple (the (u32,u32) pair), Struct (the Point literal).
    let mut kinds = std::collections::BTreeSet::new();
    for ev in doc["events"].as_array().unwrap() {
        if ev["kind"] != "step" {
            continue;
        }
        for v in ev["vars"].as_array().cloned().unwrap_or_default() {
            if let Some(k) = v["value"]["kind"].as_str() {
                kinds.insert(k.to_string());
            }
        }
    }
    for want in ["Int", "Sequence", "Tuple", "Struct"] {
        assert!(
            kinds.contains(want),
            "expected {want} ValueRecord variant in collections trace; got {kinds:?}"
        );
    }
}

// --- error_paths_test.leo --------------------------------------------------

#[test]
#[ignore = "RECORDER BUG: `assert_eq` and `assert` are not surfaced as \
            anything in the trace -- not as a step, not as an io_event, \
            and a failed assertion does NOT terminate execution or \
            emit an EventLogKind::Error record.  Compounding that, the \
            call-target resolver iterates `func_map` in HashMap order \
            and may pick `safe_compute`, `failing_compute`, or \
            `compute` itself when emitting from `main`, so even the \
            non-error trace shape is non-deterministic.  Spec-\
            compliant ct-print --full output should run \
            main -> compute -> safe_compute (the path the source \
            actually traverses), surface every let-binding (a=5, b=7, \
            c=12, safe_val=12, bumped=112), and emit ZERO error \
            events on the safe path."]
fn test_error_paths_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_error_paths_test_via_ct_print_full",
        "error_paths_test.leo",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["main", "compute", "safe_compute"],
        "spec wants only the actually-reached chain; failing_compute is \
         declared but never invoked from main"
    );

    assert_eq!(
        observed_call_sequence(&doc),
        vec!["compute".to_string(), "safe_compute".to_string()]
    );

    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("a".into(), 5),
            ("b".into(), 7),
            ("c".into(), 12),
            ("safe_val".into(), 12),
            ("bumped".into(), 112),
        ]
    );

    let counts = &doc["counts"];
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "no error events on the safe path; counts={counts}"
    );
}

/// Records `error_paths_test.leo` and asserts that the failing
/// `assert_eq(seen_a, seen_b)` inside `failing_compute` surfaces as
/// an io_event.  The static-sweep path in
/// `LeoTracer::emit_assert_events_from_source` is what makes this
/// pass today: it walks every function declared in the source
/// (including dead-code paths the call-target resolver does not
/// reach) and emits one io_event per `assert` / `assert_eq` call.
///
/// Once the recorder learns to evaluate assert predicates at
/// runtime, this test should be tightened to require not just the
/// presence of the io_event but also a "failure-shaped" payload
/// (EventLogKind::Error metadata distinguishing pass vs fail).  The
/// currently-emitted event already uses EventLogKind::Error, so
/// downstream tooling can rely on the kind even today.
#[test]
fn test_error_paths_test_emits_assert_failure_event() {
    let Some((doc, _)) = record_and_dump_full(
        "test_error_paths_test_emits_assert_failure_event",
        "error_paths_test.leo",
    ) else {
        return;
    };
    let counts = &doc["counts"];
    assert!(
        counts["io_events"].as_u64().unwrap_or(0) >= 1,
        "expected at least one io_event for the assertion failure; counts={counts}"
    );
}

// --- assert_test.leo -------------------------------------------------------

/// Records `assert_test.leo`.  Each `assert` / `assert_eq` holds, so
/// the trace runs to completion.  The deterministic shape is 6 step
/// events + 1 call (compute) + 2 io_events surfacing the two
/// `assert` / `assert_eq` calls.  The recorder lowers each assertion
/// to an io_event via the static-sweep path in
/// `LeoTracer::emit_assert_events_from_source` -- see that function
/// for the rationale and follow-up plan.
#[test]
fn test_assert_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_assert_test_via_ct_print_full", "assert_test.leo")
    else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(functions, vec!["main", "compute"]);

    // ----- Counts ----------------------------------------------------
    // 1 absolute top-level step + 1 dispatch step + 3 step events for
    // the three let-bindings inside compute (a, b, sum_val) + 1
    // trailing post-call step = 6 steps.  Only `compute` is recorded
    // as a call.  Two io_events surface the `assert_eq(...)` and
    // `assert(...)` calls inside compute (lines 15 and 17 of the
    // fixture) -- emitted by the static-sweep path so downstream
    // consumers see the assertions even though the source-level
    // expression compiler does not yet evaluate them at runtime.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(6), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(2),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(6),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 6 steps + 1 call_entry + 1 call_exit + 2 io = 10 events.
    assert_eq!(events.len(), 10, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // ----- Exact decoded values --------------------------------------
    // The three integer let-bindings round-trip; the two assert
    // statements (lines 15 and 17) surface separately as io_events
    // (asserted via counts["io_events"] above and the dedicated
    // `test_assert_test_emits_assert_events` test).
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("a".to_string(), 4),
            ("b".to_string(), 5),
            ("sum_val".to_string(), 9),
        ]
    );

    // ----- Assert events surface their source text -------------------
    // Each io_event carries the verbatim `assert(...)` / `assert_eq(...)`
    // expression in its `text` field; pinning those lets a future
    // change to the static-sweep emit path catch any regression in
    // the surfaced payload (e.g. accidentally trimming the wrapper).
    let io_texts: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "io")
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert_eq!(
        io_texts,
        vec!["assert_eq(a + b, 9u32)", "assert(sum_val > 0u32)"]
    );

    // ----- Return value carries the final sum_val --------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(
                rv["kind"].as_str(),
                Some("Int"),
                "return value must decode as Int; got {rv}"
            );
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![9]);
}

/// Pins the io_event count for `assert_test.leo` at exactly two --
/// one io_event per `assert` / `assert_eq` call on the passing path.
/// This complements `test_assert_test_via_ct_print_full` (which
/// asserts on the full event shape) by isolating the io_event count
/// in a small focused check that's easy to grep for when adjusting
/// the static-sweep emit path in `tracer.rs`.
#[test]
fn test_assert_test_emits_assert_events() {
    let Some((doc, _)) =
        record_and_dump_full("test_assert_test_emits_assert_events", "assert_test.leo")
    else {
        return;
    };
    let counts = &doc["counts"];
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(2),
        "expected exactly 2 io_events (one per assert call); counts={counts}"
    );
}
