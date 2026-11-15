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
        let owner = version.run_owner();
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
        Some(ServiceWorkerIdleTimeout { owner, token })
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
                        owner: event.owner.clone(),
                        result: ServiceWorkerFetchResult::Failure(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Lifecycle(event) => {
                    self.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
                        event_id: event.event_id,
                        owner: event.owner.clone(),
                        kind: event.kind,
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Message(event) => {
                    self.finish_message_event_completed(ServiceWorkerMessageCompletion {
                        event_id: event.event_id,
                        owner: event.owner.clone(),
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Notification(event) => {
                    self.finish_notification_event_completed(ServiceWorkerNotificationCompletion {
                        event_id: event.event_id,
                        owner: event.owner.clone(),
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Push(event) => {
                    self.finish_push_event_completed(ServiceWorkerPushCompletion {
                        event_id: event.event_id,
                        owner: event.owner.clone(),
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::Sync(event) => {
                    self.finish_sync_event_completed(ServiceWorkerSyncCompletion {
                        event_id: event.event_id,
                        registration_id: event.registration_id,
                        owner: event.owner.clone(),
                        tag: event.tag,
                        result: Err(message.to_owned()),
                    });
                }
                ServiceWorkerPendingStartEvent::PeriodicSync(event) => {
                    self.finish_periodic_sync_event_completed(
                        ServiceWorkerPeriodicSyncCompletion {
                            event_id: event.event_id,
                            registration_id: event.registration_id,
                            owner: event.owner.clone(),
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
            owner: event.owner.clone(),
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
        self.try_activate_waiting_version_locked(state, registration_id, waiting_version_id)
            .unwrap_or_default()
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
                            owner: version.run_owner(),
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
                            owner: version.run_owner(),
                            kind,
                        }),
                    );
                    Some(ServiceWorkerLifecycleStart::Queued)
                }
                LifecycleRunningAction::StartStopped => {
                    let owner = version.replace_run_owner();
                    version.last_start_error = None;
                    let host = RendererServiceWorkerHost::new_loading(&owner);
                    let params = version.launch_config.to_launch_params(
                        registration_id,
                        &owner,
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
                            owner,
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
        let Some(version) = state.versions.get_mut(&event.owner.version_id()) else {
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
                event.owner = version.run_owner();
                ServiceWorkerMessageStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Message(event));
                ServiceWorkerMessageStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                let owner = version.replace_run_owner();
                version.last_start_error = None;
                let host = RendererServiceWorkerHost::new_loading(&owner);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    &owner,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.owner = owner;
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
        let Some(version) = state.versions.get_mut(&event.owner.version_id()) else {
            return ServiceWorkerNotificationStart::Dropped;
        };
        if version.registration_id != registration_id {
            return ServiceWorkerNotificationStart::Dropped;
        }
        if version.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating {
            Self::begin_version_event_locked(version);
            event.owner = version.run_owner();
            version
                .pending_start_events
                .push_back(ServiceWorkerPendingStartEvent::Notification(event));
            return ServiceWorkerNotificationStart::Queued;
        }
        if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
            return ServiceWorkerNotificationStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                ServiceWorkerNotificationStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Notification(event));
                ServiceWorkerNotificationStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                let owner = version.replace_run_owner();
                version.last_start_error = None;
                let host = RendererServiceWorkerHost::new_loading(&owner);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    &owner,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.owner = owner;
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
        let Some(version) = state.versions.get_mut(&event.owner.version_id()) else {
            return ServiceWorkerPushStart::Dropped;
        };
        if version.registration_id != registration_id {
            return ServiceWorkerPushStart::Dropped;
        }
        if version.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating {
            Self::begin_version_event_locked(version);
            event.owner = version.run_owner();
            version
                .pending_start_events
                .push_back(ServiceWorkerPendingStartEvent::Push(event));
            return ServiceWorkerPushStart::Queued;
        }
        if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
            return ServiceWorkerPushStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                ServiceWorkerPushStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Push(event));
                ServiceWorkerPushStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                let owner = version.replace_run_owner();
                version.last_start_error = None;
                let host = RendererServiceWorkerHost::new_loading(&owner);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    &owner,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.owner = owner;
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
        let Some(version) = state.versions.get_mut(&event.owner.version_id()) else {
            return ServiceWorkerSyncStart::Dropped;
        };
        if version.registration_id != registration_id {
            return ServiceWorkerSyncStart::Dropped;
        }
        if version.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating {
            Self::begin_version_event_locked(version);
            event.owner = version.run_owner();
            version
                .pending_start_events
                .push_back(ServiceWorkerPendingStartEvent::Sync(event));
            return ServiceWorkerSyncStart::Queued;
        }
        if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
            return ServiceWorkerSyncStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                ServiceWorkerSyncStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::Sync(event));
                ServiceWorkerSyncStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                let owner = version.replace_run_owner();
                version.last_start_error = None;
                let host = RendererServiceWorkerHost::new_loading(&owner);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    &owner,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.owner = owner;
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
        let Some(version) = state.versions.get_mut(&event.owner.version_id()) else {
            return ServiceWorkerPeriodicSyncStart::Dropped;
        };
        if version.registration_id != registration_id {
            return ServiceWorkerPeriodicSyncStart::Dropped;
        }
        if version.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating {
            Self::begin_version_event_locked(version);
            event.owner = version.run_owner();
            version
                .pending_start_events
                .push_back(ServiceWorkerPendingStartEvent::PeriodicSync(event));
            return ServiceWorkerPeriodicSyncStart::Queued;
        }
        if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
            return ServiceWorkerPeriodicSyncStart::Dropped;
        }

        match &version.running_state {
            ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                let host = host.clone();
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                ServiceWorkerPeriodicSyncStart::Dispatch(Box::new((host, event)))
            }
            ServiceWorkerVersionRunningState::Starting { .. } => {
                Self::begin_version_event_locked(version);
                event.owner = version.run_owner();
                version
                    .pending_start_events
                    .push_back(ServiceWorkerPendingStartEvent::PeriodicSync(event));
                ServiceWorkerPeriodicSyncStart::Queued
            }
            ServiceWorkerVersionRunningState::Stopped => {
                let owner = version.replace_run_owner();
                version.last_start_error = None;
                let host = RendererServiceWorkerHost::new_loading(&owner);
                let params = version.launch_config.to_launch_params(
                    registration_id,
                    &owner,
                    version.script_url.clone(),
                    scope_url,
                    storage_key,
                    version.script_kind,
                );
                Self::begin_version_event_locked(version);
                event.owner = owner;
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
    ) -> Option<Vec<LifecycleProgress>> {
        let version = state.versions.get(&version_id)?;
        if version.registration_id != registration_id {
            return None;
        }
        let registration = state.registrations.get(&registration_id)?;
        if registration
            .active_version_id
            .and_then(|active| state.versions.get(&active))
            .is_some_and(|active| {
                active.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating
            })
        {
            return None;
        }
        let start = self.lifecycle_event_for_version_locked(
            state,
            version_id,
            ServiceWorkerLifecycleEventKind::Activate,
        )?;
        let mut progress =
            self.publish_activating_version_locked(state, registration_id, version_id);
        if let Some(start) = Self::lifecycle_start_to_progress(start) {
            progress.push(start);
        }
        Some(progress)
    }

    // The activating worker is already active. In particular, activate's
    // waitUntil(clients.claim()) must not wait for activation to finish.
    pub(super) fn publish_activating_version_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Vec<LifecycleProgress> {
        let registration = state.registrations.get_mut(&registration_id).unwrap();
        let previous_active_version_id = registration.active_version_id.replace(version_id);
        registration.waiting_version_id = None;
        let mut progress = Vec::new();
        let mut failed_replaced_pending_events = Vec::new();
        let mut failed_replaced_fetch_completions = Vec::new();
        let mut previous_target_cleanup = None;
        if let Some(previous_active_version_id) = previous_active_version_id
            && previous_active_version_id != version_id
            && let Some(previous_version) = state.versions.get_mut(&previous_active_version_id)
        {
            previous_version.lifecycle_state = ServiceWorkerVersionLifecycleState::Redundant;
            previous_version.skip_waiting_requested = false;
            let previous_owner = previous_version.run_owner();
            failed_replaced_pending_events =
                previous_version.pending_start_events.drain(..).collect();
            let failed_replaced_activation_fetch_events = previous_version
                .pending_activation_fetch_events
                .drain(..)
                .collect::<Vec<_>>();
            let previous_host = previous_version.running_state.take_host_for_shutdown();
            let previous_host_run = previous_host.as_ref().map(|host| host.run_identity());
            if let Some(host) = previous_host {
                progress.push(LifecycleProgress::TerminateHost(host));
            }
            previous_target_cleanup = Some((previous_active_version_id, previous_host_run));
            let pending_start_fetch_event_ids = failed_replaced_pending_events
                .iter()
                .filter_map(|event| match event {
                    ServiceWorkerPendingStartEvent::Fetch(event) => Some(event.event_id),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            let pending_activation_fetch_event_ids = failed_replaced_activation_fetch_events
                .iter()
                .map(|event| event.event_id)
                .collect::<HashSet<_>>();
            failed_replaced_fetch_completions = state
                .pending_fetch_jobs
                .iter()
                .filter(|(event_id, job)| {
                    job.is_bound_to_owner(&previous_owner)
                        && !pending_start_fetch_event_ids.contains(event_id)
                        && !pending_activation_fetch_event_ids.contains(event_id)
                })
                .map(|(event_id, job)| ServiceWorkerFetchCompletion {
                    event_id: *event_id,
                    owner: ServiceWorkerRunOwner::new(job.version_id(), job.run_identity().clone()),
                    result: ServiceWorkerFetchResult::Failure(
                        "service worker was replaced by a newer active worker".to_owned(),
                    ),
                })
                .collect();
            failed_replaced_fetch_completions.extend(
                failed_replaced_activation_fetch_events
                    .into_iter()
                    .map(|event| ServiceWorkerFetchCompletion {
                        event_id: event.event_id,
                        owner: event.owner,
                        result: ServiceWorkerFetchResult::Failure(
                            "service worker was replaced by a newer active worker".to_owned(),
                        ),
                    }),
            );
        }
        if let Some((previous_active_version_id, previous_run)) = previous_target_cleanup {
            if let Some(run) = previous_run {
                state.record_target_stopped(
                    previous_active_version_id,
                    run,
                    "replaced_by_newer_active_worker",
                );
            }
            state.record_target_destroyed(previous_active_version_id);
        }
        let mut lifecycle_events = vec![ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
            version_id,
            state: "activating",
        }];
        if let Some(previous_active_version_id) = previous_active_version_id
            && previous_active_version_id != version_id
        {
            lifecycle_events.insert(
                0,
                ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                    version_id: previous_active_version_id,
                    state: "redundant",
                },
            );
        }
        progress.extend(
            lifecycle_notifications_for_registration_locked(
                state,
                registration_id,
                lifecycle_events,
            )
            .into_iter()
            .map(|notification| LifecycleProgress::NotifyLifecycle(Box::new(notification))),
        );

        let snapshot = service_worker_registration_snapshot(
            state,
            state.registrations.get(&registration_id).unwrap(),
        );
        let pending_ready_jobs = std::mem::take(&mut state.pending_ready_jobs);
        for job in pending_ready_jobs {
            if job.registration_id == registration_id {
                progress.push(LifecycleProgress::ReadyCompleted(Box::new((
                    job,
                    snapshot.clone(),
                ))));
            } else {
                state.pending_ready_jobs.push(job);
            }
        }
        if previous_active_version_id
            .is_some_and(|previous_version_id| previous_version_id != version_id)
        {
            progress.extend(
                controller_change_deliveries_for_controlled_clients_locked(state, registration_id)
                    .into_iter()
                    .map(LifecycleProgress::NotifyControllerChange),
            );
        }
        progress.push(LifecycleProgress::FailReplacedWorkerEvents(Box::new((
            failed_replaced_pending_events,
            failed_replaced_fetch_completions,
        ))));
        progress
    }
}
