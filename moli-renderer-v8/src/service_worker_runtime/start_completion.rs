use std::fmt;

use super::{
    clients::ServiceWorkerClientQuery,
    events::{
        ServiceWorkerClientFocus, ServiceWorkerClientMessage, ServiceWorkerClientNavigate,
        ServiceWorkerClientsOpenWindow, ServiceWorkerCloseNotification,
        ServiceWorkerFetchCompletion, ServiceWorkerFetchStreamChunk,
        ServiceWorkerFetchStreamStarted, ServiceWorkerGetNotifications,
        ServiceWorkerLifecycleCompletion, ServiceWorkerMessageCompletion,
        ServiceWorkerNotificationCompletion, ServiceWorkerPeriodicSyncCompletion,
        ServiceWorkerPeriodicSyncGetTags, ServiceWorkerPeriodicSyncRegistration,
        ServiceWorkerPeriodicSyncUnregistration, ServiceWorkerPushCompletion,
        ServiceWorkerPushGetSubscription, ServiceWorkerPushSubscribe, ServiceWorkerPushUnsubscribe,
        ServiceWorkerShowNotification, ServiceWorkerSyncCompletion, ServiceWorkerSyncGetTags,
        ServiceWorkerSyncRegistration, ServiceWorkerWorkerMessage,
    },
    host::SharedRendererServiceWorkerHost,
    ids::{ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    run_owner::ServiceWorkerRunOwner,
    script_loading::{ServiceWorkerScriptResource, ServiceWorkerScriptUpdateCheckCompletion},
    state::WeakServiceWorkerRuntimeService,
    version::{
        ServiceWorkerFetchHandlerType, ServiceWorkerIdleTimeout, ServiceWorkerVersionStartFailure,
    },
};
use crate::worker::WorkerScriptResource;

/// Output work from one physical Worker parent FIFO, not published facts.
/// It shares the existing service lane with bootstrap and lifecycle decisions.
#[derive(Debug)]
pub(super) enum ServiceWorkerTargetOutput {
    Console(crate::worker::WorkerConsoleMessage),
    Exception {
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: crate::worker::WorkerParentErrorEventKind,
        phase: crate::worker::WorkerErrorPhase,
        source: crate::worker::WorkerErrorSource,
    },
    InspectorMessages(Vec<crate::worker::WorkerRuntimeInspectorMessageBatch>),
    InspectorResponse(crate::runtime::RendererRuntimeInspectorResponsePublication),
}

pub(super) struct ServiceWorkerRuntimeCompletion {
    runtime_service: WeakServiceWorkerRuntimeService,
    kind: ServiceWorkerRuntimeCompletionKind,
}

enum ServiceWorkerRuntimeCompletionKind {
    TargetOutput {
        owner: ServiceWorkerRunOwner,
        output: ServiceWorkerTargetOutput,
    },
    VersionStartCompleted {
        owner: ServiceWorkerRunOwner,
        final_script_url: String,
        script_resource: ServiceWorkerScriptResource,
        fetch_handler_type: ServiceWorkerFetchHandlerType,
    },
    VersionStartFailed {
        owner: ServiceWorkerRunOwner,
        failure: ServiceWorkerVersionStartFailure,
    },
    ImportedScriptLoaded {
        registration_id: ServiceWorkerRegistrationId,
        owner: ServiceWorkerRunOwner,
        resource: WorkerScriptResource,
    },
    MainScriptUpdateCheckCompleted {
        registration_id: ServiceWorkerRegistrationId,
        result: ServiceWorkerScriptUpdateCheckCompletion,
    },
    LifecycleEventCompleted {
        completion: ServiceWorkerLifecycleCompletion,
    },
    FetchEventCompleted {
        completion: ServiceWorkerFetchCompletion,
    },
    FetchStreamStarted {
        started: ServiceWorkerFetchStreamStarted,
    },
    FetchStreamChunk {
        chunk: ServiceWorkerFetchStreamChunk,
    },
    MessageEventCompleted {
        completion: ServiceWorkerMessageCompletion,
    },
    NotificationEventCompleted {
        completion: ServiceWorkerNotificationCompletion,
    },
    PushEventCompleted {
        completion: ServiceWorkerPushCompletion,
    },
    SyncEventCompleted {
        completion: ServiceWorkerSyncCompletion,
    },
    PeriodicSyncEventCompleted {
        completion: ServiceWorkerPeriodicSyncCompletion,
    },
    ShowNotificationRequested {
        request: Box<ServiceWorkerShowNotification>,
        source_host: SharedRendererServiceWorkerHost,
    },
    GetNotificationsRequested {
        request: ServiceWorkerGetNotifications,
        source_host: SharedRendererServiceWorkerHost,
    },
    SyncRegistrationRequested {
        request: ServiceWorkerSyncRegistration,
        source_host: SharedRendererServiceWorkerHost,
    },
    SyncGetTagsRequested {
        request: ServiceWorkerSyncGetTags,
        source_host: SharedRendererServiceWorkerHost,
    },
    PeriodicSyncRegistrationRequested {
        request: ServiceWorkerPeriodicSyncRegistration,
        source_host: SharedRendererServiceWorkerHost,
    },
    PeriodicSyncGetTagsRequested {
        request: ServiceWorkerPeriodicSyncGetTags,
        source_host: SharedRendererServiceWorkerHost,
    },
    PeriodicSyncUnregistrationRequested {
        request: ServiceWorkerPeriodicSyncUnregistration,
        source_host: SharedRendererServiceWorkerHost,
    },
    PushSubscribeRequested {
        request: ServiceWorkerPushSubscribe,
        source_host: SharedRendererServiceWorkerHost,
    },
    PushGetSubscriptionRequested {
        request: ServiceWorkerPushGetSubscription,
        source_host: SharedRendererServiceWorkerHost,
    },
    PushUnsubscribeRequested {
        request: ServiceWorkerPushUnsubscribe,
        source_host: SharedRendererServiceWorkerHost,
    },
    CloseNotificationRequested {
        request: ServiceWorkerCloseNotification,
        owner: ServiceWorkerRunOwner,
    },
    ClientMessage {
        message: ServiceWorkerClientMessage,
    },
    WorkerMessage {
        message: ServiceWorkerWorkerMessage,
    },
    ClientQuery {
        query: ServiceWorkerClientQuery,
        owner: ServiceWorkerRunOwner,
    },
    ClientNavigate {
        navigate: ServiceWorkerClientNavigate,
        owner: ServiceWorkerRunOwner,
    },
    ClientNavigateCompleted {
        completion: crate::types::ServiceWorkerClientNavigateCompletion,
    },
    ClientFocus {
        focus: ServiceWorkerClientFocus,
        owner: ServiceWorkerRunOwner,
    },
    ClientFocusCompleted {
        completion: crate::types::ServiceWorkerClientFocusCompletion,
    },
    ClientsOpenWindow {
        open_window: ServiceWorkerClientsOpenWindow,
        owner: ServiceWorkerRunOwner,
    },
    ClientsOpenWindowCompleted {
        completion: crate::types::ServiceWorkerClientsOpenWindowCompletion,
    },
    IdleTimeout(ServiceWorkerIdleTimeout),
    SkipWaitingRequested {
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    },
    ClientsClaimRequested {
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    },
}

impl ServiceWorkerRuntimeCompletion {
    pub(super) fn target_output(
        runtime_service: WeakServiceWorkerRuntimeService,
        owner: ServiceWorkerRunOwner,
        output: ServiceWorkerTargetOutput,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::TargetOutput { owner, output },
        }
    }

    pub(super) fn version_start_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        owner: ServiceWorkerRunOwner,
        final_script_url: String,
        script_resource: ServiceWorkerScriptResource,
        fetch_handler_type: ServiceWorkerFetchHandlerType,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::VersionStartCompleted {
                owner,
                final_script_url,
                script_resource,
                fetch_handler_type,
            },
        }
    }

    pub(super) fn version_start_failed(
        runtime_service: WeakServiceWorkerRuntimeService,
        owner: ServiceWorkerRunOwner,
        failure: ServiceWorkerVersionStartFailure,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::VersionStartFailed { owner, failure },
        }
    }

    pub(super) fn imported_script_loaded(
        runtime_service: WeakServiceWorkerRuntimeService,
        registration_id: ServiceWorkerRegistrationId,
        owner: ServiceWorkerRunOwner,
        resource: WorkerScriptResource,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ImportedScriptLoaded {
                registration_id,
                owner,
                resource,
            },
        }
    }

    pub(super) fn main_script_update_check_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        registration_id: ServiceWorkerRegistrationId,
        result: ServiceWorkerScriptUpdateCheckCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::MainScriptUpdateCheckCompleted {
                registration_id,
                result,
            },
        }
    }

    pub(super) fn lifecycle_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerLifecycleCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::LifecycleEventCompleted { completion },
        }
    }

    pub(super) fn fetch_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerFetchCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::FetchEventCompleted { completion },
        }
    }

    pub(super) fn fetch_stream_started(
        runtime_service: WeakServiceWorkerRuntimeService,
        started: ServiceWorkerFetchStreamStarted,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::FetchStreamStarted { started },
        }
    }

    pub(super) fn fetch_stream_chunk(
        runtime_service: WeakServiceWorkerRuntimeService,
        chunk: ServiceWorkerFetchStreamChunk,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::FetchStreamChunk { chunk },
        }
    }

    pub(super) fn message_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerMessageCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::MessageEventCompleted { completion },
        }
    }

    pub(super) fn notification_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerNotificationCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::NotificationEventCompleted { completion },
        }
    }

    pub(super) fn push_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerPushCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PushEventCompleted { completion },
        }
    }

    pub(super) fn sync_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerSyncCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::SyncEventCompleted { completion },
        }
    }

    pub(super) fn periodic_sync_event_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: ServiceWorkerPeriodicSyncCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PeriodicSyncEventCompleted { completion },
        }
    }

    pub(super) fn show_notification_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerShowNotification,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ShowNotificationRequested {
                request: Box::new(request),
                source_host,
            },
        }
    }

    pub(super) fn get_notifications_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerGetNotifications,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::GetNotificationsRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn sync_registration_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerSyncRegistration,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::SyncRegistrationRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn sync_get_tags_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerSyncGetTags,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::SyncGetTagsRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn periodic_sync_registration_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerPeriodicSyncRegistration,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PeriodicSyncRegistrationRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn periodic_sync_get_tags_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerPeriodicSyncGetTags,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PeriodicSyncGetTagsRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn periodic_sync_unregistration_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerPeriodicSyncUnregistration,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PeriodicSyncUnregistrationRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn push_subscribe_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerPushSubscribe,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PushSubscribeRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn push_get_subscription_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerPushGetSubscription,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PushGetSubscriptionRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn push_unsubscribe_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerPushUnsubscribe,
        source_host: SharedRendererServiceWorkerHost,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::PushUnsubscribeRequested {
                request,
                source_host,
            },
        }
    }

    pub(super) fn close_notification_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        request: ServiceWorkerCloseNotification,
        owner: ServiceWorkerRunOwner,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::CloseNotificationRequested { request, owner },
        }
    }

    pub(super) fn client_message(
        runtime_service: WeakServiceWorkerRuntimeService,
        message: ServiceWorkerClientMessage,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientMessage { message },
        }
    }

    pub(super) fn worker_message(
        runtime_service: WeakServiceWorkerRuntimeService,
        message: ServiceWorkerWorkerMessage,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::WorkerMessage { message },
        }
    }

    pub(super) fn client_query(
        runtime_service: WeakServiceWorkerRuntimeService,
        query: ServiceWorkerClientQuery,
        owner: ServiceWorkerRunOwner,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientQuery { query, owner },
        }
    }

    pub(super) fn client_navigate(
        runtime_service: WeakServiceWorkerRuntimeService,
        navigate: ServiceWorkerClientNavigate,
        owner: ServiceWorkerRunOwner,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientNavigate { navigate, owner },
        }
    }

    pub(super) fn client_navigate_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: crate::types::ServiceWorkerClientNavigateCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientNavigateCompleted { completion },
        }
    }

    pub(super) fn client_focus(
        runtime_service: WeakServiceWorkerRuntimeService,
        focus: ServiceWorkerClientFocus,
        owner: ServiceWorkerRunOwner,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientFocus { focus, owner },
        }
    }

    pub(super) fn client_focus_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: crate::types::ServiceWorkerClientFocusCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientFocusCompleted { completion },
        }
    }

    pub(super) fn clients_open_window(
        runtime_service: WeakServiceWorkerRuntimeService,
        open_window: ServiceWorkerClientsOpenWindow,
        owner: ServiceWorkerRunOwner,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientsOpenWindow { open_window, owner },
        }
    }

    pub(super) fn clients_open_window_completed(
        runtime_service: WeakServiceWorkerRuntimeService,
        completion: crate::types::ServiceWorkerClientsOpenWindowCompletion,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientsOpenWindowCompleted { completion },
        }
    }

    pub(super) fn idle_timeout(
        runtime_service: WeakServiceWorkerRuntimeService,
        timeout: ServiceWorkerIdleTimeout,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::IdleTimeout(timeout),
        }
    }

    pub(super) fn skip_waiting_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::SkipWaitingRequested {
                registration_id,
                version_id,
            },
        }
    }

    pub(super) fn clients_claim_requested(
        runtime_service: WeakServiceWorkerRuntimeService,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Self {
        Self {
            runtime_service,
            kind: ServiceWorkerRuntimeCompletionKind::ClientsClaimRequested {
                registration_id,
                version_id,
            },
        }
    }

    pub(super) fn complete(self) {
        match self.kind {
            ServiceWorkerRuntimeCompletionKind::TargetOutput { owner, output } => {
                if let Some(service) = self.runtime_service.upgrade() {
                    service.finish_target_output(owner, output);
                }
            }
            ServiceWorkerRuntimeCompletionKind::VersionStartCompleted {
                owner,
                final_script_url,
                script_resource,
                fetch_handler_type,
            } => {
                let (version_id, run) = owner.into_parts();
                self.runtime_service.finish_worker_start_completed(
                    version_id,
                    run,
                    final_script_url,
                    Some(script_resource),
                    fetch_handler_type,
                )
            }
            ServiceWorkerRuntimeCompletionKind::VersionStartFailed { owner, failure } => {
                let (version_id, run) = owner.into_parts();
                self.runtime_service
                    .finish_worker_start_failed(version_id, run, failure)
            }
            ServiceWorkerRuntimeCompletionKind::ImportedScriptLoaded {
                registration_id,
                owner,
                resource,
            } => {
                let (version_id, run) = owner.into_parts();
                self.runtime_service.finish_imported_script_loaded(
                    registration_id,
                    version_id,
                    run,
                    resource,
                )
            }
            ServiceWorkerRuntimeCompletionKind::MainScriptUpdateCheckCompleted {
                registration_id,
                result,
            } => self
                .runtime_service
                .finish_main_script_update_check_completed(registration_id, result),
            ServiceWorkerRuntimeCompletionKind::LifecycleEventCompleted { completion } => {
                self.runtime_service
                    .finish_lifecycle_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::FetchEventCompleted { completion } => {
                self.runtime_service
                    .finish_fetch_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::FetchStreamStarted { started } => {
                self.runtime_service.finish_fetch_stream_started(started);
            }
            ServiceWorkerRuntimeCompletionKind::FetchStreamChunk { chunk } => {
                self.runtime_service.finish_fetch_stream_chunk(chunk);
            }
            ServiceWorkerRuntimeCompletionKind::MessageEventCompleted { completion } => {
                self.runtime_service
                    .finish_message_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::NotificationEventCompleted { completion } => {
                self.runtime_service
                    .finish_notification_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::PushEventCompleted { completion } => {
                self.runtime_service.finish_push_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::SyncEventCompleted { completion } => {
                self.runtime_service.finish_sync_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncEventCompleted { completion } => {
                self.runtime_service
                    .finish_periodic_sync_event_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::ShowNotificationRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_show_notification_requested(*request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::GetNotificationsRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_get_notifications_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::SyncRegistrationRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_sync_registration_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::SyncGetTagsRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_sync_get_tags_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncRegistrationRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_periodic_sync_registration_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncGetTagsRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_periodic_sync_get_tags_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncUnregistrationRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_periodic_sync_unregistration_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::PushSubscribeRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_push_subscribe_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::PushGetSubscriptionRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service.finish_push_get_subscription_requested(
                    request,
                    run,
                    source_host,
                );
            }
            ServiceWorkerRuntimeCompletionKind::PushUnsubscribeRequested {
                request,
                source_host,
            } => {
                let run = source_host.run_identity();
                self.runtime_service
                    .finish_push_unsubscribe_requested(request, run, source_host);
            }
            ServiceWorkerRuntimeCompletionKind::CloseNotificationRequested { request, owner } => {
                if owner.version_id() != request.version_id {
                    return;
                }
                self.runtime_service
                    .finish_close_notification_requested(request, owner.cloned_run_identity());
            }
            ServiceWorkerRuntimeCompletionKind::ClientMessage { message } => {
                self.runtime_service.finish_client_message(message);
            }
            ServiceWorkerRuntimeCompletionKind::WorkerMessage { message } => {
                self.runtime_service.finish_worker_message(message);
            }
            ServiceWorkerRuntimeCompletionKind::ClientQuery { query, owner } => {
                if owner.version_id() != query.version_id {
                    return;
                }
                self.runtime_service
                    .finish_client_query_requested(query, owner.cloned_run_identity());
            }
            ServiceWorkerRuntimeCompletionKind::ClientNavigate { navigate, owner } => {
                if owner.version_id() != navigate.source_version_id {
                    return;
                }
                self.runtime_service
                    .finish_client_navigate_requested(navigate, owner.cloned_run_identity());
            }
            ServiceWorkerRuntimeCompletionKind::ClientNavigateCompleted { completion } => {
                self.runtime_service
                    .finish_client_navigate_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::ClientFocus { focus, owner } => {
                if owner.version_id() != focus.source_version_id {
                    return;
                }
                self.runtime_service
                    .finish_client_focus_requested(focus, owner.cloned_run_identity());
            }
            ServiceWorkerRuntimeCompletionKind::ClientFocusCompleted { completion } => {
                self.runtime_service
                    .finish_client_focus_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::ClientsOpenWindow { open_window, owner } => {
                if owner.version_id() != open_window.source_version_id {
                    return;
                }
                self.runtime_service
                    .finish_clients_open_window_requested(open_window, owner.cloned_run_identity());
            }
            ServiceWorkerRuntimeCompletionKind::ClientsOpenWindowCompleted { completion } => {
                self.runtime_service
                    .finish_clients_open_window_completed(completion);
            }
            ServiceWorkerRuntimeCompletionKind::IdleTimeout(timeout) => {
                self.runtime_service.finish_worker_idle_timeout(timeout)
            }
            ServiceWorkerRuntimeCompletionKind::SkipWaitingRequested {
                registration_id,
                version_id,
            } => self
                .runtime_service
                .finish_worker_skip_waiting_requested(registration_id, version_id),
            ServiceWorkerRuntimeCompletionKind::ClientsClaimRequested {
                registration_id,
                version_id,
            } => self
                .runtime_service
                .finish_worker_clients_claim_requested(registration_id, version_id),
        }
    }
}

impl fmt::Debug for ServiceWorkerRuntimeCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ServiceWorkerRuntimeCompletionKind::TargetOutput { owner, .. } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::TargetOutput")
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::VersionStartCompleted { owner, .. } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::VersionStartCompleted")
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::VersionStartFailed { owner, .. } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::VersionStartFailed")
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ImportedScriptLoaded {
                registration_id,
                owner,
                ..
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ImportedScriptLoaded")
                .field("registration_id", registration_id)
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::MainScriptUpdateCheckCompleted {
                registration_id,
                result,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::MainScriptUpdateCheckCompleted")
                .field("registration_id", registration_id)
                .field("result_is_ok", &result.is_ok())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::LifecycleEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::LifecycleEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .field("kind", &completion.kind)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::FetchEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::FetchEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::FetchStreamStarted { started } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::FetchStreamStarted")
                .field("event_id", &started.event_id)
                .field("owner", &started.owner)
                .field("body_source_id", &started.body_source_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::FetchStreamChunk { chunk } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::FetchStreamChunk")
                .field("event_id", &chunk.event_id)
                .field("body_source_id", &chunk.body_source_id)
                .field("bytes_len", &chunk.bytes.len())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::MessageEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::MessageEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::NotificationEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::NotificationEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PushEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PushEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::SyncEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::SyncEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .field("tag", &completion.tag)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncEventCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PeriodicSyncEventCompleted")
                .field("event_id", &completion.event_id)
                .field("owner", &completion.owner)
                .field("tag", &completion.tag)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ShowNotificationRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ShowNotificationRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::GetNotificationsRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::GetNotificationsRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::SyncRegistrationRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::SyncRegistrationRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .field("tag", &request.tag)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::SyncGetTagsRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::SyncGetTagsRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncRegistrationRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PeriodicSyncRegistrationRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .field("tag", &request.tag)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncGetTagsRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PeriodicSyncGetTagsRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PeriodicSyncUnregistrationRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PeriodicSyncUnregistrationRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .field("tag", &request.tag)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::CloseNotificationRequested { request, owner } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::CloseNotificationRequested")
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("notification_id", &request.notification_id)
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientMessage { message } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientMessage")
                .field("source_version_id", &message.source_version_id)
                .field("target_client_id", &message.target_client_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::WorkerMessage { message } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::WorkerMessage")
                .field("source_version_id", &message.source_version_id)
                .field("target_version_id", &message.target_version_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientQuery { query, owner } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientQuery")
                .field("request_id", &query.request_id)
                .field("registration_id", &query.registration_id)
                .field("version_id", &query.version_id)
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientNavigate { navigate, owner } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientNavigate")
                .field("request_id", &navigate.request_id)
                .field("source_version_id", &navigate.source_version_id)
                .field("owner", owner)
                .field("target_client_id", &navigate.target_client_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientNavigateCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientNavigateCompleted")
                .field("request_id", &completion.request_id)
                .field("source_version_id", &completion.source_version_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientFocus { focus, owner } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientFocus")
                .field("request_id", &focus.request_id)
                .field("source_version_id", &focus.source_version_id)
                .field("owner", owner)
                .field("target_client_id", &focus.target_client_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientFocusCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientFocusCompleted")
                .field("request_id", &completion.request_id)
                .field("source_version_id", &completion.source_version_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientsOpenWindow { open_window, owner } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientsOpenWindow")
                .field("request_id", &open_window.request_id)
                .field("source_version_id", &open_window.source_version_id)
                .field("owner", owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::ClientsOpenWindowCompleted { completion } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientsOpenWindowCompleted")
                .field("request_id", &completion.request_id)
                .field("source_version_id", &completion.source_version_id)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PushSubscribeRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PushSubscribeRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PushGetSubscriptionRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PushGetSubscriptionRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::PushUnsubscribeRequested {
                request,
                source_host,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::PushUnsubscribeRequested")
                .field("request_id", &request.request_id)
                .field("registration_id", &request.registration_id)
                .field("version_id", &request.version_id)
                .field("owner", &source_host.run_owner())
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::IdleTimeout(timeout) => f
                .debug_struct("ServiceWorkerRuntimeCompletion::IdleTimeout")
                .field("owner", &timeout.owner)
                .finish_non_exhaustive(),
            ServiceWorkerRuntimeCompletionKind::SkipWaitingRequested {
                registration_id,
                version_id,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::SkipWaitingRequested")
                .field("registration_id", registration_id)
                .field("version_id", version_id)
                .finish(),
            ServiceWorkerRuntimeCompletionKind::ClientsClaimRequested {
                registration_id,
                version_id,
            } => f
                .debug_struct("ServiceWorkerRuntimeCompletion::ClientsClaimRequested")
                .field("registration_id", registration_id)
                .field("version_id", version_id)
                .finish(),
        }
    }
}
