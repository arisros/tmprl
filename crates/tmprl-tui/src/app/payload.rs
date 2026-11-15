//! Payloads: decoding through the codec server, the `!` pipe, and `$EDITOR`.

use super::*;

impl App {
    pub(super) fn payload_key(p: &Payload) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        p.encoding.hash(&mut h);
        p.data.hash(&mut h);
        h.finish()
    }

    pub fn decode_state(&self, p: &Payload) -> DecodeState {
        let key = Self::payload_key(p);
        if self.codec.is_none() {
            DecodeState::NoCodec
        } else if self.decoding.contains(&key) {
            DecodeState::InFlight
        } else if let Some(why) = self.decode_failed.get(&key) {
            DecodeState::Failed(why.clone())
        } else {
            DecodeState::Idle
        }
    }

    pub fn shown_text(&self, p: &Payload) -> Option<Arc<ShownText>> {
        let key = Self::payload_key(p);
        if let Some(hit) = self.shown.borrow().get(&key) {
            return Some(hit.clone());
        }
        let tmprl_core::payload::Rendered::Text(text) = p.render() else {
            return None;
        };
        let shown = Arc::new(cut_for_pane(&text));
        let mut cache = self.shown.borrow_mut();
        if cache.len() >= SHOWN_CACHE {
            cache.clear();
        }
        cache.insert(key, shown.clone());
        Some(shown)
    }

    pub(super) fn maybe_decode(&mut self) {
        if !self.view.show_detail || self.view.screen != Screen::History {
            return;
        }
        let Some(codec) = self.codec.clone() else {
            return;
        };

        let wanted: Vec<Payload> = self
            .payloads_under_cursor()
            .into_iter()
            .map(|(_, p)| p)
            .filter(|p| p.needs_codec())
            .filter(|p| {
                let key = Self::payload_key(p);
                !self.decoded.contains_key(&key) && !self.decoding.contains(&key)
            })
            .collect();
        if wanted.is_empty() {
            return;
        }

        for p in &wanted {
            self.decoding.insert(Self::payload_key(p));
        }
        let keys: Vec<u64> = wanted.iter().map(Self::payload_key).collect();
        let namespace = self
            .view
            .viewing
            .as_ref()
            .map(|w| w.namespace.clone())
            .unwrap_or_default();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let result = codec
                .decode(&namespace, &wanted)
                .await
                .map(|out| keys.into_iter().zip(out).collect::<Vec<_>>());
            let _ = tx.send(Msg::Decoded(result));
        });
    }

    pub(super) fn apply_decoded(&mut self) {
        if self.decoded.is_empty() {
            return;
        }
        apply_decoded_to(&self.decoded, &mut self.view);
        for view in self.parked.values_mut() {
            apply_decoded_to(&self.decoded, view);
        }
    }

    pub(super) fn decode_from_cache(&self, events: &mut [tmprl_core::history::NormalizedEvent]) {
        if self.decoded.is_empty() {
            return;
        }
        for event in events {
            swap_decoded(&self.decoded, &mut event.payloads);
        }
    }

    pub(super) fn payloads_under_cursor(&self) -> Vec<(String, tmprl_core::payload::Payload)> {
        if let Some(queried) = &self.view.queried {
            return queried.clone();
        }
        let Some(outline) = self.view.history.value() else {
            return Vec::new();
        };
        match outline.row_at(self.view.cursor) {
            Some(Row::Event { event, .. }) => outline
                .event(event)
                .map(|e| e.payloads.clone())
                .unwrap_or_default(),
            Some(Row::Group { group, .. }) => {
                let Some(g) = outline.group(group) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                for id in [g.events.first(), g.events.last()].into_iter().flatten() {
                    if let Some(e) = outline.events().iter().find(|e| e.id == *id) {
                        out.extend(e.payloads.iter().cloned());
                    }
                }
                out
            }
            None => Vec::new(),
        }
    }

    pub(super) fn open_pipe(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("piping applies to a workflow history".into(), Note::Warn));
            return;
        }
        let payloads = self.payloads_under_cursor();
        if payloads.is_empty() {
            self.note = Some(("nothing here to pipe".into(), Note::Warn));
            return;
        }
        if tmprl_core::payload::payloads_as_json(&payloads).0.is_none() {
            self.note = Some((
                "no readable payload here, encrypted or binary".into(),
                Note::Warn,
            ));
            return;
        }
        self.prompt = Some(Prompt {
            kind: PromptKind::Pipe,
            buf: "jq .".into(),
        });
        self.mode = Mode::Command;
    }

    pub(super) fn run_pipe(&mut self, command: String) {
        let (json, skipped) = tmprl_core::payload::payloads_as_json(&self.payloads_under_cursor());
        let Some(json) = json else {
            self.note = Some(("nothing readable to pipe".into(), Note::Warn));
            return;
        };
        if !skipped.is_empty() {
            self.note = Some((
                format!("piping without {} (not readable)", skipped.join(", ")),
                Note::Warn,
            ));
        }

        self.view.show_detail = true;
        self.view.detail_scroll = 0;
        self.view.piped = Some(Ok(format!("running `{command}`…")));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = pipe_through(&command, json.into_bytes()).await;
            let _ = tx.send(Msg::Piped(result));
        });
    }

    pub(super) fn open_editor(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some(("payloads live in a workflow history".into(), Note::Warn));
            return;
        }
        let (json, skipped) = tmprl_core::payload::payloads_as_json(&self.payloads_under_cursor());
        let Some(json) = json else {
            self.note = Some(("nothing readable here to open".into(), Note::Warn));
            return;
        };

        let dir = std::env::temp_dir().join(format!("tmprl-{}", uuid::Uuid::new_v4()));
        if let Err(e) = create_private_dir(&dir) {
            self.note = Some((
                format!("could not create {}: {e}", dir.display()),
                Note::Error,
            ));
            return;
        }
        let stem = self
            .view
            .viewing
            .as_ref()
            .map(|w| w.run_id.clone())
            .unwrap_or_else(|| "payload".into());
        let path = dir.join(format!("{stem}.json"));
        if let Err(e) = write_private_file(&path, json.as_bytes()) {
            let _ = std::fs::remove_dir_all(&dir);
            self.note = Some((
                format!("could not write {}: {e}", path.display()),
                Note::Error,
            ));
            return;
        }

        let mut what = "a read-only copy; edits are not saved back".to_string();
        if !skipped.is_empty() {
            what = format!("{what}; without {} (not readable)", skipped.join(", "));
        }
        self.editing = Some(EditRequest { path, dir, what });
    }

    pub(super) fn open_source(&mut self) {
        if self.view.screen != Screen::History {
            self.note = Some((
                "the source of an event is found from a workflow history".into(),
                Note::Warn,
            ));
            return;
        }
        let Some(command) = self.source_command.clone() else {
            self.note = Some((
                "no `[source] command` in config.toml to ask where the code is".into(),
                Note::Warn,
            ));
            return;
        };
        let Some((program, args)) =
            tmprl_core::source::argv(&command, std::env::var("HOME").ok().as_deref())
        else {
            return;
        };
        let (Some(workflow), Some(outline)) =
            (self.view.viewing.as_ref(), self.view.history.value())
        else {
            return;
        };
        let (group, focused) = match outline.row_at(self.view.cursor) {
            Some(Row::Group { group, .. }) => (group, None),
            Some(Row::Event { group, event }) => (group, outline.event(event)),
            None => {
                self.note = Some(("nothing here to find the source of".into(), Note::Warn));
                return;
            }
        };
        let Some(group) = outline.group(group) else {
            return;
        };
        let events: Vec<&tmprl_core::history::NormalizedEvent> = outline
            .events()
            .iter()
            .filter(|e| group.events.contains(&e.id))
            .collect();
        let payloads: Vec<(String, tmprl_core::payload::Payload)> = events
            .iter()
            .flat_map(|e| e.payloads.iter().cloned())
            .collect();
        let payloads = tmprl_core::payload::payloads_as_json(&payloads).0;
        let request = tmprl_core::source::request(
            &self.profile,
            workflow,
            group,
            &events,
            focused,
            payloads.as_deref(),
        )
        .to_string();

        self.note = Some((
            format!("asking {program} where `{}` comes from…", group.subject),
            Note::Info,
        ));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = run_resolver(&program, &args, request.into_bytes()).await;
            let _ = tx.send(Msg::Source(result));
        });
    }

    pub(super) fn source_reply(&mut self, result: Result<String, String>) {
        match result.and_then(|out| tmprl_core::source::locate(&out)) {
            Ok(at) if at.path.is_file() => {
                self.opening = Some(SourceRequest {
                    path: at.path,
                    line: at.line,
                    root: at.root.filter(|dir| dir.is_dir()),
                })
            }
            Ok(at) => {
                self.note = Some((
                    format!("source: {} is not a file", at.path.display()),
                    Note::Error,
                ))
            }
            Err(why) => self.note = Some((format!("source: {why}"), Note::Error)),
        }
    }

    pub fn take_source_request(&mut self) -> Option<SourceRequest> {
        self.opening.take()
    }

    pub fn finish_source(&mut self, req: &SourceRequest, error: Option<String>) {
        self.dirty = true;
        self.note = Some(match error {
            Some(e) => (format!("editor: {e}"), Note::Error),
            None => (format!("closed {}", req.path.display()), Note::Info),
        });
    }

    pub fn take_edit_request(&mut self) -> Option<EditRequest> {
        self.editing.take()
    }

    pub fn finish_edit(&mut self, req: &EditRequest, error: Option<String>) {
        self.dirty = true;
        let _ = std::fs::remove_dir_all(&req.dir);
        self.note = Some(match error {
            Some(e) => (format!("editor: {e}"), Note::Error),
            None => (
                format!("closed {} — {}", req.path.display(), req.what),
                Note::Info,
            ),
        });
    }
}

const SHOWN_CACHE: usize = 64;
pub(crate) const PANE_MAX_LINES: usize = 5_000;
const PANE_MAX_LINE_CHARS: usize = 4_000;

pub(crate) fn cut_for_pane(text: &str) -> ShownText {
    let mut lines = Vec::new();
    let mut total = 0;
    for line in text.lines() {
        total += 1;
        if lines.len() >= PANE_MAX_LINES {
            continue;
        }
        lines.push(cut_line(line));
    }
    ShownText {
        hidden: total - lines.len(),
        lines,
    }
}

fn cut_line(line: &str) -> String {
    match line.char_indices().nth(PANE_MAX_LINE_CHARS) {
        None => line.to_string(),
        Some((at, _)) => {
            let rest = line[at..].chars().count();
            format!("{}… {rest} more characters", &line[..at])
        }
    }
}

fn apply_decoded_to(decoded: &HashMap<u64, Payload>, view: &mut View) {
    for queried in view.queried.iter_mut() {
        swap_decoded(decoded, queried);
    }
    let mut changed = false;
    for event in &mut view.history_events {
        changed |= swap_decoded(decoded, &mut event.payloads);
    }
    if !changed {
        return;
    }
    match view.history.value_mut() {
        Some(outline) => {
            for p in outline.payloads_mut() {
                if !p.needs_codec() {
                    continue;
                }
                if let Some(plain) = decoded.get(&App::payload_key(p)) {
                    *p = plain.clone();
                }
            }
        }
        None => {
            let events = view.history_events.clone();
            let groups = group_events(&events);
            view.history = Loadable::loaded(Outline::new(events, groups));
        }
    }
}

fn swap_decoded(decoded: &HashMap<u64, Payload>, payloads: &mut [(String, Payload)]) -> bool {
    let mut changed = false;
    for (_, p) in payloads {
        if !p.needs_codec() {
            continue;
        }
        if let Some(plain) = decoded.get(&App::payload_key(p)) {
            *p = plain.clone();
            changed = true;
        }
    }
    changed
}

pub(super) fn create_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}

// create_new refuses to follow a planted symlink.
pub(super) fn write_private_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}
