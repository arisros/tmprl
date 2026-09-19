# Changelog

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
