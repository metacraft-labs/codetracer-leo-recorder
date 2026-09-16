## codetracer-leo-recorder

A recorder of Leo/Aleo smart contract executions that produces [CodeTracer](https://github.com/metacraft-labs/CodeTracer) traces.

> [!WARNING]
> Currently it is in a very early phase: we're welcoming contribution and discussion!

### Overview

codetracer-leo-recorder compiles Leo programs and traces their execution
through the Aleo VM. It captures instruction-level traces (add, sub,
mul, hash, ternary, branch, cast, call), including finalize block
execution, and emits a CodeTracer CTFS multi-stream trace bundle.

### Building

```bash
cargo build
```

### Usage

Record a trace from a Leo source file:

```bash
codetracer-leo-recorder record <leo-file> --out-dir <dir>
```

Replay an on-chain program execution via the Aleo REST API:

```bash
codetracer-leo-recorder replay --program-id <id> --function <name> [--input <value>...] --out-dir <dir>
```

The recorder always writes traces in the canonical CodeTracer CTFS
multi-stream format. There is no `--format` flag — see "Converting
traces" below for human-readable output.

#### Converting traces to JSON / text

The recorder is CTFS-only. To convert a recorded `.ct` bundle to a
human-readable form, use `ct print` from
[`codetracer-trace-format-nim`](https://github.com/metacraft-labs/codetracer-trace-format-nim):

```bash
ct-print --json <recording-dir>/<program>.ct
```

`ct-print` accepts `--json`, `--json-events`, `--summary`, and
`--follow` modes; see its `--help` for details. This conversion path
is the canonical way to produce textual oracles for golden-snapshot
tests, debugging, and interop with non-CodeTracer tools.

### Architecture

The recorder is organized into the following modules:

* `recorder.rs` — top-level recording orchestration and CTFS trace bundle output
* `tracer.rs` — AVM instruction parsing and step-level trace capture
* `source_map.rs` — mapping from AVM instructions back to Leo source locations
* `finalize.rs` — tracing of finalize block execution
* `transition.rs` — tracing of transition execution
* `replay.rs` — on-chain program replay via the Aleo REST API

### Testing

Test programs live in `test-programs/leo/`. Run the test suite with:

```bash
cargo test
just test     # also runs verify-cli-convention-no-silent-skip.sh
```

### Environment variables

The recorder respects the standard CodeTracer recorder env-var contract
defined in `Recorder-CLI-Conventions.md` §5:

| Variable                              | CLI equivalent | Description                                                                                  |
|---------------------------------------|----------------|----------------------------------------------------------------------------------------------|
| `CODETRACER_LEO_RECORDER_OUT_DIR`     | `--out-dir`    | Fallback output directory when `--out-dir` is omitted. The CLI flag always wins.             |
| `CODETRACER_LEO_RECORDER_DISABLED`    | —              | Set to `1` or `true` to run the recorder in pass-through mode (no trace artefacts written).  |
| `CODETRACER_LEO_RECORDER_LOG_LEVEL`   | —              | Recorder log verbosity (advisory; the Leo recorder currently logs to stderr unconditionally).|

Tooling-specific:

* `RUST_LOG` — controls log verbosity (standard `env_logger` syntax, e.g. `RUST_LOG=debug`)

### Contributing

We'd be very happy if the community finds this useful, and if anyone wants to:

* Use and test the Leo/Aleo support or CodeTracer.
* Provide feedback and discuss alternative implementation ideas: in the issue tracker, or in our [discord](https://discord.gg/qSDCAFMP).
* Contribute code to enhance the Leo/Aleo support of CodeTracer.
* Provide [sponsorship](https://opencollective.com/codetracer), so we can hire dedicated full-time maintainers for this project.

### Legal info

LICENSE: MIT

Copyright (c) 2025 Metacraft Labs Ltd
