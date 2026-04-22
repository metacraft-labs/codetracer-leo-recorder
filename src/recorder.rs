//! Recording logic for Leo execution traces.
//!
//! This module provides the top-level `record` function that reads a Leo
//! source file, evaluates variable assignments, captures the trace, and writes
//! CodeTracer output.

use std::path::Path;

use codetracer_trace_writer_nim::TraceEventsFileFormat;
use eyre::{Context, Result};

use crate::tracer::LeoTracer;

/// Record a Leo execution trace.
///
/// Reads the Leo source file at `source_path`, parses function definitions
/// and variable assignments, evaluates them, captures the trace, and writes
/// CodeTracer trace files to `out_dir`.
pub fn record(source_path: &Path, out_dir: &Path, format: TraceEventsFileFormat) -> Result<()> {
    let source_code = std::fs::read_to_string(source_path)
        .with_context(|| format!("failed to read source file: {}", source_path.display()))?;

    LeoTracer::trace_program(source_path, &source_code, out_dir, format)
}
