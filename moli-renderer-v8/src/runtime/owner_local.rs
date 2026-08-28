use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use super::owner_local_store::{
    RendererPageToken, has_current_render_runtime_owner_local_store,
    remove_page_after_target_close_on_bound_owner_local_store,
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

fn remove_page_after_target_close(token: RendererPageToken, terminated_active_execution: bool) {
    remove_page_after_target_close_on_bound_owner_local_store(token, terminated_active_execution)
}

#[doc(hidden)]
pub struct RendererAttachedPage {
    pub(super) token: RendererPageToken,
    pub(super) devtools_agent_token: RendererDevToolsAgentToken,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) javascript_dialog_broker: RendererJavaScriptDialogBroker,
    pub(super) devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
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
        self.committed_document_post_response_continuation = Some(
            RendererPageCommandPostResponseContinuation::new(move || {
                let _ = page_wake_tx.send(RendererOwnerWake::committed_document_parser_unblocked(
                    token,
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

pub struct RendererPageHandle {
    local_executor: JsLocalExecutor,
    render_runtime: RenderRuntimeHandle,
    token: Option<RendererPageToken>,
    devtools_agent_token: RendererDevToolsAgentToken,
    page_context_cancel_tx: RendererPageContextCancelSender,
    javascript_dialog_broker: RendererJavaScriptDialogBroker,
    devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
    script_execution_control: crate::script_execution_control::RendererScriptExecutionControl,
    committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
    _not_send: PhantomData<Rc<()>>,
}

pub struct RendererPageCommandPending {
    dispatch: Option<RendererPageCommandPendingDispatch>,
    javascript_dialog_watch: Option<RendererJavaScriptDialogWatch>,
    cancellation: Option<Arc<AtomicU8>>,
}

enum RendererPageCommandPendingDispatch {
    Owner(oneshot::Receiver<anyhow::Result<RendererOwnerReply>>),
    InspectorMain(Box<RendererRuntimeInspectorMainCommandRoute>),
}

const PAGE_COMMAND_CANCELLATION_PENDING: u8 = 0;
const PAGE_COMMAND_CANCELLATION_CANCELLED: u8 = 2;

/// A non-owning request for reserving the next Document in one exact live
/// renderer Page.
///
/// This carries only the renderer owner route and stable Page token. It may be
/// moved into protocol background work without cloning the Page handle or its
/// close authority.
pub struct RendererPageReplacementReservationPending {
    render_runtime: RenderRuntimeHandle,
    token: RendererPageToken,
    output_owner_reservation_id: RendererPageOutputOwnerReservationId,
    _not_send: PhantomData<Rc<()>>,
}

impl RendererPageReplacementReservationPending {
    pub fn output_owner_reservation_id(&self) -> RendererPageOutputOwnerReservationId {
        self.output_owner_reservation_id
    }

    pub async fn await_ready(self) -> Result<RendererPageReservationToken> {
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::ReserveLivePageReplacement {
                token: self.token,
                output_owner_reservation_id: self.output_owner_reservation_id,
            })
            .await?
        {
            RendererOwnerReply::LivePageReplacementReserved(reservation) => Ok(reservation),
            _ => Err(anyhow!(
                "renderer owner returned non-reservation reply for live Page replacement"
            )),
        }
    }
}

#[derive(Clone)]
pub struct RendererPageTestingHandle {
    render_runtime: RenderRuntimeHandle,
    token: RendererPageToken,
    _not_send: PhantomData<Rc<()>>,
}

#[must_use = "the owner detach command must retain this guard until it drops the V8 session"]
pub struct RendererRuntimeInspectorSessionDetachGuard {
    inspector_pause_bridge: Option<crate::devtools::pause::RendererInspectorPauseBridge>,
}

impl RendererRuntimeInspectorSessionDetachGuard {
    fn new(inspector_pause_bridge: crate::devtools::pause::RendererInspectorPauseBridge) -> Self {
        inspector_pause_bridge.arm_session_detach();
        Self {
            inspector_pause_bridge: Some(inspector_pause_bridge),
        }
    }
}

impl Drop for RendererRuntimeInspectorSessionDetachGuard {
    fn drop(&mut self) {
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
    pub fn crash_devtools_target_from_io(&self) {
        self.devtools_target
            .crash_page_target_from_io(self.renderer_page_id(), self.devtools_agent_token);
    }

    #[doc(hidden)]
    pub fn take_committed_document_post_response_continuation(
        &mut self,
    ) -> Option<RendererPageCommandPostResponseContinuation> {
        self.committed_document_post_response_continuation.take()
    }

    /// Adopts the current Document's inspector identity without replacing the
    /// Page handle or its close/command authority.
    pub fn adopt_page_replacement(
        &mut self,
        replacement: &RendererPageReplacementCommit,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            replacement.owner_local_host_id() == self.owner_local_host_id()
                && replacement.page_id() == self.renderer_page_id(),
            "renderer Page replacement belongs to a different stable Page"
        );
        self.devtools_target.detach_page(
            self.renderer_page_id(),
            self.devtools_agent_token,
            "Inspector Page document was replaced",
        );
        self.javascript_dialog_broker.dismiss_pending();
        self.devtools_agent_token = replacement.renderer_devtools_agent_token();
        self.javascript_dialog_broker = replacement.javascript_dialog_broker.clone();
        self.devtools_target = replacement.devtools_target.clone();
        Ok(())
    }

    /// Reserves a replacement Document for this exact live Page generation.
    ///
    /// The renderer owner serializes this snapshot with Page turns. Preparing
    /// the replacement fails closed if another cross-document commit changes
    /// the resident `PageVm` before the reservation is consumed. A newer
    /// unconsumed reservation for the same Page supersedes an older one.
    pub async fn reserve_replacement_document_for_navigation(
        &self,
    ) -> Result<RendererPageReservationToken> {
        self.start_replacement_document_reservation()
            .await_ready()
            .await
    }

    pub fn start_replacement_document_reservation(
        &self,
    ) -> RendererPageReplacementReservationPending {
        RendererPageReplacementReservationPending {
            render_runtime: self.render_runtime.clone(),
            token: self.token(),
            output_owner_reservation_id: RendererPageOutputOwnerReservationId::allocate(),
            _not_send: PhantomData,
        }
    }

    pub fn take_pending_modal_javascript_dialogs(&self) -> Vec<RendererPendingJavaScriptDialog> {
        self.javascript_dialog_broker.take_pending()
    }

    pub fn has_open_modal_javascript_dialog(&self) -> bool {
        self.javascript_dialog_broker.has_open_dialog()
    }

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

    /// Enqueues the DevTools IO-agent script policy without borrowing the
    /// owner-resident `PageVm` that may currently be executing JavaScript.
    #[doc(hidden)]
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

    pub fn arm_runtime_inspector_session_detach(
        &self,
    ) -> RendererRuntimeInspectorSessionDetachGuard {
        RendererRuntimeInspectorSessionDetachGuard::new(self.devtools_target.pause())
    }

    /// Disconnects one frontend Inspector route without waiting for the Page
    /// owner to return from JavaScript.
    ///
    /// Chromium acknowledges `Target.detachFromTarget` after dropping the
    /// browser-side DevToolsSession pipes; destruction of the renderer-side
    /// V8InspectorSession is a subsequent Main-thread task. Mirror that
    /// boundary here: cancel both ingress lanes synchronously, then enqueue an
    /// owner-only cleanup whose reply is deliberately detached. The retained
    /// pause guard also releases a nested debugger loop so the cleanup can
    /// eventually reach the owner.
    pub fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        let session = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id
                .as_deref()
                .filter(|session_id| !session_id.is_empty()),
        );
        self.devtools_target
            .detach_session(self.devtools_agent_token, &session);

        let pause_guard = self.arm_runtime_inspector_session_detach();
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
            false,
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
            false,
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
            false,
        )
    }

    #[doc(hidden)]
    pub fn enqueue_cancellable_async_command(
        &self,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageCommandPending> {
        self.enqueue_async_command_with_capture_policy(
            command,
            RendererPageStateCapturePolicy::FullReport,
            false,
            None,
            true,
        )
    }

    #[doc(hidden)]
    pub fn enqueue_cancellable_protocol_command(
        &self,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageCommandPending> {
        self.enqueue_async_command_with_capture_policy(
            command,
            RendererPageStateCapturePolicy::ProtocolTurn,
            false,
            None,
            true,
        )
    }

    fn enqueue_async_command_with_capture_policy(
        &self,
        command: RendererPageCommand,
        capture_policy: RendererPageStateCapturePolicy,
        route_protocol_main_receiver: bool,
        inspector_session_id: Option<String>,
        cancellable: bool,
    ) -> anyhow::Result<RendererPageCommandPending> {
        let javascript_dialog_watch = command
            .interruptible_by_javascript_dialog()
            .then(|| self.javascript_dialog_broker.watch());
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
                dispatch: Some(RendererPageCommandPendingDispatch::InspectorMain(Box::new(
                    route,
                ))),
                javascript_dialog_watch,
                cancellation: None,
            });
        }
        let command = match command {
            RendererPageCommand::Inspector(envelope) => {
                let route = self.devtools_target.main_ref().enqueue_owner_command(
                    self.token(),
                    self.devtools_agent_token,
                    envelope,
                    capture_policy,
                );
                return Ok(RendererPageCommandPending {
                    dispatch: Some(RendererPageCommandPendingDispatch::InspectorMain(Box::new(
                        route,
                    ))),
                    javascript_dialog_watch,
                    cancellation: None,
                });
            }
            command => command,
        };
        let cancellation =
            cancellable.then(|| Arc::new(AtomicU8::new(PAGE_COMMAND_CANCELLATION_PENDING)));
        let owner_command = match capture_policy {
            RendererPageStateCapturePolicy::FullReport => {
                RendererOwnerCommand::RunAsyncPageCommand {
                    token: self.token(),
                    command,
                    cancellation: cancellation.clone(),
                }
            }
            RendererPageStateCapturePolicy::ProtocolTurn => {
                RendererOwnerCommand::RunProtocolPageCommand {
                    token: self.token(),
                    command,
                    cancellation: cancellation.clone(),
                }
            }
        };
        let reply_rx = self.render_runtime.enqueue(owner_command)?;
        Ok(RendererPageCommandPending {
            dispatch: Some(RendererPageCommandPendingDispatch::Owner(reply_rx)),
            javascript_dialog_watch,
            cancellation,
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
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::WaitForNetworkIdle {
                token: self.token(),
                timeout_ms,
                loader,
            })
            .await?
        {
            RendererOwnerReply::AsyncPageCommandRan(result) => Ok(result.into_reply_and_state()),
            _ => Err(anyhow!(
                "renderer owner returned non-async page-command reply for wait-for-network-idle"
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
        let terminated_active_execution = self.devtools_target.close_page_target(
            token.page_id,
            self.devtools_agent_token,
            "Inspector target closed with its Page handle",
        );
        self.javascript_dialog_broker.dismiss_pending();
        self.page_context_cancel_tx
            .cancel(RendererPageContextCancelReason::PageClosed);
        tracing::debug!(
            page_id = token.page_id.as_u64(),
            terminated_active_execution,
            "closing renderer page handle"
        );

        if is_on_named_owner_execution_lane_for(&self.local_executor)
            && has_current_render_runtime_owner_local_store()
        {
            remove_page_after_target_close(token, terminated_active_execution);
            self.token = None;
            tracing::debug!("renderer page handle closed on owner lane");
            return Ok(());
        }

        // Keep the token installed until the owner acknowledges removal. If
        // this future is cancelled, Drop can still enqueue detached cleanup.
        match self
            .render_runtime
            .dispatch(RendererOwnerCommand::RemovePage {
                token,
                terminated_active_execution,
            })
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

impl RendererPageCommandPending {
    pub async fn wait(mut self) -> Result<RendererCommandTurnOutput> {
        let dispatch = self
            .dispatch
            .take()
            .expect("renderer Page command pending dispatch can be consumed only once");
        let wait_for_dispatch = async move {
            match dispatch {
                RendererPageCommandPendingDispatch::Owner(reply_rx) => {
                    match reply_rx
                        .await
                        .map_err(|_| anyhow!("render runtime reply channel closed"))??
                    {
                        RendererOwnerReply::AsyncPageCommandRan(result) => Ok(*result),
                        _ => Err(anyhow!(
                            "renderer owner returned non-async page-command reply for async page command"
                        )),
                    }
                }
                RendererPageCommandPendingDispatch::InspectorMain(route) => {
                    match route.wait_for_completion().await? {
                        RendererRuntimeInspectorMainCommandCompletion::Owner(output) => Ok(*output),
                        RendererRuntimeInspectorMainCommandCompletion::Page(output) => Ok(*output),
                        RendererRuntimeInspectorMainCommandCompletion::Inspector
                        | RendererRuntimeInspectorMainCommandCompletion::InspectorSessionResponse {
                            ..
                        } => Err(anyhow!(
                            "an owner-only Inspector Main command entered nested dispatch"
                        )),
                        RendererRuntimeInspectorMainCommandCompletion::OwnerSessionResponse {
                            ..
                        }
                        | RendererRuntimeInspectorMainCommandCompletion::OwnerSessionErrorSettled(
                            _,
                        ) => Err(anyhow!(
                            "an owner-only Page command settled a DevTools session response"
                        )),
                        RendererRuntimeInspectorMainCommandCompletion::Canceled(message) => {
                            Err(anyhow!(message))
                        }
                    }
                }
            }
        };
        tokio::pin!(wait_for_dispatch);
        let result = if let Some(javascript_dialog_watch) = self.javascript_dialog_watch.take() {
            tokio::select! {
                biased;
                reply = &mut wait_for_dispatch => reply,
                predecessor = javascript_dialog_watch.wait_until_open() => {
                    return Err(super::RendererPageCommandInterruptedByJavaScriptDialog::new(
                        predecessor,
                    ).into());
                }
            }
        } else {
            wait_for_dispatch.await
        };
        // The owner has either completed the command or returned a terminal
        // error. Disarm Drop so only an actually abandoned wait can cancel a
        // command that is still queued behind the renderer lane.
        self.cancellation.take();
        result
    }
}

impl Drop for RendererPageCommandPending {
    fn drop(&mut self) {
        let Some(cancellation) = self.cancellation.take() else {
            return;
        };
        let _ = cancellation.compare_exchange(
            PAGE_COMMAND_CANCELLATION_PENDING,
            PAGE_COMMAND_CANCELLATION_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl Drop for RendererPageHandle {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        self.devtools_target.detach_page(
            token.page_id,
            self.devtools_agent_token,
            "Inspector Page handle was dropped",
        );
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
            .dispatch_detached(RendererOwnerCommand::RemovePage {
                token,
                terminated_active_execution: false,
            });
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
    pub async fn install_related_page_window_proxy_for_experiment(
        &self,
        peer: &Self,
        property_name: &str,
    ) -> Result<()> {
        if !self.shares_local_host(peer) {
            return Err(anyhow!(
                "related WindowProxy probe requires Pages on the same renderer owner"
            ));
        }
        match self
            .render_runtime
            .dispatch(
                RendererOwnerCommand::TestingInstallRelatedPageWindowProxyForExperiment {
                    target: self.token,
                    peer: peer.token,
                    property_name: property_name.to_owned(),
                },
            )
            .await?
        {
            RendererOwnerReply::TestingRelatedPageWindowProxyInstalled => Ok(()),
            other => Err(anyhow!(
                "renderer owner returned non-WindowProxy-install testing reply: {:?}",
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
