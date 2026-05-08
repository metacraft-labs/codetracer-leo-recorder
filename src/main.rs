//! CLI entry point for the CodeTracer Leo recorder.
//!
//! Supports the `record` subcommand which takes a Leo source file,
//! compiles it through the upstream `leo` CLI (or the in-crate
//! Leo→Aleo fallback generator), executes the resulting Aleo
//! Instructions on a built-in AVM interpreter, and writes a CodeTracer
//! CTFS trace bundle.  The `replay` subcommand fetches a deployed
//! Aleo program from the network and replays its execution locally.
//!
//! # Usage
//!
//! ```text
//! codetracer-leo-recorder record <leo-file> --out-dir <output-dir>
//! ```
//!
//! The recorder always writes traces in the canonical CodeTracer multi-stream
//! CTFS format (see `Recorder-CLI-Conventions.md` §4 in `codetracer-specs`).
//! No `--format` flag is exposed: human-readable conversion is handled
//! out-of-band by `ct print` (shipped with `codetracer-trace-format-nim`).
//!
//! # Environment variables
//!
//! * `CODETRACER_LEO_RECORDER_OUT_DIR` — fallback for `--out-dir` when the
//!   flag is not given. The CLI flag always wins.
//! * `CODETRACER_LEO_RECORDER_DISABLED` — set to `1` or `true` to skip
//!   recording entirely. The recorder still validates its inputs (where
//!   applicable) and propagates a clean exit code.
//! * `CODETRACER_LEO_RECORDER_LOG_LEVEL` — recorder log verbosity (advisory;
//!   the Leo recorder currently logs to stderr unconditionally).

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use eyre::{Context, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Environment variable used as a fallback for `--out-dir` when the CLI
/// flag is omitted.  Convention: see `Recorder-CLI-Conventions.md` §5.
const ENV_OUT_DIR: &str = "CODETRACER_LEO_RECORDER_OUT_DIR";

/// Environment variable that, when set to `1`/`true`, disables tracing
/// entirely — the recorder runs as a transparent pass-through.
const ENV_DISABLED: &str = "CODETRACER_LEO_RECORDER_DISABLED";

/// Default output directory used when neither `--out-dir` nor
/// `CODETRACER_LEO_RECORDER_OUT_DIR` is set.
const DEFAULT_OUT_DIR: &str = "./ct-traces/";

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// CodeTracer Leo recorder -- record Leo / Aleo smart-contract execution traces.
///
/// Traces are always written in the canonical CTFS multi-stream format.
/// To convert a recorded `.ct` bundle to JSON / text for inspection, use
/// `ct print` from `codetracer-trace-format-nim`.
#[derive(Debug, Parser)]
#[command(
    name = "codetracer-leo-recorder",
    version,
    about = "Record Leo smart contract execution traces for CodeTracer (CTFS-only). \
             Use `ct print` from codetracer-trace-format-nim for human-readable conversion.",
    long_about = "Record Leo / Aleo smart-contract execution traces for CodeTracer.\n\
                  \n\
                  Output is always written in the canonical CodeTracer CTFS\n\
                  multi-stream format. Use `ct print` (shipped with the\n\
                  codetracer-trace-format-nim sibling) to convert a recorded\n\
                  `.ct` bundle to JSON or other human-readable forms.\n\
                  \n\
                  Environment variables:\n\
                    CODETRACER_LEO_RECORDER_OUT_DIR    fallback for --out-dir\n\
                    CODETRACER_LEO_RECORDER_DISABLED   set to 1/true to skip recording\n\
                    CODETRACER_LEO_RECORDER_LOG_LEVEL  log verbosity (advisory)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Record execution of a Leo program.
    ///
    /// Compiles the given .leo source file, executes the resulting Aleo
    /// Instructions on the built-in AVM interpreter, captures the
    /// execution trace, and writes a CTFS bundle to `--out-dir`.
    Record(RecordArgs),

    /// Replay a deployed Aleo program fetched from the network.
    ///
    /// Fetches the program's Aleo Instructions source via the Aleo node
    /// REST API, executes the specified function through the AVM interpreter,
    /// and writes a CTFS trace bundle to `--out-dir`.
    Replay(ReplayArgs),

    /// Print version information.
    Version,
}

#[derive(Debug, clap::Args)]
struct RecordArgs {
    /// Path to the Leo source (.leo) file.
    program: PathBuf,

    /// Directory where the trace files will be written.
    ///
    /// The directory will be created if it does not exist.  Falls back to
    /// the `CODETRACER_LEO_RECORDER_OUT_DIR` environment variable when the
    /// flag is omitted.
    #[arg(short = 'o', long)]
    out_dir: Option<PathBuf>,
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
    ///
    /// Falls back to the `CODETRACER_LEO_RECORDER_OUT_DIR` environment
    /// variable when the flag is omitted.
    #[arg(short = 'o', long)]
    out_dir: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the effective output directory:
///   1. `--out-dir` if given on the CLI.
///   2. `CODETRACER_LEO_RECORDER_OUT_DIR` env var.
///   3. `DEFAULT_OUT_DIR` ("./ct-traces/").
fn resolve_out_dir(cli_out_dir: Option<PathBuf>) -> PathBuf {
    if let Some(path) = cli_out_dir {
        return path;
    }
    if let Some(value) = std::env::var_os(ENV_OUT_DIR) {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    PathBuf::from(DEFAULT_OUT_DIR)
}

/// Whether the recorder is disabled via env var.  When true, the CLI
/// must execute its target operation in pass-through mode without
/// emitting any trace artefacts.
fn recording_disabled() -> bool {
    match std::env::var(ENV_DISABLED) {
        Ok(value) => {
            let v = value.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        }
        Err(_) => false,
    }
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
            println!("codetracer-leo-recorder {}", env!("CARGO_PKG_VERSION"));
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

    if recording_disabled() {
        // Pass-through: the Leo recorder doesn't run a separate target
        // process — it compiles & executes the source itself — so disabling
        // recording simply means "don't emit any trace artefacts".
        eprintln!("{ENV_DISABLED} is set; skipping trace recording (no output written).");
        return Ok(());
    }

    // 2. Resolve and create the output directory
    let out_dir = resolve_out_dir(args.out_dir);
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    // 3. Run the recorder (CTFS only)
    codetracer_leo_recorder::recorder::record(&source_path, &out_dir)?;

    eprintln!("Trace files written to {}", out_dir.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// `replay` implementation
// ---------------------------------------------------------------------------

/// Execute the `replay` subcommand.
fn replay(args: ReplayArgs) -> Result<()> {
    if recording_disabled() {
        eprintln!("{ENV_DISABLED} is set; skipping replay recording (no output written).");
        return Ok(());
    }

    let out_dir = resolve_out_dir(args.out_dir);
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("cannot create output dir: {}", out_dir.display()))?;

    let config = codetracer_leo_recorder::replay::ReplayConfig {
        program_id: args.program_id,
        function_name: args.function,
        inputs: args.inputs,
        endpoint: args.endpoint,
    };

    codetracer_leo_recorder::replay::replay_program(&config, &out_dir)
}
