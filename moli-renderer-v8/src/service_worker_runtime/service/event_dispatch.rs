use super::*;
use crate::network::ResourceRequestClient;
use crate::network_host::new_network_body_source_id;
use crate::service_worker_runtime::{
    MaterializedServiceWorkerFetchResponseHead, ServiceWorkerFetchRequest,
    ServiceWorkerNavigationPreloadFailure, ServiceWorkerNavigationPreloadResponseStarted,
    ServiceWorkerNavigationPreloadStreamChunk, ServiceWorkerNavigationPreloadStreamFinished,
    ServiceWorkerRequestDestination,
};

const NAVIGATION_PRELOAD_CANCELLED_BEFORE_SETTLED_MESSAGE: &str = "The service worker navigation preload request was cancelled before 'preloadResponse' settled. \
     If you intend to use 'preloadResponse', use waitUntil() or respondWith() to wait for the \
     promise to settle.";
const NAVIGATION_PRELOAD_NETWORK_ERROR_MESSAGE: &str = "The service worker navigation preload request failed due to a network error. This may have \
     been an actual network error, or caused by the browser simulating offline to see if the page \
     works offline: see https://w3c.github.io/manifest/#installability-signals";

struct ServiceWorkerNavigationPreloadDispatch {
    event_id: ServiceWorkerEventId,
    version_id: ServiceWorkerVersionId,
    run: RendererServiceWorkerRunIdentity,
    request_url: url::Url,
    request_mode: moli_fetch::RequestMode,
    request_client: ResourceRequestClient,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    cancel_handle: moli_fetch::FetchCancelHandle,
    request: Result<moli_fetch::Request, String>,
}

fn service_worker_fetch_event_can_use_navigation_preload(job: &ServiceWorkerFetchJob) -> bool {
    job.request_mode == moli_fetch::RequestMode::Navigate
        && matches!(
            job.destination,
            ServiceWorkerRequestDestination::Document | ServiceWorkerRequestDestination::Iframe
        )
}

fn navigation_preload_request_for_job(
    job: &ServiceWorkerFetchJob,
    header_value: &str,
) -> Result<moli_fetch::Request, String> {
    let mut headers = job.request_headers.clone();
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("service-worker-navigation-preload"));
    headers.push((
        "Service-Worker-Navigation-Preload".to_owned(),
        header_value.to_owned(),
    ));
    moli_fetch::Request::new_bytes(
        &job.request_method,
        job.request_url.as_str(),
        job.request_body_bytes.clone(),
        headers,
    )
    .map(|request| {
        let request = if job.network_context.frame_id.is_some() {
            request.with_subframe_navigation_cookie_context()
        } else {
            request.with_top_level_navigation_cookie_context()
        };
        request
            .with_initiator_url(&job.network_context.document_url)
            .with_request_mode(job.request_mode)
            .with_credentials_mode(job.credentials_mode)
            // Chromium reports the first navigation preload redirect to
            // `preloadResponse` as an opaqueredirect response instead of
            // following it internally.
            .with_redirect_mode(moli_fetch::RequestRedirectMode::Manual)
            .with_fetch_priority_hint(job.priority)
            .with_browser_navigation_kind(if job.is_reload {
                moli_fetch::BrowserNavigationRequestKind::Reload
            } else {
                moli_fetch::BrowserNavigationRequestKind::Navigate
            })
            .with_page_network_policy()
    })
    .map_err(|error| error.to_string())
}

fn navigation_preload_response_head(
    head: moli_fetch::ResponseHead,
) -> MaterializedServiceWorkerFetchResponseHead {
    MaterializedServiceWorkerFetchResponseHead {
        final_url: Some(head.final_url),
        response_type: "default".to_owned(),
        redirected: head.redirected,
        status: head.status,
        headers: head.headers,
    }
}

fn navigation_preload_response_start_failure_message(
    cancel_handle: &moli_fetch::FetchCancelHandle,
) -> String {
    if cancel_handle.is_cancelled() {
        NAVIGATION_PRELOAD_CANCELLED_BEFORE_SETTLED_MESSAGE.to_owned()
    } else {
        NAVIGATION_PRELOAD_NETWORK_ERROR_MESSAGE.to_owned()
    }
}

async fn stream_navigation_preload_response(
    service: ServiceWorkerRuntimeService,
    host: SharedRendererServiceWorkerHost,
    dispatch: ServiceWorkerNavigationPreloadDispatch,
) {
    let cancel_handle = dispatch.cancel_handle.clone();
    let mut raw = match dispatch.request {
        Ok(request) => match dispatch
            .request_client
            .fetch_raw_stream_with_cancel(request, cancel_handle.clone())
            .await
        {
            Ok(raw) => raw,
            Err(_) => {
                let _ = host.fail_navigation_preload(ServiceWorkerNavigationPreloadFailure {
                    event_id: dispatch.event_id,
                    version_id: dispatch.version_id,
                    run: dispatch.run.clone(),
                    message: navigation_preload_response_start_failure_message(&cancel_handle),
                });
                return;
            }
        },
        Err(message) => {
            let _ = host.fail_navigation_preload(ServiceWorkerNavigationPreloadFailure {
                event_id: dispatch.event_id,
                version_id: dispatch.version_id,
                run: dispatch.run.clone(),
                message,
            });
            return;
        }
    };

    let body_source_id = new_network_body_source_id();
    let response_head = navigation_preload_response_head(raw.head());
    if !service.mark_navigation_preload_response_started(
        dispatch.event_id,
        dispatch.version_id,
        dispatch.run.clone(),
    ) {
        cancel_handle.cancel();
        return;
    }
    if !host.start_navigation_preload_response(ServiceWorkerNavigationPreloadResponseStarted {
        event_id: dispatch.event_id,
        version_id: dispatch.version_id,
        run: dispatch.run.clone(),
        request_url: dispatch.request_url,
        request_mode: dispatch.request_mode,
        body_source_id,
        response_head,
    }) {
        cancel_handle.cancel();
        return;
    }

    while let Some(bytes) = raw.next_chunk().await {
        if !host.enqueue_navigation_preload_chunk(ServiceWorkerNavigationPreloadStreamChunk {
            event_id: dispatch.event_id,
            body_source_id,
            bytes,
        }) {
            cancel_handle.cancel();
            return;
        }
    }

    let result = raw
        .finish()
        .await
        .map_err(|_| NAVIGATION_PRELOAD_NETWORK_ERROR_MESSAGE.to_owned());
    let _ = host.finish_navigation_preload_stream(ServiceWorkerNavigationPreloadStreamFinished {
        event_id: dispatch.event_id,
        version_id: dispatch.version_id,
        run: dispatch.run.clone(),
        body_source_id,
        result,
    });
}

impl ServiceWorkerRuntimeService {
    pub(super) fn start_queued_launch(&self, launch: ServiceWorkerQueuedLaunch) {
        let Some(launch) = self.defer_install_launch_until_debugger_if_needed(launch) else {
            return;
        };
        self.start_queued_launch_with_devtools_pause_policy(launch, false);
    }

    pub(super) fn start_queued_launch_without_devtools_pause(
        &self,
        launch: ServiceWorkerQueuedLaunch,
    ) {
        self.start_queued_launch_with_devtools_pause_policy(launch, true);
    }

    fn start_queued_launch_with_devtools_pause_policy(
        &self,
        mut launch: ServiceWorkerQueuedLaunch,
        debugger_release_consumed: bool,
    ) {
        self.apply_devtools_evaluation_pause_to_launch_if_needed(
            &mut launch,
            debugger_release_consumed,
        );
        for notification in launch.lifecycle_notifications {
            notification.send();
        }
        launch
            .host
            .start_loading(self.clone(), launch.params, launch.preloaded_script);
    }

    pub(super) fn apply_devtools_evaluation_pause_to_launch_if_needed(
        &self,
        launch: &mut ServiceWorkerQueuedLaunch,
        debugger_release_consumed: bool,
    ) {
        if debugger_release_consumed {
            return;
        }
        if self.take_devtools_evaluation_release_for_version(launch.params.version_id) {
            return;
        }
        if self.launch_should_pause_on_start_for_devtools(launch.params.version_id) {
            launch.params.pause_evaluation_until_debugger = true;
        }
    }

    fn launch_should_pause_on_start_for_devtools(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        let state = self.inner.state.lock();
        let Some(version) = state.versions.get(&version_id) else {
            return false;
        };
        version.should_pause_on_start_for_devtools
            && (version.lifecycle_state == ServiceWorkerVersionLifecycleState::Installing
                || state.devtools_attached_versions.contains(&version_id))
    }

    fn defer_install_launch_until_debugger_if_needed(
        &self,
        launch: ServiceWorkerQueuedLaunch,
    ) -> Option<ServiceWorkerQueuedLaunch> {
        if launch.preloaded_script.is_some() {
            return Some(launch);
        }
        let version_id = launch.params.version_id;
        let run = launch.params.run.clone();
        let mut state = self.inner.state.lock();
        let should_defer = state.versions.get(&version_id).is_some_and(|version| {
            version.should_pause_on_start_for_devtools
                && version.run == run
                && version.lifecycle_state == ServiceWorkerVersionLifecycleState::Installing
                && matches!(
                    version.running_state,
                    ServiceWorkerVersionRunningState::Starting { .. }
                )
        });
        if !should_defer || state.pending_devtools_launches.contains_key(&version_id) {
            return Some(launch);
        }
        state.pending_devtools_launches.insert(version_id, launch);
        None
    }

    pub(crate) fn dispatch_controlled_fetch(&self, dispatch: ServiceWorkerFetchDispatch) -> bool {
        let request = dispatch.request.clone();
        let fetch_job = ServiceWorkerFetchJob {
            internal_id: dispatch.internal_id,
            version_id: ServiceWorkerVersionId(0),
            run: None,
            request_url: request.url.clone(),
            request_method: request.method.clone(),
            request_headers: request.headers.clone(),
            request_body: dispatch.request_body_text,
            request_body_bytes: request.body.clone(),
            cors_preflight_request_headers: dispatch.cors_preflight_request_headers,
            client_id: request.client_id,
            resulting_client_id: request.resulting_client_id,
            destination: request.destination,
            is_reload: request.is_reload,
            metadata: request.metadata.clone(),
            request_mode: request.request_mode,
            credentials_mode: request.credentials_mode,
            redirect_mode: request.redirect_mode,
            priority: request.priority,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            request_cookie_report: dispatch.request_cookie_report,
            network_context: dispatch.network_context,
            completion_tx: dispatch.completion_tx,
            request_client: dispatch.request_client,
            resource_task_runner: dispatch.resource_task_runner,
            cancel_handle: dispatch.cancel_handle,
            navigation_preload_cancel_handle: None,
            streaming_body_source_id: None,
            direct_completion_tx: dispatch.direct_completion_tx,
        };
        self.dispatch_controlled_fetch_job(fetch_job, request)
            .is_ok()
    }

    pub(super) fn dispatch_controlled_fetch_job(
        &self,
        mut fetch_job: ServiceWorkerFetchJob,
        request: ServiceWorkerFetchRequest,
    ) -> Result<(), Box<ServiceWorkerFetchJob>> {
        let (dispatch_event, start_launch, fallback_job) = {
            let mut state = self.inner.state.lock();
            let (document_url, storage_key) = match state.live_clients.get(&request.client_id) {
                Some(client) => (client.document_url.clone(), client.storage_key.clone()),
                None => return Err(Box::new(fetch_job)),
            };
            let registration_match_url =
                if service_worker_fetch_request_is_navigation_destination(request.destination) {
                    &request.url
                } else {
                    &document_url
                };
            let Some(registration_id) = state
                .registrations
                .values()
                .filter(|registration| registration.active_version_id.is_some())
                .filter(|registration| {
                    registration
                        .controlled_client_ids
                        .contains(&request.client_id)
                })
                .filter(|registration| {
                    service_worker_registration_matches_url(
                        registration,
                        registration_match_url,
                        &storage_key,
                    )
                })
                .max_by_key(|registration| registration.scope_url.as_str().len())
                .map(|registration| registration.id)
            else {
                return Err(Box::new(fetch_job));
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return Err(Box::new(fetch_job));
            };
            let Some(version_id) = registration.active_version_id else {
                return Err(Box::new(fetch_job));
            };
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(version) = state.versions.get_mut(&version_id) else {
                return Err(Box::new(fetch_job));
            };
            fetch_job.version_id = version_id;
            fetch_job.bind_to_run(version.run.clone());
            if version.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating {
                let event_id =
                    ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
                let event = ServiceWorkerFetchEvent {
                    event_id,
                    version_id,
                    run: version.run.clone(),
                    request,
                    navigation_preload_sent: false,
                };
                version.pending_activation_fetch_events.push_back(event);
                state.pending_fetch_jobs.insert(event_id, fetch_job);
                (None, None, None)
            } else if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return Err(Box::new(fetch_job));
            } else if version.fetch_handler_existence != ServiceWorkerFetchHandlerExistence::Unknown
                && version.fetch_handler_type.allows_fetch_event_skip()
            {
                (None, None, Some(fetch_job))
            } else {
                let event_id =
                    ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
                Self::begin_version_event_locked(version);
                let action = match &version.running_state {
                    ServiceWorkerVersionRunningState::Running { host } => {
                        let event = ServiceWorkerFetchEvent {
                            event_id,
                            version_id,
                            run: version.run.clone(),
                            request,
                            navigation_preload_sent: false,
                        };
                        (Some((host.clone(), event)), None, None)
                    }
                    ServiceWorkerVersionRunningState::Starting { .. } => {
                        version.pending_start_events.push_back(
                            ServiceWorkerPendingStartEvent::Fetch(ServiceWorkerFetchEvent {
                                event_id,
                                version_id,
                                run: version.run.clone(),
                                request,
                                navigation_preload_sent: false,
                            }),
                        );
                        (None, None, None)
                    }
                    ServiceWorkerVersionRunningState::Stopped => {
                        version.run = RendererServiceWorkerRunIdentity::fresh();
                        version.last_start_error = None;
                        let run = version.run.clone();
                        fetch_job.bind_to_run(run.clone());
                        let host = RendererServiceWorkerHost::new_loading(version_id, &run);
                        version.launch_config.document_url = document_url.clone();
                        let params = version.launch_config.to_launch_params(
                            registration_id,
                            version_id,
                            &run,
                            version.script_url.clone(),
                            scope_url,
                            registration_storage_key,
                            version.script_kind,
                        );
                        let event = ServiceWorkerFetchEvent {
                            event_id,
                            version_id,
                            run: run.clone(),
                            request,
                            navigation_preload_sent: false,
                        };
                        version
                            .pending_start_events
                            .push_back(ServiceWorkerPendingStartEvent::Fetch(event));
                        version.running_state =
                            ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                        (
                            None,
                            Some(ServiceWorkerQueuedLaunch {
                                params,
                                host,
                                lifecycle_notifications: Vec::new(),
                                preloaded_script: None,
                            }),
                            None,
                        )
                    }
                };
                state.pending_fetch_jobs.insert(event_id, fetch_job);
                action
            }
        };

        if let Some(job) = fallback_job {
            self.dispatch_fetch_fallback(job);
            return Ok(());
        }
        if let Some(launch) = start_launch {
            self.start_queued_launch(launch);
        }
        let Some((host, event)) = dispatch_event else {
            return Ok(());
        };
        self.dispatch_fetch_event(host, event);
        Ok(())
    }

    pub(super) fn dispatch_pending_activation_fetch_events(
        &self,
        events: Vec<ServiceWorkerFetchEvent>,
    ) {
        for event in events {
            enum PendingActivationFetchAction {
                Dispatch(SharedRendererServiceWorkerHost),
                Fallback,
                Failure(String),
            }

            let action = {
                let mut state = self.inner.state.lock();
                match state.versions.get_mut(&event.version_id) {
                    Some(version) if version.run != event.run => {
                        PendingActivationFetchAction::Failure(
                            "service worker fetch dispatch failed: stale activating worker"
                                .to_owned(),
                        )
                    }
                    Some(version)
                        if version.lifecycle_state
                            != ServiceWorkerVersionLifecycleState::Activated =>
                    {
                        PendingActivationFetchAction::Failure(
                            "service worker fetch dispatch failed: worker activation did not complete"
                                .to_owned(),
                        )
                    }
                    Some(version)
                        if version.fetch_handler_existence
                            != ServiceWorkerFetchHandlerExistence::Unknown
                            && version.fetch_handler_type.allows_fetch_event_skip() =>
                    {
                        PendingActivationFetchAction::Fallback
                    }
                    Some(version) => match &version.running_state {
                        ServiceWorkerVersionRunningState::Running { host } => {
                            let host = host.clone();
                            Self::begin_version_event_locked(version);
                            PendingActivationFetchAction::Dispatch(host)
                        }
                        ServiceWorkerVersionRunningState::Stopped
                        | ServiceWorkerVersionRunningState::Starting { .. } => {
                            PendingActivationFetchAction::Failure(
                                "service worker fetch dispatch failed: activated worker is not running"
                                    .to_owned(),
                            )
                        }
                    },
                    None => PendingActivationFetchAction::Failure(
                        "service worker fetch dispatch failed: activating worker was removed"
                            .to_owned(),
                    ),
                }
            };

            match action {
                PendingActivationFetchAction::Dispatch(host) => {
                    self.dispatch_fetch_event(host, event);
                }
                PendingActivationFetchAction::Fallback => {
                    self.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        result: ServiceWorkerFetchResult::Fallback,
                    });
                }
                PendingActivationFetchAction::Failure(message) => {
                    self.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        result: ServiceWorkerFetchResult::Failure(message),
                    });
                }
            }
        }
    }

    pub(crate) fn dispatch_message_to_version(
        &self,
        version_id: ServiceWorkerVersionId,
        source_client_id: ServiceWorkerClientId,
        source_origin: Option<String>,
        payload: V8StructuredClonePayload,
    ) -> bool {
        let start = {
            let mut state = self.inner.state.lock();
            let Some(source_client) = state.live_clients.get(&source_client_id) else {
                return false;
            };
            let source_client_url = source_client.document_url.clone();
            let source_origin = source_origin.unwrap_or_else(|| {
                moli_url::origin_ascii_serialization(&source_client.document_url)
            });
            let Some(registration_id) = state
                .versions
                .get(&version_id)
                .map(|version| version.registration_id)
            else {
                return false;
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            if !registration.references_version(version_id) {
                return false;
            }
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let source_client_snapshot =
                service_worker_client_snapshot(registration, source_client);
            let Some(version) = state.versions.get_mut(&version_id) else {
                return false;
            };
            if version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant {
                return false;
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerMessageEvent {
                event_id,
                version_id,
                run: version.run.clone(),
                source_client_id: Some(source_client_id),
                source_client_url: Some(source_client_url),
                source_client_snapshot: Some(source_client_snapshot),
                source_worker: None,
                source_origin,
                payload,
                window_interaction_allowed: false,
            };
            self.start_message_event_locked(
                &mut state,
                registration_id,
                scope_url,
                registration_storage_key,
                event,
            )
        };
        match start {
            ServiceWorkerMessageStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_message_event(host, event);
            }
            ServiceWorkerMessageStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerMessageStart::Queued | ServiceWorkerMessageStart::Dropped => {}
        }
        true
    }

    pub(crate) fn dispatch_notification_click_event(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        notification_id: u64,
        title: String,
        tag: String,
        metadata: ServiceWorkerNotificationMetadata,
        actions: Vec<ServiceWorkerNotificationAction>,
        action: String,
        data: V8StructuredClonePayload,
    ) -> bool {
        self.dispatch_notification_event(
            ServiceWorkerNotificationEventKind::Click,
            registration_id,
            notification_id,
            title,
            tag,
            metadata,
            actions,
            action,
            data,
        )
    }

    pub(crate) fn dispatch_push_for_scope(&self, scope_url: &Url, data: Option<Vec<u8>>) -> bool {
        let start = {
            let mut state = self.inner.state.lock();
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| {
                    registration.scope_url == *scope_url
                        && !registration.pending_unregistration
                        && registration.active_version_id.is_some()
                })
                .map(|registration| registration.id)
            else {
                return false;
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            let Some(version_id) = registration.active_version_id else {
                return false;
            };
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(version) = state.versions.get_mut(&version_id) else {
                return false;
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return false;
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerPushEvent {
                event_id,
                version_id,
                run: version.run.clone(),
                data,
            };
            self.start_push_event_locked(
                &mut state,
                registration_id,
                scope_url,
                registration_storage_key,
                event,
            )
        };
        match start {
            ServiceWorkerPushStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_push_event_to_host(host, event);
            }
            ServiceWorkerPushStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerPushStart::Queued | ServiceWorkerPushStart::Dropped => {}
        }
        true
    }

    pub(crate) fn register_sync_for_scope(&self, scope_url: &Url, tag: String) -> bool {
        let (start, accepted) = {
            let mut state = self.inner.state.lock();
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| {
                    registration.scope_url == *scope_url
                        && !registration.pending_unregistration
                        && registration.active_version_id.is_some()
                })
                .map(|registration| registration.id)
            else {
                return false;
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            let Some(version_id) = registration.active_version_id else {
                return false;
            };
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(version) = state.versions.get(&version_id) else {
                return false;
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return false;
            }
            let run = version.run.clone();
            let sync_key = (registration_id, tag.clone());
            if state
                .sync_registrations
                .get_mut(&sync_key)
                .is_some_and(|record| record.mark_refire_after_finish_if_active())
            {
                (ServiceWorkerSyncStart::Queued, true)
            } else {
                let event_id =
                    ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
                let event = ServiceWorkerSyncEvent {
                    event_id,
                    registration_id,
                    version_id,
                    run,
                    tag: tag.clone(),
                    last_chance: false,
                };
                let start = self.start_sync_event_locked(
                    &mut state,
                    registration_id,
                    scope_url,
                    registration_storage_key,
                    event,
                );
                let accepted = !matches!(start, ServiceWorkerSyncStart::Dropped);
                if accepted {
                    state
                        .sync_registrations
                        .entry(sync_key)
                        .and_modify(|record| {
                            record.failed_attempts = 0;
                            record.mark_active(event_id);
                        })
                        .or_insert_with(|| ServiceWorkerSyncRegistrationRecord::active(event_id));
                }
                (start, accepted)
            }
        };
        match start {
            ServiceWorkerSyncStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_sync_event_to_host(host, event);
            }
            ServiceWorkerSyncStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerSyncStart::Queued | ServiceWorkerSyncStart::Dropped => {}
        }
        accepted
    }

    pub(crate) fn retry_sync_for_scope(&self, scope_url: &Url, tag: &str) -> bool {
        let (start, accepted) = {
            let mut state = self.inner.state.lock();
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| {
                    registration.scope_url == *scope_url
                        && !registration.pending_unregistration
                        && registration.active_version_id.is_some()
                })
                .map(|registration| registration.id)
            else {
                return false;
            };
            let sync_key = (registration_id, tag.to_owned());
            let Some(sync_record) = state.sync_registrations.get(&sync_key) else {
                return false;
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            let Some(version_id) = registration.active_version_id else {
                return false;
            };
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(version) = state.versions.get(&version_id) else {
                return false;
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return false;
            }
            if !sync_record.is_idle() {
                return false;
            }
            let run = version.run.clone();
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerSyncEvent {
                event_id,
                registration_id,
                version_id,
                run,
                tag: tag.to_owned(),
                last_chance: sync_record.failed_attempts > 0,
            };
            let start = self.start_sync_event_locked(
                &mut state,
                registration_id,
                scope_url,
                registration_storage_key,
                event,
            );
            let accepted = !matches!(start, ServiceWorkerSyncStart::Dropped);
            if accepted && let Some(record) = state.sync_registrations.get_mut(&sync_key) {
                record.mark_active(event_id);
            }
            (start, accepted)
        };
        match start {
            ServiceWorkerSyncStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_sync_event_to_host(host, event);
            }
            ServiceWorkerSyncStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerSyncStart::Queued | ServiceWorkerSyncStart::Dropped => {}
        }
        accepted
    }

    pub(crate) fn sync_tags_for_scope(&self, scope_url: &Url) -> Vec<String> {
        let state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url && !registration.pending_unregistration
            })
            .map(|registration| registration.id)
        else {
            return Vec::new();
        };
        let mut tags: Vec<String> = state
            .sync_registrations
            .keys()
            .filter(|(id, _)| *id == registration_id)
            .map(|(_, tag)| tag.clone())
            .collect();
        tags.sort();
        tags
    }

    pub(crate) fn register_periodic_sync_for_scope(
        &self,
        scope_url: &Url,
        tag: String,
        min_interval_ms: u64,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(registration_id) = active_registration_id_for_scope_locked(&state, scope_url)
        else {
            return false;
        };
        state
            .periodic_sync_registrations
            .entry((registration_id, tag))
            .and_modify(|record| record.update_min_interval(min_interval_ms))
            .or_insert_with(|| ServiceWorkerPeriodicSyncRegistrationRecord::new(min_interval_ms));
        true
    }

    pub(crate) fn periodic_sync_tags_for_scope(&self, scope_url: &Url) -> Vec<String> {
        let state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url && !registration.pending_unregistration
            })
            .map(|registration| registration.id)
        else {
            return Vec::new();
        };
        let mut tags: Vec<String> = state
            .periodic_sync_registrations
            .keys()
            .filter(|(id, _)| *id == registration_id)
            .map(|(_, tag)| tag.clone())
            .collect();
        tags.sort();
        tags
    }

    pub(crate) fn unregister_periodic_sync_for_scope(&self, scope_url: &Url, tag: &str) -> bool {
        let mut state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url && !registration.pending_unregistration
            })
            .map(|registration| registration.id)
        else {
            return false;
        };
        state
            .periodic_sync_registrations
            .remove(&(registration_id, tag.to_owned()));
        true
    }

    pub(crate) fn dispatch_periodic_sync_for_scope(&self, scope_url: &Url, tag: &str) -> bool {
        let (start, accepted) = {
            let mut state = self.inner.state.lock();
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| {
                    registration.scope_url == *scope_url
                        && !registration.pending_unregistration
                        && registration.active_version_id.is_some()
                })
                .map(|registration| registration.id)
            else {
                return false;
            };
            let periodic_sync_key = (registration_id, tag.to_owned());
            if !state
                .periodic_sync_registrations
                .contains_key(&periodic_sync_key)
            {
                return false;
            }
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            let Some(version_id) = registration.active_version_id else {
                return false;
            };
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(version) = state.versions.get(&version_id) else {
                return false;
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return false;
            }
            let run = version.run.clone();
            if state
                .periodic_sync_registrations
                .get_mut(&periodic_sync_key)
                .is_some_and(|record| record.mark_refire_after_finish_if_active())
            {
                (ServiceWorkerPeriodicSyncStart::Queued, true)
            } else {
                let event_id =
                    ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
                let event = ServiceWorkerPeriodicSyncEvent {
                    event_id,
                    registration_id,
                    version_id,
                    run,
                    tag: tag.to_owned(),
                };
                let start = self.start_periodic_sync_event_locked(
                    &mut state,
                    registration_id,
                    scope_url,
                    registration_storage_key,
                    event,
                );
                let accepted = !matches!(start, ServiceWorkerPeriodicSyncStart::Dropped);
                if accepted
                    && let Some(record) = state
                        .periodic_sync_registrations
                        .get_mut(&periodic_sync_key)
                {
                    record.mark_active(event_id);
                }
                (start, accepted)
            }
        };
        match start {
            ServiceWorkerPeriodicSyncStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_periodic_sync_event_to_host(host, event);
            }
            ServiceWorkerPeriodicSyncStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerPeriodicSyncStart::Queued | ServiceWorkerPeriodicSyncStart::Dropped => {}
        }
        accepted
    }

    pub(crate) fn subscribe_push_for_scope(
        &self,
        scope_url: &Url,
        user_visible_only: bool,
    ) -> Option<ServiceWorkerPushSubscriptionSnapshot> {
        let mut state = self.inner.state.lock();
        let registration_id = active_registration_id_for_scope_locked(&state, scope_url)?;
        let snapshot =
            service_worker_push_subscription_snapshot(registration_id, user_visible_only);
        state
            .push_subscriptions
            .insert(registration_id, snapshot.clone());
        Some(snapshot)
    }

    pub(crate) fn push_subscription_for_scope(
        &self,
        scope_url: &Url,
    ) -> Option<ServiceWorkerPushSubscriptionSnapshot> {
        let state = self.inner.state.lock();
        let registration_id = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url && !registration.pending_unregistration
            })
            .map(|registration| registration.id)?;
        state.push_subscriptions.get(&registration_id).cloned()
    }

    pub(crate) fn unsubscribe_push_for_scope(&self, scope_url: &Url) -> bool {
        let mut state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url && !registration.pending_unregistration
            })
            .map(|registration| registration.id)
        else {
            return false;
        };
        state.push_subscriptions.remove(&registration_id).is_some()
    }

    pub(crate) fn show_notification_for_scope(
        &self,
        scope_url: &Url,
        title: String,
        tag: String,
        metadata: ServiceWorkerNotificationMetadata,
        actions: Vec<ServiceWorkerNotificationAction>,
        data: V8StructuredClonePayload,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url
                    && !registration.pending_unregistration
                    && registration.active_version_id.is_some()
            })
            .map(|registration| registration.id)
        else {
            return false;
        };
        self.record_notification_locked(
            &mut state,
            registration_id,
            title,
            tag,
            metadata,
            actions,
            data,
        );
        true
    }

    pub(crate) fn notifications_for_scope(
        &self,
        scope_url: &Url,
        tag: Option<&str>,
    ) -> Vec<ServiceWorkerNotificationSnapshot> {
        let state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url
                    && !registration.pending_unregistration
                    && registration.active_version_id.is_some()
            })
            .map(|registration| registration.id)
        else {
            return Vec::new();
        };
        service_worker_notifications_for_registration_locked(&state, registration_id, tag)
    }

    pub(crate) fn close_notification(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        notification_id: u64,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(position) = state.notification_records.iter().position(|record| {
            record.registration_id == registration_id && record.id == notification_id
        }) else {
            return false;
        };
        state.notification_records.remove(position);
        true
    }

    pub(super) fn record_notification_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        title: String,
        tag: String,
        mut metadata: ServiceWorkerNotificationMetadata,
        actions: Vec<ServiceWorkerNotificationAction>,
        data: V8StructuredClonePayload,
    ) {
        if metadata.timestamp.is_none() {
            metadata.timestamp = Some(current_epoch_millis());
        }
        let id = self
            .inner
            .next_notification_id
            .fetch_add(1, Ordering::Relaxed);
        let record = ServiceWorkerNotificationRecord {
            id,
            registration_id,
            title,
            tag,
            metadata,
            actions,
            data,
        };
        if !record.tag.is_empty()
            && let Some(existing) = state.notification_records.iter_mut().find(|existing| {
                existing.registration_id == registration_id && existing.tag == record.tag
            })
        {
            *existing = record;
        } else {
            state.notification_records.push(record);
        }
    }

    pub(crate) fn dispatch_notification_click_for_scope(
        &self,
        scope_url: &Url,
        title: String,
        action: String,
    ) -> bool {
        let record = {
            let state = self.inner.state.lock();
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| {
                    registration.scope_url == *scope_url
                        && !registration.pending_unregistration
                        && registration.active_version_id.is_some()
                })
                .map(|registration| registration.id)
            else {
                return false;
            };
            state
                .notification_records
                .iter()
                .filter(|record| record.registration_id == registration_id && record.title == title)
                .max_by_key(|record| record.id)
                .cloned()
        };
        let Some(record) = record else {
            return false;
        };
        if let Some(url) = record
            .actions
            .iter()
            .find(|candidate| candidate.action == action)
            .and_then(|action| action.navigate.clone())
        {
            return self.dispatch_notification_action_navigation(record.registration_id, url);
        }
        self.dispatch_notification_click_event(
            record.registration_id,
            record.id,
            record.title,
            record.tag,
            record.metadata,
            record.actions,
            action,
            record.data,
        )
    }

    pub(crate) fn dispatch_notification_close_for_scope(
        &self,
        scope_url: &Url,
        title: String,
    ) -> bool {
        let record = {
            let mut state = self.inner.state.lock();
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| {
                    registration.scope_url == *scope_url
                        && !registration.pending_unregistration
                        && registration.active_version_id.is_some()
                })
                .map(|registration| registration.id)
            else {
                return false;
            };
            let Some(position) = state
                .notification_records
                .iter()
                .enumerate()
                .filter(|(_, record)| {
                    record.registration_id == registration_id && record.title == title
                })
                .max_by_key(|(_, record)| record.id)
                .map(|(position, _)| position)
            else {
                return false;
            };
            state.notification_records.remove(position)
        };
        self.dispatch_notification_event(
            ServiceWorkerNotificationEventKind::Close,
            record.registration_id,
            record.id,
            record.title,
            record.tag,
            record.metadata,
            record.actions,
            String::new(),
            record.data,
        )
    }

    fn dispatch_notification_event(
        &self,
        kind: ServiceWorkerNotificationEventKind,
        registration_id: ServiceWorkerRegistrationId,
        notification_id: u64,
        title: String,
        tag: String,
        metadata: ServiceWorkerNotificationMetadata,
        actions: Vec<ServiceWorkerNotificationAction>,
        action: String,
        data: V8StructuredClonePayload,
    ) -> bool {
        let start = {
            let mut state = self.inner.state.lock();
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            let Some(version_id) = registration.active_version_id else {
                return false;
            };
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(version) = state.versions.get_mut(&version_id) else {
                return false;
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return false;
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerNotificationEvent {
                event_id,
                kind,
                registration_id,
                version_id,
                run: version.run.clone(),
                notification_id,
                title,
                tag,
                metadata,
                actions,
                action,
                data,
            };
            self.start_notification_event_locked(
                &mut state,
                registration_id,
                scope_url,
                registration_storage_key,
                event,
            )
        };
        match start {
            ServiceWorkerNotificationStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_notification_event_to_host(host, event);
            }
            ServiceWorkerNotificationStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerNotificationStart::Queued | ServiceWorkerNotificationStart::Dropped => {}
        }
        true
    }

    fn dispatch_notification_action_navigation(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        url: Url,
    ) -> bool {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            if registration.pending_unregistration {
                return false;
            }
            let Some(active_version_id) = registration.active_version_id else {
                return false;
            };
            let Some(active_version) = state.versions.get(&active_version_id) else {
                return false;
            };
            if active_version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return false;
            }
            let target_client = registration
                .controlled_client_ids
                .iter()
                .filter_map(|client_id| state.live_clients.get(client_id))
                .filter(|client| client.client_type == ServiceWorkerClientType::Window)
                .min_by_key(|client| client.id.as_u64());
            let Some(target_client) = target_client else {
                return false;
            };
            let Some(completion_tx) = target_client.endpoint.page_task_sender() else {
                return false;
            };
            (
                target_client
                    .window_completion_target()
                    .expect("notification action window client host"),
                completion_tx,
            )
        };
        let (host, completion_tx) = delivery;
        completion_tx
            .send_service_worker_notification_action_navigate_request(
                crate::types::ServiceWorkerNotificationActionNavigateRequestCompletion {
                    host,
                    url,
                },
            )
            .is_ok()
    }

    pub(super) fn dispatch_fetch_event(
        &self,
        host: SharedRendererServiceWorkerHost,
        mut event: ServiceWorkerFetchEvent,
    ) {
        let navigation_preload = self.navigation_preload_dispatch_for_event(&event);
        event.navigation_preload_sent = navigation_preload.is_some();
        if host.dispatch_fetch_event(event.clone()) {
            if let Some(navigation_preload) = navigation_preload {
                self.start_navigation_preload_dispatch(host, navigation_preload);
            }
            return;
        }
        self.enqueue_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id: event.event_id,
            version_id: event.version_id,
            run: event.run.clone(),
            result: ServiceWorkerFetchResult::Failure(
                "service worker fetch dispatch failed: worker is not running".to_owned(),
            ),
        });
    }

    fn navigation_preload_dispatch_for_event(
        &self,
        event: &ServiceWorkerFetchEvent,
    ) -> Option<ServiceWorkerNavigationPreloadDispatch> {
        let mut state = self.inner.state.lock();
        let job = state.pending_fetch_jobs.get(&event.event_id)?;
        if job.version_id != event.version_id || !job.is_bound_to_run(&event.run) {
            return None;
        }
        if !service_worker_fetch_event_can_use_navigation_preload(job) {
            return None;
        }
        let version = state.versions.get(&event.version_id)?;
        let registration = state.registrations.get(&version.registration_id)?;
        if registration.active_version_id != Some(event.version_id)
            || !registration.navigation_preload_state.enabled
        {
            return None;
        }

        let request_url = job.request_url.clone();
        let request_mode = job.request_mode;
        let request_client = job.request_client.clone();
        let resource_task_runner = job.resource_task_runner.clone();
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        let request = navigation_preload_request_for_job(
            job,
            registration.navigation_preload_state.header_value.as_str(),
        );
        if request.is_ok()
            && let Some(job) = state.pending_fetch_jobs.get_mut(&event.event_id)
            && job.version_id == event.version_id
            && job.is_bound_to_run(&event.run)
        {
            job.navigation_preload_cancel_handle = Some(cancel_handle.clone());
        }

        Some(ServiceWorkerNavigationPreloadDispatch {
            event_id: event.event_id,
            version_id: event.version_id,
            run: event.run.clone(),
            request_url,
            request_mode,
            request_client,
            resource_task_runner,
            cancel_handle,
            request,
        })
    }

    fn start_navigation_preload_dispatch(
        &self,
        host: SharedRendererServiceWorkerHost,
        dispatch: ServiceWorkerNavigationPreloadDispatch,
    ) {
        let service = self.clone();
        dispatch.resource_task_runner.clone().spawn(async move {
            stream_navigation_preload_response(service, host, dispatch).await;
        });
    }

    pub(super) fn mark_navigation_preload_response_started(
        &self,
        event_id: ServiceWorkerEventId,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(job) = state.pending_fetch_jobs.get_mut(&event_id) else {
            return false;
        };
        if job.version_id != version_id || !job.is_bound_to_run(&run) {
            return false;
        }
        job.clear_pending_navigation_preload_cancel_handle();
        true
    }

    pub(super) fn dispatch_notification_event_to_host(
        &self,
        host: SharedRendererServiceWorkerHost,
        event: ServiceWorkerNotificationEvent,
    ) {
        if host.dispatch_notification_event(event.clone()) {
            return;
        }
        self.enqueue_notification_event_completed(ServiceWorkerNotificationCompletion {
            event_id: event.event_id,
            version_id: event.version_id,
            run: event.run.clone(),
            result: Err(
                "service worker notification dispatch failed: worker is not running".to_owned(),
            ),
        });
    }

    pub(super) fn dispatch_push_event_to_host(
        &self,
        host: SharedRendererServiceWorkerHost,
        event: ServiceWorkerPushEvent,
    ) {
        if host.dispatch_push_event(event.clone()) {
            return;
        }
        self.enqueue_push_event_completed(ServiceWorkerPushCompletion {
            event_id: event.event_id,
            version_id: event.version_id,
            run: event.run.clone(),
            result: Err("service worker push dispatch failed: worker is not running".to_owned()),
        });
    }

    pub(super) fn dispatch_sync_event_to_host(
        &self,
        host: SharedRendererServiceWorkerHost,
        event: ServiceWorkerSyncEvent,
    ) {
        if host.dispatch_sync_event(event.clone()) {
            return;
        }
        self.enqueue_sync_event_completed(ServiceWorkerSyncCompletion {
            event_id: event.event_id,
            registration_id: event.registration_id,
            version_id: event.version_id,
            run: event.run.clone(),
            tag: event.tag,
            result: Err("service worker sync dispatch failed: worker is not running".to_owned()),
        });
    }

    pub(super) fn dispatch_periodic_sync_event_to_host(
        &self,
        host: SharedRendererServiceWorkerHost,
        event: ServiceWorkerPeriodicSyncEvent,
    ) {
        if host.dispatch_periodic_sync_event(event.clone()) {
            return;
        }
        self.enqueue_periodic_sync_event_completed(ServiceWorkerPeriodicSyncCompletion {
            event_id: event.event_id,
            registration_id: event.registration_id,
            version_id: event.version_id,
            run: event.run.clone(),
            tag: event.tag,
            result: Err(
                "service worker periodic sync dispatch failed: worker is not running".to_owned(),
            ),
        });
    }

    pub(super) fn dispatch_message_event(
        &self,
        host: SharedRendererServiceWorkerHost,
        event: ServiceWorkerMessageEvent,
    ) {
        if host.dispatch_message_event(event.clone()) {
            return;
        }
        self.enqueue_message_event_completed(ServiceWorkerMessageCompletion {
            event_id: event.event_id,
            version_id: event.version_id,
            run: event.run.clone(),
            result: Err("service worker message dispatch failed: worker is not running".to_owned()),
        });
    }
}

fn service_worker_fetch_request_is_navigation_destination(
    destination: ServiceWorkerRequestDestination,
) -> bool {
    matches!(
        destination,
        ServiceWorkerRequestDestination::Document | ServiceWorkerRequestDestination::Iframe
    )
}
