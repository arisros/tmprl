//! A failed request, kept in enough detail to say what went wrong and what to try.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    Cancelled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
    Unauthenticated,
    Rejected,
    Codec,
    Local,
}

impl Code {
    pub fn name(self) -> &'static str {
        match self {
            Code::Cancelled => "Cancelled",
            Code::Unknown => "Unknown",
            Code::InvalidArgument => "InvalidArgument",
            Code::DeadlineExceeded => "DeadlineExceeded",
            Code::NotFound => "NotFound",
            Code::AlreadyExists => "AlreadyExists",
            Code::PermissionDenied => "PermissionDenied",
            Code::ResourceExhausted => "ResourceExhausted",
            Code::FailedPrecondition => "FailedPrecondition",
            Code::Aborted => "Aborted",
            Code::OutOfRange => "OutOfRange",
            Code::Unimplemented => "Unimplemented",
            Code::Internal => "Internal",
            Code::Unavailable => "Unavailable",
            Code::DataLoss => "DataLoss",
            Code::Unauthenticated => "Unauthenticated",
            Code::Rejected => "Rejected",
            Code::Codec => "Codec",
            Code::Local => "Local",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Code::Cancelled => "cancelled",
            Code::Unknown => "unknown error",
            Code::InvalidArgument => "invalid argument",
            Code::DeadlineExceeded => "timed out",
            Code::NotFound => "not found",
            Code::AlreadyExists => "already exists",
            Code::PermissionDenied => "permission denied",
            Code::ResourceExhausted => "rate limited",
            Code::FailedPrecondition => "failed precondition",
            Code::Aborted => "aborted",
            Code::OutOfRange => "out of range",
            Code::Unimplemented => "not supported",
            Code::Internal => "server error",
            Code::Unavailable => "unavailable",
            Code::DataLoss => "data loss",
            Code::Unauthenticated => "not authenticated",
            Code::Rejected => "rejected by the workflow",
            Code::Codec => "codec",
            Code::Local => "error",
        }
    }

    pub fn hint(self) -> Option<&'static str> {
        match self {
            Code::Unavailable => Some("the server did not answer; check the connection, R retries"),
            Code::DeadlineExceeded => Some("the server took too long; R retries"),
            Code::PermissionDenied => Some("this profile is not allowed to do that here"),
            Code::Unauthenticated => {
                Some("the credentials were refused; check the profile's api_key or certificate")
            }
            Code::NotFound => Some("it is gone, or it is in another namespace"),
            Code::InvalidArgument => Some("the server refused the request as written"),
            Code::ResourceExhausted => Some("the server is rate limiting; wait, then R"),
            Code::Unimplemented => Some("this server does not support that call"),
            Code::Cancelled
            | Code::Unknown
            | Code::AlreadyExists
            | Code::FailedPrecondition
            | Code::Aborted
            | Code::OutOfRange
            | Code::Internal
            | Code::DataLoss
            | Code::Rejected
            | Code::Codec
            | Code::Local => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub operation: &'static str,
    pub code: Code,
    pub message: String,
}

impl Fault {
    pub fn rpc(operation: &'static str, code: Code, message: impl Into<String>) -> Self {
        Self {
            operation,
            code,
            message: message.into(),
        }
    }

    pub fn codec(message: impl Into<String>) -> Self {
        Self {
            operation: "",
            code: Code::Codec,
            message: message.into(),
        }
    }

    pub fn hint(&self) -> Option<&'static str> {
        self.code.hint()
    }

    // Temporal Cloud's refusal wording, since its status code is not confirmed.
    pub fn is_transient(&self) -> bool {
        matches!(
            self.code,
            Code::Unavailable
                | Code::DeadlineExceeded
                | Code::Unknown
                | Code::Aborted
                | Code::Internal
                | Code::ResourceExhausted
                | Code::Cancelled
        )
    }

    pub fn is_refusal(&self) -> bool {
        if self.code == Code::PermissionDenied {
            return true;
        }
        let m = self.message.to_ascii_lowercase();
        m.contains("permission denied")
            || m.contains("does not have permission")
            || m.contains("request unauthorized")
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.operation.is_empty() {
            return f.write_str(&self.message);
        }
        write!(
            f,
            "{} failed ({}): {}",
            self.operation,
            self.code.label(),
            self.message
        )
    }
}

impl std::error::Error for Fault {}

impl From<String> for Fault {
    fn from(message: String) -> Self {
        Self {
            operation: "",
            code: Code::Local,
            message,
        }
    }
}

impl From<&str> for Fault {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

pub fn retry_after(attempt: u32) -> std::time::Duration {
    let secs = 1u64 << attempt.saturating_sub(1).min(5);
    std::time::Duration::from_secs(secs.min(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rpc_failure_names_the_call_the_code_and_the_reason() {
        let f = Fault::rpc("ListNamespaces", Code::Unavailable, "transport error");
        assert_eq!(
            f.to_string(),
            "ListNamespaces failed (unavailable): transport error"
        );
        assert!(f.hint().unwrap().contains("R retries"));
    }

    #[test]
    fn a_failure_with_no_rpc_behind_it_is_only_its_message() {
        assert_eq!(Fault::codec("returned 502").to_string(), "returned 502");
        assert_eq!(Fault::from("boom").to_string(), "boom");
        assert_eq!(Fault::from("boom").code, Code::Local);
    }

    #[test]
    fn a_refusal_is_recognised_by_code_before_wording() {
        assert!(Fault::rpc("ListNamespaces", Code::PermissionDenied, "no").is_refusal());
        assert!(Fault::rpc("ListNamespaces", Code::Unknown, "Request unauthorized.").is_refusal());
        assert!(!Fault::rpc("ListNamespaces", Code::Unavailable, "transport error").is_refusal());
    }

    #[test]
    fn a_bad_credential_is_not_dressed_up_as_a_scoped_key() {
        assert!(!Fault::rpc("ListNamespaces", Code::Unauthenticated, "bad key").is_refusal());
    }

    const ALL: [Code; 19] = [
        Code::Cancelled,
        Code::Unknown,
        Code::InvalidArgument,
        Code::DeadlineExceeded,
        Code::NotFound,
        Code::AlreadyExists,
        Code::PermissionDenied,
        Code::ResourceExhausted,
        Code::FailedPrecondition,
        Code::Aborted,
        Code::OutOfRange,
        Code::Unimplemented,
        Code::Internal,
        Code::Unavailable,
        Code::DataLoss,
        Code::Unauthenticated,
        Code::Rejected,
        Code::Codec,
        Code::Local,
    ];

    #[test]
    fn every_code_has_a_distinct_name() {
        let names: std::collections::HashSet<_> = ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), ALL.len());
    }

    #[test]
    fn every_code_reads_as_words_on_the_note_line() {
        for code in ALL {
            let label = code.label();
            assert!(!label.is_empty(), "{code:?}");
            assert_eq!(label, label.to_lowercase(), "{code:?}");
        }
        let labels: std::collections::HashSet<_> = ALL.iter().map(|c| c.label()).collect();
        assert_eq!(labels.len(), ALL.len(), "two codes must not read the same");
    }

    #[test]
    fn a_hint_is_only_given_where_there_is_something_to_try() {
        let hinted: Vec<Code> = ALL.into_iter().filter(|c| c.hint().is_some()).collect();
        assert_eq!(
            hinted,
            [
                Code::InvalidArgument,
                Code::DeadlineExceeded,
                Code::NotFound,
                Code::PermissionDenied,
                Code::ResourceExhausted,
                Code::Unimplemented,
                Code::Unavailable,
                Code::Unauthenticated,
            ]
        );
        assert!(Code::Internal.hint().is_none());
        assert!(Code::Rejected.hint().is_none());
    }

    #[test]
    fn a_dropped_connection_is_worth_retrying_and_a_wrong_request_is_not() {
        assert!(Fault::rpc("GetWorkflowExecutionHistory", Code::Unavailable, "").is_transient());
        assert!(
            Fault::rpc("GetWorkflowExecutionHistory", Code::DeadlineExceeded, "").is_transient()
        );
        assert!(!Fault::rpc("GetWorkflowExecutionHistory", Code::NotFound, "").is_transient());
        assert!(
            !Fault::rpc("GetWorkflowExecutionHistory", Code::PermissionDenied, "").is_transient()
        );
        assert!(!Fault::codec("bad key").is_transient());
    }

    #[test]
    fn retries_back_off_to_thirty_seconds() {
        let secs: Vec<u64> = (1..=8).map(|n| retry_after(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
    }
}
