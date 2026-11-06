//! Namespace listing.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::workflowservice::v1::ListNamespacesRequest;

use super::OpError;
use crate::Conn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceInfo {
    pub name: String,
    pub state: String,
    pub retention_days: i64,
    pub description: String,
}

impl Conn {
    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceInfo>, OpError> {
        let mut wf = self.wf();
        let mut out = Vec::new();
        let mut page_token = Vec::new();

        loop {
            let resp = wf
                .list_namespaces(Request::new(ListNamespacesRequest {
                    page_size: 100,
                    next_page_token: page_token,
                    ..Default::default()
                }))
                .await
                .map_err(|s| super::rpc("ListNamespaces", s))?
                .into_inner();

            for ns in resp.namespaces {
                let Some(info) = ns.namespace_info else {
                    continue;
                };
                let state = format!("{:?}", info.state());
                out.push(NamespaceInfo {
                    name: info.name,
                    state,
                    retention_days: ns
                        .config
                        .and_then(|c| c.workflow_execution_retention_ttl)
                        .map(|d| d.seconds / 86_400)
                        .unwrap_or(0),
                    description: info.description,
                });
            }

            if resp.next_page_token.is_empty() {
                break;
            }
            page_token = resp.next_page_token;
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}
