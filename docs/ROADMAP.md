# Roadmap

Where `tmprl` goes next, and what it has decided not to become. The order is set by one
question: what stops someone who is not the author from relying on this? Depth for a power
user comes after that, not before.

Nothing here is a date. A release ships when its theme is true; until then, work ships as
patches and candidates. The 0.2 to 0.4 releases went out before this rule held, so the themes
start again at 0.5.

| Release | Theme | The test it has to pass |
|---|---|---|
| 0.5 | Trustworthy for strangers | A slow cluster, a dropped VPN, a light terminal and a pasted id all behave |
| 0.6 | Workflow complete | Anything the web UI does to one workflow, tmprl does too |
| 0.7 | Infrastructure and sharing | "Is anything polling this queue?" and "send me what you are looking at" have answers |
| 0.8 | Editor depth | The vim model is finished rather than suggested |

Effort is S (days), M (a week or two) or L (several weeks). Within a release, open rows are
in the order they will be built: how often the item is reached for while debugging, against
what it costs.

---

## 0.5 · Trustworthy for strangers

Every item is something that failed in a way a new user would read as the tool being broken.
Shipped as 0.5.0, with its two open items, help that knows the screen and query bar views,
moved to the top of 0.6.

| Item | Today | Effort |
|---|---|---|
| ✓ Request deadlines and reconnect | Done. The SDK already gives each RPC a 30s deadline (70s for a long poll), keepalive, and a channel that reconnects; the connect and the codec client now time out too, and follow and the retrying-activity poll retry with backoff instead of stopping | M |
| ✓ Errors that say what happened | Done. A failure keeps its gRPC code to the UI, names what to try, and `:messages` keeps every note of the session | M |
| ✓ Large histories | Done: each page is folded into the outline, so 100k events arrive in about 1.5 seconds instead of 52, with opt-in benchmarks (`cargo test -- --ignored bench_`) at 100k events and at the 50 MB history limit. A multi-megabyte payload is formatted once and scrolls without hanging | M |
| ✓ Paste and line editing | Done: every prompt and the query bar have a cursor, word and line deletes, and history. A paste is text, and with nothing open it goes to the workflow finder | S |
| ✓ Colours | Done: `theme.toml`, `NO_COLOR`, a 16-colour palette that follows the terminal's own theme, and `[display] colors` to force the depth | M |
| ✓ CLI flags | Done: `--namespace`, `--query`, `--workflow`, `--address`, `--readonly`, `--log` | S |
| ✓ `:` command line | Done: `<Tab>` and `<S-Tab>` fill in the offers, `<Up>` recalls, `:q` quits | S |
| ✓ Schedules | Done: every schedule, from every namespace in scope | S |
| ✓ Reasons | Done: terminate and reset ask for the reason, pre-filled with the old default | S |
| ✓ Read-only gaps | Done. The create-schedule form refuses before it opens | S |
| ✓ Project hygiene | Done. `CONTRIBUTING.md`, `SECURITY.md`, issue forms, and CI against two server versions | S |

## 0.6 · Workflow complete

| Item | Today | Effort |
|---|---|---|
| Help that knows the screen | From 0.5. `?` and which-key list every command everywhere; a command that does not apply refuses when run. Wanted: commands declare where they apply | M |
| Query bar views | From 0.5. Saving the current query as a view needs `views.toml` edited by hand | S |
| ✓ Workflow info pane | Done: a workflow bar on every history, and `gi` opens a card under it that starts with what it is waiting on, each with its state, then the run, timing, timeouts, the SDK and its language, the last worker and its build, the pollers, lineage, memo and search attributes. The next three rows read the same reply | M |
| ✓ Going to the source | Done: `gf` asks a resolver named in `config.toml` where an event's code is, decoding its payloads first, and lists the places it passed through in a picker. `demo/find-source.sh` is a resolver for a Go worker | M |
| ✓ Going to the source with no resolver | Done: with no resolver, or when it fails, `gf` follows the failure's stack trace (Go, Java, Node, Python, .NET) to a file in the repository, and with no trace (a Go returned error) looks the activity or workflow up by its registered name, in `[source] roots`. Still wanted: a language server's references, for a name the text alone cannot follow | M |
| Retry policy | Attempts, backoff and the last failure are shown. The policy itself is not, and heartbeat and last-started times are fetched but never drawn | S |
| Run chain | `newRunId` and `firstRunId` are shown as text. Wanted: next and previous run, and every run of an id as a list | S |
| Filter by an attribute's value | A workflow's own search attributes are never shown. With the pane above, a row becomes a filter clause. `/` on the workflow list also matches memo and search attribute values, which the list already carries | S |
| Signal and update payloads | Both send a name and no input. An update's result is discarded. Ships with codec encode, never before it | M |
| Codec encode | The codec client only decodes, so anything sent to an encrypted namespace would go out as plaintext. Outgoing payloads need `/encode` first | S |
| Task queue describe | The dashboard shows backlog, its age and the poller count for queues with running workflows, and the info card lists who polls a running workflow's queue, or that nobody does. Wanted: a view of one queue, with each poller and the rates, reached from the info card's task queue or a typed name. A stuck workflow with nothing polling its queue is the most common stall | M |
| Build id display | The info card shows the build of the last worker to complete a workflow task. Wanted: on each poller too; no browser, see below | S |
| Reset options | No choice of what to reapply, and no statement of which events are discarded | S |
| Query | Built: `<leader>cq` picks a handler from the workflow's own `__temporal_workflow_metadata`, falls back to the names an older SDK's refusal lists, then to a typed name, and shows the answer in the payload pane. `<leader>cs` shows the call stack through `__stack_trace`. Still wanted: `<leader>cQ` runs every handler and shows one object keyed by name; `<leader>cq` on a `V` selection runs one handler on each workflow, of one type, capped. Input waits for signal and update payloads and codec encode | M |
| Batch over a query | Temporal's server-side batch, with a `CountWorkflowExecutions` dry run and the count typed to confirm | M |
| ✓ Child and parent navigation | Done: `gc` on a child's row opens its history, `gp` opens the parent, `<C-o>` comes back | S |

## 0.7 · Infrastructure and sharing

| Item | Today | Effort |
|---|---|---|
| Export | `Y` yanks JSON, and past the yank limit writes it to a file in the temp directory. Wanted: a Markdown summary of a workflow and JSON to a file | M |
| Open a history from a file | The app cannot start without a server. Wanted: `tmprl --history <file>` and a capture command. This also gives a demo with no server and fixtures for tests | M to L |
| Headless `--exec` | No non-interactive mode. Every action already runs through one dispatch point. Wanted: JSON output of what only tmprl computes, the grouped history and the dashboard, which is also the interface for agents | M |
| Stricter confirmation per profile | Three tiers exist. Wanted: an opt-in that asks for more on a named profile | S |
| Dashboard | Built, `gd` or `--dashboard`: counts by status, recent failures, the types failing most, activities being retried, task queues in use and schedules, each item one `Enter` from the workflows it stands for, laid out by `dashboard.toml`, or by what the namespace shows when there is none. Refreshes itself, `[refresh] dashboard`. Shows each task queue's backlog and pollers. Still wanted: arranging panels in the app | L |
| ✓ Switch profile in the app | Done: `<leader>P`, or `-` from the namespace list, and the cluster address is in the header. Still one profile per process; a pane on its own profile is the table across clusters below | M |

## 0.8 · Editor depth

| Item | Today | Effort |
|---|---|---|
| Diff | Two histories aligned by group with linked scrolling, in two splits | L |
| A cursor inside a payload | Folding, yanking a path or a sub-value. Today the pane scrolls and wraps, and nothing more | L |
| Quickfix | A visible, editable list of workflows to step through with `]q` and `[q`, and to batch over | M |
| Marks | The jumplist exists, marks do not | S |
| Pipe polish | Yank the output of `!` | S |
| Macros and `.` | Recorded as command ids | M |

---

## Not planned

| | Why |
|---|---|
| A query builder form | The raw visibility query is the interface; see [ARCHITECTURE §5](ARCHITECTURE.md#the-raw-query-is-the-interface). Completion and `<leader>fg` already write clauses into it |
| An incident mode | It is a saved workspace, notes and an export under one name. Notes belong in an editor, and export is on the list by itself |
| A table of counts across clusters | A connection per cluster to draw one screen. Switching profile is on the list |
| A list of every task queue | Temporal has no RPC that lists them. A queue is reached from a workflow that uses it |
| Each failed attempt of an activity | The server keeps the last failure only. tmprl shows that |
| Steps a workflow has not reached yet | A history records what happened. Nothing in it describes what will |
| Deployment and versioning screens, a Nexus screen, archival | Out of scope. Build ids are displayed where a workflow or a poller carries one |
| Restoring a session's layout | Opening straight onto a query or a workflow covers most of the need. This is the windows of a session; the dashboard's own layout is a config file |
| Plugins | Not while command ids, config keys and the client boundary are still moving |
| Built-in AI | `!` already pipes a payload to any tool the user chooses |
| An MCP server | It serves an agent, not a terminal. Community MCP servers already wrap the RPCs, `--exec` gives agents the grouped output, and a mutation must stay behind the human confirmation |
| Searching the input of every workflow | One history fetch and one codec call per workflow, against the namespace's rate limit. A search attribute set by the worker is the fix, and then the query bar finds it |

## How an item gets on this list

It has to make a workflow easier to understand, navigate or operate from a terminal, and
Temporal has to expose the data it needs. A request that fails the second test goes under
*Not planned* with the reason, so it is not proposed twice.
