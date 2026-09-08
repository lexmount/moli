use std::sync::Arc;

use crate::conn::{BackgroundProtocolEvent, CdpConnection};
use moli_core::page::{RendererPendingWindowOpenEvent, RendererPopupOpening};

/// A FIFO observation, not permission to create or activate a Window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagePreparedPopupOpening {
    opening: Arc<RendererPopupOpening>,
}

impl PagePreparedPopupOpening {
    pub(super) fn new(opening: Arc<RendererPopupOpening>) -> Self {
        Self { opening }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagePreparedWindowOpenEvent {
    session_ids: Vec<Option<String>>,
    event: RendererPendingWindowOpenEvent,
}

impl PagePreparedWindowOpenEvent {
    pub(super) fn new(
        session_ids: Vec<Option<String>>,
        event: RendererPendingWindowOpenEvent,
    ) -> Self {
        Self { session_ids, event }
    }
}

pub(super) fn emit_window_open_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    events: Vec<PagePreparedWindowOpenEvent>,
) {
    for prepared in events {
        for session_id in prepared.session_ids {
            out.push(BackgroundProtocolEvent::page_window_open(
                session_id.as_deref(),
                &prepared.event.url,
                &prepared.event.window_name,
                &prepared.event.window_features,
                prepared.event.user_gesture,
            ));
        }
    }
}

pub(super) async fn emit_prepared(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    openings: Vec<PagePreparedPopupOpening>,
) {
    for prepared in openings {
        let opening = prepared.opening;
        let Some(admission) = conn.wait_for_renderer_popup(opening.clone()).await else {
            continue;
        };
        out.extend(conn.project_observed_popup(admission.web_contents).await);
        let Some(context) = conn.browser_context_by_browser_id(admission.web_contents.context())
        else {
            continue;
        };
        let browser_context_id = context.id.clone();
        let target_id = context
            .target_id_for_web_contents(admission.web_contents.id())
            .map(str::to_owned);
        if !admission.created
            && let Some(target_id) = target_id.as_deref()
        {
            crate::domains::target::observe_reused_popup_navigation(
                conn,
                out,
                &browser_context_id,
                target_id,
                opening.url(),
            );
        }
        super::javascript_dialog::settle_pending_popup_dialogs(
            conn,
            out,
            &browser_context_id,
            admission.source_document,
            opening.popup_id(),
            target_id.as_deref(),
        );
    }
}

/// Produce real renderer input but deliberately leave its FIFO unprojected.
/// Projection-unit tests can then install/retire attachments before emission.
#[cfg(test)]
pub(super) async fn capture_openings_for_test(
    conn: &mut CdpConnection,
    owner: &crate::conn::TargetPageResidenceIdentity,
    script: &str,
) -> Vec<Arc<RendererPopupOpening>> {
    capture_popup_script_for_test(conn, owner, script, 1).await
}

#[cfg(test)]
async fn capture_popup_script_for_test(
    conn: &mut CdpConnection,
    owner: &crate::conn::TargetPageResidenceIdentity,
    script: &str,
    expected: usize,
) -> Vec<Arc<RendererPopupOpening>> {
    use crate::conn::CdpCommandTaskStep;
    let attachment = conn
        .target_page_protocol_attachment_identity_for_target(
            owner.browser_context_id(),
            owner.target_id().unwrap(),
        )
        .unwrap();
    let (sender, mut receiver) = moli_core::renderer_output_transport_channel();
    conn.browser_context_by_id(owner.browser_context_id())
        .unwrap()
        .loaded_document_renderer_inspection_endpoint_for_test()
        .unwrap()
        .bind_output_transport(sender.clone())
        .unwrap();
    conn.set_renderer_publication_sender(sender);
    let raw = serde_json::json!({
        "id": 900001, "method": "Runtime.evaluate",
        "sessionId": attachment.session_id(),
        "params": { "expression": script },
    })
    .to_string();
    let mut step = conn.start_command_dispatch(&raw);
    while let CdpCommandTaskStep::Pending(pending) = step {
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), pending.wait())
            .await
            .expect("popup-producing Runtime command");
        step = conn.complete_pending_command_dispatch(completed).await;
    }
    let openings = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut openings = Vec::new();
        while openings.len() < expected {
            let message = receiver.recv().await.expect("renderer input stream");
            if let moli_core::RendererOutputTransportMessage::Publication(publication) = message {
                for record in publication.into_records() {
                    if let moli_core::RendererOutputItem::Observation(
                        moli_core::RendererProtocolObservation::Popup(opening),
                    ) = record.into_parts().1
                    {
                        openings.push(opening);
                    }
                }
            }
        }
        openings
    })
    .await
    .expect("exact number of accepted popup inputs");
    for opening in &openings {
        conn.wait_for_renderer_popup(opening.clone())
            .await
            .expect("native admission");
    }
    // A later explicit focus change is independent of the held observation.
    // Keep these attachment-projection fixtures on their original source page.
    conn.select_page_target_for_connection_async(owner.target_id().unwrap())
        .await
        .unwrap();
    conn.take_scheduler_events();
    openings
}

#[cfg(test)]
mod tests;
