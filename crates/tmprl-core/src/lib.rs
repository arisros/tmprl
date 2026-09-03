//! Pure domain logic for tmprl.
//!
//! Nothing in this crate performs IO, touches a terminal, or is async. That is deliberate:
//! it is the layer where the difficult logic lives, so it is the layer that must be
//! trivially testable. If something here needs a runtime, it is in the wrong crate.

pub mod command;
pub mod config;
pub mod history;
pub mod key;
pub mod keymap;
pub mod loadable;
pub mod mode;
pub mod outline;
pub mod payload;
pub mod query;
pub mod workflow;

pub use command::{Action, Command, Registry};
pub use config::{ConfigError, SavedView};
pub use history::{Category, Group, GroupRef, NormalizedEvent, Outcome, Role};
pub use key::{Chord, ChordSeq, Key, KeyParseError, Mods};
pub use keymap::{Binding, Keymap, Pending, PendingEntry, Resolution, default_keymap};
pub use loadable::Loadable;
pub use mode::Mode;
pub use outline::{Outline, Row, Summary};
pub use payload::{Payload, Rendered};
pub use workflow::{StatusCounts, WorkflowList, WorkflowRow, WorkflowStatus};
