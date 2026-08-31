# Architecture

> **Read this first.** This document mixes built code with design that is not written yet,
> and every section says which it is. §§2–5 are implemented and tested; §§6–9 are still the
> plan being built against, written down in advance so the shape is agreed before there is
> code sitting on top of it.
>
> | Marker | Meaning |
> |---|---|
> | **`BUILT`** | Exists in the repository and is tested |
> | **`PLANNED`** | Designed, not implemented |

This document explains how `tmprl` is meant to be put together and, more importantly, *why*.
If you are here to change something, read the [Design rules](#10-design-rules) first —
most of the structure exists to protect those four rules.

---

## 1. The problem shape

A Temporal UI is not a CRUD app. Three properties drive the entire design:

**It is a viewer for an append-only log.** A workflow's history only ever grows. Histories
routinely reach tens of thousands of events, and pathological ones reach millions. Anything
that materialises a whole history into a list of rendered rows will fall over.

**Almost every read is a network call, and some of them block for a minute.** Temporal's
`GetWorkflowExecutionHistory` with `wait_new_event: true` is a *long poll* — it deliberately
does not return until a new event arrives. That is the mechanism behind follow mode, and it
means "fetch data" and "draw a frame" can never be the same thread of control.

**The interesting structure is implicit.** The wire format is a flat list of events. The
thing a human wants to see — "activity `ChargeCard` was scheduled, started, failed, retried
three times, then succeeded 40s later" — is spread across seven events linked only by integer
back-references. Reconstructing that is the single hardest part of the port, and it is pure
logic that deserves to be tested without a server anywhere in sight.

---

## 2. The four crates · BUILT (3 of 4; `tmprl-ui` is M2)

```
┌──────────────────────────────────────────────────────────────────┐
│  tmprl-tui      ratatui. Input handling and drawing.             │
│                 Render is a pure function of &App.               │
├──────────────────────────────────────────────────────────────────┤
│  tmprl-ui       The vim window tree. Splits, tabs, focus.        │
│                 No ratatui types — just rectangles and a tree.   │
├──────────────────────────────────────────────────────────────────┤
│  tmprl-core     Domain logic. History normalisation, visibility  │
│                 queries, diff, the command registry, the keymap. │
│                 Pure and synchronous. No IO, no async.           │
├──────────────────────────────────────────────────────────────────┤
│  tmprl-client   All network IO. gRPC, TLS, profiles, codec.      │
│                 Knows nothing about the UI.                      │
└──────────────────────────────────────────────────────────────────┘
```

Dependencies point strictly downward. The split is not decoration — it is what makes the
project testable:

| Crate | Status | How it is tested | Tests |
|---|---|---|---|
| `tmprl-client` | built | Integration tests against `temporal server start-dev` | 18 |
| `tmprl-core` | built | Plain unit tests. No server, no terminal, no async runtime. | 75 |
| `tmprl-tui` | built | Rendered into ratatui's `TestBackend` and asserted on | 49 |
| `tmprl-ui` | planned (M2) | Plain unit tests over the layout tree | — |

That `tmprl-core` carries the most tests while needing the least to run them is the
arrangement working as intended.

The bulk of the difficult logic lives in `tmprl-core`, the layer that needs *nothing* to
test — no server, no terminal, no async runtime.

`tmprl-client` depends on `tmprl-core`. The domain types that carry logic — execution
status, a workflow row, the paged list — live in `tmprl-core`, and `tmprl-client` maps
protobuf into them. That way the ordering, deduplication and cursor-anchoring rules are
tested with no server in sight, and the generated types still stop at the client boundary.

### Why `tmprl-client` exists at all

`temporalio-client` is pre-1.0 and its public surface moves between releases. Rather than
scatter `temporalio_*` types through the UI, every RPC goes through a wrapper here. A version
bump then breaks one crate, loudly, in one place — instead of breaking forty call sites.

Two things we deliberately do *not* implement:

- **Profile loading.** `ClientOptions::load_from_config()` is Temporal's own loader. It reads
  `~/.config/temporalio/temporal.toml`, layers the `TEMPORAL_*` environment variables over it,
  and resolves TLS certificate files off disk. Writing our own would only drift from what the
  `temporal` CLI does, and "tmprl connects differently from the CLI" is a bug we would rather
  not be able to have.
- **Retries.** The client applies its own retry policy beneath the raw service traits.

One thing we *do* have to fix at this boundary: Temporal's `envconfig::ConfigError` boxes a
bare `dyn Error`, which is not `Sync`. That makes it unusable across `anyhow` and tokio task
boundaries. `ConnectError` flattens it to a string so that everything above this crate gets a
`Send + Sync` error.

---

## 3. Control flow · BUILT

The application is Elm-shaped: a single reducer over a single state value, with all IO
pushed to the edges.

```
      ┌─ crossterm EventStream ──┐
      │                          │
      ├─ backend mpsc::Receiver ─┼──►  tokio::select!
      │                          │           │
      └─ tick interval ──────────┘           ▼
                                     reduce(&mut App, Msg)
                                       │             │
                          spawn(task) ─┘             └─► dirty? → render(&App)
                               │
                               └──────────────► backend tx ──┐
                                                             │
                                          (loops back in as a Msg)
```

### The rule that everything else follows from

> **`reduce` is synchronous and never awaits.**

A keystroke is handled by mutating state and, if data is needed, *spawning* a task. The task
sends its result back as another message. Nothing in the input path can block on the network,
so no RPC — including a 60-second long poll — can freeze the UI.

The corollary is that **every piece of remote data is explicitly four-state**:

```rust
enum Loadable<T> {
    NotAsked,
    Loading,
    Loaded(T, Instant),   // with the time it was fetched, for staleness display
    Failed(Error),
}
```

Panes render all four. There is no code path where a view waits for data, because there is no
way to express waiting — only a way to express "not here yet", which draws a skeleton.

### Frame pacing

Rendering is dirty-flag driven: a frame is drawn only when a message actually changed
something. A 1 Hz tick keeps relative timestamps honest. Adaptive pacing — faster while
streaming, slower when idle — arrives with follow mode in M2, where there will finally be
something to animate. This is not micro-optimisation: the expected deployment is a TUI over
SSH inside tmux, where every redraw is bytes on a wire.

---

## 4. The command registry · BUILT

Every user-visible action is registered exactly once:

```rust
Command {
    id:    "workflow.terminate",
    title: "Terminate workflow",
    args:  &[Arg::Reason],
    run:   fn(&mut App, Args) -> Outcome,
}
```

Five separate features resolve through that one table:

1. **Key bindings** — `keys.toml` maps a chord to a command id
2. **The `:` command line** — resolves a typed name to a command id
3. **The which-key popup** — enumerates commands reachable from the current prefix
4. **Macro replay** — a macro is a recorded list of command ids and arguments
5. **Headless mode** — `tmprl --exec workflow.terminate --id=…` for scripting

This is the highest-leverage decision in the codebase. The alternative — a `match` on
`KeyEvent` in the input handler — makes all five of those features separate, divergent
implementations, and makes remapping impossible. Here, adding a command gets you all five for
free, and macros are portable text rather than replayed keystrokes.

It also means the answer to "what can this program do?" is a list you can print, rather than
something you infer from reading the input handler.

---

## 5. The workflow list · BUILT

The first screen that is a real port rather than a list of names, and the one that fixed the
assumptions the rest of the read path will inherit.

### The server does not sort, so we do

`ListWorkflowExecutions` returns rows in no defined order, and standard visibility rejects an
`ORDER BY` clause outright — the dev server answers `operation is not supported: 'ORDER BY'
clause`. Both facts are pinned by integration tests, so a future server that changes its mind
tells us.

Ordering is therefore the client's job. `WorkflowList` sorts newest-first on every append and
deduplicates by `(namespace, run_id)`:

- **Sorting on append, not once.** Pages arrive unordered, so a later page routinely contains
  rows that belong above rows already on screen.
- **Deduplicating on the pair, not the run id.** Run ids are unique per namespace, not across
  a fan-out. Deduplicating on the run id alone would silently drop a row.
- **Deduplicating at all.** A page is a snapshot of a set that is changing underneath it, so
  the same execution legitimately arrives twice. A table that lists a workflow twice makes an
  operator doubt the whole screen.

### The cursor is an identity, not an index

The list is live. Rows appear above the cursor while you are reading it, so a cursor stored
as a row index quietly ends up on a different workflow. The cursor is stored as the
`(namespace, run_id)` of the row it is on and re-found after every load.

### Stale replies are dropped, not painted

Every fetch carries a generation, bumped whenever the query or the scope changes. A reply
whose generation no longer matches is discarded. Without this, editing a query while a slow
request is in flight repaints the table with results for a query the user has already
abandoned — a race that shows up exactly when the cluster is slow, which is when it is least
welcome.

### The raw query is the interface

The visibility query is always on screen and always the literal string sent to the server.
Saved views fill it and leave it editable; the filter builder planned for M2 will compile
into it. Nothing holds a structured filter that renders down to a query the user cannot see
or correct — that abstraction is the most irritating thing about the web UI's filter bar, and
it is being deliberately rejected rather than ported.

The only query rewriting anywhere is what the RPCs demand: `CountWorkflowExecutions` does not
accept `ORDER BY` and needs its own `GROUP BY`, so `tmprl_core::query::count_query` strips and
appends those clauses. It skips quoted strings, so a workflow id containing the words `order
by` is not mistaken for a clause.

### Counts are one call

The header tallies come from a single `CountWorkflowExecutions ... GROUP BY ExecutionStatus`
rather than a call per status. The group values come back as `json/plain` Keyword payloads
holding a quoted status name. Grouped counts are approximate by Temporal's own documentation,
so the total is taken from the response's `count` field rather than summed from the groups.

### Fan-out is N requests on one channel

A `Conn` clone shares a single HTTP/2 channel, so listing several namespaces is one connection
and N concurrent streams. Each namespace pages independently and exhausts at a different
point, so the continuation token is per namespace rather than one token for the merged list.

---

## 6. Reconstructing history · PLANNED

This is the part that makes the difference between a port and a wrapper.

### What the server sends

A flat, ordered list of `HistoryEvent`. An activity that failed once and then succeeded
produces something like:

```
 5  ActivityTaskScheduled       activity_id="charge", type="ChargeCard"
 6  ActivityTaskStarted         scheduled_event_id=5
 7  ActivityTaskFailed          scheduled_event_id=5, started_event_id=6
 8  ActivityTaskScheduled       activity_id="charge"          ← retry
 9  ActivityTaskStarted         scheduled_event_id=8
10  ActivityTaskCompleted       scheduled_event_id=8, started_event_id=9
```

Six rows. One thing happened.

### What we build from it

**Stage 1 — normalise.** Each proto event becomes a `NormalizedEvent { id, time, group_key,
kind, title, fields, links }` through a single `match` over the attributes `oneof`.

That match is **exhaustive on purpose**. Temporal adds event types over time (Nexus and
worker-versioning events are recent additions). With a `_ => {}` arm, a new event type renders
as a blank row and nobody notices for months. Exhaustive, it is a compile error the moment we
bump the protos — which is exactly when we want to hear about it.

**Stage 2 — group.** Events are folded into groups by following their back-references
(`scheduled_event_id`, `initiated_event_id`, `started_event_id`). The six rows above become
one `ActivityGroup` with two attempts, a final status of `Completed`, and a duration.

Groups are the unit of everything downstream:

- **Compact view** — one row per group
- **Timeline view** — one Gantt bar per group, positioned by its start and end
- **Outline** — a collapsible tree of groups, for jumping around a long history
- **Diff** — two histories aligned by group key, via LCS

**Stage 3 — virtualise.** Only the visible slice is ever turned into rendered rows. Scrolling
a 100k-event history moves an index; it does not rebuild a list. A minimap strip down the
right edge shows failure density across the whole history, which is how you find the
interesting part of a long run without scrolling through it.

---

## 7. The window model · PLANNED

`tmprl-ui` holds a layout tree, not a fixed master-detail arrangement:

```rust
struct Tab { root: Window }

enum Window {
    Leaf(ViewId),
    Split { dir: Dir, children: Vec<Window>, sizes: Vec<u16> },
}
```

Splits and tabs behave like vim's, with vim's bindings. This buys two things.

The obvious one is that it feels right to anyone with vim in their fingers.

The less obvious one: **diff falls out of it for free.** Comparing a good run against a bad
run is just two workflow-detail views in a vertical split with linked scrolling, aligned by
compact-group key. There is no separate diff screen to build, and the comparison works for any
two views, not just the pair someone anticipated.

---

## 8. Payloads and the codec server · PLANNED

Temporal payloads are opaque bytes plus metadata. When a cluster uses a codec server, they are
also encrypted, and decoding requires an HTTP round trip to a service the user runs.

Decoding is therefore:

- **lazy** — only payloads currently on screen are decoded, never a whole history
- **cached** by payload hash
- **non-blocking** — an encoded value renders immediately with a `🔒` badge and is replaced in
  place when the decode resolves, following the same `Loadable` pattern as everything else

The wire contract is Temporal's: `POST {endpoint}/decode`, proto3-JSON `Payloads` in the body,
`X-Namespace` header, optional `Authorization`.

---

## 9. Mutations · PLANNED

`tmprl` can terminate workflows and run batch operations across thousands of them. The
safety design is deliberate:

- Every destructive action routes through **one** confirmation modal.
- That modal shows **the equivalent `temporal` CLI command**. This teaches the CLI, makes the
  action auditable at a glance, and gives the user a way to run it elsewhere if they would
  rather not trust the TUI.
- Batch operations show a `CountWorkflowExecutions` **dry run** first — how many workflows the
  query actually matches — and require typing that count to proceed.
- Every mutation appends to `~/.local/state/tmprl/audit.jsonl`.

The batch flow is reached through the quickfix list: select workflows in the table, `<C-q>` to
stage them, then run an operation over the staged set. Staging is a visible, editable list
rather than an invisible selection, because "which 4,000 workflows am I about to terminate?"
should be a question with an answer on screen.

---

## 10. Design rules

Four rules, in priority order. Most of the structure above exists to enforce them.

1. **Nothing blocks the input path.** `reduce` is sync. If you need data, spawn and let the
   result arrive as a message. If you find yourself wanting `.await` in a reducer, the state
   machine is missing a state.
2. **Domain logic stays out of the render path.** If it can be computed without knowing the
   terminal size, it belongs in `tmprl-core` where it can be unit tested.
3. **New behaviour is a `Command`, not a key handler.** Anything else silently loses
   remappability, the palette, macros and headless mode.
4. **Exhaustive matches over protocol enums.** No `_ => {}` over Temporal event types. We want
   the compiler to tell us when Temporal adds something.

---

## 11. Things that will bite you · BUILT

Collected because each one cost real time to discover.

| | |
|---|---|
| **`protoc` is a build requirement** | `temporalio-protos` compiles protos from source. Missing it fails inside a build script, which reads like a network error but isn't. |
| **Raw services hang off `Connection`, not `Client`** | `client.connection().workflow_service()`. There are same-named methods on `TemporalServiceClient` too, which makes the error message misleading. |
| **Protos are in `temporalio-common`** | `temporalio_common::protos::temporal::api::*::v1`, requiring a direct dependency on `temporalio-common`. Not `temporal-sdk-core-protos`, which is a different, older crate. |
| **`ConfigError` is not `Sync`** | It boxes a bare `dyn Error`. Flatten it at the crate boundary or it poisons every `anyhow` signature above it. |
| **`wait_new_event: true` blocks** | Correct in follow mode, a hang anywhere else. Any test touching history must pass `false`. |
| **Timestamps are `prost_wkt_types`** | Not `prost_types`. The generated protos use `prost_wkt_types::Timestamp` and nothing re-exports it, so reading a `start_time` needs a direct `prost-wkt-types` dependency. The compiler's "expected `prost_wkt_types::pbtime::Timestamp`" is the only clue. |
| **`ListWorkflowExecutions` is unordered** | And standard visibility rejects `ORDER BY` — `operation is not supported: 'ORDER BY' clause`. Sorting is the client's job; see §5. |
| **`GROUP BY` returns payloads** | An `AggregationGroup`'s `group_values` are `Payload`s, not strings: `json/plain`, type `Keyword`, data `"Running"` *with* the quotes. |
| **Grouped counts are approximate** | Temporal documents this. Sum the groups and you understate the total, so read `response.count` for the total instead. |
| **Debug info does not fit** | 228 crates, and `temporalio-protos` dominates. `[profile.dev] debug = false`; use `--profile dbg` when you actually need a debugger. |
