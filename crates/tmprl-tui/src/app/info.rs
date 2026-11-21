//! The workflow info block: the describe reply and `gi`.

use tmprl_client::Described;

use super::*;

impl App {
    pub(super) fn toggle_info(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some((
                "workflow info is shown on a history, open one first".into(),
                Note::Warn,
            ));
            return;
        }
        self.view.show_info = !self.view.show_info;
        self.maybe_decode();
    }

    fn take_status(&mut self, info: &tmprl_core::info::WorkflowInfo) {
        let Some(row) = self.view.viewing.as_mut() else {
            return;
        };
        if row.run_id != info.run_id {
            return;
        }
        row.status = info.status;
        row.close_time = info.close_time.or(row.close_time);
        row.history_length = info.history_length.max(row.history_length);
        let (namespace, task_queue) = (row.namespace.clone(), row.task_queue.clone());
        if let Some(list) = self.view.workflows.value_mut() {
            list.update_run(
                (&namespace, &info.run_id),
                info.status,
                info.close_time,
                info.history_length,
            );
        }
        if info.status == tmprl_core::WorkflowStatus::Running && !self.view.pollers_asked {
            self.load_pollers(&namespace, &task_queue);
        }
    }

    pub(super) fn described(&mut self, generation: u64, result: Result<Box<Described>, Fault>) {
        if generation != self.view.generation {
            return;
        }
        match result.map(|d| *d) {
            Ok(Described { mut info, pending }) => {
                for p in info.payloads_mut() {
                    if let Some(plain) = self.decoded.get(&Self::payload_key(p)) {
                        *p = plain.clone();
                    }
                }
                self.take_status(&info);
                self.view.pending = pending;
                self.view.info = Loadable::loaded(info);
                self.maybe_decode();
            }
            Err(e) => {
                if self.view.info.value().is_none() {
                    self.view.info = Loadable::Failed(e.clone());
                }
                self.fail_as(format!("describe: {e}"), e, Note::Warn);
            }
        }
    }
}
