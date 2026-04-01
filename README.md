## codetracer-leo-recorder

A recorder of Leo/Aleo smart contract executions that produces [CodeTracer](https://github.com/metacraft-labs/CodeTracer) traces.

> [!WARNING]
> Currently it is in a very early phase: we're welcoming contribution and discussion!

### Overview

codetracer-leo-recorder compiles Leo programs and traces their execution through the Aleo VM. It captures instruction-level traces (add, sub, mul, hash, ternary, branch, cast, call), including finalize block execution, and emits structured trace files compatible with CodeTracer.

### Building

```bash
cargo build
```

### Usage

Record a trace from a Leo source file:

```bash
codetracer-leo-recorder record <leo-file> --out-dir <dir> [--format binary|json]
# Produces trace files in <dir>.
# --format selects the output format (defaults to binary).
```

Replay an on-chain program execution via the Aleo REST API:

```bash
codetracer-leo-recorder replay <program-id> --out-dir <dir> [--format binary|json]
```

However, you probably want to use it in combination with CodeTracer, which would be released soon.

### Architecture

The recorder is organized into the following modules:

* `recorder.rs` — top-level recording orchestration and trace file output
* `tracer.rs` — AVM instruction parsing and step-level trace capture
* `source_map.rs` — mapping from AVM instructions back to Leo source locations
* `finalize.rs` — tracing of finalize block execution
* `transition.rs` — tracing of transition execution
* `replay.rs` — on-chain program replay via the Aleo REST API

### Testing

Test programs live in `test-programs/leo/`. Run the test suite with:

```bash
cargo test
```

### Environment variables

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
