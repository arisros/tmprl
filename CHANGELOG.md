# Changelog

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
