use anyhow::Result;

use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererDocumentLifecycleIdentity,
    RendererDomBidiNodeSharedIdResolution, RendererPendingFileChooserActivation,
};

use crate::automation::{
    DevToolsRemoteHandleId, webdriver_bidi_node_shared_id_for_backend_node_id,
};
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, NoDocumentLoaded,
    ProtocolWorkflowDriver, TargetPageResidenceIdentity,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedFileChooserActivation {
    page_owner: TargetPageResidenceIdentity,
    source_document: RendererDocumentLifecycleIdentity,
    source_frame_id: String,
    backend_node_id: u32,
    allow_multiple: bool,
}

impl PreparedFileChooserActivation {
    pub(crate) fn capture(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        page_owner: TargetPageResidenceIdentity,
        activation: RendererPendingFileChooserActivation,
    ) -> Option<Self> {
        if !conn.target_page_residence_identity_is_current(&page_owner)
            || conn
                .target_page_residence_identity_for_owner(owner)
                .as_ref()
                != Some(&page_owner)
        {
            return None;
        }
        let source_document = activation.source_document();
        let source_frame_id = activation
            .source_frame_id()
            .map(str::to_owned)
            .or_else(|| {
                conn.target_session_owner_frame_tree_identity_for_owner(owner)
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

#[cfg(test)]
pub(super) async fn emit_prepared_activations_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    activations: Vec<PreparedFileChooserActivation>,
) {
    for activation in activations {
        emit_prepared_activation_async(conn, out, owner, activation).await;
    }
}

#[cfg(test)]
pub(super) async fn emit_prepared_activation_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    activation: PreparedFileChooserActivation,
) {
    emit_prepared_activation(
        &mut ProtocolWorkflowDriver::inline(conn),
        out,
        owner,
        activation,
    )
    .await;
}

pub(super) async fn emit_prepared_activation(
    driver: &mut ProtocolWorkflowDriver<'_>,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    activation: PreparedFileChooserActivation,
) {
    let query_activation = activation.clone();
    let query_owner = owner.clone();
    let pending = match driver
        .with_state(move |state| {
            state.start_input_file_chooser_query(&query_owner, &query_activation)
        })
        .await
    {
        Ok(Some(pending)) => pending,
        Ok(None) | Err(_) => return,
    };
    let resolution = match pending {
        Ok(pending) => match pending.wait().await {
            Ok(completed) => {
                let query_activation = activation.clone();
                driver
                    .with_state(move |state| {
                        state.finish_input_file_chooser_query(&query_activation, completed)
                    })
                    .await
                    .and_then(std::convert::identity)
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    let element_shared_id = match resolution {
        Ok(RendererDomBidiNodeSharedIdResolution::SharedId(shared_id)) => shared_id.into(),
        resolution => {
            if let Err(error) = resolution {
                tracing::debug!(%error, backend_node_id = activation.backend_node_id,
                    "failed to resolve renderer BiDi shared id for file chooser");
            }
            let shared_id =
                webdriver_bidi_node_shared_id_for_backend_node_id(activation.backend_node_id);
            let registration_activation = activation.clone();
            let registration_owner = owner.clone();
            let binding = shared_id.as_str().to_owned();
            let pending = driver
                .with_state(move |state| {
                    state.start_input_file_chooser_binding(
                        &registration_owner,
                        &registration_activation,
                        binding,
                    )
                })
                .await
                .and_then(std::convert::identity);
            let registered = match pending {
                Ok(pending) => match pending.wait().await {
                    Ok(completed) => {
                        let registration_activation = activation.clone();
                        driver
                            .with_state(move |state| {
                                state.finish_input_file_chooser_binding(
                                    &registration_activation,
                                    completed,
                                )
                            })
                            .await
                            .and_then(std::convert::identity)
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };
            if let Err(error) = registered {
                tracing::debug!(%error, shared_id = shared_id.as_str(), backend_node_id = activation.backend_node_id,
                    "failed to register renderer BiDi shared id for file chooser; preserving event identity");
            }
            shared_id
        }
    };
    let session_id = owner.session_id().map(str::to_owned);
    if let Ok(Some(event)) = driver
        .with_state(move |state| {
            state.finish_input_file_chooser_activation(
                session_id.as_deref(),
                activation,
                element_shared_id,
            )
        })
        .await
    {
        out.push(event);
    }
}

impl PreparedFileChooserActivation {
    pub(crate) fn start_query(
        &self,
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
    ) -> Option<Result<PendingPageCommand>> {
        if !activation_is_current(conn, self)
            || conn
                .target_page_residence_identity_for_owner(owner)
                .as_ref()
                != Some(&self.page_owner)
        {
            trace_stale_activation(owner.session_id(), self);
            return None;
        }
        if !conn
            .target_page_session_state_for_owner(owner)
            .is_some_and(|state| {
                state.page_intercept_file_chooser_dialog_enabled
                    || state.page_file_chooser_opened_event_enabled
            })
        {
            return None;
        }
        let inspector = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
        Some(self.page(conn).and_then(|page| {
            page.start_document_bidi_node_shared_id_for_backend_node_id(
                inspector,
                self.backend_node_id,
            )
        }))
    }

    pub(crate) fn finish_query(
        &self,
        conn: &mut CdpConnection,
        completed: CompletedPageCommand,
    ) -> Result<RendererDomBidiNodeSharedIdResolution> {
        self.page(conn)?
            .finish_document_bidi_node_shared_id_for_backend_node_id(completed)
    }

    pub(crate) fn start_binding(
        &self,
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        shared_id: String,
    ) -> Result<PendingPageCommand> {
        if conn
            .target_page_residence_identity_for_owner(owner)
            .as_ref()
            != Some(&self.page_owner)
        {
            return Err(anyhow::Error::new(NoDocumentLoaded));
        }
        let inspector = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
        self.page(conn)?.start_register_document_bidi_node_binding(
            inspector,
            shared_id,
            self.backend_node_id,
        )
    }

    pub(crate) fn finish_binding(
        &self,
        conn: &mut CdpConnection,
        completed: CompletedPageCommand,
    ) -> Result<()> {
        self.page(conn)?
            .finish_register_document_bidi_node_binding(completed)
    }

    fn page<'a>(&self, conn: &'a mut CdpConnection) -> Result<&'a mut moli_core::page::Page> {
        if !activation_is_current(conn, self) {
            return Err(anyhow::Error::new(NoDocumentLoaded));
        }
        conn.loaded_page_mut_for_protocol_access_for_owner(&CommandOwnerScope::for_page_residence(
            &self.page_owner,
        ))
    }

    pub(crate) fn finish_event(
        self,
        conn: &CdpConnection,
        session_id: Option<&str>,
        shared_id: DevToolsRemoteHandleId,
    ) -> Option<BackgroundProtocolEvent> {
        if !activation_is_current(conn, &self)
            || session_id.is_some_and(|session_id| {
                conn.target_page_residence_identity_for_session(Some(session_id))
                    .as_ref()
                    != Some(&self.page_owner)
            })
        {
            trace_stale_activation(session_id, &self);
            return None;
        }
        Some(BackgroundProtocolEvent::page_file_chooser_opened(
            session_id,
            &self.source_frame_id,
            if self.allow_multiple {
                "selectMultiple"
            } else {
                "selectSingle"
            },
            self.backend_node_id,
            Some(shared_id),
        ))
    }
}

fn activation_is_current(conn: &CdpConnection, activation: &PreparedFileChooserActivation) -> bool {
    conn.target_page_residence_identity_is_current(&activation.page_owner)
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
