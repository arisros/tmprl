//! Things that change a cluster, and the confirmation that stands in front of them.
//!
//! Everything up to now has been a reader. These are the operations that cannot be undone by
//! pressing `R`, so the design in `docs/ARCHITECTURE.md` §9 puts one confirmation in front of
//! every one of them, and that confirmation shows **the equivalent `temporal` CLI command**.
//!
//! That last part is the load-bearing bit. It teaches the CLI, it makes the action auditable
//! at a glance, you can read exactly what is about to happen rather than trusting a verb,
//! and it gives an escape hatch to anyone who would rather run it themselves. Which means the
//! rendered command has to be *correct*: someone will copy it and run it. The flags here are
//! checked against `temporal workflow --help`, and the quoting is tested.

/// A change to a cluster, fully specified.
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
        /// JSON, as typed. `None` sends no input at all, which is not the same as `null`.
        input: Option<String>,
    },
    Delete {
        namespace: String,
        workflow_id: String,
        run_id: String,
    },
    /// Rewind to a completed workflow task and replay forward from there. The event id is
    /// already resolved to a valid reset point. See `history::reset_point`.
    Reset {
        namespace: String,
        workflow_id: String,
        run_id: String,
        event_id: i64,
        reason: String,
    },
    /// Send an update and wait for its outcome. Unlike a signal, an update can be rejected
    /// by the workflow and reports back.
    Update {
        namespace: String,
        workflow_id: String,
        run_id: String,
        name: String,
        input: Option<String>,
    },
}

impl Mutation {
    /// What the confirmation calls it.
    pub fn verb(&self) -> &'static str {
        match self {
            Mutation::Cancel { .. } => "Cancel",
            Mutation::Terminate { .. } => "Terminate",
            Mutation::Signal { .. } => "Signal",
            Mutation::Delete { .. } => "Delete",
            Mutation::Reset { .. } => "Reset",
            Mutation::Update { .. } => "Update",
        }
    }

    /// How to say it happened. Spelled out rather than derived: "cancel" + "d" is "canceld",
    /// and a status line that cannot spell does not inspire confidence in what it just did.
    pub fn past_tense(&self) -> &'static str {
        match self {
            Mutation::Cancel { .. } => "cancelled",
            Mutation::Terminate { .. } => "terminated",
            Mutation::Signal { .. } => "signalled",
            Mutation::Delete { .. } => "deleted",
            Mutation::Reset { .. } => "reset",
            Mutation::Update { .. } => "updated",
        }
    }

    pub fn namespace(&self) -> &str {
        match self {
            Mutation::Cancel { namespace, .. }
            | Mutation::Terminate { namespace, .. }
            | Mutation::Signal { namespace, .. }
            | Mutation::Delete { namespace, .. }
            | Mutation::Reset { namespace, .. }
            | Mutation::Update { namespace, .. } => namespace,
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
        }
    }

    /// Whether the workflow does not survive it.
    ///
    /// A signal is a change but not a loss; a delete removes the history itself and cannot be
    /// walked back even by starting the workflow again.
    pub fn is_destructive(&self) -> bool {
        // A reset abandons everything after the reset point, so it loses work even though
        // the workflow survives. An update, like a signal, only adds to it.
        !matches!(self, Mutation::Signal { .. } | Mutation::Update { .. })
    }

    /// Whether it destroys the record as well as the run. Only `delete` does, which is why it
    /// is the one that asks for more than a keypress.
    pub fn destroys_history(&self) -> bool {
        matches!(self, Mutation::Delete { .. })
    }

    /// The equivalent `temporal` command, ready to paste into a shell.
    ///
    /// Flags follow `temporal workflow --help`: `-w/--workflow-id`, `-r/--run-id`,
    /// `-n/--namespace`, `--reason`, `--name`, `--input`. The long forms are used because
    /// this is meant to be read as much as run.
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
            // `update execute` rather than `update start`: tmprl waits for the outcome, and
            // the command shown should be the one that behaves the same way.
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
        }
    }

    /// One line for `~/.local/state/tmprl/audit.jsonl`.
    ///
    /// The CLI equivalent is recorded alongside the fields, so the log answers "what was
    /// actually done" without the reader having to reconstruct it from parts.
    pub fn audit_line(&self, at_epoch_millis: i64, outcome: &str) -> String {
        let mut out = String::from("{");
        out.push_str(&format!(r#""at":{at_epoch_millis},"#));
        out.push_str(&format!(r#""action":{},"#, json_string(self.verb())));
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

/// What a confirmation is waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub mutation: Mutation,
    /// When set, the reader must type this exactly before the action is allowed. Reserved for
    /// the operations where a single keypress is too cheap for what it does.
    pub typed_word: Option<String>,
    /// What they have typed so far.
    pub entered: String,
}

impl Confirm {
    pub fn new(mutation: Mutation) -> Self {
        // Deleting destroys the history itself, so it costs a word rather than a keypress.
        // Everything else is one confirmation, as §9 says.
        let typed_word = mutation.destroys_history().then(|| "delete".to_string());
        Self {
            mutation,
            typed_word,
            entered: String::new(),
        }
    }

    /// Whether pressing Enter now would go ahead.
    pub fn is_satisfied(&self) -> bool {
        match &self.typed_word {
            None => true,
            Some(word) => self.entered.trim() == word,
        }
    }

    /// What to tell the reader they still owe.
    pub fn prompt(&self) -> String {
        match &self.typed_word {
            None => "⏎ to confirm   Esc to cancel".into(),
            Some(word) if self.is_satisfied() => "⏎ to confirm   Esc to cancel".into(),
            Some(word) => format!("type `{word}` to confirm   Esc to cancel"),
        }
    }
}

/// Quote a value for a POSIX shell.
///
/// Single quotes, with the one escape a single-quoted string needs. This matters more than it
/// looks: the rendered command is meant to be copied and run, and a workflow id can contain
/// spaces, quotes or a `;`. A command that is *almost* right is worse than none.
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
        // `--input ''` is a JSON parse error, not "no input".
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
        // Someone will copy this and run it. A workflow id with a space or a semicolon must
        // not turn into two commands.
        let m = Mutation::Terminate {
            namespace: "default".into(),
            workflow_id: "order 1; rm -rf /".into(),
            run_id: "r".into(),
            reason: "it's stuck".into(),
        };
        let cli = m.cli();
        assert!(cli.contains("'order 1; rm -rf /'"), "got {cli}");
        // The one escape a single-quoted shell string needs.
        assert!(cli.contains(r"'it'\''s stuck'"), "got {cli}");
    }

    #[test]
    fn plain_values_are_not_quoted_needlessly() {
        // The command is meant to be read as much as run.
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
        // A reset abandons everything after the point, so it loses work even though the
        // workflow survives.
        assert!(m.is_destructive());
        assert!(!m.destroys_history(), "the history is still there");
        assert_eq!(Confirm::new(m).typed_word, None);
    }

    #[test]
    fn an_update_uses_execute_because_tmprl_waits_for_the_outcome() {
        // `update start` returns once accepted; `update execute` waits. The command shown
        // should behave the way tmprl does.
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
        // Like a signal, an update adds to a workflow rather than ending it.
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
        // "cancel" + "d" is "canceld".
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

    #[test]
    fn an_audit_line_records_what_was_done_and_how_it_ended() {
        let line = terminate("stuck").audit_line(1_700_000_000_000, "ok");
        assert!(line.contains(r#""action":"Terminate""#), "{line}");
        assert!(line.contains(r#""workflowId":"order-1""#), "{line}");
        assert!(line.contains(r#""outcome":"ok""#), "{line}");
        assert!(line.contains(r#""at":1700000000000"#), "{line}");
        // The command goes in too, so the log answers "what happened" on its own.
        assert!(line.contains("temporal workflow terminate"), "{line}");
        // And it is one line, because the file is JSONL.
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
        let line = m.audit_line(0, "failed");
        assert!(
            !line.contains('\n'),
            "a JSONL line cannot contain a newline"
        );
        assert!(line.contains(r#"\"quotes\""#), "{line}");
    }

    #[test]
    fn a_failed_mutation_is_still_recorded() {
        // The log is what was attempted, not only what succeeded.
        let line = terminate("x").audit_line(1, "failed: permission denied");
        assert!(line.contains("permission denied"), "{line}");
    }
}
