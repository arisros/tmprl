# tmprl

[![CI](https://github.com/arisros/tmprl/actions/workflows/ci.yml/badge.svg)](https://github.com/arisros/tmprl/actions/workflows/ci.yml)

A terminal client for [Temporal](https://temporal.io), aiming at parity with the Temporal
Web UI — built to be operated from the keyboard rather than a browser.

> ### Status: early. There is no TUI yet.
>
> What exists today is the gRPC access layer — connecting, profile handling, and the four
> RPCs the interface will be built on — plus the design documents for the rest.
> **Nothing in this repository draws a user interface.** If you want a working Temporal TUI
> today, see [Prior art](#prior-art).

---

## What works today

- Connecting to a Temporal frontend: local, self-hosted, Temporal Cloud via API key, or mTLS.
- Profile resolution that matches the `temporal` CLI exactly — same TOML file, same
  environment variables, same precedence.
- Thin wrappers over `ListNamespaces`, `CountWorkflowExecutions`, `ListWorkflowExecutions`
  and `GetWorkflowExecutionHistory`.
- An integration suite that pins those RPC contracts against a live server.

That is genuinely all of it. You can run the example below and read data; you cannot yet
browse it interactively.

## What it is meant to become

A modal, keyboard-driven client covering what the web UI covers — workflows, histories,
schedules, batch operations, task queues, workers, nexus endpoints — with the things a
browser can't do: following a running workflow like `tail -f`, piping payloads through `jq`,
yanking to the system clipboard, and diffing two runs side by side.

The intended architecture is written down in **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**
and the interface design in **[docs/INTERFACE.md](docs/INTERFACE.md)**. Both describe code
that has not been written yet, and say so section by section.

---

## Requirements

| | |
|---|---|
| Rust | 1.95+ (edition 2024) |
| **`protoc`** | **Required.** `temporalio-protos` compiles Temporal's protobufs from source at build time. |

```sh
# Debian/Ubuntu
apt-get install -y protobuf-compiler
# macOS
brew install protobuf
```

Without `protoc` the build fails inside a build script with `Could not find protoc`, which
is easy to misread as a network problem. It isn't.

## Build

```sh
cargo build
```

Debug info is off in the `dev` profile. The dependency tree is 228 crates and the generated
protos dominate it; with debug info on, `target/` runs to several gigabytes. When you
actually need a debugger, use `cargo build --profile dbg`.

## Run the example

```sh
# terminal 1
temporal server start-dev

# terminal 2
temporal workflow start --task-queue demo --type Demo --workflow-id demo-1
cargo run -p tmprl-client --example spike
```

```
connected  profile=default  namespace=default

namespaces (2):
  - default
  - temporal-system

total workflows in `default`: 1

workflows (1 shown):
  Running    Demo    demo-1
  next_page_token: 0 bytes

history of demo-1 (2 events):
     1  WorkflowExecutionStarted
     2  WorkflowTaskScheduled
```

## Test

```sh
temporal server start-dev &
cargo test
```

The integration tests **skip** when no server is reachable, so `cargo test` stays green on a
machine that has never run Temporal. Set `TMPRL_REQUIRE_SERVER=1` to turn that skip into a
hard failure — CI does, so that a broken connection layer can't pass as a green build.

## Configuration

`tmprl` has no connection config of its own. It reads the same profiles the `temporal` CLI
reads, so if the CLI can reach your cluster, so can this:

```toml
# ~/.config/temporalio/temporal.toml
[profile.prod]
address   = "my-ns.a1b2c.tmprl.cloud:7233"
namespace = "my-ns.a1b2c"
api_key   = "…"

[profile.staging.tls]
client_cert_path = "/etc/temporal/client.pem"
client_key_path  = "/etc/temporal/client.key"
```

Precedence follows the CLI: flags, then `TEMPORAL_*` environment variables, then the TOML file.

## Layout

```
crates/tmprl-client   all network IO — gRPC, TLS, profiles     ← the only crate that exists
crates/tmprl-core     domain logic — history, queries, keymap   (planned)
crates/tmprl-ui       window tree — splits, tabs, focus         (planned)
crates/tmprl-tui      ratatui rendering and input               (planned)
```

The split exists so the hard logic — reconstructing histories, compiling visibility queries,
diffing runs — lands in a layer that needs neither a terminal nor a server to test. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Roadmap

- [x] **M0a** gRPC layer, profile loading, integration tests
- [ ] **M0b** event loop, command registry, modal keymap, statusline
- [ ] **M1** workflow list, visibility queries, saved views, multi-namespace
- [ ] **M2** workflow detail, history views, follow mode, jq, codec server
- [ ] **M3** mutations — signal, cancel, terminate, reset, update, delete
- [ ] **M4** schedules
- [ ] **M5** batch operations
- [ ] **M6** task queues, workers, deployments, nexus, archival
- [ ] **M7** diff, macros, headless `--exec`, themes

## Prior art

[`galaxy-io/tempo`](https://github.com/galaxy-io/tempo) is a Go/tview Temporal TUI that
works today — browsing, history, cancel/terminate/signal, schedules, themes. If you need a
terminal Temporal client right now, use that one.

`tmprl` differs in intent: full web-UI parity including batch operations, nexus, worker
deployments, reset and codec servers, and a modal editor model rather than a menu. Whether
that difference is worth a second implementation is a fair question, and the answer isn't in
yet.

## Contributing

The four rules in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#9-design-rules--planned) are the ones
worth reading before writing code. Issues and discussion welcome; given the stage, design
feedback is more useful than patches.

## License

MIT — see [LICENSE](LICENSE).
