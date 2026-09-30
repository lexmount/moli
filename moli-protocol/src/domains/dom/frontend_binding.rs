use crate::automation::DevToolsDomNodeReference;
use moli_core::page::{CompletedPageCommand, Page, RendererDomFrontendNodeBindingResolution};

pub(super) fn finish_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsDomNodeReference, String> {
    page.finish_document_frontend_node_binding(completion)
        .map_err(|error| format!("Could not resolve frontend node binding: {error}"))
        .map(reference_from_resolution)
        .and_then(|reference| {
            reference.ok_or_else(|| "Could not find node with given id".to_owned())
        })
}

fn reference_from_resolution(
    resolution: RendererDomFrontendNodeBindingResolution,
) -> Option<DevToolsDomNodeReference> {
    match resolution {
        RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id) => {
            Some(DevToolsDomNodeReference::BackendNodeId(backend_node_id))
        }
        RendererDomFrontendNodeBindingResolution::NotFound => None,
    }
}
