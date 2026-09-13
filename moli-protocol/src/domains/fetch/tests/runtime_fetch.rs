use super::*;
use crate::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsAddNetworkInterceptCommand,
    DevToolsContinueInterceptedRequestCommand, DevToolsContinueInterceptedResponseCommand,
    DevToolsEvaluateScriptCommand, DevToolsNetworkInterceptId, DevToolsNetworkInterceptPattern,
    DevToolsNetworkInterceptPhase, DevToolsNetworkResourceType, DevToolsRequestId,
    DevToolsResultOwnership, DevToolsSessionId, DevToolsTargetId, FrontendProtocol,
};
use crate::testing::{
    drain_scheduler_events_like_scheduler,
    drain_scheduler_events_like_scheduler_preserving_internal_fields,
    protocol_events_into_internal_messages, wait_until_scheduler_message,
};

async fn wait_for_request_paused(ctx: &mut TestContext, url: &str, description: &str) -> Value {
    wait_for_request_paused_on_session(ctx, "SID-1", url, None, description).await
}

pub(super) async fn wait_for_request_paused_on_session(
    ctx: &mut TestContext,
    session_id: &str,
    url: &str,
    resource_type: Option<&str>,
    description: &str,
) -> Value {
    wait_until_messages(ctx, Some(session_id), description, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!(session_id)
                && message["params"]["request"]["url"] == json!(url)
                && resource_type
                    .is_none_or(|expected| message["params"]["resourceType"] == json!(expected))
        })
    })
    .await;
    ctx.take_first_matching("Fetch.requestPaused event", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["sessionId"] == json!(session_id)
            && message["params"]["request"]["url"] == json!(url)
            && resource_type
                .is_none_or(|expected| message["params"]["resourceType"] == json!(expected))
    })
}

async fn open_auto_attached_popup_from_session(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    url: &str,
) -> (String, String) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": format!("window.open('{url}', '_blank') !== null"),
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing popup open response in {messages:?}"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    let created = messages
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .unwrap_or_else(|| panic!("missing popup targetCreated in {messages:?}"));
    let target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let attached = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        })
        .unwrap_or_else(|| panic!("missing popup attachedToTarget in {messages:?}"));
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id")
        .to_owned();
    (target_id, popup_session_id)
}

async fn wait_for_auth_required(
    ctx: &mut TestContext,
    request_id: &str,
    description: &str,
) -> Value {
    wait_until_messages(ctx, Some("SID-1"), description, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Fetch.authRequired")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["requestId"] == json!(request_id)
        })
    })
    .await;
    ctx.take_first_matching("Fetch.authRequired event", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["requestId"] == json!(request_id)
    })
}

async fn wait_for_attached_fetch_request_paused(
    ctx: &mut TestContext,
    session_id: &str,
    url: &str,
    response_status: Option<u16>,
    description: &str,
) -> Value {
    wait_for_target_fetch_request_paused(ctx, Some(session_id), url, response_status, description)
        .await
}

async fn wait_for_target_fetch_request_paused(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    url: &str,
    response_status: Option<u16>,
    description: &str,
) -> Value {
    wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"] == json!(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!("XHR")
            && match response_status {
                Some(status) => message["params"]["responseStatusCode"] == json!(status),
                None => true,
            }
    })
    .await;
    ctx.take_first_matching(description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"] == json!(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!("XHR")
            && match response_status {
                Some(status) => message["params"]["responseStatusCode"] == json!(status),
                None => true,
            }
    })
}

async fn wait_for_background_request_paused(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    url: &str,
    resource_type: &str,
    description: &str,
) -> Value {
    wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"].as_str() == Some(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!(resource_type)
    })
    .await;
    ctx.take_first_matching(description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"].as_str() == Some(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!(resource_type)
    })
}

#[path = "runtime_fetch/runtime_subresources.rs"]
mod runtime_subresources;
#[path = "runtime_fetch/session_interception.rs"]
mod session_interception;
#[path = "runtime_fetch/worker_requests.rs"]
mod worker_requests;
