# Changelog

## Unreleased

- **Yank**: a yank over the limit is written to a file instead of being refused, and the
  status line says where: `510853 bytes is over the yank limit (65536), written to
  /tmp/tmprl-…/yank.json`. The file is readable by you only and stays until you remove it.
  `[yank] max_bytes` in `config.toml` raises the limit for a terminal that takes more.
- **Dashboard**, `gd`: one screen of panels over the namespaces in scope, with workflows by
  status, failures of the last day, the workflow types failing most, the task queues running
  workflows are on, and schedules. `Enter` on an item opens what it stands for, `]p` and `[p`
  step between panels, and a dashboard left in a split keeps receiving its own data.
- **The dashboard adapts** when there is no `dashboard.toml`: a panel with nothing in it
  gives up its room, a namespace with no failures shows what is running instead, and
  schedules appear as the paused ones and the ones about to run. `R` lets emptied panels go.
- **The dashboard refreshes itself**, every 30 seconds by default: `[refresh] dashboard` in
  `config.toml` takes a duration or `"off"`. A request that fails keeps the panel's last
  answer, marked `stale`, is retried less often while it keeps failing, and is reported once.
- **Task queue health** on the dashboard: each queue with running workflows shows its
  backlog, how long the oldest task has waited, and how many workers are polling. A backlog
  nothing is polling stands out as an error.
- **`dashboard.toml`** lays the dashboard out: rows of panels, each with a kind, a query, a
  time window and a share of the screen. A key it does not know sets the file aside with a
  message, and the built-in layout is used. `--config-path` lists the file.
- **Dashboard time windows**: `since` now works on a `counts` panel, so a status line can
  cover the last day instead of everything. `older = "3d"` is the other bound, for workflows
  that started more than three days ago and are still running. `by = "close"` measures both
  from when a workflow closed, so "failures, last 24h" includes one that started last week
  and failed ten minutes ago.
- **Dashboard tallies are counted**: on a namespace with more workflows than one page, the
  types and task queues panels used to show how many of the first 50 rows each name had.
  Each name is now counted on the server, so the number is the real one. A number still
  waiting for its count is drawn `~26`, and the panel says `names from 50` because a type the
  page missed is still missing.
- **Dashboard failures say why**: a failed, terminated or cancelled workflow in a dashboard
  list shows its reason beside the id, the root of the failure chain or the reason it was
  ended with, read once from the workflow's closing event. A type that every row shares is
  no longer repeated down the list. With a failure converter that encodes failures the
  reason is whatever the server holds, since it is not sent through the codec.
- **Dashboard rows fit what they show**: a row whose panels hold two lines no longer takes
  a third of the screen. It keeps what it needs and the rest goes to the rows with more to
  list. A row given a fixed height with `lines` is left as it is.
- **Dashboard histograms**: a `histogram` panel draws how many workflows started or closed
  in each stretch of its window as columns, so a spike in failures shows as one.
  `since = "24h"` with `bucket = "1h"` is a column an hour, and without `bucket` the step
  is chosen to fit. `Enter` on a column opens the workflows in that stretch. With no
  `dashboard.toml`, a namespace where something failed today gets a "Failures per hour"
  chart under the status line.
- **Dashboard navigation**: `h` `j` `k` `l` and the four arrows now move the way they
  point. `j` at the foot of a list goes to the panel under it, not to the one beside it,
  `h` and `l` cross between panels and step along a chart or the status line. `h`, `l`,
  `<Left>` and `<Right>` are new bindings, `motion.left` and `motion.right`.
- **`--dashboard`** opens tmprl straight onto the dashboard, in the profile's namespace or
  the one `-n` names.

## 0.1.5 — 2026-10-03

- No user-visible changes.

## 0.1.4 — 2026-10-03

- **Colours**: `NO_COLOR` is honoured, a terminal that does not announce truecolor through
  `COLORTERM` gets its own sixteen named colours, which follow a light or dark terminal theme,
  and `theme.toml` repaints individual slots with `#rrggbb` or a colour name. A truecolor
  terminal that does not export `COLORTERM`, usual over SSH, now gets sixteen colours.
- **Errors**: a failed request says which call failed and why in words, `ListWorkflowExecutions
  failed (unavailable): transport error`, and an empty list whose load failed adds what to
  try. A key scoped to one namespace is now recognised by the server's `PermissionDenied`
  rather than by the wording of its refusal.
- **`:messages`**, also `<leader>xm`: every note the status line has shown this session, with
  the gRPC code and the call behind each failure. The note line still clears on the next key.
- **Command line**: `-n` opens a namespace's workflows, `-q` opens the list with a query
  applied, `-w` opens the history of a workflow id or run id, `--address` overrides the
  profile's server while keeping its TLS and API key, and `--readonly` refuses every mutation
  for the run. A long option also takes `--option=value`.
- An option that needs a value no longer takes a following argument that starts with `-`, so
  `tmprl -n --readonly` is an error rather than a namespace called `--readonly`.
- **Fixed**: `cargo install tmprl` no longer needs `protoc`. The protos are compiled by a
  vendored pure Rust compiler, so a Rust toolchain is the only build requirement.
- **Fixed**: the create-schedule form opened on a read-only profile and refused only after
  it was filled in. It now refuses before it opens.

## 0.1.3 — 2026-09-29

- fix: drop the protoc build requirement
- feat: keep the grpc code and add :messages
- feat: add theme.toml, NO_COLOR and a 16-colour palette
- feat: add namespace, query, workflow, address and readonly flags
- fix: refuse the create form on a read-only profile

## 0.1.2 — 2026-09-28

- **Updates** in a history are named by their handler instead of their
  `protocol_instance_id`, so the list no longer reads as a column of uuids and `/` finds an
  update by name. The id stays as an `updateId` field, to correlate with the caller.
- **Query completion**: the query bar completes the clause being typed and `<Tab>` takes it.
  It completes a whole clause such as `ExecutionStatus = 'Running'`, respects quoting so
  `'send and forget'` stays one clause, and offers nothing before you type or after a clause
  ends in a space.
- **Filter picker**: offers the search attributes registered on the cluster, custom ones
  included, and time windows (the last hour, 6 hours, 24 hours, 7 days), each compiled to
  the instant it means because the visibility grammar has no `now()`.
- **Scrollbars**: a list or pane that overflows shows a thumb, so you can see how much is
  below. A pane that fits is unchanged.
- **Search inside payloads**: `/` on a history matches the values in readable inputs and
  results, not only a row's name and fields. `/8812` finds the value wherever it sits;
  `/customer.id=8812` finds it only as a customer id. The statusline names the path that
  matched, and `K` highlights it and scrolls to it. `/` never calls the codec, so a miss
  says how many encrypted payloads it could not see.

## 0.1.1 — 2026-09-21

- `[layout] payload = "right"` in `config.toml` opens the `K` payload pane beside the
  history list instead of under it. Terminals narrower than 100 columns still stack it.
- **Timeline**: `<Space>G` draws a history the way Temporal's web UI does, each group's
  events as dots on one time axis, coloured by how it ended. Idle stretches fold to `≀`;
  `zg` unfolds them.
- **Clock times**: `<Space>T` swaps ages for clock readings in every list, in the zone set by
  `timezone` in `config.toml` (default: the machine's). Closed workflows gain a close time.
- **Failures**: `K` on a failure shows the whole chain, every `caused by` down to the root,
  its type, the SDK that raised it, whether it was retryable, and the stack traces.
- **Yank** inside tmux goes through `tmux load-buffer -w`, so it reaches the clipboard
  without extra tmux settings.
- **Find past the pane**: `<Space>ff` sends a prompt of six characters or more to the server
  as a `WorkflowId` or a `RunId`, 300ms after typing stops, so an id pasted from a log finds
  its workflow whether or not the pane has loaded it. Matches are tagged `found by id` and
  open from the picker. `/` on a history keeps reading pages until the pattern turns up;
  `<Esc>` stops it. On a workflow list `/` stays local, the query bar is the server-side
  filter.
- **Retries in progress**: a running workflow's activity now shows `×4/10` (`∞` when the
  policy never gives up), `retry in 12s` while it backs off, and the failure that caused the
  retry. `K` adds the state, the next attempt time and the worker that ran the last one.
  Temporal writes no event for a retry, so this comes from describe: read when the history
  opens, on `R`, and every 5s under `F`.

## 0.1.0

The first release.

- **Workflows**: list, query bar, saved views (`<Space>1`–`9`), grouped counts, and runs
  across several Temporal environments from `temporal.toml` profiles.
- **Histories**: foldable event groups, `]f` / `[f` between failures, workflow tasks on
  `zp`, and `F` to follow a running workflow.
- **Payloads**: `K` shows the payloads under the cursor, decoded through a codec server
  when needed; `!` pipes them through `jq`; yank over OSC 52.
- **Search**: `/`, `<Space>ff` pickers and a jumplist (`<C-o>` / `<C-i>`).
- **Mutations**: cancel, terminate, signal, delete, reset and update, one row or a `V`
  selection. Each shows the equivalent `temporal` command first and is written to an audit log;
  a `readonly` profile refuses them.
- **Schedules**: list, pause, trigger, delete, backfill and create.
- **Windows**: splits and tabs, a `?` help overlay and a `:` command line generated from the
  command registry, and keys rebindable in `keys.toml`.
