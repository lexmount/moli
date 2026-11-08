use super::*;
use crate::automation::AutomationEvent;

pub(super) fn handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let command = match build_cdp_handle_javascript_dialog_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return plan,
    };
    if !cdp_session_has_pending_dialog(conn, cmd.session_id) {
        return CommandOutputPlan::error(-32602, "No dialog is showing");
    }
    complete_devtools_handle_javascript_dialog_command(conn, command)
}

pub(super) fn build_cdp_handle_javascript_dialog_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsHandleJavaScriptDialogCommand, CommandOutputPlan> {
    let params: HandleJavaScriptDialogParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(CommandOutputPlan::error(-32602, "InvalidParams")),
    };
    Ok(cdp_handle_javascript_dialog_command(
        conn,
        cmd,
        params.accept,
        Some(params.prompt_text.unwrap_or_default()),
    ))
}

fn cdp_handle_javascript_dialog_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    accept: bool,
    prompt_text: Option<String>,
) -> DevToolsHandleJavaScriptDialogCommand {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    DevToolsHandleJavaScriptDialogCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        accept,
        prompt_text,
    }
}

fn cdp_session_has_pending_dialog(conn: &CdpConnection, session_id: Option<&str>) -> bool {
    let Some(generation) = conn.page_domain_subscription_generation_for_session_owner(session_id)
    else {
        return false;
    };
    let owner = CommandOwnerScope::capture(conn, session_id);
    let current_page_owner = javascript_dialog_page_owner(conn, &owner);
    conn.target_javascript_dialog_state_for_owner(&owner)
        .and_then(|state| state.peek_next())
        .is_some_and(|dialog| {
            current_page_owner.as_ref() == Some(dialog.page_owner())
                && dialog.was_opened_for_subscription(generation)
        })
}

fn javascript_dialog_page_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Option<TargetPageResidenceIdentity> {
    conn.pending_target_page_residence_identity_for_owner(owner)
        .or_else(|| conn.target_page_residence_identity_for_owner(owner))
}

fn no_javascript_dialog_error() -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::NoSuchAlert, "No dialog is showing")
}

fn page_dialog_subscriptions(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Vec<(Option<String>, u64)> {
    conn.page_event_session_ids_for_owner(owner)
        .into_iter()
        .filter_map(|session_id| {
            let event_owner = session_id
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            let state = conn.target_devtools_session_state_for_owner(&event_owner)?;
            if state.is_closing() {
                return None;
            }
            Some((
                session_id,
                state
                    .page_session_state
                    .page_domain_subscription_generation()?,
            ))
        })
        .collect()
}

pub(super) enum PageDialogSubscriptionEnd {
    Disable,
    Detach,
}

pub(super) fn end_page_dialog_subscription(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    reason: PageDialogSubscriptionEnd,
) -> Option<Vec<BackgroundProtocolEvent>> {
    let owner = CommandOwnerScope::capture(conn, session_id);
    let generation = conn.page_domain_subscription_generation_for_session_owner(session_id);
    if !conn.disable_page_domain_for_session_owner(session_id) {
        return None;
    }
    if conn.has_automation_javascript_dialog_handler() {
        return Some(Vec::new());
    }
    Some(match reason {
        // Chromium's explicit Page.disable cancels this observer's callbacks.
        PageDialogSubscriptionEnd::Disable => generation
            .map(|generation| {
                dismiss_matching_dialogs(conn, &owner, |dialog| {
                    dialog.was_opened_for_subscription(generation)
                })
            })
            .unwrap_or_default(),
        // Detach only abandons dialogs with no remaining original observer.
        PageDialogSubscriptionEnd::Detach => {
            dismiss_orphaned_javascript_dialogs_for_owner(conn, &owner)
        }
    })
}

fn dismiss_matching_dialogs(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    matches: impl FnMut(&crate::conn::TargetJavaScriptDialog) -> bool,
) -> Vec<BackgroundProtocolEvent> {
    let current_page = javascript_dialog_page_owner(conn, owner);
    let dialogs = conn
        .with_target_javascript_dialog_state_for_owner_mut(owner, |state| {
            state.take_matching(matches)
        })
        .unwrap_or_default();
    let mut events = Vec::new();
    for dialog in dialogs {
        let _ = dialog.finish(false, String::new());
        if current_page.as_ref() == Some(dialog.page_owner()) {
            events.extend(broadcast_javascript_dialog_closed(
                conn,
                owner,
                UserPromptClosedEvent {
                    target_id: dialog.page_owner().target_id().map(Into::into),
                    frame_id: dialog.source_frame_id().into(),
                    prompt_type: dialog.dialog_type().to_owned(),
                    accepted: false,
                    user_text: String::new(),
                },
            ));
        }
    }
    events
}

pub(in crate::domains) fn prepare_page_session_disposal(
    conn: &mut CdpConnection,
    session_id: &str,
) -> Vec<BackgroundProtocolEvent> {
    end_page_dialog_subscription(conn, Some(session_id), PageDialogSubscriptionEnd::Detach)
        .unwrap_or_default()
}

fn dismiss_orphaned_javascript_dialogs_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Vec<BackgroundProtocolEvent> {
    let subscriptions = page_dialog_subscriptions(conn, owner);
    dismiss_matching_dialogs(conn, owner, |dialog| {
        !subscriptions
            .iter()
            .any(|(_, generation)| dialog.was_opened_for_subscription(*generation))
    })
}

pub(crate) fn dismiss_unhandled_javascript_dialogs(
    conn: &mut CdpConnection,
) -> Vec<BackgroundProtocolEvent> {
    if conn.has_automation_javascript_dialog_handler() {
        return Vec::new();
    }
    let owners = conn
        .browser_context
        .iter()
        .chain(conn.inactive_browser_contexts.iter())
        .flat_map(|context| {
            context
                .page_targets
                .active()
                .into_iter()
                .chain(context.background_targets())
                .map(move |target| {
                    CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
                        browser_context_id: context.id.clone(),
                        target_id: target.target_id().to_owned(),
                        session_key: moli_page_types::DevToolsSessionKey::Primary,
                    })
                })
        })
        .collect::<Vec<_>>();
    owners
        .iter()
        .flat_map(|owner| dismiss_orphaned_javascript_dialogs_for_owner(conn, owner))
        .collect()
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
        Ok(closed_events) => {
            let mut plan = CommandOutputPlan::default();
            for closed_event in closed_events {
                plan.push_background_event(closed_event);
            }
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
    let current_page_owner = javascript_dialog_page_owner(conn, owner);
    let dialog = conn
        .target_javascript_dialog_state_for_owner(owner)
        .and_then(|state| state.peek_next())
        .filter(|dialog| current_page_owner.as_ref() == Some(dialog.page_owner()))
        .ok_or_else(no_javascript_dialog_error)?;
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
) -> Result<AutomationResult, DevToolsError> {
    let current_page_owner = javascript_dialog_page_owner(conn, owner);
    conn.with_target_javascript_dialog_state_for_owner_mut(owner, |dialog_state| {
        let dialog = dialog_state
            .peek_next_mut()
            .filter(|dialog| current_page_owner.as_ref() == Some(dialog.page_owner()))
            .ok_or_else(no_javascript_dialog_error)?;
        if dialog.dialog_type() != "prompt" {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "Dialog is not a prompt",
            ));
        }
        dialog.set_prompt_text(command.prompt_text);
        Ok(AutomationResult::Empty)
    })
    .ok_or_else(no_javascript_dialog_error)?
}

pub(super) fn finish_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
    owner: &CommandOwnerScope,
) -> Result<Vec<BackgroundProtocolEvent>, DevToolsError> {
    let command_prompt_text = command.prompt_text;
    let current_page_owner = javascript_dialog_page_owner(conn, owner);
    let (dialog, stored_prompt_text) = conn
        .with_target_javascript_dialog_state_for_owner_mut(owner, |dialog_state| {
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
        .ok_or_else(no_javascript_dialog_error)?;
    let user_input = command_prompt_text
        .or(stored_prompt_text)
        .unwrap_or_default();
    let _ = dialog.finish(command.accept, user_input.clone());
    let closed_event = UserPromptClosedEvent {
        target_id: command.context.target_id,
        frame_id: dialog.source_frame_id().into(),
        prompt_type: dialog.dialog_type().to_owned(),
        accepted: command.accept,
        user_text: user_input,
    };
    // Chromium reports the closing dialog to every session attached to the page
    // that has the Page domain enabled, regardless of which session resolved it.
    Ok(broadcast_javascript_dialog_closed(
        conn,
        owner,
        closed_event,
    ))
}

fn broadcast_javascript_dialog_closed(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    closed_event: UserPromptClosedEvent,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = vec![BackgroundProtocolEvent::automation_only(
        AutomationEvent::UserPromptClosed(closed_event.clone()),
    )];
    events.extend(
        page_dialog_subscriptions(conn, owner)
            .into_iter()
            .map(|(session_id, _)| {
                BackgroundProtocolEvent::page_javascript_dialog_closed(
                    session_id.as_deref(),
                    closed_event.clone(),
                )
            }),
    );
    events
}

pub(in crate::domains) fn emit_javascript_dialog_activity_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    prepared_outputs: &mut ProtocolOutputPayloads,
) {
    if let Some(dialogs) = prepared_outputs
        .page_mut()
        .and_then(PagePreparedOutputSlot::take_javascript_dialogs)
    {
        javascript_dialog::emit_prepared(conn, out, dialogs);
    }
}

pub(super) fn execute_devtools_get_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsGetJavaScriptDialogCommand,
) -> Result<AutomationResult, DevToolsError> {
    let owner = page_command_owner(conn, &command.context)?;
    let result = finish_devtools_get_javascript_dialog_command(conn, &owner);
    result.map(AutomationResult::JavaScriptDialog)
}

pub(super) fn execute_devtools_set_javascript_dialog_prompt_text_command(
    conn: &mut CdpConnection,
    command: DevToolsSetJavaScriptDialogPromptTextCommand,
) -> Result<AutomationResult, DevToolsError> {
    let owner = page_command_owner(conn, &command.context)?;
    finish_devtools_set_javascript_dialog_prompt_text_command(conn, command, &owner)
}

pub(super) fn execute_devtools_handle_javascript_dialog_command(
    conn: &mut CdpConnection,
    command: DevToolsHandleJavaScriptDialogCommand,
) -> (
    Result<AutomationResult, DevToolsError>,
    Vec<BackgroundProtocolEvent>,
) {
    let owner = match page_command_owner(conn, &command.context) {
        Ok(owner) => owner,
        Err(error) => return (Err(error), Vec::new()),
    };
    let result = finish_devtools_handle_javascript_dialog_command(conn, command, &owner);
    match result {
        Ok(events) => (Ok(AutomationResult::Empty), events),
        Err(error) => (Err(error), Vec::new()),
    }
}

#[cfg(test)]
use moli_core::page::RendererPendingJavaScriptDialog;

use crate::automation::PageJavaScriptDialogOpeningEvent;
#[cfg(test)]
use crate::conn::TargetJavaScriptDialogScopeObserver;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, TargetPageProtocolAttachmentIdentity,
    TargetPageResidenceIdentity, TargetPreparedJavaScriptDialog,
    TargetPreparedJavaScriptDialogRoute,
};

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

fn dialog_attachment_is_current(
    conn: &CdpConnection,
    attachment: &TargetPageProtocolAttachmentIdentity,
) -> bool {
    if conn.target_page_output_attachment_is_current(attachment) {
        return true;
    }
    let owner = CommandOwnerScope::for_page_attachment(attachment);
    // A newly opened auxiliary Window can run a modal dialog before its
    // initial Page handle is adopted. That adoption waits for the paused
    // renderer turn, so use the exact initial-build reservation here.
    conn.runtime_session_owner_slot_for_owner(&owner)
        .is_some_and(|slot| slot.has_initial_document_page_build_in_progress())
        && conn
            .pending_target_page_residence_identity_for_owner(&owner)
            .as_ref()
            == Some(attachment.page_owner())
}

fn source_is_current(conn: &CdpConnection, dialog: &PreparedJavaScriptDialog) -> bool {
    dialog_attachment_is_current(conn, dialog.source_attachment())
        && conn
            .runtime_session_owner_slot_for_owner(&CommandOwnerScope::for_page_attachment(
                dialog.source_attachment(),
            ))
            .is_some_and(|slot| slot.observes_javascript_dialog_scope(dialog.source_dialog_scope()))
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
    if !source_is_current(conn, &dialog) || !dialog_attachment_is_current(conn, &destination) {
        trace_stale_source(&dialog);
        dialog.dismiss();
        return;
    }

    let event_owner = CommandOwnerScope::for_page_attachment(&destination);
    let destination_page_owner = destination.page_owner().clone();
    let target_id = destination_page_owner.target_id().map(Into::into);
    let source_url = dialog.source_url().to_owned();
    let message = dialog.message().to_owned();
    let dialog_type = dialog.dialog_type().to_owned();
    let default_prompt = dialog.default_prompt().to_owned();
    let audience = page_dialog_subscriptions(conn, &event_owner);
    let has_handler = !audience.is_empty() || conn.has_automation_javascript_dialog_handler();
    let opening_event = PageJavaScriptDialogOpeningEvent {
        frame_id: Some(source_frame_id.clone().into()),
        url: source_url,
        message,
        dialog_type: dialog_type.clone(),
        // Moli has no native browser UI that can resolve the dialog.
        has_browser_handler: false,
        default_prompt,
    };
    if !has_handler {
        // A late prefix without a handler settles only this dialog. Existing
        // queued dialogs belong to their own subscription cleanup paths.
        dialog.dismiss();
        out.push(BackgroundProtocolEvent::automation_only(
            AutomationEvent::PageJavaScriptDialogOpening(opening_event),
        ));
        out.push(BackgroundProtocolEvent::automation_only(
            AutomationEvent::UserPromptClosed(UserPromptClosedEvent {
                target_id: destination_page_owner.target_id().map(Into::into),
                frame_id: source_frame_id.into(),
                prompt_type: dialog_type,
                accepted: false,
                user_text: String::new(),
            }),
        ));
        return;
    }
    let subscriptions = audience.iter().map(|(_, generation)| *generation).collect();
    // The Page owns the completion; sessions only hold permission to handle it.
    let installed =
        conn.with_target_javascript_dialog_state_for_owner_mut(&event_owner, move |state| {
            let mut installed_dialog =
                dialog.into_target_dialog(destination_page_owner, source_frame_id);
            installed_dialog.set_opening_subscription_generations(subscriptions);
            state.push(installed_dialog);
        });
    if installed.is_none() {
        return;
    }

    out.push(BackgroundProtocolEvent::automation_only(
        AutomationEvent::PageJavaScriptDialogOpening(opening_event.clone()),
    ));
    for (event_session_id, _) in audience {
        out.push(BackgroundProtocolEvent::page_javascript_dialog_opening(
            event_session_id.as_deref(),
            opening_event.clone(),
        ));
    }
    if let Some(accept) = conn.automation_javascript_dialog_response(&dialog_type) {
        let command = DevToolsHandleJavaScriptDialogCommand {
            context: crate::automation::AutomationContext {
                protocol: crate::automation::FrontendProtocol::WebDriverBidi,
                session_id: destination.session_id().map(Into::into),
                target_id,
                browser_context_id: Some(destination.page_owner().browser_context_id().into()),
            },
            accept,
            prompt_text: Some(String::new()),
        };
        if let Ok(events) =
            finish_devtools_handle_javascript_dialog_command(conn, command, &event_owner)
        {
            out.extend(events);
        }
    }
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
