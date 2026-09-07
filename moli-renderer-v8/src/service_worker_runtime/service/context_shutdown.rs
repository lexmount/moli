use super::registration_jobs::remove_version_and_shutdown_host_locked;
use super::*;

impl ServiceWorkerRuntimeService {
    pub(crate) fn terminate_all_for_context_shutdown(&self) {
        // No callback may retain or reopen work after its physical Context
        // has retired, even when an inspection endpoint is still held.
        self.service_lane().close();
        let (progress, aborted_jobs) = self.take_context_shutdown_work();
        for aborted_job in aborted_jobs {
            Self::send_aborted_job(aborted_job);
        }
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
    }

    #[cfg(test)]
    pub(crate) fn stop_all_running_hosts_for_test(&self) {
        self.devtools_stop_all_workers()
            .expect("test host stop should use the production ServiceWorker retirement path");
    }

    fn take_context_shutdown_work(&self) -> (Vec<LifecycleProgress>, Vec<ServiceWorkerAbortedJob>) {
        let mut state = self.inner.state.lock();
        let mut progress = vec![LifecycleProgress::ForceUpdatePageLoadCompleted(
            state.take_all_force_update_page_load_waiters(),
        )];
        let mut aborted_jobs = state
            .pending_main_script_update_checks
            .drain()
            .map(|(_, pending_check)| pending_check.abort())
            .collect::<Vec<_>>();
        aborted_jobs.extend(abort_pending_register_jobs_for_context_shutdown_locked(
            &mut state,
        ));
        aborted_jobs.extend(state.job_coordinator.abort_all());
        progress.extend(state.pending_fetch_jobs.drain().map(|(_, job)| {
            job.cancel_handle.cancel();
            LifecycleProgress::FetchFailed(Box::new((
                job,
                SERVICE_WORKER_JOB_ABORTED_ERROR.to_owned(),
            )))
        }));
        let version_ids = state.versions.keys().copied().collect::<Vec<_>>();
        for version_id in version_ids {
            progress.extend(remove_version_and_shutdown_host_locked(
                &mut state, version_id,
            ));
        }
        // Drop runtime-owned callbacks and launch resources, not the persisted
        // registrations in StoragePartition's resource store.
        state.registrations.clear();
        state.pending_ready_jobs.clear();
        state.lifecycle_watchers.clear();
        state.live_clients.clear();
        state.notification_records.clear();
        state.sync_registrations.clear();
        state.periodic_sync_registrations.clear();
        state.push_subscriptions.clear();
        state.pending_devtools_launches.clear();
        state.pending_devtools_evaluation_releases.clear();
        state.devtools_related_pause_on_start_policies.clear();
        state.main_script_update_check_diagnostics.clear();
        state.stored_registration_cache.clear();
        state.stored_registration_cache_revision = None;
        (progress, aborted_jobs)
    }

    fn send_aborted_job(aborted_job: ServiceWorkerAbortedJob) {
        match aborted_job {
            ServiceWorkerAbortedJob::Register(callbacks) => {
                ServiceWorkerRegisterJob::send_all(
                    callbacks,
                    Err(ServiceWorkerRegistrationError::abort(
                        SERVICE_WORKER_JOB_ABORTED_ERROR,
                    )),
                );
            }
            ServiceWorkerAbortedJob::Unregister(callbacks) => {
                for callback in callbacks {
                    callback.send(false);
                }
            }
        }
    }
}

fn abort_pending_register_jobs_for_context_shutdown_locked(
    state: &mut ServiceWorkerRuntimeState,
) -> Vec<ServiceWorkerAbortedJob> {
    state
        .registrations
        .values_mut()
        .flat_map(|registration| {
            registration.installing_version_id = None;
            registration
                .pending_register_jobs
                .drain()
                .filter_map(|(_, mut pending_job)| {
                    let callbacks = pending_job.abort_before_install(
                        ServiceWorkerRegistrationError::abort(SERVICE_WORKER_JOB_ABORTED_ERROR),
                    );
                    if callbacks.is_empty() {
                        None
                    } else {
                        Some(ServiceWorkerAbortedJob::Register(callbacks))
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
