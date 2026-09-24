use moli_core::browser::{BrowserContextId, ServiceWorkerCommand};
use moli_shared_worker::SharedWorkerInstanceId;

use super::CdpConnection;

impl CdpConnection {
    pub(crate) fn browser_context_handle_for_devtools_id(
        &self,
        browser_context_id: &str,
    ) -> Result<BrowserContextId, String> {
        self.browser_context_by_id(browser_context_id)
            .map(super::BrowserContext::browser_context_id)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())
    }

    pub(crate) fn execute_browser_service_worker_command(
        &self,
        context: BrowserContextId,
        command: ServiceWorkerCommand,
    ) -> Result<(), String> {
        self.browser_context_by_browser_id(context)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())?
            .execute_service_worker_command(command)
    }

    pub(crate) fn close_browser_shared_worker(
        &self,
        context: BrowserContextId,
        instance_id: SharedWorkerInstanceId,
    ) -> Result<bool, String> {
        Ok(self
            .browser_context_by_browser_id(context)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())?
            .close_shared_worker(instance_id))
    }

    pub(crate) fn close_browser_dedicated_worker(
        &self,
        context: BrowserContextId,
        instance_id: u64,
    ) -> Result<bool, String> {
        Ok(self
            .browser_context_by_browser_id(context)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())?
            .close_dedicated_worker(instance_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_worker_control_rejects_a_replaced_devtools_context_id() {
        let mut conn = crate::test_support::connection();
        conn.install_browser_context_fixture_for_test(
            conn.new_browser_context_fixture_for_test("BID-worker-control"),
        );
        let original = conn
            .browser_context_handle_for_devtools_id("BID-worker-control")
            .unwrap();

        conn.install_browser_context_fixture_for_test(
            conn.new_browser_context_fixture_for_test("BID-worker-control"),
        );
        let replacement = conn
            .browser_context_handle_for_devtools_id("BID-worker-control")
            .unwrap();
        assert_ne!(original, replacement);

        assert_eq!(
            conn.execute_browser_service_worker_command(
                original,
                ServiceWorkerCommand::SetForceUpdateOnPageLoad(true),
            ),
            Err("BrowserContext unavailable".into())
        );
        assert!(
            !conn
                .browser_context
                .as_ref()
                .unwrap()
                .service_worker_force_update_on_page_load()
        );
    }
}
