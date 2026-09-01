use std::sync::Arc;

use tracing::trace;

use crate::{
    page_task_queue::is_worker_host_bridge_message,
    worker::{
        WorkerErrorPhase, WorkerParentErrorEventKind, WorkerRuntimeEvent, WorkerToParentMessage,
    },
};

use super::host::RendererSharedWorkerHost;

impl RendererSharedWorkerHost {
    pub(super) fn handle_worker_parent_message(
        self: &Arc<Self>,
        message: WorkerToParentMessage,
        script_url: &str,
    ) {
        match message {
            WorkerToParentMessage::Error {
                message,
                filename,
                lineno,
                colno,
                event_kind,
                phase,
                source,
            } => {
                if phase == WorkerErrorPhase::Bootstrap {
                    if event_kind == WorkerParentErrorEventKind::ErrorEvent {
                        self.notify_all_clients_error_with_location(
                            message, filename, lineno, colno, event_kind,
                        );
                        return;
                    }
                    self.fail_bootstrap(message, filename, lineno, colno, event_kind);
                    return;
                }
                trace!(
                    url = %script_url,
                    message,
                    filename,
                    lineno,
                    colno,
                    ?event_kind,
                    ?source,
                    "dropping shared worker runtime error; SharedWorker.onerror is reserved for load and connection failures"
                );
            }
            WorkerToParentMessage::SharedWorkerClosed => {
                self.begin_runtime_response_retirement();
                self.notify_worker_closed();
            }
            WorkerToParentMessage::RuntimeInspectorResponse(publication) => {
                self.publish_runtime_inspector_response(publication);
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_) => {
                trace!(url = %script_url, ?message, "dropping service worker lifecycle message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(_) => {
                trace!(url = %script_url, ?message, "dropping service worker fetch message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_) => {
                trace!(url = %script_url, ?message, "dropping service worker fetch stream message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(_) => {
                trace!(url = %script_url, ?message, "dropping service worker message event completion on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerNotificationCompleted(_) => {
                trace!(url = %script_url, ?message, "dropping service worker notification event completion on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_) => {
                trace!(url = %script_url, ?message, "dropping service worker push event completion on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerSyncCompleted(_) => {
                trace!(url = %script_url, ?message, "dropping service worker sync event completion on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_) => {
                trace!(url = %script_url, ?message, "dropping service worker periodic sync event completion on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerShowNotification(_) => {
                trace!(url = %script_url, ?message, "dropping service worker showNotification request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerGetNotifications(_) => {
                trace!(url = %script_url, ?message, "dropping service worker getNotifications request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerSyncRegistration(_) => {
                trace!(url = %script_url, ?message, "dropping service worker sync registration request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerSyncGetTags(_) => {
                trace!(url = %script_url, ?message, "dropping service worker sync getTags request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_) => {
                trace!(url = %script_url, ?message, "dropping service worker periodic sync registration request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_) => {
                trace!(url = %script_url, ?message, "dropping service worker periodic sync getTags request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_) => {
                trace!(url = %script_url, ?message, "dropping service worker periodic sync unregister request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerCloseNotification(_) => {
                trace!(url = %script_url, ?message, "dropping service worker notification close request on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerClientMessage(_) => {
                trace!(url = %script_url, ?message, "dropping service worker client message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerWorkerMessage(_) => {
                trace!(url = %script_url, ?message, "dropping service worker worker message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerClientQuery(_) => {
                trace!(url = %script_url, ?message, "dropping service worker client query on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerClientNavigate(_) => {
                trace!(url = %script_url, ?message, "dropping service worker client navigate on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerClientFocus(_) => {
                trace!(url = %script_url, ?message, "dropping service worker client focus on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_) => {
                trace!(url = %script_url, ?message, "dropping service worker clients.openWindow on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerSkipWaiting { .. } => {
                trace!(url = %script_url, ?message, "dropping service worker skipWaiting message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerClientsClaim { .. } => {
                trace!(url = %script_url, ?message, "dropping service worker clients.claim message on shared worker host");
            }
            WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. } => {
                trace!(url = %script_url, ?message, "dropping service worker imported script resource on shared worker host");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(messages) => {
                self.record_runtime_inspector_messages_if_running(messages, script_url);
            }
            message @ WorkerToParentMessage::Console(_) => {
                if let WorkerToParentMessage::Console(console) = &message {
                    self.record_console_message_if_running(console);
                }
                self.send_host_bridge_message(message, script_url);
            }
            message if is_worker_host_bridge_message(&message) => {
                self.send_host_bridge_message(message, script_url);
            }
            WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Post(_) => {
                trace!(url = %script_url, ?message, "dropping unsupported shared worker parent message");
            }
        }
    }

    pub(super) fn notify_worker_closed(&self) {
        let runtime_service = self.runtime_service();
        if !runtime_service.enqueue_service_lane_worker_closed(self.instance_id()) {
            return;
        }
        runtime_service.signal_service_lane_wake();
    }

    fn send_host_bridge_message(&self, message: WorkerToParentMessage, script_url: &str) -> bool {
        let Some(sender) = self.worker_host_bridge_sender() else {
            trace!(
                url = %script_url,
                instance_id = self.instance_id().as_u64(),
                ?message,
                "dropping shared worker host bridge message because no live client can receive owner wakeups"
            );
            return false;
        };
        let result = sender.send(WorkerRuntimeEvent::SharedWorkerMessage {
            instance_id: self.instance_id(),
            message: Box::new(message),
        });
        if let Err(error) = result {
            trace!(
                url = %script_url,
                instance_id = self.instance_id().as_u64(),
                ?error,
                "failed to enqueue shared worker host bridge message"
            );
            return false;
        }
        true
    }
}
