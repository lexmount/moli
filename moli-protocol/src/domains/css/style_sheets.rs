use crate::conn::{BackgroundProtocolEvent, CdpConnection, Cmd, build_event};
use crate::domains::command_output::CommandOutputPlan;
use moli_core::page::{
    RendererStyleSheetHeader, RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};
use serde_json::{Value, json};

use super::{
    PendingCssCommandDispatch, PendingCssCommandKind, PendingCssCommandStartError,
    loaded_page_mut_for_session, top_frame_id_for_session,
};

pub(super) fn start_pending_enable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingCssCommandDispatch>, PendingCssCommandStartError> {
    set_css_enabled(conn, cmd, true);
    if conn
        .ensure_document_accessible_for_session_owner(cmd.session_id)
        .is_err()
    {
        return Ok(None);
    }
    let frame_id = top_frame_id_for_session(conn, cmd.session_id).unwrap_or_default();
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Ok(None);
    };
    let pending = page
        .start_style_sheet_inventory_for_document_and_inspector_session(
            renderer_inspector_session_id,
        )
        .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(Some(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::Enable { frame_id },
        pending,
    )))
}

pub(super) fn start_pending_disable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingCssCommandDispatch>, PendingCssCommandStartError> {
    set_css_enabled(conn, cmd, false);
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Ok(None);
    };
    let pending = page
        .start_reset_css_agent_session(renderer_inspector_session_id)
        .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(Some(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::Disable,
        pending,
    )))
}

pub(super) fn complete_enable_command_output_plan(
    frame_id: &str,
    session_id: Option<&str>,
    update: RendererStyleSheetInventoryUpdate,
) -> CommandOutputPlan {
    let mut plan = CommandOutputPlan::success();
    for style_sheet_id in update.removed {
        plan.push_background_event(BackgroundProtocolEvent::immediate(build_event(
            "CSS.styleSheetRemoved",
            json!({
                "styleSheetId": style_sheet_id,
            }),
            session_id,
        )));
    }
    for header in update.added {
        plan.push_background_event(BackgroundProtocolEvent::immediate(build_event(
            "CSS.styleSheetAdded",
            json!({
                "header": style_sheet_header_value(frame_id, &header),
            }),
            session_id,
        )));
    }
    plan
}

pub(super) fn get_style_sheet_command_output_plan(
    style_sheet_id: &str,
    frame_id: &str,
    payload: RendererStyleSheetPayload,
) -> CommandOutputPlan {
    let RendererStyleSheetPayload {
        text,
        title,
        disabled,
        source_url,
        is_inline,
    } = payload;
    let (end_line, end_column) = style_sheet_text_end_position(&text);

    CommandOutputPlan::result(json!({
        "styleSheet": {
            "styleSheetId": style_sheet_id,
            "frameId": frame_id,
            "sourceURL": source_url,
            "origin": "regular",
            "title": title,
            "disabled": disabled,
            "isInline": is_inline,
            "startLine": 0,
            "startColumn": 0,
            "endLine": end_line,
            "endColumn": end_column,
            "length": text.len(),
            "text": text,
        }
    }))
}

fn set_css_enabled(conn: &mut CdpConnection, cmd: &Cmd<'_>, enabled: bool) {
    conn.mutate_target_page_state_for_session(cmd.session_id, |state| {
        state.css_enabled = enabled;
    });
}

fn style_sheet_header_value(frame_id: &str, header: &RendererStyleSheetHeader) -> Value {
    json!({
        "styleSheetId": header.style_sheet_id,
        "frameId": frame_id,
        "sourceURL": header.source_url,
        "origin": "regular",
        "title": header.title,
        "disabled": header.disabled,
        "isInline": header.is_inline,
        "startLine": 0,
        "startColumn": 0,
        "endLine": header.end_line,
        "endColumn": header.end_column,
        "length": header.length,
    })
}

fn style_sheet_text_end_position(text: &str) -> (u32, u32) {
    let end_line = text.lines().count().saturating_sub(1) as u32;
    let end_column = text
        .lines()
        .last()
        .map(|line| line.chars().count())
        .unwrap_or(0) as u32;
    (end_line, end_column)
}
