# tmprl

[![CI](https://github.com/arisros/tmprl/actions/workflows/ci.yml/badge.svg)](https://github.com/arisros/tmprl/actions/workflows/ci.yml)

A terminal client for [Temporal](https://temporal.io), aiming at parity with the Temporal
Web UI — built to be operated from the keyboard rather than a browser.

> ### Status: early, but it runs.
>
> `tmprl` starts, connects, and gives you a modal, vim-keyed browser for namespaces,
> **workflows and their histories** — with an editable visibility query, per-status counts,
> saved views, infinite scroll, a collapsible event outline, `tail -f`-style follow mode,
> splits and tabs. It can **cancel, terminate, signal, delete, reset and update** workflows,
> each behind a confirmation that shows the equivalent `temporal` CLI command. If you need a Temporal TUI
> for real work today, see [Prior art](#prior-art).

---

## What works today

- **Connecting** to a Temporal frontend: local, self-hosted, Temporal Cloud via API key, or
  mTLS — using the same profiles and `TEMPORAL_*` variables as the `temporal` CLI.
- **A modal interface**: Normal, Insert, Visual and Command modes, `jk` to leave Insert,
  and counts that compose with motions (`7j`, `5gg`).
- **A namespace list** with a hybrid relative/absolute gutter, so counts are readable off
  the screen rather than estimated.
- **A workflow list** — `Enter` on a namespace. Pages in as you scroll, sorted newest-first,
  with per-status counts in the header from one `GROUP BY` call. Status is drawn as a glyph
  as well as a colour, so the column reads on 16 colours and for a colour-blind reader.
- **An editable visibility query**, always on screen and always the raw string. `i` edits it,
  `Enter` applies, `Esc` abandons. Saved views write *into* it and leave it editable — there
  is no filter widget hiding the query from you.
- **Saved views** in `views.toml`, on `<Space>1`–`<Space>9`, listed by name in the which-key
  popup.
- **Multiple namespaces at once**: select them with `V` on the namespace list and press
  `Enter`. The result is one merged table, newest-first, each row tagged with its namespace.
- **Remappable keys** through `keys.toml`, resolved against the command registry, so a typo
  is an error at startup rather than a key that silently does nothing.
- **Discovery**: a which-key popup on an incomplete prefix, and a scrollable `?` help overlay
  — both generated from the command registry and keymap, so neither can go stale.
- **A `:` command line** with completions over every registered command.
- **A workflow history** — `Enter` on a workflow. Events are folded into groups, so an
  activity that was scheduled, started and completed is one row rather than three, carrying
  its retry count and failure message. `za` folds a group open, `zR`/`zM` expand and collapse
  everything, `zp` reveals the workflow-task plumbing that is hidden by default, and `]f`/`[f`
  jump between failures. Only the visible rows are ever built, so a very long history scrolls
  by moving an index.
- **Follow mode** (`F`) — tail a running workflow like `tail -f`. New events appear as they
  happen; it stops by itself when the workflow closes and says so.
- **Payloads** (`K`) — inputs and results, decoded and pretty-printed, in a pane under the
  history. Encrypted payloads say they need a codec server rather than showing ciphertext;
  binary ones say what they are rather than corrupting your terminal.
- **Piping** (`!`) — filter those payloads through any command, pre-filled with `jq .`. What
  goes down the pipe is a JSON object keyed by label, so `jq .result` picks one out.
- **Codec server** — point `config.toml` at one and encrypted payloads decode in place,
  lazily and cached, so everything else reads the plaintext without knowing.
- **Splits and tabs** with vim's bindings — `<Space>sv` / `<Space>sh` to split, `<C-w>hjkl`
  to move between panes, `<Space>t{o,x,n,p}` for tabs. Each pane keeps its own screen,
  cursor, query and history.
- **Cancel, terminate, signal, delete, reset and update** (`<Space>m{c,t,s,d,r,u}`), each behind one
  confirmation that shows the equivalent `temporal` CLI command, so you read what is about to
  happen rather than trusting a verb. Every attempt is appended to
  `~/.local/state/tmprl/audit.jsonl`.
- **Yank** (`y`, `Y`) to the system clipboard over OSC 52, so it works over SSH.

Not yet: schedules, batch operations.

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

Debug info is off in the `dev` profile. The dependency tree is 230 crates and the generated
protos dominate it; with debug info on, `target/` runs to several gigabytes. When you
actually need a debugger, use `cargo build --profile dbg`.

## Run it

```sh
# terminal 1
temporal server start-dev

# terminal 2
cargo run -p tmprl-tui
```

You land on the namespace list:

```
 tmprl profile=default  ns=default                                        3 namespaces
   1 default                     Registered       1d
   2 payments                    Registered       3d
   1 temporal-system             Registered       7d

 NORMAL  ? help   : commands
```

`Enter` opens one; `V` then `Enter` opens several as a single merged list:

```
 tmprl profile=default  ns=default +1                         ● 125  ■ 1  126 total
 query  all workflows — i to filter
   1 ● Running      charge-3            ChargeCard        payments             1m
   1 ● Running      charge-2            ChargeCard        payments             1m
   2 ● Running      charge-1            ChargeCard        payments             1m
   3 ● Running      scroll-068          ScrollWorkflow    default              2m
   4 ● Running      scroll-099          ScrollWorkflow    default              2m

 NORMAL  ? help   : commands
```

`i` edits the query bar, `Enter` applies it, `-` goes back up a level. `?` lists every
binding. `<Space>` opens the which-key popup. `<Space>q` or `<C-c>` quits.

There is also a non-interactive example that exercises the RPC layer directly, useful for
checking connectivity without the interface:

```sh
cargo run -p tmprl-client --example spike
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

Everything else lives in `$TMPRL_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tmprl`, else
`~/.config/tmprl`. Both files are optional:

```toml
# views.toml — saved queries on <Space>1 … <Space>9
[[view]]
key   = "1"
name  = "Running now"
query = "ExecutionStatus = 'Running'"
```

```toml
# keys.toml — chord → command id, overriding the defaults
[normal]
"ZZ"    = "app.quit"
"<C-r>" = "app.refresh"
```

Command ids are the ones `?` and `:` show. An unknown id, an unparseable chord or a duplicate
view key is reported in the statusline at startup rather than quietly skipped.

## Layout

```
crates/tmprl-client   all network IO — gRPC, TLS, codec, profiles built,  46 tests
crates/tmprl-core     domain logic: modes, keymap, histories    built, 138 tests
crates/tmprl-tui      ratatui rendering and input               built, 132 tests
crates/tmprl-ui       window tree — splits, tabs, focus         built,  35 tests
```

The split exists so the hard logic — reconstructing histories, compiling visibility queries,
diffing runs — lands in a layer that needs neither a terminal nor a server to test. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Roadmap

- [x] **M0a** gRPC layer, profile loading, integration tests
- [x] **M0b** event loop, command registry, modal keymap, statusline, which-key, yank
- [x] **M1** workflow list, visibility queries, saved views, multi-namespace, `keys.toml`
- [x] **M2** history views, follow mode, jq, codec server, splits and tabs
- [x] **M3** mutations: signal, cancel, terminate, delete, reset, update
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

The four rules in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#10-design-rules) are the ones
worth reading before writing code. Issues and discussion welcome; given the stage, design
feedback is more useful than patches.

## License

MIT — see [LICENSE](LICENSE).
