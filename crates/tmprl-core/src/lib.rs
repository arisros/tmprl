//! Pure domain logic for tmprl.
//!
//! Nothing in this crate performs IO, touches a terminal, or is async. That is deliberate:
//! it is the layer where the difficult logic lives, so it is the layer that must be
//! trivially testable. If something here needs a runtime, it is in the wrong crate.

pub mod clock;
pub mod command;
pub mod config;
pub mod form;
pub mod fuzzy;
pub mod history;
pub mod jumplist;
pub mod key;
pub mod keymap;
pub mod loadable;
pub mod mode;
pub mod mutation;
pub mod outline;
pub mod payload;
pub mod picker;
pub mod query;
pub mod schedule;
pub mod search;
pub mod timerange;
pub mod workflow;

pub use clock::{Clock, TimeFormat};
pub use command::{Action, Command, PayloadPart, Registry};
pub use config::{CodecConfig, Config, ConfigError, SavedView};
pub use fuzzy::Match;
pub use history::{Category, Group, GroupRef, NormalizedEvent, Outcome, Role};
pub use jumplist::Jumplist;
pub use key::{Chord, ChordSeq, Key, KeyParseError, Mods};
pub use keymap::{Binding, Keymap, Pending, PendingEntry, Resolution, default_keymap};
pub use loadable::Loadable;
pub use mode::Mode;
pub use mutation::{Confirm, Mutation};
pub use outline::{Outline, Row, Summary};
pub use payload::{Payload, Rendered};
pub use picker::{Picker, Target};
pub use schedule::ScheduleRow;
pub use search::{Hit, Search};
pub use workflow::{StatusCounts, WorkflowList, WorkflowRow, WorkflowStatus};
