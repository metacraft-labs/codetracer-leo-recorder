//! Recording logic for Leo execution traces.
//!
//! This module provides the top-level `record` function that reads a Leo
//! source file, evaluates variable assignments, captures the trace, and writes
//! a CodeTracer CTFS trace bundle.
//!
//! The output format is fixed to CTFS — see
//! `Recorder-CLI-Conventions.md` §4 in `codetracer-specs`.  Use
//! `ct print` (from `codetracer-trace-format-nim`) for human-readable
//! conversion of the produced bundle.

use std::path::Path;

use eyre::{Context, Result};

use crate::tracer::LeoTracer;

/// Record a Leo execution trace.
///
/// Reads the Leo source file at `source_path`, parses function definitions
/// and variable assignments, evaluates them, captures the trace, and writes
/// CodeTracer trace files to `out_dir`.
pub fn record(source_path: &Path, out_dir: &Path) -> Result<()> {
    let source_code = std::fs::read_to_string(source_path)
        .with_context(|| format!("failed to read source file: {}", source_path.display()))?;

    LeoTracer::trace_program(source_path, &source_code, out_dir)
}
