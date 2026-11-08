use super::{BackgroundProtocolEvent, CdpConnection};
use crate::automation::FrontendProtocol;

impl CdpConnection {
    /// Registers a native adapter for all browser contexts owned by this connection.
    /// A non-modal handler can handle prompts without making the renderer wait.
    /// Re-registering an adapter updates its waiting policy without adding a lease.
    pub fn register_automation_javascript_dialog_handler(
        &mut self,
        protocol: FrontendProtocol,
        wait_enabled: bool,
    ) -> bool {
        if protocol == FrontendProtocol::Cdp {
            return false;
        }
        self.automation_javascript_dialog_handlers
            .insert(protocol, wait_enabled);
        self.sync_automation_javascript_dialog_handlers();
        true
    }

    /// Releases only this adapter's registration and settles abandoned dialogs
    /// before its runtime cleanup can wait on the renderer.
    pub fn unregister_automation_javascript_dialog_handler(
        &mut self,
        protocol: FrontendProtocol,
    ) -> Vec<BackgroundProtocolEvent> {
        if self
            .automation_javascript_dialog_handlers
            .remove(&protocol)
            .is_none()
        {
            return Vec::new();
        }
        self.sync_automation_javascript_dialog_handlers();
        crate::domains::page::dismiss_unhandled_javascript_dialogs(self)
    }

    pub fn has_automation_javascript_dialog_handler(&self) -> bool {
        !self.automation_javascript_dialog_handlers.is_empty()
    }

    pub(crate) fn automation_javascript_dialog_wait_enabled(&self) -> bool {
        self.automation_javascript_dialog_handlers
            .values()
            .any(|wait_enabled| *wait_enabled)
    }

    fn sync_automation_javascript_dialog_handlers(&self) {
        for context in self
            .browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
        {
            self.sync_javascript_dialog_handler_enabled_for_browser_context(&context.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::{
        AutomationCommand, AutomationContext, DevToolsGetBrowserContextsCommand,
        DevToolsGetRealmsCommand, DevToolsGetTargetInfoCommand, DevToolsGetTargetsCommand,
    };
    use crate::conn::BrowserContext;

    fn connection() -> CdpConnection {
        let mut conn = CdpConnection::default();
        conn.install_browser_context_fixture_for_test(BrowserContext::new_with_page_for_test(
            "BID-1", "TID-1",
        ));
        conn
    }

    fn context(protocol: FrontendProtocol) -> AutomationContext {
        AutomationContext {
            protocol,
            session_id: None,
            target_id: Some("TID-1".into()),
            browser_context_id: Some("BID-1".into()),
        }
    }

    #[test]
    fn native_discovery_commands_do_not_register_dialog_handlers() {
        for protocol in [
            FrontendProtocol::WebDriverClassic,
            FrontendProtocol::WebDriverBidi,
        ] {
            let mut conn = connection();
            for command in [
                AutomationCommand::GetTargets(DevToolsGetTargetsCommand {
                    context: context(protocol),
                    root: None,
                    max_depth: None,
                    filter: None,
                }),
                AutomationCommand::GetBrowserContexts(DevToolsGetBrowserContextsCommand {
                    context: context(protocol),
                }),
            ] {
                let outcome = conn
                    .try_execute_immediate_automation_command(command)
                    .expect("immediate discovery command");
                outcome.into_complete_parts().0.expect("discovery succeeds");
                assert!(!conn.has_automation_javascript_dialog_handler());
                assert!(!conn.automation_javascript_dialog_wait_enabled());
            }
        }
    }

    #[test]
    fn failed_native_commands_do_not_register_dialog_handlers() {
        for protocol in [
            FrontendProtocol::WebDriverClassic,
            FrontendProtocol::WebDriverBidi,
        ] {
            let mut conn = connection();
            let outcome = conn
                .try_execute_immediate_automation_command(AutomationCommand::GetTargetInfo(
                    DevToolsGetTargetInfoCommand {
                        context: context(protocol),
                        target_id: Some("missing-target".into()),
                    },
                ))
                .expect("immediate target query");
            assert!(outcome.into_complete_parts().0.is_err());
            assert!(!conn.has_automation_javascript_dialog_handler());
            assert!(!conn.automation_javascript_dialog_wait_enabled());
        }
    }

    #[test]
    fn non_immediate_native_commands_do_not_register_dialog_handlers() {
        for protocol in [
            FrontendProtocol::WebDriverClassic,
            FrontendProtocol::WebDriverBidi,
        ] {
            let mut conn = connection();
            let command = AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
                context: context(protocol),
                realm_type: None,
            });
            assert!(
                conn.try_execute_immediate_automation_command(command)
                    .is_err()
            );
            assert!(!conn.has_automation_javascript_dialog_handler());
            assert!(!conn.automation_javascript_dialog_wait_enabled());
        }
    }

    #[test]
    fn native_dialog_registrations_own_availability_and_modal_waits_independently() {
        let mut conn = CdpConnection::default();
        assert!(!conn.register_automation_javascript_dialog_handler(FrontendProtocol::Cdp, true));
        assert!(!conn.has_automation_javascript_dialog_handler());

        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false,
            )
        );
        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false,
            )
        );
        let first = conn.new_browser_context("BID-first".to_owned());
        let first_runtime = first.renderer_runtime();
        conn.install_browser_context_fixture_for_test(first);
        assert!(conn.has_automation_javascript_dialog_handler());
        assert!(!first_runtime.javascript_dialog_handler_enabled());

        assert!(conn.register_automation_javascript_dialog_handler(
            FrontendProtocol::WebDriverClassic,
            true,
        ));
        assert!(first_runtime.javascript_dialog_handler_enabled());
        let second = conn.new_browser_context("BID-second".to_owned());
        let second_runtime = second.renderer_runtime();
        conn.push_inactive_browser_context_fixture_for_test(second);
        assert!(second_runtime.javascript_dialog_handler_enabled());

        assert!(
            conn.unregister_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverClassic
            )
            .is_empty()
        );
        assert!(conn.has_automation_javascript_dialog_handler());
        assert!(!first_runtime.javascript_dialog_handler_enabled());
        assert!(!second_runtime.javascript_dialog_handler_enabled());

        assert!(
            conn.unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi)
                .is_empty()
        );
        assert!(!conn.has_automation_javascript_dialog_handler());
        assert!(!conn.automation_javascript_dialog_wait_enabled());
        assert!(
            conn.unregister_automation_javascript_dialog_handler(FrontendProtocol::WebDriverBidi)
                .is_empty()
        );
    }
}
