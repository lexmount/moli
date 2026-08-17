use super::*;

impl ServiceWorkerRuntimeService {
    pub(super) fn begin_version_event_locked(version: &mut ServiceWorkerVersion) {
        version.in_flight_event_count += 1;
        version.idle_timeout_token = None;
    }

    pub(super) fn maybe_schedule_idle_timeout_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        version_id: ServiceWorkerVersionId,
    ) -> Option<ServiceWorkerIdleTimeout> {
        let version = state.versions.get_mut(&version_id)?;
        if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
            || version.in_flight_event_count != 0
            || !version.pending_start_events.is_empty()
            || !version.pending_activation_fetch_events.is_empty()
        {
            return None;
        }
        let registration_id = version.registration_id;
        let run = version.run.clone();
        let ServiceWorkerVersionRunningState::Running { host } = &version.running_state else {
            return None;
        };
        if !host.has_running_worker() {
            return None;
        }
        if !state
            .registrations
            .get(&registration_id)
            .is_some_and(|registration| registration.active_version_id == Some(version_id))
        {
            return None;
        }
        let token = ServiceWorkerIdleTimeoutToken::fresh();
        version.idle_timeout_token = Some(token.clone());
        Some(ServiceWorkerIdleTimeout {
            version_id,
            run,
            token,
        })
    }

    pub(super) fn fail_pending_start_events(
        &self,
        failed_pending_events: Vec<ServiceWorkerPendingStartEvent>,
        message: &str,
    ) {
        for event in failed_pending_events {
            match event {
                ServiceWorkerPendingStartEvent::Fetch(event) => {
                    self.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        result: ServiceWorkerFetchResult::Failure(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Lifecycle(event) => {
                    self.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        kind: event.kind,
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Message(event) => {
                    self.finish_message_event_completed(ServiceWorkerMessageCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Notification(event) => {
                    self.finish_notification_event_completed(ServiceWorkerNotificationCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Push(event) => {
                    self.finish_push_event_completed(ServiceWorkerPushCompletion {
                        event_id: event.event_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Sync(event) => {
                    self.finish_sync_event_completed(ServiceWorkerSyncCompletion {
                        event_id: event.event_id,
                        registration_id: event.registration_id,
                        version_id: event.version_id,
                        run: event.run.clone(),
                        tag: event.tag,
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::PeriodicSync(event) => {
                    self.finish_periodic_sync_event_completed(
                        ServiceWorkerPeriodicSyncCompletion {
                            event_id: event.event_id,
                            registration_id: event.registration_id,
                            version_id: event.version_id,
                            run: event.run.clone(),
                            tag: event.tag,
                            result: Err(message.to_owned()),
                        },
                    );
                }
            }
        }
    }

    pub(super) fn dispatch_lifecycle_event(
        &self,
        host: SharedRendererServiceWorkerHost,
        event: ServiceWorkerLifecycleEvent,
    ) {
        if host.dispatch_lifecycle_event(event.clone()) {
            return;
        }
        self.enqueue_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: event.event_id,
            version_id: event.version_id,
            run: event.run.clone(),
            kind: event.kind,
            result: Err(
                "service worker lifecycle dispatch failed: worker is not running".to_owned(),
            ),
        });
    }

    pub(super) fn lifecycle_start_to_progress(
        start: ServiceWorkerLifecycleStart,
    ) -> Option<LifecycleProgress> {
        match start {
            ServiceWorkerLifecycleStart::Dispatch(dispatch) => {
                Some(LifecycleProgress::Dispatch(dispatch))
            }
            ServiceWorkerLifecycleStart::Start(launch) => {
                Some(LifecycleProgress::StartWorker(launch))
            }
            ServiceWorkerLifecycleStart::Queued => None,
        }
    }

    pub(super) fn activation_progress_for_registration_if_ready_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
    ) -> Vec<LifecycleProgress> {
        let Some(waiting_version_id) = state
            .registrations
            .get(&registration_id)
            .and_then(|registration| registration.waiting_version_id)
        else {
            return Vec::new();
        };
        let Some(start) =
            self.try_activate_waiting_version_locked(state, registration_id, waiting_version_id)
        else {
            return Vec::new();
        };
        let mut progress = lifecycle_notifications_for_registration_locked(
            state,
            registration_id,
            vec![ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                version_id: waiting_version_id,
                state: "activating",
            }],
        )
        .into_iter()
        .map(|notification| LifecycleProgress::NotifyLifecycle(Box::new(notification)))
        .collect::<Vec<_>>();
        if let Some(start_progress) = Self::lifecycle_start_to_progress(start) {
            progress.push(start_progress);
        }
        progress
    }

    pub(super) fn activation_progress_for_active_version_if_ready_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        active_version_id: ServiceWorkerVersionId,
    ) -> Vec<LifecycleProgress> {
        let Some(registration_id) = state
            .versions
            .get(&active_version_id)
            .map(|version| version.registration_id)
        else {
            return Vec::new();
        };
        let Some(registration) = state.registrations.get(&registration_id) else {
            return Vec::new();
        };
        if registration.active_version_id != Some(active_version_id) {
            return Vec::new();
        }
        self.activation_progress_for_registration_if_ready_locked(state, registration_id)
    }

    pub(super) fn unregistration_progress_for_registration_if_ready_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
    ) -> Vec<LifecycleProgress> {
        let Some(registration) = state.registrations.get(&registration_id) else {
            return Vec::new();
        };
        if !registration_ready_to_delete_locked(state, registration) {
            return Vec::new();
        }
        let registration_key = registration.key();
        let has_queued_jobs = state.job_coordinator.has_jobs(&registration_key);
        if has_queued_jobs {
            self.delete_registration_resources_for_key_locked(&registration_key);
            let mut progress = execute_pending_clear_locked(
                state,
                registration_id,
                ServiceWorkerPendingClearAction::KeepRegistrationForQueuedJob,
            );
            progress.extend(self.advance_registration_job_queue_locked(state, registration_id));
            return progress;
        }
        self.delete_registration_resources_for_key_locked(&registration_key);
        execute_pending_clear_locked(
            state,
            registration_id,
            ServiceWorkerPendingClearAction::DeleteRegistration,
        )
    }

    pub(super) fn unregistration_progress_for_version_if_ready_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        version_id: ServiceWorkerVersionId,
    ) -> Vec<LifecycleProgress> {
        let Some(registration_id) = state
            .versions
            .get(&version_id)
            .map(|version| version.registration_id)
        else {
            return Vec::new();
        };
        self.unregistration_progress_for_registration_if_ready_locked(state, registration_id)
    }

    pub(super) fn lifecycle_event_for_version_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        version_id: ServiceWorkerVersionId,
        kind: ServiceWorkerLifecycleEventKind,
    ) -> Option<ServiceWorkerLifecycleStart> {
        let (registration_id, scope_url, storage_key) = {
            let version = state.versions.get(&version_id)?;
            let registration = state.registrations.get(&version.registration_id)?;
            match kind {
                ServiceWorkerLifecycleEventKind::Install => {
                    if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Installing {
                        return None;
                    }
                    if registration.installing_version_id != Some(version_id) {
                        return None;
                    }
                }
                ServiceWorkerLifecycleEventKind::Activate => {
                    if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Installed {
                        return None;
                    }
                    if registration.waiting_version_id != Some(version_id) {
                        return None;
                    }
                    if registration.active_version_id.is_some()
                        && !version.skip_waiting_requested
                        && !registration_ready_to_activate_locked(state, registration)
                    {
                        return None;
                    }
                }
            }
            (
                version.registration_id,
                registration.scope_url.clone(),
                registration.storage_key.clone(),
            )
        };

        let target_status_changed = kind == ServiceWorkerLifecycleEventKind::Activate;

        enum LifecycleRunningAction {
            Dispatch(SharedRendererServiceWorkerHost),
            QueueStarting,
            StartStopped,
        }

        let start = {
            let version = state.versions.get_mut(&version_id)?;

            let running_action = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                    LifecycleRunningAction::Dispatch(host.clone())
                }
                ServiceWorkerVersionRunningState::Starting { .. } => {
                    LifecycleRunningAction::QueueStarting
                }
                ServiceWorkerVersionRunningState::Stopped => LifecycleRunningAction::StartStopped,
                ServiceWorkerVersionRunningState::Running { .. } => return None,
            };

            if target_status_changed {
                version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
            }

            match running_action {
                LifecycleRunningAction::Dispatch(host) => {
                    Self::begin_version_event_locked(version);
                    let event_id = ServiceWorkerEventId(
                        self.inner.next_event_id.fetch_add(1, Ordering::Relaxed),
                    );
                    Some(ServiceWorkerLifecycleStart::Dispatch((
                        host,
                        ServiceWorkerLifecycleEvent {
                            event_id,
                            version_id,
                            run: version.run.clone(),
                            kind,
                        },
                    )))
                }
                LifecycleRunningAction::QueueStarting => {
                    Self::begin_version_event_locked(version);
                    let event_id = ServiceWorkerEventId(
                        self.inner.next_event_id.fetch_add(1, Ordering::Relaxed),
                    );
                    version.pending_start_events.push_back(
                        ServiceWorkerPendingStartEvent::Lifecycle(ServiceWorkerLifecycleEvent {
                            event_id,
                            version_id,
                            run: version.run.clone(),
                            kind,
                        }),
                    );
                    Some(ServiceWorkerLifecycleStart::Queued)
                }
                LifecycleRunningAction::StartStopped => {
                    version.run = RendererServiceWorkerRunIdentity::fresh();
                    version.last_start_error = None;
                    let run = version.run.clone();
                    let host = RendererServiceWorkerHost::new_loading(version_id, &run);
                    let params = version.launch_config.to_launch_params(
                        registration_id,
                        version_id,
                        &run,
                        version.script_url.clone(),
                        scope_url,
                        storage_key,
                        version.script_kind,
                    );
                    Self::begin_version_event_locked(version);
                    let event_id = ServiceWorkerEventId(
                        self.inner.next_event_id.fetch_add(1, Ordering::Relaxed),
                    );
                    version.pending_start_events.push_back(
                        ServiceWorkerPendingStartEvent::Lifecycle(ServiceWorkerLifecycleEvent {
                            event_id,
                            version_id,
                            run,
                            kind,
                        }),
                    );
                    version.running_state =
                        ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                    Some(ServiceWorkerLifecycleStart::Start(Box::new(
                        ServiceWorkerQueuedLaunch {
                            params,
                            host,
                            lifecycle_notifications: Vec::new(),
                            preloaded_script: None,
                        },
                    )))
                }
            }
        };
        if target_status_changed {
            state.record_target_version_updated(version_id);
        }
        start
    }

    pub(super) fn start_message_event_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        scope_url: Url,
        storage_key: String,
        mut event: ServiceWorkerMessageEvent,
    ) -> ServiceWorkerMessageStart {
        let Some(version) = state.versions.get_mut(&event.version_id) else {
            return ServiceWorkerMessageStart::Dropped;
        };
        if version.registration_id != registration_id
            || version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant
        {
            return ServiceWorkerMessageStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                ServiceWorkerMessageStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Message(event));
                ServiceWorkerMessageStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                version.run = RendererServiceWorkerRunIdentity::fresh();
                version.last_start_error = None;
                let run = version.run.clone();
                let host = RendererServiceWorkerHost::new_loading(event.version_id, &run);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    event.version_id,
                    &run,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.run = run;
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Message(event));
                version.running_state =
                    ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                ServiceWorkerMessageStart::Start(Box::new(ServiceWorkerQueuedLaunch {
                    params,
                    host,
                    lifecycle_notifications: Vec::new(),
                    preloaded_script: None,
                }))
            }
            ServiceWorkerVersionRunningState::Running { .. } => ServiceWorkerMessageStart::Dropped,
        }
    }

    pub(super) fn start_notification_event_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        scope_url: Url,
        storage_key: String,
        mut event: ServiceWorkerNotificationEvent,
    ) -> ServiceWorkerNotificationStart {
        let Some(version) = state.versions.get_mut(&event.version_id) else {
            return ServiceWorkerNotificationStart::Dropped;
        };
        if version.registration_id != registration_id
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
        {
            return ServiceWorkerNotificationStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                ServiceWorkerNotificationStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Notification(event));
                ServiceWorkerNotificationStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                version.run = RendererServiceWorkerRunIdentity::fresh();
                version.last_start_error = None;
                let run = version.run.clone();
                let host = RendererServiceWorkerHost::new_loading(event.version_id, &run);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    event.version_id,
                    &run,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.run = run;
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Notification(event));
                version.running_state =
                    ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                ServiceWorkerNotificationStart::Start(Box::new(ServiceWorkerQueuedLaunch {
                    params,
                    host,
                    lifecycle_notifications: Vec::new(),
                    preloaded_script: None,
                }))
            }
            ServiceWorkerVersionRunningState::Running { .. } => {
                ServiceWorkerNotificationStart::Dropped
            }
        }
    }

    pub(super) fn start_push_event_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        scope_url: Url,
        storage_key: String,
        mut event: ServiceWorkerPushEvent,
    ) -> ServiceWorkerPushStart {
        let Some(version) = state.versions.get_mut(&event.version_id) else {
            return ServiceWorkerPushStart::Dropped;
        };
        if version.registration_id != registration_id
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
        {
            return ServiceWorkerPushStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                ServiceWorkerPushStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Push(event));
                ServiceWorkerPushStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                version.run = RendererServiceWorkerRunIdentity::fresh();
                version.last_start_error = None;
                let run = version.run.clone();
                let host = RendererServiceWorkerHost::new_loading(event.version_id, &run);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    event.version_id,
                    &run,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.run = run;
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Push(event));
                version.running_state =
                    ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                ServiceWorkerPushStart::Start(Box::new(ServiceWorkerQueuedLaunch {
                    params,
                    host,
                    lifecycle_notifications: Vec::new(),
                    preloaded_script: None,
                }))
            }
            ServiceWorkerVersionRunningState::Running { .. } => ServiceWorkerPushStart::Dropped,
        }
    }

    pub(super) fn start_sync_event_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        scope_url: Url,
        storage_key: String,
        mut event: ServiceWorkerSyncEvent,
    ) -> ServiceWorkerSyncStart {
        let Some(version) = state.versions.get_mut(&event.version_id) else {
            return ServiceWorkerSyncStart::Dropped;
        };
        if version.registration_id != registration_id
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
        {
            return ServiceWorkerSyncStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                ServiceWorkerSyncStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Sync(event));
                ServiceWorkerSyncStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                version.run = RendererServiceWorkerRunIdentity::fresh();
                version.last_start_error = None;
                let run = version.run.clone();
                let host = RendererServiceWorkerHost::new_loading(event.version_id, &run);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    event.version_id,
                    &run,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.run = run;
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Sync(event));
                version.running_state =
                    ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                ServiceWorkerSyncStart::Start(Box::new(ServiceWorkerQueuedLaunch {
                    params,
                    host,
                    lifecycle_notifications: Vec::new(),
                    preloaded_script: None,
                }))
            }
            ServiceWorkerVersionRunningState::Running { .. } => ServiceWorkerSyncStart::Dropped,
        }
    }

    pub(super) fn start_periodic_sync_event_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        scope_url: Url,
        storage_key: String,
        mut event: ServiceWorkerPeriodicSyncEvent,
    ) -> ServiceWorkerPeriodicSyncStart {
        let Some(version) = state.versions.get_mut(&event.version_id) else {
            return ServiceWorkerPeriodicSyncStart::Dropped;
        };
        if version.registration_id != registration_id
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
        {
            return ServiceWorkerPeriodicSyncStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                ServiceWorkerPeriodicSyncStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.run = version.run.clone();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::PeriodicSync(event));
                ServiceWorkerPeriodicSyncStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                version.run = RendererServiceWorkerRunIdentity::fresh();
                version.last_start_error = None;
                let run = version.run.clone();
                let host = RendererServiceWorkerHost::new_loading(event.version_id, &run);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    event.version_id,
                    &run,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.run = run;
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::PeriodicSync(event));
                version.running_state =
                    ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                ServiceWorkerPeriodicSyncStart::Start(Box::new(ServiceWorkerQueuedLaunch {
                    params,
                    host,
                    lifecycle_notifications: Vec::new(),
                    preloaded_script: None,
                }))
            }
            ServiceWorkerVersionRunningState::Running { .. } => {
                ServiceWorkerPeriodicSyncStart::Dropped
            }
        }
    }

    pub(super) fn try_activate_waiting_version_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Option<ServiceWorkerLifecycleStart> {
        let version = state.versions.get(&version_id)?;
        if version.registration_id != registration_id {
            return None;
        }
        self.lifecycle_event_for_version_locked(
            state,
            version_id,
            ServiceWorkerLifecycleEventKind::Activate,
        )
    }
}
