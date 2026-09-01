//! Body-only application of Worker host/control records.
//!
//! These records share the Page Networking source, but they do not all enter
//! the Page's V8 context. The selected-task dispatcher owns the ordinary
//! checkpoint for every applied current task. Console publication and
//! relay-terminal settlement remain body-only even though they do not enter
//! V8 themselves; stale-target cleanup produces no current task completion.
//! No branch owns callback-style child/runtime follow-up.

use std::{cell::RefCell, rc::Rc};

use anyhow::{Result, bail};
use moli_shared_worker::SharedWorkerInstanceId;

use super::ScriptVm;
use crate::{
    native_bridge::JsContextHost,
    types::{DedicatedWorkerId, PendingSubresourceContinueEvent, SubresourceResourceType},
    worker::{
        WorkerPendingSubresourceFetch, WorkerRuntimeEvent, WorkerToParentMessage,
        WorkerWebSocketFrameEvent, WorkerWebSocketLifecycleEvent,
    },
};

/// What an authorized Worker host-bridge body actually did.
///
/// This is an execution result, not scheduler metadata. It can only be
/// produced after the exact root Page has been authorized and the concrete
/// Worker record has been consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerHostBridgeBodyEffect {
    /// The record updated output/control state without entering the Page V8
    /// context. The body performs no checkpoint itself; the applied current
    /// Page task still owes its ordinary selected-task checkpoint.
    StateAppliedWithoutPageContext,
    /// The record entered the current Page context to update Worker-owned
    /// network/control state. The body deliberately leaves the ordinary
    /// task-end checkpoint to the selected-task dispatcher.
    StateAppliedInPageContext,
    /// The exact DedicatedWorker execution context or realm was already
    /// retired. No current Page context was entered.
    ExactTargetUnavailable,
}

/// Exact Worker namespace used while translating one host/control record.
///
/// It is deliberately separate from `WorkerRuntimeEvent`: by the time this
/// value exists, the caller has already selected the right V8 context. The
/// common record dispatcher can therefore match each message kind once while
/// still choosing the owner-specific pending-fetch and WebSocket identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerHostRecordOwner {
    Dedicated(DedicatedWorkerId),
    Shared(SharedWorkerInstanceId),
}

impl WorkerHostRecordOwner {
    fn record_pending_fetch(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        context_host: &Rc<RefCell<JsContextHost>>,
        pending: WorkerPendingSubresourceFetch,
    ) {
        let context = v8::Global::new(scope, scope.get_current_context());
        match (self, pending.info.resource_type) {
            (Self::Dedicated(worker_id), SubresourceResourceType::Xhr) => {
                context_host
                    .borrow_mut()
                    .record_pending_worker_subresource_xhr(
                        context,
                        worker_id,
                        pending.fetch_id,
                        pending.load,
                        pending.credentials_mode,
                        pending.network_partition_key,
                        pending.info,
                    );
            }
            (Self::Dedicated(worker_id), SubresourceResourceType::CspReport) => {
                context_host
                    .borrow_mut()
                    .record_pending_worker_subresource_csp_report(
                        context,
                        worker_id,
                        pending.fetch_id,
                        pending.load,
                        pending.credentials_mode,
                        pending.request_mode,
                        pending.network_partition_key,
                        pending.info,
                    );
            }
            (Self::Dedicated(worker_id), _) => {
                context_host
                    .borrow_mut()
                    .record_pending_worker_subresource_fetch(
                        context,
                        worker_id,
                        pending.fetch_id,
                        pending.load,
                        pending.credentials_mode,
                        pending.request_mode,
                        pending.network_partition_key,
                        pending.info,
                    );
            }
            (Self::Shared(instance_id), SubresourceResourceType::Xhr) => {
                context_host
                    .borrow_mut()
                    .record_pending_shared_worker_subresource_xhr(
                        context,
                        instance_id,
                        pending.fetch_id,
                        pending.load,
                        pending.credentials_mode,
                        pending.network_partition_key,
                        pending.info,
                    );
            }
            (Self::Shared(instance_id), SubresourceResourceType::CspReport) => {
                context_host
                    .borrow_mut()
                    .record_pending_shared_worker_subresource_csp_report(
                        context,
                        instance_id,
                        pending.fetch_id,
                        pending.load,
                        pending.credentials_mode,
                        pending.request_mode,
                        pending.network_partition_key,
                        pending.info,
                    );
            }
            (Self::Shared(instance_id), _) => {
                context_host
                    .borrow_mut()
                    .record_pending_shared_worker_subresource_fetch(
                        context,
                        instance_id,
                        pending.fetch_id,
                        pending.load,
                        pending.credentials_mode,
                        pending.request_mode,
                        pending.network_partition_key,
                        pending.info,
                    );
            }
        }
    }

    fn cancel_pending_fetch(
        self,
        context_host: &Rc<RefCell<JsContextHost>>,
        fetch_id: u32,
        error_text: String,
    ) {
        match self {
            Self::Dedicated(worker_id) => {
                context_host
                    .borrow_mut()
                    .cancel_pending_worker_subresource_fetch(worker_id, fetch_id, error_text);
            }
            Self::Shared(instance_id) => {
                context_host
                    .borrow_mut()
                    .cancel_pending_shared_worker_subresource_fetch(
                        instance_id,
                        fetch_id,
                        error_text,
                    );
            }
        }
    }

    fn encoded_websocket_id(self, local_socket_id: u64) -> u64 {
        match self {
            Self::Dedicated(worker_id) => {
                ScriptVm::worker_websocket_socket_id(worker_id, local_socket_id).as_u64()
            }
            Self::Shared(instance_id) => {
                ScriptVm::shared_worker_websocket_socket_id(instance_id, local_socket_id).as_u64()
            }
        }
    }

    fn websocket_lifecycle_event(
        self,
        event: &WorkerWebSocketLifecycleEvent,
    ) -> crate::types::WebSocketLifecycleEvent {
        match self {
            Self::Dedicated(worker_id) => {
                ScriptVm::worker_websocket_lifecycle_event(worker_id, event)
            }
            Self::Shared(instance_id) => {
                ScriptVm::shared_worker_websocket_lifecycle_event(instance_id, event)
            }
        }
    }

    fn websocket_frame_event(
        self,
        event: &WorkerWebSocketFrameEvent,
    ) -> crate::types::WebSocketNetworkEvent {
        match self {
            Self::Dedicated(worker_id) => ScriptVm::worker_websocket_frame_event(worker_id, event),
            Self::Shared(instance_id) => {
                ScriptVm::shared_worker_websocket_frame_event(instance_id, event)
            }
        }
    }

    fn rejects_client_facing_record(self) -> bool {
        matches!(self, Self::Dedicated(_))
    }
}

impl ScriptVm {
    pub(crate) fn apply_current_worker_host_bridge_event_body(
        &mut self,
        event: WorkerRuntimeEvent,
    ) -> Result<WorkerHostBridgeBodyEffect> {
        if let Some(worker_id) = event.dedicated_worker_id()
            && !self
                ._context_host
                .borrow_mut()
                .worker_execution_context_is_current(worker_id)
        {
            return Ok(WorkerHostBridgeBodyEffect::ExactTargetUnavailable);
        }

        if let WorkerRuntimeEvent::Message { worker_id, message } = &event
            && let WorkerToParentMessage::Console(message) = message.as_ref()
        {
            let applied = self
                ._context_host
                .borrow_mut()
                .record_dedicated_worker_target_console_message(*worker_id, message.clone());
            return Ok(if applied {
                WorkerHostBridgeBodyEffect::StateAppliedWithoutPageContext
            } else {
                WorkerHostBridgeBodyEffect::ExactTargetUnavailable
            });
        }

        let console_message = match &event {
            WorkerRuntimeEvent::SharedWorkerMessage { message, .. } => match message.as_ref() {
                WorkerToParentMessage::Console(message) => Some(message.clone()),
                _ => None,
            },
            WorkerRuntimeEvent::Message { .. } | WorkerRuntimeEvent::HostBridgeDrained { .. } => {
                None
            }
        };
        if let Some(message) = console_message {
            self.record_worker_console_message(message);
            return Ok(WorkerHostBridgeBodyEffect::StateAppliedWithoutPageContext);
        }

        if let WorkerRuntimeEvent::Message { worker_id, message } = &event
            && let WorkerToParentMessage::RuntimeInspectorMessages(batches) = message.as_ref()
        {
            let applied = self
                ._context_host
                .borrow_mut()
                .record_dedicated_worker_runtime_inspector_messages(*worker_id, batches.clone());
            return Ok(if applied {
                WorkerHostBridgeBodyEffect::StateAppliedWithoutPageContext
            } else {
                WorkerHostBridgeBodyEffect::ExactTargetUnavailable
            });
        }

        match event {
            WorkerRuntimeEvent::HostBridgeDrained { worker_id } => {
                let applied = self
                    ._context_host
                    .borrow_mut()
                    .mark_dedicated_worker_host_bridge_drained(worker_id);
                Ok(if applied {
                    WorkerHostBridgeBodyEffect::StateAppliedWithoutPageContext
                } else {
                    WorkerHostBridgeBodyEffect::ExactTargetUnavailable
                })
            }
            WorkerRuntimeEvent::SharedWorkerMessage {
                instance_id,
                message,
            } => self.apply_shared_worker_host_bridge_record_body(instance_id, *message),
            WorkerRuntimeEvent::Message {
                message,
                worker_id: _,
            } if matches!(
                message.as_ref(),
                WorkerToParentMessage::RuntimeInspectorResponse(_)
            ) =>
            {
                let WorkerToParentMessage::RuntimeInspectorResponse(publication) = *message else {
                    unreachable!("the guarded Worker response variant must still match")
                };
                let published = self
                    ._context_host
                    .borrow()
                    .publish_worker_runtime_inspector_response(publication);
                Ok(if published {
                    WorkerHostBridgeBodyEffect::StateAppliedWithoutPageContext
                } else {
                    WorkerHostBridgeBodyEffect::ExactTargetUnavailable
                })
            }
            WorkerRuntimeEvent::Message { worker_id, message } => {
                self.apply_dedicated_worker_host_bridge_record_body(worker_id, *message)
            }
        }
    }

    fn record_worker_console_message(&mut self, message: crate::worker::WorkerConsoleMessage) {
        if let Some(execution_context_id) = self.runtime_observable_default_execution_context_id() {
            self.runtime_observable_source_queue.record_console_message(
                crate::runtime::RuntimeConsoleMessageSnapshot {
                    execution_context_id,
                    message: message.message,
                    args: message.args,
                    stack: message.stack,
                },
            );
        } else {
            self.runtime_observable_source_queue
                .record_pending_console_event(
                    crate::native_bridge::PendingRuntimeObservableConsoleSourceEvent::new(
                        self.page_default_runtime_observable_context_token,
                        message.message,
                        message.args,
                        message.stack,
                    ),
                );
        }
    }

    fn apply_shared_worker_host_bridge_record_body(
        &mut self,
        instance_id: SharedWorkerInstanceId,
        message: WorkerToParentMessage,
    ) -> Result<WorkerHostBridgeBodyEffect> {
        let context_host = self._context_host.clone();
        self.with_default_context_scope(|scope, _host_ptr| {
            Self::apply_worker_host_bridge_record_in_scope(
                scope,
                &context_host,
                WorkerHostRecordOwner::Shared(instance_id),
                message,
            )
        })?;
        Ok(WorkerHostBridgeBodyEffect::StateAppliedInPageContext)
    }

    fn apply_dedicated_worker_host_bridge_record_body(
        &mut self,
        worker_id: DedicatedWorkerId,
        message: WorkerToParentMessage,
    ) -> Result<WorkerHostBridgeBodyEffect> {
        let context_host = self._context_host.clone();
        let applied = self.with_default_context_scope(
            |scope, _host_ptr| {
                let target = {
                    let mut host = context_host.borrow_mut();
                    host.worker_dispatch_target(scope, worker_id)
                };
                let Some((_dispatch_scope, realm_token, context, _worker)) = target else {
                    return Ok(false);
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                if crate::native_bridge::current_runtime_observable_context_token(scope)
                    != Some(realm_token)
                {
                    context_host.borrow_mut().forget_worker(worker_id);
                    tracing::debug!(
                        worker_id = worker_id.as_u64(),
                        ?realm_token,
                        actual_realm_token = ?crate::native_bridge::current_runtime_observable_context_token(scope),
                        "dropped DedicatedWorker host record for stale realm"
                    );
                    return Ok(false);
                }

                Self::apply_worker_host_bridge_record_in_scope(
                    scope,
                    &context_host,
                    WorkerHostRecordOwner::Dedicated(worker_id),
                    message,
                )?;
                Ok(true)
            },
        )?;
        Ok(if applied {
            WorkerHostBridgeBodyEffect::StateAppliedInPageContext
        } else {
            WorkerHostBridgeBodyEffect::ExactTargetUnavailable
        })
    }

    fn apply_worker_host_bridge_record_in_scope(
        scope: &mut v8::PinScope<'_, '_>,
        context_host: &Rc<RefCell<JsContextHost>>,
        owner: WorkerHostRecordOwner,
        message: WorkerToParentMessage,
    ) -> Result<()> {
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => {
                context_host.borrow_mut().record_subresource_network(record);
            }
            WorkerToParentMessage::PendingSubresourceFetch(pending) => {
                owner.record_pending_fetch(scope, context_host, pending);
            }
            WorkerToParentMessage::PendingSubresourceFetchCanceled {
                fetch_id,
                error_text,
            } => {
                owner.cancel_pending_fetch(context_host, fetch_id, error_text);
            }
            WorkerToParentMessage::SubresourceContinue(event) => match event {
                PendingSubresourceContinueEvent::ResponsePaused(info) => {
                    context_host
                        .borrow_mut()
                        .record_worker_subresource_response_pause(info);
                }
                PendingSubresourceContinueEvent::AuthRequired(info) => {
                    context_host
                        .borrow_mut()
                        .record_worker_subresource_auth_pause(info);
                }
                PendingSubresourceContinueEvent::Completed { internal_id } => {
                    context_host
                        .borrow_mut()
                        .record_worker_subresource_completed(internal_id);
                }
            },
            WorkerToParentMessage::WebSocketSubresource(record) => {
                let Some(local_socket_id) = record.websocket_socket_id() else {
                    return Ok(());
                };
                context_host.borrow_mut().record_subresource_network(
                    record.with_websocket_socket_id(owner.encoded_websocket_id(local_socket_id)),
                );
            }
            WorkerToParentMessage::WebSocketLifecycle(event) => {
                context_host
                    .borrow_mut()
                    .record_websocket_lifecycle_event(owner.websocket_lifecycle_event(&event));
            }
            WorkerToParentMessage::WebSocketFrame(event) => {
                context_host
                    .borrow_mut()
                    .record_websocket_network_event(owner.websocket_frame_event(&event));
            }
            WorkerToParentMessage::Post(_) | WorkerToParentMessage::Error { .. }
                if owner.rejects_client_facing_record() =>
            {
                bail!("client-facing DedicatedWorker event entered the host-bridge source");
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::Error { .. }
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
            | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerShowNotification(_)
            | WorkerToParentMessage::ServiceWorkerGetNotifications(_)
            | WorkerToParentMessage::ServiceWorkerSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_)
            | WorkerToParentMessage::ServiceWorkerCloseNotification(_)
            | WorkerToParentMessage::ServiceWorkerClientMessage(_)
            | WorkerToParentMessage::ServiceWorkerWorkerMessage(_)
            | WorkerToParentMessage::ServiceWorkerClientQuery(_)
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerClientFocus(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerClosed
            | WorkerToParentMessage::RuntimeInspectorResponse(_) => {}
        }
        Ok(())
    }
}
