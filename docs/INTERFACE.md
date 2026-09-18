# Interface design

> **Status: partly implemented.** The modal core, the namespace and workflow lists, the
> workflow history outline, follow mode, payload rendering and piping, the visibility query
> bar, saved views, search, the pickers, the jumplist, counts, which-key, the `:` command
> line, the help overlay and yank all work today. Bindings for features that do not exist
> yet (marks, macros, `.`, and the M5 quickfix list) are **specified here but deliberately
> not bound**, a key
> that opens an empty screen is worse than a key that does nothing at all. The keymap
> tables below mark which is which.
>
> Run `?` in the application for the bindings that are actually live; that overlay is
> generated from the keymap, so it is never out of date, and it scrolls.

---

## The model

`tmprl` is a modal application. It borrows Neovim's model rather than inventing one, on the
grounds that the people who want a Temporal client in their terminal are overwhelmingly
people who already have those motions in their fingers, and that a second, nearly-identical
set of bindings to learn is a cost with no return.

Modes: **Normal**, **Insert** (query bar, payload editors, forms), **Visual** and
**V-Line** (selecting rows for batch operations), **Command** (`:`), and
**Operator-Pending**.

Consequences that follow from taking the model seriously rather than decoratively:

- **Counts work.** `7j`, `10G`, `3<C-d>`. Lists render a hybrid relative/absolute gutter, so
  a count is something you can read off the screen rather than estimate.
- **`jk` leaves Insert**: everywhere, in addition to `Esc`.
- **Yank goes to the system clipboard by default**: `clipboard=unnamedplus` semantics, not a
  private register nobody can paste out of.
- **Marks and a jumplist** (`m{a-z}`, `` `{a-z} ``, `<C-o>`, `<C-i>`) that work *across*
  workflows and namespaces, not just within one view.
- **Macros** (`q{reg}`, `@{reg}`, `@@`) and `.` to repeat.

Macros record command ids, not keystrokes. A recorded macro is therefore readable text that
survives a remap. See [the command registry](ARCHITECTURE.md#4-the-command-registry--built).

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
either fail outright or copy into a clipboard on the *server*, which helps nobody, silently.

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
| `Enter` | open the focused item, namespace → workflows → history | **live** |
| `Enter` (in Visual) | open every selected namespace as one merged list | **live** |
| `-` | **go up a level**, run → workflow → namespace → cluster | **live** |
| `<leader>-` | floating object browser | M2 |
| `<C-o>` / `<C-i>` (or `<Tab>`) | jumplist back / forward | **live** |
| `<leader>N` | switch namespace | **live** |
| `<leader>P` | switch connection profile | M2 |

Multi-namespace is a visual selection rather than a picker: `V j <CR>` on the namespace list
opens those namespaces as one table, merge-sorted by start time, with each row tagged by the
namespace it came from. Selection is machinery the interface already has, so this needed no
new concept.

The same selection drives batch mutations. `V` a range of workflows or schedules and any
`<leader>m` action applies to every selected row rather than the one under the cursor, one
request per row, in order. A destructive batch asks for the count to be typed before it
runs, because one keypress is too cheap to end a dozen workflows; deleting still asks for
`delete`, since that destroys the histories themselves.

This acts on the rows you picked, not on a query. Temporal's server-side batch API takes a
visibility query and acts on whatever matches at the time it runs, which can be more than
was counted when it was confirmed. That is a separate command, not this one.

`<Tab>` is bound alongside `<C-i>` because on a terminal they are the same key: Ctrl+I is
byte `0x09`, which is exactly what Tab sends, so a binding on `<C-i>` alone never fires
outside the few terminals speaking the Kitty keyboard protocol. Terminal vim has the same
collision and resolves it the same way.

The jumplist records the moves vim would call jumps, changing screen, `gg` and `G`, a
search that found something, taking a row or a filter from a picker, and not the ones it
would not: `j`, `k` and `n` are walking, not jumping. Switching pane with `<leader>fb` is
not a jump either, since every entry describes a position *within* a pane and changing which
pane is focused moves no cursor. A move that gets refused records nothing, because recording
one discards the forward entries and a keystroke that did nothing should not cost you
`<C-i>`. A jumplist that recorded every line is a scroll history, and `<C-o>`
stops being worth pressing. A position is stored as a run id rather than a row index,
because every index a pane has is into a list a refresh can replace; coming back re-fetches,
so a jump shows what is there now rather than reinstating a stale table.

`-` deserves a note: it is modelled on [oil.nvim](https://github.com/stevearc/oil.nvim)'s
treatment of a directory as an editable buffer. Temporal's objects form a hierarchy, and
"go up" is a more useful primitive than a breadcrumb you have to aim at.

### Finding

| Key | Action | |
|---|---|---|
| `i` | edit the visibility query; `Enter` applies, `Esc` abandons | **live** |
| `<leader>1`–`<leader>9` | saved views from `views.toml` | **live** |
| `<leader>ff` | find a workflow | **live** |
| `<leader>fg` | filter builder that compiles into the query bar | **live** |
| `<leader>fb` | open panes, vim's `:ls` | **live** |
| `<leader>fl` | jump to an event or group in the current history | **live** |
| `<leader>fh` | find a command | **live** |
| `/` `n` `N` | search within the current view, with `smartcase` | **live** |

`/` searches the rows the pane has already loaded; it never refetches. That is the whole
difference from the query bar, which asks the server to change what exists. The two compose:
narrow with a query, then find within the result.

Matching is vim's `smartcase`, an all-lowercase pattern ignores case, one with any uppercase
does not, which is the right default for data where types are camel case. `n` and `N` wrap,
and say so when they do; a `n` that stops silently at the last match reads as a broken key.

What a row matches on is wider than what fits in its columns: a run id is searchable but not
rendered. So a row can match with nothing on it lit up, which is why the statusline reports a
count rather than leaving you to find the highlight.

Saved views are bound under the **leader**, not to bare digits as this document originally
specified. A leading digit in Normal mode starts a count, and counts composing with every
motion (`7j`, `10G`) is worth more than saving one keystroke. Only views that `views.toml`
actually defines get a binding, so the which-key popup never advertises an empty slot, it
lists them by name.

### The query bar

The visibility query is always on screen and always the raw string. Anything that filters the
list writes *into* that text rather than replacing it with a structure you cannot see: a
saved view fills the bar and leaves it editable, and the filter builder planned for M2 will
do the same. This is the one piece of the web UI's design being deliberately rejected rather
than ported, a lossy abstraction over the query is what makes that filter widget
frustrating to use.

Pickers are bottom-docked with a preview pane, following Telescope's `ivy` layout. Docked
rather than centred so the list you opened it from stays on screen: a picker that covers its
own context makes you close it to remember what you were looking at. `<C-n>` / `<C-p>` move,
`Enter` takes, `Esc` and backspace-on-empty close.

Matching is fuzzy and scored, not a bare subsequence test. `ord` is a subsequence of most
Temporal ids, so an unranked list buries the hit you want. Word starts, contiguous runs and
early matches all score, and the characters that matched are highlighted in each row, so the
ranking can be read rather than guessed at. The same matcher backs `:` completion.

`<leader>fg` builds its clauses from the **rows already loaded**: the workflow types and task
queues it offers are the ones this namespace actually has. Accepting one `AND`s it onto the
query bar and leaves the text editable, so filters compose by visiting the picker twice.

### Windows

| Key | Action | |
|---|---|---|
| `<leader>sv` / `<leader>sh` | split side by side / above and below | **live** |
| `<leader>se` / `<leader>sx` | equalise / close | **live** |
| `<C-w>hjkl` | move focus | **live** |
| `<leader>r{h,j,k,l}` | resize by 10 | **live** |
| `<leader>t{o,x,n,p}` | tab open / close / next / previous | **live** |

Two workflow-detail views in a split *is* the diff feature. There is no separate diff screen.

A split forks where you are, not what you have loaded: the new pane opens on the same screen,
scope and query and fetches its own copy. Splitting is almost always "show me this again so I
can take one of them somewhere else". Linked scrolling, which is what turns two histories into
a readable diff, is not built yet.

### Reading a history

Events are folded into groups: an activity that was scheduled, started and completed is one
row, not three, and it carries its own retry count and failure message. Workflow tasks, the
worker polling, are the majority of events in a real history and almost never what you came
to read, so they are folded away until `zp`.

The fold bindings are vim's `z` family deliberately, so the which-key popup on `z` reads the
way vim's does. `zp` is not a vim binding, but it sits in the same namespace as the folds it
resembles. `]f` / `[f` follow vim-unimpaired's bracket-motion convention.

`!` filters what the cursor is on through an external command, the way vim's `!` filters
lines. The prompt opens pre-filled with `jq .`, because that is what it is for and an empty
prompt means retyping the same three characters every time.

A row usually carries more than one payload, an activity has both an `input` and a `result`,
so "pipe the payload" would be ambiguous. What is piped is a JSON object keyed by label, which
makes the obvious expressions work: `jq .` shows everything, `jq .result` picks one,
`jq .input[1]` picks an argument. A payload that is encrypted or binary is left out and the
statusline says which, because piping ciphertext into `jq` produces a parse error that
explains nothing.

The command runs through a shell, so `!jq .result | head -20` works. Its output replaces the
payload pane; a failure shows the command's own stderr, since when a `jq` expression is wrong
jq's message is the entire diagnosis.

`F` tails a running workflow, the way `tail -f` does. The statusline carries a **FOLLOW**
badge while it is on, because a view that rewrites itself under you needs to say so, a
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
| `F` | follow, tail a running workflow | **live** |
| `<leader>cs` | call stack (`__stack_trace` query) | M2 |
| `<leader>cq` | send a query to the workflow | M2 |
| `!` | pipe the focused payloads through a command | **live** |
| `K` | show the payloads under the cursor | **live** |
| `<C-e>` / `<C-y>` | scroll the payload pane | **live** |
| `<leader>e` | open the payloads in `$EDITOR`, read-only | **live** |

### Acting

| Key | Action | |
|---|---|---|
| `v` / `V` | select rows | **live** |
| `:` | command palette | **live** |
| `?` | help overlay, scrollable with `j` / `k` | **live** |
| `<Esc>` | cancel pending input / close overlay | **live** |
| `R` | reload from the server | **live** |
| `<leader>T` | times as a clock reading or as an age | **live** |
| `<leader>q` / `<C-c>` | quit | **live** |
| `<C-q>` | send selection to the quickfix list | M5 |
| `<leader>mc` | cancel this workflow, or the selection | **live** |
| `<leader>mt` | terminate this workflow, or the selection | **live** |
| `<leader>ms` | signal this workflow, or the selection | **live** |
| `<leader>md` | delete this workflow, or the selection | **live** |
| `<leader>mr` | reset to the event under the cursor | **live** |
| `<leader>mp` | pause or resume a schedule | **live** |
| `<leader>mg` | run a schedule now | **live** |
| `<leader>mD` | delete a schedule | **live** |
| `<leader>mb` | backfill a schedule over a past window | **live** |
| `<leader>mn` | create a schedule, in a form | **live** |
| `<leader>mu` | send an update and wait for its outcome | **live** |
| `<leader>xx` | problem list, failed / timed out / terminated | **live** |
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

**Live for one workflow at a time**: cancel, terminate, signal, delete, reset and update,
under `<leader>m`, clear of bare `m`, which marks reserve. Batch operations are M5.

`<leader>mr` resets to the event under the cursor. Temporal only resets to a *completed
workflow task*, and those are the rows the outline folds away, so the target resolves backwards
to the nearest valid point. The confirmation shows the event id it landed on, so the move is
visible rather than silent.

Every mutation routes through one confirmation modal, which displays **the equivalent
`temporal` CLI command**. That teaches the CLI, makes the action auditable at a glance, and
gives an escape hatch to anyone who would rather not trust a TUI with it:

```
┌ confirm, terminate ────────────────────────────────────────────────┐
│  Terminate kill-me                                                  │
│  in default                                                         │
│                                                                     │
│  the equivalent command:                                            │
│    temporal workflow terminate --namespace default --workflow-id    │
│    kill-me --run-id 01a06850-20c6-755b-8f02-d9000b16e8cf --reason   │
│    'terminated from tmprl'                                          │
│                                                                     │
│  ⏎ to confirm   Esc to cancel                                       │
└─────────────────────────────────────────────────────────────────────┘
```

While it is up it owns every key, so nothing bound elsewhere can fire underneath it, and `Esc`
is always a way out. **Delete asks for more**: it destroys the history itself, not just the
run, so it wants the word `delete` typed.

Batch operations will additionally show a `CountWorkflowExecutions` dry run and require typing
the affected count. Every mutation appends to `~/.local/state/tmprl/audit.jsonl`, failures
included, the question that log answers is what was *attempted*.

## Theming

Colours will come from `~/.config/tmprl/theme.toml`; that loader is not written yet, and the
palette is currently compiled in. What *is* live is the part that matters most: status is
encoded in shape as well as hue. Every execution status has its own glyph, `●` running,
`✓` completed, `✗` failed, `■` terminated, `⊘` cancelled, `◔` timed out, `↻` continued-as-new,
`‖` paused, used identically in the table and in the header tallies. Colour only reinforces
it, so the workflow list stays readable in a 16-colour terminal and for a colour-blind
reader.

## Configuration files

| File | Holds |
|---|---|
| `~/.config/tmprl/config.toml` | codec server endpoint and payload pane position and display timezone, **live**; refresh intervals and defaults *planned* |
| `~/.config/tmprl/keys.toml` | key chord → command id, **live** |
| `~/.config/tmprl/theme.toml` | colours, *planned* |
| `~/.config/tmprl/views.toml` | saved visibility queries, **live** |
| `~/.local/state/tmprl/audit.jsonl` | every mutation attempted, **live** |

The directory is `$TMPRL_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tmprl`, else `~/.config/tmprl`.
A `config.toml` points at a codec server, if the cluster uses one:

```toml
timezone = "Asia/Jakarta"   # optional; default: the machine's own zone

[codec]
endpoint = "http://localhost:8081"
auth     = "Bearer …"   # optional; sent verbatim as Authorization
```

Every time the server reports is UTC epoch millis, and every time on screen is that value
rendered in this zone. Lists carry an age by default, `5m`; `<leader>T` swaps every list on
screen to a clock reading, `09-17 14:03`, which is the form you quote in a ticket or line up
against a service log. The workflow list spends the extra width on two columns then, start
and close, because "when did it finish" is the other half of the question. A zone the tz
database does not know is refused when `config.toml` is read, rather than quietly becoming
UTC and misdating every row by hours.

Encrypted payloads are decoded lazily, only what the pane is showing, never a whole history,
and cached, so scrolling back over a row costs nothing. Without an endpoint the badge says
where to set one.

`K` opens the payload pane under the list. To have it beside the list instead:

```toml
[layout]
payload = "right"       # "bottom" is the default
```

Either way the list stays on screen and `j` / `k` move the row the pane shows. Below 100
columns `right` stacks anyway, since neither half would be readable.
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

Connection settings are deliberately *not* in this list, those come from
`~/.config/temporalio/temporal.toml`, the same file the `temporal` CLI uses.
