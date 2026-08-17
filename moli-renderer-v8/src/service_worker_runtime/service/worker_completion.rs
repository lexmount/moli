use super::*;

impl ServiceWorkerRuntimeService {
    #[cfg(test)]
    pub(super) fn finish_worker_start_completed(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        final_script_url: String,
    ) {
        self.finish_worker_start_completed_with_script_resource(
            version_id,
            run,
            final_script_url,
            None,
            ServiceWorkerFetchHandlerType::NotSkippable,
        );
    }

    pub(super) fn finish_worker_start_completed_with_script_resource(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        final_script_url: String,
        script_resource: Option<ServiceWorkerScriptResource>,
        fetch_handler_type: ServiceWorkerFetchHandlerType,
    ) {
        let Ok(final_script_url) = Url::parse(&final_script_url) else {
            self.finish_worker_start_failed(
                version_id,
                run,
                ServiceWorkerVersionStartFailure::Bootstrap {
                    failure: WorkerBootstrapFailure {
                        message: "service worker final script URL is invalid".to_owned(),
                        filename: final_script_url,
                        lineno: 0,
                        colno: 0,
                        event_kind: crate::worker::WorkerParentErrorEventKind::Event,
                        phase: crate::worker::WorkerErrorPhase::Bootstrap,
                        source: crate::worker::WorkerErrorSource::Runtime,
                    },
                },
            );
            return;
        };
        let loaded_main_script_resource = script_resource.is_some();
        let (lifecycle_start, register_completion, pending_events) = {
            let mut state = self.inner.state.lock();
            let (registration_id, lifecycle_state, pending_events, run) = {
                let Some(version) = state.versions.get_mut(&version_id) else {
                    return;
                };
                if version.run != run {
                    return;
                }
                let ServiceWorkerVersionRunningState::Starting { host } =
                    &mut version.running_state
                else {
                    return;
                };
                if host.version_id() != version_id || host.run_identity() != run {
                    return;
                }
                version.final_script_url = Some(final_script_url);
                version.main_script_resource = script_resource;
                version.fetch_handler_existence = match fetch_handler_type {
                    ServiceWorkerFetchHandlerType::NoHandler => {
                        ServiceWorkerFetchHandlerExistence::DoesNotExist
                    }
                    ServiceWorkerFetchHandlerType::NotSkippable
                    | ServiceWorkerFetchHandlerType::EmptyFetchHandler => {
                        ServiceWorkerFetchHandlerExistence::Exists
                    }
                };
                version.fetch_handler_type = fetch_handler_type;
                version.last_start_error = None;
                let run = host.run_identity();
                let host = host.clone();
                version.running_state = ServiceWorkerVersionRunningState::Running { host };
                let pending_events = version.pending_start_events.drain(..).collect::<Vec<_>>();
                (
                    version.registration_id,
                    version.lifecycle_state,
                    pending_events,
                    run,
                )
            };
            let installing_version_is_current = state
                .registrations
                .get(&registration_id)
                .is_some_and(|registration| registration.installing_version_id == Some(version_id));
            if lifecycle_state == ServiceWorkerVersionLifecycleState::Installing
                && loaded_main_script_resource
                && installing_version_is_current
            {
                bump_registration_last_update_check_time_locked(&mut state, registration_id);
            }

            let lifecycle_start =
                if lifecycle_state == ServiceWorkerVersionLifecycleState::Installing {
                    self.lifecycle_event_for_version_locked(
                        &mut state,
                        version_id,
                        ServiceWorkerLifecycleEventKind::Install,
                    )
                } else {
                    None
                };
            let register_completion = if lifecycle_start.is_some() {
                let snapshot = state
                    .registrations
                    .get(&registration_id)
                    .filter(|registration| registration.references_version(version_id))
                    .map(|registration| service_worker_registration_snapshot(&state, registration));
                snapshot.and_then(|snapshot| {
                    state
                        .registrations
                        .get_mut(&registration_id)
                        .and_then(|registration| {
                            registration.pending_register_jobs.get_mut(&version_id)
                        })
                        .map(|pending_job| {
                            let callbacks = pending_job.complete_install_started(snapshot.clone());
                            (callbacks, snapshot)
                        })
                })
            } else {
                None
            };
            state.record_target_started(version_id, run);
            (lifecycle_start, register_completion, pending_events)
        };
        if let Some(start) = lifecycle_start
            && let Some(progress) = Self::lifecycle_start_to_progress(start)
        {
            self.run_lifecycle_progress(progress);
        }
        if let Some((callbacks, snapshot)) = register_completion
            && !callbacks.is_empty()
        {
            ServiceWorkerRegisterJob::send_all(callbacks, Ok(snapshot));
        }
        for event in pending_events {
            match event {
                ServiceWorkerPendingStartEvent::Fetch(event) => {
                    let dispatch = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            if version.fetch_handler_existence
                                != ServiceWorkerFetchHandlerExistence::Unknown
                                && version.fetch_handler_type.allows_fetch_event_skip()
                            {
                                return Some(Err(()));
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(Ok(host.clone()))
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    match dispatch {
                        Some(Ok(host)) => {
                            self.dispatch_fetch_event(host, event);
                        }
                        Some(Err(())) => {
                            self.enqueue_fetch_event_completed(ServiceWorkerFetchCompletion {
                                event_id: event.event_id,
                                version_id: event.version_id,
                                run: event.run.clone(),
                                result: ServiceWorkerFetchResult::Fallback,
                            });
                        }
                        None => {
                            self.enqueue_fetch_event_completed(ServiceWorkerFetchCompletion {
                                event_id: event.event_id,
                                version_id: event.version_id,
                                run: event.run.clone(),
                                result: ServiceWorkerFetchResult::Failure(
                                    "service worker fetch dispatch failed: worker did not start"
                                        .to_owned(),
                                ),
                            });
                        }
                    }
                }
                ServiceWorkerPendingStartEvent::Lifecycle(event) => {
                    let host = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(host.clone())
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    if let Some(host) = host {
                        self.dispatch_lifecycle_event(host, event);
                    } else {
                        self.enqueue_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
                            event_id: event.event_id,
                            version_id: event.version_id,
                            run: event.run.clone(),
                            kind: event.kind,
                            result: Err(
                                "service worker lifecycle dispatch failed: worker did not start"
                                    .to_owned(),
                            ),
                        });
                    }
                }
                ServiceWorkerPendingStartEvent::Message(event) => {
                    let host = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(host.clone())
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    if let Some(host) = host {
                        self.dispatch_message_event(host, event);
                    } else {
                        self.enqueue_message_event_completed(ServiceWorkerMessageCompletion {
                            event_id: event.event_id,
                            version_id: event.version_id,
                            run: event.run.clone(),
                            result: Err(
                                "service worker message dispatch failed: worker did not start"
                                    .to_owned(),
                            ),
                        });
                    }
                }
                ServiceWorkerPendingStartEvent::Notification(event) => {
                    let host = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(host.clone())
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    if let Some(host) = host {
                        self.dispatch_notification_event_to_host(host, event);
                    } else {
                        self.enqueue_notification_event_completed(
                            ServiceWorkerNotificationCompletion {
                                event_id: event.event_id,
                                version_id: event.version_id,
                                run: event.run.clone(),
                                result: Err(
                                    "service worker notification dispatch failed: worker did not start"
                                        .to_owned(),
                                ),
                            },
                        );
                    }
                }
                ServiceWorkerPendingStartEvent::Push(event) => {
                    let host = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(host.clone())
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    if let Some(host) = host {
                        self.dispatch_push_event_to_host(host, event);
                    } else {
                        self.enqueue_push_event_completed(ServiceWorkerPushCompletion {
                            event_id: event.event_id,
                            version_id: event.version_id,
                            run: event.run.clone(),
                            result: Err(
                                "service worker push dispatch failed: worker did not start"
                                    .to_owned(),
                            ),
                        });
                    }
                }
                ServiceWorkerPendingStartEvent::Sync(event) => {
                    let host = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(host.clone())
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    if let Some(host) = host {
                        self.dispatch_sync_event_to_host(host, event);
                    } else {
                        self.enqueue_sync_event_completed(ServiceWorkerSyncCompletion {
                            event_id: event.event_id,
                            registration_id: event.registration_id,
                            version_id: event.version_id,
                            run: event.run.clone(),
                            tag: event.tag,
                            result: Err(
                                "service worker sync dispatch failed: worker did not start"
                                    .to_owned(),
                            ),
                        });
                    }
                }
                ServiceWorkerPendingStartEvent::PeriodicSync(event) => {
                    let host = {
                        let state = self.inner.state.lock();
                        state.versions.get(&event.version_id).and_then(|version| {
                            if version.run != event.run {
                                return None;
                            }
                            match &version.running_state {
                                ServiceWorkerVersionRunningState::Running { host } => {
                                    Some(host.clone())
                                }
                                ServiceWorkerVersionRunningState::Stopped
                                | ServiceWorkerVersionRunningState::Starting { .. } => None,
                            }
                        })
                    };
                    if let Some(host) = host {
                        self.dispatch_periodic_sync_event_to_host(host, event);
                    } else {
                        self.enqueue_periodic_sync_event_completed(
                            ServiceWorkerPeriodicSyncCompletion {
                                event_id: event.event_id,
                                registration_id: event.registration_id,
                                version_id: event.version_id,
                                run: event.run.clone(),
                                tag: event.tag,
                                result: Err(
                                    "service worker periodic sync dispatch failed: worker did not start"
                                        .to_owned(),
                                ),
                            },
                        );
                    }
                }
            }
        }
    }

    pub(in crate::service_worker_runtime) fn finish_worker_start_identical_script_update(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        loaded_script_resource: &ServiceWorkerScriptResource,
    ) -> bool {
        let (register_completion, queue_progress, force_update_page_load_waiters) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return false;
            };
            if version.run != run
                || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Installing
                || !version.allow_identical_script_update
            {
                return false;
            }
            let ServiceWorkerVersionRunningState::Starting { host } = &version.running_state else {
                return false;
            };
            if host.version_id() != version_id || host.run_identity() != run {
                return false;
            }
            let registration_id = version.registration_id;
            let script_url = version.script_url.clone();
            let script_kind = version.script_kind;
            let Some(registration) = state.registrations.get(&registration_id) else {
                return false;
            };
            if registration.installing_version_id != Some(version_id)
                || registration.active_version_id.is_none()
            {
                return false;
            }
            let newest_existing_version_id = registration
                .waiting_version_id
                .or(registration.active_version_id)
                .filter(|candidate| *candidate != version_id);
            let Some(newest_existing_version) =
                newest_existing_version_id.and_then(|id| state.versions.get(&id))
            else {
                return false;
            };
            let Some(existing_resource) = newest_existing_version.main_script_resource.as_ref()
            else {
                return false;
            };
            if newest_existing_version.script_url != script_url
                || newest_existing_version.script_kind != script_kind
                || existing_resource.body_sha256 != loaded_script_resource.body_sha256
            {
                return false;
            }

            bump_registration_last_update_check_time_locked(&mut state, registration_id);
            let Some(registration) = state.registrations.get_mut(&registration_id) else {
                return false;
            };
            registration.installing_version_id = None;
            let Some(snapshot) = state
                .registrations
                .get(&registration_id)
                .map(|registration| service_worker_registration_snapshot(&state, registration))
            else {
                return false;
            };
            let force_update_page_load_waiters =
                state.take_force_update_page_load_waiters_for_version(version_id);
            let register_completion = state
                .registrations
                .get_mut(&registration_id)
                .and_then(|registration| registration.pending_register_jobs.remove(&version_id))
                .map(|mut pending_job| {
                    let callbacks = pending_job.complete_without_install(snapshot.clone());
                    (callbacks, snapshot)
                });
            state.record_target_destroyed(version_id);
            state.versions.remove(&version_id);
            let queue_progress =
                self.advance_registration_job_queue_locked(&mut state, registration_id);
            (
                register_completion,
                queue_progress,
                force_update_page_load_waiters,
            )
        };

        if let Some((jobs, snapshot)) = register_completion {
            ServiceWorkerRegisterJob::send_all(jobs, Ok(snapshot));
        }
        for waiter in force_update_page_load_waiters {
            let _ = waiter.send(());
        }
        for progress in queue_progress {
            self.run_lifecycle_progress(progress);
        }
        true
    }

    pub(super) fn finish_main_script_update_check_completed(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        result: ServiceWorkerScriptUpdateCheckCompletion,
    ) {
        let (register_completion, launch, queue_progress) = {
            let mut state = self.inner.state.lock();
            let Some(pending_check) = state
                .pending_main_script_update_checks
                .remove(&registration_id)
            else {
                return;
            };
            let diagnostic_script_url = pending_check.queued_job.script_url.to_string();
            let diagnostic_newest_version_id = pending_check.newest_version_id;
            let record_update_check =
                |state: &mut ServiceWorkerRuntimeState,
                 result: &'static str,
                 failure_status: Option<&'static str>,
                 message: Option<String>,
                 imported_script_url: Option<String>| {
                    state.main_script_update_check_diagnostics.insert(
                        registration_id,
                        ServiceWorkerMainScriptUpdateCheckDiagnostics {
                            script_url: diagnostic_script_url.clone(),
                            newest_version_id: diagnostic_newest_version_id,
                            result,
                            failure_status,
                            message,
                            imported_script_url,
                        },
                    );
                };
            let new_version_id = pending_check.new_version_id;
            let mut queued_job = pending_check.queued_job;
            let stale_error = || {
                ServiceWorkerScriptUpdateCheckFailure::stale(
                    "service worker main script update check became stale".to_owned(),
                )
            };
            let cleanup_precreated_version = |state: &mut ServiceWorkerRuntimeState| {
                self.cleanup_precreated_update_check_version_locked(
                    state,
                    registration_id,
                    new_version_id,
                )
            };
            let cleanup_precreated_version_and_advance_queue =
                |state: &mut ServiceWorkerRuntimeState| {
                    let mut progress = cleanup_precreated_version(state);
                    progress
                        .extend(self.advance_registration_job_queue_locked(state, registration_id));
                    progress
                };
            let registration_is_current =
                state
                    .registrations
                    .get(&registration_id)
                    .is_some_and(|registration| {
                        registration
                            .waiting_version_id
                            .or(registration.active_version_id)
                            == Some(pending_check.newest_version_id)
                    });
            let newest_resource_is_current = state
                .versions
                .get(&pending_check.newest_version_id)
                .and_then(|version| version.main_script_resource.as_ref())
                .is_some_and(|resource| resource.body_sha256 == pending_check.newest_body_sha256);
            if !registration_is_current || !newest_resource_is_current {
                let failure = stale_error();
                let message = failure.message.clone();
                let register_error = registration_error_for_update_check_failure(failure.clone());
                record_update_check(
                    &mut state,
                    "stale",
                    Some(failure.status.as_str()),
                    Some(message.clone()),
                    None,
                );
                let callbacks = std::mem::take(&mut queued_job.callbacks);
                let queue_progress = cleanup_precreated_version_and_advance_queue(&mut state);
                (Some((callbacks, Err(register_error))), None, queue_progress)
            } else {
                match result {
                    Err(failure) => {
                        let message = failure.message.clone();
                        let register_error =
                            registration_error_for_update_check_failure(failure.clone());
                        record_update_check(
                            &mut state,
                            "failed",
                            Some(failure.status.as_str()),
                            Some(message.clone()),
                            None,
                        );
                        let callbacks = std::mem::take(&mut queued_job.callbacks);
                        let queue_progress =
                            cleanup_precreated_version_and_advance_queue(&mut state);
                        (Some((callbacks, Err(register_error))), None, queue_progress)
                    }
                    Ok(update_check) => {
                        let ServiceWorkerScriptUpdateCheckResult {
                            main_script,
                            change,
                        } = update_check;
                        match change {
                            ServiceWorkerScriptUpdateCheckChange::Identical => {
                                bump_registration_last_update_check_time_locked(
                                    &mut state,
                                    registration_id,
                                );
                                match self.store_registration_resources_locked(
                                    &state,
                                    registration_id,
                                    pending_check.newest_version_id,
                                ) {
                                    Err(register_error) => {
                                        let message = register_error.message.clone();
                                        record_update_check(
                                            &mut state,
                                            "store-failed",
                                            Some("abort"),
                                            Some(message),
                                            None,
                                        );
                                        let callbacks = std::mem::take(&mut queued_job.callbacks);
                                        let queue_progress =
                                            cleanup_precreated_version_and_advance_queue(
                                                &mut state,
                                            );
                                        (
                                            Some((callbacks, Err(register_error))),
                                            None,
                                            queue_progress,
                                        )
                                    }
                                    Ok(_) => {
                                        record_update_check(
                                            &mut state,
                                            "identical",
                                            None,
                                            None,
                                            None,
                                        );
                                        let mut queue_progress =
                                            cleanup_precreated_version(&mut state);
                                        let registration =
                                            state.registrations.get(&registration_id).expect(
                                                "current update check registration should exist",
                                            );
                                        let snapshot = service_worker_registration_snapshot(
                                            &state,
                                            registration,
                                        );
                                        let callbacks = std::mem::take(&mut queued_job.callbacks);
                                        queue_progress.extend(
                                            self.advance_registration_job_queue_locked(
                                                &mut state,
                                                registration_id,
                                            ),
                                        );
                                        (Some((callbacks, Ok(snapshot))), None, queue_progress)
                                    }
                                }
                            }
                            ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped
                            | ServiceWorkerScriptUpdateCheckChange::MainScriptDifferent
                            | ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                                ..
                            } => {
                                let imported_script_url = match &change {
                                    ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                                        script_url,
                                    } => Some(script_url.to_string()),
                                    ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped
                                    | ServiceWorkerScriptUpdateCheckChange::MainScriptDifferent
                                    | ServiceWorkerScriptUpdateCheckChange::Identical => None,
                                };
                                let result = match change {
                                    ServiceWorkerScriptUpdateCheckChange::MainScriptDifferent => {
                                        "main-script-different"
                                    }
                                    ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped => {
                                        "script-comparison-skipped"
                                    }
                                    ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                                        ..
                                    } => "imported-script-different",
                                    ServiceWorkerScriptUpdateCheckChange::Identical => {
                                        unreachable!("identical update check handled above")
                                    }
                                };
                                bump_registration_last_update_check_time_locked(
                                    &mut state,
                                    registration_id,
                                );
                                match self.store_registration_resources_locked(
                                    &state,
                                    registration_id,
                                    pending_check.newest_version_id,
                                ) {
                                    Err(register_error) => {
                                        let message = register_error.message.clone();
                                        record_update_check(
                                            &mut state,
                                            "store-failed",
                                            Some("abort"),
                                            Some(message),
                                            imported_script_url,
                                        );
                                        let callbacks = std::mem::take(&mut queued_job.callbacks);
                                        let queue_progress =
                                            cleanup_precreated_version_and_advance_queue(
                                                &mut state,
                                            );
                                        (
                                            Some((callbacks, Err(register_error))),
                                            None,
                                            queue_progress,
                                        )
                                    }
                                    Ok(_) => {
                                        record_update_check(
                                            &mut state,
                                            result,
                                            None,
                                            None,
                                            imported_script_url,
                                        );
                                        let callbacks_if_failed = queued_job.callbacks.clone();
                                        let launch = self
                                            .start_precreated_queued_registration_with_preloaded_script(
                                                &mut state,
                                                registration_id,
                                                new_version_id,
                                                queued_job,
                                                main_script,
                                            );
                                        if launch.is_some() {
                                            (None, launch, Vec::new())
                                        } else {
                                            let failure = stale_error();
                                            let message = failure.message.clone();
                                            let register_error =
                                                registration_error_for_update_check_failure(
                                                    failure.clone(),
                                                );
                                            record_update_check(
                                                &mut state,
                                                "stale",
                                                Some(failure.status.as_str()),
                                                Some(message.clone()),
                                                None,
                                            );
                                            let queue_progress =
                                                cleanup_precreated_version_and_advance_queue(
                                                    &mut state,
                                                );
                                            (
                                                Some((callbacks_if_failed, Err(register_error))),
                                                None,
                                                queue_progress,
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };

        if let Some((callbacks, result)) = register_completion {
            ServiceWorkerRegisterJob::send_all(callbacks, result);
        }
        if let Some(launch) = launch {
            self.start_queued_launch(launch);
        }
        for progress in queue_progress {
            self.run_lifecycle_progress(progress);
        }
    }

    pub(super) fn finish_imported_script_loaded(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        resource: WorkerScriptResource,
    ) {
        let resource = ServiceWorkerScriptResource::from_worker_script_resource(resource);
        let mut state = self.inner.state.lock();
        let should_store = {
            let Some(version) = state.versions.get_mut(&version_id) else {
                return;
            };
            if version.registration_id != registration_id || version.run != run {
                return;
            }
            version
                .imported_script_resources
                .insert(resource.final_url.to_string(), resource);
            matches!(
                version.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Installed
                    | ServiceWorkerVersionLifecycleState::Activated
            )
        };
        if should_store {
            let _ = self.store_registration_resources_locked(&state, registration_id, version_id);
        }
    }

    pub(super) fn finish_worker_start_failed(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        failure: ServiceWorkerVersionStartFailure,
    ) {
        let message = failure.to_diagnostic_message();
        let register_error = registration_error_for_start_failure(&failure, message.clone());
        let (completion, queue_progress, failed_pending_events, force_update_page_load_waiters) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&version_id) else {
                return;
            };
            if version.run != run {
                return;
            }
            let should_apply = match &version.running_state {
                ServiceWorkerVersionRunningState::Starting { host }
                | ServiceWorkerVersionRunningState::Running { host } => {
                    host.version_id() == version_id && host.run_identity() == run
                }
                ServiceWorkerVersionRunningState::Stopped => false,
            };
            if !should_apply {
                return;
            }
            let previous = std::mem::replace(
                &mut version.running_state,
                ServiceWorkerVersionRunningState::Stopped,
            );
            let target_run = previous.into_host().map(|host| {
                let run = host.run_identity();
                host.terminate_without_join();
                run
            });
            let failed_pending_events = version.pending_start_events.drain(..).collect::<Vec<_>>();
            version.last_start_error = Some(message.clone());
            let registration_id = version.registration_id;
            let failed_installing_version =
                version.lifecycle_state == ServiceWorkerVersionLifecycleState::Installing;
            if failed_installing_version {
                version.lifecycle_state = ServiceWorkerVersionLifecycleState::Redundant;
            }
            let completion =
                state
                    .registrations
                    .get_mut(&registration_id)
                    .and_then(|registration| {
                        let completion = registration
                            .pending_register_jobs
                            .remove(&version_id)
                            .map(|mut pending_job| {
                                pending_job.abort_before_install(register_error.clone())
                            });
                        if registration.installing_version_id == Some(version_id) {
                            registration.installing_version_id = None;
                        }
                        completion
                    });
            let force_update_page_load_waiters = if failed_installing_version {
                state.take_force_update_page_load_waiters_for_version(version_id)
            } else {
                Vec::new()
            };
            if failed_installing_version {
                state.record_target_destroyed(version_id);
                state.versions.remove(&version_id);
            } else if let Some(run) = target_run {
                state.record_target_stopped(version_id, run, "start_failed");
            }
            let queue_progress =
                self.advance_registration_job_queue_locked(&mut state, registration_id);
            if failed_installing_version {
                prune_empty_registration_after_start_failure_locked(&mut state, registration_id);
            }
            (
                completion,
                queue_progress,
                failed_pending_events,
                force_update_page_load_waiters,
            )
        };
        if let Some(callbacks) = completion {
            ServiceWorkerRegisterJob::send_all(callbacks, Err(register_error));
        }
        for waiter in force_update_page_load_waiters {
            let _ = waiter.send(());
        }
        self.fail_pending_start_events(failed_pending_events, &message);
        for progress in queue_progress {
            self.run_lifecycle_progress(progress);
        }
    }

    pub(super) fn finish_worker_idle_timeout(&self, timeout: ServiceWorkerIdleTimeout) {
        let version_id = timeout.version_id;
        let host = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return;
            };
            if version.run != timeout.run
                || version.idle_timeout_token.as_ref() != Some(&timeout.token)
                || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
                || version.in_flight_event_count != 0
                || !version.pending_start_events.is_empty()
                || !version.pending_activation_fetch_events.is_empty()
            {
                return;
            }
            let registration_id = version.registration_id;
            let ServiceWorkerVersionRunningState::Running { host } = &version.running_state else {
                return;
            };
            if !host.has_running_worker() {
                return;
            }
            if !state
                .registrations
                .get(&registration_id)
                .is_some_and(|registration| registration.active_version_id == Some(version_id))
            {
                return;
            }
            let Some(version) = state.versions.get_mut(&version_id) else {
                return;
            };
            let ServiceWorkerVersionRunningState::Running { host } = &version.running_state else {
                return;
            };
            if !host.has_running_worker() {
                return;
            }
            version.idle_timeout_token = None;
            let host = version.running_state.take_host_for_shutdown();
            if let Some(host) = &host {
                state.record_target_stopped(version_id, host.run_identity(), "idle_timeout");
            }
            host
        };
        if let Some(host) = host {
            host.terminate_without_join();
        }
    }
}

fn registration_error_for_update_check_failure(
    failure: ServiceWorkerScriptUpdateCheckFailure,
) -> ServiceWorkerRegistrationError {
    match failure.status {
        ServiceWorkerScriptUpdateCheckFailureStatus::ScriptLoadFailed => {
            registration_error_for_script_load_failure(failure.message)
        }
        ServiceWorkerScriptUpdateCheckFailureStatus::Internal => {
            ServiceWorkerRegistrationError::unknown(failure.message)
        }
        ServiceWorkerScriptUpdateCheckFailureStatus::Stale => {
            ServiceWorkerRegistrationError::abort(failure.message)
        }
    }
}

fn registration_error_for_script_load_failure(message: String) -> ServiceWorkerRegistrationError {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("returned 404") {
        return ServiceWorkerRegistrationError::new(
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::NotFound,
            message,
        );
    }
    if normalized.contains("cross-origin")
        || normalized.contains("service-worker-allowed")
        || normalized.contains("not under the max scope allowed")
        || normalized.contains("disallowed escape")
    {
        return ServiceWorkerRegistrationError::new(
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Security,
            message,
        );
    }
    if normalized.contains("failed to load service worker script")
        || normalized.contains("network loader not available")
        || normalized.contains("http request")
    {
        return ServiceWorkerRegistrationError::network(message);
    }
    ServiceWorkerRegistrationError::type_error(message)
}

fn registration_error_for_start_failure(
    failure: &ServiceWorkerVersionStartFailure,
    message: String,
) -> ServiceWorkerRegistrationError {
    match failure {
        ServiceWorkerVersionStartFailure::HostThreadSpawn { .. } => {
            ServiceWorkerRegistrationError::type_error(message)
        }
        ServiceWorkerVersionStartFailure::ScriptLoad { .. } => {
            registration_error_for_script_load_failure(message)
        }
        ServiceWorkerVersionStartFailure::Bootstrap { .. } => ServiceWorkerRegistrationError::new(
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::ScriptEvaluateFailed,
            message,
        ),
        ServiceWorkerVersionStartFailure::BootstrapChannelClosed => {
            ServiceWorkerRegistrationError::unknown(message)
        }
    }
}

fn prune_empty_registration_after_start_failure_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) {
    let should_remove = state
        .registrations
        .get(&registration_id)
        .is_some_and(|registration| {
            if registration.installing_version_id.is_some()
                || registration.waiting_version_id.is_some()
                || registration.active_version_id.is_some()
                || registration.pending_unregistration
                || !registration.pending_register_jobs.is_empty()
                || !registration.controlled_client_ids.is_empty()
            {
                return false;
            }
            let registration_key = registration.key();
            state
                .job_coordinator
                .queued_register_job_count(&registration_key)
                == 0
                && state
                    .job_coordinator
                    .queued_unregistration_job_count(&registration_key)
                    == 0
        });
    if should_remove {
        state.registrations.remove(&registration_id);
    }
}
