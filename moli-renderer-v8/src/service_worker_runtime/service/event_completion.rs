use super::*;

impl ServiceWorkerRuntimeService {
    pub(crate) fn abort_controlled_fetch(&self, internal_id: u64) -> bool {
        self.abort_controlled_fetch_with_reason(internal_id, None)
    }

    pub(crate) fn abort_controlled_fetch_with_reason(
        &self,
        internal_id: u64,
        reason: Option<crate::structured_clone::V8StructuredClonePayload>,
    ) -> bool {
        let aborted = {
            let mut state = self.inner.state.lock();
            let Some(event_id) = state
                .pending_fetch_jobs
                .iter()
                .find_map(|(event_id, job)| (job.internal_id == internal_id).then_some(*event_id))
            else {
                return false;
            };
            let Some(mut job) = state.pending_fetch_jobs.remove(&event_id) else {
                return false;
            };
            let mut should_update_lifecycle = false;
            if let Some(version) = state.versions.get_mut(&job.version_id)
                && job.is_bound_to_run(&version.run)
            {
                let activation_len_before = version.pending_activation_fetch_events.len();
                version
                    .pending_activation_fetch_events
                    .retain(|event| event.event_id != event_id);
                let removed_pending_activation =
                    version.pending_activation_fetch_events.len() != activation_len_before;

                version.pending_start_events.retain(|event| {
                    !matches!(
                        event,
                        ServiceWorkerPendingStartEvent::Fetch(event)
                            if event.event_id == event_id
                    )
                });

                if !removed_pending_activation {
                    version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
                }
                should_update_lifecycle = true;
            }
            let stream_cancel = job.streaming_body_source_id.and_then(|body_source_id| {
                let version = state.versions.get(&job.version_id)?;
                if !job.is_bound_to_run(&version.run) {
                    return None;
                }
                let ServiceWorkerVersionRunningState::Running { host } = &version.running_state
                else {
                    return None;
                };
                (host.version_id() == job.version_id && job.is_bound_to_run(&host.run_identity()))
                    .then_some((host.clone(), event_id, body_source_id))
            });
            let request_signal_abort = state.versions.get(&job.version_id).and_then(|version| {
                if !job.is_bound_to_run(&version.run) {
                    return None;
                }
                let ServiceWorkerVersionRunningState::Running { host } = &version.running_state
                else {
                    return None;
                };
                (host.version_id() == job.version_id && job.is_bound_to_run(&host.run_identity()))
                    .then_some((host.clone(), event_id))
            });
            job.cancel_handle.cancel();
            job.cancel_pending_navigation_preload();
            if should_update_lifecycle {
                let unregistration_progress = self
                    .unregistration_progress_for_version_if_ready_locked(
                        &mut state,
                        job.version_id,
                    );
                let activation_progress = if unregistration_progress.is_empty() {
                    self.activation_progress_for_active_version_if_ready_locked(
                        &mut state,
                        job.version_id,
                    )
                } else {
                    Vec::new()
                };
                let idle_timeout =
                    self.maybe_schedule_idle_timeout_locked(&mut state, job.version_id);
                let mut progress = unregistration_progress;
                progress.extend(activation_progress);
                Some((
                    job.version_id,
                    job.run_identity().clone(),
                    job,
                    idle_timeout,
                    progress,
                    stream_cancel,
                    request_signal_abort,
                ))
            } else {
                Some((
                    job.version_id,
                    job.run_identity().clone(),
                    job,
                    None,
                    Vec::new(),
                    stream_cancel,
                    request_signal_abort,
                ))
            }
        };
        let Some((
            version_id,
            run,
            job,
            idle_timeout,
            lifecycle_progress,
            stream_cancel,
            request_signal_abort,
        )) = aborted
        else {
            return false;
        };
        if let Some((host, event_id)) = request_signal_abort {
            host.abort_fetch_event_request_signal(event_id, reason);
        }
        if let Some((host, event_id, body_source_id)) = stream_cancel {
            host.cancel_fetch_stream(event_id, body_source_id);
        }
        let abort_result =
            ServiceWorkerFetchResult::Failure(crate::network_host::ABORTED_ERROR_TEXT.to_owned());
        let diagnostic = service_worker_fetch_diagnostic_from_job_result(&job, &abort_result);
        self.enqueue_target_fetch_diagnostic(version_id, run, diagnostic);
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
        true
    }

    pub(super) fn finish_fetch_event_completed(&self, completion: ServiceWorkerFetchCompletion) {
        let version_id = completion.version_id;
        let run = completion.run;
        let (job, result, idle_timeout, lifecycle_progress) = {
            let mut state = self.inner.state.lock();
            {
                let Some(version) = state.versions.get(&version_id) else {
                    return;
                };
                if version.run != run {
                    return;
                }
            }
            let Some(mut job) = state.pending_fetch_jobs.remove(&completion.event_id) else {
                return;
            };
            job.cancel_pending_navigation_preload();
            if let Some(version) = state.versions.get_mut(&version_id) {
                version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            }
            let unregistration_progress =
                self.unregistration_progress_for_version_if_ready_locked(&mut state, version_id);
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(&mut state, version_id)
            } else {
                Vec::new()
            };
            let idle_timeout = self.maybe_schedule_idle_timeout_locked(&mut state, version_id);
            let mut progress = unregistration_progress;
            progress.extend(activation_progress);
            (job, completion.result, idle_timeout, progress)
        };
        let diagnostic = service_worker_fetch_diagnostic_from_job_result(&job, &result);
        self.enqueue_target_fetch_diagnostic(version_id, run, diagnostic);
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
        match result {
            ServiceWorkerFetchResult::Fallback => {
                self.dispatch_fetch_fallback(job);
            }
            ServiceWorkerFetchResult::Response(response) => {
                self.complete_fetch_with_service_worker_response(job, response);
            }
            ServiceWorkerFetchResult::Failure(message) => {
                self.complete_fetch_with_failure(job, message);
            }
        }
    }

    pub(super) fn finish_message_event_completed(
        &self,
        completion: ServiceWorkerMessageCompletion,
    ) {
        let (idle_timeout, lifecycle_progress) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&completion.version_id) else {
                return;
            };
            if version.run != completion.run {
                return;
            }
            version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            if let Err(message) = completion.result {
                version.last_start_error = Some(message);
            }
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                completion.version_id,
            );
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(
                    &mut state,
                    completion.version_id,
                )
            } else {
                Vec::new()
            };
            let idle_timeout =
                self.maybe_schedule_idle_timeout_locked(&mut state, completion.version_id);
            let mut progress = unregistration_progress;
            progress.extend(activation_progress);
            (idle_timeout, progress)
        };
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
    }

    pub(super) fn finish_notification_event_completed(
        &self,
        completion: ServiceWorkerNotificationCompletion,
    ) {
        let (idle_timeout, lifecycle_progress) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&completion.version_id) else {
                return;
            };
            if version.run != completion.run {
                return;
            }
            version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            if let Err(message) = completion.result {
                version.last_start_error = Some(message);
            }
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                completion.version_id,
            );
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(
                    &mut state,
                    completion.version_id,
                )
            } else {
                Vec::new()
            };
            let idle_timeout =
                self.maybe_schedule_idle_timeout_locked(&mut state, completion.version_id);
            let mut progress = unregistration_progress;
            progress.extend(activation_progress);
            (idle_timeout, progress)
        };
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
    }

    pub(super) fn finish_push_event_completed(&self, completion: ServiceWorkerPushCompletion) {
        let (idle_timeout, lifecycle_progress) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&completion.version_id) else {
                return;
            };
            if version.run != completion.run {
                return;
            }
            version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            if let Err(message) = completion.result {
                version.last_start_error = Some(message);
            }
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                completion.version_id,
            );
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(
                    &mut state,
                    completion.version_id,
                )
            } else {
                Vec::new()
            };
            let idle_timeout =
                self.maybe_schedule_idle_timeout_locked(&mut state, completion.version_id);
            let mut progress = unregistration_progress;
            progress.extend(activation_progress);
            (idle_timeout, progress)
        };
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
    }

    pub(super) fn finish_sync_event_completed(&self, completion: ServiceWorkerSyncCompletion) {
        enum SyncFollowUp {
            Refire { scope_url: Url, tag: String },
            Retry { scope_url: Url, tag: String },
        }

        let (idle_timeout, lifecycle_progress, follow_up) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&completion.version_id) else {
                return;
            };
            if version.run != completion.run {
                return;
            }
            version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            let sync_key = (completion.registration_id, completion.tag);
            let mut follow_up = None;
            match completion.result {
                Ok(()) => {
                    match state
                        .sync_registrations
                        .get_mut(&sync_key)
                        .and_then(|record| record.finish_active_dispatch(completion.event_id))
                    {
                        Some(true) => {
                            if let Some(record) = state.sync_registrations.get_mut(&sync_key) {
                                record.failed_attempts = 0;
                            }
                            follow_up = state.registrations.get(&sync_key.0).map(|registration| {
                                SyncFollowUp::Refire {
                                    scope_url: registration.scope_url.clone(),
                                    tag: sync_key.1.clone(),
                                }
                            });
                        }
                        Some(false) => {
                            state.sync_registrations.remove(&sync_key);
                        }
                        None => {}
                    }
                }
                Err(message) => {
                    version.last_start_error = Some(message);
                    let dispatch_refire = state
                        .sync_registrations
                        .get_mut(&sync_key)
                        .and_then(|record| record.finish_active_dispatch(completion.event_id));
                    match dispatch_refire {
                        Some(true) => {
                            if let Some(record) = state.sync_registrations.get_mut(&sync_key) {
                                record.failed_attempts = 0;
                            }
                            follow_up = state.registrations.get(&sync_key.0).map(|registration| {
                                SyncFollowUp::Refire {
                                    scope_url: registration.scope_url.clone(),
                                    tag: sync_key.1.clone(),
                                }
                            });
                        }
                        Some(false) => match state.sync_registrations.get_mut(&sync_key) {
                            Some(record) if record.failed_attempts == 0 => {
                                record.failed_attempts = 1;
                                follow_up =
                                    state.registrations.get(&sync_key.0).map(|registration| {
                                        SyncFollowUp::Retry {
                                            scope_url: registration.scope_url.clone(),
                                            tag: sync_key.1.clone(),
                                        }
                                    });
                            }
                            Some(_) => {
                                state.sync_registrations.remove(&sync_key);
                            }
                            None => {}
                        },
                        None => {}
                    }
                }
            }
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                completion.version_id,
            );
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(
                    &mut state,
                    completion.version_id,
                )
            } else {
                Vec::new()
            };
            let idle_timeout =
                self.maybe_schedule_idle_timeout_locked(&mut state, completion.version_id);
            let mut progress = unregistration_progress;
            progress.extend(activation_progress);
            (idle_timeout, progress, follow_up)
        };
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
        if let Some(follow_up) = follow_up {
            match follow_up {
                SyncFollowUp::Refire { scope_url, tag } => {
                    let _ = self.register_sync_for_scope(&scope_url, tag);
                }
                SyncFollowUp::Retry { scope_url, tag } => {
                    let _ = self.retry_sync_for_scope(&scope_url, &tag);
                }
            }
        }
    }

    pub(super) fn finish_periodic_sync_event_completed(
        &self,
        completion: ServiceWorkerPeriodicSyncCompletion,
    ) {
        let (idle_timeout, lifecycle_progress, refire) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&completion.version_id) else {
                return;
            };
            if version.run != completion.run
                || version.registration_id != completion.registration_id
            {
                return;
            }
            version.in_flight_event_count = version.in_flight_event_count.saturating_sub(1);
            if let Err(message) = &completion.result {
                version.last_start_error = Some(format!(
                    "periodicsync `{}` failed: {message}",
                    completion.tag
                ));
            }
            let periodic_sync_key = (completion.registration_id, completion.tag.clone());
            let refire = state
                .periodic_sync_registrations
                .get_mut(&periodic_sync_key)
                .and_then(|record| record.finish_active_dispatch(completion.event_id))
                .filter(|refire_after_finish| *refire_after_finish)
                .and_then(|_| {
                    state
                        .registrations
                        .get(&periodic_sync_key.0)
                        .map(|registration| (registration.scope_url.clone(), periodic_sync_key.1))
                });
            let unregistration_progress = self.unregistration_progress_for_version_if_ready_locked(
                &mut state,
                completion.version_id,
            );
            let activation_progress = if unregistration_progress.is_empty() {
                self.activation_progress_for_active_version_if_ready_locked(
                    &mut state,
                    completion.version_id,
                )
            } else {
                Vec::new()
            };
            let idle_timeout =
                self.maybe_schedule_idle_timeout_locked(&mut state, completion.version_id);
            let mut progress = unregistration_progress;
            progress.extend(activation_progress);
            (idle_timeout, progress, refire)
        };
        for progress in lifecycle_progress {
            self.run_lifecycle_progress(progress);
        }
        if let Some(idle_timeout) = idle_timeout {
            self.schedule_idle_timeout(idle_timeout);
        }
        if let Some((scope_url, tag)) = refire {
            let _ = self.dispatch_periodic_sync_for_scope(&scope_url, &tag);
        }
    }
}

pub(super) fn service_worker_fetch_diagnostic_from_job_result(
    job: &ServiceWorkerFetchJob,
    result: &ServiceWorkerFetchResult,
) -> crate::runtime::RendererServiceWorkerFetchDiagnostic {
    let result = match result {
        ServiceWorkerFetchResult::Fallback => {
            crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Fallback
        }
        ServiceWorkerFetchResult::Response(response) => {
            crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Response {
                final_url: response
                    .final_url
                    .as_ref()
                    .unwrap_or(&job.request_url)
                    .as_str()
                    .to_owned(),
                status: response.status,
                status_text: response.status_text.clone(),
                response_headers: response.headers.clone(),
                body_len: response.body.len(),
            }
        }
        ServiceWorkerFetchResult::Failure(message) => {
            crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Failure {
                message: message.clone(),
            }
        }
    };
    crate::runtime::RendererServiceWorkerFetchDiagnostic {
        internal_id: job.internal_id,
        document_url: job.network_context.document_url.as_str().to_owned(),
        request_url: job.request_url.as_str().to_owned(),
        method: job.request_method.clone(),
        request_headers: job.request_headers.clone(),
        request_body: job.request_body.clone(),
        destination: service_worker_fetch_diagnostic_destination(job.network_context.resource_type)
            .to_owned(),
        result,
    }
}

fn service_worker_fetch_diagnostic_destination(
    resource_type: crate::types::SubresourceResourceType,
) -> &'static str {
    match resource_type {
        crate::types::SubresourceResourceType::Script => "script",
        crate::types::SubresourceResourceType::Stylesheet => "style",
        crate::types::SubresourceResourceType::Image => "image",
        crate::types::SubresourceResourceType::Font => "font",
        crate::types::SubresourceResourceType::Audio => "audio",
        crate::types::SubresourceResourceType::Video => "video",
        crate::types::SubresourceResourceType::Media => "video",
        crate::types::SubresourceResourceType::TextTrack => "track",
        crate::types::SubresourceResourceType::Dictionary => "dictionary",
        _ => "",
    }
}
