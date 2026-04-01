//! CLI entry point for the CodeTracer Leo recorder.
//!
//! Supports the `record` subcommand which takes a Leo source file,
//! parses and evaluates variable assignments, and writes CodeTracer trace
//! output files.
//!
//! # Usage
//!
//! ```text
//! codetracer-leo-recorder record <leo-file> \
//!     --out-dir <output-dir> \
//!     [--format binary|json]
//! ```

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use codetracer_trace_writer::TraceEventsFileFormat;
use eyre::{Context, Result};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// CodeTracer Leo recorder -- record Leo smart contract execution traces.
#[derive(Debug, Parser)]
#[command(
    name = "codetracer-leo-recorder",
    version,
    about = "Record Leo smart contract execution traces for CodeTracer"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Record execution of a Leo program.
    ///
    /// Parses the given .leo source file, evaluates variable assignments,
    /// captures the execution trace, and writes CodeTracer trace files
    /// to `--out-dir`.
    Record(RecordArgs),

    /// Replay a deployed Aleo program fetched from the network.
    ///
    /// Fetches the program's Aleo Instructions source via the Aleo node
    /// REST API, executes the specified function through the AVM interpreter,
    /// and writes CodeTracer trace files to `--out-dir`.
    Replay(ReplayArgs),

    /// Print version information.
    Version,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Binary,
    Json,
}

#[derive(Debug, clap::Args)]
struct RecordArgs {
    /// Path to the Leo source (.leo) file.
    program: PathBuf,

    /// Directory where the trace files will be written.
    ///
    /// The directory will be created if it does not exist.
    #[arg(short = 'o', long, default_value = "./ct-traces/")]
    out_dir: PathBuf,

    /// Output format for the trace data.
    #[arg(short = 'f', long, default_value = "binary")]
    format: OutputFormat,
}

#[derive(Debug, clap::Args)]
struct ReplayArgs {
    /// The on-chain program ID (e.g. `credits.aleo`).
    #[arg(long)]
    program_id: String,

    /// The function to execute within the program.
    #[arg(long)]
    function: String,

    /// Input values for the function (repeatable, e.g. `--input 10u32 --input 20u32`).
    #[arg(long = "input")]
    inputs: Vec<String>,

    /// Aleo node REST API endpoint.
    #[arg(long, default_value = "https://api.explorer.aleo.org/v1")]
    endpoint: String,

    /// Directory where the trace files will be written.
    #[arg(short = 'o', long, default_value = "./ct-traces/")]
    out_dir: PathBuf,

    /// Output format for the trace data.
    #[arg(short = 'f', long, default_value = "binary")]
    format: OutputFormat,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Record(args) => record(args),
        Commands::Replay(args) => replay(args),
        Commands::Version => {
            println!(
                "codetracer-leo-recorder {}",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// `record` implementation
// ---------------------------------------------------------------------------

/// Execute the `record` subcommand.
fn record(args: RecordArgs) -> Result<()> {
    // 1. Validate the source file exists
    let source_path = args
        .program
        .canonicalize()
        .with_context(|| format!("source file not found: {}", args.program.display()))?;

    eprintln!("Source file: {}", source_path.display());

    let format = match args.format {
        OutputFormat::Binary => TraceEventsFileFormat::Binary,
        OutputFormat::Json => TraceEventsFileFormat::Json,
    };

    // 2. Create the output directory
    let out_dir = &args.out_dir;
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    // 3. Run the recorder
    codetracer_leo_recorder::recorder::record(&source_path, out_dir, format)?;

    eprintln!("Trace files written to {}", out_dir.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// `replay` implementation
// ---------------------------------------------------------------------------

/// Execute the `replay` subcommand.
fn replay(args: ReplayArgs) -> Result<()> {
    let format = match args.format {
        OutputFormat::Binary => TraceEventsFileFormat::Binary,
        OutputFormat::Json => TraceEventsFileFormat::Json,
    };

    let config = codetracer_leo_recorder::replay::ReplayConfig {
        program_id: args.program_id,
        function_name: args.function,
        inputs: args.inputs,
        endpoint: args.endpoint,
    };

    codetracer_leo_recorder::replay::replay_program(&config, &args.out_dir, format)
}
