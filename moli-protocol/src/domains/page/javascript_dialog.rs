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
            has_browser_handler: true,
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
