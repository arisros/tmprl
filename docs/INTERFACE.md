# Interface design

> **Status: partly implemented.** The modal core, the namespace and workflow lists, the
> workflow history outline, follow mode, payload rendering and piping, the visibility query
> bar, saved views, search, the pickers, the jumplist, counts, which-key, the `:` command
> line, the help overlay and yank all work today. Bindings for features that do not exist
> yet (marks, macros, `.`, and the quickfix list) are **specified here but deliberately
> not bound**, a key
> that opens an empty screen is worse than a key that does nothing at all. The keymap
> tables below mark which is which, and [ROADMAP.md](ROADMAP.md) says when the rest is due.
>
> Run `?` in the application for the bindings that are actually live; that overlay is
> generated from the keymap, so it is never out of date, and it scrolls.

---

## The model

`tmprl` is a modal application. It borrows Neovim's model rather than inventing one, on the
grounds that the people who want a Temporal client in their terminal are overwhelmingly
people who already have those motions in their fingers, and that a second, nearly-identical
set of bindings to learn is a cost with no return.

Modes: **Normal**, **Insert** (the query bar), **Visual** and **V-Line** (selecting rows for
batch operations, and the same thing today, since a row is the only unit there is to select),
and **Command** (`:`). There is no Operator-Pending mode: `y` is a command, not an operator.

Consequences that follow from taking the model seriously rather than decoratively:

- **Counts work** on `j` and `k`: `7j`. Lists render a hybrid relative/absolute gutter, so a
  count is something you can read off the screen rather than estimate. `G` and `<C-d>` do not
  take one yet, so `10G` goes to the last row.
- **`jk` leaves Insert**: everywhere, in addition to `Esc`.
- **Yank goes to the system clipboard by default**: `clipboard=unnamedplus` semantics, not a
  private register nobody can paste out of.
- **A jumplist** (`<C-o>`, `<C-i>`) that works *across* workflows and namespaces, not just
  within one view. **Marks** (`m{a-z}`, `` `{a-z} ``) are planned on the same footing.
- **Macros** (`q{reg}`, `@{reg}`, `@@`) and `.` to repeat are planned.

Macros will record command ids, not keystrokes. A recorded macro is therefore readable text
that survives a remap. See [the command registry](ARCHITECTURE.md#4-the-command-registry--built).

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
actually sitting at. tmprl emits OSC 52 and never falls back to a local clipboard tool.

Inside tmux, tmprl hands the text to `tmux load-buffer -w` instead. tmux ignores OSC 52 from
applications unless `set-clipboard` is `on`, and the default is `external`; `-w` makes tmux
emit the sequence to the client terminal itself, so no tmux option is needed. tmux still
needs the `Ms` capability for the client terminal, which many terminfo entries lack:

```tmux
set -ga terminal-overrides ',*:Ms=\E]52;%p1%s;%p2%s\7'
```

Terminals commonly cap OSC 52 around 100 KB and cut the rest without saying so. A yank
longer than 65536 bytes therefore does not touch the clipboard: it is written to
`yank.json` (`yank.txt` when it is not JSON) in a fresh directory under the temp directory,
readable by you only, and the status line and `:messages` give the path. tmprl does not
remove the file. A terminal that takes more can have the limit raised:

```toml
[yank]
max_bytes = 1048576     # 65536 is the default
```

## Keymap

Leader is `Space`. A which-key-style popup appears as soon as a prefix is incomplete.

### Navigation

| Key | Action | |
|---|---|---|
| `j` `k` `gg` `G` `<C-d>` `<C-u>` | move; `j` and `k` take a count | **live** |
| `<Down>` `<Up>` | move | **live** |
| `Enter` | open the focused item, namespace → workflows → history | **live** |
| `Enter` (in Visual) | open every selected namespace as one merged list | **live** |
| `-` | **go up a level**, run → workflow → namespace → cluster | **live** |
| `<leader>-` | floating object browser | planned |
| `<C-o>` / `<C-i>` (or `<Tab>`) | jumplist back / forward | **live** |
| `<leader>N` | switch namespace | **live** |
| `gd` | the dashboard, over this pane's namespaces | **live** |
| `]p` / `[p` | next / previous dashboard panel | **live** |
| `<leader>P` | switch connection profile | planned |

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

### The dashboard

`gd` opens one screen of panels over the namespaces the pane is scoped to: workflows by
status, failures of the last day, the workflow types failing most, the task queues running
workflows are on, and schedules. From the namespace list it takes the namespace under the
cursor, or every namespace in a Visual selection, the way `Enter` does.

The cursor runs through every panel's items as one list, so `j`, `k`, counts, `gg`, `G`,
`/` and `n` work as they do anywhere, and `]p` / `[p` step a panel at a time. `Enter` opens
what the item stands for:

| Item | `Enter` opens |
|---|---|
| a status | the workflow list, filtered to that status |
| a failed workflow | its history |
| a workflow type | the workflow list, the panel's query narrowed to that type |
| a task queue | the workflow list, running workflows on that queue |
| a schedule | the schedule list, cursor on it |
| a column of a histogram | the workflow list, the panel's query within that stretch of time |

The query a panel opens lands in the query bar like any other, and `<C-o>` comes back.
The dashboard asks again by itself, every 30 seconds unless `config.toml` says otherwise,
and the header says so: `auto 30s`. Each request is asked for again one interval after its
last answer and never while it is still out, so a slow cluster is not asked faster than it
answers. Only dashboards in the tab on screen refresh, focused or not; one in another tab
catches up when its tab is opened. `R` asks for everything at once.

Neither blanks what is on screen. A panel whose refresh failed keeps its last answer and
says how old it is in its title, `stale 2m`. A request that keeps failing is tried less and
less often, up to five minutes apart, and the note line reports it once, when it first goes
bad, not at every try. One the server refuses outright is left alone until `R`. A request
with no answer after the interval, or ten seconds if that is longer, counts as failed. A list drawn from one page of
a longer one says how many rows it saw, `of 50 sampled`. A tally of types or task queues
over such a page is not a count, so each name it found is counted on the server: until that
count arrives the number reads `~26`, and afterwards the panel says `names from 50`, since
a name the page missed is still missing. At most 24 names a dashboard are counted this way.

A workflow in a list that failed, was terminated or was cancelled says why beside its id,
when the panel is wide enough: the failure at the root of the chain, or the reason given for
ending it. It is read once from the workflow's closing event, for at most 40 workflows a
dashboard. A timeout has no reason to give. The type column is left out when every row has
the same type.

Task queues are the ones found on running workflows: Temporal has no call that lists them.
Each of the first eight is described, and its line says what that found: how many tasks are
waiting and how long the oldest has waited, `backlog 12, oldest 4m`, and how many workers
polled it lately, `2 pollers`. A backlog with `no pollers` is drawn as an error, because
work is waiting and nothing is taking it. The numbers cover workflow and activity tasks
together and are the server's approximations. A server too old to report a backlog shows
pollers only.
Nothing on the dashboard is a target for `<leader>m`; open the workflow first.

On a terminal too small for every panel, the ones around the cursor are drawn and the rest
are counted, `+3 hidden`. Moving the cursor scrolls the dashboard to them.

Without a `dashboard.toml` the panels follow what the namespace shows. Everything is
drawn while nothing is known yet, and a panel that turns out to have nothing in it gives up
its room:

| The namespace has | The dashboard shows |
|---|---|
| failures in the last day | "Recent failures", and "Failing types" once two or more types are failing |
| no failures, but running workflows | "Running", in the room the failures would have had |
| running workflows | "Task queues" |
| paused schedules | "Paused schedules" |
| schedules with a next run | "Upcoming schedules", soonest first |
| schedules that are neither | "Schedules" |
| nothing at all | every panel, each saying it is empty |

A panel that is still loading, or whose request failed, stays, so the screen says what it
is waiting for and what went wrong. A panel that had items and has lost them also stays,
empty, until `R` or until the dashboard is opened again: a layout that rearranges itself
while it is being read is worse than an empty box.

A `dashboard.toml`, described under [Configuration files](#configuration-files), replaces
all of this with the panels it names, and those are never rearranged. `tmprl --dashboard`
opens straight onto the dashboard.

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

On a history that includes the payloads, flattened one leaf per `path=value`, so `/8812` finds
the value and `/customer.id=8812` finds it only where it is a customer id. A group matches on
what `K` shows for it, its input and result, so a folded group is found without `zR`. Only
readable payloads count: `/` never calls the codec, so a value still encrypted is invisible to
it until `K` has shown that row, and a miss says how many payloads it could not read.

A payload value is not on the row, so there is nothing there to light up. When only a payload
explains a hit, the statusline names the path, `3 match(es), in .input.customer.id`. The `K`
pane paints the match and scrolls to it when a search lands; for a `path=value` pattern it
paints the value, since the pane shows JSON and never the flattened form.

Saved views are bound under the **leader**, not to bare digits as this document originally
specified. A leading digit in Normal mode starts a count, and counts composing with every
motion (`7j`, `10G`) is worth more than saving one keystroke. Only views that `views.toml`
actually defines get a binding, so the which-key popup never advertises an empty slot, it
lists them by name.

### The query bar

The visibility query is always on screen and always the raw string. Anything that filters the
list writes *into* that text rather than replacing it with a structure you cannot see: a
saved view fills the bar and leaves it editable, and the filter builder on `<leader>fg` does
the same. This is the one piece of the web UI's design being deliberately rejected rather
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

`<leader>fg` builds its clauses from three places, and two of them are the point. Statuses
are fixed by the protocol. Workflow types and task queues come from the **rows already
loaded**, so the values offered are ones this namespace actually has. Search attributes come
from the **cluster**, so a custom `CustomerId` is offered without anyone having hardcoded it,
in the shape its type demands: a keyword takes `= ''`, an int takes `> 0`, a datetime takes a
timestamp, and offering the wrong one would look right and fail on send.

Time windows are offered as the instant they mean, `StartTime > '2026-09-21T06:03:22Z'`,
because the visibility grammar has no `now()`. The instant is computed when the picker opens,
so the clause is a fixed point rather than a window sliding while you read it. A timestamp is
unsearchable by eye, so those entries also match on words that are never shown: typing
`last hour` finds the one an hour back, `today` finds midnight **in your zone**, not UTC.

Accepting one `AND`s it onto the query bar and leaves the text editable, so filters compose
by visiting the picker twice.

The same catalogue is reachable without leaving the bar. Typing in Insert mode offers the
clauses that match what is being typed, and `<Tab>` takes the highlighted one (the jumplist's
`<Tab>` is bound in Normal mode only, so the two never meet); `<C-n>` and
`<C-p>` move through the list, and `<Esc>` dismisses it without costing the line. What is
replaced is the whole **clause**, everything since the last `AND` or `OR`, not the word at
the end: `ExecutionStatus = 'Running'` has spaces in it, and completing a word would leave
the field name behind and produce `ExecutionStatus = ExecutionStatus = 'Running'`. A quoted
literal is not a split, so `WorkflowId = 'send and forget'` stays one clause.

Nothing is offered until something is typed, and nothing is offered once a clause ends in a
space: a list that appears the moment Insert mode opens, or that offers back the text just
finished, is in the way of the common case, which is typing a query you already know. `<Tab>`
completes but never applies; `⏎` still applies, so a query typed in full is never diverted by
a list nobody was reading.

A list longer than its pane carries a scrollbar down the right edge: thumb only, no track
and no arrows, so a pane with nothing to scroll looks exactly as it did. The thumb is sized
to the share of the list on screen, which is the part a row count cannot tell you: `1/4312`
says where the cursor is, the thumb says how much of the whole you are looking at.

Where a pane is boxed, the help overlay, the bar rides the border and costs nothing. The
lists are not boxed, and their rightmost column holds the age, so there the bar takes a
column of its own rather than painting over a value.

### Windows

| Key | Action | |
|---|---|---|
| `<leader>sv` / `<leader>sh` | split side by side / above and below | **live** |
| `<leader>se` / `<leader>sx` | equalise / close | **live** |
| `<C-w>hjkl` | move focus | **live** |
| `<leader>r{h,j,k,l}` | resize by 10 | **live** |
| `<leader>t{o,x,n,p}` | tab open / close / next / previous | **live** |

Two histories in a split will be the diff feature. There is no separate diff screen.

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

`<leader>G` draws the same history as a timeline, modelled on the one in Temporal's web UI
so the two read alike. Each group is its events as dots on one shared axis, joined by a line
coloured by how the group ended, in the web UI's own colours: green completed, red failed or
terminated, orange timed out, amber canceled, blue for the running workflow. An activity's
queued stretch, scheduled to started, is the faded first part of its line; a retried activity
that got there fades from red to green and carries `↻ 3 •` before its name; anything still
running trails a dashed line to the edge. The axis counts from the start of the run, `1m 30s`,
or reads as clock times under `<leader>T`.

An idle stretch, where nothing but the workflow itself was open, is folded to a `≀` mark once
it is a tenth of the time drawn to scale, the web UI's rule, because one two-hour timer drawn
to scale turns every activity around it into a dot. `zg` unfolds them. The rows are the
outline's rows, so `za`, `]f`, `K`, search and the cursor all work unchanged, and a split can
show the list beside the timeline of the same run. The two activity and workflow colours the
web UI uses, `#8b008b` and `#0014a8`, are lifted to shade 7 of the same hue: as a one-cell line
on a dark terminal the originals all but disappear.

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

A failure is a chain, not a sentence. The server wraps what the worker raised: an activity
failure over the application failure over whatever that one came from, and the outermost
message is usually the least specific of the three, a variation on "activity task failed". A
row has space for one line of it, so the row shows the type and message of the outermost link
and `K` shows the rest: every `caused by` down to the root, the SDK that raised it, whether it
was marked non-retryable, and the stack traces, which sit after the payloads because a
fifty-line Java trace would otherwise push the input out of sight.

`<leader>ff` ranks the rows this pane has loaded, and a prompt those rows cannot answer is
sent to the server: `WorkflowId = '…' OR RunId = '…'`, then `STARTS_WITH` if that finds
nothing. Either kind of id works, because the id pasted out of a log line is as often a run
id as a workflow id. Rows that came back are tagged `found by id`, whether they were already
in the pane's list or had to be fetched, and either way they open. The query goes out 300ms after typing stops,
never while the loaded rows still match, and never for a prompt under six characters, so
reading the list costs nothing and looking for one id costs one round trip.

`/` searches the rows a pane holds, and on a history it will read on to find them. A run's
events are finite and all belong to the workflow on screen, so when a pattern is in none of
the events loaded, tmprl keeps fetching pages (1000 events at a time) and searching until it
lands or the run runs out. The statusline counts as it goes and `<Esc>` stops it. On a
workflow list `/` stays local: paging a namespace to find a row is unbounded, and that is
what the query bar is for.

A retry in progress is not in the history at all. Temporal writes `ActivityTaskStarted`, the
event carrying the attempt and the last failure, only when the activity closes, so a retrying
activity is a bare `ActivityTaskScheduled`. For a running workflow tmprl therefore also calls
`DescribeWorkflowExecution` and matches its pending activities to the open rows by activity id:
the row shows `×4/10` (attempt and maximum, `∞` when unlimited), `retry in 12s` while backing
off, and the last failure; `K` adds the state, the next attempt time and the last worker. This
is fetched when the history opens and on `R`, and every 5 seconds under `F`, because a retry
writes no event for the follow long poll to wake on.

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
| `<leader>G` | the history as a timeline, like the web UI's | **live** |
| `zg` | fold or unfold the timeline's idle stretches | **live** |
| `]f` / `[f` | jump to the next / previous failure | **live** |
| `F` | follow, tail a running workflow | **live** |
| `<leader>cs` | call stack (`__stack_trace` query) | planned |
| `<leader>cq` | send a query to the workflow | planned |
| `!` | pipe the focused payloads through a command | **live** |
| `K` | show the payloads and the full failure under the cursor | **live** |
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
| `<C-q>` | send selection to the quickfix list | planned |
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
| `<leader>xm` or `:messages` | everything the note line has said | **live** |
| `<leader>xQ` | open the quickfix list | planned |

The quickfix list, when it exists, is how larger batches will be staged. Select rows, `<C-q>`
to stage them, then run an operation over the staged set. Staging is a visible, editable list
rather than an invisible selection, because *"which four thousand workflows am I about to
terminate?"* should be a question with an answer on screen. Until then a batch is a `V`
selection.

Signal and update ask for a name and send no input; a payload for either is planned, along
with the query on `<leader>cq`.

A note on the status line is gone at the next key, which suits "yanked 3 rows" and does not
suit a failure. `:messages` keeps the session's notes, newest last. A failed request reads as
the call, the reason in words and the server's own message, `ListWorkflowExecutions failed
(unavailable): transport error`, and under it the log adds the name gRPC uses for the code,
since that is what a server log says, and what to try when there is something to try. An
empty list whose load failed says the same on the spot.

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
under `<leader>m`, clear of bare `m`, which marks reserve. A batch over a *query*, which is
Temporal's own server-side batch API, is planned for 0.3; the batch over a selection of rows
described above is live.

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

A query-driven batch will additionally show a `CountWorkflowExecutions` dry run and require
typing the affected count. Every mutation appends to `~/.local/state/tmprl/audit.jsonl`, failures
included, the question that log answers is what was *attempted*.

## Command line

Everything is optional. With no arguments `tmprl` connects as the default profile and opens
on the namespace list.

| Flag | |
|---|---|
| `-p`, `--profile <NAME>` | connect as this profile from `temporal.toml` |
| `-n`, `--namespace <NAME>` | open on that namespace's workflow list, overriding the profile's namespace |
| `-q`, `--query <QUERY>` | open on the workflow list with this visibility query in the bar, applied |
| `-w`, `--workflow <ID>` | open the history of this workflow id or run id |
| `--dashboard` | open on the dashboard, of the profile's namespace or the one `-n` names |
| `--address <HOST:PORT>` | connect to this server instead of the profile's address |
| `--readonly` | refuse every mutation for this run |
| `--temporal-config <PATH>` | read connection profiles from this file (alias: `--config`) |
| `--config-path` | print every file `tmprl` reads and whether it exists, then exit |
| `-h`, `--help` / `-V`, `--version` | print the usage, or the version, then exit |

A long flag also takes its value as `--flag=value`, which is the way to pass a value that
starts with a dash. A flag given twice takes the last value, and an argument `tmprl` does not
know exits with status 2 before anything connects.

`-q`, `-w` and `--dashboard` use the profile's namespace unless `-n` names another, and `-`
from wherever they land still goes up a level, so the namespace list is one key away.
`--dashboard` cannot be combined with `-q` or `-w`, which each name a different screen to
open on; the combination exits with status 2.

`-w` asks the server for an exact match on either id, the same lookup `<leader>ff` falls back
to, without that picker's prefix search: it opens what it finds, so it must not guess. When a
workflow id has several runs the newest opens and the statusline says how many matched. When
nothing matches, `tmprl` lands on the workflow list (with the `-q` query if one was given) and
says so, rather than exiting.

`--address` replaces the address and nothing else. TLS, the API key and gRPC metadata still
come from the profile, and a bare `host:port` is dialled with TLS exactly when the profile
would have used it. It outranks `TEMPORAL_ADDRESS`, as `-n` outranks `TEMPORAL_NAMESPACE`.

`--readonly` is `readonly = true` from `config.toml` for one run: the header reads
`prod [ro]`, and a mutation is refused before its prompt, form or confirmation opens. The flag
can only add the restriction; nothing on the command line lifts a profile's own.

## Theming

Status is encoded in shape as well as hue. Every execution status has its own glyph, `●`
running, `✓` completed, `✗` failed, `■` terminated, `⊘` cancelled, `◔` timed out, `↻`
continued-as-new, `‖` paused, used identically in the table and in the header tallies. Colour
only reinforces it, so the workflow list stays readable for a colour-blind reader, and with no
colour at all.

### How much colour

Decided once, at startup, from two environment variables. tmprl cannot see the terminal's
background, so the fallback is the palette that does not need to.

| Environment | Palette |
|---|---|
| `NO_COLOR` set to anything but the empty string | none: bold, dim, underline and reverse video only |
| otherwise, `COLORTERM` is `truecolor` or `24bit` | 24-bit RGB, tuned for a dark background |
| otherwise | the terminal's own 16 named colours |

The 16-colour palette names only colours with a hue (blue, green, yellow, red, and magenta
and cyan for the mode indicator). Text is the terminal's own foreground, chrome is that
foreground dimmed, and the cursor row is reverse video, so it reads on a light background
as well as a dark one and follows whatever theme the terminal has. `TERM` is not consulted:
it says what a terminal emulates, not what it can paint. A truecolor terminal that does not
export `COLORTERM`, which is common over `ssh`, therefore gets 16 colours until it does.

Without colour the things that were told apart by colour are told apart by weight: the cursor
row and a selection are reverse video (the cursor row bold as well), the focused pane's border
is bold against the others' dim, an error is bold, and a `/` match is reverse video, or on the
cursor row the one stretch that is *not* reversed, underlined. The timeline (`<leader>G`) is
the exception: it draws how a group ended in the colour of its line, so without colour that
is only in the list view's glyphs.

### `theme.toml`

`~/.config/tmprl/theme.toml` repaints the palette, one slot per line. A slot it does not name
keeps its default.

```toml
accent = "#6b99e8"
err    = "bright-red"
sel    = "#2c3340"
```

| Slot | Paints |
|---|---|
| `fg` | primary text |
| `dim` | secondary text, such as a workflow's type |
| `faint` | chrome: gutters, borders, hints |
| `accent` | titles, the text cursor, the focused pane's border, anything running |
| `ok` | completed, and the `FOLLOW` indicator |
| `warn` | terminated and cancelled, key chords, the cursor row's line number |
| `err` | failed and timed out, error messages |
| `sel` | the background of the cursor row and of a selection |
| `mode_normal`, `mode_insert`, `mode_visual`, `mode_command` | the mode indicator's background |

A value is `#rrggbb`, or one of the terminal's colour names: `black`, `red`, `green`, `yellow`,
`blue`, `magenta`, `cyan`, `white`, and each with a `bright-` prefix. A name stays the
terminal's own colour at every depth. On a 16-colour terminal a hex value becomes the nearest
named colour: its hue picks the colour, then its brightness picks the plain or `bright-` form,
and only a value with almost no hue becomes a grey. Under `NO_COLOR` the file is checked but
not applied.

The loader is strict, like the others: a key that is not a slot, or a value that is not a
colour, is reported in the statusline at startup and the whole file is set aside, rather than
the line being skipped. A profile's `accent` in `config.toml` is separate from the `accent`
slot: it colours the profile name in the header, always in a named colour, so production
stays unmistakable whatever the theme says.

The timeline's colours are Temporal's web UI's and are not slots. They go through the same
depth rule: nearest named colour on 16, none under `NO_COLOR`.

## Configuration files

| File | Holds |
|---|---|
| `~/.config/tmprl/config.toml` | codec server endpoint, payload pane position, display timezone, yank limit and the dashboard's refresh interval, **live**; other refresh intervals and defaults *planned* |
| `~/.config/tmprl/dashboard.toml` | the dashboard's panels and their arrangement, **live** |
| `~/.config/tmprl/keys.toml` | key chord → command id, **live** |
| `~/.config/tmprl/theme.toml` | palette slot → colour, **live** |
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

The dashboard refreshes every 30 seconds. To change that, or to stop it:

```toml
[refresh]
dashboard = "2m"        # "30s" is the default, "5s" the least, "off" leaves it to R
```

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

A `dashboard.toml` is rows from top to bottom, each holding panels from left to right:

```toml
[[row]]
lines = 3                # a fixed height; or `height`, a share of what is left

[[row.panel]]
kind = "counts"

[[row]]
height = 3

[[row.panel]]
kind  = "workflows"
title = "Recent failures"
query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
since = "24h"
width = 2                # twice the share of its neighbour

[[row.panel]]
kind  = "types"
query = "ExecutionStatus IN ('Failed', 'TimedOut', 'Terminated')"
since = "24h"
```

| Key | On | Value | Default | |
|---|---|---|---|---|
| `height` | row | 1 to 100 | 1 | the row's share of the height left over. A row whose panels need less than their share takes only that, and rows with more to show get the rest |
| `lines` | row | 3 to 50 | | a fixed height instead, borders included; not with `height` |
| `kind` | panel | `counts` `workflows` `types` `queues` `histogram` `schedules` | required | what the panel shows |
| `title` | panel | text | from the kind | the title on its border |
| `width` | panel | 1 to 100 | 1 | the panel's share of the row |
| `namespaces` | panel | list of names | the pane's scope | the namespaces it asks |
| `query` | `counts` `workflows` `types` `queues` | a visibility query | none; `queues`: running workflows | what it counts, lists or tallies. No `ORDER BY`: a panel sorts its own rows |
| `since` | `counts` `workflows` `types` `histogram` | `30m` `24h` `7d` `2w` | | only workflows started within this long, worked out at each refresh |
| `older` | `counts` `workflows` `types` `histogram` | a duration, as `since` | | only workflows started longer ago than this. With `since` the two bound a stretch, and `since` must be the longer |
| `bucket` | `histogram` | a duration, as `since` | the finest round step giving at most 48 columns | the stretch of time one column stands for. It must cut the window into 2 to 48 columns |
| `by` | `counts` `workflows` `types` `histogram` | `start` `close` | `start` | the time `since` and `older` measure. `close` lists the last to close first and shows how long ago each closed; a workflow still running has no close time and is left out |
| `limit` | every kind but `counts` | 1 to 50 | 10 | the most items it holds |
| `names` | `queues` | list of names | | task queues to list even when nothing is running on them |
| `show` | `schedules` | `all` `paused` `upcoming` | `all` | which schedules; `upcoming` sorts by next run |

`histogram` draws how many workflows fell in each stretch of its window as columns, the
newest on the right, each as tall as its count against the tallest; the title gives the
peak. It needs `since`. A stretch still to be counted is drawn `░`, an empty one `·`, and
only stretches with something in them are stops for the cursor. Each column is one count:
with `by = "close"` a stretch that has passed is counted once and kept, otherwise every
column is counted again at each refresh. When the panel is narrower than its columns the
oldest are left off.

`workflows` lists the newest first, `types` tallies workflow types over the same rows, and
`queues` tallies the task queues those rows are on. Two panels with the same query, window
and namespaces share one request, so a list and its tally cannot disagree. A layout holds at
most 12 panels.

The file is read strictly, like `keys.toml`: a key that does not exist, or one that belongs
to another kind of panel, sets the whole file aside with a message naming it, and the
built-in layout is used. A misspelt `query` would otherwise show every workflow under a
"failures" title. A file with no rows is the same as no file.

Connection settings are deliberately *not* in this list, those come from
`~/.config/temporalio/temporal.toml`, the same file the `temporal` CLI uses.
