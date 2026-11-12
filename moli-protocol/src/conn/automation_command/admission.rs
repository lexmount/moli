use super::*;

/// An automation command waiting for its browsing context's Document to be ready.
/// The original host and binding remain authoritative across navigation, but
/// the Document scope is captured only when dispatch can actually start.
#[derive(Debug)]
pub struct PendingAutomationCommandAdmission {
    command: AutomationCommand,
    host: ProtocolPageHostScope,
}

impl CdpConnection {
    /// Capture the original target and session binding for multi-step input.
    pub fn automation_page_host_scope(
        &mut self,
        context: &AutomationContext,
    ) -> Option<ProtocolPageHostScope> {
        let owner = self.command_owner_scope_for_devtools_context(context)?;
        ProtocolPageHostScope::capture_for_operation(self, &owner).ok()
    }

    pub fn automation_page_host_can_start_operation(&self, scope: &ProtocolPageHostScope) -> bool {
        scope.can_start_operation(self)
    }

    fn automation_document_admission_is_pending(&mut self, context: &AutomationContext) -> bool {
        self.has_inflight_background_navigation_for_devtools_context(context)
            || matches!(
                self.devtools_context_document_navigation_state(context),
                DevToolsDocumentNavigationState::PendingNavigation
                    | DevToolsDocumentNavigationState::AwaitingCommit
            )
    }

    pub(crate) fn start_automation_command_with_navigation_admission(
        &mut self,
        command: AutomationCommand,
    ) -> Result<AutomationCommandTaskStep, Box<PendingAutomationCommandAdmission>> {
        let context = command.context();
        if context.target_id.is_some()
            && self.automation_document_admission_is_pending(context)
            && let Some(owner) = self.command_owner_scope_for_devtools_context(context)
            && let Ok(host) = ProtocolPageHostScope::capture(self, &owner)
        {
            return self.resume_automation_command_admission(Box::new(
                PendingAutomationCommandAdmission { command, host },
            ));
        }
        Ok(self.start_automation_command_dispatch(command))
    }

    pub(crate) fn resume_automation_command_admission(
        &mut self,
        pending: Box<PendingAutomationCommandAdmission>,
    ) -> Result<AutomationCommandTaskStep, Box<PendingAutomationCommandAdmission>> {
        if !pending.host.can_start_operation(self) {
            return Ok(AutomationCommandTaskStep::Complete(Box::new(
                AutomationCommandDispatchOutcome::new_with_protocol_events(
                    Err(DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        "Browsing context closed before command admission",
                    )),
                    Vec::new(),
                    Vec::new(),
                ),
            )));
        }
        if self.automation_document_admission_is_pending(pending.command.context()) {
            return Err(pending);
        }
        // Validation and dispatch share one finite owner turn. No unrelated
        // work can replace the host between this check and Document capture.
        Ok(self.start_automation_command_dispatch(pending.command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::{DevToolsGetRealmsCommand, FrontendProtocol};

    fn fixture() -> (CdpConnection, Box<PendingAutomationCommandAdmission>) {
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new_with_page_for_test("BID-admission", "TID-admission");
        context.attach_active_session("SID-admission");
        conn.install_browser_context_fixture_for_test(context);
        crate::test_support::arm_background_navigation_request_for_target(
            &mut conn,
            "TID-admission",
            "LID-admission",
        );
        let command = AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: Some("SID-admission".into()),
                target_id: Some("TID-admission".into()),
                browser_context_id: Some("BID-admission".into()),
            },
            realm_type: None,
        });
        let pending = conn
            .start_automation_command_with_navigation_admission(command)
            .err()
            .expect("a live navigation defers admission");
        (conn, pending)
    }

    fn assert_revoked(conn: &mut CdpConnection, pending: Box<PendingAutomationCommandAdmission>) {
        let Ok(AutomationCommandTaskStep::Complete(outcome)) =
            conn.resume_automation_command_admission(pending)
        else {
            panic!("a revoked host must end admission without dispatch");
        };
        let (result, scheduler, events, predecessor) = outcome.into_complete_parts();
        assert_eq!(result.unwrap_err().kind, DevToolsErrorKind::NoSuchTarget);
        assert!(scheduler.is_empty());
        assert!(events.is_empty());
        assert!(predecessor.is_none());
    }

    #[test]
    fn runtime_admission_keeps_its_owner_while_navigation_is_pending() {
        let (mut conn, pending) = fixture();
        let original = pending.host.clone();
        let pending = conn
            .resume_automation_command_admission(pending)
            .err()
            .expect("no readiness progress without a Document");
        assert!(original.is_current(&conn));
        assert!(pending.host.is_current(&conn));
        assert!(conn.has_inflight_background_navigation_for_target("TID-admission"));
    }

    #[test]
    fn runtime_admission_rejects_a_recreated_target_with_the_same_ids() {
        let (mut conn, pending) = fixture();
        let context = conn.browser_context.as_mut().unwrap();
        let old = context.take_page_target_for_close("TID-admission").unwrap();
        context.insert_page_target_host(PageTargetHost::with_url(
            "TID-admission".into(),
            Some("SID-admission".into()),
            "about:blank".into(),
        ));
        crate::test_support::arm_background_navigation_request_for_target(
            &mut conn,
            "TID-admission",
            "LID-admission",
        );
        assert_revoked(&mut conn, pending);
        assert!(conn.has_inflight_background_navigation_for_target("TID-admission"));
        drop(old);
    }

    #[test]
    fn runtime_admission_rejects_an_identically_rebound_session() {
        let (mut conn, pending) = fixture();
        let route = conn.session_route(Some("SID-admission")).unwrap();
        conn.register_session_route_for_test("SID-admission", route);
        assert!(!pending.host.binding_is_current());
        assert_revoked(&mut conn, pending);
        assert!(conn.has_inflight_background_navigation_for_target("TID-admission"));
    }

    #[test]
    fn runtime_admission_cannot_dispatch_into_a_closing_host() {
        let (mut conn, pending) = fixture();
        conn.browser_context
            .as_mut()
            .unwrap()
            .page_target_mut("TID-admission")
            .unwrap()
            .closing = true;
        assert_revoked(&mut conn, pending);
    }
}
