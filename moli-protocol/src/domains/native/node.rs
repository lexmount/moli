use moli_core::page::RendererDomFrontendNodeBindingResolution;
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeOperationStep as Step,
    RendererNativeProtocolResponse as Response, RendererPageCommand as Command,
    RendererPageReply as Reply,
};
use serde::Deserialize;

use crate::automation::DevToolsDomNodeReference;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NodeReferenceParams {
    #[serde(default)]
    pub(crate) node_id: Option<u32>,
    #[serde(default)]
    pub(crate) backend_node_id: Option<u32>,
    #[serde(default)]
    pub(crate) object_id: Option<String>,
}

/// Remaining lookup continuations may enter user code and require their owner turn.
pub(crate) fn with_backend_node(
    session: Option<String>,
    reference: DevToolsDomNodeReference,
    next: impl FnOnce(u32) -> Operation + Send + 'static,
) -> Operation {
    match reference {
        DevToolsDomNodeReference::BackendNodeId(id) => next(id).require_owner_turn(),
        DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) => {
            let command = Command::DocumentFrontendNodeBinding {
                reference: moli_core::page::RendererDomNodeReference::FrontendNodeId {
                    inspector_session_id: session,
                    frontend_node_id,
                },
            };
            let continue_binding = move |reply| match reply {
                Ok(Reply::DocumentFrontendNodeBinding(
                    RendererDomFrontendNodeBindingResolution::BackendNodeId(id),
                )) => Step::Continue(next(id)),
                Ok(Reply::DocumentFrontendNodeBinding(
                    RendererDomFrontendNodeBindingResolution::NotFound,
                )) => Step::Complete(Response::error(-32000, "Could not find node with given id")),
                Err(error) => Step::Complete(Response::error(
                    -32000,
                    format!("Could not resolve frontend node binding: {error}"),
                )),
                _ => unreachable!("native frontend-node lookup reply"),
            };
            Operation::then(command, continue_binding)
        }
    }
}
