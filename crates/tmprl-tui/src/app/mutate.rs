//! Mutations: choosing the target, the confirmation, and running them.

use super::*;

impl App {
    pub(super) fn target_schedule(&self) -> Option<ScheduleRow> {
        if self.view.screen != Screen::Schedules {
            return None;
        }
        self.view.schedule_rows().get(self.view.cursor).cloned()
    }

    pub(super) fn target_workflows(&self) -> Vec<WorkflowRow> {
        if self.view.screen == Screen::Workflows
            && let Some((lo, hi)) = self.view.selection()
        {
            let rows = self.view.workflow_rows();
            return rows[lo.min(rows.len())..(hi + 1).min(rows.len())].to_vec();
        }
        self.target_workflow().into_iter().collect()
    }

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
            Screen::Profiles | Screen::Namespaces | Screen::Schedules | Screen::Dashboard => None,
        }
    }

    pub(super) fn confirm_mutation(&mut self, kind: MutationKind) {
        if self.refuses_mutation() {
            return;
        }
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
                self.prompt = Some(Prompt {
                    kind: PromptKind::Backfill,
                    buf: LineEdit::default(),
                });
                self.mode = Mode::Command;
                return;
            }
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
                buf: LineEdit::default(),
            });
            self.mode = Mode::Command;
            return;
        }

        if matches!(kind, MutationKind::Reset) {
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
            self.ask_reason(
                vec![Mutation::Reset {
                    namespace: row.namespace,
                    workflow_id: row.workflow_id,
                    run_id: row.run_id,
                    event_id,
                    reason: String::new(),
                }],
                "reset from tmprl",
            );
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
                        reason: String::new(),
                    },
                    MutationKind::Delete => Mutation::Delete {
                        namespace,
                        workflow_id,
                        run_id,
                    },
                    _ => Mutation::Cancel {
                        namespace,
                        workflow_id,
                        run_id,
                    },
                }
            })
            .collect::<Vec<_>>();
        if matches!(kind, MutationKind::Terminate) {
            self.ask_reason(mutations, "terminated from tmprl");
            return;
        }
        self.confirm = Some(Confirm::batch(mutations));
    }

    fn ask_reason(&mut self, mutations: Vec<Mutation>, default: &str) {
        self.awaiting_reason = mutations;
        self.prompt = Some(Prompt {
            kind: PromptKind::Reason,
            buf: LineEdit::from(default),
        });
        self.mode = Mode::Command;
    }

    pub(super) fn confirm_with_reason(&mut self, reason: String) {
        let mut mutations = std::mem::take(&mut self.awaiting_reason);
        for m in &mut mutations {
            if let Some(r) = m.reason_mut() {
                *r = reason.clone();
            }
        }
        if !mutations.is_empty() {
            self.confirm = Some(Confirm::batch(mutations));
        }
    }

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

    pub(super) fn open_new_schedule_form(&mut self) {
        if self.refuses_mutation() {
            return;
        }
        self.form = Some(Form::new_schedule());
    }

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
            Key::Backspace if !form.backspace() => form.previous(),
            Key::Backspace => {}
            Key::Char(c) if chord.mods.is_none() => form.push(c),
            _ => {}
        }
    }

    pub(super) fn confirm_new_schedule(&mut self) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        if let Some(label) = form.missing() {
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
                self.view.anchor = None;
                self.mode = Mode::Normal;
                self.run_mutations(mutations);
            }
            Key::Enter => {}
            Key::Backspace => {
                confirm.entered.pop();
            }
            Key::Char(c) if chord.mods.is_none() => confirm.entered.push(c),
            _ => {}
        }
    }

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
        let sent_to = (self.profile.clone(), self.address.clone());
        tokio::spawn(async move {
            for (i, mutation) in mutations.into_iter().enumerate() {
                let result = conn.mutate(&mutation).await;
                let _ = tx.send(Msg::Mutated {
                    mutation: Box::new(mutation),
                    sent_to: sent_to.clone(),
                    result,
                    batch: (total > 1).then_some((i + 1, total)),
                });
            }
        });
    }

    pub(super) fn refuses_mutation(&mut self) -> bool {
        if !self.readonly() {
            return false;
        }
        let why = if self.readonly {
            format!("profile {} is read-only", self.profile)
        } else {
            "read-only: tmprl was started with --readonly".to_string()
        };
        self.note = Some((why, Note::Warn));
        true
    }

    pub(super) fn audit_to(
        &mut self,
        mutation: &Mutation,
        outcome: &str,
        sent_to: &(String, String),
    ) {
        let target = tmprl_core::mutation::Target {
            profile: &sent_to.0,
            address: &sent_to.1,
        };
        let line = mutation.audit_line(now_ms(), target, outcome);
        match &mut self.audit_sink {
            AuditSink::File => {
                if let Err(e) = crate::config::append_audit(&line) {
                    self.note = Some((format!("audit log: {e}"), Note::Error));
                }
            }
            #[cfg(test)]
            AuditSink::Captured(lines) => lines.push(line),
        }
    }
}
