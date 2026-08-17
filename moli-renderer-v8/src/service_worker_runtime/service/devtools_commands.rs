use super::*;

impl ServiceWorkerRuntimeService {
    pub(crate) fn set_pause_new_workers_on_start_for_devtools(&self, pause: bool) {
        self.inner
            .pause_new_workers_on_start_for_devtools
            .store(pause, Ordering::Relaxed);
        if !pause {
            self.devtools_release_all_workers_waiting_for_debugger();
        }
    }

    pub(crate) fn pause_new_workers_on_start_for_devtools(&self) -> bool {
        self.inner
            .pause_new_workers_on_start_for_devtools
            .load(Ordering::Relaxed)
    }

    pub(crate) fn set_related_pause_on_start_policies_for_devtools(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        let policies = policies
            .into_iter()
            .filter_map(
                |(registration_id, base_version_id, script_url, scope_url)| {
                    let script_url = Url::parse(&script_url).ok()?;
                    let scope_url = Url::parse(&scope_url).ok()?;
                    Some(ServiceWorkerDevToolsRelatedPauseOnStartPolicy {
                        registration_id: ServiceWorkerRegistrationId::from_u64_for_binding(
                            registration_id,
                        ),
                        base_version_id: ServiceWorkerVersionId::from_u64_for_binding(
                            base_version_id,
                        ),
                        script_url,
                        scope_url,
                    })
                },
            )
            .collect();
        self.inner
            .state
            .lock()
            .devtools_related_pause_on_start_policies = policies;
    }

    pub(crate) fn set_pause_on_start_for_version_for_devtools(
        &self,
        version_id: ServiceWorkerVersionId,
        pause: bool,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(version) = state.versions.get_mut(&version_id) else {
            return false;
        };
        version.should_pause_on_start_for_devtools = pause;
        true
    }

    pub(super) fn should_pause_new_worker_on_start_for_devtools_locked(
        &self,
        state: &ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: &Url,
        scope_url: &Url,
    ) -> bool {
        self.pause_new_workers_on_start_for_devtools()
            || state
                .devtools_related_pause_on_start_policies
                .iter()
                .any(|policy| policy.matches(registration_id, version_id, script_url, scope_url))
    }

    pub(crate) fn set_devtools_attached_for_version(
        &self,
        version_id: ServiceWorkerVersionId,
        attached: bool,
    ) {
        let mut state = self.inner.state.lock();
        if !state.versions.contains_key(&version_id) {
            state.devtools_attached_versions.remove(&version_id);
            return;
        }
        if attached {
            state.devtools_attached_versions.insert(version_id);
        } else {
            state.devtools_attached_versions.remove(&version_id);
        }
    }

    pub(super) fn version_should_pause_on_start_for_devtools_locked(
        state: &ServiceWorkerRuntimeState,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        state
            .versions
            .get(&version_id)
            .is_some_and(|version| version.should_pause_on_start_for_devtools)
    }

    pub(crate) fn devtools_run_if_waiting_for_debugger(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        let launch = {
            let mut state = self.inner.state.lock();
            state.pending_devtools_launches.remove(&version_id)
        };
        if let Some(launch) = launch {
            self.start_queued_launch_without_devtools_pause(launch);
            return true;
        }

        let start = {
            let mut state = self.inner.state.lock();
            let Some(registration_id) = state
                .versions
                .get(&version_id)
                .map(|version| version.registration_id)
            else {
                return false;
            };
            state
                .pending_main_script_update_checks
                .get_mut(&registration_id)
                .filter(|pending_check| pending_check.new_version_id == version_id)
                .and_then(|pending_check| {
                    pending_check
                        .take_deferred_load_params()
                        .map(|load_params| (registration_id, load_params))
                })
        };
        let Some((registration_id, load_params)) = start else {
            return self.devtools_pre_release_evaluation_if_worker_is_starting(version_id);
        };
        self.start_main_script_update_check(registration_id, load_params);
        true
    }

    fn devtools_pre_release_evaluation_if_worker_is_starting(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(version) = state.versions.get(&version_id) else {
            return false;
        };
        if !version.should_pause_on_start_for_devtools {
            return false;
        }
        if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Installing
            && !state.devtools_attached_versions.contains(&version_id)
        {
            return false;
        }
        let ServiceWorkerVersionRunningState::Starting { host } = &version.running_state else {
            return false;
        };
        if host.has_running_worker() {
            return false;
        }
        state
            .pending_devtools_evaluation_releases
            .insert(version_id);
        true
    }

    pub(crate) fn take_devtools_evaluation_release_for_version(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        self.inner
            .state
            .lock()
            .pending_devtools_evaluation_releases
            .remove(&version_id)
    }

    pub(crate) fn devtools_release_all_workers_waiting_for_debugger(&self) -> usize {
        let (starts, launches) = {
            let mut state = self.inner.state.lock();
            let starts = state
                .pending_main_script_update_checks
                .iter_mut()
                .filter_map(|(registration_id, pending_check)| {
                    pending_check
                        .take_deferred_load_params()
                        .map(|load_params| (*registration_id, load_params))
                })
                .collect::<Vec<_>>();
            let launches = state
                .pending_devtools_launches
                .drain()
                .map(|(_, launch)| launch)
                .collect::<Vec<_>>();
            state.pending_devtools_evaluation_releases.clear();
            (starts, launches)
        };
        let released_count = starts.len() + launches.len();
        for (registration_id, load_params) in starts {
            self.start_main_script_update_check(registration_id, load_params);
        }
        for launch in launches {
            self.start_queued_launch_without_devtools_pause(launch);
        }
        released_count
    }

    pub(crate) fn set_force_update_on_page_load_for_devtools(&self, force_update: bool) {
        self.inner
            .force_update_on_page_load
            .store(force_update, Ordering::Relaxed);
    }

    pub(crate) fn force_update_on_page_load_for_devtools(&self) -> bool {
        self.inner.force_update_on_page_load.load(Ordering::Relaxed)
    }

    pub(crate) fn devtools_force_update_registration_for_page_load(
        &self,
        scope_url: &Url,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> (bool, Option<tokio::sync::oneshot::Receiver<()>>) {
        if !self.force_update_on_page_load_for_devtools() {
            return (false, None);
        }
        let waiter_id = self
            .inner
            .next_force_update_page_load_waiter_id
            .fetch_add(1, Ordering::Relaxed);
        let (waiter_tx, waiter_rx) = tokio::sync::oneshot::channel();
        {
            let mut state = self.inner.state.lock();
            state.insert_force_update_page_load_waiter(waiter_id, waiter_tx);
        }
        let started = self
            .devtools_update_registration_for_scope_with_options(
                scope_url,
                browser_context_runtime,
                true,
                true,
                true,
                vec![waiter_id],
            )
            .unwrap_or(false);
        if !started {
            let _ = self
                .inner
                .state
                .lock()
                .take_force_update_page_load_waiters(vec![waiter_id]);
            return (false, None);
        }
        (true, Some(waiter_rx))
    }

    pub(crate) fn devtools_unregister_scope(&self, scope_url: &Url) -> Result<bool, String> {
        let start = self.start_unregistration_job(
            scope_url,
            ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url),
            None,
        );
        Ok(match start {
            ServiceWorkerUnregisterStart::Completed(result) => result,
            ServiceWorkerUnregisterStart::Queued => true,
        })
    }

    pub(crate) fn devtools_start_worker_for_scope(&self, scope_url: &Url) -> Result<bool, String> {
        let launch = {
            let mut state = self.inner.state.lock();
            let registration_key = ServiceWorkerRegistrationKey::for_scope_and_storage_key(
                scope_url,
                ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url),
            );
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| registration.key() == registration_key)
                .map(|registration| registration.id)
            else {
                return Ok(false);
            };
            let (version_id, registration_scope_url, registration_storage_key) = {
                let Some(registration) = state.registrations.get(&registration_id) else {
                    return Ok(false);
                };
                if registration.pending_unregistration {
                    return Ok(false);
                }
                let Some(version_id) = registration.active_version_id else {
                    return Ok(false);
                };
                (
                    version_id,
                    registration.scope_url.clone(),
                    registration.storage_key.clone(),
                )
            };
            let Some(version) = state.versions.get_mut(&version_id) else {
                return Ok(false);
            };
            if version.registration_id != registration_id
                || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
            {
                return Ok(false);
            }
            match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } if host.has_running_worker() => {
                    return Ok(true);
                }
                ServiceWorkerVersionRunningState::Starting { .. } => {
                    return Ok(true);
                }
                ServiceWorkerVersionRunningState::Running { .. } => {
                    return Ok(false);
                }
                ServiceWorkerVersionRunningState::Stopped => {
                    version.run = RendererServiceWorkerRunIdentity::fresh();
                    version.last_start_error = None;
                    let run = version.run.clone();
                    let host = RendererServiceWorkerHost::new_loading(version_id, &run);
                    let params = version.launch_config.to_launch_params(
                        registration_id,
                        version_id,
                        &run,
                        version.script_url.clone(),
                        registration_scope_url,
                        registration_storage_key,
                        version.script_kind,
                    );
                    version.running_state =
                        ServiceWorkerVersionRunningState::Starting { host: host.clone() };
                    Some(ServiceWorkerQueuedLaunch {
                        params,
                        host,
                        lifecycle_notifications: Vec::new(),
                        preloaded_script: None,
                    })
                }
            }
        };
        if let Some(launch) = launch {
            self.start_queued_launch(launch);
        }
        Ok(true)
    }

    pub(crate) fn devtools_stop_worker_version(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> Result<bool, String> {
        let host = {
            let mut state = self.inner.state.lock();
            let Some(host) = stop_worker_version_locked(&mut state, version_id, "devtools_stop")
            else {
                return Ok(false);
            };
            host
        };
        host.terminate_without_join();
        Ok(true)
    }

    pub(crate) fn devtools_stop_all_workers(&self) -> Result<usize, String> {
        let hosts = {
            let mut state = self.inner.state.lock();
            let version_ids = state.versions.keys().copied().collect::<Vec<_>>();
            let mut hosts = Vec::new();
            for version_id in version_ids {
                if let Some(host) =
                    stop_worker_version_locked(&mut state, version_id, "devtools_stop_all")
                {
                    hosts.push(host);
                }
            }
            hosts
        };
        let stopped_count = hosts.len();
        for host in hosts {
            host.terminate_without_join();
        }
        Ok(stopped_count)
    }

    pub(crate) fn devtools_skip_waiting_for_scope(&self, scope_url: &Url) -> Result<bool, String> {
        let progress = {
            let mut state = self.inner.state.lock();
            let registration_key = ServiceWorkerRegistrationKey::for_scope_and_storage_key(
                scope_url,
                ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url),
            );
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| registration.key() == registration_key)
                .map(|registration| registration.id)
            else {
                return Ok(false);
            };
            let Some(waiting_version_id) = state
                .registrations
                .get(&registration_id)
                .and_then(|registration| registration.waiting_version_id)
            else {
                return Ok(false);
            };
            let Some(version) = state.versions.get_mut(&waiting_version_id) else {
                return Ok(false);
            };
            if version.registration_id != registration_id {
                return Ok(false);
            }
            version.skip_waiting_requested = true;
            self.activation_progress_for_registration_if_ready_locked(&mut state, registration_id)
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
        Ok(true)
    }

    pub(crate) fn devtools_update_registration_for_scope(
        &self,
        scope_url: &Url,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> Result<bool, String> {
        self.devtools_update_registration_for_scope_with_options(
            scope_url,
            browser_context_runtime,
            true,
            false,
            false,
            Vec::new(),
        )
    }

    fn devtools_update_registration_for_scope_with_options(
        &self,
        scope_url: &Url,
        browser_context_runtime: RendererBrowserContextRuntime,
        force_bypass_cache: bool,
        skip_script_comparison: bool,
        skip_waiting_after_install: bool,
        force_update_page_load_waiter_ids: Vec<u64>,
    ) -> Result<bool, String> {
        let queued_job = {
            let state = self.inner.state.lock();
            let registration_key = ServiceWorkerRegistrationKey::for_scope_and_storage_key(
                scope_url,
                ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url),
            );
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| registration.key() == registration_key)
                .map(|registration| registration.id)
            else {
                return Ok(false);
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return Ok(false);
            };
            if registration.pending_unregistration {
                return Ok(false);
            }
            let pending_update_check_new_version_id = state
                .pending_main_script_update_checks
                .get(&registration_id)
                .map(|pending_check| pending_check.new_version_id);
            let Some(newest_version_id) = registration
                .installing_version_id
                .filter(|version_id| Some(*version_id) != pending_update_check_new_version_id)
                .or(registration.waiting_version_id)
                .or(registration.active_version_id)
            else {
                return Ok(false);
            };
            let Some(newest_version) = state.versions.get(&newest_version_id) else {
                return Ok(false);
            };
            let request_client = newest_version.launch_config.request_client();
            ServiceWorkerQueuedRegisterJob {
                script_url: newest_version.script_url.clone(),
                scope_url: registration.scope_url.clone(),
                document_url: newest_version.launch_config.document_url.clone(),
                storage_key: registration.storage_key.clone(),
                script_kind: newest_version.script_kind,
                update_via_cache: registration.update_via_cache,
                force_bypass_cache,
                skip_script_comparison,
                skip_waiting_after_install,
                force_update_page_load_waiter_ids,
                request_client,
                network_policy: newest_version.launch_config.network_policy.clone(),
                browser_context_runtime,
                broadcast_channel_top_level_site: newest_version
                    .launch_config
                    .broadcast_channel_top_level_site
                    .clone(),
                indexed_db_manager: newest_version.launch_config.indexed_db_manager.clone(),
                storage_bucket_store: newest_version.launch_config.storage_bucket_store.clone(),
                callbacks: Vec::new(),
            }
        };
        self.start_queued_register_job(queued_job);
        Ok(true)
    }

    pub(crate) fn devtools_deliver_push_message(
        &self,
        origin: &Url,
        registration_id: ServiceWorkerRegistrationId,
        data: Option<Vec<u8>>,
    ) -> Result<bool, String> {
        let start = {
            let mut state = self.inner.state.lock();
            let Some((version_id, scope_url, storage_key)) =
                active_registration_for_devtools_locked(&state, origin, registration_id)
            else {
                return Ok(false);
            };
            let Some(version) = state.versions.get(&version_id) else {
                return Ok(false);
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return Ok(false);
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerPushEvent {
                event_id,
                version_id,
                run: version.run.clone(),
                data,
            };
            self.start_push_event_locked(&mut state, registration_id, scope_url, storage_key, event)
        };
        let accepted = !matches!(start, ServiceWorkerPushStart::Dropped);
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
        Ok(accepted)
    }

    pub(crate) fn devtools_dispatch_sync_event(
        &self,
        origin: &Url,
        registration_id: ServiceWorkerRegistrationId,
        tag: String,
        last_chance: bool,
    ) -> Result<bool, String> {
        let start = {
            let mut state = self.inner.state.lock();
            let Some((version_id, scope_url, storage_key)) =
                active_registration_for_devtools_locked(&state, origin, registration_id)
            else {
                return Ok(false);
            };
            let Some(version) = state.versions.get(&version_id) else {
                return Ok(false);
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return Ok(false);
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerSyncEvent {
                event_id,
                registration_id,
                version_id,
                run: version.run.clone(),
                tag,
                last_chance,
            };
            self.start_sync_event_locked(&mut state, registration_id, scope_url, storage_key, event)
        };
        let accepted = !matches!(start, ServiceWorkerSyncStart::Dropped);
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
        Ok(accepted)
    }

    pub(crate) fn devtools_dispatch_periodic_sync_event(
        &self,
        origin: &Url,
        registration_id: ServiceWorkerRegistrationId,
        tag: String,
    ) -> Result<bool, String> {
        let start = {
            let mut state = self.inner.state.lock();
            let Some((version_id, scope_url, storage_key)) =
                active_registration_for_devtools_locked(&state, origin, registration_id)
            else {
                return Ok(false);
            };
            let Some(version) = state.versions.get(&version_id) else {
                return Ok(false);
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                return Ok(false);
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerPeriodicSyncEvent {
                event_id,
                registration_id,
                version_id,
                run: version.run.clone(),
                tag,
            };
            self.start_periodic_sync_event_locked(
                &mut state,
                registration_id,
                scope_url,
                storage_key,
                event,
            )
        };
        let accepted = !matches!(start, ServiceWorkerPeriodicSyncStart::Dropped);
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
        Ok(accepted)
    }
}

fn active_registration_for_devtools_locked(
    state: &ServiceWorkerRuntimeState,
    origin: &Url,
    registration_id: ServiceWorkerRegistrationId,
) -> Option<(ServiceWorkerVersionId, Url, String)> {
    let expected_storage_key =
        ServiceWorkerRegistrationKey::first_party_storage_key_for_url(origin);
    let registration = state.registrations.get(&registration_id)?;
    if registration.pending_unregistration || registration.storage_key != expected_storage_key {
        return None;
    }
    let version_id = registration.active_version_id?;
    Some((
        version_id,
        registration.scope_url.clone(),
        registration.storage_key.clone(),
    ))
}

fn stop_worker_version_locked(
    state: &mut ServiceWorkerRuntimeState,
    version_id: ServiceWorkerVersionId,
    reason: &'static str,
) -> Option<SharedRendererServiceWorkerHost> {
    let host = {
        let version = state.versions.get_mut(&version_id)?;
        let host = version.running_state.take_host_for_shutdown()?;
        version.idle_timeout_token = None;
        host
    };
    state.record_target_stopped(version_id, host.run_identity(), reason);
    Some(host)
}
