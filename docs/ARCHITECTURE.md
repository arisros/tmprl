# Architecture

> **Read this first.** This document mixes built code with design that is not written yet,
> and every section says which it is. §§2–8 are implemented and tested and §9 partly so;
> what remains is written down in advance so the shape is agreed before there is code
> sitting on top of it.
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

## 2. The four crates · BUILT

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
| `tmprl-client` | built | Integration tests against `temporal server start-dev`, and the codec client against a real socket | 46 |
| `tmprl-core` | built | Plain unit tests. No server, no terminal, no async runtime. | 131 |
| `tmprl-tui` | built | Rendered into ratatui's `TestBackend` and asserted on | 127 |
| `tmprl-ui` | built | Plain unit tests over the layout tree | 35 |

That `tmprl-core` carries the most tests while needing the least to run them is the
arrangement working as intended.

Every pane on screen is a leaf of `tmprl-ui`'s tree — see §7.

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

Follow mode is where that stops being hypothetical. `GetWorkflowExecutionHistory` with
`wait_new_event: true` does not return until the workflow does something, so tailing is a task
that spends most of its life parked inside a single RPC, pushing batches of events back as
messages. The reducer never waits on it; it only starts it, and aborts it when following stops
or the screen is left. Aborting matters — a poll left running holds a request open and keeps
feeding a view that has moved on.

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
something. A 1 Hz tick keeps relative timestamps honest. This is not micro-optimisation: the
expected deployment is a TUI over SSH inside tmux, where every redraw is bytes on a wire.

This section used to promise *adaptive pacing* — faster while streaming, slower when idle —
"with follow mode in M2, where there will finally be something to animate". Follow mode is
built, and adaptive pacing turned out to be unnecessary. A batch of tailed events arrives as
an ordinary message, which marks the frame dirty and draws it; when nothing is happening no
message arrives and nothing is drawn. The dirty flag *is* the adaptive pacing. Adding a
second mechanism would have meant drawing on a timer rather than on a change, which is
strictly more bytes on the wire for the same picture.

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

## 6. Reconstructing history · BUILT

This is the part that makes the difference between a port and a wrapper.

### What the server sends

A flat, ordered list of `HistoryEvent`. An activity that failed once and then succeeded
produces:

```
 5  ActivityTaskScheduled       activity_id="charge", type="ChargeCard"
 6  ActivityTaskStarted         scheduled_event_id=5, attempt=2,
                                last_failure="card declined"
 7  ActivityTaskCompleted       scheduled_event_id=5, started_event_id=6
```

Three rows. One thing happened.

> **Corrected.** This section previously showed the retry as a *second*
> `ActivityTaskScheduled` at event 8, with the later events back-referencing it. That is not
> what Temporal does. Retries are transparent: one scheduling event covers every attempt, and
> `ActivityTaskStarted` carries `attempt` ("starting at 1, the number of times this task has
> been attempted") together with `last_failure` ("the most recent failure details, if this
> task has previously failed and then been retried"). Both quotes are from the protobuf
> definitions. Grouping code written against the old sketch would have looked for a second
> scheduling event that never arrives, and would have reported every retried activity as one
> attempt.
>
> Confirmed against a real worker, not just the protobuf comments. An activity that failed
> twice and then succeeded produces exactly the three events above — `ActivityTaskScheduled`
> once, at event 5, with `ActivityTaskStarted` carrying `attempt=3`.

### What we build from it

**Stage 1 — normalise · BUILT.** Each proto event becomes a `NormalizedEvent { id, time,
name, category, group, role, outcome, subject, attempt, failure, fields }` through a single
`match` over the attributes `oneof`, in `tmprl-client`. Sixty arms, one per event type.

That match is **exhaustive on purpose**. Temporal adds event types over time (Nexus and
worker-versioning events are recent additions). With a `_ => {}` arm, a new event type renders
as a blank row and nobody notices for months. Exhaustive, it is a compile error the moment we
bump the protos — which is exactly when we want to hear about it.

The grouping key is not uniform across the protocol, so each arm names the back-reference it
follows rather than sharing a guess:

| Family | Key |
|---|---|
| activities, workflow tasks, Nexus operations | `scheduled_event_id` |
| child and external workflows | `initiated_event_id` |
| timers | `started_event_id` — the id of the `TimerStarted` event |
| updates | `accepted_event_id` |
| the workflow itself | no key; one group per execution |

`workflow_task_completed_event_id` is on most of these too and is *not* the key. It points at
the workflow task that issued the command, so following it would file every activity, timer
and child workflow under the task that scheduled it.

**Stage 2 — group · BUILT.** Normalised events are folded into groups in one forward pass,
in `tmprl-core`, where the rules are tested with hand-built events and no server. The three
rows above become one group with two attempts, a final outcome of `Completed`, and a duration.

Groups are keyed by the id of the event that opened them, because that is what every
back-reference in the protocol actually points at. Two properties are worth stating:

- **Interleaving is normal.** Several activities run at once and their events arrive
  interleaved, so a group is assembled by key, never by adjacency.
- **An orphan opens its own group.** A page that starts mid-run back-references events it does
  not contain. Dropping those would render the page empty and look like a bug in tmprl, so an
  unmatched event opens a partial group instead.

Groups are the unit of everything downstream:

- **Compact view** — one row per group
- **Timeline view** — one Gantt bar per group, positioned by its start and end
- **Outline** — a collapsible tree of groups, for jumping around a long history
- **Diff** — two histories aligned by group key, via LCS

**Stage 3 — virtualise · BUILT.** Only the visible slice is ever turned into rendered rows.
Scrolling a 100k-event history moves an index; it does not rebuild a list.

`Outline` keeps one cumulative-offset table over the visible groups, rebuilt when the *shape*
changes — a group folded, plumbing toggled — and never while scrolling. A row lookup is then a
binary search over that table, so asking for row 84,102 costs the same as asking for row 0.
`len()` is the last entry in that table rather than a count of anything.

Two consequences that are easy to get wrong, both pinned by tests:

- **Folding shut from inside a group** would strand the cursor past the end, so `toggle`
  returns the row the group's own line now occupies and the cursor moves there.
- **The summary must not count what the outline hides.** Workflow tasks are folded away by
  default; reporting one as "running" in the header sends the reader hunting for a row that is
  not on screen.

Still planned: the minimap strip down the right edge showing failure density across the whole
history. `]f` and `[f` already jump between failures, which serves the same need with a key
rather than a picture.

---

## 7. The window model · BUILT

`tmprl-ui` holds a layout tree, not a fixed master-detail arrangement:

```rust
struct Tabs { tabs: Vec<Tree>, current: usize }

enum Node {
    Leaf(ViewId),
    Split { axis: Axis, children: Vec<Node>, weights: Vec<u16> },
}
```

The crate has no dependencies at all — not even ratatui. A layout is a tree and some
arithmetic, and keeping the terminal out of it is what lets the rules be tested as plain
functions: where focus goes on `<C-w>l`, what a split's siblings inherit when one closes,
what happens at a terminal size too small to show everything.

Four decisions worth recording, each pinned by a test:

- **`Axis` is named for what you see**, `Columns` and `Rows`, not "horizontal" and "vertical".
  vim's `:split` is called horizontal and stacks windows *vertically*; that ambiguity is
  exactly how transposed layouts get written.
- **Sizes are weights, not cells.** A layout keeps its proportions when the terminal is
  resized, so an arrangement the reader set up is not scrambled by a window drag.
- **Focus moves geometrically.** The window to the right is the one that *looks* right, which
  is not always a sibling; candidates must share rows with the current window, so `<C-w>j` in
  a tall pane does not jump to something in another column that happens to sit lower.
- **Same-axis splits flatten.** Nested same-axis splits look identical on screen but make
  focus movement step through invisible levels. Flattening preserves the proportions rather
  than evening them up — `:vsplit` twice gives one half and two quarters in vim too, and
  `<C-w>=` is a separate decision.

Splits and tabs behave like vim's, with vim's bindings. This buys two things.

The obvious one is that it feels right to anyone with vim in their fingers.

The less obvious one: **diff falls out of it for free.** Comparing a good run against a bad
run is just two workflow-detail views in a vertical split with linked scrolling, aligned by
compact-group key. There is no separate diff screen to build, and the comparison works for any
two views, not just the pair someone anticipated.

### Wiring it in

The application used to hold one cursor and one history, which is what made a second pane
impossible rather than merely unwritten. Everything a window owns now lives in a `View` —
screen, cursor, query, scope, history, follow task, paging tokens, payload pane — and
everything belonging to the session stays on `App`: the mode, the keymap, the prompt, the
note line, the codec cache. There is one keyboard and one status line however many panes are
open, and one decode cache is right because the same payload in two panes should cost one
round trip.

The focused pane's `View` is held directly on `App` and the others wait in a map; moving focus
swaps between the two. That is what lets the whole reducer keep saying `self.view` without a
lookup that could fail.

Two consequences worth stating:

- **`generation` is per pane.** Two panes fetching at once must not invalidate each other's
  replies, which one shared counter would do.
- **`View` has a `Drop` that aborts its follow task.** Closing a window must not leave a long
  poll running against a pane that no longer exists.

A split forks the pane's *navigation* — screen, scope, query, which workflow — but none of its
loaded data. Splitting is almost always "show me this again so I can take one of them
somewhere else", and landing back at the namespace list would make the diff case two
navigations instead of one keystroke.

---

## 8. Payloads and the codec server · BUILT

Temporal payloads are opaque bytes plus metadata. When a cluster uses a codec server, they are
also encrypted, and decoding requires an HTTP round trip to a service the user runs.

### Rendering · BUILT

The encoding string in a payload's metadata decides everything, and deciding is pure, so it
lives in `tmprl-core` and is tested without a server:

| Encoding | Shown as |
|---|---|
| `json/plain`, `json/protobuf` | pretty-printed text |
| `text/plain` | text |
| `binary/null` | `null` — distinct from an empty string, which is a value |
| `binary/encrypted` | a `🔒` badge and a byte count |
| anything else | encoding name and byte count, **not** decoded |

That last row is the one worth defending. A payload from a custom converter can be arbitrary
bytes, and optimistically decoding those as UTF-8 is how a terminal fills with control
characters. The same applies when a payload *claims* `json/plain` but is not valid UTF-8: the
declaration is wrong, so it is not trusted enough to print. A value that is valid UTF-8 but
not valid JSON is shown raw rather than rejected — seeing the bytes beats being told they were
unparseable.

`K` opens the pane on the focused row. For a group it gathers the input from the event that
opened the group and the result from the event that closed it, because those are two different
events and showing only the row you happen to be on would hide one of them.

`!` filters that same set through an external command. What goes down the pipe is a JSON
object keyed by payload label rather than a single value — a row carries more than one, so
piping "the payload" would have to pick one arbitrarily. Keyed, `jq .result` picks it
explicitly. The command runs through a shell so that pipelines work, on a spawned task so that
a slow filter cannot freeze a keystroke, and its output arrives as an ordinary message.

### Decoding · BUILT

Decoding is:

- **lazy** — only payloads currently on screen are decoded, never a whole history
- **cached** by payload hash
- **non-blocking** — an encoded value renders immediately with a `🔒` badge and is replaced in
  place when the decode resolves, following the same `Loadable` pattern as everything else

The wire contract is Temporal's: `POST {endpoint}/decode`, proto3-JSON `Payloads` in the body,
`X-Namespace` header, optional `Authorization`. The endpoint comes from `config.toml`.

**proto3-JSON is the trap.** Every `bytes` field is base64 — the payload data *and each
metadata value*. Sending `metadata.encoding` as the plain string `binary/encrypted` produces a
body that looks right to a reader and is wrong on the wire: a conforming server base64-decodes
it, gets nonsense, and refuses the payload. Unit tests assert the shape; a second set runs the
client against a real socket and asserts what the server actually receives, because a shape
assertion can agree with itself while the wire is still wrong.

A decoded payload is swapped into the history *in place* rather than kept in a cache the views
consult. That way the pane, `!` piping and yanking all read the plaintext without knowing a
codec exists, and it is why the swap runs again after each history page — a later page can
carry a value already decoded from an earlier one.

Verified against Temporal's own `converter.NewPayloadCodecHTTPHandler` — the reference
implementation of this contract — with a worker whose data converter genuinely encrypts. That
matters more than it sounds: testing a client against one's own reading of a spec proves only
that the reading is self-consistent. A history carrying four `binary/encrypted` payloads shows
the badge with no endpoint set, and their plaintext once one is. `!` piping then works on the
decoded values without knowing a codec was involved, which is the in-place swap doing its job.

Two failure modes are refused rather than guessed around:

- **A short response.** The server must return as many payloads as it was given, in order.
  Pairing two sent with one returned would show one value's plaintext under another's label.
- **No credential of our own.** `Authorization` is sent only when `config.toml` sets it. A
  codec server is a service the user runs, and forwarding a credential they did not configure
  would be a surprise.

---

## 9. Mutations · PARTLY BUILT (single workflows; batch is M5)

`tmprl` can terminate workflows and run batch operations across thousands of them. The
safety design is deliberate:

- Every destructive action routes through **one** confirmation modal.
- That modal shows **the equivalent `temporal` CLI command**. This teaches the CLI, makes the
  action auditable at a glance, and gives the user a way to run it elsewhere if they would
  rather not trust the TUI.
- Batch operations show a `CountWorkflowExecutions` **dry run** first — how many workflows the
  query actually matches — and require typing that count to proceed.
- Every mutation appends to `~/.local/state/tmprl/audit.jsonl`.

Built: **cancel, terminate, signal and delete**, on one workflow at a time. Reset and update
are not; they need more than an execution id — a reset needs an event to go back to, and an
update needs a handler and a reply to wait for.

The rendered command has to be *right*, because someone will copy it and run it. The flags are
checked against `temporal workflow --help` rather than remembered, and values are shell-quoted:
a workflow id containing a space or a `;` must not turn into two commands. That quoting is
tested, not assumed.

Delete is the one action that asks for more than a keypress — it destroys the history itself,
not just the run, so it wants the word `delete` typed. Everything else is one confirmation, as
above. While a confirmation is up it owns every key, so nothing bound elsewhere can fire
underneath it.

Every request carries an `identity` of `tmprl@$USER`, which Temporal records on the resulting
event. A workflow terminated from here says so in its own history rather than appearing to
have stopped on its own.

The audit log is appended to, never rewritten, and **failures go in too** — the question it
answers is what was *attempted*. A failed write to it is surfaced rather than swallowed,
because that log is the record that an irreversible thing happened.

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
