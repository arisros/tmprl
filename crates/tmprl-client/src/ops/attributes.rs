//! The search attributes a cluster has registered.
//!
//! What can be filtered on is a property of the cluster, not of this build: whoever runs it
//! registers the attributes their workflows carry. Asking the server is the only way to
//! offer `CustomerId` in a filter without having been told about it.

use temporalio_client::tonic::Request;
use temporalio_common::protos::temporal::api::enums::v1::IndexedValueType;
use temporalio_common::protos::temporal::api::operatorservice::v1::ListSearchAttributesRequest;
use tmprl_core::filter::{AttributeType, SearchAttribute};

use super::OpError;
use crate::Conn;

impl Conn {
    /// Every search attribute registered for `namespace`, custom ones first.
    ///
    /// On the operator service rather than the workflow service, which is why this is the
    /// only caller of [`Conn::operator`].
    pub async fn list_search_attributes(
        &self,
        namespace: &str,
    ) -> Result<Vec<SearchAttribute>, OpError> {
        let resp = self
            .operator()
            .list_search_attributes(Request::new(ListSearchAttributesRequest {
                namespace: namespace.to_string(),
            }))
            .await
            .map_err(|s| OpError::rpc("ListSearchAttributes", s))?
            .into_inner();

        let mut out: Vec<SearchAttribute> = resp
            .custom_attributes
            .into_iter()
            .map(|(name, kind)| SearchAttribute {
                name,
                kind: attribute_type(kind),
                system: false,
            })
            .chain(
                resp.system_attributes
                    .into_iter()
                    .map(|(name, kind)| SearchAttribute {
                        name,
                        kind: attribute_type(kind),
                        system: true,
                    }),
            )
            .collect();

        // The wire form is a map, so the order it arrives in is not stable between calls.
        // Sorting here keeps the filter list from reshuffling itself on a refresh.
        out.sort_by(|a, b| a.system.cmp(&b.system).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }
}

/// The protobuf enum, as the thing the catalogue reasons about.
///
/// An unrecognised value maps to `Unspecified` rather than being dropped: a cluster running
/// a newer server than this build still has attributes worth filtering on, and a name you
/// can edit beats no offer at all.
fn attribute_type(raw: i32) -> AttributeType {
    match IndexedValueType::try_from(raw) {
        Ok(IndexedValueType::Text) => AttributeType::Text,
        Ok(IndexedValueType::Keyword) => AttributeType::Keyword,
        Ok(IndexedValueType::Int) => AttributeType::Int,
        Ok(IndexedValueType::Double) => AttributeType::Double,
        Ok(IndexedValueType::Bool) => AttributeType::Bool,
        Ok(IndexedValueType::Datetime) => AttributeType::Datetime,
        Ok(IndexedValueType::KeywordList) => AttributeType::KeywordList,
        Ok(IndexedValueType::Unspecified) | Err(_) => AttributeType::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_indexed_type_maps_to_something_the_catalogue_can_shape() {
        assert_eq!(
            attribute_type(IndexedValueType::Keyword as i32),
            AttributeType::Keyword
        );
        assert_eq!(
            attribute_type(IndexedValueType::Datetime as i32),
            AttributeType::Datetime
        );
        assert_eq!(
            attribute_type(IndexedValueType::KeywordList as i32),
            AttributeType::KeywordList
        );
    }

    #[test]
    fn a_type_this_build_does_not_know_is_offered_rather_than_dropped() {
        assert_eq!(attribute_type(9_999), AttributeType::Unspecified);
        assert_eq!(attribute_type(0), AttributeType::Unspecified);
    }
}
