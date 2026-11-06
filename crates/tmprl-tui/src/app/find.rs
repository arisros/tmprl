//! Finding things: the pickers (`<leader>f…`) and `/` search.

use super::*;

impl App {
    pub(super) fn open_picker(&mut self, kind: picker::Kind) {
        let items = match kind {
            picker::Kind::Workflows => self.workflow_items(),
            picker::Kind::HistoryRows => self.history_items(),
            picker::Kind::Panes => self.pane_items(),
            picker::Kind::Commands => self.command_items(),
            picker::Kind::Filters => self.filter_items(),
            picker::Kind::Namespaces => self.namespace_items(),
            picker::Kind::Profiles => self.profile_items(),
            picker::Kind::Queries => self.query_items(),
        };
        if items.is_empty() {
            self.note = Some((
                match kind {
                    picker::Kind::Workflows => "no workflows loaded; open a namespace first",
                    picker::Kind::HistoryRows => "no history here; open a workflow first",
                    picker::Kind::Panes => "only this pane is open",
                    picker::Kind::Commands => "no commands",
                    picker::Kind::Filters => "nothing to filter on yet",
                    picker::Kind::Namespaces => "no namespaces loaded yet",
                    picker::Kind::Profiles => "no profiles in temporal.toml",
                    picker::Kind::Queries => "this workflow lists no queries",
                }
                .into(),
                Note::Warn,
            ));
            return;
        }
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

    pub(super) fn history_items(&self) -> Vec<picker::Item> {
        if self.view.screen != Screen::History {
            return Vec::new();
        }
        let labels = self.view.row_labels();
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

    pub(super) fn pane_items(&self) -> Vec<picker::Item> {
        let ids = self.tabs.views();
        if ids.len() < 2 {
            return Vec::new();
        }
        let focused = self.tabs.current().focused();
        ids.into_iter()
            .map(|id| {
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
                picker::Item::new(
                    format!("{}  {}", c.id, c.title),
                    Target::Command(c.id.to_string()),
                )
                .with_note(c.group)
            })
            .collect()
    }

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
        // Not <C-j>/<C-k>: tmux pane navigation takes them.
        let ctrl = chord.mods.ctrl;
        match chord.key {
            Key::Esc => self.picker = None,
            Key::Enter => self.accept_picker(),
            Key::Down => self.move_picker(1),
            Key::Up => self.move_picker(-1),
            Key::Char('n') if ctrl => self.move_picker(1),
            Key::Char('p') if ctrl => self.move_picker(-1),
            Key::Backspace if !p.backspace() => self.picker = None,
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => p.push(c),
            _ => {}
        }
        self.schedule_picker_search();
    }

    pub(super) fn picker_paste(&mut self, text: &str) {
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        for c in text.chars().filter(|c| !c.is_control()) {
            p.push(c);
        }
        self.schedule_picker_search();
    }

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

    pub(super) fn picker_wants_server(&self) -> bool {
        let Some(p) = self.picker.as_ref() else {
            return false;
        };
        p.kind == picker::Kind::Workflows
            && p.is_empty()
            && p.prompt.trim().chars().count() >= PICKER_SEARCH_MIN
    }

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
                .map(|(rows, _)| rows);
            if let (Ok(rows), Some(prefix)) = (&result, prefix)
                && rows.is_empty()
            {
                result = conn
                    .list_workflows_across(&scope, &prefix, PICKER_SEARCH_LIMIT)
                    .await
                    .map(|(rows, _)| rows);
            }
            let _ = tx.send(Msg::PickerFound { search, result });
        });
    }

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

    pub(super) fn accept_picker(&mut self) {
        let Some(p) = self.picker.as_ref() else {
            return;
        };
        let Some(target) = p.accept().cloned() else {
            self.note = Some(("no entry selected".into(), Note::Warn));
            return;
        };
        self.picker = None;

        match target {
            Target::Workflow { namespace, run_id } => self.open_workflow(&namespace, &run_id),
            Target::Row(row) => {
                if row >= self.row_count() {
                    self.note = Some(("that row has gone".into(), Note::Warn));
                    return;
                }
                self.mark_jump();
                self.set_cursor(row);
            }
            Target::Command(id) => self.run(&id, None),
            Target::Pane(id) => self.focus_pane(ViewId(id)),
            Target::Query(clause) => self.add_clause(&clause),
            Target::Namespace(name) => self.switch_namespace(&name),
            Target::Profile(name) => self.switch_profile(&name),
            Target::Handler(name) => self.run_query(name),
        }
    }

    pub(super) fn open_search(&mut self) {
        self.mode = Mode::Command;
        self.prompt = Some(Prompt {
            kind: PromptKind::Search,
            buf: LineEdit::default(),
        });
    }

    pub(super) fn run_search(&mut self, pattern: String) {
        self.search = Search::new(pattern);
        if self.row_count() == 0 {
            return;
        }
        let from = self.here();
        if self.seek(self.view.cursor, true, true) {
            self.jumps.push(from);
        }
    }

    pub(super) fn jump_match(&mut self, forward: bool) {
        if self.search.is_empty() {
            self.note = Some(("no search yet, press / first".into(), Note::Warn));
            return;
        }
        self.seek(self.view.cursor, forward, false);
    }

    pub(super) fn seek(&mut self, from: usize, forward: bool, inclusive: bool) -> bool {
        let labels = self.view.search_labels();
        let total = search::count(&self.search, &labels);
        match search::find(&self.search, &labels, from, forward, inclusive) {
            Some(hit) => {
                self.set_cursor(hit.row);
                self.view.detail_seek = true;
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
                    format!(
                        "/{}  {total} match(es){where_}{}",
                        self.search.pattern(),
                        self.matched_in(hit.row)
                    ),
                    Note::Info,
                ));
                true
            }
            None => {
                if self.start_scan(from, forward, inclusive) {
                    return false;
                }
                self.note = Some((
                    format!(
                        "no match for /{} in {total} row(s){}",
                        self.search.pattern(),
                        self.unsearched_note()
                    ),
                    Note::Warn,
                ));
                false
            }
        }
    }

    fn matched_in(&self, row: usize) -> String {
        self.view
            .payload_match(row, &self.search)
            .map(|path| format!(", in {path}"))
            .unwrap_or_default()
    }

    fn unsearched_note(&self) -> String {
        if self.view.screen != Screen::History {
            return String::new();
        }
        match self.view.undecoded_payloads() {
            0 => String::new(),
            n => format!(", {n} encrypted payload(s) not searched"),
        }
    }

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
            self.view.detail_seek = true;
            self.note = Some((
                format!(
                    "/{}  found after reading {} more event(s){}",
                    self.search.pattern(),
                    loaded - scan.started_with,
                    self.matched_in(hit.row)
                ),
                Note::Info,
            ));
            return;
        }
        if self.view.history_token.is_empty() {
            self.scan = None;
            self.note = Some((
                format!(
                    "no match for /{} in the whole history ({loaded} events){}",
                    self.search.pattern(),
                    self.unsearched_note()
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

pub(super) fn describe_pane(v: &View) -> String {
    match v.screen {
        Screen::Profiles => "profiles".to_string(),
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
        Screen::Dashboard => format!("dashboard  {}", v.scope.join(", ")),
    }
}
