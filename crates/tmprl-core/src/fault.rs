//! A failed request, kept in enough detail to say what went wrong and what to try.
//!
//! The client produces these and the interface renders them, so the type lives here, below
//! both. What makes it worth a type rather than a sentence is the [`Code`]: "the server did
//! not answer" and "the server said no" call for different things from the reader, and a
//! flattened message cannot tell them apart without guessing at its wording.

use std::fmt;

/// Why a request failed. The gRPC status codes, plus the two failures that never reach a
/// Temporal server.
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
    /// An update the server accepted and the workflow then refused. Not a gRPC status: the
    /// call itself succeeded.
    Rejected,
    /// The codec server, which is HTTP and has no status code of this kind.
    Codec,
    /// Raised on this side of the wire, with nothing but a message to go on.
    Local,
}

impl Code {
    /// The name as gRPC spells it, which is what a server log or a bug report will use.
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

    /// The same thing in words, for a line a person reads.
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

    /// What to try, where there is something to try.
    ///
    /// Only codes with an action the reader can take get one. A hint that restates the
    /// failure is a second line saying the same thing as the first.
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
    /// The RPC that failed, as Temporal names it. Empty when no RPC was involved.
    pub operation: &'static str,
    pub code: Code,
    /// What the other side said, verbatim.
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

    /// Whether the server refused the operation rather than the call going wrong.
    ///
    /// The code is the answer where there is one. The two sentences are Temporal Cloud's,
    /// kept because which status it sends them under has not been confirmed, and missing
    /// the refusal strands a namespace-scoped key on the opening screen.
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
        // A codec refusal already says what it is; prefixing an empty operation would
        // print " failed (codec): ...".
        assert_eq!(Fault::codec("returned 502").to_string(), "returned 502");
        assert_eq!(Fault::from("boom").to_string(), "boom");
        assert_eq!(Fault::from("boom").code, Code::Local);
    }

    #[test]
    fn a_refusal_is_recognised_by_code_before_wording() {
        assert!(Fault::rpc("ListNamespaces", Code::PermissionDenied, "no").is_refusal());
        // Temporal Cloud's sentence, under whatever status it arrives with.
        assert!(Fault::rpc("ListNamespaces", Code::Unknown, "Request unauthorized.").is_refusal());
        assert!(!Fault::rpc("ListNamespaces", Code::Unavailable, "transport error").is_refusal());
    }

    #[test]
    fn a_bad_credential_is_not_dressed_up_as_a_scoped_key() {
        // Unauthenticated means the key itself was refused. Treating it as a refusal would
        // show one namespace from the profile and then fail on everything inside it.
        assert!(!Fault::rpc("ListNamespaces", Code::Unauthenticated, "bad key").is_refusal());
    }

    #[test]
    fn every_code_has_a_distinct_name() {
        let all = [
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
        let names: std::collections::HashSet<_> = all.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), all.len());
    }
}
