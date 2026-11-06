//! Things that change a cluster, and the confirmation that stands in front of them.

use crate::timerange::{Overlap, TimeRange, to_rfc3339};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    Cancel {
        namespace: String,
        workflow_id: String,
        run_id: String,
    },
    Terminate {
        namespace: String,
        workflow_id: String,
        run_id: String,
        reason: String,
    },
    Signal {
        namespace: String,
        workflow_id: String,
        run_id: String,
        name: String,
        input: Option<String>,
    },
    Delete {
        namespace: String,
        workflow_id: String,
        run_id: String,
    },
    Reset {
        namespace: String,
        workflow_id: String,
        run_id: String,
        event_id: i64,
        reason: String,
    },
    Update {
        namespace: String,
        workflow_id: String,
        run_id: String,
        name: String,
        input: Option<String>,
    },
    PauseSchedule {
        namespace: String,
        schedule_id: String,
        paused: bool,
    },
    TriggerSchedule {
        namespace: String,
        schedule_id: String,
    },
    DeleteSchedule {
        namespace: String,
        schedule_id: String,
    },
    CreateSchedule {
        namespace: String,
        schedule_id: String,
        workflow_id: String,
        workflow_type: String,
        task_queue: String,
        spec: String,
        input: Option<String>,
    },
    BackfillSchedule {
        namespace: String,
        schedule_id: String,
        range: TimeRange,
        overlap: Overlap,
    },
}

impl Mutation {
    pub fn schedule_id(&self) -> Option<&str> {
        match self {
            Mutation::PauseSchedule { schedule_id, .. }
            | Mutation::TriggerSchedule { schedule_id, .. }
            | Mutation::DeleteSchedule { schedule_id, .. }
            | Mutation::BackfillSchedule { schedule_id, .. }
            | Mutation::CreateSchedule { schedule_id, .. } => Some(schedule_id),
            _ => None,
        }
    }
}

impl Mutation {
    pub fn reason_mut(&mut self) -> Option<&mut String> {
        match self {
            Mutation::Terminate { reason, .. } | Mutation::Reset { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Mutation::Cancel { .. } => "Cancel",
            Mutation::Terminate { .. } => "Terminate",
            Mutation::Signal { .. } => "Signal",
            Mutation::Delete { .. } => "Delete",
            Mutation::Reset { .. } => "Reset",
            Mutation::Update { .. } => "Update",
            Mutation::PauseSchedule { paused: true, .. } => "Pause",
            Mutation::PauseSchedule { paused: false, .. } => "Resume",
            Mutation::TriggerSchedule { .. } => "Trigger",
            Mutation::DeleteSchedule { .. } => "Delete schedule",
            Mutation::BackfillSchedule { .. } => "Backfill",
            Mutation::CreateSchedule { .. } => "Create schedule",
        }
    }

    pub fn past_tense(&self) -> &'static str {
        match self {
            Mutation::Cancel { .. } => "cancelled",
            Mutation::Terminate { .. } => "terminated",
            Mutation::Signal { .. } => "signalled",
            Mutation::Delete { .. } => "deleted",
            Mutation::Reset { .. } => "reset",
            Mutation::Update { .. } => "updated",
            Mutation::PauseSchedule { paused: true, .. } => "paused",
            Mutation::PauseSchedule { paused: false, .. } => "resumed",
            Mutation::TriggerSchedule { .. } => "triggered",
            Mutation::DeleteSchedule { .. } => "deleted",
            Mutation::BackfillSchedule { .. } => "backfilled",
            Mutation::CreateSchedule { .. } => "created",
        }
    }

    pub fn subject_plural(&self) -> &'static str {
        match self {
            Mutation::Cancel { .. }
            | Mutation::Terminate { .. }
            | Mutation::Signal { .. }
            | Mutation::Delete { .. }
            | Mutation::Reset { .. }
            | Mutation::Update { .. } => "workflows",
            Mutation::PauseSchedule { .. }
            | Mutation::TriggerSchedule { .. }
            | Mutation::DeleteSchedule { .. }
            | Mutation::BackfillSchedule { .. }
            | Mutation::CreateSchedule { .. } => "schedules",
        }
    }

    pub fn namespace(&self) -> &str {
        match self {
            Mutation::Cancel { namespace, .. }
            | Mutation::Terminate { namespace, .. }
            | Mutation::Signal { namespace, .. }
            | Mutation::Delete { namespace, .. }
            | Mutation::Reset { namespace, .. }
            | Mutation::Update { namespace, .. }
            | Mutation::PauseSchedule { namespace, .. }
            | Mutation::TriggerSchedule { namespace, .. }
            | Mutation::DeleteSchedule { namespace, .. }
            | Mutation::BackfillSchedule { namespace, .. }
            | Mutation::CreateSchedule { namespace, .. } => namespace,
        }
    }

    pub fn workflow_id(&self) -> &str {
        match self {
            Mutation::Cancel { workflow_id, .. }
            | Mutation::Terminate { workflow_id, .. }
            | Mutation::Signal { workflow_id, .. }
            | Mutation::Delete { workflow_id, .. }
            | Mutation::Reset { workflow_id, .. }
            | Mutation::Update { workflow_id, .. } => workflow_id,
            Mutation::PauseSchedule { schedule_id, .. }
            | Mutation::TriggerSchedule { schedule_id, .. }
            | Mutation::DeleteSchedule { schedule_id, .. }
            | Mutation::BackfillSchedule { schedule_id, .. }
            | Mutation::CreateSchedule { schedule_id, .. } => schedule_id,
        }
    }

    pub fn run_id(&self) -> &str {
        match self {
            Mutation::Cancel { run_id, .. }
            | Mutation::Terminate { run_id, .. }
            | Mutation::Signal { run_id, .. }
            | Mutation::Delete { run_id, .. }
            | Mutation::Reset { run_id, .. }
            | Mutation::Update { run_id, .. } => run_id,
            Mutation::PauseSchedule { .. }
            | Mutation::TriggerSchedule { .. }
            | Mutation::DeleteSchedule { .. }
            | Mutation::BackfillSchedule { .. }
            | Mutation::CreateSchedule { .. } => "",
        }
    }

    pub fn is_destructive(&self) -> bool {
        !matches!(
            self,
            Mutation::Signal { .. }
                | Mutation::Update { .. }
                | Mutation::PauseSchedule { .. }
                | Mutation::TriggerSchedule { .. }
                | Mutation::BackfillSchedule { .. }
                | Mutation::CreateSchedule { .. }
        )
    }

    pub fn destroys_history(&self) -> bool {
        matches!(
            self,
            Mutation::Delete { .. } | Mutation::DeleteSchedule { .. }
        )
    }

    pub fn cli(&self) -> String {
        let base = |verb: &str, m: &Mutation| {
            format!(
                "temporal workflow {verb} --namespace {} --workflow-id {} --run-id {}",
                shell_quote(m.namespace()),
                shell_quote(m.workflow_id()),
                shell_quote(m.run_id()),
            )
        };
        match self {
            Mutation::Cancel { .. } => base("cancel", self),
            Mutation::Delete { .. } => base("delete", self),
            Mutation::Terminate { reason, .. } => {
                format!(
                    "{} --reason {}",
                    base("terminate", self),
                    shell_quote(reason)
                )
            }
            Mutation::Signal { name, input, .. } => {
                let mut out = format!("{} --name {}", base("signal", self), shell_quote(name));
                if let Some(input) = input {
                    out.push_str(&format!(" --input {}", shell_quote(input)));
                }
                out
            }
            Mutation::Reset {
                event_id, reason, ..
            } => format!(
                "{} --event-id {event_id} --reason {}",
                base("reset", self),
                shell_quote(reason)
            ),
            Mutation::Update { name, input, .. } => {
                let mut out = format!(
                    "temporal workflow update execute --namespace {} --workflow-id {} \
                     --run-id {} --name {}",
                    shell_quote(self.namespace()),
                    shell_quote(self.workflow_id()),
                    shell_quote(self.run_id()),
                    shell_quote(name)
                );
                if let Some(input) = input {
                    out.push_str(&format!(" --input {}", shell_quote(input)));
                }
                out
            }
            Mutation::PauseSchedule {
                namespace,
                schedule_id,
                paused,
            } => format!(
                "temporal schedule toggle --namespace {} --schedule-id {} {}",
                shell_quote(namespace),
                shell_quote(schedule_id),
                if *paused { "--pause" } else { "--unpause" }
            ),
            Mutation::TriggerSchedule {
                namespace,
                schedule_id,
            } => format!(
                "temporal schedule trigger --namespace {} --schedule-id {}",
                shell_quote(namespace),
                shell_quote(schedule_id)
            ),
            Mutation::DeleteSchedule {
                namespace,
                schedule_id,
            } => format!(
                "temporal schedule delete --namespace {} --schedule-id {}",
                shell_quote(namespace),
                shell_quote(schedule_id)
            ),
            Mutation::CreateSchedule {
                namespace,
                schedule_id,
                workflow_id,
                workflow_type,
                task_queue,
                spec,
                input,
            } => {
                let mut out = format!(
                    "temporal schedule create --namespace {} --schedule-id {} \
                     --workflow-id {} --type {} --task-queue {} --cron {}",
                    shell_quote(namespace),
                    shell_quote(schedule_id),
                    shell_quote(workflow_id),
                    shell_quote(workflow_type),
                    shell_quote(task_queue),
                    shell_quote(spec)
                );
                if let Some(input) = input {
                    out.push_str(&format!(" --input {}", shell_quote(input)));
                }
                out
            }
            Mutation::BackfillSchedule {
                namespace,
                schedule_id,
                range,
                overlap,
            } => format!(
                "temporal schedule backfill --namespace {} --schedule-id {} \
                 --start-time {} --end-time {} --overlap-policy {}",
                shell_quote(namespace),
                shell_quote(schedule_id),
                to_rfc3339(range.start_ms),
                to_rfc3339(range.end_ms),
                overlap.name()
            ),
        }
    }

    pub fn audit_line(&self, at_epoch_millis: i64, target: Target<'_>, outcome: &str) -> String {
        let mut out = String::from("{");
        out.push_str(&format!(r#""at":{at_epoch_millis},"#));
        out.push_str(&format!(r#""action":{},"#, json_string(self.verb())));
        out.push_str(&format!(r#""profile":{},"#, json_string(target.profile)));
        out.push_str(&format!(r#""address":{},"#, json_string(target.address)));
        out.push_str(&format!(
            r#""namespace":{},"#,
            json_string(self.namespace())
        ));
        out.push_str(&format!(
            r#""workflowId":{},"#,
            json_string(self.workflow_id())
        ));
        out.push_str(&format!(r#""runId":{},"#, json_string(self.run_id())));
        out.push_str(&format!(r#""outcome":{},"#, json_string(outcome)));
        out.push_str(&format!(r#""command":{}"#, json_string(&self.cli())));
        out.push('}');
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target<'a> {
    pub profile: &'a str,
    pub address: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub mutations: Vec<Mutation>,
    pub typed_word: Option<String>,
    pub entered: String,
}

impl Confirm {
    pub fn new(mutation: Mutation) -> Self {
        Self::batch(vec![mutation])
    }

    pub fn batch(mutations: Vec<Mutation>) -> Self {
        let typed_word = if mutations.iter().any(Mutation::destroys_history) {
            Some("delete".to_string())
        } else if mutations.len() > 1 && mutations.iter().any(Mutation::is_destructive) {
            Some(mutations.len().to_string())
        } else {
            None
        };
        Self {
            mutations,
            typed_word,
            entered: String::new(),
        }
    }

    pub fn first(&self) -> &Mutation {
        &self.mutations[0]
    }

    pub fn len(&self) -> usize {
        self.mutations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    pub fn is_batch(&self) -> bool {
        self.mutations.len() > 1
    }

    pub fn is_satisfied(&self) -> bool {
        match &self.typed_word {
            None => true,
            Some(word) => self.entered.trim() == word,
        }
    }

    pub fn caution(&self) -> Option<String> {
        self.typed_word.as_ref()?;
        Some(if self.first().destroys_history() {
            "this destroys the history itself.".to_string()
        } else {
            format!(
                "this covers {} {}.",
                self.len(),
                self.first().subject_plural()
            )
        })
    }

    pub fn prompt(&self) -> String {
        match &self.typed_word {
            None => "⏎ to confirm   Esc to cancel".into(),
            Some(word) if self.is_satisfied() => "⏎ to confirm   Esc to cancel".into(),
            Some(word) => format!("type `{word}` to confirm   Esc to cancel"),
        }
    }
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminate(reason: &str) -> Mutation {
        Mutation::Terminate {
            namespace: "default".into(),
            workflow_id: "order-1".into(),
            run_id: "run-abc".into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn the_caution_says_why_the_word_is_owed_rather_than_assuming_delete() {
        let wf = |id: &str| Mutation::Terminate {
            namespace: "default".into(),
            workflow_id: id.into(),
            run_id: "r".into(),
            reason: "r".into(),
        };
        let batch = Confirm::batch(vec![wf("a"), wf("b"), wf("c")]);
        assert_eq!(batch.typed_word.as_deref(), Some("3"));
        assert_eq!(batch.caution().as_deref(), Some("this covers 3 workflows."));

        let del = Confirm::new(Mutation::Delete {
            namespace: "default".into(),
            workflow_id: "a".into(),
            run_id: "r".into(),
        });
        assert_eq!(del.typed_word.as_deref(), Some("delete"));
        assert_eq!(
            del.caution().as_deref(),
            Some("this destroys the history itself.")
        );

        assert_eq!(
            Confirm::new(wf("a")).caution(),
            None,
            "one row owes nothing"
        );
    }

    #[test]
    fn a_backfill_renders_the_cli_with_rfc3339_bounds() {
        let m = Mutation::BackfillSchedule {
            namespace: "default".into(),
            schedule_id: "nightly recon".into(),
            range: TimeRange {
                start_ms: 1_788_566_400_000,
                end_ms: 1_788_652_800_000,
            },
            overlap: Overlap::BufferAll,
        };
        assert_eq!(
            m.cli(),
            "temporal schedule backfill --namespace default --schedule-id 'nightly recon' \
             --start-time 2026-09-05T00:00:00Z --end-time 2026-09-06T00:00:00Z \
             --overlap-policy BufferAll"
        );
        assert_eq!(m.verb(), "Backfill");
        assert_eq!(m.past_tense(), "backfilled");
        assert_eq!(m.schedule_id(), Some("nightly recon"));
        assert!(!m.is_destructive(), "it starts runs, it destroys nothing");
    }

    #[test]
    fn a_terminate_renders_the_command_that_would_do_it() {
        assert_eq!(
            terminate("stuck").cli(),
            "temporal workflow terminate --namespace default --workflow-id order-1 \
             --run-id run-abc --reason stuck"
        );
    }

    #[test]
    fn a_cancel_and_a_delete_carry_no_reason() {
        let cancel = Mutation::Cancel {
            namespace: "payments".into(),
            workflow_id: "charge-9".into(),
            run_id: "r1".into(),
        };
        assert_eq!(
            cancel.cli(),
            "temporal workflow cancel --namespace payments --workflow-id charge-9 --run-id r1"
        );
        let delete = Mutation::Delete {
            namespace: "payments".into(),
            workflow_id: "charge-9".into(),
            run_id: "r1".into(),
        };
        assert!(delete.cli().starts_with("temporal workflow delete "));
    }

    #[test]
    fn a_signal_without_input_does_not_pass_an_empty_one() {
        let signal = Mutation::Signal {
            namespace: "default".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
            name: "ping".into(),
            input: None,
        };
        assert!(!signal.cli().contains("--input"));

        let with_input = Mutation::Signal {
            namespace: "default".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
            name: "ping".into(),
            input: Some(r#"{"a":1}"#.into()),
        };
        assert!(with_input.cli().ends_with(r#"--input '{"a":1}'"#));
    }

    #[test]
    fn values_that_would_break_a_shell_are_quoted() {
        let m = Mutation::Terminate {
            namespace: "default".into(),
            workflow_id: "order 1; rm -rf /".into(),
            run_id: "r".into(),
            reason: "it's stuck".into(),
        };
        let cli = m.cli();
        assert!(cli.contains("'order 1; rm -rf /'"), "got {cli}");
        assert!(cli.contains(r"'it'\''s stuck'"), "got {cli}");
    }

    #[test]
    fn plain_values_are_not_quoted_needlessly() {
        assert_eq!(shell_quote("order-1"), "order-1");
        assert_eq!(shell_quote("ns.with.dots"), "ns.with.dots");
        assert_eq!(shell_quote(""), "''", "an empty value still needs quotes");
        assert_eq!(shell_quote("has space"), "'has space'");
    }

    #[test]
    fn a_reset_names_the_event_it_goes_back_to() {
        let m = Mutation::Reset {
            namespace: "default".into(),
            workflow_id: "order-1".into(),
            run_id: "r".into(),
            event_id: 9,
            reason: "bad deploy".into(),
        };
        assert_eq!(
            m.cli(),
            "temporal workflow reset --namespace default --workflow-id order-1 --run-id r \
             --event-id 9 --reason 'bad deploy'"
        );
        assert!(m.is_destructive());
        assert!(!m.destroys_history(), "the history is still there");
        assert_eq!(Confirm::new(m).typed_word, None);
    }

    #[test]
    fn an_update_uses_execute_because_tmprl_waits_for_the_outcome() {
        let m = Mutation::Update {
            namespace: "default".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
            name: "setLimit".into(),
            input: Some("50".into()),
        };
        let cli = m.cli();
        assert!(
            cli.starts_with("temporal workflow update execute "),
            "{cli}"
        );
        assert!(cli.ends_with("--name setLimit --input 50"), "{cli}");
        assert!(!m.is_destructive());
    }

    #[test]
    fn an_update_without_input_passes_none() {
        let m = Mutation::Update {
            namespace: "d".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
            name: "ping".into(),
            input: None,
        };
        assert!(!m.cli().contains("--input"));
    }

    #[test]
    fn every_verb_has_a_past_tense_that_is_a_word() {
        for (m, expected) in [
            (
                Mutation::Cancel {
                    namespace: "d".into(),
                    workflow_id: "w".into(),
                    run_id: "r".into(),
                },
                "cancelled",
            ),
            (terminate("x"), "terminated"),
            (
                Mutation::Reset {
                    namespace: "d".into(),
                    workflow_id: "w".into(),
                    run_id: "r".into(),
                    event_id: 1,
                    reason: "x".into(),
                },
                "reset",
            ),
        ] {
            assert_eq!(m.past_tense(), expected);
        }
    }

    #[test]
    fn pausing_a_schedule_renders_the_toggle_command() {
        let m = Mutation::PauseSchedule {
            namespace: "payments".into(),
            schedule_id: "nightly".into(),
            paused: true,
        };
        assert_eq!(
            m.cli(),
            "temporal schedule toggle --namespace payments --schedule-id nightly --pause"
        );
        assert_eq!(m.verb(), "Pause");
        assert_eq!(m.past_tense(), "paused");
        assert!(!m.is_destructive(), "unpausing puts it back");

        let resumed = Mutation::PauseSchedule {
            namespace: "payments".into(),
            schedule_id: "nightly".into(),
            paused: false,
        };
        assert!(resumed.cli().ends_with("--unpause"));
        assert_eq!(resumed.verb(), "Resume");
    }

    #[test]
    fn triggering_is_reversible_but_deleting_a_schedule_is_not() {
        let trigger = Mutation::TriggerSchedule {
            namespace: "d".into(),
            schedule_id: "s".into(),
        };
        assert!(!trigger.is_destructive());
        assert_eq!(
            trigger.cli(),
            "temporal schedule trigger --namespace d --schedule-id s"
        );

        let delete = Mutation::DeleteSchedule {
            namespace: "d".into(),
            schedule_id: "s".into(),
        };
        assert!(delete.is_destructive());
        assert!(delete.destroys_history(), "the schedule is gone for good");
        assert_eq!(Confirm::new(delete).typed_word.as_deref(), Some("delete"));
    }

    #[test]
    fn a_schedule_operation_reports_its_id_and_has_no_run() {
        let m = Mutation::TriggerSchedule {
            namespace: "d".into(),
            schedule_id: "nightly".into(),
        };
        assert_eq!(m.schedule_id(), Some("nightly"));
        assert_eq!(m.workflow_id(), "nightly", "the id column shows it");
        assert_eq!(m.run_id(), "", "a schedule has no execution");
        assert_eq!(terminate("x").schedule_id(), None);
    }

    #[test]
    fn a_signal_is_a_change_but_not_a_loss() {
        let signal = Mutation::Signal {
            namespace: "d".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
            name: "n".into(),
            input: None,
        };
        assert!(!signal.is_destructive());
        assert!(terminate("x").is_destructive());
    }

    #[test]
    fn only_delete_destroys_the_history_and_only_it_asks_for_a_word() {
        let delete = Mutation::Delete {
            namespace: "d".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
        };
        assert!(delete.destroys_history());
        assert!(!terminate("x").destroys_history());

        let mut confirm = Confirm::new(delete);
        assert_eq!(confirm.typed_word.as_deref(), Some("delete"));
        assert!(!confirm.is_satisfied(), "a keypress is too cheap for this");
        assert!(confirm.prompt().contains("type `delete`"));

        confirm.entered = "delet".into();
        assert!(!confirm.is_satisfied(), "nearly is not the same as typed");
        confirm.entered = "delete".into();
        assert!(confirm.is_satisfied());
        assert!(confirm.prompt().contains("⏎"));
    }

    #[test]
    fn everything_else_needs_one_confirmation_and_no_typing() {
        let confirm = Confirm::new(terminate("stuck"));
        assert_eq!(confirm.typed_word, None);
        assert!(confirm.is_satisfied());
        assert!(confirm.prompt().contains("⏎ to confirm"));
    }

    fn sit() -> Target<'static> {
        Target {
            profile: "sit",
            address: "http://temporal-sit.internal:7233",
        }
    }

    #[test]
    fn an_audit_line_records_what_was_done_and_how_it_ended() {
        let line = terminate("stuck").audit_line(1_700_000_000_000, sit(), "ok");
        assert!(line.contains(r#""action":"Terminate""#), "{line}");
        assert!(line.contains(r#""workflowId":"order-1""#), "{line}");
        assert!(line.contains(r#""outcome":"ok""#), "{line}");
        assert!(line.contains(r#""at":1700000000000"#), "{line}");
        assert!(line.contains("temporal workflow terminate"), "{line}");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn an_audit_line_escapes_what_it_quotes() {
        let m = Mutation::Terminate {
            namespace: "d".into(),
            workflow_id: "has \"quotes\" and\nnewline".into(),
            run_id: "r".into(),
            reason: "x".into(),
        };
        let line = m.audit_line(0, sit(), "failed");
        assert!(
            !line.contains('\n'),
            "a JSONL line cannot contain a newline"
        );
        assert!(line.contains(r#"\"quotes\""#), "{line}");
    }

    #[test]
    fn a_failed_mutation_is_still_recorded() {
        let line = terminate("x").audit_line(1, sit(), "failed: permission denied");
        assert!(line.contains("permission denied"), "{line}");
    }

    #[test]
    fn an_audit_line_records_which_cluster_was_hit() {
        let line = terminate("stuck").audit_line(1, sit(), "ok");
        assert!(line.contains(r#""profile":"sit""#), "{line}");
        assert!(
            line.contains(r#""address":"http://temporal-sit.internal:7233""#),
            "{line}"
        );

        let prod = Target {
            profile: "prod",
            address: "http://temporal.internal:7233",
        };
        let other = terminate("stuck").audit_line(1, prod, "ok");
        assert_ne!(
            line, other,
            "the same namespace on two clusters must differ"
        );
    }

    #[test]
    fn only_terminate_and_reset_carry_a_reason() {
        let mut t = Mutation::Terminate {
            namespace: "d".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
            reason: "old".into(),
        };
        *t.reason_mut().unwrap() = "bad deploy".into();
        assert!(matches!(t, Mutation::Terminate { ref reason, .. } if reason == "bad deploy"));

        let mut c = Mutation::Cancel {
            namespace: "d".into(),
            workflow_id: "w".into(),
            run_id: "r".into(),
        };
        assert!(c.reason_mut().is_none());
    }
}
