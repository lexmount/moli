use moli_core::page::{CompletedPageCommand, RendererDomNodeStackTraceResolution};
use serde::Deserialize;
use serde_json::json;

use super::resolve::{
    DomCommandOutput, DomCommandTaskStep, PendingDomCommandDispatch, PendingDomCommandKind,
    PendingDomCommandStartError, PendingDomCommandWork,
};
use super::*;

#[derive(Deserialize)]
struct SetNodeStackTracesEnabledParams {
    enable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetNodeStackTracesParams {
    node_id: u32,
}

pub(super) fn start_set_node_stack_traces_enabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: SetNodeStackTracesEnabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page = super::loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_set_document_node_stack_traces_enabled(renderer_inspector_session_id, params.enable)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::SetNodeStackTracesEnabled,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) fn start_get_node_stack_traces_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: GetNodeStackTracesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page = super::loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_node_stack_trace(renderer_inspector_session_id, params.node_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::GetNodeStackTraces,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) fn complete_set_node_stack_traces_enabled_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return DomCommandTaskStep::Complete;
    };
    if let Err(error) = page.finish_set_document_node_stack_traces_enabled(completion) {
        out.push_error(
            -32000,
            format!("Could not configure DOM node stack traces: {error}"),
        );
        return DomCommandTaskStep::Complete;
    }
    out.push_success();
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_node_stack_traces_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let resolution = {
        let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
            out.push_error(-32000, "NoDocumentLoaded");
            return DomCommandTaskStep::Complete;
        };
        match page.finish_document_node_stack_trace(completion) {
            Ok(resolution) => resolution,
            Err(error) => {
                out.push_error(
                    -32000,
                    format!("Could not get DOM node stack traces: {error}"),
                );
                return DomCommandTaskStep::Complete;
            }
        }
    };
    match resolution {
        RendererDomNodeStackTraceResolution::Found(Some(trace)) => {
            out.push_result(json!({
                "creation": {
                    "callFrames": trace.call_frames.into_iter().map(|frame| json!({
                        "functionName": frame.function_name,
                        "scriptId": frame.script_id,
                        "url": frame.url,
                        "lineNumber": frame.line_number,
                        "columnNumber": frame.column_number,
                    })).collect::<Vec<_>>()
                }
            }));
        }
        RendererDomNodeStackTraceResolution::Found(None) => out.push_success(),
        RendererDomNodeStackTraceResolution::MissingNode => {
            out.push_error(-32000, "Could not find node with given id");
        }
    }
    DomCommandTaskStep::Complete
}
