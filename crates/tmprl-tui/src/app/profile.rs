//! Connection profiles: listing the ones in `temporal.toml` and switching to one.

use tmprl_client::ProfileRef;
use tmprl_core::fault::Code;

use super::*;

#[derive(Debug, Clone)]
pub(super) struct Switch {
    seq: u64,
    name: String,
    env_override: Option<&'static str>,
}

const OVERRIDING_VARS: [&str; 2] = ["TEMPORAL_ADDRESS", "TEMPORAL_NAMESPACE"];

pub(super) fn env_override(env: bool, is_set: impl Fn(&str) -> bool) -> Option<&'static str> {
    if !env {
        return None;
    }
    OVERRIDING_VARS.into_iter().find(|v| is_set(v))
}

impl App {
    pub(super) fn epoch(&self) -> u64 {
        self.tx.epoch
    }

    pub fn set_temporal_config(&mut self, path: Option<String>) {
        self.temporal_config = path;
    }

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
        let env_override = env_override(resolved.env, |v| {
            std::env::var_os(v).is_some_and(|x| !x.is_empty())
        });
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
