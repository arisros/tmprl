# Interface design

> **Status: none of this is implemented.** This document specifies the intended interface so
> the shape is settled before it is built. No key described here currently does anything.
> See [ARCHITECTURE.md](ARCHITECTURE.md) for the structure underneath it.

---

## The model

`tmprl` is a modal application. It borrows Neovim's model rather than inventing one, on the
grounds that the people who want a Temporal client in their terminal are overwhelmingly
people who already have those motions in their fingers — and that a second, nearly-identical
set of bindings to learn is a cost with no return.

Modes: **Normal**, **Insert** (query bar, payload editors, forms), **Visual** and
**V-Line** (selecting rows for batch operations), **Command** (`:`), and
**Operator-Pending**.

Consequences that follow from taking the model seriously rather than decoratively:

- **Counts work.** `7j`, `10G`, `3<C-d>`. Lists render a hybrid relative/absolute gutter, so
  a count is something you can read off the screen rather than estimate.
- **`jk` leaves Insert**, everywhere, in addition to `Esc`.
- **Yank goes to the system clipboard by default** — `clipboard=unnamedplus` semantics, not a
  private register nobody can paste out of.
- **Marks and a jumplist** (`m{a-z}`, `` `{a-z} ``, `<C-o>`, `<C-i>`) that work *across*
  workflows and namespaces, not just within one view.
- **Macros** (`q{reg}`, `@{reg}`, `@@`) and `.` to repeat.

Macros record command ids, not keystrokes. A recorded macro is therefore readable text that
survives a remap — see [the command registry](ARCHITECTURE.md#4-the-command-registry--planned).

## Two constraints imposed by tmux

These are not stylistic choices. They are the reason two obvious bindings are unavailable.

### `C-h` / `C-j` / `C-k` / `C-l` cannot be used

The widely-used [vim-tmux-navigator](https://github.com/christoomey/vim-tmux-navigator) setup
binds all four **prefix-less** to `select-pane`. tmux consumes them before any application in
the pane ever sees them. A TUI that binds them appears broken to a large fraction of its
likely users, in a way that looks like the TUI's fault.

So:

| Purpose | Binding |
|---|---|
| Move between tmprl panes | `<C-w>h` `<C-w>j` `<C-w>k` `<C-w>l` |
| Move within a picker | `<C-n>` / `<C-p>` |

`<C-w>` is also what vim itself uses for window motions, so this is the more consistent
choice regardless.

### Yank must be OSC 52, not `xclip`

The common deployment is SSH into a remote host, often headless. There, `xclip` and `xsel`
either fail outright or copy into a clipboard on the *server* — which helps nobody, silently.

[OSC 52](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h4-Operating-System-Commands)
transmits the copied text back over the terminal connection to the machine the human is
actually sitting at. tmprl emits OSC 52 and falls back to a local clipboard only when it can
determine there is a usable one.

For this to work through tmux, tmux needs `set -g set-clipboard on`. Many terminfo entries
also lack the `Ms` capability, without which tmux refuses to emit OSC 52 at all:

```tmux
set -g set-clipboard on
set -ga terminal-overrides ',*:Ms=\E]52;%p1%s;%p2%s\7'
```

## Keymap

Leader is `Space`. A which-key-style popup appears after 500ms on an incomplete prefix.

### Navigation

| Key | Action |
|---|---|
| `j` `k` `gg` `G` `<C-d>` `<C-u>` | move, with counts |
| `Enter` | open the focused item |
| `-` | **go up a level** — run → workflow → namespace → cluster |
| `<leader>-` | floating object browser |
| `<C-o>` / `<C-i>` | jumplist back / forward |
| `<leader>N` | switch namespace |
| `<leader>P` | switch connection profile |

`-` deserves a note: it is modelled on [oil.nvim](https://github.com/stevearc/oil.nvim)'s
treatment of a directory as an editable buffer. Temporal's objects form a hierarchy, and
"go up" is a more useful primitive than a breadcrumb you have to aim at.

### Finding

| Key | Action |
|---|---|
| `<leader>ff` | find a workflow |
| `<leader>fg` | query across executions (visibility query) |
| `<leader>fb` | open workflow buffers |
| `<leader>fl` | jump to an event or group in the current history |
| `<leader>fh` | help |
| `/` `n` `N` | search within the current view |
| `1`–`9` | saved views |

Pickers are bottom-docked with a preview pane, following Telescope's `ivy` layout.

### Windows

| Key | Action |
|---|---|
| `<leader>sv` / `<leader>sh` | split vertical / horizontal |
| `<leader>se` / `<leader>sx` | equalise / close |
| `<C-w>hjkl` | move focus |
| `<leader>r{h,j,k,l}` | resize by 10 |
| `<leader>t{o,x,n,p}` | tab open / close / next / previous |

Two workflow-detail views in a split, with linked scrolling, *is* the diff feature. There is
no separate diff screen.

### Inspecting

| Key | Action |
|---|---|
| `F` | follow — tail a running workflow |
| `<leader>cs` | call stack (`__stack_trace` query) |
| `<leader>cq` | send a query to the workflow |
| `y` / `Y` | yank field / whole record as JSON |
| `!` | pipe selection through `jq` |
| `<leader>e` | open the payload in `$EDITOR` |

### Acting

| Key | Action |
|---|---|
| `v` / `V` | select rows |
| `<C-q>` | send selection to the quickfix list |
| `<leader>xx` | problem list — failed and task-failure workflows |
| `<leader>xQ` | open the quickfix list |
| `:` | command palette |

The quickfix list is how batch operations are staged. Select rows, `<C-q>` to stage them,
then run an operation over the staged set. Staging is a visible, editable list rather than an
invisible selection, because *"which four thousand workflows am I about to terminate?"*
should be a question with an answer on screen.

Every binding above is a lookup into the command registry, so all of it is remappable through
`~/.config/tmprl/keys.toml`.

## Destructive actions

Every mutation routes through one confirmation modal, which displays **the equivalent
`temporal` CLI command**. That teaches the CLI, makes the action auditable at a glance, and
gives an escape hatch to anyone who would rather not trust a TUI with it.

Batch operations additionally show a `CountWorkflowExecutions` dry run and require typing the
affected count. Every mutation appends to `~/.local/state/tmprl/audit.jsonl`.

## Theming

Colours come from `~/.config/tmprl/theme.toml`. `NO_COLOR` is respected. Status is encoded in
shape as well as hue — a glyph and a position, not colour alone — so the interface remains
readable for colour-blind users and in a 16-colour terminal.

## Configuration files

| File | Holds |
|---|---|
| `~/.config/tmprl/config.toml` | codec server endpoint, refresh intervals, defaults |
| `~/.config/tmprl/keys.toml` | key chord → command id |
| `~/.config/tmprl/theme.toml` | colours |
| `~/.config/tmprl/views.toml` | saved visibility queries |
| `~/.local/state/tmprl/audit.jsonl` | every mutation performed |

Connection settings are deliberately *not* in this list — those come from
`~/.config/temporalio/temporal.toml`, the same file the `temporal` CLI uses.
