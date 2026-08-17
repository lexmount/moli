use super::*;

impl ServiceWorkerRuntimeService {
    pub(super) fn finish_lifecycle_event_completed(
        &self,
        completion: ServiceWorkerLifecycleCompletion,
    ) {
        let (
            next_progress,
            activated_pending_fetch_events,
            failed_replaced_pending_events,
            failed_replaced_fetch_completions,
        ) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&completion.version_id) else {
                return;
            };
            if version.run != completion.run {
                return;
            }
            let mut activated_pending_fetch_events = Vec::new();
            let mut failed_replaced_pending_events = Vec::new();
            let mut failed_replaced_fetch_completions = Vec::new();
            version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            let mut progress = match completion.kind {
                ServiceWorkerLifecycleEventKind::Install => {
                    if let Err(message) = completion.result {
                        let register_error =
                            ServiceWorkerRegistrationError::install(message.clone());
                        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Redundant;
                        version.last_start_error = Some(message.clone());
                        let registration_id = version.registration_id;
                        let register_failed = state
                            .registrations
                            .get_mut(&registration_id)
                            .and_then(|registration| {
                                registration.installing_version_id = None;
                                registration
                                    .pending_register_jobs
                                    .remove(&completion.version_id)
                            })
                            .and_then(|mut pending_job| {
                                let callbacks =
                                    pending_job.complete_install_failure(register_error.clone());
                                if callbacks.is_empty() {
                                    None
                                } else {
                                    Some(vec![LifecycleProgress::RegisterFailed((
                                        callbacks,
                                        register_error.clone(),
                                    ))])
                                }
                            })
                            .unwrap_or_default();
                        let lifecycle_notifications =
                            lifecycle_notifications_for_registration_locked(
                                &state,
                                registration_id,
                                vec![ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                                    version_id: completion.version_id,
                                    state: "redundant",
                                }],
                            );
                        let mut progress = lifecycle_notifications
                            .into_iter()
                            .map(|notification| {
                                LifecycleProgress::NotifyLifecycle(Box::new(notification))
                            })
                            .chain(register_failed)
                            .collect::<Vec<_>>();
                        progress.push(LifecycleProgress::ForceUpdatePageLoadCompleted(
                            state.take_force_update_page_load_waiters_for_version(
                                completion.version_id,
                            ),
                        ));
                        progress.extend(self.cleanup_failed_install_version_locked(
                            &mut state,
                            registration_id,
                            completion.version_id,
                        ));
                        progress
                    } else {
                        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Installed;
                        let registration_id = version.registration_id;
                        let skip_waiting_requested = version.skip_waiting_requested;
                        state.record_target_version_updated(completion.version_id);
                        let mut pending_register_job = None;
                        let previous_waiting_version_id = if let Some(registration) =
                            state.registrations.get_mut(&registration_id)
                        {
                            let previous_waiting_version_id = registration.waiting_version_id;
                            registration.installing_version_id = None;
                            registration.waiting_version_id = Some(completion.version_id);
                            pending_register_job = registration
                                .pending_register_jobs
                                .remove(&completion.version_id);
                            previous_waiting_version_id
                        } else {
                            None
                        };
                        match self.store_registration_resources_locked(
                            &state,
                            registration_id,
                            completion.version_id,
                        ) {
                            Err(register_error) => {
                                let message = register_error.message.clone();
                                if let Some(version) =
                                    state.versions.get_mut(&completion.version_id)
                                {
                                    version.lifecycle_state =
                                        ServiceWorkerVersionLifecycleState::Redundant;
                                    version.last_start_error = Some(message.clone());
                                }
                                if let Some(registration) =
                                    state.registrations.get_mut(&registration_id)
                                    && registration.waiting_version_id
                                        == Some(completion.version_id)
                                {
                                    registration.waiting_version_id = previous_waiting_version_id;
                                }
                                let register_failed = pending_register_job
                                    .and_then(|mut pending_job| {
                                        let callbacks = pending_job
                                            .complete_install_failure(register_error.clone());
                                        if callbacks.is_empty() {
                                            None
                                        } else {
                                            Some(vec![LifecycleProgress::RegisterFailed((
                                                callbacks,
                                                register_error.clone(),
                                            ))])
                                        }
                                    })
                                    .unwrap_or_default();
                                let lifecycle_notifications =
                                    lifecycle_notifications_for_registration_locked(
                                        &state,
                                        registration_id,
                                        vec![
                                            ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                                                version_id: completion.version_id,
                                                state: "redundant",
                                            },
                                        ],
                                    );
                                let mut progress = lifecycle_notifications
                                    .into_iter()
                                    .map(|notification| {
                                        LifecycleProgress::NotifyLifecycle(Box::new(notification))
                                    })
                                    .chain(register_failed)
                                    .collect::<Vec<_>>();
                                progress.push(LifecycleProgress::ForceUpdatePageLoadCompleted(
                                    state.take_force_update_page_load_waiters_for_version(
                                        completion.version_id,
                                    ),
                                ));
                                progress.extend(self.cleanup_failed_install_version_locked(
                                    &mut state,
                                    registration_id,
                                    completion.version_id,
                                ));
                                progress
                            }
                            Ok(_) => {
                                let skip_waiting_after_install =
                                    pending_register_job.as_ref().is_some_and(|pending_job| {
                                        pending_job.skip_waiting_after_install()
                                    });
                                let should_skip_waiting =
                                    skip_waiting_requested || skip_waiting_after_install;
                                let register_completed =
                                    pending_register_job.and_then(|mut pending_job| {
                                        let registration =
                                            state.registrations.get(&registration_id)?;
                                        let snapshot = service_worker_registration_snapshot(
                                            &state,
                                            registration,
                                        );
                                        let callbacks =
                                            pending_job.complete_install_success(snapshot.clone());
                                        if callbacks.is_empty() {
                                            None
                                        } else {
                                            Some(LifecycleProgress::RegisterCompleted(Box::new((
                                                callbacks, snapshot,
                                            ))))
                                        }
                                    });
                                let installed_notifications =
                                    lifecycle_notifications_for_registration_locked(
                                        &state,
                                        registration_id,
                                        vec![
                                            ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                                                version_id: completion.version_id,
                                                state: "installed",
                                            },
                                        ],
                                    );
                                if should_skip_waiting
                                    && let Some(version) =
                                        state.versions.get_mut(&completion.version_id)
                                {
                                    version.skip_waiting_requested = true;
                                }
                                let mut progress = installed_notifications
                                    .into_iter()
                                    .map(|notification| {
                                        LifecycleProgress::NotifyLifecycle(Box::new(notification))
                                    })
                                    .collect::<Vec<_>>();
                                if let Some(register_completed) = register_completed {
                                    progress.push(register_completed);
                                }
                                progress.extend(
                                    self.activation_progress_for_registration_if_ready_locked(
                                        &mut state,
                                        registration_id,
                                    ),
                                );
                                progress.extend(self.advance_registration_job_queue_locked(
                                    &mut state,
                                    registration_id,
                                ));
                                progress
                            }
                        }
                    }
                }
                ServiceWorkerLifecycleEventKind::Activate => {
                    if let Err(message) = completion.result {
                        // Chromium still commits a non-shutdown activate event failure; a rejected
                        // activate waitUntil is observable as an error, not as a failed install.
                        version.last_start_error = Some(message);
                    }
                    version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activated;
                    activated_pending_fetch_events =
                        version.pending_activation_fetch_events.drain(..).collect();
                    let should_claim_clients = version.clients_claim_requested;
                    let registration_id = version.registration_id;
                    state.record_target_version_updated(completion.version_id);
                    let Some(registration) = state.registrations.get_mut(&registration_id) else {
                        return;
                    };
                    let previous_active_version_id = registration.active_version_id;
                    if registration.installing_version_id == Some(completion.version_id) {
                        registration.installing_version_id = None;
                    }
                    if registration.waiting_version_id == Some(completion.version_id) {
                        registration.waiting_version_id = None;
                    }
                    registration.active_version_id = Some(completion.version_id);
                    let _ = registration;
                    let _ = self.store_registration_resources_locked(
                        &state,
                        registration_id,
                        completion.version_id,
                    );
                    let mut progress = Vec::new();
                    let mut previous_target_cleanup = None;
                    if let Some(previous_active_version_id) = previous_active_version_id
                        && previous_active_version_id != completion.version_id
                        && let Some(previous_version) =
                            state.versions.get_mut(&previous_active_version_id)
                    {
                        previous_version.lifecycle_state =
                            ServiceWorkerVersionLifecycleState::Redundant;
                        previous_version.skip_waiting_requested = false;
                        let previous_run = previous_version.run.clone();
                        failed_replaced_pending_events =
                            previous_version.pending_start_events.drain(..).collect();
                        let failed_replaced_activation_fetch_events = previous_version
                            .pending_activation_fetch_events
                            .drain(..)
                            .collect::<Vec<_>>();
                        let previous_host = previous_version.running_state.take_host_for_shutdown();
                        let previous_host_run =
                            previous_host.as_ref().map(|host| host.run_identity());
                        if let Some(host) = previous_host {
                            progress.push(LifecycleProgress::TerminateHost(host));
                        }
                        previous_target_cleanup =
                            Some((previous_active_version_id, previous_host_run));
                        let pending_start_fetch_event_ids = failed_replaced_pending_events
                            .iter()
                            .filter_map(|event| match event {
                                ServiceWorkerPendingStartEvent::Fetch(event) => {
                                    Some(event.event_id)
                                }
                                _ => None,
                            })
                            .collect::<HashSet<_>>();
                        let pending_activation_fetch_event_ids =
                            failed_replaced_activation_fetch_events
                                .iter()
                                .map(|event| event.event_id)
                                .collect::<HashSet<_>>();
                        failed_replaced_fetch_completions = state
                            .pending_fetch_jobs
                            .iter()
                            .filter(|(event_id, job)| {
                                job.version_id == previous_active_version_id
                                    && job.is_bound_to_run(&previous_run)
                                    && !pending_start_fetch_event_ids.contains(event_id)
                                    && !pending_activation_fetch_event_ids.contains(event_id)
                            })
                            .map(|(event_id, job)| ServiceWorkerFetchCompletion {
                                event_id: *event_id,
                                version_id: job.version_id,
                                run: job.run_identity().clone(),
                                result: ServiceWorkerFetchResult::Failure(
                                    "service worker was replaced by a newer active worker"
                                        .to_owned(),
                                ),
                            })
                            .collect();
                        failed_replaced_fetch_completions.extend(
                            failed_replaced_activation_fetch_events
                                .into_iter()
                                .map(|event| ServiceWorkerFetchCompletion {
                                    event_id: event.event_id,
                                    version_id: event.version_id,
                                    run: event.run.clone(),
                                    result: ServiceWorkerFetchResult::Failure(
                                        "service worker was replaced by a newer active worker"
                                            .to_owned(),
                                    ),
                                }),
                        );
                    }
                    if let Some((previous_active_version_id, previous_run)) =
                        previous_target_cleanup
                    {
                        if let Some(run) = previous_run {
                            state.record_target_stopped(
                                previous_active_version_id,
                                run,
                                "replaced_by_newer_active_worker",
                            );
                        }
                        state.record_target_destroyed(previous_active_version_id);
                    }
                    let mut lifecycle_events =
                        vec![ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                            version_id: completion.version_id,
                            state: "activated",
                        }];
                    if let Some(previous_active_version_id) = previous_active_version_id
                        && previous_active_version_id != completion.version_id
                    {
                        lifecycle_events.push(
                            ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                                version_id: previous_active_version_id,
                                state: "redundant",
                            },
                        );
                    }
                    if previous_active_version_id
                        .is_some_and(|version_id| version_id != completion.version_id)
                    {
                        progress.extend(
                            controller_change_deliveries_for_controlled_clients_locked(
                                &state,
                                registration_id,
                            )
                            .into_iter()
                            .map(LifecycleProgress::NotifyControllerChange),
                        );
                    }
                    progress.extend(
                        lifecycle_notifications_for_registration_locked(
                            &state,
                            registration_id,
                            lifecycle_events,
                        )
                        .into_iter()
                        .map(|notification| {
                            LifecycleProgress::NotifyLifecycle(Box::new(notification))
                        }),
                    );
                    progress.push(LifecycleProgress::ForceUpdatePageLoadCompleted(
                        state
                            .take_force_update_page_load_waiters_for_version(completion.version_id),
                    ));
                    let mut pending_ready_jobs = std::mem::take(&mut state.pending_ready_jobs);
                    let Some(registration) = state.registrations.get(&registration_id) else {
                        state.pending_ready_jobs = pending_ready_jobs;
                        return;
                    };
                    let snapshot = service_worker_registration_snapshot(&state, registration);
                    if should_claim_clients {
                        let claim_result =
                            claim_live_scope_clients_locked(&mut state, registration_id);
                        progress.extend(
                            claim_result
                                .controller_change_deliveries
                                .into_iter()
                                .map(LifecycleProgress::NotifyControllerChange),
                        );
                        for changed_registration_id in claim_result.changed_registration_ids {
                            if state
                                .registrations
                                .get(&changed_registration_id)
                                .is_some_and(|registration| registration.pending_unregistration)
                            {
                                progress.extend(
                                    self.unregistration_progress_for_registration_if_ready_locked(
                                        &mut state,
                                        changed_registration_id,
                                    ),
                                );
                            } else {
                                progress.extend(
                                    self.activation_progress_for_registration_if_ready_locked(
                                        &mut state,
                                        changed_registration_id,
                                    ),
                                );
                            }
                        }
                    }
                    for job in pending_ready_jobs.drain(..) {
                        if job.registration_id == registration_id {
                            progress.push(LifecycleProgress::ReadyCompleted(Box::new((
                                job,
                                snapshot.clone(),
                            ))));
                        } else {
                            state.pending_ready_jobs.push(job);
                        }
                    }
                    progress
                }
            };
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                completion.version_id,
            );
            if unregistration_progress.is_empty() {
                if let Some(idle_timeout) =
                    self.maybe_schedule_idle_timeout_locked(&mut state, completion.version_id)
                {
                    progress.push(LifecycleProgress::ScheduleIdleTimeout(idle_timeout));
                }
            } else {
                progress.extend(unregistration_progress);
            }
            (
                progress,
                activated_pending_fetch_events,
                failed_replaced_pending_events,
                failed_replaced_fetch_completions,
            )
        };
        self.fail_pending_start_events(
            failed_replaced_pending_events,
            "service worker was replaced by a newer active worker",
        );
        for completion in failed_replaced_fetch_completions {
            self.finish_fetch_event_completed(completion);
        }
        self.dispatch_pending_activation_fetch_events(activated_pending_fetch_events);
        for progress in next_progress {
            self.run_lifecycle_progress(progress);
        }
    }

    pub(super) fn run_lifecycle_progress(&self, progress: LifecycleProgress) {
        match progress {
            LifecycleProgress::Dispatch((host, event)) => {
                self.dispatch_lifecycle_event(host, event);
            }
            LifecycleProgress::TerminateHost(host) => {
                host.terminate_without_join();
            }
            LifecycleProgress::ScheduleIdleTimeout(idle_timeout) => {
                self.schedule_idle_timeout(idle_timeout);
            }
            LifecycleProgress::ReadyCompleted(progress) => {
                let (job, snapshot) = *progress;
                job.send(snapshot);
            }
            LifecycleProgress::RegisterCompleted(progress) => {
                let (jobs, snapshot) = *progress;
                ServiceWorkerRegisterJob::send_all(jobs, Ok(snapshot));
            }
            LifecycleProgress::RegisterFailed((jobs, message)) => {
                ServiceWorkerRegisterJob::send_all(jobs, Err(message));
            }
            LifecycleProgress::ForceUpdatePageLoadCompleted(waiters) => {
                for waiter in waiters {
                    let _ = waiter.send(());
                }
            }
            LifecycleProgress::UnregisterCompleted(job) => {
                job.send_all(true);
            }
            LifecycleProgress::FetchFailed(progress) => {
                let (job, message) = *progress;
                self.complete_fetch_with_failure(job, message);
            }
            LifecycleProgress::NotifyLifecycle(notification) => {
                notification.send();
            }
            LifecycleProgress::NotifyControllerChange(delivery) => {
                delivery.send();
            }
            LifecycleProgress::StartWorker(launch) => {
                self.start_queued_launch(*launch);
            }
            LifecycleProgress::StartMainScriptUpdateCheck(update_check) => {
                let (registration_id, load_params) = *update_check;
                self.start_main_script_update_check(registration_id, load_params);
            }
        }
    }
}
