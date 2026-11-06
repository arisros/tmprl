//! Typed operations.

pub mod attributes;
pub mod codec;
pub mod describe;
pub mod history;
pub mod mutate;
pub mod namespace;
pub mod query;
pub mod schedule;
pub mod taskqueue;
pub mod workflow;

use tmprl_core::fault::Code;
pub use tmprl_core::fault::Fault as OpError;

pub(crate) fn rpc(operation: &'static str, status: temporalio_client::tonic::Status) -> OpError {
    OpError::rpc(operation, code(status.code()), status.message())
}

fn code(code: temporalio_client::tonic::Code) -> Code {
    use temporalio_client::tonic::Code as T;
    match code {
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
    fn every_grpc_code_keeps_its_meaning() {
        use temporalio_client::tonic::Code as T;
        let pairs = [
            (T::Cancelled, Code::Cancelled),
            (T::Unknown, Code::Unknown),
            (T::InvalidArgument, Code::InvalidArgument),
            (T::DeadlineExceeded, Code::DeadlineExceeded),
            (T::NotFound, Code::NotFound),
            (T::AlreadyExists, Code::AlreadyExists),
            (T::PermissionDenied, Code::PermissionDenied),
            (T::ResourceExhausted, Code::ResourceExhausted),
            (T::FailedPrecondition, Code::FailedPrecondition),
            (T::Aborted, Code::Aborted),
            (T::OutOfRange, Code::OutOfRange),
            (T::Unimplemented, Code::Unimplemented),
            (T::Internal, Code::Internal),
            (T::Unavailable, Code::Unavailable),
            (T::DataLoss, Code::DataLoss),
            (T::Unauthenticated, Code::Unauthenticated),
        ];
        for (grpc, ours) in pairs {
            assert_eq!(code(grpc), ours);
            assert_eq!(format!("{grpc:?}"), ours.name());
        }
        assert_eq!(code(T::Ok), Code::Unknown);
    }

    #[test]
    fn a_dropped_connection_is_told_apart_from_a_refusal() {
        let fault = rpc("ListNamespaces", Status::unavailable("transport error"));
        assert_eq!(fault.code, Code::Unavailable);
        assert!(!fault.is_refusal());
    }
}
