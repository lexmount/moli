use moli_core::page::{
    RendererDocumentLifecycleIdentity, RendererDomBidiNodeSharedIdResolution,
    RendererPendingFileChooserActivation,
};

use crate::conn::{BackgroundProtocolEvent, CdpConnection, TargetPageResidenceIdentity};
use crate::devtools_runtime::{
    DevToolsRemoteHandleId, webdriver_bidi_node_shared_id_for_backend_node_id,
};

/// A renderer file-chooser activation bound to the exact Page residence from
/// which it was captured, with its causal Document and frame frozen as event
/// metadata.
///
/// The renderer resolves a live source node to a backend id before this value
/// is built. Capture resolves the compact root-frame representation once.
/// Apply may survive `document.open()` within the same Page, but a replacement
/// Page attachment replacement retires the prepared activation before a
/// colliding backend id can be observed.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct PreparedFileChooserActivation {
    page_owner: TargetPageResidenceIdentity,
    source_document: RendererDocumentLifecycleIdentity,
    source_frame_id: String,
    backend_node_id: u32,
    allow_multiple: bool,
}

impl PreparedFileChooserActivation {
    pub(super) fn capture(
        conn: &CdpConnection,
        session_id: Option<&str>,
        page_owner: TargetPageResidenceIdentity,
        activation: RendererPendingFileChooserActivation,
    ) -> Option<Self> {
        if !conn.target_page_residence_identity_is_current_for_session(session_id, &page_owner) {
            return None;
        }
        let source_document = activation.source_document();
        let source_frame_id = activation
            .source_frame_id()
            .map(str::to_owned)
            .or_else(|| {
                conn.target_session_owner_frame_tree_identity(session_id)
                    .map(|(frame_id, _, _, _)| frame_id)
            })?;
        Some(Self {
            page_owner,
            source_document,
            source_frame_id,
            backend_node_id: activation.backend_node_id(),
            allow_multiple: activation.allow_multiple(),
        })
    }

    #[cfg(test)]
    pub(super) fn from_renderer_for_test(
        page_owner: TargetPageResidenceIdentity,
        root_frame_id: &str,
        activation: RendererPendingFileChooserActivation,
    ) -> Self {
        let source_document = activation.source_document();
        let source_frame_id = activation
            .source_frame_id()
            .unwrap_or(root_frame_id)
            .to_owned();
        Self {
            page_owner,
            source_document,
            source_frame_id,
            backend_node_id: activation.backend_node_id(),
            allow_multiple: activation.allow_multiple(),
        }
    }

    #[cfg(test)]
    pub(super) fn source_document(&self) -> RendererDocumentLifecycleIdentity {
        self.source_document
    }

    #[cfg(test)]
    pub(super) fn source_frame_id(&self) -> &str {
        &self.source_frame_id
    }
}

pub(super) async fn emit_prepared_activations_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    activations: Vec<PreparedFileChooserActivation>,
) {
    for activation in activations {
        emit_prepared_activation_async(conn, out, session_id, activation).await;
    }
}

pub(super) async fn emit_prepared_activation_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    activation: PreparedFileChooserActivation,
) {
    if !activation_is_current(conn, session_id, &activation) {
        trace_stale_activation(session_id, &activation);
        return;
    }
    let event_enabled = conn
        .target_page_session_state_for_session(session_id)
        .is_some_and(|state| {
            state.page_intercept_file_chooser_dialog_enabled
                || state.page_file_chooser_opened_event_enabled
        });
    if !event_enabled {
        return;
    }
    let mode = if activation.allow_multiple {
        "selectMultiple"
    } else {
        "selectSingle"
    };
    let element_shared_id =
        element_shared_id_async(conn, session_id, activation.backend_node_id).await;
    if !activation_is_current(conn, session_id, &activation) {
        trace_stale_activation(session_id, &activation);
        return;
    }
    out.push(BackgroundProtocolEvent::page_file_chooser_opened(
        session_id,
        &activation.source_frame_id,
        mode,
        activation.backend_node_id,
        Some(element_shared_id),
    ));
}

fn activation_is_current(
    conn: &CdpConnection,
    session_id: Option<&str>,
    activation: &PreparedFileChooserActivation,
) -> bool {
    conn.target_page_residence_identity_is_current_for_session(session_id, &activation.page_owner)
}

fn trace_stale_activation(session_id: Option<&str>, activation: &PreparedFileChooserActivation) {
    tracing::debug!(
        session_id,
        ?activation.source_document,
        source_frame_id = activation.source_frame_id,
        backend_node_id = activation.backend_node_id,
        browser_context_id = activation.page_owner.browser_context_id(),
        target_id = activation.page_owner.target_id(),
        page_attachment_id = activation.page_owner.page_attachment_id().get(),
        "dropping file chooser produced by a stale Page residence"
    );
}

async fn element_shared_id_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    backend_node_id: u32,
) -> DevToolsRemoteHandleId {
    // The event identity belongs to the frozen activation, while the DOM-agent
    // binding belongs to whichever Document is live when protocol consumes the
    // owner action. A synchronous document.open() may retire that binding
    // before or during this async handoff; it must not erase the element from
    // the already-accepted file-chooser event.
    match conn
        .document_bidi_node_shared_id_for_backend_node_id_for_session_owner_async(
            session_id,
            backend_node_id,
        )
        .await
    {
        Ok(RendererDomBidiNodeSharedIdResolution::SharedId(shared_id)) => {
            return shared_id.into();
        }
        Ok(RendererDomBidiNodeSharedIdResolution::NotFound) => {}
        Err(error) => {
            tracing::debug!(
                %error,
                backend_node_id,
                "failed to resolve renderer BiDi shared id for file chooser"
            );
        }
    }

    let shared_id = webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    if let Err(error) = conn
        .register_document_bidi_node_binding_for_session_owner_async(
            session_id,
            shared_id.as_str(),
            backend_node_id,
        )
        .await
    {
        tracing::debug!(
            %error,
            shared_id = shared_id.as_str(),
            backend_node_id,
            "failed to register renderer BiDi shared id for file chooser; preserving event identity"
        );
    }
    shared_id
}
