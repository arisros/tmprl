//! Finding things: the pickers (`<leader>f…`) and `/` search.

use super::*;

impl App {
    /// Open a picker over whatever `kind` names.
    ///
    /// Items are gathered once, at open. A picker over a live list that reshuffled itself
    /// under the cursor while you typed would be unusable, and every one of these lists is
    /// live: workflows page in, a followed history grows. A snapshot is also what makes
    /// `Target::Row` sound, an index into a list that cannot change while the picker holds
    /// the keyboard.
    pub(super) fn open_picker(&mut self, kind: picker::Kind) {
        let items = match kind {
            picker::Kind::Workflows => self.workflow_items(),
            picker::Kind::HistoryRows => self.history_items(),
            picker::Kind::Panes => self.pane_items(),
            picker::Kind::Commands => self.command_items(),
            picker::Kind::Filters => self.filter_items(),
            picker::Kind::Namespaces => self.namespace_items(),
        };
        if items.is_empty() {
            // Saying why beats opening an empty box: the answer is nearly always "you are
            // on the wrong screen" or "nothing has loaded yet".
            self.note = Some((
                match kind {
                    picker::Kind::Workflows => "no workflows loaded; open a namespace first",
                    picker::Kind::HistoryRows => "no history here; open a workflow first",
                    picker::Kind::Panes => "only this pane is open",
                    picker::Kind::Commands => "no commands",
                    picker::Kind::Filters => "nothing to filter on yet",
                    picker::Kind::Namespaces => "no namespaces loaded yet",
                }
                .into(),
                Note::Warn,
            ));
            return;
        }
        // Rows the last picker fetched are not this picker's business.
        self.picker_found.clear();
        self.picker_search = self.picker_search.wrapping_add(1);
        self.picker = Some(Picker::new(kind, items));
    }

    pub(super) fn workflow_items(&self) -> Vec<picker::Item> {
        self.view
            .workflow_rows()
            .iter()
            .map(workflow_item)
            .collect()
    }

    /// Rows of the history outline, as the outline currently stands.
    ///
    /// Folded rows are not listed, deliberately: the picker offers what you can navigate to,
    /// and a row inside a collapsed group is not somewhere the cursor can go. `zR` first if
    /// you want the events too.
    pub(super) fn history_items(&self) -> Vec<picker::Item> {
        if self.view.screen != Screen::History {
            return Vec::new();
        }
        let labels = self.view.search_labels();
        let Some(outline) = self.view.history.value() else {
            return Vec::new();
        };
        labels
            .into_iter()
            .enumerate()
            .map(|(row, label)| {
                let note = match outline.row_at(row) {
                    Some(Row::Group { group, .. }) => outline
                        .group(group)
                        .map(|g| g.outcome.label().to_string())
                        .unwrap_or_default(),
                    Some(Row::Event { event, .. }) => outline
                        .event(event)
                        .map(|e| format!("event {}", e.id))
                        .unwrap_or_default(),
                    None => String::new(),
                };
                picker::Item::new(label, Target::Row(row)).with_note(note)
            })
            .collect()
    }

    /// Every open pane, across every tab. vim's `:ls`, and `<leader>fb` is its `:b`.
    pub(super) fn pane_items(&self) -> Vec<picker::Item> {
        let ids = self.tabs.views();
        if ids.len() < 2 {
            return Vec::new();
        }
        let focused = self.tabs.current().focused();
        ids.into_iter()
            .map(|id| {
                // The focused pane's state is in `self.view`; every other one is parked.
                let view = if id == focused {
                    Some(&self.view)
                } else {
                    self.parked_view(id)
                };
                let label = match view {
                    Some(v) => describe_pane(v),
                    None => format!("pane {}", id.0),
                };
                picker::Item::new(label, Target::Pane(id.0)).with_note(if id == focused {
                    "current".to_string()
                } else {
                    String::new()
                })
            })
            .collect()
    }

    pub(super) fn namespace_items(&self) -> Vec<picker::Item> {
        self.view
            .namespace_rows()
            .iter()
            .map(|n| {
                picker::Item::new(n.name.clone(), Target::Namespace(n.name.clone()))
                    .with_note(n.state.clone())
                    .with_preview(format!(
                        "namespace   {}\nstate       {}\nretention   {} days\n\n{}",
                        n.name, n.state, n.retention_days, n.description,
                    ))
            })
            .collect()
    }

    pub(super) fn command_items(&self) -> Vec<picker::Item> {
        self.registry
            .search("")
            .into_iter()
            .map(|c| {
                // Id *and* title in the label, because `Picker` matches the label and
                // nothing else. With the title in the note, `:` found `list.problems` by
                // typing `failed` while `<leader>fh` did not, which is a confusing split
                // between two things that are both "find a command".
                picker::Item::new(
                    format!("{}  {}", c.id, c.title),
                    Target::Command(c.id.to_string()),
                )
                .with_note(c.group)
            })
            .collect()
    }

    /// Clauses to build a visibility query out of.
    ///
    /// The catalogue itself is [`tmprl_core::filter`], which is pure and tested there. This
    /// only gathers what it needs from the session: the values on the rows already loaded,
    /// so the offers are ones that exist in this namespace, and the search attributes the
    /// cluster registered, so a custom attribute comes with a clause shaped to its type.
    ///
    /// Each entry is a clause, not a whole query: accepting one appends it to the bar with
    /// `AND`, and leaves the text editable. The query is the interface, and a builder that
    /// replaced it with something you could not see is the web UI's mistake.
    pub(super) fn filter_items(&self) -> Vec<picker::Item> {
        self.filter_clauses()
            .into_iter()
            .map(|c| {
                picker::Item::new(c.text.clone(), Target::Query(c.text))
                    .with_note(c.note)
                    .searchable_as(c.keywords)
            })
            .collect()
    }

    /// The catalogue itself, shared by the picker and by the completion under the query
    /// bar: two ways to reach one list, not two lists that drift apart.
    pub(super) fn filter_clauses(&self) -> Vec<filter::Clause> {
        let rows = self.view.workflow_rows();
        let mut types: Vec<&str> = rows.iter().map(|w| w.workflow_type.as_str()).collect();
        types.sort_unstable();
        types.dedup();
        let mut queues: Vec<&str> = rows.iter().map(|w| w.task_queue.as_str()).collect();
        queues.sort_unstable();
        queues.dedup();

        let facts = filter::Facts {
            types: &types,
            queues: &queues,
            attributes: self
                .search_attributes
                .value()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            now_ms: now_ms(),
            clock: &self.clock,
        };

        filter::clauses(&facts)
    }

    pub(super) fn picker_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        // `<C-n>` / `<C-p>` rather than `<C-j>` / `<C-k>`: tmux's pane navigation eats the
        // latter before this application ever sees them. See docs/INTERFACE.md.
        let ctrl = chord.mods.ctrl;
        match chord.key {
            Key::Esc => self.picker = None,
            Key::Enter => self.accept_picker(),
            Key::Down => self.move_picker(1),
            Key::Up => self.move_picker(-1),
            Key::Char('n') if ctrl => self.move_picker(1),
            Key::Char('p') if ctrl => self.move_picker(-1),
            // Backspace on an empty prompt closes, as it does at every other prompt here.
            // The guard does the deleting, matching how `prompt_key` is written.
            Key::Backspace if !p.backspace() => self.picker = None,
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => p.push(c),
            _ => {}
        }
        self.schedule_picker_search();
    }

    /// Start the debounce after a picker's prompt changed.
    ///
    /// Only for the workflow picker, and only while the loaded rows match nothing: the
    /// local list is the answer whenever it has one, and a picker that fired a visibility
    /// query on every keystroke would put a query per character on a production cluster.
    fn schedule_picker_search(&mut self) {
        self.picker_search = self.picker_search.wrapping_add(1);
        if self.conn.is_none() || !self.picker_wants_server() {
            return;
        }
        let (tx, search) = (self.tx.clone(), self.picker_search);
        tokio::spawn(async move {
            tokio::time::sleep(PICKER_DEBOUNCE).await;
            let _ = tx.send(Msg::PickerDebounce { search });
        });
    }

    /// Whether the picker on screen is one a server search could help, with a prompt worth
    /// sending. Short prompts are excluded: three characters of a UUID match half a
    /// namespace, and the round trip would be spent to say so.
    pub(super) fn picker_wants_server(&self) -> bool {
        let Some(p) = self.picker.as_ref() else {
            return false;
        };
        p.kind == picker::Kind::Workflows
            && p.is_empty()
            && p.prompt.trim().chars().count() >= PICKER_SEARCH_MIN
    }

    /// The debounce expired. Issue the search if nothing has changed since.
    pub(super) fn picker_search_due(&mut self, search: u64) {
        if search != self.picker_search || !self.picker_wants_server() {
            return;
        }
        let prompt = self
            .picker
            .as_ref()
            .map(|p| p.prompt.clone())
            .unwrap_or_default();
        let Some(exact) = tmprl_core::query::by_id(&prompt) else {
            self.note = Some(("a quote cannot go in a workflow search".into(), Note::Warn));
            return;
        };
        let prefix = tmprl_core::query::by_prefix(&prompt);
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (tx, scope) = (self.tx.clone(), self.view.scope.clone());
        self.note = Some((
            format!(
                "no match in {} loaded rows, asking the server…",
                self.view.workflow_rows().len()
            ),
            Note::Info,
        ));
        tokio::spawn(async move {
            let mut result = conn
                .list_workflows_across(&scope, &exact, PICKER_SEARCH_LIMIT)
                .await
                .map(|(rows, _)| rows)
                .map_err(|e| e.to_string());
            // An id nobody has in full is still a prefix worth trying, but only when the
            // exact lookup came back with nothing: a hit there is the answer.
            if let (Ok(rows), Some(prefix)) = (&result, prefix)
                && rows.is_empty()
            {
                result = conn
                    .list_workflows_across(&scope, &prefix, PICKER_SEARCH_LIMIT)
                    .await
                    .map(|(rows, _)| rows)
                    .map_err(|e| e.to_string());
            }
            let _ = tx.send(Msg::PickerFound { search, result });
        });
    }

    /// Put fetched rows into the open picker.
    pub(super) fn picker_takes(&mut self, rows: Vec<WorkflowRow>) {
        let items: Vec<picker::Item> = rows
            .iter()
            .map(|w| workflow_item(w).from_lookup())
            .collect();
        let found = rows.len();
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        let shown = p.answer(items);
        self.picker_found.extend(rows);
        self.note = Some((
            if found == 0 {
                "the server has no workflow with that id either".to_string()
            } else {
                format!("{shown} found by id")
            },
            if found == 0 { Note::Warn } else { Note::Info },
        ));
    }

    pub(super) fn move_picker(&mut self, delta: isize) {
        if let Some(p) = self.picker.as_mut() {
            p.move_cursor(delta);
        }
    }

    /// Take the selected entry and close.
    ///
    /// The match on `Target` is exhaustive, so adding a picker whose outcome is genuinely
    /// new is a compile error here rather than a key that does nothing.
    pub(super) fn accept_picker(&mut self) {
        let Some(p) = self.picker.as_ref() else {
            return;
        };
        let Some(target) = p.accept().cloned() else {
            // Nothing matched what was typed. Closing silently would look like it took
            // something, so leave the picker open and say nothing happened.
            self.note = Some(("no entry selected".into(), Note::Warn));
            return;
        };
        self.picker = None;

        match target {
            Target::Workflow { namespace, run_id } => self.open_workflow(&namespace, &run_id),
            Target::Row(row) => {
                // The snapshot was taken when the picker opened. A history that re-grouped
                // while it was up, or a page arriving under a follow, can leave it short.
                // Saying so beats an `Enter` that closes the picker and moves nothing.
                if row >= self.row_count() {
                    self.note = Some(("that row has gone".into(), Note::Warn));
                    return;
                }
                // A jump, as docs/INTERFACE.md promises: crossing a thousand-row history is
                // exactly the move `<C-o>` should undo.
                self.mark_jump();
                self.set_cursor(row);
            }
            Target::Command(id) => self.run(&id, None),
            // Not a jump. Every jumplist entry describes a position *within* a pane, and
            // switching which pane is focused moves no cursor; recording it would make
            // `<C-o>` mean two different things.
            Target::Pane(id) => self.focus_pane(ViewId(id)),
            Target::Query(clause) => self.add_clause(&clause),
            Target::Namespace(name) => self.switch_namespace(&name),
        }
    }

    pub(super) fn open_search(&mut self) {
        // Seeded empty rather than with the last pattern. `/` almost always means "look for
        // something else"; repeating the last search is what `n` is for, and pre-filling
        // would mean clearing the line before every second search.
        // As `:` and `!` do. Without it the statusline reads NORMAL while a `/` prompt is
        // open and being typed into, and `close_prompt` resets a mode that was never set.
        self.mode = Mode::Command;
        self.prompt = Some(Prompt {
            kind: PromptKind::Search,
            buf: String::new(),
        });
    }

    /// Apply a freshly typed pattern and jump to the first match.
    ///
    /// Unlike `n`, this considers the row the cursor is already on: you have just typed the
    /// pattern, and a `/` that skips a visible match one line up reads as not having found
    /// it. Starting one row back and searching forward is how that falls out.
    pub(super) fn run_search(&mut self, pattern: String) {
        self.search = Search::new(pattern);
        if self.row_count() == 0 {
            return;
        }
        // `/` is a jump, `n` is not. vim draws the line in the same place, and for the same
        // reason: the first search is how you left, the repeats are the walking.
        //
        // Captured before the seek and pushed only if it landed: `Jumplist::push` truncates
        // the forward entries, so recording a search that matched nothing would throw away
        // the `<C-i>` list for a keystroke that moved nothing.
        let from = self.here();
        if self.seek(self.view.cursor, true, true) {
            self.jumps.push(from);
        }
    }

    /// `n` and `N`: the same pattern again, from where the cursor is now.
    pub(super) fn jump_match(&mut self, forward: bool) {
        if self.search.is_empty() {
            self.note = Some(("no search yet, press / first".into(), Note::Warn));
            return;
        }
        self.seek(self.view.cursor, forward, false);
    }

    /// Move the cursor to the next row matching the current pattern, and say what happened.
    ///
    /// Every outcome gets a message, because the alternatives are all worse: a silent
    /// no-match looks like the key is unbound, and a silent wrap looks like the cursor
    /// jumped on its own.
    ///
    /// Reports whether it landed anywhere, so a caller can decide not to record a jump for
    /// a search that found nothing.
    pub(super) fn seek(&mut self, from: usize, forward: bool, inclusive: bool) -> bool {
        let labels = self.view.search_labels();
        let total = search::count(&self.search, &labels);
        match search::find(&self.search, &labels, from, forward, inclusive) {
            Some(hit) => {
                self.set_cursor(hit.row);
                let where_ = if hit.wrapped {
                    if forward {
                        " (wrapped to the top)"
                    } else {
                        " (wrapped to the bottom)"
                    }
                } else {
                    ""
                };
                self.note = Some((
                    format!("/{}  {total} match(es){where_}", self.search.pattern()),
                    Note::Info,
                ));
                true
            }
            None => {
                if self.start_scan(from, forward, inclusive) {
                    return false;
                }
                self.note = Some((
                    format!("no match for /{} in {total} row(s)", self.search.pattern()),
                    Note::Warn,
                ));
                false
            }
        }
    }

    /// Keep looking past the events loaded so far, if this is a history with more pages.
    ///
    /// The rest of the run is finite and belongs to the workflow on screen, so paging it in
    /// is a bounded thing to do, unlike paging a namespace. Returns whether a scan started.
    fn start_scan(&mut self, from: usize, forward: bool, inclusive: bool) -> bool {
        if self.view.screen != Screen::History
            || self.view.history_token.is_empty()
            || self.search.is_empty()
        {
            return false;
        }
        self.scan = Some(Scan {
            from,
            forward,
            inclusive,
            generation: self.view.generation,
            started_with: self.view.history_events.len(),
        });
        self.note = Some((
            format!(
                "/{}: not in the {} events loaded, reading on… (<Esc> to stop)",
                self.search.pattern(),
                self.view.history_events.len()
            ),
            Note::Info,
        ));
        self.load_history_page(SCAN_PAGE_SIZE);
        true
    }

    /// A page landed while a scan was running: look again, then decide whether to go on.
    pub(super) fn continue_scan(&mut self, generation: u64) {
        let Some(scan) = self.scan else {
            return;
        };
        if scan.generation != generation || self.view.screen != Screen::History {
            self.scan = None;
            return;
        }
        let labels = self.view.search_labels();
        let loaded = self.view.history_events.len();
        if let Some(hit) = search::find(
            &self.search,
            &labels,
            scan.from,
            scan.forward,
            scan.inclusive,
        ) {
            self.scan = None;
            self.set_cursor(hit.row);
            self.note = Some((
                format!(
                    "/{}  found after reading {} more event(s)",
                    self.search.pattern(),
                    loaded - scan.started_with
                ),
                Note::Info,
            ));
            return;
        }
        if self.view.history_token.is_empty() {
            self.scan = None;
            self.note = Some((
                format!(
                    "no match for /{} in the whole history ({loaded} events)",
                    self.search.pattern()
                ),
                Note::Warn,
            ));
            return;
        }
        self.note = Some((
            format!(
                "/{}: {loaded} events read, still looking… (<Esc> to stop)",
                self.search.pattern()
            ),
            Note::Info,
        ));
        self.load_history_page(SCAN_PAGE_SIZE);
    }

    /// End a scan, saying how far it got. `<Esc>`, and anything that abandons the history.
    pub(super) fn stop_scan(&mut self) {
        if self.scan.take().is_some() {
            self.note = Some((
                format!(
                    "search stopped, {} events read",
                    self.view.history_events.len()
                ),
                Note::Info,
            ));
        }
    }
}

/// One workflow as a picker entry.
///
/// The label is the workflow id alone: a label carrying the type and the status too would
/// let a fuzzy pattern match characters spread across three different facts, which reads as
/// the picker matching nothing you typed.
fn workflow_item(w: &WorkflowRow) -> picker::Item {
    picker::Item::new(
        w.workflow_id.clone(),
        Target::Workflow {
            namespace: w.namespace.clone(),
            run_id: w.run_id.clone(),
        },
    )
    .with_note(format!("{}  {}", w.workflow_type, w.status.query_name()))
    .with_preview(format!(
        "workflow id  {}\nrun id       {}\ntype         {}\ntask queue   {}\nnamespace    {}\nstatus       {}\nevents       {}",
        w.workflow_id,
        w.run_id,
        w.workflow_type,
        w.task_queue,
        w.namespace,
        w.status.query_name(),
        w.history_length,
    ))
}

/// A one-line description of what a pane is showing, for the pane picker.
///
/// Named by *what is in it* rather than by an id, because "pane 3" tells you nothing about
/// which of four open histories it is. This mirrors how vim's `:ls` names buffers by file.
pub(super) fn describe_pane(v: &View) -> String {
    match v.screen {
        Screen::Namespaces => "namespaces".to_string(),
        Screen::Workflows => {
            let scope = v.scope.join(", ");
            if v.query.trim().is_empty() {
                format!("workflows  {scope}")
            } else {
                format!("workflows  {scope}  [{}]", v.query.trim())
            }
        }
        Screen::History => match &v.viewing {
            Some(w) => format!("history  {}  {}", w.workflow_id, w.workflow_type),
            None => "history".to_string(),
        },
        Screen::Schedules => format!("schedules  {}", v.scope.join(", ")),
    }
}
