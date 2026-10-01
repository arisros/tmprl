# Changelog

## Unreleased

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
- **Fixed**: the create-schedule form opened on a read-only profile and refused only after
  it was filled in. It now refuses before it opens.

## 0.1.3 — 2026-09-29

- No user-visible changes.

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
