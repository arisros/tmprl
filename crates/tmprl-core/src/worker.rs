//! Who runs a workflow: the SDK and its language, the last worker, its build, and the pollers.

use crate::history::{Category, NormalizedEvent, Outcome};
use crate::taskqueue::Poller;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Go,
    TypeScript,
    Python,
    Java,
    DotNet,
    Ruby,
    Php,
    Rust,
}

impl Language {
    pub fn of_sdk(sdk_name: &str) -> Option<Language> {
        let name = sdk_name.to_ascii_lowercase();
        let name = name.strip_prefix("temporal-").unwrap_or(&name);
        Some(match name {
            "go" => Language::Go,
            "typescript" | "ts" | "node" => Language::TypeScript,
            "python" => Language::Python,
            "java" => Language::Java,
            "dotnet" | ".net" | "csharp" => Language::DotNet,
            "ruby" => Language::Ruby,
            "php" => Language::Php,
            "rust" | "core" => Language::Rust,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Go => "Go",
            Language::TypeScript => "TypeScript",
            Language::Python => "Python",
            Language::Java => "Java",
            Language::DotNet => ".NET",
            Language::Ruby => "Ruby",
            Language::Php => "PHP",
            Language::Rust => "Rust",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerInfo {
    pub sdk_name: Option<String>,
    pub sdk_version: Option<String>,
    pub identity: Option<String>,
    pub build_id: Option<String>,
}

impl WorkerInfo {
    /// The SDK is recorded only when it changes, so the last one recorded is the current one.
    pub fn from_history(events: &[NormalizedEvent]) -> WorkerInfo {
        let mut info = WorkerInfo::default();
        let field = |e: &NormalizedEvent, name: &str| {
            e.fields
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone())
        };
        for e in events
            .iter()
            .filter(|e| e.category == Category::WorkflowTask && e.outcome == Outcome::Completed)
        {
            info.sdk_name = field(e, "sdkName").or(info.sdk_name);
            info.sdk_version = field(e, "sdkVersion").or(info.sdk_version);
            info.identity = field(e, "identity").or(info.identity);
            info.build_id = field(e, "buildId").or(info.build_id);
        }
        info
    }

    pub fn language(&self) -> Option<Language> {
        self.sdk_name.as_deref().and_then(Language::of_sdk)
    }

    pub fn is_empty(&self) -> bool {
        *self == WorkerInfo::default()
    }

    /// `icon` prefixes the language, `None` for a terminal without the glyphs.
    pub fn facts(
        &self,
        pollers: Option<&[Poller]>,
        now_ms: i64,
        icon: impl Fn(Language) -> Option<&'static str>,
    ) -> Vec<(&'static str, String)> {
        let mut facts = Vec::new();
        if let Some(sdk) = &self.sdk_name {
            let mut parts = Vec::new();
            if let Some(lang) = self.language() {
                parts.push(match icon(lang) {
                    Some(glyph) => format!("{glyph} {}", lang.name()),
                    None => lang.name().to_string(),
                });
            }
            parts.push(match &self.sdk_version {
                Some(v) => format!("{sdk} {v}"),
                None => sdk.clone(),
            });
            facts.push(("sdk", parts.join(" · ")));
        }
        if let Some(identity) = &self.identity {
            facts.push(("worker", identity.clone()));
        }
        if let Some(build) = &self.build_id {
            facts.push(("build", build.clone()));
        }
        if let Some(pollers) = pollers {
            facts.push(("pollers", pollers_line(pollers, now_ms)));
        }
        facts
    }
}

fn pollers_line(pollers: &[Poller], now_ms: i64) -> String {
    if pollers.is_empty() {
        return "none polling the task queue".into();
    }
    let mut sorted: Vec<&Poller> = pollers.iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.last_access_ms));
    let shown: Vec<String> = sorted
        .iter()
        .take(crate::info::MAX_LISTED)
        .map(|p| match p.last_access_ms {
            Some(at) => format!("{} ({} ago)", p.identity, crate::info::span(now_ms - at)),
            None => p.identity.clone(),
        })
        .collect();
    let more = pollers.len().saturating_sub(shown.len());
    let mut line = format!("{} · {}", pollers.len(), shown.join(", "));
    if more > 0 {
        line.push_str(&format!(" and {more} more"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{GroupRef, Role};

    fn completed(id: i64, fields: &[(&'static str, &str)]) -> NormalizedEvent {
        let mut e = NormalizedEvent::new(
            id,
            "WorkflowTaskCompleted",
            Category::WorkflowTask,
            GroupRef::Opened(id - 2),
            Role::Closes,
        );
        e.outcome = Outcome::Completed;
        e.fields = fields.iter().map(|(k, v)| (*k, v.to_string())).collect();
        e
    }

    #[test]
    fn the_sdk_is_the_last_one_recorded_and_the_worker_the_last_to_complete_a_task() {
        let events = [
            completed(
                4,
                &[
                    ("identity", "1@a@"),
                    ("sdkName", "temporal-go"),
                    ("sdkVersion", "1.25.0"),
                ],
            ),
            completed(10, &[("identity", "2@b@"), ("sdkVersion", "1.26.0")]),
            completed(16, &[("identity", "3@c@"), ("buildId", "v42")]),
        ];
        let info = WorkerInfo::from_history(&events);
        assert_eq!(info.sdk_name.as_deref(), Some("temporal-go"));
        assert_eq!(info.sdk_version.as_deref(), Some("1.26.0"));
        assert_eq!(info.identity.as_deref(), Some("3@c@"));
        assert_eq!(info.build_id.as_deref(), Some("v42"));
        assert_eq!(info.language(), Some(Language::Go));
        assert!(WorkerInfo::from_history(&[]).is_empty());
    }

    #[test]
    fn each_sdk_names_its_language() {
        for (sdk, lang) in [
            ("temporal-go", Language::Go),
            ("temporal-typescript", Language::TypeScript),
            ("temporal-python", Language::Python),
            ("temporal-java", Language::Java),
            ("temporal-dotnet", Language::DotNet),
            ("temporal-ruby", Language::Ruby),
            ("temporal-php", Language::Php),
        ] {
            assert_eq!(Language::of_sdk(sdk), Some(lang), "{sdk}");
        }
        assert_eq!(Language::of_sdk("my-own-sdk"), None);
    }

    #[test]
    fn the_facts_show_the_language_with_its_icon_and_the_freshest_pollers_first() {
        let info = WorkerInfo {
            sdk_name: Some("temporal-python".into()),
            sdk_version: Some("1.9.0".into()),
            identity: Some("7@host@".into()),
            build_id: None,
        };
        let poller = |identity: &str, at: i64| Poller {
            identity: identity.into(),
            last_access_ms: Some(at),
        };
        let pollers = [poller("old", 1_000), poller("new", 9_000)];
        let facts = info.facts(Some(&pollers), 10_000, |_| Some("P"));
        assert_eq!(facts[0], ("sdk", "P Python · temporal-python 1.9.0".into()));
        assert_eq!(facts[1], ("worker", "7@host@".into()));
        assert_eq!(facts[2].0, "pollers");
        assert!(
            facts[2].1.starts_with("2 · new (1s ago), old"),
            "{:?}",
            facts[2]
        );

        let plain = info.facts(Some(&[]), 0, |_| None);
        assert_eq!(plain[0].1, "Python · temporal-python 1.9.0");
        assert_eq!(plain[2].1, "none polling the task queue");
        assert!(
            info.facts(None, 0, |_| None)
                .iter()
                .all(|(k, _)| *k != "pollers")
        );
    }
}
