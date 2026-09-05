//! All network IO for tmprl. Knows nothing about the UI.
//!
//! Every RPC goes through the thin wrappers in this crate rather than through
//! `temporalio-client` directly. That crate is pre-1.0 and its surface moves
//! between releases, so confining it here keeps a version bump to one crate.

pub mod conn;
pub mod ops;

pub use conn::{Conn, ConnectError, ProfileRef};
pub use ops::{
    OpError,
    codec::Codec,
    history::HistoryPage,
    namespace::NamespaceInfo,
    schedule::SchedulePage,
    workflow::{Continuation, WorkflowPage},
};
