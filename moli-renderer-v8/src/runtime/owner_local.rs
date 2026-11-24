use std::{marker::PhantomData, rc::Rc, sync::Arc};

use super::owner_local_store::{
    RendererPageToken, has_current_render_runtime_owner_local_store,
    remove_page_on_bound_owner_local_store,
};
use super::*;
use crate::page_task_queue::RendererOwnerWake;
use crate::render_runtime::RenderRuntimeHandle;
use anyhow::anyhow;
use tokio::sync::oneshot;

fn remove_page(token: RendererPageToken) {
    remove_page_on_bound_owner_local_store(token)
}

#[doc(hidden)]
pub struct RendererAttachedPage {
    pub(super) vm_creation_id: u64,
    pub(super) token: RendererPageToken,
    pub(super) devtools_agent_token: RendererDevToolsAgentToken,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) javascript_dialog_broker: RendererJavaScriptDialogBroker,
    pub(super) devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
    #[cfg(feature = "devtools")]
    pub(super) script_execution_control:
        crate::script_execution_control::RendererScriptExecutionControl,
    pub(super) page_state: Arc<RendererPageState>,
    pub(super) creation_diagnostics: RendererPageCreationDiagnostics,
    pub(super) creation_artifacts: RendererPageCreationArtifacts,
    pub(super) pending_download: Option<RendererPendingDownloadActivation>,
    pub(super) committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

impl RendererAttachedPage {
    pub(super) fn defer_committed_document_parser_until_response(
        &mut self,
        page_wake_tx: tokio::sync::mpsc::UnboundedSender<RendererOwnerWake>,
    ) {
        debug_assert!(
            self.committed_document_post_response_continuation.is_none(),
            "committed-Document parser continuation can only be armed once"
        );
        let token = self.token;
        let vm_creation_id = self.vm_creation_id;
        self.committed_document_post_response_continuation = Some(
            RendererPageCommandPostResponseContinuation::new(move || {
                let _ = page_wake_tx.send(RendererOwnerWake::committed_document_parser_unblocked(
                    token,
                    vm_creation_id,
                ));
            }),
        );
    }

    pub(super) fn into_parts(
        self,
        local_executor: JsLocalExecutor,
        render_runtime: RenderRuntimeHandle,
    ) -> (
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    ) {
        (
            RendererPageHandle {
                local_executor,
                render_runtime,
                token: Some(self.token),
                devtools_agent_token: self.devtools_agent_token,
                page_context_cancel_tx: self.page_context_cancel_tx,
                javascript_dialog_broker: self.javascript_dialog_broker,
                devtools_target: self.devtools_target,
                #[cfg(feature = "devtools")]
                script_execution_control: self.script_execution_control,
                committed_document_post_response_continuation: self
                    .committed_document_post_response_continuation,
                _not_send: PhantomData,
            },
            self.page_state,
            self.creation_diagnostics,
            self.creation_artifacts,
            self.pending_download,
        )
    }
}

/// The committed Document of an existing Page. This value carries no Page
/// close authority; the original RendererPageHandle remains the sole owner.
pub struct RendererPageReplacementCommit {
    pub(super) token: RendererPageToken,
    pub(super) slot: RendererPageSlotHandle,
    pub(super) vm_creation_id: u64,
    pub(super) devtools_agent_token: RendererDevToolsAgentToken,
    pub(super) javascript_dialog_broker: RendererJavaScriptDialogBroker,
    pub(super) devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
    pub(super) page_state: Arc<RendererPageState>,
    pub(super) creation_diagnostics: RendererPageCreationDiagnostics,
    pub(super) creation_artifacts: RendererPageCreationArtifacts,
    pub(super) pending_download: Option<RendererPendingDownloadActivation>,
    pub(super) committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

impl RendererPageReplacementCommit {
    pub fn take_pending_download(&mut self) -> Option<RendererPendingDownloadActivation> {
        self.pending_download.take()
    }

    pub(super) fn defer_committed_document_parser_until_response(
        &mut self,
        page_wake_tx: tokio::sync::mpsc::UnboundedSender<RendererOwnerWake>,
    ) {
        assert!(self.committed_document_post_response_continuation.is_none());
        let token = self.token;
        let vm_creation_id = self.vm_creation_id;
        self.committed_document_post_response_continuation = Some(
            RendererPageCommandPostResponseContinuation::new(move || {
                let _ = page_wake_tx.send(RendererOwnerWake::committed_document_parser_unblocked(
                    token,
                    vm_creation_id,
                ));
            }),
        );
    }
}

pub struct RendererPageHandle {
    local_executor: JsLocalExecutor,
    render_runtime: RenderRuntimeHandle,
    token: Option<RendererPageToken>,
    devtools_agent_token: RendererDevToolsAgentToken,
    page_context_cancel_tx: RendererPageContextCancelSender,
    javascript_dialog_broker: RendererJavaScriptDialogBroker,
    devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
    #[cfg(feature = "devtools")]
    script_execution_control: crate::script_execution_control::RendererScriptExecutionControl,
    committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
    _not_send: PhantomData<Rc<()>>,
}

/// Revocable DevTools command destination for the exact admitted agent. It
/// carries no Page close authority and cannot follow a replacement Document.
#[cfg(feature = "devtools")]
pub struct RendererPageProtocolCommandTarget {
    token: RendererPageToken,
    agent: RendererDevToolsAgentToken,
    target: crate::devtools::target::RendererDevToolsTargetHandle,
    dialogs: RendererJavaScriptDialogBroker,
}

#[cfg(feature = "devtools")]
impl RendererPageProtocolCommandTarget {
    pub fn enqueue(
        self,
        command: RendererPageCommand,
        session: Option<String>,
    ) -> Result<RendererPageCommandPending> {
        let javascript_dialog_watch =
            renderer_page_javascript_dialog_watch(&command, || self.dialogs.watch());
        let route = self.target.main_ref().enqueue_protocol_page_command(
            self.token,
            self.agent,
            command,
            session,
            RendererPageStateCapturePolicy::ProtocolTurn,
        );
        let cleanup_control = Some(route.cleanup_control());
        Ok(RendererPageCommandPending {
            cleanup_control,
            dispatch: RendererPageCommandPendingDispatch::InspectorMain(Box::pin(
                route.wait_for_completion(),
            )),
            javascript_dialog_watch,
        })
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum RendererPageAccessError {
    #[error("Renderer Document is no longer current")]
    DocumentReplaced,
    #[error("renderer local host page {} is already running an owner turn", .page_id.as_u64())]
    Busy { page_id: super::PageId },
    #[error("renderer local host no longer tracks page {}", .page_id.as_u64())]
    Missing { page_id: super::PageId },
    #[error("renderer local host page {} is retiring", .page_id.as_u64())]
    Retiring { page_id: super::PageId },
}

/// A revocable command capability for one exact native Document. It keeps
/// neither a Page handle nor the renderer runtime owner alive.
#[derive(Clone, Debug)]
pub struct RendererDocumentCommandTarget {
    pub(super) render_runtime: RenderRuntimeHandle,
    pub(super) token: RendererPageToken,
    pub(super) document: RendererDocumentLifecycleIdentity,
}

/// Target-owned request policy that may change after native commit admission
/// but before the browser has observed the new Document's identity.
/// This deliberately grants no Document query or request-settlement access.
#[derive(Clone, Debug)]
pub enum RendererPageRequestPolicyUpdate {
    #[cfg(feature = "devtools")]
    FetchInterception {
        classes: crate::SubresourceInterceptionClasses,
    },
    BlockedUrls(Vec<String>),
}

impl RendererPageRequestPolicyUpdate {
    pub(super) fn into_command(self) -> RendererPageCommand {
        match self {
            #[cfg(feature = "devtools")]
            Self::FetchInterception { classes } => RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::SetFetchSubresourceInterception { classes },
            ),
            Self::BlockedUrls(patterns) => RendererPageCommand::SetBlockedUrlPatterns(patterns),
        }
    }
}

impl PartialEq for RendererDocumentCommandTarget {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token && self.document == other.document
    }
}

impl Eq for RendererDocumentCommandTarget {}

impl RendererDocumentCommandTarget {
    pub fn document(&self) -> RendererDocumentLifecycleIdentity {
        self.document
    }

    pub fn enqueue(&self, command: RendererPageCommand) -> Result<RendererPageCommandPending> {
        let reply = self
            .render_runtime
            .enqueue(RendererOwnerCommand::RunProtocolPageCommand {
                token: self.token,
                command: RendererPageCommand::ForDocument {
                    source_document: self.document,
                    command: Box::new(command),
                },
            })?;
        Ok(RendererPageCommandPending::from_owner_reply(reply))
    }
}

pub struct RendererPageCommandPending {
    dispatch: RendererPageCommandPendingDispatch,
    javascript_dialog_watch: Option<RendererJavaScriptDialogWatch>,
    #[cfg(feature = "devtools")]
    cleanup_control: Option<crate::devtools::ingress::main::RendererInspectorMainCleanupControl>,
}

impl RendererPageCommandPending {
    #[cfg(feature = "devtools")]
    pub fn retain_session_cleanup_authority(mut self) -> Self {
        if let Some(control) = self.cleanup_control.take() {
            control.retain();
        }
        self
    }

    pub(super) fn from_owner_reply(reply: oneshot::Receiver<Result<RendererOwnerReply>>) -> Self {
        Self {
            dispatch: RendererPageCommandPendingDispatch::Owner(reply),
            javascript_dialog_watch: None,
            #[cfg(feature = "devtools")]
            cleanup_control: None,
        }
    }
}

enum RendererPageCommandPendingDispatch {
    Owner(oneshot::Receiver<anyhow::Result<RendererOwnerReply>>),
    #[cfg(feature = "devtools")]
    InspectorMain(
        futures_util::future::BoxFuture<
            'static,
            Result<RendererRuntimeInspectorMainCommandCompletion>,
        >,
    ),
}

#[cfg(test)]
#[derive(Debug)]
#[doc(hidden)]
pub struct RendererRealtimeLayoutObservation {
    #[cfg(feature = "devtools")]
    pub(crate) builds: u64,
    #[cfg(feature = "devtools")]
    pub(crate) last_pass_paint_operations: Option<usize>,
    #[cfg(feature = "devtools")]
    pub(crate) screen_publishes: u64,
    #[cfg(feature = "devtools")]
    pub(crate) screen_snapshot: Option<(
        crate::document_runtime::DomHandle,
        moli_layout::LayoutTreeRetentionMetrics,
    )>,
    #[cfg(feature = "devtools")]
    pub(crate) frame_idle: bool,
    pub(crate) main_parser_source_waiting: bool,
}

#[derive(Clone)]
pub struct RendererPageTestingHandle {
    render_runtime: RenderRuntimeHandle,
    token: RendererPageToken,
    _not_send: PhantomData<Rc<()>>,
}

#[must_use = "the owner detach command must retain this guard until it drops the V8 session"]
#[cfg(feature = "devtools")]
pub struct RendererRuntimeInspectorSessionDetachGuard {
    inspector_pause_bridge: Option<crate::devtools::pause::RendererInspectorPauseBridge>,
    pending_detach: Option<(
        crate::devtools::target::RendererDevToolsTargetHandle,
        RendererDevToolsAgentToken,
        DevToolsSessionKey,
    )>,
}

#[cfg(feature = "devtools")]
impl RendererRuntimeInspectorSessionDetachGuard {
    fn new(
        inspector_pause_bridge: crate::devtools::pause::RendererInspectorPauseBridge,
        devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) -> Self {
        inspector_pause_bridge.arm_session_detach();
        devtools_target.begin_session_detach(agent_token, &session);
        Self {
            inspector_pause_bridge: Some(inspector_pause_bridge),
            pending_detach: Some((devtools_target, agent_token, session)),
        }
    }

    pub(crate) fn belongs_to_agent(&self, agent: RendererDevToolsAgentToken) -> bool {
        self.pending_detach
            .as_ref()
            .is_some_and(|(_, expected, _)| *expected == agent)
    }

    pub(crate) fn complete(&mut self) {
        if let Some((target, agent_token, session)) = self.pending_detach.take() {
            target.finish_session_detach(agent_token, &session);
        }
    }
}

#[cfg(feature = "devtools")]
impl Drop for RendererRuntimeInspectorSessionDetachGuard {
    fn drop(&mut self) {
        if let Some((target, _agent_token, _session)) = self.pending_detach.take() {
            // Reopening the ingress after owner-side V8 cleanup failed would
            // let a replacement attachment reuse the stale session. Close the
            // target so queued replacement commands fail instead of hanging or
            // inheriting the retired frontend's state.
            target.close("Inspector session detach did not reach the renderer owner");
        }
        if let Some(inspector_pause_bridge) = self.inspector_pause_bridge.take() {
            inspector_pause_bridge.disarm_session_detach();
        }
    }
}

/// Inspector admission and retirement for one exact native agent, including
/// the interval before its Page handle is adopted. It carries no DOM access.
#[cfg(feature = "devtools")]
#[derive(Clone)]
pub struct RendererPageInspectorEndpoint {
    runtime: RenderRuntimeHandle,
    token: RendererPageToken,
    agent: RendererDevToolsAgentToken,
    target: crate::devtools::target::RendererDevToolsTargetHandle,
}

#[cfg(feature = "devtools")]
impl std::fmt::Debug for RendererPageInspectorEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPageInspectorEndpoint")
            .field("agent", &self.agent)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "devtools")]
impl RendererPageInspectorEndpoint {
    pub(crate) fn new(
        runtime: RenderRuntimeHandle,
        token: RendererPageToken,
        agent: RendererDevToolsAgentToken,
        target: crate::devtools::target::RendererDevToolsTargetHandle,
    ) -> Self {
        Self {
            runtime,
            token,
            agent,
            target,
        }
    }

    pub fn agent(&self) -> RendererDevToolsAgentToken {
        self.agent
    }

    pub fn has_session(&self, id: Option<&str>) -> bool {
        self.target.io_ref().session_connections().contains(
            &crate::devtools::ingress::lane::RendererDevToolsSessionLaneKey::new(
                self.agent,
                DevToolsSessionKey::from_wire_session_id(id),
            ),
        )
    }

    #[cfg(feature = "devtools")]
    pub fn connect(
        &self,
        inspector_session_id: Option<String>,
        session_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        // Prepared-page installation may not have passed through an ordinary
        // Main command yet. The connection itself owns its execution wake.
        self.target
            .main_ref()
            .configure_owner_wake(self.runtime.clone());
        if let Some(connection) = self.target.io_ref().session_connections().begin(
            self.agent,
            DevToolsSessionKey::from_wire_session_id(inspector_session_id.as_deref()),
            self.target.main_ref().clone(),
            self.target.io_ref().clone(),
        ) {
            // Both operations are target-owned. Connect releases IO locally;
            // configuration retains a separate position before later Main work.
            self.target.main_ref().enqueue_session_initialization(
                self.token,
                self.agent,
                RendererPageCommand::DevTools(
                    RendererDevToolsPageCommand::ConnectRuntimeInspectorSession {
                        inspector_session_id: inspector_session_id.clone(),
                        connection,
                    },
                ),
                inspector_session_id.clone(),
            );
        }
        self.target.main_ref().enqueue_session_initialization(
            self.token,
            self.agent,
            RendererPageCommand::DevTools(
                RendererDevToolsPageCommand::SetSessionRuntimeBindingState {
                    agent: self.agent,
                    inspector_session_id: inspector_session_id.clone(),
                    session_runtime_bindings: session_runtime_bindings.to_vec(),
                },
            ),
            inspector_session_id,
        );
    }

    #[cfg(feature = "devtools")]
    pub fn enqueue_runtime_inspector_io_command(
        &self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        self.target.io_ref().enqueue_command(
            self.agent,
            RendererDevToolsIoCommandEnvelope::inspector(envelope),
        )
    }

    #[cfg(feature = "devtools")]
    pub fn prepare_runtime_inspector_session_retirement(
        &self,
        inspector_session_id: Option<String>,
    ) -> RendererRuntimeInspectorSessionRetirement {
        let session = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.as_deref().filter(|id| !id.is_empty()),
        );
        let guard = RendererRuntimeInspectorSessionDetachGuard::new(
            self.target.pause(),
            self.target.clone(),
            self.agent,
            session,
        );
        RendererRuntimeInspectorSessionRetirement {
            runtime: self.runtime.clone(),
            token: self.token,
            inspector_session_id,
            guard: Some(guard),
        }
    }
}

/// Original native cleanup authority, independent of the revoked Browser route.
#[cfg(feature = "devtools")]
pub struct RendererRuntimeInspectorSessionRetirement {
    runtime: RenderRuntimeHandle,
    token: RendererPageToken,
    inspector_session_id: Option<String>,
    guard: Option<RendererRuntimeInspectorSessionDetachGuard>,
}

#[cfg(feature = "devtools")]
impl RendererRuntimeInspectorSessionRetirement {
    pub fn finish(mut self) -> Result<()> {
        self.enqueue_finalizer()
    }

    fn enqueue_finalizer(&mut self) -> Result<()> {
        if let Some(pause_guard) = self.guard.take() {
            self.runtime.dispatch_detached(
                RendererOwnerCommand::FinalizeRuntimeInspectorSessionDetach {
                    token: self.token,
                    inspector_session_id: self.inspector_session_id.take(),
                    pause_guard,
                },
            )?;
        }
        Ok(())
    }
}

#[cfg(feature = "devtools")]
impl Drop for RendererRuntimeInspectorSessionRetirement {
    fn drop(&mut self) {
        let _ = self.enqueue_finalizer();
    }
}

impl RendererPageHandle {
    fn token(&self) -> RendererPageToken {
        self.token
            .expect("renderer page handle should remain open while in use")
    }

    #[cfg(feature = "devtools")]
    pub fn protocol_command_target(&self) -> RendererPageProtocolCommandTarget {
        RendererPageProtocolCommandTarget {
            token: self.token(),
            agent: self.devtools_agent_token,
            target: self.devtools_target.clone(),
            dialogs: self.javascript_dialog_broker.clone(),
        }
    }

    pub fn document_command_target(
        &self,
        document: RendererDocumentLifecycleIdentity,
    ) -> Result<RendererDocumentCommandTarget> {
        if self.token().page_id != document.frame.page_id {
            return Err(super::page_commands::DocumentCommandError::WrongPage.into());
        }
        Ok(RendererDocumentCommandTarget {
            render_runtime: self.render_runtime.clone(),
            token: self.token(),
            document,
        })
    }

    #[cfg(feature = "molio")]
    pub fn enqueue_viewport_wheel(&self, input: super::RendererViewportWheelInput) -> Result<()> {
        self.render_runtime
            .enqueue_viewport_wheel(self.token(), input)
    }

    #[cfg(feature = "molio")]
    pub async fn viewport_display_tick(
        &self,
        viewport_revision: u64,
        time: std::time::Instant,
    ) -> Result<()> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::ViewportDisplayTick {
                token: self.token(),
                viewport_revision,
                time,
            })
            .await?
        {
            RendererOwnerReply::ViewportDisplayTickProcessed => Ok(()),
            _ => anyhow::bail!("unexpected viewport display tick reply"),
        }
    }

    #[cfg(feature = "molio")]
    pub async fn subscribe_navigation_state(
        &self,
    ) -> Result<tokio::sync::watch::Receiver<Option<super::RendererPageNavigationState>>> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::SubscribePageNavigation {
                token: self.token(),
            })
            .await?
        {
            RendererOwnerReply::PageNavigationSubscription(updates) => Ok(updates),
            _ => anyhow::bail!("unexpected page navigation subscription reply"),
        }
    }

    pub fn page_id(&self) -> u64 {
        self.token().page_id.as_u64()
    }

    pub fn renderer_page_id(&self) -> super::PageId {
        self.token().page_id
    }

    pub fn owner_local_host_id(&self) -> super::RendererOwnerLocalHostId {
        self.token().local_host_id
    }

    pub fn devtools_agent_token(&self) -> RendererDevToolsAgentToken {
        self.devtools_agent_token
    }

    /// Applies the terminal `Page.crash` IO control without entering either
    /// ordinary DevTools command lane.
    #[doc(hidden)]
    #[cfg(feature = "devtools")]
    pub fn crash_devtools_target_from_io(&self) {
        self.devtools_target.crash_from_io();
    }

    #[doc(hidden)]
    pub fn take_committed_document_post_response_continuation(
        &mut self,
    ) -> Option<RendererPageCommandPostResponseContinuation> {
        self.committed_document_post_response_continuation.take()
    }

    pub fn adopt_document_replacement(
        &mut self,
        replacement: RendererPageReplacementCommit,
    ) -> Result<(
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
    )> {
        anyhow::ensure!(
            self.token() == replacement.token,
            "replacement Document belongs to a different Page"
        );
        let stable = replacement.slot.entry();
        anyhow::ensure!(
            stable.is_active() && stable.vm_creation_id() == replacement.vm_creation_id,
            "replacement Document was superseded before its handle was adopted"
        );
        // The old Document detached its Inspector bindings at commit. Detaching
        // by Page ID here would instead detach the new Document of the same Page.
        self.devtools_agent_token = replacement.devtools_agent_token;
        self.javascript_dialog_broker = replacement.javascript_dialog_broker;
        self.devtools_target = replacement.devtools_target;
        self.committed_document_post_response_continuation =
            replacement.committed_document_post_response_continuation;
        Ok((
            replacement.page_state,
            replacement.creation_diagnostics,
            replacement.creation_artifacts,
        ))
    }

    pub fn take_pending_modal_javascript_dialogs(&self) -> Vec<RendererPendingJavaScriptDialog> {
        self.javascript_dialog_broker.take_pending()
    }

    #[cfg(feature = "devtools")]
    pub fn has_runtime_inspector_session_endpoint(&self, id: Option<&str>) -> bool {
        self.inspector_endpoint().has_session(id)
    }

    #[cfg(feature = "devtools")]
    pub fn inspector_endpoint(&self) -> RendererPageInspectorEndpoint {
        RendererPageInspectorEndpoint::new(
            self.render_runtime.clone(),
            self.token(),
            self.devtools_agent_token,
            self.devtools_target.clone(),
        )
    }

    #[cfg(feature = "devtools")]
    pub fn connect_runtime_inspector_session(
        &self,
        id: Option<String>,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.inspector_endpoint().connect(id, bindings);
    }

    #[cfg(feature = "devtools")]
    pub fn enqueue_runtime_inspector_io_command(
        &self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        self.inspector_endpoint()
            .enqueue_runtime_inspector_io_command(envelope)
    }

    #[doc(hidden)]
    #[cfg(feature = "devtools")]
    pub fn enqueue_performance_get_metrics_io_command(
        &self,
        attachment: Option<RendererAgentAttachmentId>,
        inspector_session_id: Option<String>,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        self.devtools_target.io_ref().enqueue_command(
            self.devtools_agent_token,
            RendererDevToolsIoCommandEnvelope::performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    attachment,
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
            ),
        )
    }

    #[doc(hidden)]
    #[cfg(feature = "devtools")]
    pub fn enqueue_performance_get_metrics_io_command_with_response(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
        result: serde_json::Value,
        response: RendererRuntimeInspectorResponseSender,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        debug_assert_eq!(
            response.renderer_agent_attachment_id(),
            Some(attachment),
            "Performance response must belong to the command attachment"
        );
        self.devtools_target.io_ref().enqueue_command(
            self.devtools_agent_token,
            RendererDevToolsIoCommandEnvelope::performance_get_metrics_with_response(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                result,
                response,
            ),
        )
    }

    #[cfg(feature = "devtools")]
    pub fn enqueue_runtime_inspector_main_command(
        &self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> RendererRuntimeInspectorMainCommandRoute {
        self.devtools_target.main_ref().enqueue_command(
            self.token(),
            self.devtools_agent_token,
            envelope,
        )
    }

    pub fn runtime_inspector_pause_active(&self) -> bool {
        self.devtools_target.pause_ref().is_pause_active()
    }

    /// Reserves a Document against this Page's current committed generation.
    /// This does not detach the current realm or transfer Page close authority.
    #[cfg(test)]
    pub(crate) async fn reserve_replacement_document_for_navigation(
        &self,
    ) -> Result<RendererPageReservationToken> {
        let target = self.document_replacement_target(moli_fetch::FetchCancelHandle::new());
        let scope = target.replacement.clone().begin()?;
        target.reserve(scope).await
    }

    /// Captures a non-owning Page target without entering the renderer. Pass
    /// this to response preparation only after downloads and cancellations
    /// have been classified; preparation owns the old Document's pause wake.
    pub fn document_replacement_target(
        &self,
        cancellation: moli_fetch::FetchCancelHandle,
    ) -> super::RendererPageReplacementTarget {
        super::RendererPageReplacementTarget {
            render_runtime: self.render_runtime.clone(),
            token: self.token(),
            replacement: super::RendererDocumentReplacement::new(
                self.devtools_target.pause(),
                cancellation,
            ),
        }
    }

    /// Enqueues the DevTools IO-agent script policy without borrowing the
    /// owner-resident `PageVm` that may currently be executing JavaScript.
    #[doc(hidden)]
    #[cfg(feature = "devtools")]
    pub fn enqueue_set_script_execution_disabled_io_command(
        &self,
        attachment: Option<RendererAgentAttachmentId>,
        inspector_session_id: Option<String>,
        disabled: bool,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        self.devtools_target.io_ref().enqueue_command(
            self.devtools_agent_token,
            RendererDevToolsIoCommandEnvelope::set_script_execution_disabled(
                RendererInspectorIngressTicket::new(
                    attachment,
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                self.script_execution_control.clone(),
                disabled,
            ),
        )
    }

    #[doc(hidden)]
    #[cfg(feature = "devtools")]
    pub fn enqueue_set_script_execution_disabled_io_command_with_response(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
        disabled: bool,
        response: RendererRuntimeInspectorResponseSender,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        debug_assert_eq!(
            response.renderer_agent_attachment_id(),
            Some(attachment),
            "Emulation response must belong to the command attachment"
        );
        self.devtools_target.io_ref().enqueue_command(
            self.devtools_agent_token,
            RendererDevToolsIoCommandEnvelope::set_script_execution_disabled_with_response(
                RendererInspectorIngressTicket::new(
                    Some(attachment),
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                self.script_execution_control.clone(),
                disabled,
                response,
            ),
        )
    }

    /// Disconnects one frontend Inspector route without waiting for the Page
    /// owner to return from JavaScript.
    ///
    /// Chromium acknowledges `Target.detachFromTarget` after dropping the
    /// browser-side DevToolsSession pipes; destruction of the renderer-side
    /// V8InspectorSession is a subsequent Main-thread task. Mirror that
    /// boundary here: cancel both ingress lanes synchronously, then enqueue an
    /// owner-only cleanup whose reply is deliberately detached. The retained
    /// guard also releases a nested debugger loop and holds any replacement
    /// attachment's commands until cleanup reaches the owner.
    #[cfg(feature = "devtools")]
    pub fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<()> {
        self.prepare_runtime_inspector_session_retirement(inspector_session_id)
            .finish()
    }

    #[cfg(feature = "devtools")]
    pub fn prepare_runtime_inspector_session_retirement(
        &self,
        id: Option<String>,
    ) -> RendererRuntimeInspectorSessionRetirement {
        self.inspector_endpoint()
            .prepare_runtime_inspector_session_retirement(id)
    }

    pub fn enqueue_async_command(
        &self,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageCommandPending> {
        self.enqueue_async_command_with_capture_policy(
            command,
            RendererPageStateCapturePolicy::FullReport,
            false,
            None,
        )
    }

    /// Enqueues a protocol-owned command without eagerly rebuilding the
    /// testing/CLI globals report at the response boundary.
    ///
    /// The command still commits URL, title, observable/network output and the
    /// renderer publication fence. Its returned report explicitly marks the
    /// last globals snapshot dirty until a full page-state capture is
    /// requested.
    pub fn enqueue_protocol_command(
        &self,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageCommandPending> {
        self.enqueue_async_command_with_capture_policy(
            command,
            RendererPageStateCapturePolicy::ProtocolTurn,
            false,
            None,
        )
    }

    #[doc(hidden)]
    pub fn enqueue_protocol_command_in_inspector_session(
        &self,
        command: RendererPageCommand,
        inspector_session_id: Option<String>,
    ) -> anyhow::Result<RendererPageCommandPending> {
        self.enqueue_async_command_with_capture_policy(
            command,
            RendererPageStateCapturePolicy::ProtocolTurn,
            true,
            inspector_session_id,
        )
    }

    fn enqueue_async_command_with_capture_policy(
        &self,
        command: RendererPageCommand,
        capture_policy: RendererPageStateCapturePolicy,
        route_protocol_main_receiver: bool,
        inspector_session_id: Option<String>,
    ) -> anyhow::Result<RendererPageCommandPending> {
        #[cfg(not(feature = "devtools"))]
        let _ = (route_protocol_main_receiver, inspector_session_id);
        let javascript_dialog_watch = renderer_page_javascript_dialog_watch(&command, || {
            self.javascript_dialog_broker.watch()
        });
        if moli_trace::cdp_nav_timing_enabled()
            && let Some(command_label) = command.cdp_nav_timing_label()
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                page_id = self.page_id(),
                stage = "page_handle_command_dispatch",
            );
        }
        #[cfg(feature = "devtools")]
        if route_protocol_main_receiver {
            let route = self
                .devtools_target
                .main_ref()
                .enqueue_protocol_page_command(
                    self.token(),
                    self.devtools_agent_token,
                    command,
                    inspector_session_id,
                    capture_policy,
                );
            let cleanup_control = Some(route.cleanup_control());
            return Ok(RendererPageCommandPending {
                cleanup_control,
                dispatch: RendererPageCommandPendingDispatch::InspectorMain(Box::pin(
                    route.wait_for_completion(),
                )),
                javascript_dialog_watch,
            });
        }
        let command = match command {
            #[cfg(feature = "devtools")]
            RendererPageCommand::DevTools(RendererDevToolsPageCommand::Inspector(envelope)) => {
                let route = self.devtools_target.main_ref().enqueue_owner_command(
                    self.token(),
                    self.devtools_agent_token,
                    envelope,
                    capture_policy,
                );
                let cleanup_control = Some(route.cleanup_control());
                return Ok(RendererPageCommandPending {
                    cleanup_control,
                    dispatch: RendererPageCommandPendingDispatch::InspectorMain(Box::pin(
                        route.wait_for_completion(),
                    )),
                    javascript_dialog_watch,
                });
            }
            command => command,
        };
        let owner_command = match capture_policy {
            RendererPageStateCapturePolicy::FullReport => {
                RendererOwnerCommand::RunAsyncPageCommand {
                    token: self.token(),
                    command,
                }
            }
            RendererPageStateCapturePolicy::ProtocolTurn => {
                RendererOwnerCommand::RunProtocolPageCommand {
                    token: self.token(),
                    command,
                }
            }
            RendererPageStateCapturePolicy::BackgroundInitialization => unreachable!(
                "unobserved session initialization must use its target-owned Main receiver"
            ),
        };
        let reply_rx = self.render_runtime.enqueue(owner_command)?;
        Ok(RendererPageCommandPending {
            dispatch: RendererPageCommandPendingDispatch::Owner(reply_rx),
            javascript_dialog_watch,
            #[cfg(feature = "devtools")]
            cleanup_control: None,
        })
    }

    pub async fn run_async_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<(RendererPageReply, Arc<RendererPageState>)> {
        Ok(self
            .enqueue_async_command(command)?
            .wait()
            .await?
            .into_reply_and_state())
    }

    pub async fn wait_for_network_idle(
        &self,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    ) -> Result<(RendererPageReply, Arc<RendererPageState>)> {
        self.wait_for_idle_with_mode(crate::RendererPageIdleWaitMode::Network, timeout_ms, loader)
            .await
    }

    pub async fn wait_for_idle(
        &self,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    ) -> Result<(RendererPageReply, Arc<RendererPageState>)> {
        self.wait_for_idle_with_mode(
            crate::RendererPageIdleWaitMode::EventLoop,
            timeout_ms,
            loader,
        )
        .await
    }

    async fn wait_for_idle_with_mode(
        &self,
        mode: crate::RendererPageIdleWaitMode,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    ) -> Result<(RendererPageReply, Arc<RendererPageState>)> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::WaitForIdle {
                token: self.token(),
                mode,
                timeout_ms,
                loader,
            })
            .await?
        {
            RendererOwnerReply::AsyncPageCommandRan(result) => Ok(result.into_reply_and_state()),
            _ => Err(anyhow!(
                "renderer owner returned non-async page-command reply for wait-for-{}",
                PageVmIdleWaitState::new(mode).label()
            )),
        }
    }

    pub async fn wait_for_dom_stable(
        &self,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    ) -> Result<(RendererPageReply, Arc<RendererPageState>)> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::WaitForDomStable {
                token: self.token(),
                timeout_ms,
                loader,
            })
            .await?
        {
            RendererOwnerReply::AsyncPageCommandRan(result) => Ok(result.into_reply_and_state()),
            _ => Err(anyhow!(
                "renderer owner returned non-async page-command reply for wait-for-dom-stable"
            )),
        }
    }

    pub async fn close_async(&mut self) -> Result<()> {
        let Some(token) = self.token else {
            return Ok(());
        };
        let requested_execution_interrupt = self
            .devtools_target
            .close("Inspector target closed with its Page handle");
        self.javascript_dialog_broker.dismiss_pending();
        self.page_context_cancel_tx
            .cancel(RendererPageContextCancelReason::PageClosed);
        tracing::debug!(
            page_id = token.page_id.as_u64(),
            requested_execution_interrupt,
            "closing renderer page handle"
        );

        if is_on_named_owner_execution_lane_for(&self.local_executor)
            && has_current_render_runtime_owner_local_store()
        {
            remove_page(token);
            self.token = None;
            tracing::debug!("renderer page handle closed on owner lane");
            return Ok(());
        }

        // Keep the token installed until the owner acknowledges removal. If
        // this future is cancelled, Drop can still enqueue detached cleanup.
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::RemovePage { token })
            .await?
        {
            RendererOwnerReply::PageRemoved => {
                self.token = None;
                tracing::debug!("renderer page handle closed through owner command");
                Ok(())
            }
            _ => Err(anyhow!(
                "renderer owner returned non-remove-page reply for remove page command"
            )),
        }
    }
}

/// Observation commands may fail fast when a modal prevents their execution.
fn renderer_page_javascript_dialog_watch(
    command: &RendererPageCommand,
    watch: impl FnOnce() -> RendererJavaScriptDialogWatch,
) -> Option<RendererJavaScriptDialogWatch> {
    command.interruptible_by_javascript_dialog().then(watch)
}

fn decode_owner_page_command_reply(
    reply: Result<Result<RendererOwnerReply>, oneshot::error::RecvError>,
) -> Result<RendererCommandTurnOutput> {
    match reply.map_err(|_| anyhow!("render runtime reply channel closed"))?? {
        RendererOwnerReply::AsyncPageCommandRan(result) => Ok(*result),
        _ => Err(anyhow!(
            "renderer owner returned non-async page-command reply for async page command"
        )),
    }
}

#[cfg(feature = "devtools")]
fn decode_inspector_main_page_command_completion(
    completion: Result<RendererRuntimeInspectorMainCommandCompletion>,
) -> Result<RendererCommandTurnOutput> {
    match completion? {
        RendererRuntimeInspectorMainCommandCompletion::Owner(output) => Ok(*output),
        RendererRuntimeInspectorMainCommandCompletion::Page(output) => Ok(*output),
        RendererRuntimeInspectorMainCommandCompletion::Inspector
        | RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse { .. } => Err(
            anyhow!("an owner-only Inspector Main command entered nested dispatch"),
        ),
        RendererRuntimeInspectorMainCommandCompletion::OwnerSessionResponse { .. }
        | RendererRuntimeInspectorMainCommandCompletion::OwnerSessionErrorSettled(_) => Err(
            anyhow!("an owner-only Page command settled a DevTools session response"),
        ),
        RendererRuntimeInspectorMainCommandCompletion::Canceled(message) => Err(anyhow!(message)),
    }
}

impl RendererPageCommandPending {
    pub async fn wait(self) -> Result<RendererCommandTurnOutput> {
        let RendererPageCommandPending {
            dispatch,
            javascript_dialog_watch,
            ..
        } = self;
        let wait_for_dispatch = async move {
            match dispatch {
                RendererPageCommandPendingDispatch::Owner(reply_rx) => {
                    decode_owner_page_command_reply(reply_rx.await)
                }
                #[cfg(feature = "devtools")]
                RendererPageCommandPendingDispatch::InspectorMain(route) => {
                    decode_inspector_main_page_command_completion(route.await)
                }
            }
        };
        tokio::pin!(wait_for_dispatch);
        let reply = match javascript_dialog_watch {
            Some(javascript_dialog_watch) => {
                tokio::select! {
                    biased;
                    reply = &mut wait_for_dispatch => reply?,
                    () = javascript_dialog_watch.wait_until_open() => {
                        return Err(anyhow!(
                            "renderer page observation interrupted by an open JavaScript dialog"
                        ));
                    }
                }
            }
            None => wait_for_dispatch.await?,
        };
        #[cfg(feature = "devtools")]
        let reply = reply.finish_screencast_raster().await?;
        Ok(reply)
    }
}

impl Drop for RendererPageHandle {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        self.devtools_target
            .detach_page(token.page_id, "Inspector Page handle was dropped");
        self.javascript_dialog_broker.dismiss_pending();
        self.page_context_cancel_tx
            .cancel(RendererPageContextCancelReason::PageClosed);

        if is_on_named_owner_execution_lane_for(&self.local_executor)
            && has_current_render_runtime_owner_local_store()
        {
            tracing::debug!(
                page_id = token.page_id.as_u64(),
                "dropping renderer page handle on owner lane"
            );
            remove_page(token);
            return;
        }

        // Drop is a best-effort cleanup path. It must not block a caller on the
        // renderer owner; code that needs teardown acknowledgement should call
        // `close_async()` before dropping the page handle.
        tracing::debug!(
            page_id = token.page_id.as_u64(),
            "dropping renderer page handle; enqueueing detached remove-page command"
        );
        let _ = self
            .render_runtime
            .dispatch_detached(RendererOwnerCommand::RemovePage { token });
    }
}

impl RendererPageTestingHandle {
    pub fn new_for_testing(handle: &RendererPageHandle) -> Self {
        Self {
            render_runtime: handle.render_runtime.clone(),
            token: handle.token(),
            _not_send: PhantomData,
        }
    }

    pub fn shares_local_host(&self, other: &Self) -> bool {
        self.token.local_host_id == other.token.local_host_id
    }

    pub async fn current_page_state_async(&self) -> Result<Arc<RendererPageState>> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::TestingCurrentPageState { token: self.token })
            .await?
        {
            RendererOwnerReply::TestingCurrentPageState(page_state) => Ok(page_state),
            other => Err(anyhow!(
                "renderer owner returned non-page-state testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub async fn renderer_page_view_async(&self) -> Result<RendererPageView> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::TestingRendererPageView { token: self.token })
            .await?
        {
            RendererOwnerReply::TestingRendererPageView(view) => Ok(view),
            other => Err(anyhow!(
                "renderer owner returned non-page-view testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub async fn owner_slot_async(&self) -> Result<RendererPageSlotHandle> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::TestingOwnerSlot { token: self.token })
            .await?
        {
            RendererOwnerReply::TestingOwnerSlot(slot) => Ok(slot),
            other => Err(anyhow!(
                "renderer owner returned non-slot testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub async fn host_instance_key_async(&self) -> Result<usize> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::TestingHostInstanceKey { token: self.token })
            .await?
        {
            RendererOwnerReply::TestingHostInstanceKey(key) => Ok(key),
            other => Err(anyhow!(
                "renderer owner returned non-host-key testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub async fn host_unique_document_isolate_count_async(&self) -> Result<usize> {
        match self
            .render_runtime
            .dispatch(
                RendererOwnerCommand::TestingHostUniqueDocumentIsolateCount { token: self.token },
            )
            .await?
        {
            RendererOwnerReply::TestingHostUniqueDocumentIsolateCount(count) => Ok(count),
            other => Err(anyhow!(
                "renderer owner returned non-host-unique-document-isolate-count testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    #[cfg(test)]
    pub(crate) async fn realtime_layout_observation_async(
        &self,
    ) -> Result<RendererRealtimeLayoutObservation> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::TestingRealtimeLayoutObservation { token: self.token })
            .await?
        {
            RendererOwnerReply::TestingRealtimeLayoutObservation(observation) => Ok(*observation),
            other => Err(anyhow!(
                "renderer owner returned a different testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    #[cfg(test)]
    pub async fn deferred_page_vm_drop_pending_count_async(&self) -> Result<usize> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::TestingDeferredPageVmDropPendingCount)
            .await?
        {
            RendererOwnerReply::TestingDeferredPageVmDropPendingCount(count) => Ok(count),
            other => Err(anyhow!(
                "renderer owner returned non-deferred-page-vm-drop-pending-count testing reply: {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }
}
