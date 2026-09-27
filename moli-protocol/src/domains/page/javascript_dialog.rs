use super::*;

pub(super) fn start_set_javascript_dialog_handler_enabled(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    enabled: bool,
) -> Result<(), String> {
    if let Ok(slot) = conn.runtime_session_owner_slot_mut(session_id)
        && let Some(page) = slot.loaded_page_mut()
    {
        return page
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    if let Some(page) = conn.browser_context.as_mut().and_then(|browser_context| {
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .loaded_page_mut()
    }) {
        return page
            .start_set_javascript_dialog_handler_enabled(enabled)
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    Ok(())
}

pub(super) fn handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let command = match build_cdp_handle_javascript_dialog_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return plan,
    };
    match start_devtools_page_command(
        conn,
        cmd.id,
        DevToolsCommand::HandleJavaScriptDialog(command),
    ) {
        PageCommandTaskStep::Complete(plan) => plan,
        PageCommandTaskStep::Pending(_) => {
            CommandOutputPlan::error(-32000, "Unexpected pending handleJavaScriptDialog command")
        }
    }
}

pub(super) fn build_cdp_handle_javascript_dialog_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsHandleJavaScriptDialogCommand, CommandOutputPlan> {
    let params: HandleJavaScriptDialogParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsHandleJavaScriptDialogCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        accept: params.accept,
        prompt_text: params.prompt_text.unwrap_or_default(),
    })
}

pub(super) fn complete_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
) -> CommandOutputPlan {
    let owner = match page_command_owner(conn, &command.context) {
        Ok(owner) => owner,
        Err(error) => return CommandOutputPlan::from_devtools_error(error),
    };
    match finish_devtools_handle_javascript_dialog_command(conn, command, &owner) {
        Ok(closed_event) => {
            let mut plan = CommandOutputPlan::default();
            plan.push_background_event(closed_event);
            plan.push_success();
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) fn finish_devtools_get_javascript_dialog_command(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<DevToolsJavaScriptDialogResult, DevToolsError> {
    let current_page_owner = conn.target_page_residence_identity_for_owner(owner);
    let Some(dialog) = conn
        .target_page_session_state_for_owner(owner)
        .and_then(|page_state| page_state.javascript_dialog_state.peek_next())
        .filter(|dialog| current_page_owner.as_ref() == Some(dialog.page_owner()))
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchAlert,
            "No dialog is showing",
        ));
    };
    Ok(DevToolsJavaScriptDialogResult {
        dialog_type: dialog.dialog_type().to_owned(),
        message: dialog.message().to_owned(),
        default_prompt: dialog.default_prompt().to_owned(),
    })
}

pub(super) fn finish_devtools_set_javascript_dialog_prompt_text_command(
    conn: &mut CdpConnection,
    command: DevToolsSetJavaScriptDialogPromptTextCommand,
    owner: &CommandOwnerScope,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let current_page_owner = conn.target_page_residence_identity_for_owner(owner);
    let Some(result) = conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
        let dialog_state = &mut state.page_session_state.javascript_dialog_state;
        let Some(dialog) = dialog_state
            .peek_next()
            .filter(|dialog| current_page_owner.as_ref() == Some(dialog.page_owner()))
        else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchAlert,
                "No dialog is showing",
            ));
        };
        if dialog.dialog_type() != "prompt" {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "Dialog is not a prompt",
            ));
        }
        if !dialog_state.set_next_prompt_text(command.prompt_text) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchAlert,
                "No dialog is showing",
            ));
        }
        Ok(DevToolsCommandResult::Empty)
    }) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchAlert,
            "No dialog is showing",
        ));
    };
    result
}

pub(super) fn finish_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
    owner: &CommandOwnerScope,
) -> Result<BackgroundProtocolEvent, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let command_prompt_text = command.prompt_text;
    let current_page_owner = conn.target_page_residence_identity_for_owner(owner);
    let Some(dialog) = conn
        .with_target_devtools_session_state_for_owner_mut(owner, |state| {
            let dialog_state = &mut state.page_session_state.javascript_dialog_state;
            if dialog_state
                .peek_next()
                .is_some_and(|dialog| current_page_owner.as_ref() != Some(dialog.page_owner()))
            {
                dialog_state.clear();
                return None;
            }
            dialog_state.pop_next_with_prompt_text()
        })
        .flatten()
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchAlert,
            "No dialog is showing",
        ));
    };
    let (dialog, stored_prompt_text) = dialog;
    let user_input = if command_prompt_text.is_empty() {
        stored_prompt_text.unwrap_or_default()
    } else {
        command_prompt_text
    };
    let _ = dialog.finish(command.accept, user_input.clone());
    let closed_event = UserPromptClosedEvent {
        target_id: command.context.target_id,
        frame_id: dialog.source_frame_id().into(),
        prompt_type: dialog.dialog_type().to_owned(),
        accepted: command.accept,
        user_text: user_input,
    };
    Ok(BackgroundProtocolEvent::page_javascript_dialog_closed(
        session_id,
        closed_event,
    ))
}

pub(in crate::domains) async fn emit_javascript_dialog_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(dialogs) = prepared_outputs
        .and_then(ProtocolOutputPayloads::page_mut)
        .and_then(PagePreparedOutputSlot::take_javascript_dialogs)
    {
        javascript_dialog::emit_prepared(conn, out, dialogs);
    }
}

pub(super) fn execute_devtools_get_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsGetJavaScriptDialogCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let owner = page_command_owner(conn, &command.context)?;
    let result = finish_devtools_get_javascript_dialog_command(conn, &owner);
    result.map(DevToolsCommandResult::JavaScriptDialog)
}

pub(super) fn execute_devtools_set_javascript_dialog_prompt_text_command(
    conn: &mut CdpConnection,
    command: DevToolsSetJavaScriptDialogPromptTextCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let owner = page_command_owner(conn, &command.context)?;
    finish_devtools_set_javascript_dialog_prompt_text_command(conn, command, &owner)
}

pub(super) fn execute_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<BackgroundProtocolEvent>,
) {
    let owner = match page_command_owner(conn, &command.context) {
        Ok(owner) => owner,
        Err(error) => return (Err(error), Vec::new()),
    };
    let result = finish_devtools_handle_javascript_dialog_command(conn, command, &owner);
    match result {
        Ok(event) => (Ok(DevToolsCommandResult::Empty), vec![event]),
        Err(error) => (Err(error), Vec::new()),
    }
}

#[cfg(test)]
use moli_core::page::RendererPendingJavaScriptDialog;

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, TargetPageProtocolAttachmentIdentity,
    TargetPreparedJavaScriptDialog, TargetPreparedJavaScriptDialogRoute,
};
#[cfg(test)]
use crate::conn::{TargetJavaScriptDialogScopeObserver, TargetPageResidenceIdentity};
use crate::devtools_runtime::PageJavaScriptDialogOpeningEvent;

pub(super) type PreparedJavaScriptDialog = TargetPreparedJavaScriptDialog;

pub(super) fn emit_prepared(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    dialogs: Vec<PreparedJavaScriptDialog>,
) {
    for dialog in dialogs {
        emit_one(conn, out, dialog);
    }
}

fn emit_one(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    dialog: PreparedJavaScriptDialog,
) {
    if !source_is_current(conn, &dialog) {
        trace_stale_source(&dialog);
        dialog.dismiss();
        return;
    }

    match dialog.route().clone() {
        TargetPreparedJavaScriptDialogRoute::AttachedPage { source_frame_id } => {
            let destination = dialog.source_attachment().clone();
            emit_to_attachment(conn, out, destination, source_frame_id, dialog);
        }
        TargetPreparedJavaScriptDialogRoute::LightweightPopup { popup_id, .. } => {
            let browser_context_id = dialog
                .source_attachment()
                .page_owner()
                .browser_context_id()
                .to_owned();
            let target_id = conn
                .browser_context_by_id(&browser_context_id)
                .and_then(|context| context.target_id_for_popup_id(popup_id))
                .map(str::to_owned);
            if let Some(target_id) = target_id {
                emit_popup_dialogs_for_target(
                    conn,
                    out,
                    &browser_context_id,
                    &target_id,
                    vec![dialog],
                );
                return;
            }
            let Some(browser_context) = conn.browser_context_by_id_mut(&browser_context_id) else {
                dialog.dismiss();
                return;
            };
            browser_context.park_pending_popup_javascript_dialog(dialog);
        }
    }
}

fn source_is_current(conn: &CdpConnection, dialog: &PreparedJavaScriptDialog) -> bool {
    conn.target_page_protocol_attachment_identity_is_current(dialog.source_attachment())
        && conn
            .runtime_session_owner_slot(dialog.source_attachment().session_id())
            .is_ok_and(|slot| slot.observes_javascript_dialog_scope(dialog.source_dialog_scope()))
}

fn trace_stale_source(dialog: &PreparedJavaScriptDialog) {
    let page_owner = dialog.source_attachment().page_owner();
    tracing::debug!(
        session_id = dialog.source_attachment().session_id(),
        dialog_id = dialog.id().sequence(),
        source_document = ?dialog.source_document(),
        browser_context_id = page_owner.browser_context_id(),
        target_id = page_owner.target_id(),
        page_attachment_id = page_owner.page_attachment_id().get(),
        route = ?dialog.route(),
        "dismissing JavaScript dialog from a stale Page attachment or dialog scope"
    );
}

fn emit_to_attachment(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    destination: TargetPageProtocolAttachmentIdentity,
    source_frame_id: String,
    dialog: PreparedJavaScriptDialog,
) {
    if !source_is_current(conn, &dialog)
        || !conn.target_page_protocol_attachment_identity_is_current(&destination)
    {
        trace_stale_source(&dialog);
        dialog.dismiss();
        return;
    }

    let event_session_id = destination.session_id().map(str::to_owned);
    let destination_page_owner = destination.page_owner().clone();
    let source_url = dialog.source_url().to_owned();
    let message = dialog.message().to_owned();
    let dialog_type = dialog.dialog_type().to_owned();
    let default_prompt = dialog.default_prompt().to_owned();
    let mut target_dialog =
        Some(dialog.into_target_dialog(destination_page_owner, source_frame_id.clone()));
    let installed = conn.with_target_devtools_session_state_for_session_mut(
        event_session_id.as_deref(),
        |state| {
            state.page_session_state.javascript_dialog_state.push(
                target_dialog
                    .take()
                    .expect("dialog installation must consume its exact prepared output"),
            );
        },
    );
    if installed.is_none() {
        let dialog = target_dialog
            .take()
            .expect("missing target session must leave the prepared dialog unconsumed");
        let _ = dialog.finish(false, String::new());
        return;
    }
    out.push(BackgroundProtocolEvent::page_javascript_dialog_opening(
        event_session_id.as_deref(),
        PageJavaScriptDialogOpeningEvent {
            frame_id: Some(source_frame_id.into()),
            url: source_url,
            message,
            dialog_type,
            // Moli has no native browser UI capable of presenting or resolving
            // JavaScript dialogs. CDP clients can still resolve this dialog via
            // Page.handleJavaScriptDialog; `false` also lets the Chromium
            // DevTools frontend dismiss it instead of waiting for nonexistent UI.
            has_browser_handler: false,
            default_prompt,
        },
    ));
}

pub(super) fn settle_pending_popup_dialogs(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    browser_context_id: &str,
    popup_id: Option<u64>,
    target_id: Option<&str>,
) {
    let Some(popup_id) = popup_id else {
        return;
    };
    let dialogs = conn
        .browser_context_by_id_mut(browser_context_id)
        .map(|context| context.take_pending_popup_javascript_dialogs(popup_id))
        .unwrap_or_default();
    let Some(target_id) = target_id else {
        drop(dialogs);
        return;
    };
    emit_popup_dialogs_for_target(conn, out, browser_context_id, target_id, dialogs);
}

fn emit_popup_dialogs_for_target(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    browser_context_id: &str,
    target_id: &str,
    dialogs: Vec<PreparedJavaScriptDialog>,
) {
    let Some(destination) =
        conn.target_page_protocol_attachment_identity_for_target(browser_context_id, target_id)
    else {
        for dialog in &dialogs {
            tracing::debug!(
                browser_context_id,
                target_id,
                route = ?dialog.route(),
                "dismissing lightweight-popup dialog without an attached Page session"
            );
        }
        drop(dialogs);
        return;
    };
    let source_frame_id = conn
        .target_session_owner_frame_tree_identity(destination.session_id())
        .map(|(root_frame_id, _, _, _)| root_frame_id)
        .or_else(|| destination.page_owner().target_id().map(str::to_owned));
    let Some(source_frame_id) = source_frame_id else {
        drop(dialogs);
        return;
    };
    for dialog in dialogs {
        emit_to_attachment(
            conn,
            out,
            destination.clone(),
            source_frame_id.clone(),
            dialog,
        );
    }
}

#[cfg(test)]
pub(super) fn capture_for_test(
    source_page_owner: TargetPageResidenceIdentity,
    source_session_id: Option<&str>,
    dialog_scope: TargetJavaScriptDialogScopeObserver,
    root_frame_id: &str,
    renderer_dialog: RendererPendingJavaScriptDialog,
) -> PreparedJavaScriptDialog {
    TargetPreparedJavaScriptDialog::capture(
        TargetPageProtocolAttachmentIdentity::new(
            source_page_owner,
            source_session_id.map(str::to_owned),
        ),
        dialog_scope,
        root_frame_id,
        renderer_dialog,
    )
}
