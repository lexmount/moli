use crate::{
    browser::ServiceWorkerCommand,
    runtime::{RendererWorkerIdentity, RendererWorkerInspectionEndpoint},
};
use moli_shared_worker::SharedWorkerInstanceId;

use super::BrowserContext;

impl BrowserContext {
    pub(in crate::browser) fn install_worker_lifecycle_handler(
        &self,
        handler: impl Fn(crate::page::RendererWorkerLifecycleInput) + Send + Sync + 'static,
    ) {
        self.renderer_runtime()
            .install_worker_lifecycle_handler(handler);
    }

    pub(in crate::browser) fn worker_snapshots(
        &self,
    ) -> impl Iterator<Item = super::super::WorkerSnapshot> + '_ {
        self.shared_workers
            .values()
            .cloned()
            .map(|info| super::super::WorkerSnapshot::Shared {
                context: self.id(),
                info,
            })
            .chain(self.service_workers.values().cloned().map(|worker| {
                super::super::WorkerSnapshot::Service {
                    context: self.id(),
                    worker,
                }
            }))
            .chain(self.dedicated_workers.values().cloned().map(|worker| {
                super::super::WorkerSnapshot::Dedicated {
                    context: self.id(),
                    worker,
                }
            }))
    }

    pub fn worker_inspection_endpoint(
        &self,
        target: RendererWorkerIdentity,
    ) -> Option<RendererWorkerInspectionEndpoint> {
        self.renderer_runtime().worker_inspection_endpoint(target)
    }

    #[cfg(test)]
    pub(crate) fn worker_runtime_for_test(&self) -> crate::runtime::RendererBrowserContextRuntime {
        self.renderer_runtime()
    }

    pub fn execute_service_worker_command(
        &self,
        command: ServiceWorkerCommand,
    ) -> Result<(), String> {
        let runtime = self.renderer_runtime();
        match command {
            ServiceWorkerCommand::SetForceUpdateOnPageLoad(force_update) => {
                runtime.set_service_worker_force_update_on_page_load_for_devtools(force_update);
                Ok(())
            }
            ServiceWorkerCommand::Unregister { scope } => runtime
                .unregister_service_worker_scope_for_devtools(&scope)
                .map(drop),
            ServiceWorkerCommand::Start { scope } => {
                runtime.start_service_worker_for_devtools(&scope).map(drop)
            }
            ServiceWorkerCommand::StopVersion { version_id } => runtime
                .stop_service_worker_for_devtools(version_id)
                .map(drop),
            ServiceWorkerCommand::StopAll => {
                runtime.stop_all_service_workers_for_devtools().map(drop)
            }
            ServiceWorkerCommand::SkipWaiting { scope } => runtime
                .skip_waiting_service_worker_for_devtools(&scope)
                .map(drop),
            ServiceWorkerCommand::UpdateRegistration { scope } => runtime
                .update_service_worker_registration_for_devtools(&scope)
                .map(drop),
            ServiceWorkerCommand::DeliverPushMessage {
                origin,
                registration_id,
                data,
            } => runtime
                .deliver_push_message_for_devtools(&origin, registration_id, data)
                .map(drop),
            ServiceWorkerCommand::DispatchSyncEvent {
                origin,
                registration_id,
                tag,
                last_chance,
            } => runtime
                .dispatch_sync_event_for_devtools(&origin, registration_id, tag, last_chance)
                .map(drop),
            ServiceWorkerCommand::DispatchPeriodicSyncEvent {
                origin,
                registration_id,
                tag,
            } => runtime
                .dispatch_periodic_sync_event_for_devtools(&origin, registration_id, tag)
                .map(drop),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn service_worker_force_update_on_page_load_for_test(&self) -> bool {
        self.renderer_runtime()
            .service_worker_force_update_on_page_load_for_devtools()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn service_worker_pause_on_start_for_test(&self) -> bool {
        self.renderer_runtime()
            .service_worker_pause_on_start_for_devtools()
    }

    pub fn controlled_service_worker_window_client_ids(
        &self,
        registration_id: u64,
        version_id: u64,
    ) -> Vec<u64> {
        self.renderer_runtime()
            .controlled_service_worker_window_client_ids_for_devtools(registration_id, version_id)
    }

    pub fn set_service_worker_pause_on_start_for_version(
        &self,
        version_id: u64,
        pause: bool,
    ) -> bool {
        self.renderer_runtime()
            .set_service_worker_pause_on_start_for_version_for_devtools(version_id, pause)
    }

    pub fn set_service_worker_pause_on_start(&self, pause: bool) {
        self.renderer_runtime()
            .set_service_worker_pause_on_start_for_devtools(pause);
    }

    pub fn set_service_worker_related_pause_on_start_policies(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        self.renderer_runtime()
            .set_service_worker_related_pause_on_start_policies_for_devtools(policies);
    }

    pub fn set_dedicated_worker_pause_on_start(&self, pause: bool) {
        self.renderer_runtime()
            .set_dedicated_worker_pause_on_start_for_devtools(pause);
    }

    pub fn set_service_worker_inspection_attached(&self, version_id: u64, attached: bool) {
        self.renderer_runtime()
            .set_service_worker_devtools_attached(version_id, attached);
    }

    pub fn close_shared_worker(&self, instance_id: SharedWorkerInstanceId) -> bool {
        self.renderer_runtime().close_shared_worker(instance_id)
    }

    pub fn close_dedicated_worker(&self, instance_id: u64) -> bool {
        self.renderer_runtime()
            .close_dedicated_worker_for_devtools(instance_id)
    }

    pub fn run_dedicated_worker_if_waiting_for_debugger(&self, instance_id: u64) -> bool {
        self.renderer_runtime()
            .run_dedicated_worker_if_waiting_for_debugger_for_devtools(instance_id)
    }

    pub fn run_service_worker_if_waiting_for_debugger(&self, version_id: u64) -> bool {
        self.renderer_runtime()
            .run_service_worker_if_waiting_for_debugger_for_devtools(version_id)
    }
}
