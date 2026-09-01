use std::sync::Arc;

use moli_storage_key::MoliStorageKey;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::{
    runtime::{
        RendererRuntimeInspectorMessage, RendererServiceWorkerRunIdentity, ServiceWorkerFetchEvent,
        ServiceWorkerLifecycleEvent, ServiceWorkerMessageEvent,
        ServiceWorkerNavigationPreloadFailure, ServiceWorkerNavigationPreloadResponseStarted,
        ServiceWorkerNavigationPreloadStreamChunk, ServiceWorkerNavigationPreloadStreamFinished,
        ServiceWorkerNotificationEvent, ServiceWorkerPeriodicSyncEvent, ServiceWorkerPushEvent,
        ServiceWorkerSyncEvent,
    },
    types::{NetworkBodySourceId, SubresourcePolicyContext},
    worker::{
        WorkerBootstrapCompletion, WorkerBootstrapSuccess, WorkerFetchHandlerType, WorkerHandle,
        WorkerSpawnOptions, WorkerToParentMessage,
    },
};

use super::{
    ids::ServiceWorkerVersionId,
    jobs::ServiceWorkerLaunchParams,
    run_owner::ServiceWorkerRunOwner,
    script_loading::{
        LoadedServiceWorkerScript, ServiceWorkerScriptResource, load_service_worker_script_source,
    },
    service::ServiceWorkerRuntimeService,
    version::{ServiceWorkerFetchHandlerType, ServiceWorkerVersionStartFailure},
};

pub(super) type SharedRendererServiceWorkerHost = Arc<RendererServiceWorkerHost>;

pub(super) struct RendererServiceWorkerHost {
    run_owner: ServiceWorkerRunOwner,
    state: Mutex<RendererServiceWorkerHostState>,
}

enum RendererServiceWorkerHostState {
    Loading,
    Running { handle: Option<WorkerHandle> },
    Failed,
    Closed,
}

impl RendererServiceWorkerHost {
    pub(super) fn new_loading(
        run_owner: &ServiceWorkerRunOwner,
    ) -> SharedRendererServiceWorkerHost {
        Arc::new(Self {
            run_owner: run_owner.clone(),
            state: Mutex::new(RendererServiceWorkerHostState::Loading),
        })
    }

    #[cfg(test)]
    pub(super) fn new_running_without_handle_for_test(
        run_owner: &ServiceWorkerRunOwner,
    ) -> SharedRendererServiceWorkerHost {
        Arc::new(Self {
            run_owner: run_owner.clone(),
            state: Mutex::new(RendererServiceWorkerHostState::Running { handle: None }),
        })
    }

    #[cfg(test)]
    pub(super) fn new_running_with_handle_for_test(
        run_owner: &ServiceWorkerRunOwner,
        handle: WorkerHandle,
    ) -> SharedRendererServiceWorkerHost {
        Arc::new(Self {
            run_owner: run_owner.clone(),
            state: Mutex::new(RendererServiceWorkerHostState::Running {
                handle: Some(handle),
            }),
        })
    }

    pub(super) fn start_loading(
        self: &Arc<Self>,
        service: ServiceWorkerRuntimeService,
        params: ServiceWorkerLaunchParams,
        preloaded_script: Option<LoadedServiceWorkerScript>,
    ) {
        assert_eq!(
            self.run_owner, params.run_owner,
            "a ServiceWorker host must start only its bound run owner"
        );
        let run_owner = params.run_owner.clone();
        let host_for_task = Arc::clone(self);
        let service_for_task = service.clone();
        let _ = std::thread::Builder::new()
            .name(format!(
                "service-worker-load-{}",
                params.run_owner.version_id().as_u64()
            ))
            .spawn(move || {
                let result = match preloaded_script {
                    Some(script) => Ok(script),
                    None => load_service_worker_script_source(&params),
                };
                host_for_task.finish_loading(service_for_task, params, result);
            })
            .map_err(|error| {
                self.mark_failed();
                service.enqueue_worker_start_failed(
                    run_owner,
                    ServiceWorkerVersionStartFailure::HostThreadSpawn {
                        message: error.to_string(),
                    },
                );
            });
    }

    pub(super) fn version_id(&self) -> ServiceWorkerVersionId {
        self.run_owner.version_id()
    }

    /// Exact identity of this concrete V8 worker run.
    pub(super) fn run_identity(&self) -> RendererServiceWorkerRunIdentity {
        self.run_owner.cloned_run_identity()
    }

    pub(super) fn run_owner(&self) -> ServiceWorkerRunOwner {
        self.run_owner.clone()
    }

    pub(super) fn has_running_worker(&self) -> bool {
        matches!(
            *self.state.lock(),
            RendererServiceWorkerHostState::Running { .. }
        )
    }

    pub(super) async fn dispatch_worker_runtime_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<crate::runtime::RendererRuntimeInspectorResponseSender>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let dispatched = {
            let state = self.state.lock();
            let RendererServiceWorkerHostState::Running {
                handle: Some(handle),
            } = &*state
            else {
                return Err("ServiceWorkerRuntimeUnavailable".to_owned());
            };
            handle.dispatch_runtime_protocol_message(
                inspector_session_id,
                raw_json,
                deferred_response,
                response_tx,
            )
        };
        if !dispatched {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        }
        response_rx
            .await
            .map_err(|_| "ServiceWorkerRuntimeUnavailable".to_owned())?
    }

    pub(super) async fn dispatch_worker_runtime_protocol_message_with_deferred_response(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: crate::runtime::RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.dispatch_worker_runtime_protocol_message(
            inspector_session_id,
            raw_json,
            Some(deferred_response),
        )
        .await
    }

    pub(super) async fn dispatch_worker_runtime_protocol_message_without_deferred_response(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.dispatch_worker_runtime_protocol_message(inspector_session_id, raw_json, None)
            .await
    }

    pub(super) fn detach_worker_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.detach_runtime_inspector_session(inspector_session_id)
    }

    pub(super) fn run_if_waiting_for_debugger_for_devtools(&self) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.run_if_waiting_for_debugger_for_devtools()
    }

    pub(super) fn dispatch_lifecycle_event(&self, event: ServiceWorkerLifecycleEvent) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_lifecycle_event(event);
        true
    }

    pub(super) fn dispatch_fetch_event(&self, event: ServiceWorkerFetchEvent) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_fetch_event(event);
        true
    }

    pub(super) fn start_navigation_preload_response(
        &self,
        started: ServiceWorkerNavigationPreloadResponseStarted,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.start_service_worker_navigation_preload_response(started);
        true
    }

    pub(super) fn enqueue_navigation_preload_chunk(
        &self,
        chunk: ServiceWorkerNavigationPreloadStreamChunk,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.enqueue_service_worker_navigation_preload_chunk(chunk);
        true
    }

    pub(super) fn finish_navigation_preload_stream(
        &self,
        finished: ServiceWorkerNavigationPreloadStreamFinished,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.finish_service_worker_navigation_preload_stream(finished);
        true
    }

    pub(super) fn fail_navigation_preload(
        &self,
        failure: ServiceWorkerNavigationPreloadFailure,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.fail_service_worker_navigation_preload(failure);
        true
    }

    pub(super) fn cancel_fetch_stream(
        &self,
        event_id: crate::runtime::ServiceWorkerEventId,
        body_source_id: NetworkBodySourceId,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.cancel_service_worker_fetch_stream(event_id, body_source_id);
        true
    }

    pub(super) fn abort_fetch_event_request_signal(
        &self,
        event_id: crate::runtime::ServiceWorkerEventId,
        reason: Option<crate::structured_clone::V8StructuredClonePayload>,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.abort_service_worker_fetch_request_signal(event_id, reason);
        true
    }

    pub(super) fn dispatch_message_event(&self, event: ServiceWorkerMessageEvent) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_message_event(event);
        true
    }

    pub(super) fn dispatch_notification_event(
        &self,
        event: ServiceWorkerNotificationEvent,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_notification_event(event);
        true
    }

    pub(super) fn dispatch_push_event(&self, event: ServiceWorkerPushEvent) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_push_event(event);
        true
    }

    pub(super) fn dispatch_sync_event(&self, event: ServiceWorkerSyncEvent) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_sync_event(event);
        true
    }

    pub(super) fn dispatch_periodic_sync_event(
        &self,
        event: ServiceWorkerPeriodicSyncEvent,
    ) -> bool {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return false;
        };
        handle.dispatch_service_worker_periodic_sync_event(event);
        true
    }

    pub(super) fn dispatch_client_query_result(
        &self,
        result: crate::runtime::ServiceWorkerClientQueryResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_client_query_result(result);
    }

    pub(super) fn dispatch_client_navigate_result(
        &self,
        result: crate::runtime::ServiceWorkerClientNavigateResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_client_navigate_result(result);
    }

    pub(super) fn dispatch_client_focus_result(
        &self,
        result: crate::runtime::ServiceWorkerClientFocusResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_client_focus_result(result);
    }

    pub(super) fn dispatch_clients_open_window_result(
        &self,
        result: crate::runtime::ServiceWorkerClientsOpenWindowResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_clients_open_window_result(result);
    }

    pub(super) fn dispatch_get_notifications_result(
        &self,
        result: crate::runtime::ServiceWorkerGetNotificationsResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_get_notifications_result(result);
    }

    pub(super) fn dispatch_show_notification_result(
        &self,
        result: crate::runtime::ServiceWorkerShowNotificationResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_show_notification_result(result);
    }

    pub(super) fn dispatch_sync_registration_result(
        &self,
        result: crate::runtime::ServiceWorkerSyncRegistrationResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_sync_registration_result(result);
    }

    pub(super) fn dispatch_sync_get_tags_result(
        &self,
        result: crate::runtime::ServiceWorkerSyncGetTagsResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_sync_get_tags_result(result);
    }

    pub(super) fn dispatch_periodic_sync_registration_result(
        &self,
        result: crate::runtime::ServiceWorkerPeriodicSyncRegistrationResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_periodic_sync_registration_result(result);
    }

    pub(super) fn dispatch_periodic_sync_get_tags_result(
        &self,
        result: crate::runtime::ServiceWorkerPeriodicSyncGetTagsResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_periodic_sync_get_tags_result(result);
    }

    pub(super) fn dispatch_periodic_sync_unregistration_result(
        &self,
        result: crate::runtime::ServiceWorkerPeriodicSyncUnregistrationResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_periodic_sync_unregistration_result(result);
    }

    pub(super) fn dispatch_push_subscribe_result(
        &self,
        result: crate::runtime::ServiceWorkerPushSubscribeResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_push_subscribe_result(result);
    }

    pub(super) fn dispatch_push_get_subscription_result(
        &self,
        result: crate::runtime::ServiceWorkerPushGetSubscriptionResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_push_get_subscription_result(result);
    }

    pub(super) fn dispatch_push_unsubscribe_result(
        &self,
        result: crate::runtime::ServiceWorkerPushUnsubscribeResult,
    ) {
        let state = self.state.lock();
        let RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        } = &*state
        else {
            return;
        };
        handle.dispatch_service_worker_push_unsubscribe_result(result);
    }

    pub(super) fn terminate_without_join(&self) {
        let handle = {
            let mut state = self.state.lock();
            match &mut *state {
                RendererServiceWorkerHostState::Running { handle } => {
                    let handle = handle.take();
                    *state = RendererServiceWorkerHostState::Closed;
                    handle
                }
                RendererServiceWorkerHostState::Loading
                | RendererServiceWorkerHostState::Failed
                | RendererServiceWorkerHostState::Closed => {
                    *state = RendererServiceWorkerHostState::Closed;
                    None
                }
            }
        };
        if let Some(handle) = handle {
            handle.terminate();
        }
    }

    fn mark_failed(&self) {
        let mut state = self.state.lock();
        if matches!(*state, RendererServiceWorkerHostState::Loading) {
            *state = RendererServiceWorkerHostState::Failed;
        }
    }

    fn finish_loading(
        self: &Arc<Self>,
        service: ServiceWorkerRuntimeService,
        params: ServiceWorkerLaunchParams,
        result: Result<super::script_loading::LoadedServiceWorkerScript, String>,
    ) {
        let script = match result {
            Ok(script) => script,
            Err(error) => {
                tracing::warn!(
                    script_url = %params.script_url,
                    scope_url = %params.scope_url,
                    error = %error,
                    "failed to load service worker script"
                );
                self.mark_failed();
                service.enqueue_worker_start_failed(
                    params.run_owner.clone(),
                    ServiceWorkerVersionStartFailure::ScriptLoad { message: error },
                );
                return;
            }
        };
        let script_resource = script.resource.clone();
        let final_script_url = script_resource.final_url.to_string();
        if service.finish_worker_start_identical_script_update(
            params.run_owner.version_id(),
            params.run_owner.cloned_run_identity(),
            &script_resource,
        ) {
            self.mark_failed();
            return;
        }
        let (bootstrap_completion_tx, bootstrap_completion_rx) =
            mpsc::unbounded_channel::<WorkerBootstrapCompletion>();
        let mut handle = spawn_service_worker(
            service.clone(),
            params.clone(),
            script,
            bootstrap_completion_tx,
        );
        if let Some(receiver) = handle.take_receiver() {
            spawn_parent_message_pump(service.clone(), Arc::clone(self), receiver);
        }
        let mut state = self.state.lock();
        if !matches!(*state, RendererServiceWorkerHostState::Loading) {
            drop(state);
            handle.terminate();
            return;
        }
        *state = RendererServiceWorkerHostState::Running {
            handle: Some(handle),
        };
        drop(state);
        if service.take_devtools_evaluation_release_for_version(params.run_owner.version_id()) {
            self.run_if_waiting_for_debugger_for_devtools();
        }
        report_bootstrap_completion(
            service,
            params.run_owner,
            final_script_url,
            script_resource,
            bootstrap_completion_rx,
        );
    }
}

fn spawn_parent_message_pump(
    service: ServiceWorkerRuntimeService,
    source_host: SharedRendererServiceWorkerHost,
    mut receiver: mpsc::UnboundedReceiver<WorkerToParentMessage>,
) {
    let version_id = source_host.version_id();
    let source_run = source_host.run_identity();
    let _ = std::thread::Builder::new()
        .name(format!("service-worker-pump-{}", version_id.as_u64()))
        .spawn(move || {
            while let Some(message) = receiver.blocking_recv() {
                match message {
                    WorkerToParentMessage::ServiceWorkerLifecycleCompleted(completion) => {
                        service.enqueue_lifecycle_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => {
                        service.enqueue_fetch_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerFetchStreamStarted(started) => {
                        service.enqueue_fetch_stream_started(started);
                    }
                    WorkerToParentMessage::ServiceWorkerFetchStreamChunk(chunk) => {
                        service.enqueue_fetch_stream_chunk(chunk);
                    }
                    WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                        service.enqueue_message_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerNotificationCompleted(completion) => {
                        service.enqueue_notification_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerPushCompleted(completion) => {
                        service.enqueue_push_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerSyncCompleted(completion) => {
                        service.enqueue_sync_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(completion) => {
                        service.enqueue_periodic_sync_event_completed(completion);
                    }
                    WorkerToParentMessage::ServiceWorkerShowNotification(request) => {
                        service
                            .enqueue_show_notification_requested(request, Arc::clone(&source_host));
                    }
                    WorkerToParentMessage::ServiceWorkerGetNotifications(request) => {
                        service
                            .enqueue_get_notifications_requested(request, Arc::clone(&source_host));
                    }
                    WorkerToParentMessage::ServiceWorkerSyncRegistration(request) => {
                        service
                            .enqueue_sync_registration_requested(request, Arc::clone(&source_host));
                    }
                    WorkerToParentMessage::ServiceWorkerSyncGetTags(request) => {
                        service.enqueue_sync_get_tags_requested(request, Arc::clone(&source_host));
                    }
                    WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(request) => {
                        service.enqueue_periodic_sync_registration_requested(
                            request,
                            Arc::clone(&source_host),
                        );
                    }
                    WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(request) => {
                        service.enqueue_periodic_sync_get_tags_requested(
                            request,
                            Arc::clone(&source_host),
                        );
                    }
                    WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(request) => {
                        service.enqueue_periodic_sync_unregistration_requested(
                            request,
                            Arc::clone(&source_host),
                        );
                    }
                    WorkerToParentMessage::ServiceWorkerPushSubscribe(request) => {
                        service.enqueue_push_subscribe_requested(request, Arc::clone(&source_host));
                    }
                    WorkerToParentMessage::ServiceWorkerPushGetSubscription(request) => {
                        service.enqueue_push_get_subscription_requested(
                            request,
                            Arc::clone(&source_host),
                        );
                    }
                    WorkerToParentMessage::ServiceWorkerPushUnsubscribe(request) => {
                        service
                            .enqueue_push_unsubscribe_requested(request, Arc::clone(&source_host));
                    }
                    WorkerToParentMessage::ServiceWorkerCloseNotification(request) => {
                        service
                            .enqueue_close_notification_requested(request, source_host.run_owner());
                    }
                    WorkerToParentMessage::ServiceWorkerClientMessage(message) => {
                        service.enqueue_client_message(message);
                    }
                    WorkerToParentMessage::ServiceWorkerWorkerMessage(message) => {
                        service.enqueue_worker_message(message);
                    }
                    WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                        service.enqueue_client_query(query, source_host.run_owner());
                    }
                    WorkerToParentMessage::ServiceWorkerClientNavigate(navigate) => {
                        service.enqueue_client_navigate(navigate, source_host.run_owner());
                    }
                    WorkerToParentMessage::ServiceWorkerClientFocus(focus) => {
                        service.enqueue_client_focus(focus, source_host.run_owner());
                    }
                    WorkerToParentMessage::ServiceWorkerClientsOpenWindow(open_window) => {
                        service.enqueue_clients_open_window(open_window, source_host.run_owner());
                    }
                    WorkerToParentMessage::ServiceWorkerSkipWaiting {
                        registration_id,
                        version_id,
                    } => {
                        service.enqueue_skip_waiting_requested(registration_id, version_id);
                    }
                    WorkerToParentMessage::ServiceWorkerClientsClaim {
                        registration_id,
                        version_id,
                    } => {
                        service.enqueue_clients_claim_requested(registration_id, version_id);
                    }
                    WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
                        registration_id,
                        version_id,
                        resource,
                    } => {
                        if version_id != source_host.version_id() {
                            continue;
                        }
                        service.enqueue_imported_script_loaded(
                            registration_id,
                            source_host.run_owner(),
                            resource,
                        );
                    }
                    WorkerToParentMessage::Error {
                        message,
                        filename,
                        lineno,
                        colno,
                        event_kind,
                        phase,
                        source,
                    } => {
                        service.enqueue_target_exception_message(
                            version_id,
                            source_run.clone(),
                            message,
                            filename,
                            lineno,
                            colno,
                            event_kind,
                            phase,
                            source,
                        );
                    }
                    WorkerToParentMessage::Console(message) => {
                        service.enqueue_target_console_message(
                            version_id,
                            source_run.clone(),
                            message,
                        );
                    }
                    WorkerToParentMessage::RuntimeInspectorMessages(messages) => {
                        service.enqueue_target_runtime_inspector_messages(
                            version_id,
                            source_run.clone(),
                            messages,
                        );
                    }
                    WorkerToParentMessage::RuntimeInspectorResponse(publication) => {
                        let _ = publication.commit(None);
                    }
                    WorkerToParentMessage::Post(_)
                    | WorkerToParentMessage::SharedWorkerClosed
                    | WorkerToParentMessage::SubresourceNetwork(_)
                    | WorkerToParentMessage::PendingSubresourceFetch(_)
                    | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
                    | WorkerToParentMessage::SubresourceContinue(_)
                    | WorkerToParentMessage::WebSocketSubresource(_)
                    | WorkerToParentMessage::WebSocketLifecycle(_)
                    | WorkerToParentMessage::WebSocketFrame(_) => {}
                }
            }
        });
}

fn report_bootstrap_completion(
    service: ServiceWorkerRuntimeService,
    owner: ServiceWorkerRunOwner,
    final_script_url: String,
    script_resource: ServiceWorkerScriptResource,
    mut receiver: mpsc::UnboundedReceiver<WorkerBootstrapCompletion>,
) {
    match receiver.blocking_recv() {
        Some(WorkerBootstrapCompletion {
            result: Ok(success),
        }) => {
            service.enqueue_worker_start_completed(
                owner,
                final_script_url,
                script_resource,
                service_worker_fetch_handler_type(success),
            );
        }
        Some(WorkerBootstrapCompletion {
            result: Err(failure),
        }) => {
            service.enqueue_worker_start_failed(
                owner,
                ServiceWorkerVersionStartFailure::Bootstrap { failure },
            );
        }
        None => {
            service.enqueue_worker_start_failed(
                owner,
                ServiceWorkerVersionStartFailure::BootstrapChannelClosed,
            );
        }
    }
}

fn service_worker_fetch_handler_type(
    success: WorkerBootstrapSuccess,
) -> ServiceWorkerFetchHandlerType {
    match success.service_worker_fetch_handler_type {
        WorkerFetchHandlerType::NoHandler => ServiceWorkerFetchHandlerType::NoHandler,
        WorkerFetchHandlerType::NotSkippable => ServiceWorkerFetchHandlerType::NotSkippable,
        WorkerFetchHandlerType::EmptyFetchHandler => {
            ServiceWorkerFetchHandlerType::EmptyFetchHandler
        }
    }
}

fn spawn_service_worker(
    service: ServiceWorkerRuntimeService,
    params: ServiceWorkerLaunchParams,
    script: LoadedServiceWorkerScript,
    bootstrap_completion_tx: mpsc::UnboundedSender<WorkerBootstrapCompletion>,
) -> WorkerHandle {
    let storage_key = moli_storage_key::deserialize_serialized_storage_key(&params.storage_key)
        .unwrap_or_else(|| MoliStorageKey::first_party_from_url(&params.scope_url, None));
    let policy_context =
        service_worker_script_policy_context(&script.resource.final_url, &script.resource.headers);
    crate::worker::spawn_worker_with_options(
        WorkerSpawnOptions::new_with_request_client(
            script.source,
            script.resource.final_url.to_string(),
            params.request_client.clone(),
        )
        .with_script_kind(params.script_kind)
        .with_module_static_import_initiator_url(params.document_url.clone())
        .with_referrer_policy(script.response_referrer_policy)
        .with_content_security_policies(script.response_content_security_policies)
        .with_content_security_report_only_policies(
            script.response_content_security_report_only_policies,
        )
        .with_content_security_reporting_endpoints(
            script.response_content_security_reporting_endpoints,
        )
        .with_network_policy(params.network_policy)
        .with_policy_context(policy_context)
        .with_worker_context_runtime(params.worker_context_runtime)
        .with_service_worker_runtime(service)
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: params.registration_id,
            version_id: params.run_owner.version_id(),
            scope_url: params.scope_url.clone(),
        })
        .with_api_storage_key(Some(storage_key))
        .with_broadcast_channel_top_level_site(params.broadcast_channel_top_level_site)
        .with_indexed_db_manager(params.indexed_db_manager)
        .with_storage_bucket_store(params.storage_bucket_store)
        .with_bootstrap_completion_sender(bootstrap_completion_tx)
        .with_pause_evaluation_until_debugger(params.pause_evaluation_until_debugger),
    )
}

fn service_worker_script_policy_context(
    final_url: &url::Url,
    headers: &[(String, String)],
) -> SubresourcePolicyContext {
    SubresourcePolicyContext {
        cross_origin_embedder_policy:
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(headers),
        document_isolation_policy:
            crate::cross_origin_isolation::document_isolation_policy_from_headers(headers),
        cross_origin_isolated:
            crate::cross_origin_isolation::response_headers_enable_cross_origin_isolation(
                final_url, headers,
            ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_hosts_preserve_their_reserved_exact_run_identity() {
        let version_id = ServiceWorkerVersionId(7);
        let first_run = RendererServiceWorkerRunIdentity::fresh();
        let second_run = RendererServiceWorkerRunIdentity::fresh();
        let first_owner = ServiceWorkerRunOwner::new(version_id, first_run.clone());
        let second_owner = ServiceWorkerRunOwner::new(version_id, second_run.clone());
        let first = RendererServiceWorkerHost::new_loading(&first_owner);
        let second = RendererServiceWorkerHost::new_loading(&second_owner);

        assert_eq!(first.run_identity(), first_run);
        assert_eq!(second.run_identity(), second_run);
        assert_ne!(first.run_identity(), second.run_identity());
    }

    #[test]
    fn service_worker_script_policy_context_uses_response_headers() {
        let final_url = url::Url::parse("https://worker.test/service-worker.js")
            .expect("valid service worker url");
        let headers = vec![
            (
                "Cross-Origin-Embedder-Policy".to_owned(),
                "require-corp".to_owned(),
            ),
            (
                "Cross-Origin-Opener-Policy".to_owned(),
                "same-origin".to_owned(),
            ),
            (
                "Document-Isolation-Policy".to_owned(),
                "isolate-and-credentialless".to_owned(),
            ),
        ];

        let policy_context = service_worker_script_policy_context(&final_url, &headers);

        assert_eq!(
            policy_context.cross_origin_embedder_policy,
            crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp
        );
        assert_eq!(
            policy_context.document_isolation_policy,
            crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndCredentialless
        );
        assert!(policy_context.cross_origin_isolated);
    }

    #[test]
    fn service_worker_script_policy_context_requires_trustworthy_url_for_capability() {
        let final_url = url::Url::parse("http://worker.test/service-worker.js")
            .expect("valid service worker url");
        let headers = vec![
            (
                "Cross-Origin-Embedder-Policy".to_owned(),
                "require-corp".to_owned(),
            ),
            (
                "Cross-Origin-Opener-Policy".to_owned(),
                "same-origin".to_owned(),
            ),
            (
                "Document-Isolation-Policy".to_owned(),
                "isolate-and-require-corp".to_owned(),
            ),
        ];

        let policy_context = service_worker_script_policy_context(&final_url, &headers);

        assert_eq!(
            policy_context.cross_origin_embedder_policy,
            crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp
        );
        assert_eq!(
            policy_context.document_isolation_policy,
            crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp
        );
        assert!(!policy_context.cross_origin_isolated);
    }
}
