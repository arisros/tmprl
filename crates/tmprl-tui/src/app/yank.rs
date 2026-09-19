//! Yanking: the field, the row as JSON, and the payloads under the cursor.

use super::*;

impl App {
    pub(super) fn field_under_cursor(&self) -> String {
        match self.view.screen {
            Screen::Namespaces => self
                .namespace_rows()
                .get(self.view.cursor)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            // The workflow id is the field you actually want to paste into a CLI command.
            Screen::Workflows => self
                .workflow_rows()
                .get(self.view.cursor)
                .map(|w| w.workflow_id.clone())
                .unwrap_or_default(),
            Screen::History => self.history_field_under_cursor(),
            Screen::Schedules => self
                .view
                .schedule_rows()
                .get(self.view.cursor)
                .map(|s| s.schedule_id.clone())
                .unwrap_or_default(),
        }
    }

    /// On a group line, the thing it is about; on an event line, the event's own name. Both
    /// are what you would paste into a search or a CLI command.
    pub(super) fn history_field_under_cursor(&self) -> String {
        let Some(outline) = self.view.history.value() else {
            return String::new();
        };
        match outline.row_at(self.view.cursor) {
            Some(Row::Group { group, .. }) => outline
                .group(group)
                .map(|g| {
                    if g.subject.is_empty() {
                        format!("{:?}", g.category)
                    } else {
                        g.subject.clone()
                    }
                })
                .unwrap_or_default(),
            Some(Row::Event { event, .. }) => outline
                .event(event)
                .map(|e| e.name.to_string())
                .unwrap_or_default(),
            None => String::new(),
        }
    }

    /// The selected rows as JSON, or just the row under the cursor when nothing is selected.
    pub(super) fn records_selected(&self) -> String {
        let (lo, hi) = self
            .selection()
            .unwrap_or((self.view.cursor, self.view.cursor));
        let take = hi.saturating_sub(lo) + 1;
        let picked: Vec<String> = match self.view.screen {
            Screen::Namespaces => self
                .namespace_rows()
                .iter()
                .skip(lo)
                .take(take)
                .map(|n| {
                    format!(
                        r#"{{"name":{},"state":{},"retentionDays":{}}}"#,
                        json_string(&n.name),
                        json_string(&n.state),
                        n.retention_days
                    )
                })
                .collect(),
            Screen::Workflows => self
                .workflow_rows()
                .iter()
                .skip(lo)
                .take(take)
                .map(|w| {
                    format!(
                        r#"{{"namespace":{},"workflowId":{},"runId":{},"type":{},"taskQueue":{},"status":{},"historyLength":{}}}"#,
                        json_string(&w.namespace),
                        json_string(&w.workflow_id),
                        json_string(&w.run_id),
                        json_string(&w.workflow_type),
                        json_string(&w.task_queue),
                        json_string(w.status.query_name()),
                        w.history_length
                    )
                })
                .collect(),
            Screen::History => self.history_records(lo, take),
            Screen::Schedules => self
                .view
                .schedule_rows()
                .iter()
                .skip(lo)
                .take(take)
                .map(|s| {
                    format!(
                        r#"{{"namespace":{},"scheduleId":{},"workflowType":{},"paused":{},"spec":{}}}"#,
                        json_string(&s.namespace),
                        json_string(&s.schedule_id),
                        json_string(&s.workflow_type),
                        s.paused,
                        json_string(&s.spec)
                    )
                })
                .collect(),
        };
        match picked.len() {
            0 => String::new(),
            1 => picked.into_iter().next().unwrap(),
            _ => format!("[{}]", picked.join(",")),
        }
    }

    /// The selected history rows as JSON. A group serialises as the summary the compact
    /// view shows; an event as its own fields.
    pub(super) fn history_records(&self, lo: usize, take: usize) -> Vec<String> {
        let Some(outline) = self.view.history.value() else {
            return Vec::new();
        };
        (lo..lo.saturating_add(take))
            .map_while(|r| outline.row_at(r))
            .filter_map(|row| match row {
                Row::Group { group, .. } => outline.group(group).map(|g| {
                    format!(
                        r#"{{"group":{},"category":{},"outcome":{},"attempts":{},"events":{}}}"#,
                        json_string(&g.subject),
                        json_string(&format!("{:?}", g.category)),
                        json_string(g.outcome.label()),
                        g.attempts,
                        g.events.len()
                    )
                }),
                Row::Event { event, .. } => outline.event(event).map(|e| {
                    format!(
                        r#"{{"eventId":{},"event":{},"subject":{}}}"#,
                        e.id,
                        json_string(e.name),
                        json_string(&e.subject)
                    )
                }),
            })
            .collect()
    }

    /// Yank the payloads under the cursor, or the subset `part` names.
    ///
    /// A single match is yanked unwrapped: `<leader>yr` on an ordinary activity should give
    /// the result itself, ready to paste, not `{"result": …}`. Several are yanked as the
    /// keyed object, because then the labels are what tells `input[0]` from `input[1]`.
    pub(super) fn yank_payload(&mut self, part: PayloadPart) {
        if self.view.screen != Screen::History {
            self.note = Some(("payloads are on a workflow history".into(), Note::Warn));
            return;
        }
        let picked: Vec<_> = self
            .payloads_under_cursor()
            .into_iter()
            .filter(|(label, _)| match part {
                PayloadPart::All => true,
                // `input`, and `input[0]`, `input[1]` … when the activity took several.
                PayloadPart::Input => label == "input" || label.starts_with("input["),
                PayloadPart::Result => label == "result" || label.starts_with("result["),
            })
            .collect();

        if picked.is_empty() {
            self.note = Some((
                match part {
                    PayloadPart::All => "nothing to yank here".into(),
                    PayloadPart::Input => "no input on this row".to_string(),
                    PayloadPart::Result => "no result on this row".to_string(),
                },
                Note::Warn,
            ));
            return;
        }

        let (json, skipped) = tmprl_core::payload::payloads_as_json(&picked);
        let Some(json) = json else {
            // Every match was encrypted or binary. Yanking ciphertext would look like it
            // worked, which is worse than saying why it did not.
            self.note = Some((
                format!("cannot yank: {} is not decoded text", skipped.join(", ")),
                Note::Warn,
            ));
            return;
        };

        let text = if picked.len() == 1 {
            tmprl_core::payload::unwrap_single(&json).unwrap_or(json)
        } else {
            json
        };
        self.yank(text);
        if !skipped.is_empty()
            && let Some((note, _)) = self.note.as_mut()
        {
            note.push_str(&format!(" ({} skipped)", skipped.join(", ")));
        }
    }

    pub(super) fn yank(&mut self, text: String) {
        if text.is_empty() {
            self.note = Some(("nothing to yank".into(), Note::Warn));
            return;
        }
        let n = text.len();
        match crate::clipboard::yank(&text) {
            Ok(()) => {
                self.note = Some((format!("yanked {n} bytes to clipboard"), Note::Info));
                self.view.anchor = None;
                self.mode = Mode::Normal;
            }
            Err(e) => self.note = Some((format!("yank failed: {e}"), Note::Error)),
        }
    }
}

/// Minimal JSON string escaping, enough for the identifiers and enum names yanked today.
/// A real serializer arrives with payload rendering in M2.
pub(super) fn json_string(s: &str) -> String {
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
