use moli_core::{browser::ServiceWorkerCommand, runtime::RendererBrowserContextRuntime};
use moli_shared_worker::SharedWorkerInstanceId;

use super::{BrowserContext, physical::BrowserContext as PhysicalBrowserContext};

impl PhysicalBrowserContext {
    fn execute_service_worker_command(&self, command: ServiceWorkerCommand) -> Result<(), String> {
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
}

impl BrowserContext {
    pub(in crate::conn) fn execute_service_worker_command(
        &self,
        command: ServiceWorkerCommand,
    ) -> Result<(), String> {
        self.physical.execute_service_worker_command(command)
    }

    #[cfg(test)]
    pub(crate) fn service_worker_force_update_on_page_load(&self) -> bool {
        self.physical
            .renderer_runtime()
            .service_worker_force_update_on_page_load_for_devtools()
    }

    #[cfg(test)]
    pub(crate) fn service_worker_pause_on_start(&self) -> bool {
        self.physical
            .renderer_runtime()
            .service_worker_pause_on_start_for_devtools()
    }

    pub(crate) fn controlled_service_worker_window_client_ids(
        &self,
        registration_id: u64,
        version_id: u64,
    ) -> Vec<u64> {
        self.physical
            .renderer_runtime()
            .controlled_service_worker_window_client_ids_for_devtools(registration_id, version_id)
    }

    pub(crate) fn set_service_worker_pause_on_start_for_version(
        &self,
        version_id: u64,
        pause: bool,
    ) -> bool {
        self.physical
            .renderer_runtime()
            .set_service_worker_pause_on_start_for_version_for_devtools(version_id, pause)
    }

    pub(crate) fn set_service_worker_pause_on_start(&self, pause: bool) {
        self.physical
            .renderer_runtime()
            .set_service_worker_pause_on_start_for_devtools(pause);
    }

    pub(crate) fn set_service_worker_related_pause_on_start_policies(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        self.physical
            .renderer_runtime()
            .set_service_worker_related_pause_on_start_policies_for_devtools(policies);
    }

    pub(crate) fn set_dedicated_worker_pause_on_start(&self, pause: bool) {
        self.physical
            .renderer_runtime()
            .set_dedicated_worker_pause_on_start_for_devtools(pause);
    }

    pub(crate) fn set_service_worker_devtools_attached(&self, version_id: u64, attached: bool) {
        self.physical
            .renderer_runtime()
            .set_service_worker_devtools_attached(version_id, attached);
    }

    pub(in crate::conn) fn close_shared_worker(&self, instance_id: SharedWorkerInstanceId) -> bool {
        self.physical
            .renderer_runtime()
            .close_shared_worker_for_target_close(instance_id)
    }

    pub(in crate::conn) fn close_dedicated_worker(&self, instance_id: u64) -> bool {
        self.physical
            .renderer_runtime()
            .close_dedicated_worker_for_devtools(instance_id)
    }

    pub(crate) fn attach_dedicated_worker_inspector_session(
        &self,
        instance_id: u64,
        session_id: Option<String>,
    ) -> bool {
        self.physical
            .renderer_runtime()
            .attach_dedicated_worker_runtime_inspector_session(instance_id, session_id)
    }

    pub(crate) fn detach_shared_worker_inspector_session(
        &self,
        instance_id: SharedWorkerInstanceId,
        session_id: Option<String>,
    ) -> bool {
        self.physical
            .renderer_runtime()
            .detach_shared_worker_runtime_inspector_session(instance_id, session_id)
    }

    pub(crate) fn detach_dedicated_worker_inspector_session(
        &self,
        instance_id: u64,
        session_id: Option<String>,
    ) -> bool {
        self.physical
            .renderer_runtime()
            .detach_dedicated_worker_runtime_inspector_session(instance_id, session_id)
    }

    pub(crate) fn detach_service_worker_inspector_session(
        &self,
        version_id: u64,
        session_id: Option<String>,
    ) -> bool {
        self.physical
            .renderer_runtime()
            .detach_service_worker_runtime_inspector_session(version_id, session_id)
    }

    pub(in crate::conn) fn run_dedicated_worker_if_waiting_for_debugger(
        &self,
        instance_id: u64,
    ) -> bool {
        self.physical
            .renderer_runtime()
            .run_dedicated_worker_if_waiting_for_debugger_for_devtools(instance_id)
    }

    pub(crate) fn run_service_worker_if_waiting_for_debugger(&self, version_id: u64) -> bool {
        self.physical
            .renderer_runtime()
            .run_service_worker_if_waiting_for_debugger_for_devtools(version_id)
    }

    /// Layered DevTools inspection is the only Protocol path allowed to retain
    /// the cloneable renderer endpoint while a worker command is in flight.
    pub(in crate::conn) fn worker_runtime_inspection_endpoint(
        &self,
    ) -> RendererBrowserContextRuntime {
        self.physical.renderer_runtime()
    }
}
