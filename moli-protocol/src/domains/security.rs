use chromiumoxide_cdp::cdp::browser_protocol::security::SetIgnoreCertificateErrorsParams;

use super::actions::SecurityAction;
use super::command_output::CommandOutputPlan;
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};

pub(crate) struct PendingSecurityCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    pending: moli_core::page::PendingPageCommand,
}

pub(crate) struct CompletedSecurityCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    completed: Result<moli_core::page::CompletedPageCommand, String>,
}

pub(crate) enum SecurityCommandTaskStep {
    Pending(PendingSecurityCommandDispatch),
    Complete(CommandOutputPlan),
}

impl PendingSecurityCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedSecurityCommandDispatch {
        CompletedSecurityCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedSecurityCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

fn sync_command_output_plan(cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<SecurityAction>() {
        Some(
            SecurityAction::Enable
            | SecurityAction::Disable
            | SecurityAction::HandleCertificateError
            | SecurityAction::SetOverrideCertificateErrors,
        ) => CommandOutputPlan::success(),
        Some(SecurityAction::SetIgnoreCertificateErrors) | None => {
            CommandOutputPlan::error(-32601, "UnknownMethod")
        }
    }
}

pub(crate) fn try_start_security_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> SecurityCommandTaskStep {
    if cmd.parse_action::<SecurityAction>() != Some(SecurityAction::SetIgnoreCertificateErrors) {
        return SecurityCommandTaskStep::Complete(sync_command_output_plan(cmd));
    }
    let params: SetIgnoreCertificateErrorsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return SecurityCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    match conn.start_set_tls_verify_host_for_session_owner(cmd.session_id, !params.ignore) {
        Ok(Some(pending)) => SecurityCommandTaskStep::Pending(PendingSecurityCommandDispatch {
            command_id: cmd.id,
            owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
            pending,
        }),
        Ok(None) => SecurityCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(message) if message == "BrowserContextNotLoaded" => SecurityCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => {
            SecurityCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message))
        }
    }
}

pub(crate) fn complete_pending_security_command(
    conn: &mut CdpConnection,
    completed: CompletedSecurityCommandDispatch,
) -> CommandOutputPlan {
    let completion = match completed.completed {
        Ok(completion) => completion,
        Err(error) => return CommandOutputPlan::error(-32000, error),
    };
    let owner_scope = completed.owner_scope;
    match conn.finish_rebuild_resource_runtime_for_route(
        owner_scope.session_id(),
        owner_scope.session_owner_route(),
        completion,
    ) {
        Ok(()) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{conn::BrowserContext, testing::TestContext};

    #[tokio::test(flavor = "multi_thread")]
    async fn set_ignore_certificate_errors_toggles_tls_verification() {
        let mut ctx = TestContext::new();
        ctx.conn
            .insert_browser_context(BrowserContext::new("BID-9".into()));

        ctx.process_async(json!({
            "id": 11,
            "method": "Security.setIgnoreCertificateErrors",
            "params": { "ignore": true }
        }))
        .await;
        ctx.expect_result(11, json!({}), None);
        assert!(!ctx.conn.tls_verify_host());

        ctx.process_async(json!({
            "id": 12,
            "method": "Security.setIgnoreCertificateErrors",
            "params": { "ignore": false }
        }))
        .await;
        ctx.expect_result(12, json!({}), None);
        assert!(ctx.conn.tls_verify_host());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_ignore_certificate_errors_is_scoped_to_active_browser_context() {
        let mut ctx = TestContext::new();
        let mut first = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        first.attach_active_session("SID-1");
        let mut second = BrowserContext::new_with_page_for_test("BID-2", "TID-2");
        second.attach_active_session("SID-2");
        ctx.conn.insert_browser_context(first);
        ctx.conn.insert_browser_context(second);

        ctx.process_async(json!({
            "id": 13,
            "method": "Security.setIgnoreCertificateErrors",
            "sessionId": "SID-2",
            "params": { "ignore": true }
        }))
        .await;
        ctx.expect_result(13, json!({}), Some("SID-2"));
        assert_eq!(ctx.conn.browser_context.as_ref().unwrap().id, "BID-1");
        assert!(ctx.conn.tls_verify_host());
        assert_eq!(
            ctx.conn
                .inactive_browser_contexts
                .iter()
                .find(|bc| bc.id == "BID-2")
                .expect("session browser context")
                .tls_verify_host_override,
            Some(false)
        );
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .and_then(|bc| bc.tls_verify_host_override),
            None
        );
    }
}
