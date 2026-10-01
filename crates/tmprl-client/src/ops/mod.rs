//! Typed operations.
//!
//! Everything above this crate talks in these types, never in protobuf types. That is what
//! keeps a `temporalio-client` bump contained: the generated types stop here.

pub mod attributes;
pub mod codec;
pub mod describe;
pub mod history;
pub mod mutate;
pub mod namespace;
pub mod schedule;
pub mod workflow;

use tmprl_core::fault::Code;
/// What every operation fails with. The type is `tmprl-core`'s, so the interface can read
/// the code without depending on tonic; the name is kept because it is what this crate's
/// signatures have always said.
pub use tmprl_core::fault::Fault as OpError;

pub(crate) fn rpc(operation: &'static str, status: temporalio_client::tonic::Status) -> OpError {
    OpError::rpc(operation, code(status.code()), status.message())
}

fn code(code: temporalio_client::tonic::Code) -> Code {
    use temporalio_client::tonic::Code as T;
    match code {
        // A call that failed cannot carry Ok; if one ever does, it is not a known failure.
        T::Ok | T::Unknown => Code::Unknown,
        T::Cancelled => Code::Cancelled,
        T::InvalidArgument => Code::InvalidArgument,
        T::DeadlineExceeded => Code::DeadlineExceeded,
        T::NotFound => Code::NotFound,
        T::AlreadyExists => Code::AlreadyExists,
        T::PermissionDenied => Code::PermissionDenied,
        T::ResourceExhausted => Code::ResourceExhausted,
        T::FailedPrecondition => Code::FailedPrecondition,
        T::Aborted => Code::Aborted,
        T::OutOfRange => Code::OutOfRange,
        T::Unimplemented => Code::Unimplemented,
        T::Internal => Code::Internal,
        T::Unavailable => Code::Unavailable,
        T::DataLoss => Code::DataLoss,
        T::Unauthenticated => Code::Unauthenticated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_client::tonic::Status;

    #[test]
    fn a_status_keeps_its_code_and_its_message() {
        let fault = rpc("ListNamespaces", Status::permission_denied("not for you"));
        assert_eq!(fault.code, Code::PermissionDenied);
        assert_eq!(fault.operation, "ListNamespaces");
        assert_eq!(fault.message, "not for you");
    }

    #[test]
    fn a_dropped_connection_is_told_apart_from_a_refusal() {
        // The two call for different things from the reader, which is the reason the code
        // is carried at all.
        let fault = rpc("ListNamespaces", Status::unavailable("transport error"));
        assert_eq!(fault.code, Code::Unavailable);
        assert!(!fault.is_refusal());
    }
}
