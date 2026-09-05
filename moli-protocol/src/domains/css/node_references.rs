use super::{
    InlineStyleQueryKind, PendingCssCommandDispatch, PendingCssCommandKind,
    PendingCssCommandStartError,
};
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use moli_core::page::{PendingPageCommand, RendererDomFrontendNodeBindingResolution};

pub(super) fn start_frontend_node_binding_for_computed_style(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frontend_node_id: u32,
) -> Result<PendingCssCommandDispatch, PendingCssCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(inspection) = crate::domains::dom::dom_inspection_for_owner(conn, &owner) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let pending = inspection
        .start_document_frontend_node_binding(frontend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::ResolveFrontendNodeForComputedStyle,
        pending,
    ))
}

pub(super) fn start_frontend_node_binding_for_inline_style(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frontend_node_id: u32,
    kind: InlineStyleQueryKind,
) -> Result<Option<PendingCssCommandDispatch>, PendingCssCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(inspection) = crate::domains::dom::dom_inspection_for_owner(conn, &owner) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let pending = inspection
        .start_document_frontend_node_binding(frontend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(Some(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::ResolveFrontendNodeForInlineStyle { kind },
        pending,
    )))
}

pub(super) fn backend_node_id_from_frontend_resolution(
    resolution: RendererDomFrontendNodeBindingResolution,
) -> Option<u32> {
    match resolution {
        RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id) => {
            Some(backend_node_id)
        }
        RendererDomFrontendNodeBindingResolution::NotFound => None,
    }
}
