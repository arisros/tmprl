//! The dashboard: panels over a scope, the layout they sit in, and `dashboard.toml`.
//!
//! A panel does not fetch. It reads from a [`Source`], and panels asking for the same thing
//! share one, so "recent failures" and "failing types" are one request and always agree.

mod board;
mod compose;
#[cfg(test)]
mod fixtures;
mod layout;
mod pacer;
mod parse;
mod source;

pub use board::{Anchor, Board, Drill};
pub use compose::{Facts, Slot, compose};
pub use layout::{
    DEFAULT_LIMIT, Layout, MAX_LIMIT, MAX_PANELS, PanelKind, PanelSpec, RowSpec, Show, Size,
    TimeField, Window,
};
pub use pacer::{Outcome, Pacer};
pub use parse::parse_dashboard;
pub use source::{
    Item, MAX_QUEUES, MAX_TALLIES, QueueRef, Source, SourceData, discover_queues, tally_types,
};
