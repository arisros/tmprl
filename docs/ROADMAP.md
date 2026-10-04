# Roadmap

Where `tmprl` goes next, and what it has decided not to become. The order is set by one
question: what stops someone who is not the author from relying on this? Depth for a power
user comes after that, not before.

Nothing here is a date. A release ships when its theme is true.

| Release | Theme | The test it has to pass |
|---|---|---|
| 0.2 | Trustworthy for strangers | A slow cluster, a dropped VPN, a light terminal and a pasted id all behave |
| 0.3 | Workflow complete | Anything the web UI does to one workflow, tmprl does too |
| 0.4 | Infrastructure and sharing | "Is anything polling this queue?" and "send me what you are looking at" have answers |
| 0.5 | Editor depth | The vim model is finished rather than suggested |

Effort is S (days), M (a week or two) or L (several weeks).

---

## 0.2 · Trustworthy for strangers

Every item is something that fails today in a way a new user would read as the tool being
broken.

| Item | Today | Effort |
|---|---|---|
| Request deadlines and reconnect | No timeout on any RPC or on the codec HTTP client. One connection, made at startup, never re-made. The pending-activity poll stops at its first error | M |
| ✓ Errors that say what happened | Done. A failure keeps its gRPC code to the UI, names what to try, and `:messages` keeps every note of the session | M |
| Large histories | Each arriving page re-merges, regroups and clones every event loaded so far. No benchmark exists. Wanted: incremental paging, and benchmarks at 100k events and with multi-megabyte payloads | M |
| Paste and line editing | Prompts append and backspace only. Bracketed paste is not handled, so text pasted in Normal mode runs as keys | S |
| ✓ Colours | Done: `theme.toml`, `NO_COLOR`, and a 16-colour palette that follows the terminal's own theme. Still wanted: a key to force the depth where `COLORTERM` is not exported | M |
| ✓ CLI flags | Done: `--namespace`, `--query`, `--workflow`, `--address`, `--readonly`. Still wanted: `--log` | S |
| `:` command line | The completion list cannot be navigated and `<Tab>` does nothing. No history, no `:q` | S |
| Help that knows the screen | `?` and which-key list every command everywhere; a command that does not apply refuses when run. Wanted: commands declare where they apply | M |
| Query bar | No cursor movement inside the line, no history, no way to save the current query as a view | S |
| Schedules | Only the first 50 are fetched, and only from the first namespace in scope | S |
| Reasons | Terminate and reset send a fixed reason | S |
| ✓ Read-only gaps | Done. The create-schedule form refuses before it opens | S |
| ✓ Project hygiene | Done. `CONTRIBUTING.md`, `SECURITY.md`, issue forms, and CI against two server versions | S |

## 0.3 · Workflow complete

| Item | Today | Effort |
|---|---|---|
| Workflow info pane | `DescribeWorkflowExecution` is already called and everything but the pending activities is discarded. Wanted on the history screen: status, type, run id, task queue, times, timeouts, memo, search attributes, parent and root, pending children, the pending workflow task | M |
| Filter by an attribute's value | A workflow's own search attributes are never shown. With the pane above, a row becomes a filter clause | S |
| Signal and update payloads | Both send a name and no input. An update's result is discarded | M |
| Query | No `QueryWorkflow`. Also unlocks the call stack through `__stack_trace` | M |
| Codec encode | The codec client only decodes, so anything sent to an encrypted namespace would go out as plaintext. Outgoing payloads need `/encode` first | S |
| Run chain | `newRunId` and `firstRunId` are shown as text. Wanted: next and previous run, and every run of an id as a list | S |
| Child and parent navigation | A child's ids are shown as text. Wanted: `Enter` on a child group opens its history, and a key goes to the parent | S |
| Reset options | No choice of what to reapply, and no statement of which events are discarded | S |
| Retry policy | Attempts, backoff and the last failure are shown. The policy itself is not, and heartbeat and last-started times are fetched but never drawn | S |
| Batch over a query | Temporal's server-side batch, with a `CountWorkflowExecutions` dry run and the count typed to confirm | M |

## 0.4 · Infrastructure and sharing

| Item | Today | Effort |
|---|---|---|
| Dashboard | Built with a fixed layout, `gd`: counts by status, recent failures, the types failing most, task queues in use and schedules, each item one `Enter` from the workflows it stands for. Still wanted: `dashboard.toml`, panels chosen from what the namespace shows, auto-refresh, task queue health on the panel, arranging panels in the app | L |
| Task queue describe | Nothing. `DescribeTaskQueue` gives pollers, backlog count and age, and rates. Reached from a workflow's task queue or a typed name | M |
| Build id display | Nothing. Shown on the info pane and on pollers; no browser, see below | S |
| Open a history from a file | The app cannot start without a server. Wanted: `tmprl --history <file>` and a capture command. This also gives a demo with no server and fixtures for tests | M to L |
| Export | `Y` yanks JSON, capped at 64 KB. Wanted: a Markdown summary of a workflow and JSON to a file | M |
| Switch profile in the app | One connection per process. Also show the cluster address in the header | M |
| Stricter confirmation per profile | Three tiers exist. Wanted: an opt-in that asks for more on a named profile | S |
| Headless `--exec` | No non-interactive mode. Every action already runs through one dispatch point | M |

## 0.5 · Editor depth

| Item | Today | Effort |
|---|---|---|
| Quickfix | A visible, editable list of workflows to step through with `]q` and `[q`, and to batch over | M |
| Marks | The jumplist exists, marks do not | S |
| Diff | Two histories aligned by group with linked scrolling, in two splits | L |
| A cursor inside a payload | Folding, yanking a path or a sub-value. Today the pane scrolls and nothing more | L |
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

## How an item gets on this list

It has to make a workflow easier to understand, navigate or operate from a terminal, and
Temporal has to expose the data it needs. A request that fails the second test goes under
*Not planned* with the reason, so it is not proposed twice.
