//! Reading a workflow history, and flattening it into normalised events.
//!
//! The match over the attributes `oneof` is **exhaustive on purpose**, per design rule 4.
//! Temporal adds event types regularly — Nexus, worker versioning and workflow pausing are
//! all recent. With a `_ => {}` arm a new event type renders as a blank row and nobody
//! notices for a release or two; exhaustive, it is a compile error the moment the protos
//! are bumped, which is exactly when we want to hear about it.
//!
//! Grouping keys come straight from the protocol's back-references. Which field to follow
//! is not uniform, so it is spelled out per arm rather than guessed:
//!
//! * activities, workflow tasks and Nexus operations → `scheduled_event_id`
//! * child and external workflows → `initiated_event_id`
//! * timers → `started_event_id` (the id of the `TimerStarted` event)
//! * updates → `accepted_event_id`
//!
//! `workflow_task_completed_event_id` appears on many of these too, but it points at the
//! workflow task that *caused* the command, not at the thing's own group — following it
//! would file every activity under the task that scheduled it.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::{Payload as ProtoPayload, Payloads, WorkflowExecution},
    enums::v1::HistoryEventFilterType,
    failure::v1::Failure,
    history::v1::{HistoryEvent, history_event::Attributes},
    workflowservice::v1::GetWorkflowExecutionHistoryRequest,
};
use tmprl_core::history::{Category, GroupRef, NormalizedEvent, Outcome, Role};
use tmprl_core::payload::Payload;

use super::OpError;
use crate::Conn;

/// One page of a workflow's history.
#[derive(Debug, Clone, Default)]
pub struct HistoryPage {
    pub events: Vec<NormalizedEvent>,
    /// Empty on the last page.
    pub next_page_token: Vec<u8>,
}

impl HistoryPage {
    pub fn has_more(&self) -> bool {
        !self.next_page_token.is_empty()
    }
}

impl Conn {
    /// One long-poll step of follow mode.
    ///
    /// This is [`Conn::get_history`] with `wait_new_event: true`, which makes the call
    /// **block until the workflow does something** — up to about a minute, then it returns
    /// empty-handed and you call again. Never reach for this outside a task dedicated to
    /// following; anything else it is on will simply stop.
    ///
    /// The behaviour of the continuation token differs from paging, and follow mode is built
    /// on the difference. Measured against a dev server:
    ///
    /// | | `wait_new_event: false` | `wait_new_event: true` |
    /// |---|---|---|
    /// | running workflow, caught up | returns 0 events, **empty** token | blocks, then returns new events, token stays non-empty |
    /// | closed workflow | empty token | empty token, terminal event last |
    ///
    /// So an **empty token here means the workflow has closed** and there is nothing further
    /// to follow — that is the loop's termination condition, and it is authoritative in a way
    /// that inspecting the last event's type is not.
    ///
    /// Passing an empty token restarts from event 1, so a caller resuming a follow should
    /// hand back the last non-empty token it saw. The page that token sits in is replayed,
    /// which is why events are merged rather than appended — see
    /// `tmprl_core::history::merge_events`.
    pub async fn follow_history(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
        next_page_token: Vec<u8>,
    ) -> Result<HistoryPage, OpError> {
        self.history_request(namespace, workflow_id, run_id, 100, next_page_token, true)
            .await
    }

    /// One page of history, normalised.
    ///
    /// `wait_new_event` is false here. Setting it true turns this into a long poll that does
    /// not return until something happens, which is correct for follow mode and a hang
    /// everywhere else — so follow mode gets [`Conn::follow_history`] rather than a flag on
    /// this one that is easy to pass by accident.
    pub async fn get_history(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
        page_size: i32,
        next_page_token: Vec<u8>,
    ) -> Result<HistoryPage, OpError> {
        self.history_request(
            namespace,
            workflow_id,
            run_id,
            page_size,
            next_page_token,
            false,
        )
        .await
    }

    async fn history_request(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
        page_size: i32,
        next_page_token: Vec<u8>,
        wait_new_event: bool,
    ) -> Result<HistoryPage, OpError> {
        let resp = self
            .wf()
            .get_workflow_execution_history(Request::new(GetWorkflowExecutionHistoryRequest {
                namespace: namespace.to_string(),
                execution: Some(WorkflowExecution {
                    workflow_id: workflow_id.to_string(),
                    run_id: run_id.to_string(),
                }),
                maximum_page_size: page_size,
                next_page_token,
                wait_new_event,
                history_event_filter_type: HistoryEventFilterType::AllEvent as i32,
                skip_archival: false,
            }))
            .await
            .map_err(|s| OpError::rpc("GetWorkflowExecutionHistory", s))?
            .into_inner();

        Ok(HistoryPage {
            events: resp
                .history
                .map(|h| h.events)
                .unwrap_or_default()
                .into_iter()
                .map(normalize)
                .collect(),
            next_page_token: resp.next_page_token,
        })
    }
}

/// What the match over the attributes produces. Assembled into a [`NormalizedEvent`] with
/// the id and timestamp, which every event has regardless of its type.
struct Mapped {
    category: Category,
    group: GroupRef,
    role: Role,
    outcome: Outcome,
    subject: String,
    attempt: Option<i32>,
    failure: Option<String>,
    fields: Vec<(&'static str, String)>,
    payloads: Vec<(String, Payload)>,
}

/// Start an arm. `group` is the group this event joins; `role` is what it does to it.
fn at(category: Category, group: GroupRef, role: Role) -> Mapped {
    Mapped {
        category,
        group,
        role,
        outcome: Outcome::Pending,
        subject: String::new(),
        attempt: None,
        failure: None,
        fields: Vec::new(),
        payloads: Vec::new(),
    }
}

impl Mapped {
    fn subject(mut self, s: impl Into<String>) -> Self {
        self.subject = s.into();
        self
    }
    fn ends(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }
    fn failed(mut self, f: Option<Failure>) -> Self {
        self.failure = f.map(|f| f.message);
        self
    }
    fn attempt(mut self, n: i32) -> Self {
        self.attempt = Some(n);
        self
    }
    fn field(mut self, k: &'static str, v: impl Into<String>) -> Self {
        let v = v.into();
        if !v.is_empty() {
            self.fields.push((k, v));
        }
        self
    }

    /// Attach an argument list. A single value is labelled plainly; several are indexed,
    /// because an activity's third argument is not interchangeable with its first.
    fn args(mut self, label: &str, p: Option<Payloads>) -> Self {
        let Some(list) = p else { return self };
        let n = list.payloads.len();
        for (i, raw) in list.payloads.into_iter().enumerate() {
            let name = if n == 1 {
                label.to_string()
            } else {
                format!("{label}[{i}]")
            };
            self.payloads.push((name, convert(raw)));
        }
        self
    }

    /// Attach a single payload.
    fn arg(mut self, label: &str, p: Option<ProtoPayload>) -> Self {
        if let Some(raw) = p {
            self.payloads.push((label.to_string(), convert(raw)));
        }
        self
    }
}

/// Protobuf payload to domain payload. The metadata values are bytes on the wire; the two
/// keys tmprl reads are ASCII.
fn convert(p: ProtoPayload) -> Payload {
    let meta = |k: &str| {
        p.metadata
            .get(k)
            .and_then(|v| std::str::from_utf8(v).ok())
            .map(str::to_string)
    };
    Payload {
        encoding: meta("encoding").unwrap_or_default(),
        type_hint: meta("type"),
        data: p.data,
    }
}

/// Flatten one protobuf event.
pub fn normalize(e: HistoryEvent) -> NormalizedEvent {
    let id = e.event_id;
    let time = e
        .event_time
        .map(|t| t.seconds * 1000 + i64::from(t.nanos) / 1_000_000);
    // `event_type()` borrows, so read the name before the attributes are moved out.
    let name = event_name(e.event_type().as_str_name());

    let m = match e.attributes {
        // An event with no attributes is representable on the wire but meaningless. It is
        // rendered as itself rather than dropped, so a history never silently loses a row.
        None => at(Category::Workflow, GroupRef::Workflow, Role::Continues),

        // ── the workflow itself ──────────────────────────────────────────────
        Some(Attributes::WorkflowExecutionStartedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Opens)
                .subject(a.workflow_type.map(|t| t.name).unwrap_or_default())
                .field(
                    "taskQueue",
                    a.task_queue.map(|q| q.name).unwrap_or_default(),
                )
                .field("attempt", a.attempt.to_string())
                .field("firstRunId", a.first_execution_run_id)
        }
        Some(Attributes::WorkflowExecutionCompletedEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes).ends(Outcome::Completed)
        }
        Some(Attributes::WorkflowExecutionFailedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::Failed)
                .failed(a.failure)
        }
        Some(Attributes::WorkflowExecutionTimedOutEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes).ends(Outcome::TimedOut)
        }
        Some(Attributes::WorkflowExecutionCanceledEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes).ends(Outcome::Canceled)
        }
        Some(Attributes::WorkflowExecutionTerminatedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::Terminated)
                .field("reason", a.reason)
                .field("identity", a.identity)
        }
        Some(Attributes::WorkflowExecutionContinuedAsNewEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::ContinuedAsNew)
                .field("newRunId", a.new_execution_run_id)
        }
        Some(Attributes::WorkflowExecutionCancelRequestedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
                .field("cause", a.cause)
                .field("identity", a.identity)
        }
        Some(Attributes::WorkflowExecutionSignaledEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
                .subject(a.signal_name)
                .field("identity", a.identity)
        }
        Some(Attributes::WorkflowExecutionPausedEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
        }
        Some(Attributes::WorkflowExecutionUnpausedEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
        }
        Some(Attributes::WorkflowExecutionOptionsUpdatedEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
        }
        Some(Attributes::WorkflowPropertiesModifiedEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
        }
        Some(Attributes::WorkflowPropertiesModifiedExternallyEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
        }
        Some(Attributes::WorkflowExecutionTimeSkippingTransitionedEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Continues)
        }

        // ── workflow tasks ───────────────────────────────────────────────────
        Some(Attributes::WorkflowTaskScheduledEventAttributes(a)) => {
            at(Category::WorkflowTask, GroupRef::Opened(id), Role::Opens)
                .attempt(a.attempt)
                .field(
                    "taskQueue",
                    a.task_queue.map(|q| q.name).unwrap_or_default(),
                )
        }
        Some(Attributes::WorkflowTaskStartedEventAttributes(a)) => at(
            Category::WorkflowTask,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        )
        .field("identity", a.identity),
        Some(Attributes::WorkflowTaskCompletedEventAttributes(a)) => at(
            Category::WorkflowTask,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed),
        Some(Attributes::WorkflowTaskTimedOutEventAttributes(a)) => at(
            Category::WorkflowTask,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::TimedOut),
        Some(Attributes::WorkflowTaskFailedEventAttributes(a)) => at(
            Category::WorkflowTask,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .failed(a.failure),

        // ── activities ───────────────────────────────────────────────────────
        Some(Attributes::ActivityTaskScheduledEventAttributes(a)) => {
            at(Category::Activity, GroupRef::Opened(id), Role::Opens)
                .subject(a.activity_type.map(|t| t.name).unwrap_or_default())
                .field("activityId", a.activity_id)
                .field(
                    "taskQueue",
                    a.task_queue.map(|q| q.name).unwrap_or_default(),
                )
                .args("input", a.input)
        }
        Some(Attributes::ActivityTaskStartedEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        )
        // A retry does not schedule again; this is where the attempt count lives, and
        // `last_failure` is why the previous attempt did not stick.
        .attempt(a.attempt)
        .failed(a.last_failure)
        .field("identity", a.identity),
        Some(Attributes::ActivityTaskCompletedEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed)
        .args("result", a.result),
        Some(Attributes::ActivityTaskFailedEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .failed(a.failure),
        Some(Attributes::ActivityTaskTimedOutEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::TimedOut)
        .failed(a.failure),
        Some(Attributes::ActivityTaskCanceledEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Canceled),
        Some(Attributes::ActivityTaskCancelRequestedEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        ),
        Some(Attributes::ActivityPropertiesModifiedExternallyEventAttributes(a)) => at(
            Category::Activity,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        ),

        // ── timers ───────────────────────────────────────────────────────────
        Some(Attributes::TimerStartedEventAttributes(a)) => {
            at(Category::Timer, GroupRef::Opened(id), Role::Opens)
                .subject(a.timer_id)
                .field(
                    "startToFireTimeout",
                    a.start_to_fire_timeout
                        .map(|d| format!("{}s", d.seconds))
                        .unwrap_or_default(),
                )
        }
        Some(Attributes::TimerFiredEventAttributes(a)) => at(
            Category::Timer,
            GroupRef::Opened(a.started_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed),
        Some(Attributes::TimerCanceledEventAttributes(a)) => at(
            Category::Timer,
            GroupRef::Opened(a.started_event_id),
            Role::Closes,
        )
        .ends(Outcome::Canceled),

        // ── child workflows ──────────────────────────────────────────────────
        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a)) => {
            at(Category::ChildWorkflow, GroupRef::Opened(id), Role::Opens)
                .subject(a.workflow_type.map(|t| t.name).unwrap_or_default())
                .field("workflowId", a.workflow_id)
                .field("namespace", a.namespace)
                .args("input", a.input)
        }
        Some(Attributes::StartChildWorkflowExecutionFailedEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .field("workflowId", a.workflow_id),
        Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Continues,
        )
        .field(
            "runId",
            a.workflow_execution.map(|w| w.run_id).unwrap_or_default(),
        ),
        Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed)
        .args("result", a.result),
        Some(Attributes::ChildWorkflowExecutionFailedEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .failed(a.failure),
        Some(Attributes::ChildWorkflowExecutionCanceledEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Canceled),
        Some(Attributes::ChildWorkflowExecutionTimedOutEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::TimedOut),
        Some(Attributes::ChildWorkflowExecutionTerminatedEventAttributes(a)) => at(
            Category::ChildWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Terminated),

        // ── signalling and cancelling other workflows ────────────────────────
        Some(Attributes::SignalExternalWorkflowExecutionInitiatedEventAttributes(a)) => at(
            Category::ExternalWorkflow,
            GroupRef::Opened(id),
            Role::Opens,
        )
        .subject(a.signal_name)
        .field(
            "workflowId",
            a.workflow_execution
                .map(|w| w.workflow_id)
                .unwrap_or_default(),
        )
        .field("namespace", a.namespace)
        .args("input", a.input),
        Some(Attributes::SignalExternalWorkflowExecutionFailedEventAttributes(a)) => at(
            Category::ExternalWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .field("cause", a.cause.to_string()),
        Some(Attributes::ExternalWorkflowExecutionSignaledEventAttributes(a)) => at(
            Category::ExternalWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed),
        Some(Attributes::RequestCancelExternalWorkflowExecutionInitiatedEventAttributes(a)) => at(
            Category::ExternalWorkflow,
            GroupRef::Opened(id),
            Role::Opens,
        )
        .subject("cancel")
        .field(
            "workflowId",
            a.workflow_execution
                .map(|w| w.workflow_id)
                .unwrap_or_default(),
        ),
        Some(Attributes::RequestCancelExternalWorkflowExecutionFailedEventAttributes(a)) => at(
            Category::ExternalWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .field("cause", a.cause.to_string()),
        Some(Attributes::ExternalWorkflowExecutionCancelRequestedEventAttributes(a)) => at(
            Category::ExternalWorkflow,
            GroupRef::Opened(a.initiated_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed),

        // ── updates ──────────────────────────────────────────────────────────
        Some(Attributes::WorkflowExecutionUpdateAdmittedEventAttributes(_)) => {
            at(Category::Update, GroupRef::Opened(id), Role::Opens)
        }
        Some(Attributes::WorkflowExecutionUpdateAcceptedEventAttributes(a)) => {
            at(Category::Update, GroupRef::Opened(id), Role::Opens).subject(a.protocol_instance_id)
        }
        Some(Attributes::WorkflowExecutionUpdateCompletedEventAttributes(a)) => at(
            Category::Update,
            GroupRef::Opened(a.accepted_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed),
        Some(Attributes::WorkflowExecutionUpdateRejectedEventAttributes(a)) => {
            // Rejected before acceptance, so there is no accepted event to hang it on.
            at(Category::Update, GroupRef::Opened(id), Role::Opens)
                .subject(a.protocol_instance_id)
                .ends(Outcome::Rejected)
                .failed(a.failure)
        }

        // ── Nexus ────────────────────────────────────────────────────────────
        Some(Attributes::NexusOperationScheduledEventAttributes(a)) => {
            at(Category::Nexus, GroupRef::Opened(id), Role::Opens)
                .subject(format!("{}/{}", a.service, a.operation))
                .field("endpoint", a.endpoint)
                .arg("input", a.input)
        }
        Some(Attributes::NexusOperationStartedEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        ),
        Some(Attributes::NexusOperationCompletedEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Completed),
        Some(Attributes::NexusOperationFailedEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Failed)
        .failed(a.failure),
        Some(Attributes::NexusOperationCanceledEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::Canceled),
        Some(Attributes::NexusOperationTimedOutEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Closes,
        )
        .ends(Outcome::TimedOut),
        Some(Attributes::NexusOperationCancelRequestedEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        ),
        Some(Attributes::NexusOperationCancelRequestCompletedEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        ),
        Some(Attributes::NexusOperationCancelRequestFailedEventAttributes(a)) => at(
            Category::Nexus,
            GroupRef::Opened(a.scheduled_event_id),
            Role::Continues,
        )
        .failed(a.failure),

        // ── bookkeeping ──────────────────────────────────────────────────────
        Some(Attributes::MarkerRecordedEventAttributes(a)) => {
            at(Category::Marker, GroupRef::Opened(id), Role::Opens)
                .subject(a.marker_name)
                .failed(a.failure)
        }
        Some(Attributes::UpsertWorkflowSearchAttributesEventAttributes(_)) => at(
            Category::SearchAttributes,
            GroupRef::Opened(id),
            Role::Opens,
        ),
    };

    NormalizedEvent {
        id,
        time,
        name,
        category: m.category,
        group: m.group,
        role: m.role,
        outcome: m.outcome,
        subject: m.subject,
        attempt: m.attempt,
        failure: m.failure,
        fields: m.fields,
        payloads: m.payloads,
    }
}

/// `EVENT_TYPE_ACTIVITY_TASK_SCHEDULED` → `ActivityTaskScheduled`.
///
/// Returns a `&'static str` by matching the protobuf's own `as_str_name()` output, which is
/// itself `&'static`. Building the name at runtime would mean leaking or allocating for
/// every event in a history that can run to millions.
fn event_name(proto: &'static str) -> &'static str {
    // Trim the prefix and rebuild the CamelCase name is not possible without allocating, so
    // the raw protobuf name is kept when it is not one we recognise. Unknown names only
    // appear for event types added after this was written, which the exhaustive match above
    // will have already flagged at compile time.
    proto.strip_prefix("EVENT_TYPE_").unwrap_or(proto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::ActivityType,
        enums::v1::EventType,
        history::v1::{
            ActivityTaskFailedEventAttributes, ActivityTaskScheduledEventAttributes,
            ActivityTaskStartedEventAttributes, TimerFiredEventAttributes,
            TimerStartedEventAttributes, WorkflowExecutionStartedEventAttributes,
        },
    };

    fn event(id: i64, ty: EventType, attrs: Attributes) -> HistoryEvent {
        HistoryEvent {
            event_id: id,
            event_time: Some(prost_wkt_types::Timestamp {
                seconds: id,
                nanos: 0,
            }),
            event_type: ty as i32,
            attributes: Some(attrs),
            ..Default::default()
        }
    }

    #[test]
    fn an_activity_scheduled_event_opens_a_group_at_its_own_id() {
        let n = normalize(event(
            5,
            EventType::ActivityTaskScheduled,
            Attributes::ActivityTaskScheduledEventAttributes(
                ActivityTaskScheduledEventAttributes {
                    activity_id: "charge".into(),
                    activity_type: Some(ActivityType {
                        name: "ChargeCard".into(),
                    }),
                    // Points at the workflow task that scheduled this. Following it would
                    // file the activity under that task instead of giving it its own group.
                    workflow_task_completed_event_id: 4,
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.id, 5);
        assert_eq!(n.group, GroupRef::Opened(5), "not Opened(4)");
        assert_eq!(n.role, Role::Opens);
        assert_eq!(n.category, Category::Activity);
        assert_eq!(n.subject, "ChargeCard");
        assert_eq!(n.name, "ACTIVITY_TASK_SCHEDULED");
        assert_eq!(n.time, Some(5_000));
        assert!(
            n.fields
                .iter()
                .any(|(k, v)| *k == "activityId" && v == "charge")
        );
    }

    #[test]
    fn an_activity_started_event_joins_its_scheduled_group_and_carries_the_attempt() {
        let n = normalize(event(
            6,
            EventType::ActivityTaskStarted,
            Attributes::ActivityTaskStartedEventAttributes(ActivityTaskStartedEventAttributes {
                scheduled_event_id: 5,
                attempt: 3,
                last_failure: Some(Failure {
                    message: "card declined".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ));

        assert_eq!(n.group, GroupRef::Opened(5));
        assert_eq!(n.role, Role::Continues);
        assert_eq!(
            n.attempt,
            Some(3),
            "retries do not re-schedule; the count is here"
        );
        assert_eq!(n.failure.as_deref(), Some("card declined"));
    }

    #[test]
    fn a_failed_activity_closes_its_group_with_the_failure_message() {
        let n = normalize(event(
            7,
            EventType::ActivityTaskFailed,
            Attributes::ActivityTaskFailedEventAttributes(ActivityTaskFailedEventAttributes {
                scheduled_event_id: 5,
                started_event_id: 6,
                failure: Some(Failure {
                    message: "out of retries".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ));

        assert_eq!(n.group, GroupRef::Opened(5));
        assert_eq!(n.role, Role::Closes);
        assert_eq!(n.outcome, Outcome::Failed);
        assert!(n.outcome.is_failure());
        assert_eq!(n.failure.as_deref(), Some("out of retries"));
    }

    #[test]
    fn a_timer_is_grouped_by_its_started_event_not_a_scheduled_one() {
        // Timers are the one family that back-references `started_event_id`. Reaching for
        // `scheduled_event_id` out of habit would leave every fired timer orphaned.
        let started = normalize(event(
            10,
            EventType::TimerStarted,
            Attributes::TimerStartedEventAttributes(TimerStartedEventAttributes {
                timer_id: "sleep-1".into(),
                workflow_task_completed_event_id: 9,
                ..Default::default()
            }),
        ));
        let fired = normalize(event(
            11,
            EventType::TimerFired,
            Attributes::TimerFiredEventAttributes(TimerFiredEventAttributes {
                timer_id: "sleep-1".into(),
                started_event_id: 10,
            }),
        ));

        assert_eq!(started.group, GroupRef::Opened(10));
        assert_eq!(started.subject, "sleep-1");
        assert_eq!(fired.group, started.group, "the fired timer must join it");
        assert_eq!(fired.outcome, Outcome::Completed);
    }

    #[test]
    fn the_workflow_start_event_belongs_to_the_workflow_group() {
        let n = normalize(event(
            1,
            EventType::WorkflowExecutionStarted,
            Attributes::WorkflowExecutionStartedEventAttributes(
                WorkflowExecutionStartedEventAttributes {
                    workflow_type: Some(
                        temporalio_common::protos::temporal::api::common::v1::WorkflowType {
                            name: "OrderWorkflow".into(),
                        },
                    ),
                    attempt: 1,
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.group, GroupRef::Workflow);
        assert_eq!(n.role, Role::Opens);
        assert_eq!(n.subject, "OrderWorkflow");
    }

    #[test]
    fn an_event_with_no_attributes_is_kept_rather_than_dropped() {
        let mut e = HistoryEvent {
            event_id: 99,
            ..Default::default()
        };
        e.attributes = None;
        let n = normalize(e);
        assert_eq!(n.id, 99);
        assert_eq!(n.group, GroupRef::Workflow);
    }

    fn json_payload(body: &str) -> ProtoPayload {
        ProtoPayload {
            metadata: [("encoding".to_string(), b"json/plain".to_vec())]
                .into_iter()
                .collect(),
            data: body.as_bytes().to_vec(),
            external_payloads: Vec::new(),
        }
    }

    #[test]
    fn an_activity_carries_its_input_payload() {
        let n = normalize(event(
            5,
            EventType::ActivityTaskScheduled,
            Attributes::ActivityTaskScheduledEventAttributes(
                ActivityTaskScheduledEventAttributes {
                    activity_type: Some(ActivityType {
                        name: "ChargeCard".into(),
                    }),
                    input: Some(Payloads {
                        payloads: vec![json_payload("100")],
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.payloads.len(), 1);
        let (label, p) = &n.payloads[0];
        assert_eq!(label, "input", "a lone argument is not indexed");
        assert_eq!(p.encoding, "json/plain");
        assert_eq!(
            p.render(),
            tmprl_core::payload::Rendered::Text("100".into())
        );
    }

    #[test]
    fn several_arguments_are_indexed() {
        // An activity's third argument is not interchangeable with its first, so an
        // unlabelled list would lose which is which.
        let n = normalize(event(
            5,
            EventType::ActivityTaskScheduled,
            Attributes::ActivityTaskScheduledEventAttributes(
                ActivityTaskScheduledEventAttributes {
                    input: Some(Payloads {
                        payloads: vec![json_payload("1"), json_payload("\"two\"")],
                    }),
                    ..Default::default()
                },
            ),
        ));

        let labels: Vec<&str> = n.payloads.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["input[0]", "input[1]"]);
    }

    #[test]
    fn an_event_with_no_payloads_carries_none() {
        let n = normalize(event(
            6,
            EventType::ActivityTaskStarted,
            Attributes::ActivityTaskStartedEventAttributes(ActivityTaskStartedEventAttributes {
                scheduled_event_id: 5,
                ..Default::default()
            }),
        ));
        assert!(n.payloads.is_empty());
    }

    #[test]
    fn payload_metadata_is_decoded_from_bytes() {
        // Metadata values are bytes on the wire; the encoding is what decides how the value
        // is shown, so getting it out wrongly would make every payload opaque.
        let mut raw = json_payload("{}");
        raw.metadata.insert("type".to_string(), b"Keyword".to_vec());
        let p = convert(raw);
        assert_eq!(p.encoding, "json/plain");
        assert_eq!(p.type_hint.as_deref(), Some("Keyword"));
    }

    #[test]
    fn a_page_knows_whether_more_exist() {
        assert!(!HistoryPage::default().has_more());
        assert!(
            HistoryPage {
                next_page_token: vec![1],
                ..Default::default()
            }
            .has_more()
        );
    }
}
