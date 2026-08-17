use super::*;
use moli_fetch::RequestCacheMode;

fn same_registration_options_fast_path_snapshot(
    state: &ServiceWorkerRuntimeState,
    registration: &ServiceWorkerRegistration,
    script_url: &Url,
    script_kind: WorkerScriptKind,
    update_via_cache: ServiceWorkerUpdateViaCache,
    main_script_update_check_available: bool,
) -> Option<ServiceWorkerRegistrationSnapshot> {
    registration.active_version_id?;
    if registration.update_via_cache != update_via_cache {
        return None;
    }
    let newest_version_id = registration
        .installing_version_id
        .or(registration.waiting_version_id)
        .or(registration.active_version_id)?;
    let newest_version = state.versions.get(&newest_version_id)?;
    if newest_version.script_url != *script_url || newest_version.script_kind != script_kind {
        return None;
    }
    if main_script_update_check_available && newest_version.main_script_resource.is_some() {
        return None;
    }
    Some(service_worker_registration_snapshot(state, registration))
}

impl ServiceWorkerRuntimeService {
    fn record_force_update_page_load_devtools_message_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        queued_job: &ServiceWorkerQueuedRegisterJob,
    ) {
        if queued_job.force_update_page_load_waiter_ids.is_empty() {
            return;
        }
        state.record_target_console_message(
            version_id,
            run,
            RendererServiceWorkerConsoleMessage {
                message: SERVICE_WORKER_FORCE_UPDATE_DEVTOOLS_CONSOLE_MESSAGE.to_owned(),
                args: Vec::new(),
                stack: None,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn start_registration(
        &self,
        script_url: Url,
        scope_url: Url,
        document_url: Url,
        script_kind: WorkerScriptKind,
        request_client: ResourceRequestClient,
        network_policy: WorkerNetworkPolicy,
        browser_context_runtime: RendererBrowserContextRuntime,
        broadcast_channel_top_level_site: Option<String>,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        update_via_cache: ServiceWorkerUpdateViaCache,
        register_request_id: u64,
        register_runtime_generation: u64,
        register_completion_tx: RendererPageServiceWorkerTaskSender,
    ) {
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        self.start_registration_with_storage_key(
            script_url,
            scope_url,
            document_url,
            storage_key,
            script_kind,
            request_client,
            network_policy,
            browser_context_runtime,
            broadcast_channel_top_level_site,
            indexed_db_manager,
            storage_bucket_store,
            update_via_cache,
            register_request_id,
            register_runtime_generation,
            register_completion_tx,
        );
    }

    pub(crate) fn start_registration_with_storage_key(
        &self,
        script_url: Url,
        scope_url: Url,
        document_url: Url,
        storage_key: String,
        script_kind: WorkerScriptKind,
        request_client: ResourceRequestClient,
        network_policy: WorkerNetworkPolicy,
        browser_context_runtime: RendererBrowserContextRuntime,
        broadcast_channel_top_level_site: Option<String>,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        update_via_cache: ServiceWorkerUpdateViaCache,
        register_request_id: u64,
        register_runtime_generation: u64,
        register_completion_tx: RendererPageServiceWorkerTaskSender,
    ) {
        let queued_job = ServiceWorkerQueuedRegisterJob {
            script_url,
            scope_url,
            document_url,
            storage_key,
            script_kind,
            update_via_cache,
            force_bypass_cache: false,
            skip_script_comparison: false,
            skip_waiting_after_install: false,
            force_update_page_load_waiter_ids: Vec::new(),
            request_client,
            network_policy,
            browser_context_runtime,
            broadcast_channel_top_level_site,
            indexed_db_manager,
            storage_bucket_store,
            callbacks: vec![ServiceWorkerRegisterJob {
                request_id: register_request_id,
                runtime_generation: register_runtime_generation,
                completion_tx: register_completion_tx,
            }],
        };
        self.start_queued_register_job(queued_job);
    }

    pub(super) fn start_queued_register_job(&self, queued_job: ServiceWorkerQueuedRegisterJob) {
        let (launch, update_check, completed_register_callbacks) = {
            let mut state = self.inner.state.lock();
            let registration_key = queued_job.registration_key();
            self.restore_stored_registration_for_queued_job_locked(&mut state, &queued_job);
            let registration_id = state
                .registrations
                .values()
                .find(|registration| registration.key() == registration_key)
                .map(|registration| registration.id)
                .unwrap_or_else(|| {
                    ServiceWorkerRegistrationId(
                        self.inner
                            .next_registration_id
                            .fetch_add(1, Ordering::Relaxed),
                    )
                });
            state
                .registrations
                .entry(registration_id)
                .or_insert_with(|| ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: registration_key.storage_key.clone(),
                    scope_url: queued_job.scope_url.clone(),
                    script_url: queued_job.script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: None,
                    pending_unregistration: false,
                    update_via_cache: queued_job.update_via_cache,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                });
            let (
                installing_version_id,
                pending_unregistration,
                registration_scope_url,
                registration_update_via_cache,
            ) = {
                let Some(registration) = state.registrations.get(&registration_id) else {
                    return;
                };
                (
                    registration.installing_version_id,
                    registration.pending_unregistration,
                    registration.scope_url.clone(),
                    registration.update_via_cache,
                )
            };
            if pending_unregistration
                && let Some(registration) = state.registrations.get_mut(&registration_id)
            {
                registration.pending_unregistration = false;
            }
            let can_coalesce_with_installing = installing_version_id
                .and_then(|version_id| state.versions.get(&version_id))
                .is_some_and(|version| {
                    version.script_url == queued_job.script_url
                        && version.script_kind == queued_job.script_kind
                        && registration_scope_url == queued_job.scope_url
                        && registration_update_via_cache == queued_job.update_via_cache
                });
            let pending_update_check_matches = state
                .pending_main_script_update_checks
                .get(&registration_id)
                .map(|pending_update_check| {
                    pending_update_check.matches_registration_job(&queued_job)
                });
            if let Some(matches_pending_update_check) = pending_update_check_matches {
                if matches_pending_update_check {
                    state
                        .pending_main_script_update_checks
                        .get_mut(&registration_id)
                        .expect("pending update check should exist")
                        .append_callbacks_from(queued_job);
                } else {
                    state
                        .job_coordinator
                        .enqueue_register(registration_key, queued_job);
                }
                (None, None, None)
            } else if can_coalesce_with_installing {
                let callbacks = queued_job.callbacks;
                let Some(installing_version_id) = installing_version_id else {
                    return;
                };
                state.bind_force_update_page_load_waiters(
                    installing_version_id,
                    queued_job.force_update_page_load_waiter_ids.clone(),
                );
                if let Some(pending_job) =
                    state
                        .registrations
                        .get_mut(&registration_id)
                        .and_then(|registration| {
                            registration
                                .pending_register_jobs
                                .get_mut(&installing_version_id)
                        })
                {
                    (
                        None,
                        None,
                        pending_job.add_callbacks(callbacks, queued_job.skip_waiting_after_install),
                    )
                } else {
                    let snapshot =
                        state
                            .registrations
                            .get(&registration_id)
                            .and_then(|registration| {
                                if !registration.references_version(installing_version_id) {
                                    return None;
                                }
                                Some(service_worker_registration_snapshot(&state, registration))
                            });
                    (
                        None,
                        None,
                        snapshot.map(|snapshot| (callbacks, Ok(snapshot))),
                    )
                }
            } else if installing_version_id.is_some() {
                state
                    .job_coordinator
                    .enqueue_register(registration_key, queued_job);
                (None, None, None)
            } else if !queued_job.skip_script_comparison
                && let Some(snapshot) =
                    state
                        .registrations
                        .get(&registration_id)
                        .and_then(|registration| {
                            same_registration_options_fast_path_snapshot(
                                &state,
                                registration,
                                &queued_job.script_url,
                                queued_job.script_kind,
                                queued_job.update_via_cache,
                                true,
                            )
                        })
            {
                let callbacks = queued_job.callbacks;
                (None, None, Some((callbacks, Ok(snapshot))))
            } else if let Some(update_check) = self.start_main_script_update_check_locked(
                &mut state,
                registration_id,
                queued_job.clone(),
            ) {
                match update_check {
                    ServiceWorkerMainScriptUpdateCheckStart::Start(update_check) => {
                        (None, Some(*update_check), None)
                    }
                    ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => (None, None, None),
                }
            } else {
                (
                    self.start_queued_registration_now(&mut state, registration_id, queued_job),
                    None,
                    None,
                )
            }
        };
        if let Some((callbacks, result)) = completed_register_callbacks {
            ServiceWorkerRegisterJob::send_all(callbacks, result);
        }
        if let Some((registration_id, load_params)) = update_check {
            self.start_main_script_update_check(registration_id, load_params);
        }
        if let Some(launch) = launch {
            self.start_queued_launch(launch);
        }
    }

    #[cfg(test)]
    pub(super) fn mark_registration_unregistered(&self, scope_url: &Url) -> bool {
        match self.start_unregistration_job(
            scope_url,
            ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url),
            None,
        ) {
            ServiceWorkerUnregisterStart::Completed(result) => result,
            ServiceWorkerUnregisterStart::Queued => true,
        }
    }

    #[cfg(test)]
    pub(crate) fn start_unregistration(
        &self,
        scope_url: &Url,
        request_id: u64,
        runtime_generation: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerUnregisterStart {
        self.start_unregistration_with_storage_key(
            scope_url,
            ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url),
            request_id,
            runtime_generation,
            completion_tx,
        )
    }

    pub(crate) fn start_unregistration_with_storage_key(
        &self,
        scope_url: &Url,
        storage_key: String,
        request_id: u64,
        runtime_generation: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerUnregisterStart {
        self.start_unregistration_job(
            scope_url,
            storage_key,
            Some(ServiceWorkerUnregisterJob {
                request_id,
                runtime_generation,
                completion_tx,
            }),
        )
    }

    pub(super) fn start_unregistration_job(
        &self,
        scope_url: &Url,
        storage_key: String,
        job: Option<ServiceWorkerUnregisterJob>,
    ) -> ServiceWorkerUnregisterStart {
        let (start, progress) = {
            let mut state = self.inner.state.lock();
            let registration_key =
                ServiceWorkerRegistrationKey::for_scope_and_storage_key(scope_url, storage_key);
            let Some(registration_id) = state
                .registrations
                .values()
                .find(|registration| registration.key() == registration_key)
                .map(|registration| registration.id)
            else {
                return ServiceWorkerUnregisterStart::Completed(false);
            };
            let Some(registration) = state.registrations.get(&registration_id) else {
                return ServiceWorkerUnregisterStart::Completed(false);
            };
            let register_job_in_progress = registration.installing_version_id.is_some()
                || state
                    .pending_main_script_update_checks
                    .contains_key(&registration_id);
            if registration.pending_unregistration
                || state
                    .job_coordinator
                    .has_queued_unregistration(&registration_key)
            {
                if register_job_in_progress && job.is_some() {
                    state
                        .job_coordinator
                        .enqueue_unregister(registration_key, job);
                    return ServiceWorkerUnregisterStart::Queued;
                }
                return ServiceWorkerUnregisterStart::Completed(false);
            }
            if register_job_in_progress {
                state
                    .job_coordinator
                    .enqueue_unregister(registration_key, job);
                (ServiceWorkerUnregisterStart::Queued, Vec::new())
            } else {
                let Some(registration) = state.registrations.get_mut(&registration_id) else {
                    return ServiceWorkerUnregisterStart::Completed(false);
                };
                registration.pending_unregistration = true;
                let progress = self.unregistration_progress_for_registration_if_ready_locked(
                    &mut state,
                    registration_id,
                );
                (ServiceWorkerUnregisterStart::Completed(true), progress)
            }
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
        start
    }

    pub(super) fn cleanup_failed_install_version_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        failed_version_id: ServiceWorkerVersionId,
    ) -> Vec<LifecycleProgress> {
        if registration_has_installed_version_locked(state, registration_id) {
            let progress = remove_version_and_shutdown_host_locked(state, failed_version_id);
            return progress;
        }

        let Some(registration_key) = state
            .registrations
            .get(&registration_id)
            .map(ServiceWorkerRegistration::key)
        else {
            let progress = remove_version_and_shutdown_host_locked(state, failed_version_id);
            return progress;
        };

        self.delete_registration_resources_for_key_locked(&registration_key);
        if state.job_coordinator.has_jobs(&registration_key) {
            let mut progress = execute_pending_clear_locked(
                state,
                registration_id,
                ServiceWorkerPendingClearAction::KeepRegistrationForQueuedJob,
            );
            progress.extend(self.advance_registration_job_queue_locked(state, registration_id));
            return progress;
        }

        execute_pending_clear_locked(
            state,
            registration_id,
            ServiceWorkerPendingClearAction::DeleteRegistration,
        )
    }

    #[cfg(test)]
    pub(super) fn store_registration_resources_for_test(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Result<bool, ServiceWorkerRegistrationError> {
        let state = self.inner.state.lock();
        self.store_registration_resources_locked(&state, registration_id, version_id)
    }

    pub(super) fn start_main_script_update_check_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        queued_job: ServiceWorkerQueuedRegisterJob,
    ) -> Option<ServiceWorkerMainScriptUpdateCheckStart> {
        let registration = state.registrations.get(&registration_id)?;
        if registration.installing_version_id.is_some()
            || registration.active_version_id.is_none()
            || registration.update_via_cache != queued_job.update_via_cache
        {
            return None;
        }
        let newest_version_id = registration
            .waiting_version_id
            .or(registration.active_version_id)?;
        let newest_version = state.versions.get(&newest_version_id)?;
        let newest_resource = newest_version.main_script_resource.as_ref()?;
        if newest_version.script_url != queued_job.script_url
            || newest_version.script_kind != queued_job.script_kind
        {
            return None;
        }
        let newest_body_sha256 = newest_resource.body_sha256.clone();
        let imported_scripts = newest_version
            .imported_script_resources
            .values()
            .cloned()
            .collect();
        let (new_version_id, _, _, _) =
            self.create_installing_version_locked(state, registration_id, &queued_job, false)?;
        state.bind_force_update_page_load_waiters(
            new_version_id,
            queued_job.force_update_page_load_waiter_ids.clone(),
        );
        let load_params = ServiceWorkerScriptLoadParams {
            script_url: queued_job.script_url.clone(),
            scope_url: queued_job.scope_url.clone(),
            document_url: queued_job.document_url.clone(),
            request_client: queued_job.request_client.clone(),
            cache_mode: main_script_update_check_cache_mode(
                queued_job.update_via_cache,
                queued_job.force_bypass_cache,
            ),
        };
        let update_check_params = ServiceWorkerScriptUpdateCheckParams {
            main_script: load_params,
            newest_main_body_sha256: newest_body_sha256.clone(),
            imported_scripts,
            imported_script_cache_mode: imported_script_update_check_cache_mode(
                queued_job.update_via_cache,
                queued_job.force_bypass_cache,
            ),
            skip_script_comparison: queued_job.skip_script_comparison,
        };
        let pause_until_debugger =
            Self::version_should_pause_on_start_for_devtools_locked(state, new_version_id);
        let mut pending_update_check = ServiceWorkerPendingMainScriptUpdateCheck::new(
            queued_job,
            newest_version_id,
            newest_body_sha256,
            new_version_id,
        );
        if pause_until_debugger {
            pending_update_check.defer_until_debugger(update_check_params.clone());
        }
        state
            .pending_main_script_update_checks
            .insert(registration_id, pending_update_check);
        if pause_until_debugger {
            Some(ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger)
        } else {
            Some(ServiceWorkerMainScriptUpdateCheckStart::Start(Box::new((
                registration_id,
                update_check_params,
            ))))
        }
    }

    pub(super) fn start_main_script_update_check(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        load_params: ServiceWorkerScriptUpdateCheckParams,
    ) {
        let service_for_task = self.clone();
        let spawn_result = std::thread::Builder::new()
            .name(format!(
                "service-worker-update-check-{}",
                registration_id.as_u64()
            ))
            .spawn(move || {
                let result = load_service_worker_script_update_check(&load_params);
                service_for_task
                    .enqueue_main_script_update_check_completed(registration_id, result);
            });
        if let Err(error) = spawn_result {
            self.enqueue_main_script_update_check_completed(
                registration_id,
                Err(ServiceWorkerScriptUpdateCheckFailure::internal(format!(
                    "service worker script update check failed to start: {error}"
                ))),
            );
        }
    }

    fn start_queued_registration_now(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        queued_job: ServiceWorkerQueuedRegisterJob,
    ) -> Option<ServiceWorkerQueuedLaunch> {
        self.start_queued_registration_now_with_preloaded_script(
            state,
            registration_id,
            queued_job,
            None,
        )
    }

    pub(super) fn start_queued_registration_now_with_preloaded_script(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        mut queued_job: ServiceWorkerQueuedRegisterJob,
        preloaded_script: Option<LoadedServiceWorkerScript>,
    ) -> Option<ServiceWorkerQueuedLaunch> {
        let allow_identical_script_update = preloaded_script.is_none();
        let (version_id, run, launch_config, host) = self.create_installing_version_locked(
            state,
            registration_id,
            &queued_job,
            allow_identical_script_update,
        )?;
        state.bind_force_update_page_load_waiters(
            version_id,
            queued_job.force_update_page_load_waiter_ids.clone(),
        );
        self.record_force_update_page_load_devtools_message_locked(
            state,
            version_id,
            host.run_identity(),
            &queued_job,
        );
        let register_callbacks = std::mem::take(&mut queued_job.callbacks);
        let mut pending_register_job = ServiceWorkerPendingRegisterJob::new_with_options(
            register_callbacks,
            queued_job.skip_waiting_after_install,
        );
        pending_register_job.start_current_moli_job();
        let registration = state.registrations.get_mut(&registration_id)?;
        registration
            .pending_register_jobs
            .insert(version_id, pending_register_job);
        let lifecycle_notifications = lifecycle_notifications_for_registration_locked(
            state,
            registration_id,
            vec![ServiceWorkerLifecycleClientEvent::UpdateFound],
        );
        let request_client = launch_config.request_client();
        Some(ServiceWorkerQueuedLaunch {
            params: ServiceWorkerLaunchParams {
                registration_id,
                version_id,
                run,
                script_url: queued_job.script_url,
                scope_url: queued_job.scope_url,
                storage_key: queued_job.storage_key,
                document_url: launch_config.document_url,
                script_kind: queued_job.script_kind,
                request_client,
                network_policy: launch_config.network_policy,
                worker_context_runtime: launch_config.worker_context_runtime,
                broadcast_channel_top_level_site: launch_config.broadcast_channel_top_level_site,
                indexed_db_manager: launch_config.indexed_db_manager,
                storage_bucket_store: launch_config.storage_bucket_store,
                pause_evaluation_until_debugger: false,
            },
            host,
            lifecycle_notifications,
            preloaded_script,
        })
    }

    pub(super) fn start_precreated_queued_registration_with_preloaded_script(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        new_version_id: ServiceWorkerVersionId,
        mut queued_job: ServiceWorkerQueuedRegisterJob,
        preloaded_script: LoadedServiceWorkerScript,
    ) -> Option<ServiceWorkerQueuedLaunch> {
        let registration = state.registrations.get_mut(&registration_id)?;
        if registration.installing_version_id != Some(new_version_id) {
            return None;
        }
        let version = state.versions.get(&new_version_id)?;
        if version.registration_id != registration_id
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Installing
            || version.script_url != queued_job.script_url
            || version.script_kind != queued_job.script_kind
        {
            return None;
        }
        let ServiceWorkerVersionRunningState::Starting { host } = &version.running_state else {
            return None;
        };
        let host = host.clone();
        let run = version.run.clone();
        let launch_config = version.launch_config.clone();
        let register_callbacks = std::mem::take(&mut queued_job.callbacks);
        let mut pending_register_job = ServiceWorkerPendingRegisterJob::new_with_options(
            register_callbacks,
            queued_job.skip_waiting_after_install,
        );
        pending_register_job.start_current_moli_job();
        registration
            .pending_register_jobs
            .insert(new_version_id, pending_register_job);
        self.record_force_update_page_load_devtools_message_locked(
            state,
            new_version_id,
            host.run_identity(),
            &queued_job,
        );
        let lifecycle_notifications = lifecycle_notifications_for_registration_locked(
            state,
            registration_id,
            vec![ServiceWorkerLifecycleClientEvent::UpdateFound],
        );
        let request_client = launch_config.request_client();
        Some(ServiceWorkerQueuedLaunch {
            params: ServiceWorkerLaunchParams {
                registration_id,
                version_id: new_version_id,
                run,
                script_url: queued_job.script_url,
                scope_url: queued_job.scope_url,
                storage_key: queued_job.storage_key,
                document_url: launch_config.document_url,
                script_kind: queued_job.script_kind,
                request_client,
                network_policy: launch_config.network_policy,
                worker_context_runtime: launch_config.worker_context_runtime,
                broadcast_channel_top_level_site: launch_config.broadcast_channel_top_level_site,
                indexed_db_manager: launch_config.indexed_db_manager,
                storage_bucket_store: launch_config.storage_bucket_store,
                pause_evaluation_until_debugger: false,
            },
            host,
            lifecycle_notifications,
            preloaded_script: Some(preloaded_script),
        })
    }

    pub(super) fn create_installing_version_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        queued_job: &ServiceWorkerQueuedRegisterJob,
        allow_identical_script_update: bool,
    ) -> Option<(
        ServiceWorkerVersionId,
        RendererServiceWorkerRunIdentity,
        ServiceWorkerVersionLaunchConfig,
        SharedRendererServiceWorkerHost,
    )> {
        let version_id = loop {
            let id =
                ServiceWorkerVersionId(self.inner.next_version_id.fetch_add(1, Ordering::Relaxed));
            if !state.versions.contains_key(&id) {
                break id;
            }
        };
        let run = RendererServiceWorkerRunIdentity::fresh();
        let registration = state.registrations.get_mut(&registration_id)?;
        registration.script_url = queued_job.script_url.clone();
        registration.scope_url = queued_job.scope_url.clone();
        registration.update_via_cache = queued_job.update_via_cache;
        registration.installing_version_id = Some(version_id);
        registration.pending_unregistration = false;
        let launch_config = ServiceWorkerVersionLaunchConfig::from_queued_register_job(queued_job);
        let host = RendererServiceWorkerHost::new_loading(version_id, &run);
        let should_pause_on_start_for_devtools = self
            .should_pause_new_worker_on_start_for_devtools_locked(
                state,
                registration_id,
                version_id,
                &queued_job.script_url,
                &queued_job.scope_url,
            );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: queued_job.script_url.clone(),
                final_script_url: None,
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update,
                should_pause_on_start_for_devtools,
                script_kind: queued_job.script_kind,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: launch_config.clone(),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Starting { host: host.clone() },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        state.record_target_created(
            registration_id,
            version_id,
            queued_job.script_url.clone(),
            queued_job.scope_url.clone(),
        );
        Some((version_id, run, launch_config, host))
    }

    pub(super) fn cleanup_precreated_update_check_version_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Vec<LifecycleProgress> {
        if let Some(registration) = state.registrations.get_mut(&registration_id)
            && registration.installing_version_id == Some(version_id)
        {
            registration.installing_version_id = None;
            registration.pending_register_jobs.remove(&version_id);
        }
        let mut progress = vec![LifecycleProgress::ForceUpdatePageLoadCompleted(
            state.take_force_update_page_load_waiters_for_version(version_id),
        )];
        progress.extend(remove_version_and_shutdown_host_locked(state, version_id));
        progress
    }

    pub(super) fn advance_registration_job_queue_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
    ) -> Vec<LifecycleProgress> {
        let mut progress = Vec::new();
        loop {
            let registration_key = {
                let Some(registration) = state.registrations.get(&registration_id) else {
                    return progress;
                };
                if registration.installing_version_id.is_some()
                    || state
                        .pending_main_script_update_checks
                        .contains_key(&registration_id)
                {
                    return progress;
                }
                registration.key()
            };
            let Some(queued_job) = state.job_coordinator.pop_next(&registration_key) else {
                return progress;
            };
            match queued_job {
                ServiceWorkerQueuedJob::Register(queued_job) => {
                    let queued_job = *queued_job;
                    if let Some(update_check) = self.start_main_script_update_check_locked(
                        state,
                        registration_id,
                        queued_job.clone(),
                    ) {
                        if let ServiceWorkerMainScriptUpdateCheckStart::Start(update_check) =
                            update_check
                        {
                            progress
                                .push(LifecycleProgress::StartMainScriptUpdateCheck(update_check));
                        }
                    } else if let Some(launch) =
                        self.start_queued_registration_now(state, registration_id, queued_job)
                    {
                        progress.push(LifecycleProgress::StartWorker(Box::new(launch)));
                    }
                    return progress;
                }
                ServiceWorkerQueuedJob::Unregister(mut queued_job) => {
                    let Some(registration) = state.registrations.get_mut(&registration_id) else {
                        return progress;
                    };
                    queued_job.mark_pending();
                    registration.pending_unregistration = true;
                    progress.push(LifecycleProgress::UnregisterCompleted(queued_job));
                    progress.extend(
                        self.unregistration_progress_for_registration_if_ready_locked(
                            state,
                            registration_id,
                        ),
                    );
                    if state
                        .registrations
                        .get(&registration_id)
                        .is_some_and(|registration| registration.pending_unregistration)
                    {
                        return progress;
                    }
                }
            }
        }
    }
}

fn main_script_update_check_cache_mode(
    update_via_cache: ServiceWorkerUpdateViaCache,
    force_bypass_cache: bool,
) -> RequestCacheMode {
    if force_bypass_cache {
        return RequestCacheMode::Validate;
    }
    match update_via_cache {
        ServiceWorkerUpdateViaCache::All => RequestCacheMode::Default,
        ServiceWorkerUpdateViaCache::Imports | ServiceWorkerUpdateViaCache::None => {
            RequestCacheMode::Validate
        }
    }
}

fn imported_script_update_check_cache_mode(
    update_via_cache: ServiceWorkerUpdateViaCache,
    force_bypass_cache: bool,
) -> RequestCacheMode {
    if force_bypass_cache {
        return RequestCacheMode::Validate;
    }
    match update_via_cache {
        ServiceWorkerUpdateViaCache::All | ServiceWorkerUpdateViaCache::Imports => {
            RequestCacheMode::Default
        }
        ServiceWorkerUpdateViaCache::None => RequestCacheMode::Validate,
    }
}

fn registration_has_installed_version_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) -> bool {
    let Some(registration) = state.registrations.get(&registration_id) else {
        return false;
    };
    [
        registration.waiting_version_id,
        registration.active_version_id,
    ]
    .into_iter()
    .flatten()
    .any(|version_id| {
        state.versions.get(&version_id).is_some_and(|version| {
            matches!(
                version.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Installed
                    | ServiceWorkerVersionLifecycleState::Activating
                    | ServiceWorkerVersionLifecycleState::Activated
            )
        })
    })
}

pub(super) fn remove_version_and_shutdown_host_locked(
    state: &mut ServiceWorkerRuntimeState,
    version_id: ServiceWorkerVersionId,
) -> Vec<LifecycleProgress> {
    state.pending_devtools_launches.remove(&version_id);
    state
        .pending_devtools_evaluation_releases
        .remove(&version_id);
    state.record_target_destroyed(version_id);
    state
        .versions
        .remove(&version_id)
        .and_then(|mut version| version.running_state.take_host_for_shutdown())
        .map(LifecycleProgress::TerminateHost)
        .into_iter()
        .collect()
}
