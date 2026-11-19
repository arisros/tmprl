//! All network IO for tmprl. Knows nothing about the UI.

pub mod conn;
pub mod ops;

pub use conn::{
    Conn, ConnectError, ProfileRef, ProfileSummary, config_file_in_use, list_profiles,
    resolve_api_key, xdg_config_file,
};
pub use ops::{
    OpError,
    codec::Codec,
    describe::Described,
    history::HistoryPage,
    namespace::NamespaceInfo,
    schedule::SchedulePage,
    workflow::{Continuation, WorkflowPage},
};
