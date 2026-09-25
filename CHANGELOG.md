# Changelog

## 0.1.2-rc.1 — 2026-09-25

- fix: name an update by its handler, not by its uuid
- docs: say what is built, and drop the milestone codes
- docs: write the rc.4 notes for a reader, fix the yank line

## 0.1.1 — 2026-09-21

- **Find past the pane**: `<Space>ff` asks the server when the rows a pane has loaded match
  nothing you typed, and `/` on a history reads the rest of the run in rather than stopping
  at the events already loaded.
- **Retries in progress**: an activity being retried right now shows its attempt, when it
  tries again and why the last one failed.

## 0.1.1-rc.4 — 2026-09-21

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

## 0.1.1-rc.3 — 2026-09-19

- **Timeline**: `<Space>G` draws a history the way Temporal's web UI does, each group's
  events as dots on one time axis, coloured by how it ended. Idle stretches fold to `≀`;
  `zg` unfolds them.
- **Clock times**: `<Space>T` swaps ages for clock readings in every list, in the zone set by
  `timezone` in `config.toml` (default: the machine's). Closed workflows gain a close time.
- **Failures**: `K` on a failure shows the whole chain, every `caused by` down to the root,
  its type, the SDK that raised it, whether it was retryable, and the stack traces.
- **Yank** inside tmux goes through `tmux load-buffer -w`, so it reaches the clipboard
  without extra tmux settings.

## 0.1.1-rc.2 — 2026-09-17

- `[layout] payload = "right"` in `config.toml` opens the `K` payload pane beside the
  history list instead of under it. Terminals narrower than 100 columns still stack it.

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
