use super::{
    InlineStyleQueryKind, PendingCssCommandDispatch, PendingCssCommandKind,
    PendingCssCommandStartError, loaded_page_mut_for_session,
};
use crate::conn::{CdpConnection, Cmd};
use moli_core::page::RendererDomFrontendNodeBindingResolution;

pub(super) fn start_frontend_node_binding_for_computed_style(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frontend_node_id: u32,
) -> Result<PendingCssCommandDispatch, PendingCssCommandStartError> {
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let pending = page
        .start_document_frontend_node_binding(renderer_inspector_session_id, frontend_node_id)
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
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let pending = page
        .start_document_frontend_node_binding(renderer_inspector_session_id, frontend_node_id)
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
