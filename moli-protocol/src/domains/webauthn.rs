use crate::conn::Cmd;
use crate::domains::actions::WebAuthnAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) fn command_output_plan(cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<WebAuthnAction>() {
        Some(WebAuthnAction::Enable | WebAuthnAction::Disable) => CommandOutputPlan::success(),
        None => CommandOutputPlan::error(-32601, "UnknownMethod"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::conn::BrowserContext;
    use crate::testing::TestContext;

    #[tokio::test(flavor = "multi_thread")]
    async fn webauthn_enable_disable_are_acknowledged() {
        // Chromium source:
        // third_party/blink/web_tests/http/tests/inspector-protocol/webauthn/*.js
        let mut ctx = TestContext::new();
        let mut browser_context = BrowserContext::new("BID-webauthn".to_owned());
        browser_context.set_active_target_id("TID-webauthn".to_owned());
        browser_context.attach_active_session("SID-1".to_owned());
        ctx.conn
            .install_browser_context_fixture_for_test(browser_context);

        ctx.process_async(json!({
            "id": 1,
            "method": "WebAuthn.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 2,
            "method": "WebAuthn.disable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(2, json!({}), Some("SID-1"));
    }
}
