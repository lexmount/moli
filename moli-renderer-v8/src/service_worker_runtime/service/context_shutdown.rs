use super::registration_jobs::remove_version_and_shutdown_host_locked;
use super::*;

impl ServiceWorkerRuntimeService {
    pub(crate) fn terminate_all_for_context_shutdown(&self) {
        let (hosts, aborted_jobs, force_update_page_load_waiters) =
            self.take_context_shutdown_work();
        for aborted_job in aborted_jobs {
            Self::send_aborted_job(aborted_job);
        }
        for waiter in force_update_page_load_waiters {
            let _ = waiter.send(());
        }
        for host in hosts {
            host.terminate_without_join();
        }
    }

    #[cfg(test)]
    pub(crate) fn stop_all_running_hosts_for_test(&self) {
        self.stop_running_workers("test_stop_all");
    }

    #[cfg(any(test, feature = "devtools"))]
    pub(super) fn stop_running_workers(&self, reason: &'static str) -> usize {
        let hosts = {
            let mut state = self.inner.state.lock();
            let version_ids = state.versions.keys().copied().collect::<Vec<_>>();
            version_ids
                .into_iter()
                .filter_map(|version_id| stop_worker_version_locked(&mut state, version_id, reason))
                .collect::<Vec<_>>()
        };
        let stopped_count = hosts.len();
        for host in hosts {
            host.terminate_without_join();
        }
        stopped_count
    }

    fn take_context_shutdown_work(
        &self,
    ) -> (
        Vec<SharedRendererServiceWorkerHost>,
        Vec<ServiceWorkerAbortedJob>,
        Vec<tokio::sync::oneshot::Sender<()>>,
    ) {
        let mut state = self.inner.state.lock();
        let force_update_page_load_waiters = state.take_all_force_update_page_load_waiters();
        let pending_update_checks = state
            .pending_main_script_update_checks
            .drain()
            .collect::<Vec<_>>();
        let mut hosts = Vec::new();
        let mut aborted_jobs = Vec::new();
        for (registration_id, pending_check) in pending_update_checks {
            if let Some(registration) = state.registrations.get_mut(&registration_id)
                && registration.installing_version_id == Some(pending_check.new_version_id)
            {
                registration.installing_version_id = None;
                registration
                    .pending_register_jobs
                    .remove(&pending_check.new_version_id);
            }
            hosts.extend(
                remove_version_and_shutdown_host_locked(&mut state, pending_check.new_version_id)
                    .into_iter()
                    .filter_map(|progress| match progress {
                        LifecycleProgress::TerminateHost(host) => Some(host),
                        _ => None,
                    }),
            );
            aborted_jobs.push(pending_check.abort());
        }
        hosts.extend(take_running_hosts_for_shutdown_locked(&mut state));
        let pending_devtools_launch_version_ids = state
            .pending_devtools_launches
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for version_id in pending_devtools_launch_version_ids {
            state.record_target_destroyed(version_id);
        }
        state.pending_devtools_launches.clear();
        state.pending_devtools_evaluation_releases.clear();
        aborted_jobs.extend(abort_pending_register_jobs_for_context_shutdown_locked(
            &mut state,
        ));
        aborted_jobs.extend(state.job_coordinator.abort_all());
        (hosts, aborted_jobs, force_update_page_load_waiters)
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

#[cfg(any(test, feature = "devtools"))]
pub(super) fn stop_worker_version_locked(
    state: &mut ServiceWorkerRuntimeState,
    version_id: ServiceWorkerVersionId,
    reason: &'static str,
) -> Option<SharedRendererServiceWorkerHost> {
    let version = state.versions.get_mut(&version_id)?;
    let host = version.running_state.take_host_for_shutdown()?;
    version.idle_timeout_token = None;
    state.record_target_stopped(version_id, host.run_identity(), reason);
    Some(host)
}

fn take_running_hosts_for_shutdown_locked(
    state: &mut ServiceWorkerRuntimeState,
) -> Vec<SharedRendererServiceWorkerHost> {
    state
        .versions
        .values_mut()
        .filter_map(|version| version.running_state.take_host_for_shutdown())
        .collect()
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
