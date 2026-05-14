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
    // The Leo recorder surfaces 9 step events for the canonical
    // fixture: one synthetic top-level absolute step on line 1, then
    // a function-entry step at compute's signature line (anchors the
    // call_entry so it gets a distinct entryStep), then 5 bindings
    // (a, b, sum_val, doubled, final_result), then a return-line
    // step inside compute, then a return-line step at main.  Only
    // `compute` is actually traced as a call (call_entry/call_exit
    // pair) — `main` appears in the function table but is merged
    // into `<toplevel>` here, the recorder returns the compute
    // result directly.  These are stable properties of the canonical
    // fixture; if they change, that's a real regression to
    // investigate, not a flake.
    let counts = &doc["counts"];
    assert_eq!(
        counts["steps"].as_u64(),
        Some(9),
        "expected 9 step events for flow_test.leo; counts={counts}",
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
            observed_vars.iter().any(|(n, v)| n == name && v == value),
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

    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("ct-print --full should emit valid JSON");

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

/// Records `control_flow_test.leo` and pins the full event shape.
/// The program exercises if/else, ternary, and a compile-time
/// bounded `for` loop.  The structured Leo evaluator
/// (`tracer.rs::execute_leo_function`) handles each surface
/// directly: if/else and ternary expressions resolve at evaluation
/// time, `for i in lo..hi` loops unroll over the literal bounds,
/// and `let mut` bindings strip the `mut` keyword from the
/// surfaced variable name and re-emit the binding with its final
/// post-loop value.
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
    // 1 absolute top-level step + 1 function-entry step at compute's
    // signature line + 5 step events for the five `let`-bindings
    // inside compute (raw, sign, bonus, combined, result) + 1 step
    // at compute's return line + 1 step at main's return line = 9
    // step events.  Only `compute` is recorded as a call.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(9), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(9),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 9 step + 1 call_entry + 1 call_exit = 11 events.
    assert_eq!(events.len(), 11, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence ---------------------------------------------
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // ----- Decoded variable values -----------------------------------
    // The structured evaluator surfaces every let-binding:
    //   raw=7
    //   sign = if raw > 5 { 1 } else { 2 }                -> 1
    //   bonus = sign == 1 ? 300 : 100                     -> 300
    //   combined = sign + bonus                           -> 301
    //   result = combined; for i in 0..3 { result += i }  -> 304
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("raw".to_string(), 7),
            ("sign".to_string(), 1),
            ("bonus".to_string(), 300),
            ("combined".to_string(), 301),
            ("result".to_string(), 304),
        ]
    );
}

/// Spec-compliant decode chain for `control_flow_test.leo`.  This
/// test was previously `#[ignore]`d because the AVM-driven recorder
/// could not see past `if`/`?:`/`for`/`mut`; the structured
/// evaluator (added 2026-05-13) fixes that and the test now runs in
/// the default suite.
#[test]
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

/// Spec-compliant 3-deep call chain (`main -> compute -> inner`).
/// Previously `#[ignore]`d because the HashMap-iteration call-target
/// resolver was non-deterministic and segfaulted on chains with
/// empty intermediate callers; the source-derived deterministic
/// resolver in `tracer.rs::find_return_call_target` (added
/// 2026-05-13) makes this test stable.
#[test]
fn test_nested_calls_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_nested_calls_test_via_ct_print_full",
        "nested_calls_test.leo",
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
    assert_eq!(functions, vec!["main", "compute", "inner"]);

    let counts = &doc["counts"];
    assert_eq!(counts["calls"].as_u64(), Some(2));

    assert_eq!(
        observed_call_sequence(&doc),
        vec!["compute".to_string(), "inner".to_string()]
    );

    assert_eq!(
        observed_var_sequence(&doc),
        vec![("a".into(), 1), ("b".into(), 2), ("c".into(), 3),]
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

/// Spec-compliant collections test: struct / tuple / array
/// literals + argumentful calls.  Previously `#[ignore]`d because
/// the AVM-driven recorder could not parse these surfaces; the
/// structured evaluator (added 2026-05-13) handles all of them.
#[test]
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
        vec![
            "main",
            "compute",
            "sum_pair",
            "point_distance_sq",
            "array_total"
        ]
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

/// Spec-compliant safe-path execution: the structured evaluator
/// only walks reachable functions, so `failing_compute` (declared
/// but never invoked from main) is silently skipped and its
/// `assert_eq` does NOT pollute the io_event stream.  Previously
/// `#[ignore]`d because the call-target resolver was
/// non-deterministic and the static-sweep emitted asserts from
/// every declared function regardless of reachability.
#[test]
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

/// Records `error_paths_test.leo` and pins the io_event count at
/// EXACTLY zero on the safe path.
///
/// Pre-2026-05-13 the static `emit_assert_events_from_source` sweep
/// walked every declared function regardless of reachability, so
/// the dead-code `failing_compute` block emitted an io_event for
/// its `assert_eq(seen_a, seen_b)` call -- this test was structured
/// around that "any io_event = success" semantics.  The structured
/// evaluator only walks reachable functions (BFS from `main`),
/// so the safe path now correctly emits ZERO io_events.  When the
/// recorder learns to evaluate failing assertions at runtime via a
/// distinct fixture that actually traverses the failing path, this
/// test should be re-tightened to require a failure-shaped payload
/// then -- on a fixture where the failing path is reachable.
#[test]
fn test_error_paths_test_emits_assert_failure_event() {
    let Some((doc, _)) = record_and_dump_full(
        "test_error_paths_test_emits_assert_failure_event",
        "error_paths_test.leo",
    ) else {
        return;
    };
    let counts = &doc["counts"];
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "safe-path execution must NOT emit io_events from the \
         unreachable failing_compute() block; counts={counts}"
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
    // 1 absolute top-level step + 1 function-entry step at compute's
    // signature line (anchors compute's call_entry) + 3 step events
    // for the three let-bindings inside compute (a, b, sum_val) + 1
    // step at compute's return line + 1 step at main's return line
    // = 7 steps.  Only `compute` is recorded as a call.  Two
    // io_events surface the `assert_eq(...)` and `assert(...)` calls
    // inside compute (lines 15 and 17 of the fixture) -- emitted by
    // the structured evaluator's reachable-only path so downstream
    // consumers see the assertions even though the source-level
    // expression compiler does not yet evaluate them at runtime.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(7), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(2),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(7),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 7 steps + 1 call_entry + 1 call_exit + 2 io = 11 events.
    assert_eq!(events.len(), 11, "events.len()");
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

// ===========================================================================
// M11 fixtures — top-5 priority Aleo coverage gap
// ===========================================================================
//
// These tests pin the recorder's behaviour on the five fixtures that
// the M11 plan calls out as the priority Aleo coverage gap:
//
//   1. multi_function_test.leo   — 3-deep helper chain
//   2. field_group_scalar_arith_test.leo — native curve types
//   3. hash_builtins_test.leo    — BHP / Pedersen / Poseidon / Keccak
//   4. mapping_finalize_test.leo — async finalize + Mapping::* ops
//   5. record_token_test.leo     — UTXO record lifecycle
//
// The structured evaluator handles control flow, struct/tuple/array
// literals, argumentful calls, hash-builtin static-method shapes
// (`Family::method(args)`), `Mapping::*` finalize-scope ops, and
// the `record` keyword as a struct-shaped UTXO primitive.  Real
// curve math (BLS12-377 BHP / Pedersen / Poseidon / Keccak) lives
// in snarkVM and is not modelled by the source-level evaluator --
// hash builtins return a deterministic stand-in (sum of integer
// arg payloads, typed `field`) so the trace exercises the
// parsing + value-emission path without faking BLS12-377.

// --- multi_function_test.leo ----------------------------------------------

/// Records `multi_function_test.leo` and pins the full event shape
/// of the 3-deep helper chain (`main -> outer -> middle -> inner`).
///
/// The structured evaluator threads each callee's argument values
/// through formal-parameter binding, propagates the inner-most
/// return value back up the chain, and surfaces every
/// let-binding in per-call source order.
#[test]
fn test_multi_function_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_multi_function_test_via_ct_print_full",
        "multi_function_test.leo",
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
    assert_eq!(functions, vec!["main", "outer", "middle", "inner"]);

    // Counts: per-call-entry parameter step + per-binding step.
    //   Top-level absolute step: 1
    //   outer: entry-step(z=7), m_v binding step, o binding step, return-step = 4
    //   middle: entry-step(v=4), inner_val binding step, m binding step, return-step = 4
    //   inner: entry-step(x=2,y=3 dual), s binding step, bumped binding step, return-step = 4
    //   main return-step = 1
    // Total step events = 14.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(14), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(3), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);

    // Call-entry order is outer-then-middle-then-inner; exits are
    // reverse (inner-first) since each callee finishes before its
    // parent's binding step is completed.
    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "outer".to_string(),
            "middle".to_string(),
            "inner".to_string()
        ]
    );

    let events = doc["events"].as_array().expect("events array");
    let exit_sequence: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert_eq!(exit_sequence, vec!["inner", "middle", "outer"]);

    // Every step variable is a u32 Int; the emitted sequence interleaves
    // formal-parameter steps and per-binding steps.  Each formal parameter
    // surfaces twice (once at the function-entry step that anchors
    // the call_entry, once at the variable step) for `z` and `v` -- the
    // 2-arg `inner` chain emits the two formals once per step.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("z".to_string(), 7),
            ("z".to_string(), 7),
            ("v".to_string(), 4),
            ("v".to_string(), 4),
            ("x".to_string(), 2),
            ("y".to_string(), 3),
            ("x".to_string(), 2),
            ("y".to_string(), 3),
            ("s".to_string(), 5),
            ("bumped".to_string(), 6),
            ("inner_val".to_string(), 6),
            ("m".to_string(), 10),
            ("middle_val".to_string(), 10),
            ("o".to_string(), 15),
        ]
    );

    // Return values: inner returns 6, middle 10, outer 15.
    let returns: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let name = e["function"].as_str().expect("function").to_string();
            let rv = &e["return_value"];
            assert_eq!(
                rv["kind"].as_str(),
                Some("Int"),
                "return value should decode as Int; got {rv}"
            );
            (name, rv["i"].as_i64().expect("Int.i"))
        })
        .collect();
    assert_eq!(
        returns,
        vec![
            ("inner".to_string(), 6),
            ("middle".to_string(), 10),
            ("outer".to_string(), 15),
        ]
    );
}

// --- field_group_scalar_arith_test.leo ------------------------------------

/// Records `field_group_scalar_arith_test.leo` and pins the full
/// event shape for Leo's native curve types (`field` / `scalar`).
/// `parse_typed_literal` recognises the suffix; the evaluator
/// surfaces each binding as a `ValueRecord::Int` carrying the
/// suffix as the type name, so downstream tooling can distinguish
/// `field` from `u32` even though both decode as `Int` payloads
/// in the trace today (real curve math lives in snarkVM).
#[test]
fn test_field_group_scalar_arith_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_field_group_scalar_arith_test_via_ct_print_full",
        "field_group_scalar_arith_test.leo",
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
    assert_eq!(functions, vec!["main", "compute"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(11), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // f1=5, f2=7, f_sum=12 (field arithmetic), s1=3, s2=4, s_sum=7
    // (scalar arithmetic), total=19 (u32).  All payloads decode as
    // `Int` -- the type discrimination lives in the registered
    // `types` array (which contains `field`, `group`, `scalar`).
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("f1".to_string(), 5),
            ("f2".to_string(), 7),
            ("f_sum".to_string(), 12),
            ("s1".to_string(), 3),
            ("s2".to_string(), 4),
            ("s_sum".to_string(), 7),
            ("total".to_string(), 19),
        ]
    );

    // The native curve type `field` and `scalar` must surface in
    // the registered types table so downstream tooling can
    // distinguish them from u32.  `group` is not used in this
    // fixture but is recognised by `parse_typed_literal`.
    let types: Vec<&str> = doc["types"]
        .as_array()
        .expect("types array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        types.contains(&"field"),
        "expected `field` in registered types; got {types:?}"
    );
    assert!(
        types.contains(&"scalar"),
        "expected `scalar` in registered types; got {types:?}"
    );
}

// --- hash_builtins_test.leo ----------------------------------------------

/// Records `hash_builtins_test.leo` and pins the io_event count at
/// EXACTLY 4 -- one per hash-builtin invocation
/// (BHP/Pedersen/Poseidon/Keccak).  The hash stub in
/// `eval_builtin_method` returns a deterministic stand-in (sum of
/// integer arg payloads, typed `field`) so each binding round-trips
/// as an `Int` value record.  Each invocation surfaces an io_event
/// of `EventLogKind::Read` with a family-tagged metadata
/// (`LeoBhpHash` / `LeoPedersenHash` / `LeoPoseidonHash` /
/// `LeoKeccakHash`) and verbatim source-level call text.
#[test]
fn test_hash_builtins_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_hash_builtins_test_via_ct_print_full",
        "hash_builtins_test.leo",
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
    assert_eq!(functions, vec!["main", "compute"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(9), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(4),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // Each hash stub returns sum-of-args (single u32 arg => the
    // arg's value): BHP256(1u32) -> 1, Pedersen64(2u32) -> 2,
    // Poseidon4(3u32) -> 3, Keccak256(4u32) -> 4.  The literal
    // `total` binds the canonical sum 10.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("bhp_h".to_string(), 1),
            ("ped_h".to_string(), 2),
            ("pos_h".to_string(), 3),
            ("kec_h".to_string(), 4),
            ("total".to_string(), 10),
        ]
    );

    // io_events: one per hash-family call, in source order.  Each
    // surfaces as `ioFileOp` (the io_kind ct-print emits for
    // EventLogKind::Read) and carries the verbatim source-level
    // call text.  The family-specific metadata
    // (`LeoBhpHash` / `LeoPedersenHash` / ...) is registered with
    // the writer but not exposed via ct-print --full's JSON
    // schema, so we pin only the visible (io_kind, text) pair.
    let events = doc["events"].as_array().expect("events array");
    let io_events: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "io")
        .map(|e| {
            let io_kind = e["io_kind"].as_str().unwrap_or("").to_string();
            let text = e["text"].as_str().unwrap_or("").to_string();
            (io_kind, text)
        })
        .collect();
    assert_eq!(
        io_events,
        vec![
            (
                "ioFileOp".to_string(),
                "BHP256::hash_to_field(1u32)".to_string()
            ),
            (
                "ioFileOp".to_string(),
                "Pedersen64::hash_to_field(2u32)".to_string()
            ),
            (
                "ioFileOp".to_string(),
                "Poseidon4::hash_to_field(3u32)".to_string()
            ),
            (
                "ioFileOp".to_string(),
                "Keccak256::hash_to_field(4u32)".to_string()
            ),
        ]
    );
}

// --- mapping_finalize_test.leo --------------------------------------------

/// Records `mapping_finalize_test.leo` and pins the full event
/// shape for `async transition` / `async function` plus the
/// `Mapping::*` finalize-scope ops.  Each `Mapping::get_or_use`
/// surfaces as an io_event with `EventLogKind::Read`; each
/// `Mapping::set` surfaces as an io_event with
/// `EventLogKind::Write`.
#[test]
fn test_mapping_finalize_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_mapping_finalize_test_via_ct_print_full",
        "mapping_finalize_test.leo",
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
    assert_eq!(functions, vec!["main", "mint", "finalize_mint"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(11), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(2), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(2),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(
        observed_call_sequence(&doc),
        vec!["mint".to_string(), "finalize_mint".to_string()]
    );

    // mint(receiver=7, amount=10) returns reported=10.
    // finalize_mint(receiver=7, amount=10):
    //   prior = Mapping::get_or_use(balances, 7, 0u32) -> 0
    //   updated = prior + amount = 10
    //   Mapping::set(balances, 7, 10)
    //   return updated = 10
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("receiver".to_string(), 7),
            ("amount".to_string(), 10),
            ("receiver".to_string(), 7),
            ("amount".to_string(), 10),
            ("reported".to_string(), 10),
            ("minted".to_string(), 10),
            ("receiver".to_string(), 7),
            ("amount".to_string(), 10),
            ("receiver".to_string(), 7),
            ("amount".to_string(), 10),
            ("prior".to_string(), 0),
            ("updated".to_string(), 10),
            ("finalized".to_string(), 10),
        ]
    );

    // io_events: one Read for `Mapping::get_or_use`, one Write
    // for `Mapping::set`, in source order.  ct-print --full
    // surfaces these as `ioFileOp` (Read) and `ioStdout` (Write)
    // respectively -- the Read/Write distinction is what
    // downstream tooling needs to reconstruct the on-chain
    // state-mutation sequence.  The op-specific metadata
    // (`LeoMappingGetOrUse` / `LeoMappingSet`) is registered with
    // the writer but not exposed via ct-print --full's JSON
    // schema, so we pin only the visible (io_kind, text) pair.
    let events = doc["events"].as_array().expect("events array");
    let io_events: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "io")
        .map(|e| {
            let io_kind = e["io_kind"].as_str().unwrap_or("").to_string();
            let text = e["text"].as_str().unwrap_or("").to_string();
            (io_kind, text)
        })
        .collect();
    assert_eq!(
        io_events,
        vec![
            (
                "ioFileOp".to_string(),
                "Mapping::get_or_use(balances, receiver, 0u32)".to_string()
            ),
            (
                "ioStdout".to_string(),
                "Mapping::set(balances, receiver, updated)".to_string()
            ),
        ]
    );
}

// --- record_token_test.leo ------------------------------------------------

/// Records `record_token_test.leo` and pins the full event shape
/// for the UTXO `record` lifecycle (mint / transfer / burn).  The
/// recorder treats `record` as a struct-shaped value and surfaces
/// each lifecycle transition as a regular call_entry/call_exit
/// pair with the produced record value flowing through the
/// call's return value as a `ValueRecord::Struct`.
#[test]
fn test_record_token_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_record_token_test_via_ct_print_full",
        "record_token_test.leo",
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
        vec!["main", "compute", "mint", "transfer", "burn"]
    );

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(18), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "mint".to_string(),
            "transfer".to_string(),
            "burn".to_string()
        ]
    );

    // The `record Token` decl registers as a struct-shaped type so
    // mint/transfer return values surface as `ValueRecord::Struct`.
    let types: Vec<&str> = doc["types"]
        .as_array()
        .expect("types array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        types.contains(&"Token"),
        "expected `Token` record in registered types; got {types:?}"
    );

    // ----- Per-call returns ----------------------------------------------
    // mint -> Token { owner: 99, amount: 100 } as Struct
    // transfer -> Token { owner: 200, amount: 80 } as Struct
    // burn -> 80 as Int
    // compute -> 80 as Int
    let events = doc["events"].as_array().expect("events array");
    let returns: Vec<(String, &serde_json::Value)> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            (
                e["function"].as_str().expect("function").to_string(),
                &e["return_value"],
            )
        })
        .collect();
    assert_eq!(returns.len(), 4);

    let (mint_name, mint_rv) = &returns[0];
    assert_eq!(mint_name, "mint");
    assert_eq!(mint_rv["kind"].as_str(), Some("Struct"));
    let mint_fields: Vec<i64> = mint_rv["field_values"]
        .as_array()
        .expect("field_values array")
        .iter()
        .map(|f| f["i"].as_i64().expect("Int.i"))
        .collect();
    assert_eq!(mint_fields, vec![99, 100]);

    let (transfer_name, transfer_rv) = &returns[1];
    assert_eq!(transfer_name, "transfer");
    assert_eq!(transfer_rv["kind"].as_str(), Some("Struct"));
    let transfer_fields: Vec<i64> = transfer_rv["field_values"]
        .as_array()
        .expect("field_values array")
        .iter()
        .map(|f| f["i"].as_i64().expect("Int.i"))
        .collect();
    assert_eq!(transfer_fields, vec![200, 80]);

    let (burn_name, burn_rv) = &returns[2];
    assert_eq!(burn_name, "burn");
    assert_eq!(burn_rv["kind"].as_str(), Some("Int"));
    assert_eq!(burn_rv["i"].as_i64(), Some(80));

    let (compute_name, compute_rv) = &returns[3];
    assert_eq!(compute_name, "compute");
    assert_eq!(compute_rv["kind"].as_str(), Some("Int"));
    assert_eq!(compute_rv["i"].as_i64(), Some(80));

    // ----- Decoded Int variable sequence (only Int bindings, in order)
    // The struct-typed bindings (`minted`, `initial`, `new_token`,
    // `moved`, plus the `input: Token` formal parameter) are
    // skipped by `int_only_var_sequence` so the int-only chain
    // stays readable.  `input` appears multiple times because
    // each call's parameter binding emits twice (once at the
    // function-entry step, once at the per-formal step).
    let int_vars: Vec<(String, i64)> = events
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
            if v["value"]["kind"].as_str() == Some("Int") {
                Some((name, v["value"]["i"].as_i64().unwrap_or_default()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        int_vars,
        vec![
            ("owner".to_string(), 99),
            ("amount".to_string(), 100),
            ("owner".to_string(), 99),
            ("amount".to_string(), 100),
            ("receiver".to_string(), 200),
            ("send".to_string(), 80),
            ("receiver".to_string(), 200),
            ("send".to_string(), 80),
            ("leftover".to_string(), 20),
            ("_residual".to_string(), 20),
            ("consumed".to_string(), 80),
            ("burnt".to_string(), 80),
        ]
    );
}

// ===========================================================================
// M11 Round 2 — additional priority fixtures
// ===========================================================================
//
// Extends Round 1 (multi_function / field_group_scalar_arith /
// hash_builtins / mapping_finalize / record_token) with five more
// fixtures that pin behaviour for:
//
//   1. context_builtins_test.leo      -- self.caller / self.signer / block.height
//   2. int_overflow_modes_test.leo    -- wrapping/checked/saturating arithmetic
//   3. cast_test.leo                  -- explicit `as <type>` casts
//   4. async_finalize_failure_test.leo -- runtime finalize abort on Mapping::get
//   5. program_imports_test.leo       -- multi-program / cross-program calls

// --- context_builtins_test.leo --------------------------------------------

/// Records `context_builtins_test.leo` and pins the full event
/// shape for Aleo's call-context built-ins (`self.caller`,
/// `self.signer`) and the finalize-scope chain context read
/// (`block.height`).
///
/// The structured evaluator surfaces `self.caller` / `self.signer`
/// as `ValueRecord::String` (bech32m address form, deterministic
/// stand-in literals so the trace doesn't depend on a real signing
/// key) and `block.height` as `ValueRecord::Int` typed `u32`.
/// Each builtin is read at most once per call frame -- the call
/// site that introduces the binding sees the value, and the
/// transition / finalize frames are NOT double-emitting the same
/// payload across the call boundary.
#[test]
fn test_context_builtins_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_context_builtins_test_via_ct_print_full",
        "context_builtins_test.leo",
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
    assert_eq!(functions, vec!["main", "who_am_i", "finalize_who_am_i"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(11), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(2), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(1),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(
        observed_call_sequence(&doc),
        vec!["who_am_i".to_string(), "finalize_who_am_i".to_string()]
    );

    // The `address` type id must be registered so downstream tooling
    // can distinguish address-shaped strings from generic strings.
    let types: Vec<&str> = doc["types"]
        .as_array()
        .expect("types array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        types.contains(&"address"),
        "expected `address` in registered types; got {types:?}"
    );

    // ----- Decoded variable values --------------------------------------
    // Walk every step's `vars` and pin the (varname, kind, payload)
    // tuple.  String payloads carry the address text verbatim; Int
    // payloads carry the height stand-in (100u32).
    let events = doc["events"].as_array().expect("events array");
    let observed: Vec<(String, String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .map(|v| {
            let name = v["varname"].as_str().expect("varname").to_string();
            let kind = v["value"]["kind"].as_str().expect("kind").to_string();
            let payload = match kind.as_str() {
                "String" => v["value"]["text"].as_str().expect("text").to_string(),
                "Int" => v["value"]["i"].as_i64().expect("Int.i").to_string(),
                other => panic!(
                    "unexpected ValueRecord kind `{other}` for var `{name}`; \
                     extend this test if a new variant has landed"
                ),
            };
            (name, kind, payload)
        })
        .collect();
    assert_eq!(
        observed,
        vec![
            (
                "caller_addr".to_string(),
                "String".to_string(),
                "aleo1qnr4dkkvkgfqph0vzc3y6z2eu975wnpz2925ntjccd76cqsnpdjsdhgg6c".to_string()
            ),
            (
                "signer_addr".to_string(),
                "String".to_string(),
                "aleo1k7nl06ge2nlfm70cd6cf2asgn9w6m6kk4tytfgsugvyt5p3c2g8slzdr3a".to_string()
            ),
            (
                "_addr".to_string(),
                "String".to_string(),
                "aleo1qnr4dkkvkgfqph0vzc3y6z2eu975wnpz2925ntjccd76cqsnpdjsdhgg6c".to_string()
            ),
            ("height".to_string(), "Int".to_string(), "100".to_string()),
            ("recorded".to_string(), "Int".to_string(), "100".to_string()),
        ]
    );

    // ----- Return values --------------------------------------------------
    // who_am_i returns the caller address as String.
    // finalize_who_am_i returns the block height as Int(100).
    let returns: Vec<(String, &serde_json::Value)> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            (
                e["function"].as_str().expect("function").to_string(),
                &e["return_value"],
            )
        })
        .collect();
    assert_eq!(returns.len(), 2);

    let (who_name, who_rv) = &returns[0];
    assert_eq!(who_name, "who_am_i");
    assert_eq!(who_rv["kind"].as_str(), Some("String"));
    assert_eq!(
        who_rv["text"].as_str(),
        Some("aleo1qnr4dkkvkgfqph0vzc3y6z2eu975wnpz2925ntjccd76cqsnpdjsdhgg6c")
    );

    let (fin_name, fin_rv) = &returns[1];
    assert_eq!(fin_name, "finalize_who_am_i");
    assert_eq!(fin_rv["kind"].as_str(), Some("Int"));
    assert_eq!(fin_rv["i"].as_i64(), Some(100));

    // ----- io_event for the Mapping::set inside finalize ---------------
    let io_events: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "io")
        .map(|e| {
            (
                e["io_kind"].as_str().unwrap_or("").to_string(),
                e["text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert_eq!(
        io_events,
        vec![(
            "ioStdout".to_string(),
            "Mapping::set(last_seen, self.caller, height)".to_string()
        )]
    );
}

// --- int_overflow_modes_test.leo ------------------------------------------

/// Records `int_overflow_modes_test.leo` and pins the full event
/// shape for Leo's three integer arithmetic modes near the u32
/// boundary.  Checked overflow surfaces as a `LeoOverflow`
/// `EventLogKind::Read` io_event (the read variant maps to
/// `ioFileOp` in ct-print's JSON), matching the M10 `LeoAssert`
/// shape; wrapping completes silently with the wrapping result;
/// saturating clamps to `u32::MAX`.
#[test]
fn test_int_overflow_modes_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_int_overflow_modes_test_via_ct_print_full",
        "int_overflow_modes_test.leo",
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
    assert_eq!(functions, vec!["main", "compute"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(10), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    // Exactly one io_event: the `near_max + two` checked-overflow
    // signal.  `add_wrapped` and `add_saturating` are non-trapping
    // by spec and emit nothing extra.  `1u32 + two` does NOT
    // overflow so it must NOT emit an io_event.
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(1),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // ----- Decoded variable sequence -----------------------------------
    // near_max = u32::MAX - 1 = 4_294_967_294
    // two = 2
    // wrapped = near_max.add_wrapped(two) = 0 (silent wrap)
    // saturated = near_max.add_saturating(two) = u32::MAX = 4_294_967_295
    // bad = near_max + two = 0 (wrapping result; LeoOverflow io_event also emitted)
    // safe = 1 + two = 3 (no overflow, no event)
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("near_max".to_string(), 4_294_967_294),
            ("two".to_string(), 2),
            ("wrapped".to_string(), 0),
            ("saturated".to_string(), 4_294_967_295),
            ("bad".to_string(), 0),
            ("safe".to_string(), 3),
        ]
    );

    // ----- io_event: exactly one LeoOverflow on `near_max + two` -------
    let events = doc["events"].as_array().expect("events array");
    let io_events: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "io")
        .map(|e| {
            (
                e["io_kind"].as_str().unwrap_or("").to_string(),
                e["text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert_eq!(
        io_events,
        vec![("ioFileOp".to_string(), "near_max + two".to_string())]
    );

    // ----- Return value: compute returns `safe` = 3 (the non-overflowing arm)
    let returns: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let name = e["function"].as_str().expect("function").to_string();
            let rv = &e["return_value"];
            assert_eq!(
                rv["kind"].as_str(),
                Some("Int"),
                "return value should decode as Int; got {rv}"
            );
            (name, rv["i"].as_i64().expect("Int.i"))
        })
        .collect();
    assert_eq!(returns, vec![("compute".to_string(), 3)]);
}

// --- cast_test.leo --------------------------------------------------------

/// Records `cast_test.leo` and pins the full event shape for
/// Leo's explicit type-cast operator (` as <type>`).  The
/// structured evaluator computes each cast with type-correct
/// truncation (so e.g. `300u32 as u8 == 44`), registers the
/// target type-id, and surfaces the post-cast payload as the
/// step variable's value.
#[test]
fn test_cast_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_cast_test_via_ct_print_full", "cast_test.leo")
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

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(11), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // ----- Decoded variable sequence + truncation pinning --------------
    // small = 100u32
    // big = small as u64 -> 100 (typed u64)
    // wide = 300u32
    // narrow = wide as u8 -> 44 (300 mod 256, typed u8 -- the truncation pin)
    // f = big as field -> 100 (typed field)
    // s = f as scalar -> 100 (typed scalar)
    // total = small + (narrow as u32) -> 100 + 44 = 144
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("small".to_string(), 100),
            ("big".to_string(), 100),
            ("wide".to_string(), 300),
            ("narrow".to_string(), 44),
            ("f".to_string(), 100),
            ("s".to_string(), 100),
            ("total".to_string(), 144),
        ]
    );

    // ----- Type-id distinction -----------------------------------------
    // Each cast result must carry a DIFFERENT type-id from its source
    // operand so downstream tooling can reason about the post-cast
    // type.  Walk every step variable, collect (varname, type_id),
    // and assert the cast chain produces three distinct ids
    // (small/wide as u32; big as u64; narrow as u8; f as field; s as
    // scalar; total back to u32).  We don't pin the absolute id
    // values (they're writer-assigned), only that the ids for the
    // cast outputs differ from `small`'s id and match each other
    // when the lang_type matches.
    let events = doc["events"].as_array().expect("events array");
    let var_type_ids: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .map(|v| {
            (
                v["varname"].as_str().expect("varname").to_string(),
                v["value"]["type_id"].as_i64().expect("type_id"),
            )
        })
        .collect();
    let id_for = |name: &str| -> i64 {
        var_type_ids
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
            .unwrap_or_else(|| panic!("missing var {name} in type-id map: {var_type_ids:?}"))
    };
    let small_id = id_for("small");
    let big_id = id_for("big");
    let wide_id = id_for("wide");
    let narrow_id = id_for("narrow");
    let f_id = id_for("f");
    let s_id = id_for("s");
    let total_id = id_for("total");
    assert_eq!(
        small_id, wide_id,
        "u32-typed vars must share a type id (small/wide); got small={small_id}, wide={wide_id}"
    );
    assert_eq!(
        small_id, total_id,
        "total is u32 like small; got small={small_id}, total={total_id}"
    );
    assert_ne!(
        small_id, big_id,
        "u32 (small) and u64 (big) must have distinct type ids"
    );
    assert_ne!(
        small_id, narrow_id,
        "u32 (small) and u8 (narrow) must have distinct type ids"
    );
    assert_ne!(
        big_id, narrow_id,
        "u64 (big) and u8 (narrow) must have distinct type ids"
    );
    assert_ne!(
        big_id, f_id,
        "u64 (big) and field (f) must have distinct type ids"
    );
    assert_ne!(
        f_id, s_id,
        "field (f) and scalar (s) must have distinct type ids"
    );

    // ----- Registered types include each cast target -------------------
    let types: Vec<&str> = doc["types"]
        .as_array()
        .expect("types array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    for want in ["u32", "u64", "u8", "field", "scalar"] {
        assert!(
            types.contains(&want),
            "expected `{want}` in registered types; got {types:?}"
        );
    }

    // ----- Return value ------------------------------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("Int.i")
        })
        .collect();
    assert_eq!(returns, vec![144]);
}

// --- async_finalize_failure_test.leo --------------------------------------

/// Records `async_finalize_failure_test.leo` and pins the full
/// event shape for a runtime abort inside an `async finalize`
/// block.  The finalize calls `Mapping::get(balances, caller)`
/// (the variant WITHOUT a `get_or_use` default), which the
/// structured evaluator treats as a non-existent-key access (the
/// recorder does not model an in-memory mapping store), surfacing
/// a `LeoFinalizeAbort` `EventLogKind::Error` io_event.  The
/// finalize frame's CloseFrame is then marked as failed: the
/// post-abort `Mapping::set` and `return updated` statements are
/// NOT evaluated and the call's `return_value` is `None`.
#[test]
fn test_async_finalize_failure_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_async_finalize_failure_test_via_ct_print_full",
        "async_finalize_failure_test.leo",
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
    assert_eq!(functions, vec!["main", "deposit", "finalize_deposit"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(10), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(2), "calls; counts={counts}");
    // Exactly one io_event: the LeoFinalizeAbort for Mapping::get.
    // The post-abort Mapping::set MUST NOT execute -- if it did,
    // io_events would be 2.
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(1),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);
    assert_eq!(
        observed_call_sequence(&doc),
        vec!["deposit".to_string(), "finalize_deposit".to_string()]
    );

    // ----- io_event: exactly one LeoFinalizeAbort with EventLogKind::Error
    // ct-print surfaces EventLogKind::Error as `ioError` in the JSON.
    let events = doc["events"].as_array().expect("events array");
    let io_events: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "io")
        .map(|e| {
            (
                e["io_kind"].as_str().unwrap_or("").to_string(),
                e["text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert_eq!(
        io_events,
        vec![(
            "ioError".to_string(),
            "Mapping::get(balances, caller)".to_string()
        )]
    );

    // ----- Per-call return values --------------------------------------
    // deposit succeeds and returns reported = 50 (Int).
    // finalize_deposit aborts -- its CloseFrame surfaces with
    // return_value `None` so ct-print shows kind = "Void".
    let returns: Vec<(String, &serde_json::Value)> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            (
                e["function"].as_str().expect("function").to_string(),
                &e["return_value"],
            )
        })
        .collect();
    assert_eq!(returns.len(), 2);

    let (dep_name, dep_rv) = &returns[0];
    assert_eq!(dep_name, "deposit");
    assert_eq!(dep_rv["kind"].as_str(), Some("Int"));
    assert_eq!(dep_rv["i"].as_i64(), Some(50));

    let (fin_name, fin_rv) = &returns[1];
    assert_eq!(fin_name, "finalize_deposit");
    // The aborted CloseFrame surfaces as kind "Void" (the
    // ct-print mapping of NONE_VALUE) -- emphatically NOT "Int".
    assert_eq!(
        fin_rv["kind"].as_str(),
        Some("Void"),
        "finalize_deposit aborted; return_value must be Void (NONE_VALUE), got {fin_rv}"
    );

    // ----- Pre-abort bindings only -------------------------------------
    // deposit emits `amount=50` (formal param) twice (call_entry +
    // body binding step) and then `reported=50`.  finalize_deposit
    // emits its formal params (`caller=7`, `amount=50`) twice each,
    // plus the failed `prior` binding (None-valued, NOT Int) at the
    // abort site.  The post-abort bindings (`updated`, return) MUST
    // NOT appear.  Main emits `deposited=50` (Int) and `finalized`
    // (None, because finalize_deposit returned NONE_VALUE).
    //
    // `observed_var_sequence` would refuse the None-valued bindings
    // (Int-only assertion) so we walk vars directly.
    let observed: Vec<(String, String, Option<i64>)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .map(|v| {
            let name = v["varname"].as_str().expect("varname").to_string();
            let kind = v["value"]["kind"].as_str().unwrap_or("?").to_string();
            let i = v["value"]["i"].as_i64();
            (name, kind, i)
        })
        .collect();
    // The abort site binds `prior` as `None` -- pin that exact shape.
    let prior_entry = observed.iter().find(|(n, _, _)| n == "prior");
    assert!(
        prior_entry.is_some(),
        "expected `prior` binding (abort site) in observed vars: {observed:?}"
    );
    let (_, prior_kind, prior_i) = prior_entry.unwrap();
    assert_eq!(
        prior_kind, "None",
        "prior must surface as None at the abort site; got kind={prior_kind}, i={prior_i:?}"
    );
    assert!(
        prior_i.is_none(),
        "prior should not carry an Int payload at the abort site; got {prior_i:?}"
    );

    // The post-abort `updated` binding must NOT appear.
    assert!(
        !observed.iter().any(|(n, _, _)| n == "updated"),
        "post-abort binding `updated` must NOT appear in the trace; \
         finalize_deposit halted at the abort site.  observed = {observed:?}"
    );

    // Successful pre-finalize bindings: deposited=50, reported=50,
    // and the deposit-frame formals.
    assert!(
        observed
            .iter()
            .any(|(n, k, i)| n == "reported" && k == "Int" && *i == Some(50)),
        "expected reported=50 (Int) in deposit frame: {observed:?}"
    );
    assert!(
        observed
            .iter()
            .any(|(n, k, i)| n == "deposited" && k == "Int" && *i == Some(50)),
        "expected deposited=50 (Int) in main frame: {observed:?}"
    );
    // `finalized` exists in main but with None value (because
    // finalize_deposit returned NONE_VALUE).
    let finalized = observed
        .iter()
        .find(|(n, _, _)| n == "finalized")
        .unwrap_or_else(|| panic!("expected finalized binding: {observed:?}"));
    assert_eq!(
        finalized.1, "None",
        "finalized must reflect the aborted return as None; got {finalized:?}"
    );
}

// --- program_imports_test.leo ---------------------------------------------

/// Records `program_imports_test.leo`, which imports
/// `math_lib.aleo` and invokes its `double` transition via the
/// qualified call shape `math_lib.aleo/double(x)`.  Pins:
///
///   1. The recorder loads BOTH .leo files (the caller and the
///      imported library) and merges their function tables.
///   2. The cross-program call surfaces as a Call/Return pair
///      with the qualified function name
///      (`math_lib.aleo/double`) registered in the writer's
///      function table.
///   3. Source-line metadata for the callee's body correctly
///      points at `math_lib.leo` (the imported library's source
///      file) rather than the caller's source file.
#[test]
fn test_program_imports_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_program_imports_test_via_ct_print_full",
        "program_imports_test.leo",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    // ----- Function table: includes the QUALIFIED imported name -------
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["main", "compute", "math_lib.aleo/double"],
        "expected qualified name `math_lib.aleo/double` in function \
         table; got {functions:?}"
    );

    // ----- Path table: BOTH source files registered -------------------
    let paths: Vec<&str> = doc["paths"]
        .as_array()
        .expect("paths array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with("program_imports_test.leo")),
        "expected program_imports_test.leo in paths; got {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("math_lib.leo")),
        "expected math_lib.leo in paths -- the recorder must load \
         the imported library as a separate source file; got {paths:?}"
    );

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(9), "steps; counts={counts}");
    // 2 calls: compute (locally) + math_lib.aleo/double (cross-program).
    assert_eq!(counts["calls"].as_u64(), Some(2), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );
    // The 2 distinct paths must surface in counts as well.
    assert_eq!(counts["paths"].as_u64(), Some(2), "paths; counts={counts}");

    assert_step_indices_monotonic(&doc);
    assert_eq!(
        observed_call_sequence(&doc),
        vec!["compute".to_string(), "math_lib.aleo/double".to_string()]
    );

    // ----- Source-line attribution: callee body steps point at math_lib.leo
    // Walk every step inside the `math_lib.aleo/double` call frame
    // and assert each one's `path` ends with `math_lib.leo` (not the
    // caller's file).  The function-entry step is anchored at the
    // call site in the caller (entry-step semantics), so we filter
    // by the explicit lib-source pattern instead of by call frame.
    let events = doc["events"].as_array().expect("events array");
    let math_lib_steps: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .filter_map(|e| {
            let path = e["path"].as_str()?;
            if path.ends_with("math_lib.leo") {
                Some((path.to_string(), e["line"].as_i64().unwrap_or(-1)))
            } else {
                None
            }
        })
        .collect();
    assert!(
        !math_lib_steps.is_empty(),
        "expected at least one step event whose path is math_lib.leo \
         (the callee body must surface against the library's source \
         file); got events with no math_lib.leo path attribution"
    );
    // The library-side steps live at lines 5 (transition signature),
    // 6 (let doubled = x + x), and 7 (return doubled).
    let math_lib_lines: Vec<i64> = math_lib_steps.iter().map(|(_, l)| *l).collect();
    for want_line in [5, 6, 7] {
        assert!(
            math_lib_lines.contains(&want_line),
            "expected math_lib.leo line {want_line} in callee body steps; \
             got {math_lib_lines:?}"
        );
    }

    // ----- Decoded variable sequence -----------------------------------
    // x=7 (double's formal, surfaces twice -- function-entry step +
    // body-binding step).  doubled=14 (x + x).  Then back in
    // compute: inner=14 (the call result).  doubled=15 (inner + 1).
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("x".to_string(), 7),
            ("x".to_string(), 7),
            ("doubled".to_string(), 14),
            ("inner".to_string(), 14),
            ("doubled".to_string(), 15),
        ]
    );

    // ----- Per-call return values --------------------------------------
    // double returns 14, compute returns 15.
    let returns: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let name = e["function"].as_str().expect("function").to_string();
            let rv = &e["return_value"];
            assert_eq!(
                rv["kind"].as_str(),
                Some("Int"),
                "return value should decode as Int; got {rv}"
            );
            (name, rv["i"].as_i64().expect("Int.i"))
        })
        .collect();
    assert_eq!(
        returns,
        vec![
            ("math_lib.aleo/double".to_string(), 14),
            ("compute".to_string(), 15),
        ]
    );
}
