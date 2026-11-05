use super::*;

impl ServiceWorkerRuntimeService {
    pub(super) fn finish_lifecycle_event_completed(
        &self,
        completion: ServiceWorkerLifecycleCompletion,
    ) {
        let version_id = completion.owner.version_id();
        let run = completion.owner.cloned_run_identity();
        let (
            next_progress,
            activated_pending_fetch_events,
            activated_pending_events,
            activated_registration_id,
        ) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&version_id) else {
                return;
            };
            if version.run != run
                || (completion.kind == ServiceWorkerLifecycleEventKind::Activate
                    && version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activating)
            {
                return;
            }
            let mut activated_pending_fetch_events = Vec::new();
            let mut activated_pending_events = Vec::new();
            let mut activated_registration_id = None;
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
                                registration.pending_register_jobs.remove(&version_id)
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
                                    version_id,
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
                            state.take_force_update_page_load_waiters_for_version(version_id),
                        ));
                        progress.extend(self.cleanup_failed_install_version_locked(
                            &mut state,
                            registration_id,
                            version_id,
                        ));
                        progress
                    } else {
                        version.lifecycle_state = ServiceWorkerVersionLifecycleState::Installed;
                        let registration_id = version.registration_id;
                        let skip_waiting_requested = version.skip_waiting_requested;
                        state.record_target_version_updated(version_id);
                        let mut pending_register_job = None;
                        let previous_waiting_version_id = if let Some(registration) =
                            state.registrations.get_mut(&registration_id)
                        {
                            let previous_waiting_version_id = registration.waiting_version_id;
                            registration.installing_version_id = None;
                            registration.waiting_version_id = Some(version_id);
                            pending_register_job =
                                registration.pending_register_jobs.remove(&version_id);
                            previous_waiting_version_id
                        } else {
                            None
                        };
                        match self.store_registration_resources_locked(
                            &state,
                            registration_id,
                            version_id,
                        ) {
                            Err(register_error) => {
                                let message = register_error.message.clone();
                                if let Some(version) = state.versions.get_mut(&version_id) {
                                    version.lifecycle_state =
                                        ServiceWorkerVersionLifecycleState::Redundant;
                                    version.last_start_error = Some(message.clone());
                                }
                                if let Some(registration) =
                                    state.registrations.get_mut(&registration_id)
                                    && registration.waiting_version_id == Some(version_id)
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
                                                version_id,
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
                                        version_id,
                                    ),
                                ));
                                progress.extend(self.cleanup_failed_install_version_locked(
                                    &mut state,
                                    registration_id,
                                    version_id,
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
                                                version_id,
                                                state: "installed",
                                            },
                                        ],
                                    );
                                if should_skip_waiting
                                    && let Some(version) = state.versions.get_mut(&version_id)
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
                    activated_pending_events = version.pending_start_events.drain(..).collect();
                    let registration_id = version.registration_id;
                    activated_registration_id = Some(registration_id);
                    state.record_target_version_updated(version_id);
                    let _ = self.store_registration_resources_locked(
                        &state,
                        registration_id,
                        version_id,
                    );
                    let mut progress = lifecycle_notifications_for_registration_locked(
                        &state,
                        registration_id,
                        vec![ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                            version_id,
                            state: "activated",
                        }],
                    )
                    .into_iter()
                    .map(|notification| LifecycleProgress::NotifyLifecycle(Box::new(notification)))
                    .collect::<Vec<_>>();
                    progress.push(LifecycleProgress::ForceUpdatePageLoadCompleted(
                        state.take_force_update_page_load_waiters_for_version(version_id),
                    ));
                    progress
                }
            };
            let unregistration_progress =
                self.unregistration_progress_for_version_if_ready_locked(&mut state, version_id);
            if unregistration_progress.is_empty() {
                if let Some(idle_timeout) =
                    self.maybe_schedule_idle_timeout_locked(&mut state, version_id)
                {
                    progress.push(LifecycleProgress::ScheduleIdleTimeout(idle_timeout));
                }
            } else {
                progress.extend(unregistration_progress);
            }
            (
                progress,
                activated_pending_fetch_events,
                activated_pending_events,
                activated_registration_id,
            )
        };
        self.dispatch_pending_activation_fetch_events(activated_pending_fetch_events);
        self.dispatch_started_worker_events(activated_pending_events);
        for progress in next_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(registration_id) = activated_registration_id {
            let progress = self.activation_progress_for_registration_if_ready_locked(
                &mut self.inner.state.lock(),
                registration_id,
            );
            for progress in progress {
                self.run_lifecycle_progress(progress);
            }
        }
    }

    pub(super) fn run_lifecycle_progress(&self, progress: LifecycleProgress) {
        match progress {
            LifecycleProgress::Dispatch((host, event)) => {
                self.dispatch_lifecycle_event(host, event);
            }
            LifecycleProgress::FailReplacedWorkerEvents(events) => {
                let (pending, completions) = *events;
                self.fail_pending_start_events(
                    pending,
                    "service worker was replaced by a newer active worker",
                );
                for completion in completions {
                    self.finish_fetch_event_completed(completion);
                }
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
