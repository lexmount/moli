use super::super::*;
use crate::testing::wait_until_messages;
use serde_json::{Value, json};

pub(super) async fn attached_smoke_session(
    ctx: &mut TestContext,
    base: u64,
) -> AttachedPageSession {
    create_attached_page_session_async(ctx, base, base + 1, base + 2, base + 3, base + 4).await
}

pub(super) async fn navigate_and_take_response(
    ctx: &mut TestContext,
    session_id: &str,
    id: u64,
    url: String,
) -> Value {
    ctx.process_and_wait_for_response_async(json!({
        "id": id,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    take_response_by_id(ctx, id)
}

pub(super) fn chromium_like_revision(value: &str) -> bool {
    value
        .strip_prefix('@')
        .is_some_and(|hash| !hash.is_empty() && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
}

pub(super) fn response_received_for_suffix<'a>(
    messages: &'a [Value],
    suffix: &str,
) -> Option<&'a Value> {
    messages.iter().find(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["response"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with(suffix))
    })
}

pub(super) fn request_will_be_sent_for_suffix<'a>(
    messages: &'a [Value],
    suffix: &str,
) -> Option<&'a Value> {
    messages.iter().find(|message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with(suffix))
    })
}

pub(super) async fn evaluate_return_by_value(
    ctx: &mut TestContext,
    session_id: &str,
    id: u64,
    expression: impl Into<String>,
) -> Value {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": expression.into(),
            "returnByValue": true
        }
    }))
    .await;
    take_response_by_id(ctx, id)
}

pub(super) async fn evaluate_string(
    ctx: &mut TestContext,
    session_id: &str,
    id: u64,
    expression: impl Into<String>,
) -> String {
    let response = evaluate_return_by_value(ctx, session_id, id, expression).await;
    response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return string: {response}"))
        .to_owned()
}

pub(super) struct CdpPageHarness {
    pub(super) target_id: String,
    pub(super) session_id: String,
}

impl CdpPageHarness {
    pub(super) async fn attach(ctx: &mut TestContext, base: u64) -> Self {
        let attached = attached_smoke_session(ctx, base).await;
        Self {
            target_id: attached.target_id,
            session_id: attached.session_id,
        }
    }

    pub(super) async fn command(
        &self,
        ctx: &mut TestContext,
        id: u64,
        method: &str,
        params: Value,
    ) -> Value {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": self.session_id,
            "params": params
        }))
        .await;
        take_response_by_id(ctx, id)
    }

    pub(super) async fn expect_empty_command(
        &self,
        ctx: &mut TestContext,
        id: u64,
        method: &str,
        params: Value,
    ) {
        let response = self.command(ctx, id, method, params).await;
        assert_eq!(
            response,
            json!({ "id": id, "result": {}, "sessionId": self.session_id })
        );
    }

    pub(super) async fn enable_page(&self, ctx: &mut TestContext, id: u64) {
        self.expect_empty_command(ctx, id, "Page.enable", json!({}))
            .await;
    }

    pub(super) async fn enable_inspector(&self, ctx: &mut TestContext, id: u64) {
        self.expect_empty_command(ctx, id, "Inspector.enable", json!({}))
            .await;
    }

    pub(super) async fn navigate(
        &self,
        ctx: &mut TestContext,
        id: u64,
        url: impl Into<String>,
    ) -> Value {
        navigate_and_take_response(ctx, &self.session_id, id, url.into()).await
    }

    pub(super) async fn evaluate_value(
        &self,
        ctx: &mut TestContext,
        id: u64,
        expression: impl Into<String>,
    ) -> Value {
        evaluate_return_by_value(ctx, &self.session_id, id, expression).await
    }

    pub(super) async fn evaluate_await_value(
        &self,
        ctx: &mut TestContext,
        id: u64,
        expression: impl Into<String>,
    ) -> Value {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": self.session_id,
            "params": {
                "expression": expression.into(),
                "awaitPromise": true,
                "returnByValue": true
            }
        }))
        .await;
        wait_until_messages(
            ctx,
            Some(self.session_id.as_str()),
            "Runtime.evaluate awaitPromise response",
            |messages| messages.iter().any(|message| message["id"] == json!(id)),
        )
        .await;
        take_response_by_id(ctx, id)
    }

    pub(super) async fn evaluate_string(
        &self,
        ctx: &mut TestContext,
        id: u64,
        expression: impl Into<String>,
    ) -> String {
        evaluate_string(ctx, &self.session_id, id, expression).await
    }
}
