//! Reading a workflow history, and flattening it into normalised events.

use std::collections::HashMap;

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    common::v1::{Payload as ProtoPayload, Payloads, WorkflowExecution},
    enums::v1::HistoryEventFilterType,
    failure::v1::{Failure, failure::FailureInfo},
    history::v1::{HistoryEvent, history_event::Attributes},
    update::v1::{Request as UpdateRequest, outcome::Value as OutcomeValue},
    workflowservice::v1::GetWorkflowExecutionHistoryRequest,
};
use tmprl_core::history::{
    Category, Failure as CoreFailure, GroupRef, NormalizedEvent, Outcome, Role,
};
use tmprl_core::payload::Payload;

use super::OpError;
use crate::Conn;

#[derive(Debug, Clone, Default)]
pub struct HistoryPage {
    pub events: Vec<NormalizedEvent>,
    pub next_page_token: Vec<u8>,
}

impl HistoryPage {
    pub fn has_more(&self) -> bool {
        !self.next_page_token.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    Page,
    Follow,
    CloseEvent,
}

impl Conn {
    pub async fn follow_history(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
        next_page_token: Vec<u8>,
    ) -> Result<HistoryPage, OpError> {
        self.history_request(
            namespace,
            workflow_id,
            run_id,
            100,
            next_page_token,
            Read::Follow,
        )
        .await
    }

    pub async fn close_event(
        &self,
        namespace: &str,
        workflow_id: &str,
        run_id: &str,
    ) -> Result<Option<NormalizedEvent>, OpError> {
        let page = self
            .history_request(
                namespace,
                workflow_id,
                run_id,
                1,
                Vec::new(),
                Read::CloseEvent,
            )
            .await?;
        Ok(page
            .events
            .into_iter()
            .find(|e| e.group == GroupRef::Workflow && e.role == Role::Closes))
    }

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
            Read::Page,
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
        read: Read,
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
                wait_new_event: read == Read::Follow,
                history_event_filter_type: match read {
                    Read::Page | Read::Follow => HistoryEventFilterType::AllEvent,
                    Read::CloseEvent => HistoryEventFilterType::CloseEvent,
                } as i32,
                skip_archival: false,
            }))
            .await
            .map_err(|s| super::rpc("GetWorkflowExecutionHistory", s))?
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

struct Mapped {
    category: Category,
    group: GroupRef,
    role: Role,
    outcome: Outcome,
    subject: String,
    attempt: Option<i32>,
    failure: Option<CoreFailure>,
    fields: Vec<(&'static str, String)>,
    payloads: Vec<(String, Payload)>,
}

pub(crate) fn normalize_failure(f: Failure) -> CoreFailure {
    let application = match &f.failure_info {
        Some(FailureInfo::ApplicationFailureInfo(a)) => Some(a),
        _ => None,
    };
    CoreFailure {
        message: f.message,
        kind: application
            .map(|a| a.r#type.clone())
            .filter(|t| !t.is_empty()),
        source: Some(f.source).filter(|s| !s.is_empty()),
        stack_trace: Some(f.stack_trace).filter(|s| !s.is_empty()),
        non_retryable: application.is_some_and(|a| a.non_retryable),
        cause: f.cause.map(|c| Box::new(normalize_failure(*c))),
    }
}

fn handler_and_input(request: Option<UpdateRequest>) -> (String, Option<Payloads>) {
    match request.and_then(|r| r.input) {
        Some(input) => (input.name, input.args),
        None => (String::new(), None),
    }
}

fn or_id(name: String, id: &str) -> String {
    if name.is_empty() {
        id.to_string()
    } else {
        name
    }
}

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
        self.failure = f.map(normalize_failure);
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

    fn keyed_args(mut self, map: HashMap<String, Payloads>) -> Self {
        let mut keys: Vec<String> = map.keys().cloned().collect();
        keys.sort();
        for k in keys {
            let list = map[&k].clone();
            self = self.args(&k, Some(list));
        }
        self
    }

    fn arg(mut self, label: &str, p: Option<ProtoPayload>) -> Self {
        if let Some(raw) = p {
            self.payloads.push((label.to_string(), convert(raw)));
        }
        self
    }
}

pub(crate) fn convert(p: ProtoPayload) -> Payload {
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

pub fn normalize(e: HistoryEvent) -> NormalizedEvent {
    let id = e.event_id;
    let time = e
        .event_time
        .map(|t| t.seconds * 1000 + i64::from(t.nanos) / 1_000_000);
    let name = event_name(e.event_type().as_str_name());

    let m = match e.attributes {
        None => at(Category::Workflow, GroupRef::Workflow, Role::Continues),

        Some(Attributes::WorkflowExecutionStartedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Opens)
                .subject(a.workflow_type.map(|t| t.name).unwrap_or_default())
                .field(
                    "taskQueue",
                    a.task_queue.map(|q| q.name).unwrap_or_default(),
                )
                .field("attempt", a.attempt.to_string())
                .field("firstRunId", a.first_execution_run_id)
                .field("parentNamespace", a.parent_workflow_namespace)
                .field(
                    "parentWorkflowId",
                    a.parent_workflow_execution
                        .as_ref()
                        .map(|w| w.workflow_id.clone())
                        .unwrap_or_default(),
                )
                .field(
                    "parentRunId",
                    a.parent_workflow_execution
                        .map(|w| w.run_id)
                        .unwrap_or_default(),
                )
                .args("input", a.input)
                .args("lastCompletionResult", a.last_completion_result)
        }
        Some(Attributes::WorkflowExecutionCompletedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::Completed)
                .args("result", a.result)
        }
        Some(Attributes::WorkflowExecutionFailedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::Failed)
                .failed(a.failure)
        }
        Some(Attributes::WorkflowExecutionTimedOutEventAttributes(_)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes).ends(Outcome::TimedOut)
        }
        Some(Attributes::WorkflowExecutionCanceledEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::Canceled)
                .args("details", a.details)
        }
        Some(Attributes::WorkflowExecutionTerminatedEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::Terminated)
                .field("reason", a.reason)
                .field("identity", a.identity)
                .args("details", a.details)
        }
        Some(Attributes::WorkflowExecutionContinuedAsNewEventAttributes(a)) => {
            at(Category::Workflow, GroupRef::Workflow, Role::Closes)
                .ends(Outcome::ContinuedAsNew)
                .field("newRunId", a.new_execution_run_id)
                .args("input", a.input)
                .args("lastCompletionResult", a.last_completion_result)
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
                .args("input", a.input)
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
        .ends(Outcome::Canceled)
        .args("details", a.details),
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
        .ends(Outcome::Canceled)
        .args("details", a.details),
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

        Some(Attributes::WorkflowExecutionUpdateAdmittedEventAttributes(a)) => {
            let (name, input) = handler_and_input(a.request);
            at(Category::Update, GroupRef::Opened(id), Role::Opens)
                .subject(name)
                .args("input", input)
        }
        Some(Attributes::WorkflowExecutionUpdateAcceptedEventAttributes(a)) => {
            let (name, input) = handler_and_input(a.accepted_request);
            at(Category::Update, GroupRef::Opened(id), Role::Opens)
                .subject(or_id(name, &a.protocol_instance_id))
                .field("updateId", a.protocol_instance_id)
                .args("input", input)
        }
        Some(Attributes::WorkflowExecutionUpdateCompletedEventAttributes(a)) => {
            let m = at(
                Category::Update,
                GroupRef::Opened(a.accepted_event_id),
                Role::Closes,
            );
            match a.outcome.and_then(|o| o.value) {
                Some(OutcomeValue::Success(p)) => {
                    m.ends(Outcome::Completed).args("result", Some(p))
                }
                Some(OutcomeValue::Failure(f)) => m.ends(Outcome::Failed).failed(Some(f)),
                None => m.ends(Outcome::Completed),
            }
        }
        Some(Attributes::WorkflowExecutionUpdateRejectedEventAttributes(a)) => {
            let (name, input) = handler_and_input(a.rejected_request);
            at(Category::Update, GroupRef::Opened(id), Role::Opens)
                .subject(or_id(name, &a.protocol_instance_id))
                .field("updateId", a.protocol_instance_id)
                .args("input", input)
                .ends(Outcome::Rejected)
                .failed(a.failure)
        }

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
        .ends(Outcome::Completed)
        .arg("result", a.result),
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

        Some(Attributes::MarkerRecordedEventAttributes(a)) => {
            at(Category::Marker, GroupRef::Opened(id), Role::Opens)
                .subject(a.marker_name)
                .failed(a.failure)
                .keyed_args(a.details)
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

fn event_name(proto: &'static str) -> &'static str {
    proto.strip_prefix("EVENT_TYPE_").unwrap_or(proto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::ActivityType,
        enums::v1::EventType,
        failure::v1::ApplicationFailureInfo,
        history::v1::{
            ActivityTaskFailedEventAttributes, ActivityTaskScheduledEventAttributes,
            ActivityTaskStartedEventAttributes, MarkerRecordedEventAttributes,
            TimerFiredEventAttributes, TimerStartedEventAttributes,
            WorkflowExecutionCompletedEventAttributes, WorkflowExecutionSignaledEventAttributes,
            WorkflowExecutionStartedEventAttributes,
            WorkflowExecutionUpdateAcceptedEventAttributes,
            WorkflowExecutionUpdateAdmittedEventAttributes,
            WorkflowExecutionUpdateCompletedEventAttributes,
            WorkflowExecutionUpdateRejectedEventAttributes,
        },
        update::v1::{Input as UpdateInput, Outcome as UpdateOutcome},
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
        assert_eq!(
            n.failure.as_ref().map(|f| f.message.as_str()),
            Some("card declined")
        );
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
        assert_eq!(
            n.failure.as_ref().map(|f| f.message.as_str()),
            Some("out of retries")
        );
    }

    #[test]
    fn a_failure_keeps_its_type_stack_trace_and_every_cause_under_it() {
        let root = Failure {
            message: "Read timed out".into(),
            source: "JavaSDK".into(),
            stack_trace: "at java.net.SocketInputStream.read(SocketInputStream.java:171)".into(),
            ..Default::default()
        };
        let middle = Failure {
            message: "card declined".into(),
            stack_trace: "at com.example.shop.Charge.run(Charge.java:42)".into(),
            failure_info: Some(FailureInfo::ApplicationFailureInfo(
                ApplicationFailureInfo {
                    r#type: "PaymentDeclined".into(),
                    non_retryable: true,
                    ..Default::default()
                },
            )),
            cause: Some(Box::new(root)),
            ..Default::default()
        };
        let n = normalize(event(
            7,
            EventType::ActivityTaskFailed,
            Attributes::ActivityTaskFailedEventAttributes(ActivityTaskFailedEventAttributes {
                scheduled_event_id: 5,
                started_event_id: 6,
                failure: Some(Failure {
                    message: "activity task failed".into(),
                    cause: Some(Box::new(middle)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ));

        let f = n.failure.expect("the event carries a failure");
        assert_eq!(f.depth(), 3, "the whole chain must survive the mapping");
        assert_eq!(f.message, "activity task failed");

        let app = f.cause.as_deref().expect("the application failure");
        assert_eq!(app.kind.as_deref(), Some("PaymentDeclined"));
        assert!(app.non_retryable, "non-retryable is why it never came back");
        assert_eq!(app.headline(), "PaymentDeclined: card declined");
        assert!(
            app.stack_trace
                .as_deref()
                .unwrap()
                .contains("Charge.java:42")
        );

        let root = f.root();
        assert_eq!(root.message, "Read timed out");
        assert_eq!(root.source.as_deref(), Some("JavaSDK"));
        assert!(
            root.stack_trace.is_some(),
            "the root trace is the one wanted"
        );
    }

    #[test]
    fn an_absent_field_on_a_failure_stays_absent_rather_than_becoming_empty_text() {
        let n = normalize(event(
            7,
            EventType::ActivityTaskFailed,
            Attributes::ActivityTaskFailedEventAttributes(ActivityTaskFailedEventAttributes {
                scheduled_event_id: 5,
                started_event_id: 6,
                failure: Some(Failure {
                    message: "plain".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ));

        let f = n.failure.unwrap();
        assert_eq!(f.depth(), 1);
        assert_eq!(f.kind, None);
        assert_eq!(f.source, None);
        assert_eq!(f.stack_trace, None);
        assert!(!f.non_retryable);
        assert_eq!(f.headline(), "plain");
    }

    #[test]
    fn a_timer_is_grouped_by_its_started_event_not_a_scheduled_one() {
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
        let mut raw = json_payload("{}");
        raw.metadata.insert("type".to_string(), b"Keyword".to_vec());
        let p = convert(raw);
        assert_eq!(p.encoding, "json/plain");
        assert_eq!(p.type_hint.as_deref(), Some("Keyword"));
    }

    #[test]
    fn a_workflow_carries_its_own_input() {
        let n = normalize(event(
            1,
            EventType::WorkflowExecutionStarted,
            Attributes::WorkflowExecutionStartedEventAttributes(
                WorkflowExecutionStartedEventAttributes {
                    input: Some(Payloads {
                        payloads: vec![json_payload(r#"{"orderId":7}"#)],
                    }),
                    ..Default::default()
                },
            ),
        ));

        let (label, p) = &n.payloads[0];
        assert_eq!(label, "input");
        assert_eq!(
            p.render(),
            tmprl_core::payload::Rendered::Text("{\n  \"orderId\": 7\n}".into())
        );
    }

    #[test]
    fn a_workflow_carries_its_own_result() {
        let n = normalize(event(
            11,
            EventType::WorkflowExecutionCompleted,
            Attributes::WorkflowExecutionCompletedEventAttributes(
                WorkflowExecutionCompletedEventAttributes {
                    result: Some(Payloads {
                        payloads: vec![json_payload(r#""ok""#)],
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.outcome, Outcome::Completed);
        let labels: Vec<&str> = n.payloads.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["result"]);
    }

    #[test]
    fn a_signal_carries_its_argument() {
        let n = normalize(event(
            8,
            EventType::WorkflowExecutionSignaled,
            Attributes::WorkflowExecutionSignaledEventAttributes(
                WorkflowExecutionSignaledEventAttributes {
                    signal_name: "approve".into(),
                    input: Some(Payloads {
                        payloads: vec![json_payload("true")],
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.subject, "approve");
        assert_eq!(n.payloads.len(), 1);
    }

    #[test]
    fn a_marker_details_map_is_ordered_by_key() {
        let details = HashMap::from([
            (
                "side-effect-id".to_string(),
                Payloads {
                    payloads: vec![json_payload("1")],
                },
            ),
            (
                "data".to_string(),
                Payloads {
                    payloads: vec![json_payload("2")],
                },
            ),
        ]);
        let n = normalize(event(
            9,
            EventType::MarkerRecorded,
            Attributes::MarkerRecordedEventAttributes(MarkerRecordedEventAttributes {
                marker_name: "SideEffect".into(),
                details,
                ..Default::default()
            }),
        ));

        let labels: Vec<&str> = n.payloads.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["data", "side-effect-id"]);
    }

    #[test]
    fn an_accepted_update_is_named_by_its_handler_not_by_its_uuid() {
        let n = normalize(event(
            699,
            EventType::WorkflowExecutionUpdateAccepted,
            Attributes::WorkflowExecutionUpdateAcceptedEventAttributes(
                WorkflowExecutionUpdateAcceptedEventAttributes {
                    protocol_instance_id: "100c4af3-55f6-4ac8-9658-8b5354d66f08".into(),
                    accepted_request: Some(UpdateRequest {
                        input: Some(UpdateInput {
                            name: "data-forward-cancel".into(),
                            args: Some(Payloads {
                                payloads: vec![json_payload("{\"actor_id\":\"099018\"}")],
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.subject, "data-forward-cancel");
        assert_eq!(n.category, Category::Update);
        assert!(
            n.fields
                .iter()
                .any(|(k, v)| *k == "updateId" && v == "100c4af3-55f6-4ac8-9658-8b5354d66f08"),
            "the id must stay reachable: {:?}",
            n.fields
        );
        assert!(
            n.payloads.iter().any(|(label, _)| label == "input"),
            "the arguments travel with the name: {:?}",
            n.payloads
        );
    }

    #[test]
    fn a_rejected_update_says_which_handler_was_refused() {
        let n = normalize(event(
            8,
            EventType::WorkflowExecutionUpdateRejected,
            Attributes::WorkflowExecutionUpdateRejectedEventAttributes(
                WorkflowExecutionUpdateRejectedEventAttributes {
                    protocol_instance_id: "b2c3".into(),
                    rejected_request: Some(UpdateRequest {
                        input: Some(UpdateInput {
                            name: "data-forward-cancel".into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    failure: Some(Failure {
                        message: "no such handler".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.subject, "data-forward-cancel");
        assert_eq!(n.outcome, Outcome::Rejected);
    }

    #[test]
    fn an_admitted_update_is_named_too() {
        let n = normalize(event(
            5,
            EventType::WorkflowExecutionUpdateAdmitted,
            Attributes::WorkflowExecutionUpdateAdmittedEventAttributes(
                WorkflowExecutionUpdateAdmittedEventAttributes {
                    request: Some(UpdateRequest {
                        input: Some(UpdateInput {
                            name: "data-forward-cancel".into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.subject, "data-forward-cancel");
    }

    #[test]
    fn an_update_with_no_request_falls_back_to_the_id_rather_than_a_blank_row() {
        let n = normalize(event(
            9,
            EventType::WorkflowExecutionUpdateAccepted,
            Attributes::WorkflowExecutionUpdateAcceptedEventAttributes(
                WorkflowExecutionUpdateAcceptedEventAttributes {
                    protocol_instance_id: "a1b2".into(),
                    accepted_request: None,
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.subject, "a1b2", "a row has to say something");
    }

    #[test]
    fn an_update_that_the_workflow_rejected_is_not_reported_as_completed() {
        let n = normalize(event(
            12,
            EventType::WorkflowExecutionUpdateCompleted,
            Attributes::WorkflowExecutionUpdateCompletedEventAttributes(
                WorkflowExecutionUpdateCompletedEventAttributes {
                    outcome: Some(UpdateOutcome {
                        value: Some(OutcomeValue::Failure(Failure {
                            message: "not allowed in this state".into(),
                            ..Default::default()
                        })),
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.outcome, Outcome::Failed);
        assert_eq!(
            n.failure.as_ref().map(|f| f.message.as_str()),
            Some("not allowed in this state")
        );
        assert!(n.payloads.is_empty());
    }

    #[test]
    fn an_update_that_succeeded_carries_its_result() {
        let n = normalize(event(
            12,
            EventType::WorkflowExecutionUpdateCompleted,
            Attributes::WorkflowExecutionUpdateCompletedEventAttributes(
                WorkflowExecutionUpdateCompletedEventAttributes {
                    outcome: Some(UpdateOutcome {
                        value: Some(OutcomeValue::Success(Payloads {
                            payloads: vec![json_payload("42")],
                        })),
                    }),
                    ..Default::default()
                },
            ),
        ));

        assert_eq!(n.outcome, Outcome::Completed);
        let labels: Vec<&str> = n.payloads.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["result"]);
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
