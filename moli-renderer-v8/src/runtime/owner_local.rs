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
        Ok(RendererPageCommandPending {
            dispatch: RendererPageCommandPendingDispatch::InspectorMain(Box::new(route)),
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

/// How a queued dispatch observes JavaScript dialogs.
#[derive(Debug)]
enum RendererPageJavaScriptDialogWatch {
    /// Observation commands abandon their queued dispatch when a dialog opens:
    /// a plain [`RendererPageCommandPending::wait`] fail-fasts on the interrupt
    /// error instead of parking behind the modal.
    Interruptible(RendererJavaScriptDialogWatch),
    /// Deferred-settle commands stay queued behind the modal. The watch exists
    /// so [`RendererPageCommandPending::wait_until_settled_or_dialog_open`] can
    /// hand the still-queued dispatch back and let a caller settle it once the
    /// renderer frees up.
    Deferred(RendererJavaScriptDialogWatch),
}

pub struct RendererPageCommandPending {
    dispatch: RendererPageCommandPendingDispatch,
    javascript_dialog_watch: Option<RendererPageJavaScriptDialogWatch>,
}

impl RendererPageCommandPending {
    pub(super) fn from_owner_reply(reply: oneshot::Receiver<Result<RendererOwnerReply>>) -> Self {
        Self {
            dispatch: RendererPageCommandPendingDispatch::Owner(reply),
            javascript_dialog_watch: None,
        }
    }
}

enum RendererPageCommandPendingDispatch {
    Owner(oneshot::Receiver<anyhow::Result<RendererOwnerReply>>),
    #[cfg(feature = "devtools")]
    InspectorMain(Box<RendererRuntimeInspectorMainCommandRoute>),
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
    pub fn enqueue_runtime_inspector_io_command(
        &self,
        envelope: RendererInspectorCommandEnvelope,
    ) -> RendererRuntimeInspectorIoCommandRoute {
        self.devtools_target.io_ref().enqueue_command(
            self.devtools_agent_token,
            RendererDevToolsIoCommandEnvelope::inspector(envelope),
        )
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
    ) -> anyhow::Result<()> {
        let session = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id
                .as_deref()
                .filter(|session_id| !session_id.is_empty()),
        );
        let pause_guard = RendererRuntimeInspectorSessionDetachGuard::new(
            self.devtools_target.pause(),
            self.devtools_target.clone(),
            self.devtools_agent_token,
            session,
        );
        let reply_rx = self.render_runtime.enqueue(
            RendererOwnerCommand::FinalizeRuntimeInspectorSessionDetach {
                token: self.token(),
                inspector_session_id,
                pause_guard,
            },
        )?;
        drop(reply_rx);
        Ok(())
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
            return Ok(RendererPageCommandPending {
                dispatch: RendererPageCommandPendingDispatch::InspectorMain(Box::new(route)),
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
                return Ok(RendererPageCommandPending {
                    dispatch: RendererPageCommandPendingDispatch::InspectorMain(Box::new(route)),
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
        };
        let reply_rx = self.render_runtime.enqueue(owner_command)?;
        Ok(RendererPageCommandPending {
            dispatch: RendererPageCommandPendingDispatch::Owner(reply_rx),
            javascript_dialog_watch,
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

/// Creates the dialog watch a queued dispatch carries, if any.
///
/// Interruptible observation commands fail fast on dialog observation.
/// Deferred-settle commands keep their dispatch queued behind the modal and
/// are only handed back through
/// [`RendererPageCommandPending::wait_until_settled_or_dialog_open`].
fn renderer_page_javascript_dialog_watch(
    command: &RendererPageCommand,
    watch: impl FnOnce() -> RendererJavaScriptDialogWatch,
) -> Option<RendererPageJavaScriptDialogWatch> {
    if command.interruptible_by_javascript_dialog() {
        return Some(RendererPageJavaScriptDialogWatch::Interruptible(watch()));
    }
    command
        .settles_after_javascript_dialog()
        .then(|| RendererPageJavaScriptDialogWatch::Deferred(watch()))
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

fn decode_inspector_main_page_command_completion(
    completion: anyhow::Result<RendererRuntimeInspectorMainCommandCompletion>,
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
        } = self;
        let wait_for_dispatch = async move {
            match dispatch {
                RendererPageCommandPendingDispatch::Owner(reply_rx) => {
                    decode_owner_page_command_reply(reply_rx.await)
                }
                #[cfg(feature = "devtools")]
                RendererPageCommandPendingDispatch::InspectorMain(route) => {
                    decode_inspector_main_page_command_completion(route.wait_for_completion().await)
                }
            }
        };
        tokio::pin!(wait_for_dispatch);
        let reply = match javascript_dialog_watch {
            Some(RendererPageJavaScriptDialogWatch::Interruptible(javascript_dialog_watch)) => {
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
            // Deferred-settle watches are only raced by
            // `wait_until_settled_or_dialog_open`; a plain wait keeps parking
            // behind the modal until the dispatch settles.
            Some(RendererPageJavaScriptDialogWatch::Deferred(_)) | None => {
                wait_for_dispatch.await?
            }
        };
        Ok(reply)
    }

    /// Settles the dispatch, or hands the still-queued dispatch back as soon as
    /// a JavaScript dialog opens.
    ///
    /// Deferred-settle commands (see
    /// `RendererPageCommand::settles_after_javascript_dialog`) stay queued
    /// behind an open modal: this consumes the dialog watch and returns the
    /// pending with the dispatch intact. Dropping that pending cancels the
    /// queued command, so the caller must keep it alive and settle it with
    /// [`RendererPageCommandPending::wait`] once the renderer frees up. The
    /// `Err` arm is observed with biased dispatch priority: a dispatch that is
    /// already complete always settles here instead of being handed back.
    pub async fn wait_until_settled_or_dialog_open(
        self,
    ) -> Result<Result<RendererCommandTurnOutput>, Self> {
        let Self {
            dispatch,
            javascript_dialog_watch,
        } = self;
        let Some(RendererPageJavaScriptDialogWatch::Deferred(watch)) = javascript_dialog_watch
        else {
            // Interruptible commands fail fast inside `wait`, and commands
            // without a dialog watch cannot park behind a modal at all.
            return Ok(Self {
                dispatch,
                javascript_dialog_watch: None,
            }
            .wait()
            .await);
        };
        match dispatch {
            RendererPageCommandPendingDispatch::Owner(reply_rx) => {
                enum OwnerSettle {
                    Reply(Result<Result<RendererOwnerReply>, oneshot::error::RecvError>),
                    DialogOpened,
                }
                let mut reply_rx = reply_rx;
                let raced = {
                    tokio::select! {
                        biased;
                        reply = &mut reply_rx => OwnerSettle::Reply(reply),
                        () = watch.wait_until_open() => OwnerSettle::DialogOpened,
                    }
                };
                match raced {
                    OwnerSettle::Reply(reply) => Ok(decode_owner_page_command_reply(reply)),
                    OwnerSettle::DialogOpened => Err(Self {
                        dispatch: RendererPageCommandPendingDispatch::Owner(reply_rx),
                        javascript_dialog_watch: None,
                    }),
                }
            }
            RendererPageCommandPendingDispatch::InspectorMain(route) => {
                enum MainSettle {
                    Completion(anyhow::Result<RendererRuntimeInspectorMainCommandCompletion>),
                    DialogOpened,
                }
                let mut route = route;
                let raced = {
                    let completion = route.wait_for_completion_by_ref();
                    tokio::pin!(completion);
                    tokio::select! {
                        biased;
                        completion = &mut completion => MainSettle::Completion(completion),
                        () = watch.wait_until_open() => MainSettle::DialogOpened,
                    }
                };
                match raced {
                    MainSettle::Completion(completion) => {
                        Ok(decode_inspector_main_page_command_completion(completion))
                    }
                    MainSettle::DialogOpened => Err(Self {
                        dispatch: RendererPageCommandPendingDispatch::InspectorMain(route),
                        javascript_dialog_watch: None,
                    }),
                }
            }
        }
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

#[cfg(test)]
mod deferred_settle_tests {
    use super::*;
    use crate::runtime::javascript_dialog::{
        RendererJavaScriptDialogRuntime, RendererModalJavaScriptDialog,
    };

    const SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

    fn pending_dialog(message: &str) -> RendererPendingJavaScriptDialog {
        let page_id = crate::runtime::PageId::new_for_testing(91);
        RendererPendingJavaScriptDialog::new(
            crate::runtime::RendererJavaScriptDialogId::new(1),
            crate::runtime::RendererDocumentLifecycleIdentity {
                frame: crate::runtime::RendererFrameToken { page_id },
                document: crate::runtime::RendererDocumentToken::new_for_testing(page_id, 1),
                epoch: crate::runtime::RendererLifecycleEpoch(1),
            },
            crate::runtime::RendererJavaScriptDialogSource::RootFrame,
            "https://example.test/dialog".to_owned(),
            "confirm".to_owned(),
            message.to_owned(),
            String::new(),
            // `begin_modal` installs the modal completion itself.
            None,
        )
    }

    /// A broker with an open modal, its runtime (the only `begin_modal`
    /// authority), and the modal handle used to close it at test end.
    fn broker_with_open_modal(
        message: &str,
    ) -> (
        RendererJavaScriptDialogRuntime,
        RendererModalJavaScriptDialog,
    ) {
        let runtime = RendererJavaScriptDialogRuntime::default();
        let (_dialog, modal) = runtime.begin_modal(pending_dialog(message));
        (runtime, modal)
    }

    fn deferred_settle_pending(
        reply_rx: oneshot::Receiver<anyhow::Result<RendererOwnerReply>>,
        broker: &RendererJavaScriptDialogBroker,
    ) -> RendererPageCommandPending {
        RendererPageCommandPending {
            dispatch: RendererPageCommandPendingDispatch::Owner(reply_rx),
            javascript_dialog_watch: Some(RendererPageJavaScriptDialogWatch::Deferred(
                broker.watch(),
            )),
        }
    }

    fn test_reply_error() -> anyhow::Error {
        anyhow!("binding state apply failed for the test")
    }

    fn send_test_reply(reply_tx: oneshot::Sender<anyhow::Result<RendererOwnerReply>>) {
        assert!(
            reply_tx.send(Err(test_reply_error())).is_ok(),
            "the test reply channel must stay open"
        );
    }

    fn assert_settled_with_test_error(
        settled: Result<Result<RendererCommandTurnOutput>, RendererPageCommandPending>,
    ) {
        let settled = match settled {
            Ok(settled) => settled,
            Err(_) => panic!("the dispatch must settle instead of being handed back"),
        };
        match settled {
            Err(error) => assert!(
                error.to_string().contains("binding state apply failed"),
                "unexpected settle error: {error:#}"
            ),
            Ok(_) => panic!("the test reply carries an apply error"),
        }
    }

    #[tokio::test]
    async fn dialog_open_at_entry_hands_the_dispatch_back_alive() {
        let (runtime, modal) = broker_with_open_modal("open at entry");
        let broker = runtime.broker();
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = deferred_settle_pending(reply_rx, &broker);

        let handed_back =
            match tokio::time::timeout(SETTLE_TIMEOUT, pending.wait_until_settled_or_dialog_open())
                .await
                .expect("an open modal must hand the dispatch back instead of parking")
            {
                Err(handed_back) => handed_back,
                Ok(_) => panic!("the dispatch must be handed back while the modal is open"),
            };

        // The handed-back dispatch stays alive: a plain wait settles once the
        // renderer reply arrives even though the modal is still open.
        send_test_reply(reply_tx);
        let settled = tokio::time::timeout(SETTLE_TIMEOUT, handed_back.wait())
            .await
            .expect("handed-back dispatch must still settle");
        assert_settled_with_test_error(Ok(settled));
        modal.cancel();
    }

    #[tokio::test]
    async fn dialog_opening_mid_wait_hands_the_dispatch_back_alive() {
        let runtime = RendererJavaScriptDialogRuntime::default();
        let broker = runtime.broker();
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = deferred_settle_pending(reply_rx, &broker);

        let (modal_tx, modal_rx) = oneshot::channel::<RendererModalJavaScriptDialog>();
        // The pending watched this runtime's broker; opening the modal through
        // the same runtime signals that watch mid-wait.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let (_dialog, modal) = runtime.begin_modal(pending_dialog("opens mid wait"));
            let _ = modal_tx.send(modal);
        });

        let handed_back =
            match tokio::time::timeout(SETTLE_TIMEOUT, pending.wait_until_settled_or_dialog_open())
                .await
                .expect("a dialog opening mid-wait must hand the dispatch back")
            {
                Err(handed_back) => handed_back,
                Ok(_) => panic!("the dispatch must be handed back while the modal is open"),
            };

        send_test_reply(reply_tx);
        let settled = tokio::time::timeout(SETTLE_TIMEOUT, handed_back.wait())
            .await
            .expect("handed-back dispatch must still settle");
        assert_settled_with_test_error(Ok(settled));
        let modal = modal_rx.await.expect("dialog opener task result");
        modal.cancel();
    }

    #[tokio::test]
    async fn without_a_dialog_the_dispatch_settles_normally() {
        let runtime = RendererJavaScriptDialogRuntime::default();
        let broker = runtime.broker();
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = deferred_settle_pending(reply_rx, &broker);

        send_test_reply(reply_tx);

        let settled =
            tokio::time::timeout(SETTLE_TIMEOUT, pending.wait_until_settled_or_dialog_open())
                .await
                .expect("dispatch must settle without a dialog");
        assert_settled_with_test_error(settled);
    }

    #[tokio::test]
    async fn completed_dispatch_wins_over_dialog_observation() {
        let (runtime, modal) = broker_with_open_modal("open before settling");
        let broker = runtime.broker();
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = deferred_settle_pending(reply_rx, &broker);
        send_test_reply(reply_tx);

        let settled =
            tokio::time::timeout(SETTLE_TIMEOUT, pending.wait_until_settled_or_dialog_open())
                .await
                .expect("dispatch must settle");
        assert_settled_with_test_error(settled);
        modal.cancel();
    }
}
