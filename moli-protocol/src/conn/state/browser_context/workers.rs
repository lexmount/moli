use moli_core::{browser::ServiceWorkerCommand, runtime::RendererBrowserContextRuntime};
use moli_shared_worker::SharedWorkerInstanceId;

use super::BrowserContext;

impl BrowserContext {
    pub(in crate::conn) fn execute_service_worker_command(
        &self,
        command: ServiceWorkerCommand,
    ) -> Result<(), String> {
        self.browser_context.execute_service_worker_command(command)
    }

    #[cfg(test)]
    pub(crate) fn service_worker_force_update_on_page_load(&self) -> bool {
        self.browser_context
            .service_worker_force_update_on_page_load_for_test()
    }

    #[cfg(test)]
    pub(crate) fn service_worker_pause_on_start(&self) -> bool {
        self.browser_context
            .service_worker_pause_on_start_for_test()
    }

    pub(crate) fn controlled_service_worker_window_client_ids(
        &self,
        registration_id: u64,
        version_id: u64,
    ) -> Vec<u64> {
        self.browser_context
            .controlled_service_worker_window_client_ids(registration_id, version_id)
    }

    pub(crate) fn set_service_worker_pause_on_start_for_version(
        &self,
        version_id: u64,
        pause: bool,
    ) -> bool {
        self.browser_context
            .set_service_worker_pause_on_start_for_version(version_id, pause)
    }

    pub(crate) fn set_service_worker_pause_on_start(&self, pause: bool) {
        self.browser_context
            .set_service_worker_pause_on_start(pause);
    }

    pub(crate) fn set_service_worker_related_pause_on_start_policies(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        self.browser_context
            .set_service_worker_related_pause_on_start_policies(policies);
    }

    pub(crate) fn set_dedicated_worker_pause_on_start(&self, pause: bool) {
        self.browser_context
            .set_dedicated_worker_pause_on_start(pause);
    }

    pub(crate) fn set_service_worker_devtools_attached(&self, version_id: u64, attached: bool) {
        self.browser_context
            .set_service_worker_inspection_attached(version_id, attached);
    }

    pub(in crate::conn) fn close_shared_worker(&self, instance_id: SharedWorkerInstanceId) -> bool {
        self.browser_context.close_shared_worker(instance_id)
    }

    pub(in crate::conn) fn close_dedicated_worker(&self, instance_id: u64) -> bool {
        self.browser_context.close_dedicated_worker(instance_id)
    }

    pub(crate) fn attach_dedicated_worker_inspector_session(
        &self,
        instance_id: u64,
        session_id: Option<String>,
    ) -> bool {
        self.browser_context
            .attach_dedicated_worker_inspector_session(instance_id, session_id)
    }

    pub(crate) fn detach_shared_worker_inspector_session(
        &self,
        instance_id: SharedWorkerInstanceId,
        session_id: Option<String>,
    ) -> bool {
        self.browser_context
            .detach_shared_worker_inspector_session(instance_id, session_id)
    }

    pub(crate) fn detach_dedicated_worker_inspector_session(
        &self,
        instance_id: u64,
        session_id: Option<String>,
    ) -> bool {
        self.browser_context
            .detach_dedicated_worker_inspector_session(instance_id, session_id)
    }

    pub(crate) fn detach_service_worker_inspector_session(
        &self,
        version_id: u64,
        session_id: Option<String>,
    ) -> bool {
        self.browser_context
            .detach_service_worker_inspector_session(version_id, session_id)
    }

    pub(in crate::conn) fn run_dedicated_worker_if_waiting_for_debugger(
        &self,
        instance_id: u64,
    ) -> bool {
        self.browser_context
            .run_dedicated_worker_if_waiting_for_debugger(instance_id)
    }

    pub(crate) fn run_service_worker_if_waiting_for_debugger(&self, version_id: u64) -> bool {
        self.browser_context
            .run_service_worker_if_waiting_for_debugger(version_id)
    }

    /// Layered DevTools inspection is the only Protocol path allowed to retain
    /// the cloneable renderer endpoint while a worker command is in flight.
    pub(in crate::conn) fn worker_runtime_inspection_endpoint(
        &self,
    ) -> RendererBrowserContextRuntime {
        self.browser_context.worker_runtime_inspection_endpoint()
    }
}
