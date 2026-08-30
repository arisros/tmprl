//! Typed operations.
//!
//! Everything above this crate talks in these types, never in protobuf types. That is what
//! keeps a `temporalio-client` bump contained: the generated types stop here.

pub mod namespace;

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("{operation} failed: {message}")]
    Rpc {
        operation: &'static str,
        code: String,
        message: String,
    },
}

impl OpError {
    pub(crate) fn rpc(operation: &'static str, status: temporalio_client::tonic::Status) -> Self {
        Self::Rpc {
            operation,
            code: format!("{:?}", status.code()),
            message: status.message().to_string(),
        }
    }
}
