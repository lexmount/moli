use super::*;
use crate::{
    automation::{AutomationContext, AutomationResult, DevToolsError},
    domains::{browser, emulation, storage},
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_automation_permission(
        self,
        completed: browser::CompletedBrowserCommandDispatch,
    ) -> Result<AutomationResult, DevToolsError> {
        browser::finish_devtools_set_permission_command(self.connection, completed)
    }

    pub(crate) fn finish_automation_emulation(
        self,
        completed: emulation::CompletedEmulationAutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        completed.finish(self.connection)
    }

    pub(crate) fn finish_automation_storage(
        self,
        completed: storage::CompletedStorageCommandDispatch,
    ) -> Result<AutomationResult, DevToolsError> {
        storage::complete_pending_set_cookies_result(self.connection, completed)
            .map(AutomationResult::SetCookies)
    }

    pub(crate) fn finish_configuration_automation_output(
        self,
        command: &AutomationContext,
        scope: Option<&crate::conn::ProtocolPageHostScope>,
        context: &mut CommandDispatchContext,
    ) {
        if scope.is_some_and(|scope| !scope.is_current(self.connection)) {
            return;
        }
        let mut events = Vec::new();
        self.connection
            .project_protocol_local_outputs_for_direct_command(command, context, &mut events);
        context.protocol_events_mut().extend(events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::*;
    use crate::conn::{AutomationCommandTaskStep, BrowserContext, ProtocolStateOwner};
    use crate::testing::TestContext;
    use serde_json::json;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-configuration".into());
        browser.set_active_target_id("TID-configuration");
        browser.attach_active_session("SID-configuration");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-configuration"))
            .await;
        context
    }

    fn command_context() -> AutomationContext {
        AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: Some("TID-configuration".into()),
            browser_context_id: None,
        }
    }

    fn permission_command() -> AutomationCommand {
        AutomationCommand::SetPermission(DevToolsSetPermissionCommand {
            context: command_context(),
            permission: json!({"name":"geolocation"}),
            setting: "granted".into(),
            origin: "https://configuration.example".into(),
            embedded_origin: None,
            browser_context_id: None,
        })
    }

    fn cookie_command() -> AutomationCommand {
        AutomationCommand::SetCookies(DevToolsSetCookiesCommand {
            context: command_context(),
            browser_context_id: None,
            cookies: vec![DevToolsCookieParam {
                name: "owned".into(),
                value: "yes".into(),
                url: Some("https://configuration.example/".into()),
                domain: None,
                path: None,
                secure: None,
                http_only: false,
                same_site: None,
                priority: None,
                source_scheme: None,
                source_port: None,
                partition_key: None,
                partition_key_opaque: None,
                expires: None,
            }],
        })
    }

    fn window_command() -> AutomationCommand {
        AutomationCommand::SetWindowState(DevToolsSetWindowStateCommand {
            context: command_context(),
            state: DevToolsWindowState::Minimized,
        })
    }

    #[tokio::test]
    async fn configuration_families_keep_typed_results_without_a_mailbox() {
        let mut context = fixture().await;
        drop(context.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        for command in [permission_command(), cookie_command(), window_command()] {
            let mut step = handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(command)
                })
                .unwrap();
            let result = loop {
                match step {
                    AutomationCommandTaskStep::Pending(pending) => {
                        // Synchronous state access is available while this
                        // command's owned renderer wait is retained elsewhere.
                        assert!(
                            handle
                                .with_state(|state| state
                                    .connection
                                    .has_browser_context_id("BID-configuration"))
                                .unwrap()
                        );
                        let completed = pending.wait().await;
                        step = handle
                            .with_state(move |state| {
                                state
                                    .connection
                                    .complete_automation_command_dispatch(completed)
                            })
                            .unwrap();
                    }
                    AutomationCommandTaskStep::Complete(outcome) => {
                        break outcome.into_result_and_owner_turn().0.unwrap();
                    }
                }
            };
            assert!(matches!(
                result,
                AutomationResult::Empty | AutomationResult::SetCookies(_)
            ));
        }
        handle
            .with_state(|state| {
                let context = state.connection.browser_context.as_ref().unwrap();
                assert!(
                    context
                        .active_page_target()
                        .owner_state
                        .window_document_hidden()
                );
                assert_eq!(context.snapshot_cookies().len(), 1);
            })
            .unwrap();
    }

    #[tokio::test]
    async fn configuration_finish_releases_connection_and_preserves_original_result() {
        for command in [permission_command(), cookie_command(), window_command()] {
            let mut context = fixture().await;
            let mut requests = context.conn.take_protocol_state_requests().unwrap();
            let AutomationCommandTaskStep::Pending(pending) =
                context.conn.start_automation_command_dispatch(command)
            else {
                panic!("configuration uses its ordinary owned workflow");
            };
            let mut wait = Box::pin(pending.wait());
            let mut operations = 0;
            let completed = loop {
                tokio::select! {
                    request = requests.recv() => {
                        let request = request.unwrap();
                        operations += 1;
                        // The original renderer wait has settled, but its
                        // completion has not been adopted. Peer work can enter.
                        let command = AutomationCommand::GetTargets(DevToolsGetTargetsCommand { context: command_context(), filter: None, root: None, max_depth: None });
                        let (result, _) = crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(&mut context.conn, command);
                        assert!(matches!(result.unwrap(), AutomationResult::GetTargets(_)));
                        request.apply(&mut context.conn);
                    }
                    completed = &mut wait => break completed,
                }
            };
            assert!(operations > 0);
            let AutomationCommandTaskStep::Complete(outcome) =
                context.conn.complete_automation_command_dispatch(completed)
            else {
                panic!("the final state commit is finite");
            };
            assert!(outcome.into_result_and_owner_turn().0.is_ok());
        }
    }

    #[tokio::test]
    async fn configuration_late_client_window_tail_cannot_mutate_a_recreated_host() {
        let mut original = fixture().await;
        let replacement = fixture().await;
        let prepared = emulation::prepare_emulation_automation_command(
            &mut original.conn,
            AutomationCommand::SetClientWindowState(DevToolsSetClientWindowStateCommand {
                context: command_context(),
                client_window: "TID-configuration".into(),
                state: DevToolsWindowState::Minimized,
                width: Some(777),
                height: Some(888),
                x: Some(9),
                y: Some(10),
            }),
        )
        .unwrap();
        let completed = prepared.wait().await;
        let expected_geometry = replacement
            .conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .owner_state
            .window_surface_geometry;
        original.conn = replacement.conn;
        let error = completed.finish(&mut original.conn).unwrap_err();
        assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
        let state = &original
            .conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .owner_state;
        assert!(!state.window_document_hidden());
        assert_eq!(state.window_surface_geometry, expected_geometry);
    }
    #[tokio::test]
    async fn configuration_cookie_default_url_stays_with_original_target_after_activation() {
        let mut context = fixture().await;
        context
            .install_buffered_navigation_fixture_for_session_owner(
                url::Url::parse("https://cookie-original.example/page").unwrap(),
                "<!doctype html><p>original</p>".into(),
                Some("SID-configuration"),
            )
            .await;
        let AutomationCommand::SetCookies(mut command) = cookie_command() else {
            unreachable!()
        };
        command.cookies[0].url = None;
        let prepared = storage::prepare_storage_automation_command(
            &mut context.conn,
            AutomationCommand::SetCookies(command),
        );
        let storage::PreparedStorageAutomationCommand::SetCookies(pending) = prepared else {
            panic!("a live document cookie owner is queried asynchronously");
        };
        let completed = pending.wait().await;
        let browser = context.conn.browser_context.as_mut().unwrap();
        browser.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-selected-later".into(),
            None,
            "https://cookie-other.example/".into(),
        ));
        browser.set_active_target_id("TID-selected-later");
        assert!(storage::complete_pending_set_cookies_result(&mut context.conn, completed).is_ok());
        let browser = context.conn.browser_context.as_ref().unwrap();
        let cookies = browser.snapshot_cookies();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].domain, "cookie-original.example");
        assert_eq!(browser.active_target_id(), Some("TID-selected-later"));
    }

    #[tokio::test]
    async fn configuration_cookie_ready_receipt_cannot_write_into_a_recreated_context() {
        let mut original = fixture().await;
        let replacement = fixture().await;
        let prepared =
            storage::prepare_storage_automation_command(&mut original.conn, cookie_command());
        let storage::PreparedStorageAutomationCommand::SetCookies(pending) = prepared else {
            panic!("a live document cookie owner is queried asynchronously");
        };
        let completed = pending.wait().await;
        original.conn = replacement.conn;
        let error = storage::complete_pending_set_cookies_result(&mut original.conn, completed)
            .unwrap_err();
        assert_eq!(error.kind, DevToolsErrorKind::Internal);
        assert!(error.message.contains("NoDocumentLoaded"));
        assert!(
            original
                .conn
                .browser_context
                .as_ref()
                .unwrap()
                .snapshot_cookies()
                .is_empty()
        );
    }
}
