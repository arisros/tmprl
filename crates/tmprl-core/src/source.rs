//! From a history event to the source that produced it: what a resolver is told, and what
//! it answers.
//!
//! tmprl knows nothing about anyone's code. A resolver is a program the user names in
//! `config.toml`; it reads one JSON object on stdin and prints where to open the editor.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::clock::rfc3339_utc;
use crate::history::{Category, Group, NormalizedEvent};
use crate::workflow::WorkflowRow;

/// The shape of the object a resolver reads. Raised when a field is renamed or removed,
/// never when one is added.
pub const CONTRACT: u32 = 1;

/// Where a resolver said to open the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: Option<u32>,
}

fn category(category: Category) -> &'static str {
    match category {
        Category::Workflow => "workflow",
        Category::WorkflowTask => "workflow_task",
        Category::Activity => "activity",
        Category::Timer => "timer",
        Category::ChildWorkflow => "child_workflow",
        Category::ExternalWorkflow => "external_workflow",
        Category::Update => "update",
        Category::Nexus => "nexus",
        Category::Marker => "marker",
        Category::SearchAttributes => "search_attributes",
    }
}

fn event(event: &NormalizedEvent) -> Value {
    let fields: Map<String, Value> = event
        .fields
        .iter()
        .map(|(name, value)| (name.to_string(), Value::String(value.clone())))
        .collect();
    json!({
        "id": event.id,
        "name": event.name,
        "time": event.time.map(rfc3339_utc),
        "subject": event.subject,
        "attempt": event.attempt,
        "fields": fields,
    })
}

/// What a resolver is told about the thing under the cursor.
///
/// `events` are the group's, in history order, with every detail row tmprl kept: the
/// worker's identity is among them, which is often the only trace of which build ran.
/// `payloads` is the JSON object `!` pipes, already decoded where a codec could.
pub fn request(
    profile: &str,
    workflow: &WorkflowRow,
    group: &Group,
    events: &[&NormalizedEvent],
    focused: Option<&NormalizedEvent>,
    payloads: Option<&str>,
) -> Value {
    json!({
        "version": CONTRACT,
        "profile": profile,
        "namespace": workflow.namespace,
        "workflow": {
            "id": workflow.workflow_id,
            "run_id": workflow.run_id,
            "type": workflow.workflow_type,
            "task_queue": workflow.task_queue,
            "status": workflow.status.query_name(),
        },
        "kind": category(group.category),
        "name": group.subject,
        "outcome": group.outcome.label(),
        "attempts": group.attempts,
        "started_at": group.started_at.map(rfc3339_utc),
        "ended_at": group.ended_at.map(rfc3339_utc),
        "event": focused.map(event),
        "events": events.iter().map(|e| event(e)).collect::<Vec<_>>(),
        "payloads": payloads
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or(Value::Null),
    })
}

/// Read a resolver's answer: the first line that says anything, `path`, `path:line` or
/// `path:line:column`. The path must be absolute, since the resolver's working directory
/// is not the reader's.
pub fn locate(output: &str) -> Result<Location, String> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or("the resolver printed nothing")?;

    let mut path = line;
    let mut numbers: Vec<u32> = Vec::new();
    while numbers.len() < 2
        && let Some((head, tail)) = path.rsplit_once(':')
        && let Ok(n) = tail.parse::<u32>()
    {
        numbers.push(n);
        path = head;
    }
    // Read from the right, so the line is the last one found: `path:line:column`.
    let at = numbers.last().copied().filter(|n| *n > 0);

    if !path.starts_with('/') {
        return Err(format!(
            "the resolver gave `{path}`, which is not an absolute path"
        ));
    }
    Ok(Location {
        path: PathBuf::from(path),
        line: at,
    })
}

/// Split a configured command into a program and its arguments, the way `$EDITOR` is
/// split: on whitespace, with a leading `~/` meaning the home directory. No shell, so
/// nothing in the command is an expansion or a pipeline.
pub fn argv(command: &str, home: Option<&str>) -> Option<(String, Vec<String>)> {
    let expand = |word: &str| match (word.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => word.to_string(),
    };
    let mut words = command.split_whitespace().map(expand);
    let program = words.next()?;
    Some((program, words.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{GroupRef, Outcome, Role};
    use crate::workflow::WorkflowStatus;

    fn workflow() -> WorkflowRow {
        WorkflowRow {
            namespace: "orders".into(),
            workflow_id: "order-7".into(),
            run_id: "run-7".into(),
            workflow_type: "Checkout".into(),
            task_queue: "checkout-v3".into(),
            status: WorkflowStatus::Failed,
            start_time: Some(1_000),
            close_time: None,
            history_length: 9,
        }
    }

    #[test]
    fn a_resolver_is_told_the_workflow_the_thing_and_every_event_of_it() {
        let mut started = NormalizedEvent::new(
            6,
            "ActivityTaskStarted",
            Category::Activity,
            GroupRef::Opened(5),
            Role::Continues,
        )
        .with_time(Some(1_700_000_000_000))
        .with_subject("ChargeCard");
        started.attempt = Some(3);
        started.fields = vec![("identity", "worker@2026.01.02@ab".into())];
        let group = Group {
            key: GroupRef::Opened(5),
            category: Category::Activity,
            subject: "ChargeCard".into(),
            events: vec![5, 6],
            started_at: Some(1_700_000_000_000),
            ended_at: None,
            outcome: Outcome::Failed,
            attempts: 3,
            failure: None,
        };

        let told = request(
            "prod",
            &workflow(),
            &group,
            &[&started],
            Some(&started),
            Some(r#"{"input": {"amount": 4}}"#),
        );
        assert_eq!(told["version"], 1);
        assert_eq!(told["profile"], "prod");
        assert_eq!(told["namespace"], "orders");
        assert_eq!(told["workflow"]["type"], "Checkout");
        assert_eq!(told["workflow"]["task_queue"], "checkout-v3");
        assert_eq!(told["kind"], "activity");
        assert_eq!(told["name"], "ChargeCard");
        assert_eq!(told["outcome"], "Failed");
        assert_eq!(told["event"]["id"], 6);
        assert_eq!(
            told["events"][0]["fields"]["identity"],
            "worker@2026.01.02@ab"
        );
        assert_eq!(told["events"][0]["time"], "2023-11-14T22:13:20Z");
        assert_eq!(told["payloads"]["input"]["amount"], 4);
    }

    #[test]
    fn an_answer_is_a_path_with_or_without_a_line() {
        let at = |text: &str| locate(text).map(|l| (l.path.display().to_string(), l.line));
        assert_eq!(at("/src/a.go"), Ok(("/src/a.go".into(), None)));
        assert_eq!(at("/src/a.go:16"), Ok(("/src/a.go".into(), Some(16))));
        assert_eq!(at("/src/a.go:16:3"), Ok(("/src/a.go".into(), Some(16))));
        assert_eq!(
            at("\n  /src/a.go:16  \nmore, ignored"),
            Ok(("/src/a.go".into(), Some(16)))
        );
        assert_eq!(
            at("/odd:name/a.go:9"),
            Ok(("/odd:name/a.go".into(), Some(9))),
            "a colon in the path is the path's"
        );
        assert_eq!(at("/src/a.go:0"), Ok(("/src/a.go".into(), None)));
    }

    #[test]
    fn an_answer_that_names_no_place_is_refused() {
        assert!(locate("").is_err());
        assert!(locate("  \n \n").is_err());
        let relative = locate("internal/a.go:4").unwrap_err();
        assert!(relative.contains("not an absolute path"), "{relative}");
    }

    #[test]
    fn a_command_is_split_like_an_editor_and_never_through_a_shell() {
        assert_eq!(
            argv("~/bin/find-source --fast", Some("/home/me")),
            Some((
                "/home/me/bin/find-source".to_string(),
                vec!["--fast".to_string()]
            ))
        );
        assert_eq!(
            argv("resolver $(rm -rf x); y", None),
            Some((
                "resolver".to_string(),
                vec!["$(rm".into(), "-rf".into(), "x);".into(), "y".into()]
            )),
            "words, not a script"
        );
        assert_eq!(argv("   ", None), None);
    }
}
