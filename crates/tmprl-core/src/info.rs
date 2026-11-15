//! A workflow's own state from `DescribeWorkflowExecution`, laid out as the info card.

use crate::payload::Payload;
use crate::pending::PendingActivity;
use crate::workflow::WorkflowStatus;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Execution {
    pub workflow_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingChild {
    pub workflow_id: String,
    pub run_id: String,
    pub workflow_type: String,
    pub initiated_id: i64,
    pub parent_close_policy: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingWorkflowTask {
    pub state: &'static str,
    pub scheduled_at: Option<i64>,
    pub started_at: Option<i64>,
    pub attempt: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowInfo {
    pub workflow_id: String,
    pub run_id: String,
    pub first_run_id: String,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub task_queue: String,
    pub start_time: Option<i64>,
    pub execution_time: Option<i64>,
    pub close_time: Option<i64>,
    pub history_length: i64,
    pub history_size_bytes: i64,
    pub execution_timeout_ms: Option<i64>,
    pub run_timeout_ms: Option<i64>,
    pub task_timeout_ms: Option<i64>,
    pub parent: Option<Execution>,
    pub root: Option<Execution>,
    pub memo: Vec<(String, Payload)>,
    pub search_attributes: Vec<(String, Payload)>,
    pub pending_children: Vec<PendingChild>,
    pub workflow_task: Option<PendingWorkflowTask>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Normal,
    Retrying,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub waiting: Vec<String>,
    pub tone: Tone,
    pub facts: Vec<(&'static str, String)>,
    pub wide: Vec<(&'static str, String)>,
    pub empty: Vec<&'static str>,
}

pub const MAX_LISTED: usize = 3;
const VALUE_WIDTH: usize = 120;

impl WorkflowInfo {
    pub fn card(
        &self,
        now_ms: i64,
        pending: &[PendingActivity],
        stamp: impl Fn(i64) -> String,
    ) -> Card {
        let (waiting, tone) = self.waiting_on(now_ms, pending);
        let mut facts = vec![("run", self.run_id.clone())];
        if !self.first_run_id.is_empty() && self.first_run_id != self.run_id {
            facts.push(("first run", self.first_run_id.clone()));
        }
        if let Some(start) = self.start_time {
            let age = match self.close_time {
                Some(_) => String::new(),
                None => format!(" ({} ago)", span(now_ms - start)),
            };
            facts.push(("started", format!("{}{age}", stamp(start))));
        }
        if let Some(at) = self
            .execution_time
            .filter(|&at| Some(at) != self.start_time)
        {
            facts.push(("first task", stamp(at)));
        }
        if let Some(close) = self.close_time {
            facts.push(("closed", stamp(close)));
        }
        let timeouts = self.timeouts();
        if !timeouts.is_empty() {
            facts.push(("timeouts", timeouts.join(" · ")));
        }
        facts.push((
            "history",
            format!(
                "{} events · {}",
                self.history_length,
                bytes(self.history_size_bytes)
            ),
        ));
        if let Some(parent) = &self.parent {
            facts.push(("parent", execution(parent)));
        }
        if let Some(root) = self
            .root
            .as_ref()
            .filter(|r| r.workflow_id != self.workflow_id || r.run_id != self.run_id)
            .filter(|r| self.parent.as_ref() != Some(*r))
        {
            facts.push(("root", execution(root)));
        }

        let mut wide = listed("memo", &self.memo);
        wide.extend(listed("search", &self.search_attributes));

        let mut empty = Vec::new();
        if self.parent.is_none() {
            empty.push("no parent");
        }
        if self.memo.is_empty() {
            empty.push("no memo");
        }
        if self.search_attributes.is_empty() {
            empty.push("no search attributes");
        }
        if self.close_time.is_none() && self.pending_children.is_empty() {
            empty.push("no children");
        }
        match (
            set(self.execution_timeout_ms),
            set(self.run_timeout_ms),
            set(self.task_timeout_ms),
        ) {
            (false, false, false) => empty.push("no timeouts"),
            (false, false, true) => empty.push("no execution or run timeout"),
            _ => {}
        }

        Card {
            waiting,
            tone,
            facts,
            wide,
            empty,
        }
    }

    pub fn payloads_mut(&mut self) -> impl Iterator<Item = &mut Payload> {
        self.memo
            .iter_mut()
            .chain(self.search_attributes.iter_mut())
            .map(|(_, p)| p)
    }

    pub fn payloads(&self) -> impl Iterator<Item = &Payload> {
        self.memo
            .iter()
            .chain(self.search_attributes.iter())
            .map(|(_, p)| p)
    }

    fn waiting_on(&self, now_ms: i64, pending: &[PendingActivity]) -> (Vec<String>, Tone) {
        if self.close_time.is_some() {
            return (vec!["nothing, the workflow is closed".into()], Tone::Quiet);
        }
        let mut items = Vec::new();
        let mut tone = Tone::Normal;
        for p in pending {
            if p.is_retrying() {
                tone = Tone::Retrying;
            }
            items.push(activity(p, now_ms));
        }
        if let Some(task) = &self.workflow_task {
            if task.attempt > 1 {
                tone = Tone::Retrying;
            }
            items.push(format!("workflow task {}", waiting(task, now_ms)));
        }
        for child in &self.pending_children {
            items.push(format!(
                "child {} {}",
                child.workflow_type, child.workflow_id
            ));
        }
        if items.is_empty() {
            return (vec!["nothing pending".into()], Tone::Quiet);
        }
        if items.len() > MAX_LISTED {
            let more = items.len() - MAX_LISTED;
            items.truncate(MAX_LISTED);
            items.push(format!("+{more} more"));
        }
        (items, tone)
    }

    fn timeouts(&self) -> Vec<String> {
        [
            ("execution", self.execution_timeout_ms),
            ("run", self.run_timeout_ms),
            ("workflow task", self.task_timeout_ms),
        ]
        .into_iter()
        .filter(|(_, ms)| set(*ms))
        .map(|(name, ms)| format!("{name} {}", span(ms.unwrap_or_default())))
        .collect()
    }
}

pub fn elapsed(start: Option<i64>, close: Option<i64>, now_ms: i64) -> Option<String> {
    let start = start?;
    Some(match close {
        Some(end) => format!("took {}", span(end - start)),
        None => span(now_ms - start),
    })
}

fn activity(p: &PendingActivity, now_ms: i64) -> String {
    let mut out = format!("activity {}", p.activity_type);
    match p.status(now_ms) {
        Some(status) => out.push_str(&format!(" · {status}")),
        None => {
            if let Some(at) = p.last_started_at {
                out.push_str(&format!(" · started {} ago", span(now_ms - at)));
            }
        }
    }
    out.push_str(&format!(" · attempt {}", p.attempts_label()));
    if let Some(f) = &p.last_failure {
        out.push_str(&format!(" · last: {}", f.message));
    }
    out
}

fn listed(label: &'static str, values: &[(String, Payload)]) -> Vec<(&'static str, String)> {
    let mut rows: Vec<(&'static str, String)> = values
        .iter()
        .take(MAX_LISTED)
        .enumerate()
        .map(|(i, (k, p))| {
            (
                if i == 0 { label } else { "" },
                format!("{k} {}", p.summary(VALUE_WIDTH)),
            )
        })
        .collect();
    if values.len() > MAX_LISTED {
        rows.push(("", format!("+{} more", values.len() - MAX_LISTED)));
    }
    rows
}

fn set(ms: Option<i64>) -> bool {
    ms.is_some_and(|ms| ms > 0)
}

fn waiting(task: &PendingWorkflowTask, now_ms: i64) -> String {
    let mut what = task.state.to_string();
    if let Some(at) = task.started_at.or(task.scheduled_at) {
        what.push_str(&format!(" {} ago", span(now_ms - at)));
    }
    if task.attempt > 1 {
        what.push_str(&format!(", attempt {}", task.attempt));
    }
    what
}

fn execution(e: &Execution) -> String {
    if e.run_id.is_empty() {
        e.workflow_id.clone()
    } else {
        format!("{} / {}", e.workflow_id, e.run_id)
    }
}

pub fn span(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    let units = [("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)];
    let mut parts = Vec::new();
    let mut rest = secs;
    for (name, size) in units {
        if rest >= size || (name == "s" && parts.is_empty()) {
            parts.push(format!("{}{name}", rest / size));
            rest %= size;
        }
        if parts.len() == 2 {
            break;
        }
    }
    parts.join(" ")
}

fn bytes(n: i64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.1} KiB", n as f64 / 1024.0),
        n => format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Failure;
    use crate::pending::PendingState;

    fn running() -> WorkflowInfo {
        WorkflowInfo {
            workflow_id: "order-123".into(),
            run_id: "r2".into(),
            first_run_id: "r2".into(),
            workflow_type: "OrderWorkflow".into(),
            status: WorkflowStatus::Running,
            task_queue: "orders".into(),
            start_time: Some(0),
            execution_time: Some(0),
            history_length: 124,
            history_size_bytes: 2048,
            task_timeout_ms: Some(10_000),
            ..Default::default()
        }
    }

    fn card(info: &WorkflowInfo, now: i64, pending: &[PendingActivity]) -> Card {
        info.card(now, pending, |ms| format!("t{ms}"))
    }

    fn fact<'a>(card: &'a Card, label: &str) -> Option<&'a str> {
        card.facts
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_plain_running_workflow_has_a_short_card() {
        let c = card(&running(), 4_860_000, &[]);
        assert_eq!(c.waiting, ["nothing pending"]);
        assert_eq!(c.tone, Tone::Quiet);
        let labels: Vec<&str> = c.facts.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["run", "started", "timeouts", "history"]);
        assert_eq!(fact(&c, "started"), Some("t0 (1h 21m ago)"));
        assert_eq!(fact(&c, "timeouts"), Some("workflow task 10s"));
        assert!(c.wide.is_empty());
        assert_eq!(
            c.empty,
            [
                "no parent",
                "no memo",
                "no search attributes",
                "no children",
                "no execution or run timeout"
            ]
        );
    }

    #[test]
    fn the_card_does_not_repeat_what_the_bar_shows() {
        let c = card(&running(), 0, &[]);
        let all: Vec<&str> = c
            .facts
            .iter()
            .chain(&c.wide)
            .map(|(_, v)| v.as_str())
            .collect();
        assert!(!all.contains(&"order-123"), "{all:?}");
        assert!(!all.contains(&"orders"), "{all:?}");
    }

    #[test]
    fn a_pending_activity_is_named_with_its_age_and_attempt() {
        let p = PendingActivity {
            activity_type: "check_s1_pd_model".into(),
            state: PendingState::Started,
            attempt: 1,
            last_started_at: Some(56_000),
            ..Default::default()
        };
        let c = card(&running(), 60_000, &[p]);
        assert_eq!(
            c.waiting,
            ["activity check_s1_pd_model · started 4s ago · attempt 1/∞"]
        );
        assert_eq!(c.tone, Tone::Normal);
    }

    #[test]
    fn a_retrying_activity_says_when_it_tries_again_and_why_it_failed() {
        let p = PendingActivity {
            activity_type: "ChargeCard".into(),
            state: PendingState::Scheduled,
            attempt: 4,
            maximum_attempts: 10,
            next_attempt_at: Some(72_000),
            last_failure: Some(Failure::new("card declined")),
            ..Default::default()
        };
        let c = card(&running(), 60_000, &[p]);
        assert_eq!(
            c.waiting,
            ["activity ChargeCard · retry in 12s · attempt 4/10 · last: card declined"]
        );
        assert_eq!(c.tone, Tone::Retrying);
    }

    #[test]
    fn more_than_three_things_waiting_are_counted_not_listed() {
        let mut info = running();
        info.workflow_task = Some(PendingWorkflowTask {
            state: "scheduled",
            scheduled_at: Some(0),
            ..Default::default()
        });
        for id in ["a", "b", "c"] {
            info.pending_children.push(PendingChild {
                workflow_id: id.into(),
                workflow_type: "Child".into(),
                ..Default::default()
            });
        }
        let c = card(&info, 240_000, &[]);
        assert_eq!(
            c.waiting,
            [
                "workflow task scheduled 4m ago",
                "child Child a",
                "child Child b",
                "+1 more"
            ]
        );
        assert!(!c.empty.contains(&"no children"));
    }

    #[test]
    fn a_closed_workflow_waits_on_nothing_whatever_the_server_still_lists() {
        let mut info = running();
        info.close_time = Some(90_000);
        info.workflow_task = Some(PendingWorkflowTask::default());
        let c = card(&info, 1_000_000, &[]);
        assert_eq!(c.waiting, ["nothing, the workflow is closed"]);
        assert_eq!(fact(&c, "closed"), Some("t90000"));
        assert_eq!(fact(&c, "started"), Some("t0"), "no age once it is closed");
        assert!(!c.empty.contains(&"no children"));
    }

    #[test]
    fn rows_that_only_sometimes_matter_appear_only_then() {
        let mut info = running();
        info.first_run_id = "r1".into();
        info.execution_time = Some(30_000);
        info.parent = Some(Execution {
            workflow_id: "payment-9".into(),
            run_id: "p1".into(),
        });
        info.root = Some(Execution {
            workflow_id: "checkout-1".into(),
            run_id: "c1".into(),
        });
        let c = card(&info, 0, &[]);
        assert_eq!(fact(&c, "first run"), Some("r1"));
        assert_eq!(fact(&c, "first task"), Some("t30000"));
        assert_eq!(fact(&c, "parent"), Some("payment-9 / p1"));
        assert_eq!(fact(&c, "root"), Some("checkout-1 / c1"));
        assert!(!c.empty.contains(&"no parent"));
    }

    #[test]
    fn a_child_whose_parent_is_the_root_names_it_once() {
        let mut info = running();
        let parent = Execution {
            workflow_id: "checkout-1".into(),
            run_id: "c1".into(),
        };
        info.parent = Some(parent.clone());
        info.root = Some(parent);
        assert_eq!(fact(&card(&info, 0, &[]), "root"), None);
    }

    #[test]
    fn every_timeout_that_is_set_shares_one_row() {
        let mut info = running();
        info.execution_timeout_ms = Some(86_400_000);
        info.run_timeout_ms = Some(7_200_000);
        let c = card(&info, 0, &[]);
        assert_eq!(
            fact(&c, "timeouts"),
            Some("execution 1d · run 2h · workflow task 10s")
        );
        assert!(!c.empty.iter().any(|e| e.contains("timeout")));
    }

    #[test]
    fn memo_and_search_attributes_get_a_row_per_key_and_cap_at_three() {
        let mut info = running();
        for k in ["a", "b", "c", "d", "e"] {
            info.memo
                .push((k.into(), Payload::new("json/plain", b"1".to_vec())));
        }
        info.search_attributes.push((
            "Tenant".into(),
            Payload::new("json/plain", b"\"acme\"".to_vec()),
        ));
        let c = card(&info, 0, &[]);
        assert_eq!(
            c.wide,
            [
                ("memo", "a 1".to_string()),
                ("", "b 1".to_string()),
                ("", "c 1".to_string()),
                ("", "+2 more".to_string()),
                ("search", "Tenant \"acme\"".to_string()),
            ]
        );
    }

    #[test]
    fn a_running_workflow_counts_up_and_a_closed_one_says_how_long_it_took() {
        assert_eq!(elapsed(Some(0), None, 6_120_000).as_deref(), Some("1h 42m"));
        assert_eq!(
            elapsed(Some(0), Some(90_000), 10_000_000).as_deref(),
            Some("took 1m 30s")
        );
        assert_eq!(elapsed(None, None, 0), None);
    }

    #[test]
    fn a_span_keeps_its_two_largest_units() {
        assert_eq!(span(0), "0s");
        assert_eq!(span(12_000), "12s");
        assert_eq!(span(90_000), "1m 30s");
        assert_eq!(span(3_600_000), "1h");
        assert_eq!(span(93_784_000), "1d 2h");
        assert_eq!(span(-5), "0s");
    }
}
