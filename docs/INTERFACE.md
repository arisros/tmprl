# Interface design

> **Status: partly implemented.** The modal core, the namespace and workflow lists, the
> workflow history outline, follow mode, payload rendering, the visibility query bar, saved
> views, counts, which-key, the `:` command line, the help overlay and yank all work today. Bindings for features that do not exist yet (histories,
> splits, pickers, follow mode) are **specified here but deliberately not bound** — a key
> that opens an empty screen is worse than a key that does nothing at all. The keymap
> tables below mark which is which.
>
> Run `?` in the application for the bindings that are actually live; that overlay is
> generated from the keymap, so it is never out of date, and it scrolls.

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
survives a remap — see [the command registry](ARCHITECTURE.md#4-the-command-registry--built).

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

| Key | Action | |
|---|---|---|
| `j` `k` `gg` `G` `<C-d>` `<C-u>` | move, with counts | **live** |
| `<Down>` `<Up>` | move | **live** |
| `Enter` | open the focused item — namespace → workflows → history | **live** |
| `Enter` (in Visual) | open every selected namespace as one merged list | **live** |
| `-` | **go up a level** — run → workflow → namespace → cluster | **live** |
| `<leader>-` | floating object browser | M2 |
| `<C-o>` / `<C-i>` | jumplist back / forward | M7 |
| `<leader>N` | switch namespace | M2 |
| `<leader>P` | switch connection profile | M2 |

Multi-namespace is a visual selection rather than a picker: `V j <CR>` on the namespace list
opens those namespaces as one table, merge-sorted by start time, with each row tagged by the
namespace it came from. Selection is machinery the interface already has, so this needed no
new concept.

`-` deserves a note: it is modelled on [oil.nvim](https://github.com/stevearc/oil.nvim)'s
treatment of a directory as an editable buffer. Temporal's objects form a hierarchy, and
"go up" is a more useful primitive than a breadcrumb you have to aim at.

### Finding

| Key | Action | |
|---|---|---|
| `i` | edit the visibility query; `Enter` applies, `Esc` abandons | **live** |
| `<leader>1`–`<leader>9` | saved views from `views.toml` | **live** |
| `<leader>ff` | find a workflow | M2 |
| `<leader>fg` | filter builder that compiles into the query bar | M2 |
| `<leader>fb` | open workflow buffers | M2 |
| `<leader>fl` | jump to an event or group in the current history | M2 |
| `<leader>fh` | help | M2 |
| `/` `n` `N` | search within the current view | M2 |

Saved views are bound under the **leader**, not to bare digits as this document originally
specified. A leading digit in Normal mode starts a count, and counts composing with every
motion (`7j`, `10G`) is worth more than saving one keystroke. Only views that `views.toml`
actually defines get a binding, so the which-key popup never advertises an empty slot — it
lists them by name.

### The query bar

The visibility query is always on screen and always the raw string. Anything that filters the
list writes *into* that text rather than replacing it with a structure you cannot see: a
saved view fills the bar and leaves it editable, and the filter builder planned for M2 will
do the same. This is the one piece of the web UI's design being deliberately rejected rather
than ported — a lossy abstraction over the query is what makes that filter widget
frustrating to use.

Pickers are bottom-docked with a preview pane, following Telescope's `ivy` layout.

### Windows

All of this arrives with the window tree in M2; none of it is bound today.

| Key | Action | |
|---|---|---|
| `<leader>sv` / `<leader>sh` | split vertical / horizontal | M2 |
| `<leader>se` / `<leader>sx` | equalise / close | M2 |
| `<C-w>hjkl` | move focus | M2 |
| `<leader>r{h,j,k,l}` | resize by 10 | M2 |
| `<leader>t{o,x,n,p}` | tab open / close / next / previous | M2 |

Two workflow-detail views in a split, with linked scrolling, *is* the diff feature. There is
no separate diff screen.

### Reading a history

Events are folded into groups: an activity that was scheduled, started and completed is one
row, not three, and it carries its own retry count and failure message. Workflow tasks — the
worker polling — are the majority of events in a real history and almost never what you came
to read, so they are folded away until `zp`.

The fold bindings are vim's `z` family deliberately, so the which-key popup on `z` reads the
way vim's does. `zp` is not a vim binding, but it sits in the same namespace as the folds it
resembles. `]f` / `[f` follow vim-unimpaired's bracket-motion convention.

`F` tails a running workflow, the way `tail -f` does. The statusline carries a **FOLLOW**
badge while it is on, because a view that rewrites itself under you needs to say so — a
screen that changes on its own otherwise reads as a glitch. Following stops on `F`, on leaving
the history, and by itself when the workflow closes, which it reports rather than leaving the
badge up over a view that has quietly stopped moving. Following a workflow that has *already*
closed is refused with a message instead of polling for events that can never arrive.

### Inspecting

| Key | Action | |
|---|---|---|
| `y` / `Y` | yank field / whole record as JSON | **live** |
| `za` | fold a history group open or shut | **live** |
| `zR` / `zM` | expand / collapse every group | **live** |
| `zp` | show or hide the workflow-task plumbing | **live** |
| `]f` / `[f` | jump to the next / previous failure | **live** |
| `F` | follow — tail a running workflow | **live** |
| `<leader>cs` | call stack (`__stack_trace` query) | M2 |
| `<leader>cq` | send a query to the workflow | M2 |
| `!` | pipe selection through `jq` | M2 |
| `K` | show the payloads under the cursor | **live** |
| `<C-e>` / `<C-y>` | scroll the payload pane | **live** |
| `<leader>e` | open the payload in `$EDITOR` | M2 |

### Acting

| Key | Action | |
|---|---|---|
| `v` / `V` | select rows | **live** |
| `:` | command palette | **live** |
| `?` | help overlay, scrollable with `j` / `k` | **live** |
| `<Esc>` | cancel pending input / close overlay | **live** |
| `R` | reload from the server | **live** |
| `<leader>q` / `<C-c>` | quit | **live** |
| `<C-q>` | send selection to the quickfix list | M5 |
| `<leader>xx` | problem list — failed and task-failure workflows | M2 |
| `<leader>xQ` | open the quickfix list | M5 |

The quickfix list is how batch operations are staged. Select rows, `<C-q>` to stage them,
then run an operation over the staged set. Staging is a visible, editable list rather than an
invisible selection, because *"which four thousand workflows am I about to terminate?"*
should be a question with an answer on screen.

Every binding is a lookup into the command registry, so all of it **is** remappable through
`~/.config/tmprl/keys.toml`:

```toml
[normal]
"ZZ"    = "app.quit"
"<C-r>" = "app.refresh"

[insert]
"jj" = "mode.normal"
```

A chord bound here replaces the built-in binding for the same chord and mode; everything else
is left alone. The loader is strict on purpose: an unknown command id, an unparseable chord
or an unknown mode is reported in the statusline at startup rather than skipped. A binding
that is silently dropped is a key that does nothing, with no way to find out why.

## Destructive actions

Every mutation routes through one confirmation modal, which displays **the equivalent
`temporal` CLI command**. That teaches the CLI, makes the action auditable at a glance, and
gives an escape hatch to anyone who would rather not trust a TUI with it.

Batch operations additionally show a `CountWorkflowExecutions` dry run and require typing the
affected count. Every mutation appends to `~/.local/state/tmprl/audit.jsonl`.

## Theming

Colours will come from `~/.config/tmprl/theme.toml`; that loader is not written yet, and the
palette is currently compiled in. What *is* live is the part that matters most: status is
encoded in shape as well as hue. Every execution status has its own glyph — `●` running,
`✓` completed, `✗` failed, `■` terminated, `⊘` cancelled, `◔` timed out, `↻` continued-as-new,
`‖` paused — used identically in the table and in the header tallies. Colour only reinforces
it, so the workflow list stays readable in a 16-colour terminal and for a colour-blind
reader.

## Configuration files

| File | Holds |
|---|---|
| `~/.config/tmprl/config.toml` | codec server endpoint, refresh intervals, defaults — *planned* |
| `~/.config/tmprl/keys.toml` | key chord → command id — **live** |
| `~/.config/tmprl/theme.toml` | colours — *planned* |
| `~/.config/tmprl/views.toml` | saved visibility queries — **live** |
| `~/.local/state/tmprl/audit.jsonl` | every mutation performed — *planned* |

The directory is `$TMPRL_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tmprl`, else `~/.config/tmprl`.
A `views.toml` looks like:

```toml
[[view]]
key   = "1"
name  = "Running now"
query = "ExecutionStatus = 'Running'"

[[view]]
key   = "2"
name  = "Broken"
query = "ExecutionStatus = 'Failed' OR ExecutionStatus = 'Terminated'"
```

Connection settings are deliberately *not* in this list — those come from
`~/.config/temporalio/temporal.toml`, the same file the `temporal` CLI uses.
