//! Mutations: choosing the target, the confirmation, and running them.

use super::*;

impl App {
    /// The schedule a schedule mutation would act on.
    pub(super) fn target_schedule(&self) -> Option<ScheduleRow> {
        if self.view.screen != Screen::Schedules {
            return None;
        }
        self.view.schedule_rows().get(self.view.cursor).cloned()
    }

    /// The workflow a mutation would act on: the row under the cursor on the workflow list,
    /// or the one whose history is open.
    /// Every workflow a mutation would act on.
    ///
    /// The visual selection when there is one, otherwise the single row under the cursor.
    /// One path rather than two, so a batch cannot drift from what a single action does.
    pub(super) fn target_workflows(&self) -> Vec<WorkflowRow> {
        if self.view.screen == Screen::Workflows
            && let Some((lo, hi)) = self.view.selection()
        {
            let rows = self.view.workflow_rows();
            return rows[lo.min(rows.len())..(hi + 1).min(rows.len())].to_vec();
        }
        self.target_workflow().into_iter().collect()
    }

    /// Every schedule a mutation would act on, on the same rule.
    pub(super) fn target_schedules(&self) -> Vec<ScheduleRow> {
        if self.view.screen == Screen::Schedules
            && let Some((lo, hi)) = self.view.selection()
        {
            let rows = self.view.schedule_rows();
            return rows[lo.min(rows.len())..(hi + 1).min(rows.len())].to_vec();
        }
        self.target_schedule().into_iter().collect()
    }

    pub(super) fn target_workflow(&self) -> Option<WorkflowRow> {
        match self.view.screen {
            Screen::Workflows => self.view.workflow_rows().get(self.view.cursor).cloned(),
            Screen::History => self.view.viewing.clone(),
            Screen::Namespaces | Screen::Schedules => None,
        }
    }

    /// Open the confirmation for a mutation. Nothing happens to the cluster here.
    pub(super) fn confirm_mutation(&mut self, kind: MutationKind) {
        if self.refuses_mutation() {
            return;
        }
        // Schedule operations act on a schedule id, not an execution.
        if matches!(
            kind,
            MutationKind::PauseSchedule
                | MutationKind::TriggerSchedule
                | MutationKind::DeleteSchedule
                | MutationKind::BackfillSchedule
        ) {
            let rows = self.target_schedules();
            if rows.is_empty() {
                self.note = Some(("no schedule under the cursor".into(), Note::Warn));
                return;
            }
            if matches!(kind, MutationKind::BackfillSchedule) {
                // The window has to be typed, so this reaches a confirmation only after the
                // prompt comes back.
                self.prompt = Some(Prompt {
                    kind: PromptKind::Backfill,
                    buf: String::new(),
                });
                self.mode = Mode::Command;
                return;
            }
            // Pause reads the state of the *first* row and moves every selected schedule to
            // the same one. Toggling each independently would leave a mixed selection still
            // mixed, which is never what one keypress was asking for.
            let target_paused = !rows[0].paused;
            let mutations = rows
                .into_iter()
                .map(|row| {
                    let (namespace, schedule_id) = (row.namespace, row.schedule_id);
                    match kind {
                        MutationKind::PauseSchedule => Mutation::PauseSchedule {
                            namespace,
                            schedule_id,
                            paused: target_paused,
                        },
                        MutationKind::TriggerSchedule => Mutation::TriggerSchedule {
                            namespace,
                            schedule_id,
                        },
                        _ => Mutation::DeleteSchedule {
                            namespace,
                            schedule_id,
                        },
                    }
                })
                .collect();
            self.confirm = Some(Confirm::batch(mutations));
            return;
        }

        // Both need a name, which has to be typed. Reuse the prompt rather than inventing a
        // second text field; the name applies to every row the selection covers.
        if matches!(kind, MutationKind::Signal | MutationKind::Update) {
            if self.target_workflows().is_empty() {
                self.note = Some(("no workflow under the cursor".into(), Note::Warn));
                return;
            }
            self.prompt = Some(Prompt {
                kind: if matches!(kind, MutationKind::Signal) {
                    PromptKind::Signal
                } else {
                    PromptKind::Update
                },
                buf: String::new(),
            });
            self.mode = Mode::Command;
            return;
        }

        if matches!(kind, MutationKind::Reset) {
            // A reset goes back to an event, which only a history has, and a history is one
            // workflow. There is nothing to batch.
            let Some(row) = self.target_workflow() else {
                self.note = Some(("no workflow under the cursor".into(), Note::Warn));
                return;
            };
            let Some(event_id) = self.reset_target() else {
                self.note = Some((
                    "reset needs a workflow history with a completed workflow task above \
                     the cursor"
                        .into(),
                    Note::Warn,
                ));
                return;
            };
            self.confirm = Some(Confirm::new(Mutation::Reset {
                namespace: row.namespace,
                workflow_id: row.workflow_id,
                run_id: row.run_id,
                event_id,
                reason: "reset from tmprl".into(),
            }));
            return;
        }

        let rows = self.target_workflows();
        if rows.is_empty() {
            self.note = Some(("no workflow under the cursor".into(), Note::Warn));
            return;
        }

        let mutations = rows
            .into_iter()
            .map(|row| {
                let (namespace, workflow_id, run_id) = (row.namespace, row.workflow_id, row.run_id);
                match kind {
                    MutationKind::Terminate => Mutation::Terminate {
                        namespace,
                        workflow_id,
                        run_id,
                        // A reason is required by the API and useful in the history. Editing
                        // it before confirming is not built; a default beats an empty string.
                        reason: "terminated from tmprl".into(),
                    },
                    MutationKind::Delete => Mutation::Delete {
                        namespace,
                        workflow_id,
                        run_id,
                    },
                    // Cancel, and the kinds already returned above.
                    _ => Mutation::Cancel {
                        namespace,
                        workflow_id,
                        run_id,
                    },
                }
            })
            .collect();
        self.confirm = Some(Confirm::batch(mutations));
    }

    /// Turn a typed signal or update name into a confirmation.
    pub(super) fn confirm_named(&mut self, kind: PromptKind, name: String) {
        let rows = self.target_workflows();
        if rows.is_empty() {
            return;
        }
        let mutations = rows
            .into_iter()
            .map(|row| {
                let (namespace, workflow_id, run_id) = (row.namespace, row.workflow_id, row.run_id);
                let name = name.clone();
                match kind {
                    PromptKind::Update => Mutation::Update {
                        namespace,
                        workflow_id,
                        run_id,
                        name,
                        input: None,
                    },
                    _ => Mutation::Signal {
                        namespace,
                        workflow_id,
                        run_id,
                        name,
                        input: None,
                    },
                }
            })
            .collect();
        self.confirm = Some(Confirm::batch(mutations));
    }

    /// Turn a typed backfill window into a confirmation.
    ///
    /// A bad range stops here with the parser's own message rather than reaching the server,
    /// which would answer with something less specific.
    pub(super) fn confirm_backfill(&mut self, entered: String) {
        let Some(row) = self.target_schedule() else {
            return;
        };
        match parse_backfill(&entered, now_ms()) {
            Ok((range, overlap)) => {
                self.confirm = Some(Confirm::new(Mutation::BackfillSchedule {
                    namespace: row.namespace,
                    schedule_id: row.schedule_id,
                    range,
                    overlap,
                }));
            }
            Err(e) => self.note = Some((e, Note::Warn)),
        }
    }

    /// Open the form that collects a new schedule.
    ///
    /// Unlike every other mutation this one has no target under the cursor: it is creating
    /// the thing, so it only needs a namespace to create it in.
    pub(super) fn open_new_schedule_form(&mut self) {
        self.form = Some(Form::new_schedule());
    }

    /// Keys while the form is up.
    ///
    /// It owns every key, so nothing bound elsewhere fires mid-edit and a literal `j` goes
    /// into the field rather than moving a cursor somewhere behind it.
    pub(super) fn form_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(form) = self.form.as_mut() else {
            return;
        };
        match chord.key {
            Key::Esc => {
                self.form = None;
                self.mode = Mode::Normal;
            }
            Key::Tab => form.next(),
            Key::BackTab => form.previous(),
            Key::Down => form.next(),
            Key::Up => form.previous(),
            Key::Enter => self.confirm_new_schedule(),
            // Backspace on an empty field moves back rather than closing the form: a form is
            // several fields deep, so losing all of them to one key would be a trap.
            // The guard does the deleting, as `prompt_key` and `picker_key` do.
            Key::Backspace if !form.backspace() => form.previous(),
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => form.push(c),
            _ => {}
        }
    }

    /// Validate the form and put the command in front of the reader.
    pub(super) fn confirm_new_schedule(&mut self) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        if let Some(label) = form.missing() {
            // Send them to the field rather than only naming it.
            form.focus(label);
            self.note = Some((format!("{label} is required"), Note::Warn));
            return;
        }
        let input = form.get("input");
        let mutation = Mutation::CreateSchedule {
            namespace: self
                .view
                .scope
                .first()
                .cloned()
                .unwrap_or_else(|| self.namespace.clone()),
            schedule_id: form.get("schedule id").to_string(),
            workflow_id: form.get("workflow id").to_string(),
            workflow_type: form.get("workflow type").to_string(),
            task_queue: form.get("task queue").to_string(),
            spec: form.get("spec").to_string(),
            input: (!input.is_empty()).then(|| input.to_string()),
        };
        self.form = None;
        self.confirm = Some(Confirm::new(mutation));
    }

    /// The event a reset would go back to: the last completed workflow task at or before the
    /// cursor. Resolved rather than demanded, because the workflow tasks the server needs
    /// are exactly the rows the outline folds away.
    pub(super) fn reset_target(&self) -> Option<i64> {
        if self.view.screen != Screen::History {
            return None;
        }
        let outline = self.view.history.value()?;
        let at = match outline.row_at(self.view.cursor)? {
            Row::Event { event, .. } => outline.event(event)?.id,
            Row::Group { group, .. } => *outline.group(group)?.events.last()?,
        };
        tmprl_core::history::reset_point(outline.events(), at)
    }

    /// Keys while a confirmation is up.
    ///
    /// This owns every key, so nothing bound elsewhere can act while a destructive action is
    /// pending. Enter is the only way forward and Esc is always a way out.
    pub(super) fn confirm_key(&mut self, chord: Chord) {
        use tmprl_core::Key;
        let Some(confirm) = self.confirm.as_mut() else {
            return;
        };
        match chord.key {
            Key::Esc => {
                self.confirm = None;
                self.note = Some(("cancelled".into(), Note::Info));
            }
            Key::Enter if confirm.is_satisfied() => {
                let mutations = std::mem::take(&mut confirm.mutations);
                self.confirm = None;
                // The rows it covered are about to change or disappear, so the selection
                // that named them is spent. Leaving it up would invite a second batch over
                // a range that no longer means what it did.
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.run_mutations(mutations);
            }
            // Enter with the word unfinished is not a refusal, just not yet.
            Key::Enter => {}
            Key::Backspace => {
                confirm.entered.pop();
            }
            Key::Char(c) if chord.mods.is_none() => confirm.entered.push(c),
            _ => {}
        }
    }

    /// Send it. Spawned like every other RPC, a mutation must not freeze a keystroke either.
    /// Carry out every mutation a confirmation covered.
    ///
    /// One after another rather than all at once: the fan-out paging bug taught that a
    /// request count scaling with the size of the set is how a namespace rate limit is hit,
    /// and a batch the reader selected by hand is never large enough for the latency to
    /// matter. Each result comes back as its own `Mutated`, so a failure halfway through is
    /// reported for the row it happened on rather than sinking the whole set.
    pub(super) fn run_mutations(&mut self, mutations: Vec<Mutation>) {
        if self.refuses_mutation() {
            return;
        }
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let Some(first) = mutations.first() else {
            return;
        };
        let total = mutations.len();
        self.note = Some((
            if total > 1 {
                format!("{} {total}…", first.verb().to_lowercase())
            } else {
                format!("{}…", first.verb().to_lowercase())
            },
            Note::Info,
        ));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            for (i, mutation) in mutations.into_iter().enumerate() {
                let result = conn.mutate(&mutation).await.map_err(|e| e.to_string());
                let _ = tx.send(Msg::Mutated {
                    mutation: Box::new(mutation),
                    result,
                    batch: (total > 1).then_some((i + 1, total)),
                });
            }
        });
    }

    /// Refuse a mutation on a read-only profile, and say which profile refused it.
    ///
    /// Checked at both ends: here, so a refusal costs one keystroke rather than a typed
    /// signal payload, and again in `run_mutation`, which is the only path to the wire.
    pub(super) fn refuses_mutation(&mut self) -> bool {
        if self.readonly {
            self.note = Some((format!("profile {} is read-only", self.profile), Note::Warn));
            return true;
        }
        false
    }

    /// Record what was attempted, whether or not it worked.
    ///
    /// Appended, never rewritten, and failures go in too: the log is what was *attempted*,
    /// which is the question being asked when someone reads it.
    pub(super) fn audit(&mut self, mutation: &Mutation, outcome: &str) {
        let target = tmprl_core::mutation::Target {
            profile: &self.profile,
            address: &self.address,
        };
        if let Err(e) = crate::config::append_audit(&mutation.audit_line(now_ms(), target, outcome))
        {
            // A failed audit write must not be silent: the log is the record that an
            // irreversible thing happened.
            self.note = Some((format!("audit log: {e}"), Note::Error));
        }
    }
}
