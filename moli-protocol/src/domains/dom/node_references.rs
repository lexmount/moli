use crate::automation::DevToolsDomNodeReference;
pub(super) use crate::domains::native::NodeReferenceParams;

pub(super) fn devtools_node_reference_from_ids(
    node_id: Option<u32>,
    backend_node_id: Option<u32>,
) -> Option<DevToolsDomNodeReference> {
    node_id
        .map(DevToolsDomNodeReference::FrontendNodeId)
        .or_else(|| backend_node_id.map(DevToolsDomNodeReference::BackendNodeId))
}
