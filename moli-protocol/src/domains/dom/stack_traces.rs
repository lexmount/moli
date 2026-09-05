use moli_core::page::PendingPageCommand;
use moli_core::page::{CompletedPageCommand, RendererDomNodeStackTraceResolution};
use serde::Deserialize;
use serde_json::json;

use super::resolve::{
    DomCommandOutput, DomCommandTaskStep, PendingDomCommandDispatch, PendingDomCommandKind,
    PendingDomCommandStartError,
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
    let inspection = super::dom_inspection_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_set_document_node_stack_traces_enabled(params.enable)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::SetNodeStackTracesEnabled,
        pending,
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
    let inspection = super::dom_inspection_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = inspection
        .start_document_node_stack_trace(params.node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::GetNodeStackTraces,
        pending,
    }))
}

pub(super) fn complete_set_node_stack_traces_enabled_command(
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    if let Err(error) = completion.finish_set_document_node_stack_traces_enabled() {
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
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let resolution = {
        match completion.finish_document_node_stack_trace() {
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
