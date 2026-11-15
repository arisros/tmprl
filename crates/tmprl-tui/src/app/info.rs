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
