//! Connection profiles: listing the ones in `temporal.toml` and switching to one.
//!
//! A switch is global. Every pane, the jumplist and every cache belong to the cluster they
//! were filled from, so the session starts over on the new one: one pane, on its namespace
//! list. The old connection stays in use until the new one has connected, so a profile that
//! cannot be reached leaves you where you were.

use tmprl_client::ProfileRef;
use tmprl_core::fault::Code;

use super::*;

/// A switch that has not connected yet.
#[derive(Debug, Clone)]
pub(super) struct Switch {
    seq: u64,
    name: String,
    /// A `TEMPORAL_*` variable the loader will layer over the profile, named in the note,
    /// since otherwise every profile quietly reaches the same cluster.
    env_override: Option<&'static str>,
}

const OVERRIDING_VARS: [&str; 2] = ["TEMPORAL_ADDRESS", "TEMPORAL_NAMESPACE"];

impl App {
    pub(super) fn epoch(&self) -> u64 {
        self.tx.epoch
    }

    /// The file `--temporal-config` named, which every profile is read from.
    pub fn set_temporal_config(&mut self, path: Option<String>) {
        self.temporal_config = path;
    }

    /// What `config.toml` says about the connected profile.
    pub(super) fn apply_profile_config(&mut self) {
        let resolved = self
            .config
            .as_ref()
            .map(|c| c.resolve(&self.profile))
            .unwrap_or_default();
        self.codec = resolved
            .codec
            .map(|c| Arc::new(Codec::new(c.endpoint, c.auth)));
        self.accent = resolved.accent;
        self.readonly = resolved.readonly;
    }

    pub fn profile_rows(&self) -> &[ProfileSummary] {
        self.view.profile_rows()
    }

    pub(super) fn load_profiles(&mut self) {
        // Detached, nothing is fetched, the file included.
        if self.conn.is_none() {
            return;
        }
        self.view.profiles.begin_refresh();
        let (tx, file) = (self.tx.clone(), self.temporal_config.clone());
        tokio::task::spawn_blocking(move || {
            let result = tmprl_client::list_profiles(file.as_deref()).map_err(|e| Fault {
                operation: "",
                code: Code::Local,
                message: e.to_string(),
            });
            let _ = tx.send(Msg::Profiles(result));
        });
    }

    pub(super) fn on_profiles(&mut self, result: Result<Vec<ProfileSummary>, Fault>) {
        match result {
            Ok(mut list) => {
                // Connected through the environment alone, the profile is in no file, and a
                // list without the one you are on reads as if you were nowhere.
                if !list.iter().any(|p| p.name == self.profile) {
                    list.insert(
                        0,
                        ProfileSummary {
                            name: self.profile.clone(),
                            address: Some(self.address.clone()),
                            namespace: Some(self.namespace.clone()),
                        },
                    );
                }
                let first = self.view.profiles.value().is_none();
                self.view.profiles = Loadable::loaded(list);
                if self.view.screen == Screen::Profiles {
                    // The screen opened before the list existed, so its cursor could not
                    // be put on the connected profile then.
                    if first {
                        self.view.cursor = self.connected_profile_row();
                    }
                    self.clamp_cursor();
                }
                if std::mem::take(&mut self.profile_picker_pending) {
                    self.open_picker(picker::Kind::Profiles);
                }
            }
            Err(e) => {
                self.profile_picker_pending = false;
                self.fail(e.clone(), Note::Error);
                self.view.profiles = Loadable::Failed(e);
            }
        }
    }

    /// `<leader>P`. The list is read off disk, so the first press may have to wait for it.
    pub(super) fn find_profile(&mut self) {
        if self.view.profiles.value().is_some() {
            self.open_picker(picker::Kind::Profiles);
            return;
        }
        self.profile_picker_pending = true;
        self.note = Some(("loading profiles…".into(), Note::Info));
        self.load_profiles();
    }

    pub(super) fn profile_items(&self) -> Vec<picker::Item> {
        self.profile_rows()
            .iter()
            .map(|p| {
                let address = p.address.as_deref().unwrap_or("localhost:7233");
                let namespace = p.namespace.as_deref().unwrap_or("default");
                let readonly = self
                    .config
                    .as_ref()
                    .is_some_and(|c| c.resolve(&p.name).readonly);
                picker::Item::new(p.name.clone(), Target::Profile(p.name.clone()))
                    .with_note(if p.name == self.profile {
                        "connected"
                    } else {
                        ""
                    })
                    .with_preview(format!(
                        "profile     {}\naddress     {address}\nnamespace   {namespace}\nreadonly    {}",
                        p.name,
                        if readonly { "yes" } else { "no" },
                    ))
            })
            .collect()
    }

    fn connected_profile_row(&self) -> usize {
        self.profile_rows()
            .iter()
            .position(|p| p.name == self.profile)
            .unwrap_or(0)
    }

    /// The profile list, one level above the namespaces.
    pub(super) fn open_profiles(&mut self) {
        self.mark_jump();
        self.view.namespace_cursor = self.view.cursor;
        self.view.anchor = None;
        self.mode = Mode::Normal;
        self.view.screen = Screen::Profiles;
        self.view.cursor = self.connected_profile_row();
        if self.view.profiles.value().is_none() {
            self.load_profiles();
        }
    }

    pub(super) fn open_focused_profile(&mut self) {
        let Some(name) = self
            .profile_rows()
            .get(self.view.cursor)
            .map(|p| p.name.clone())
        else {
            self.note = Some(("nothing to open".into(), Note::Warn));
            return;
        };
        self.switch_profile(&name);
    }

    pub(super) fn switch_profile(&mut self, name: &str) {
        if name == self.profile {
            // The one already connected: the switch is a step back down to its namespaces.
            self.switching = None;
            if self.view.screen == Screen::Profiles {
                self.mark_jump();
                self.view.screen = Screen::Namespaces;
                self.view.cursor = self.view.namespace_cursor;
                self.clamp_cursor();
                if self.view.namespaces.value().is_none() {
                    self.load_namespaces();
                }
            } else {
                self.note = Some((format!("already on {name}"), Note::Info));
            }
            return;
        }
        let resolved = self
            .config
            .as_ref()
            .map(|c| c.resolve(name))
            .unwrap_or_default();
        let env_override = resolved
            .env
            .then(|| {
                OVERRIDING_VARS
                    .into_iter()
                    .find(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
            })
            .flatten();
        self.switch_seq += 1;
        let seq = self.switch_seq;
        self.switching = Some(Switch {
            seq,
            name: name.to_string(),
            env_override,
        });
        self.note = Some((format!("connecting to {name}…"), Note::Info));
        if self.conn.is_none() {
            return;
        }
        // `--address` and `-n` were for the profile tmprl started on, not this one.
        let profile = ProfileRef {
            name: Some(name.to_string()),
            config_file: self.temporal_config.clone(),
            address: None,
            namespace: None,
            api_key: resolved.api_key,
            ignore_env: !resolved.env,
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Conn::connect(&profile)
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Connected { seq, result });
        });
    }

    pub(super) fn on_connected(&mut self, seq: u64, result: Result<Box<Conn>, String>) {
        let Some(switch) = self.switching.take_if(|s| s.seq == seq) else {
            return;
        };
        match result {
            Err(e) => {
                self.note = Some((
                    format!("{}: {e}; still on {}", switch.name, self.profile),
                    Note::Error,
                ));
            }
            Ok(conn) => {
                let (address, namespace) =
                    (conn.address().to_string(), conn.namespace().to_string());
                self.adopt(
                    switch.name.clone(),
                    address,
                    namespace,
                    Some(Arc::new(*conn)),
                );
                self.note = Some(match switch.env_override {
                    Some(var) => (
                        format!(
                            "profile: {} ({var} is set and overrides it; env = false in [profile.{}] ignores it)",
                            switch.name, switch.name
                        ),
                        Note::Warn,
                    ),
                    None => (format!("profile: {}", switch.name), Note::Info),
                });
            }
        }
    }

    /// Start the session over on a new connection.
    pub(super) fn adopt(
        &mut self,
        profile: String,
        address: String,
        namespace: String,
        conn: Option<Arc<Conn>>,
    ) {
        self.tx.epoch += 1;
        self.conn = conn;
        self.profile = profile;
        self.address = address;
        self.namespace = namespace;
        self.apply_profile_config();

        let profiles = std::mem::take(&mut self.view.profiles);
        let id = self.fresh_view_id();
        self.tabs = Tabs::new(id);
        // Dropping a view stops its follow poll and its dashboard requests.
        self.parked.clear();
        self.view = View::new(&self.namespace);
        self.view.profiles = profiles;

        self.jumps = Jumplist::default();
        self.mode = Mode::Normal;
        self.pending = Pending::default();
        self.picker = None;
        self.picker_found.clear();
        self.profile_picker_pending = false;
        self.prompt = None;
        self.confirm = None;
        self.form = None;
        self.completion = None;
        self.scan = None;
        self.decoded.clear();
        self.decoding.clear();
        self.decode_failed.clear();
        self.search_attributes = Loadable::default();
        self.attributes_for = None;
        self.load_namespaces();
    }
}
