# Roadmap

Where `tmprl` goes next, and what it has decided not to become. The order is set by one
question: what stops someone who is not the author from relying on this? Depth for a power
user comes after that, not before.

Nothing here is a date. A release ships when its theme is true; until then, work ships as
patches and prereleases. The 0.2 to 0.4 releases went out before this rule held, so the themes
start again at 0.5.

| Release | Theme | The test it has to pass |
|---|---|---|
| 0.5 | Trustworthy for strangers | A slow cluster, a dropped VPN, a light terminal and a pasted id all behave |
| 1.0 | Stable | The keys, commands, config keys and flags in `docs/public-api.txt` and `docs/INTERFACE.md` stop changing |
| 1.x | Workflow complete | Anything the web UI does to one workflow, tmprl does too |
| 1.x | Infrastructure and sharing | "Is anything polling this queue?" and "send me what you are looking at" have answers |
| 1.x | Editor depth | The vim model is finished rather than suggested |

1.0 comes straight after 0.5, through `1.0.0-alpha`, `-beta` and `-rc` (see
[RELEASING](RELEASING.md#the-road-to-10)). Everything after it adds keys and commands rather
than changing them, so it ships as 1.x minors; an item that would rename or remove one waits
for 2.0.

Effort is S (days), M (a week or two) or L (several weeks).

---

## 0.5 · Trustworthy for strangers

Every item is something that fails today in a way a new user would read as the tool being
broken.

| Item | Today | Effort |
|---|---|---|
| ✓ Request deadlines and reconnect | Done. The SDK already gave each RPC a 30s deadline, keepalive and a channel that reconnects; the connect and the codec client now time out too, and follow and the pending-activity poll retry with backoff instead of stopping | M |
| ✓ Errors that say what happened | Done. A failure keeps its gRPC code to the UI, names what to try, and `:messages` keeps every note of the session | M |
| ✓ Large histories | Done. Grouping was a linear lookup per event, 52s for 100k events; now 1.5s, with opt-in benchmarks at 100k events and at the 50 MB history limit. Each page still regroups everything loaded, which the benchmarks show is cheap enough | M |
| ✓ Paste and line editing | Done: one line editor with a cursor, the shell's keys and `<Up>` history, and bracketed paste. A paste with nothing open goes to the workflow finder | S |
| ✓ Colours | Done: `theme.toml`, `NO_COLOR`, a 16-colour palette that follows the terminal's own theme, and `[display] colors` to force the depth | M |
| ✓ CLI flags | Done: `--namespace`, `--query`, `--workflow`, `--address`, `--readonly`, `--log` | S |
| ✓ `:` command line | Done: `<Tab>` and `<S-Tab>` fill in the offers, `<Up>` recalls, `:q` quits | S |
| Help that knows the screen | `?` and which-key list every command everywhere; a command that does not apply refuses when run. Wanted: commands declare where they apply. Moved after 1.0: it changes what help shows, not a key or an id | M |
| Query bar | Cursor movement and history are done, with the line editor. Still wanted: saving the current query as a view | S |
| ✓ Schedules | Done: every page, from every namespace in scope | S |
| ✓ Reasons | Done: a reason prompt, pre-filled with the old default | S |
| ✓ Read-only gaps | Done. The create-schedule form refuses before it opens | S |
| ✓ Project hygiene | Done. `CONTRIBUTING.md`, `SECURITY.md`, issue forms, and CI against two server versions | S |

## 1.x · Workflow complete

| Item | Today | Effort |
|---|---|---|
| Workflow info pane | `DescribeWorkflowExecution` is already called and everything but the pending activities is discarded. Wanted on the history screen: status, type, run id, task queue, times, timeouts, memo, search attributes, parent and root, pending children, the pending workflow task | M |
| Filter by an attribute's value | A workflow's own search attributes are never shown. With the pane above, a row becomes a filter clause. `/` on the workflow list also matches memo and search attribute values, which the list already carries | S |
| Signal and update payloads | Both send a name and no input. An update's result is discarded | M |
| Query | Built: `<leader>cq` picks a handler from the workflow's own `__temporal_workflow_metadata`, falls back to the names an older SDK's refusal lists, then to a typed name, and shows the answer in the payload pane. `<leader>cs` shows the call stack through `__stack_trace`. Still wanted: `<leader>cQ` runs every handler and shows one object keyed by name; `<leader>cq` on a `V` selection runs one handler on each workflow, of one type, capped. Input waits for signal and update payloads and codec encode | M |
| Codec encode | The codec client only decodes, so anything sent to an encrypted namespace would go out as plaintext. Outgoing payloads need `/encode` first | S |
| Run chain | `newRunId` and `firstRunId` are shown as text. Wanted: next and previous run, and every run of an id as a list | S |
| ✓ Child and parent navigation | Done: `gc` opens the child under the cursor, `gp` the parent, `<C-o>` comes back. A group's ids show in the payload pane and `y` copies them | S |
| Reset options | No choice of what to reapply, and no statement of which events are discarded | S |
| Retry policy | Attempts, backoff and the last failure are shown. The policy itself is not, and heartbeat and last-started times are fetched but never drawn | S |
| Batch over a query | Temporal's server-side batch, with a `CountWorkflowExecutions` dry run and the count typed to confirm | M |

## 1.x · Infrastructure and sharing

| Item | Today | Effort |
|---|---|---|
| Dashboard | Built, `gd` or `--dashboard`: counts by status, recent failures, the types failing most, activities being retried, task queues in use and schedules, each item one `Enter` from the workflows it stands for, laid out by `dashboard.toml`, or by what the namespace shows when there is none. Refreshes itself, `[refresh] dashboard`. Shows each task queue's backlog and pollers. Still wanted: arranging panels in the app | L |
| Task queue describe | The dashboard shows backlog, its age and the poller count for queues with running workflows. Still wanted: a view of one queue, with each poller and the rates, reached from a workflow's task queue or a typed name | M |
| Build id display | Nothing. Shown on the info pane and on pollers; no browser, see below | S |
| Open a history from a file | The app cannot start without a server. Wanted: `tmprl --history <file>` and a capture command. This also gives a demo with no server and fixtures for tests | M to L |
| Export | `Y` yanks JSON, and past the yank limit writes it to a file in the temp directory. Wanted: a Markdown summary of a workflow and JSON to a file | M |
| ✓ Switch profile in the app | Done: `<leader>P`, or `-` from the namespace list, and the cluster address is in the header. Still one profile per process; a pane on its own profile is the table across clusters below | M |
| Stricter confirmation per profile | Three tiers exist. Wanted: an opt-in that asks for more on a named profile | S |
| Headless `--exec` | No non-interactive mode. Every action already runs through one dispatch point. Wanted: JSON output of what only tmprl computes, the grouped history and the dashboard, which is also the interface for agents | M |

## 1.x · Editor depth

| Item | Today | Effort |
|---|---|---|
| Quickfix | A visible, editable list of workflows to step through with `]q` and `[q`, and to batch over | M |
| Marks | The jumplist exists, marks do not | S |
| Diff | Two histories aligned by group with linked scrolling, in two splits | L |
| A cursor inside a payload | Folding, yanking a path or a sub-value. Today the pane scrolls and wraps, cut at 5,000 lines, and formats a payload once rather than per key press | L |
| Pipe polish | Yank the output of `!`, and a history for its prompt | S |
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
