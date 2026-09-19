//! Payloads: decoding through the codec server, the `!` pipe, and `$EDITOR`.

use super::*;

impl App {
    /// Identity of an encrypted payload, for the decode cache.
    ///
    /// The ciphertext plus its encoding: the same bytes decode to the same value, so a row
    /// revisited costs nothing, and two different payloads cannot collide on content alone.
    pub(super) fn payload_key(p: &Payload) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        p.encoding.hash(&mut h);
        p.data.hash(&mut h);
        h.finish()
    }

    /// What the pane should say about an encrypted payload.
    ///
    /// "Needs a codec server" is only true when none is configured; once one is, the honest
    /// answer is that a request is out.
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

    /// Ask the codec server about anything encrypted under the cursor.
    ///
    /// Lazy on purpose: only what the pane is actually showing. Decoding a whole history
    /// up front would be thousands of round trips for values nobody looked at.
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
                .map(|out| keys.into_iter().zip(out).collect::<Vec<_>>())
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Decoded(result));
        });
    }

    /// Swap every decoded payload into the history in place.
    ///
    /// Replacing the payload rather than keeping a cache the views consult means everything
    /// downstream (the pane, `!` piping, yanking) reads the plaintext without knowing a
    /// codec exists. It is also why this runs after each history page: a later page can
    /// carry the same encrypted value.
    pub(super) fn apply_decoded(&mut self) {
        if self.decoded.is_empty() {
            return;
        }
        let mut changed = false;
        for event in &mut self.view.history_events {
            for (_, p) in &mut event.payloads {
                if !p.needs_codec() {
                    continue;
                }
                if let Some(plain) = self.decoded.get(&Self::payload_key(p)) {
                    *p = plain.clone();
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
        let groups = group_events(&self.view.history_events);
        let events = self.view.history_events.clone();
        match self.view.history.value_mut() {
            Some(outline) => outline.replace(events, groups),
            None => self.view.history = Loadable::loaded(Outline::new(events, groups)),
        }
    }

    /// The payloads the cursor is on, as one JSON object.
    ///
    /// For a group that is its input *and* its result, which live on two different events,
    /// the same pair the payload pane shows.
    pub(super) fn payloads_under_cursor(&self) -> Vec<(String, tmprl_core::payload::Payload)> {
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

    /// Open the `!` prompt, if there is anything under the cursor worth piping.
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
            // Encrypted or binary. Piping it produces a parse error that explains nothing,
            // so refuse with a reason instead.
            self.note = Some((
                "no readable payload here, encrypted or binary".into(),
                Note::Warn,
            ));
            return;
        }
        self.prompt = Some(Prompt {
            kind: PromptKind::Pipe,
            // Pre-filled: `jq` is what this is for, and an empty prompt makes you type the
            // same three characters every time.
            buf: "jq .".into(),
        });
        self.mode = Mode::Command;
    }

    /// Run the typed command with the focused payloads on stdin.
    ///
    /// Spawned, never awaited here, an external command can take as long as it likes and
    /// must not be able to freeze a keystroke. The output arrives as a `Msg`.
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

        // The output replaces the pane, so open it if it is shut, otherwise the result
        // would land somewhere the reader cannot see.
        self.view.show_detail = true;
        self.view.detail_scroll = 0;
        self.view.piped = Some(Ok(format!("running `{command}`…")));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = pipe_through(&command, json.into_bytes()).await;
            let _ = tx.send(Msg::Piped(result));
        });
    }

    /// Write the payloads under the cursor to a file and ask the loop to open it.
    ///
    /// The same JSON object `!` pipes, for one representation rather than two: `.result`
    /// means the same thing whether it is going to `jq` or to your editor.
    ///
    /// A **copy**, and the message says so. Nothing can be written back, Temporal history is
    /// immutable and a payload is not a document. Letting an editor open it and then
    /// silently discarding what was typed would be the worst of both, so the note is the
    /// honest part of the feature, not an afterthought.
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

        // A fresh directory per invocation, mode 0700, with the payload inside it at 0600.
        //
        // Not `temp_dir().join("tmprl-<run>-<row>.json")`, which was the first attempt and
        // is wrong twice over on a shared box: the name is derivable from a run id, so
        // another user can pre-create it as a symlink and have `fs::write` follow it, and
        // the file is created 0644, leaving decoded payloads world-readable in /tmp. A
        // unique directory removes the guess and the mode removes the audience.
        let dir = std::env::temp_dir().join(format!("tmprl-{}", uuid::Uuid::new_v4()));
        if let Err(e) = create_private_dir(&dir) {
            self.note = Some((
                format!("could not create {}: {e}", dir.display()),
                Note::Error,
            ));
            return;
        }
        // Named after the run so a stack of editor tabs is still navigable, and suffixed
        // `.json` so the editor picks its own highlighting without being told.
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

        // Carried on the request rather than set as a note here: the event loop takes the
        // request before the next draw, and `finish_edit` writes the note afterwards, so a
        // note set now is overwritten without ever being shown.
        let mut what = "a read-only copy; edits are not saved back".to_string();
        if !skipped.is_empty() {
            what = format!("{what}; without {} (not readable)", skipped.join(", "));
        }
        self.editing = Some(EditRequest { path, dir, what });
    }

    /// Hand the pending edit to the caller that owns the terminal.
    pub fn take_edit_request(&mut self) -> Option<EditRequest> {
        self.editing.take()
    }

    /// Report how the editor went, once the terminal is back.
    pub fn finish_edit(&mut self, req: &EditRequest, error: Option<String>) {
        self.dirty = true;
        // The copy goes away with the editor. Leaving it would accumulate readable
        // payloads in the temp directory for the length of the session, and the file was
        // never a document anyone can save.
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

/// Create a directory only this user can enter.
///
/// The mode is set at creation on Unix rather than afterwards, so there is no window in
/// which the directory exists with the default permissions.
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

/// Write a file only this user can read, failing if it already exists.
///
/// `create_new` rather than `create`: it refuses to follow a symlink someone else planted,
/// which is the attack the private directory already makes impractical and this closes
/// outright.
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
