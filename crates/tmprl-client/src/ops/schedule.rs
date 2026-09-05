//! Listing schedules.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::{
    schedule::v1::ScheduleListEntry, workflowservice::v1::ListSchedulesRequest,
};
use tmprl_core::schedule::{Calendar, Interval, Range, ScheduleRow, describe_spec};

use super::OpError;
use crate::Conn;

#[derive(Debug, Clone, Default)]
pub struct SchedulePage {
    pub rows: Vec<ScheduleRow>,
    pub next_page_token: Vec<u8>,
}

impl SchedulePage {
    pub fn has_more(&self) -> bool {
        !self.next_page_token.is_empty()
    }
}

impl Conn {
    pub async fn list_schedules(
        &self,
        namespace: &str,
        page_size: i32,
        next_page_token: Vec<u8>,
    ) -> Result<SchedulePage, OpError> {
        let resp = self
            .wf()
            .list_schedules(Request::new(ListSchedulesRequest {
                namespace: namespace.to_string(),
                maximum_page_size: page_size,
                next_page_token,
                ..Default::default()
            }))
            .await
            .map_err(|s| OpError::rpc("ListSchedules", s))?
            .into_inner();

        let mut rows: Vec<ScheduleRow> = resp
            .schedules
            .into_iter()
            .map(|e| row_from(namespace, e))
            .collect();
        // ListSchedules gives no ordering guarantee either, and a list that reshuffles
        // between refreshes is unusable. Schedule ids are stable, so sort on them.
        rows.sort_by(|a, b| a.schedule_id.cmp(&b.schedule_id));

        Ok(SchedulePage {
            rows,
            next_page_token: resp.next_page_token,
        })
    }
}

fn ranges(rs: &[temporalio_common::protos::temporal::api::schedule::v1::Range]) -> Vec<Range> {
    rs.iter()
        .map(|r| Range {
            start: r.start,
            end: r.end,
            step: r.step,
        })
        .collect()
}

fn row_from(namespace: &str, e: ScheduleListEntry) -> ScheduleRow {
    let info = e.info.unwrap_or_default();
    let spec = info.spec.unwrap_or_default();

    let intervals: Vec<Interval> = spec
        .interval
        .iter()
        .map(|i| Interval {
            every_secs: i.interval.as_ref().map(|d| d.seconds).unwrap_or(0),
            offset_secs: i.phase.as_ref().map(|d| d.seconds).unwrap_or(0),
        })
        .collect();

    let calendars: Vec<Calendar> = spec
        .structured_calendar
        .iter()
        .map(|c| Calendar {
            second: ranges(&c.second),
            minute: ranges(&c.minute),
            hour: ranges(&c.hour),
            day_of_month: ranges(&c.day_of_month),
            month: ranges(&c.month),
            day_of_week: ranges(&c.day_of_week),
        })
        .collect();

    ScheduleRow {
        namespace: namespace.to_string(),
        schedule_id: e.schedule_id,
        workflow_type: info.workflow_type.map(|t| t.name).unwrap_or_default(),
        paused: info.paused,
        notes: info.notes,
        spec: describe_spec(&spec.cron_string, &calendars, &intervals),
        // The server returns future times ascending, but sorting costs nothing and a
        // "next run" that is not the earliest would be wrong rather than merely odd.
        next_run: info
            .future_action_times
            .iter()
            .map(|t| t.seconds * 1000 + i64::from(t.nanos) / 1_000_000)
            .min(),
        recent_runs: info.recent_actions.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::WorkflowType,
        schedule::v1::{ScheduleListInfo, ScheduleSpec},
    };

    fn entry(info: ScheduleListInfo) -> ScheduleListEntry {
        ScheduleListEntry {
            schedule_id: "nightly".into(),
            info: Some(info),
            ..Default::default()
        }
    }

    #[test]
    fn a_schedule_maps_its_spec_and_state() {
        let row = row_from(
            "payments",
            entry(ScheduleListInfo {
                workflow_type: Some(WorkflowType {
                    name: "Reconcile".into(),
                }),
                paused: true,
                notes: "held during migration".into(),
                spec: Some(ScheduleSpec {
                    cron_string: vec!["0 2 * * *".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        );

        assert_eq!(row.namespace, "payments");
        assert_eq!(row.schedule_id, "nightly");
        assert_eq!(row.workflow_type, "Reconcile");
        assert!(row.paused);
        assert_eq!(row.spec, "0 2 * * *");
        assert_eq!(row.notes, "held during migration");
    }

    #[test]
    fn the_next_run_is_the_earliest_future_time() {
        let row = row_from(
            "d",
            entry(ScheduleListInfo {
                future_action_times: vec![
                    prost_wkt_types::Timestamp {
                        seconds: 300,
                        nanos: 0,
                    },
                    prost_wkt_types::Timestamp {
                        seconds: 100,
                        nanos: 0,
                    },
                ],
                ..Default::default()
            }),
        );
        assert_eq!(row.next_run, Some(100_000));
    }

    #[test]
    fn a_schedule_with_nothing_set_still_maps() {
        // Every field of the entry is optional on the wire, and one malformed row must not
        // take down the list.
        let row = row_from("d", ScheduleListEntry::default());
        assert_eq!(row.spec, "manual");
        assert_eq!(row.next_run, None);
        assert!(!row.paused);
    }
}
