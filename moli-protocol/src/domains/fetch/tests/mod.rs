use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::{
    FetchAuthChallenge, FetchRequestStage, FetchResourceTypeFilter, PendingFetchAuthNavigation,
    PendingFetchNavigation, emit_auth_required, encode_basic_auth, extract_auth_challenge,
    request_auth_for_challenge, response_headers_from_params, url_pattern_matches,
};
use crate::conn::{BrowserContext, NETWORK_ERROR_PAGE_URL, PageTargetHost};
use crate::domains::page::LOADER_ID;
use crate::testing::{
    TestContext, wait_until_frame_stopped_loading, wait_until_message, wait_until_messages,
    wait_until_scheduler_message,
};
use axum::{
    Router,
    extract::ws::{Message, WebSocketUpgrade},
    http::{
        HeaderMap, Method, StatusCode,
        header::{CONTENT_TYPE, PROXY_AUTHENTICATE, WWW_AUTHENTICATE},
    },
    response::IntoResponse,
    routing::{any, get},
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

fn attached_browser_context() -> BrowserContext {
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc
}

async fn with_loaded_http_document(
    ctx: &mut TestContext,
    url: &str,
    session_id: &str,
    target_id: &str,
) {
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id(target_id.to_owned());
    bc.attach_active_session(session_id.to_owned());
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(url, Some(session_id))
        .await;
    ctx.conn
        .runtime_session_owner_slot_mut(Some(session_id))
        .expect("Fetch fixture target")
        .enable_primary_network_events();
}

async fn with_loaded_http_background_document(
    ctx: &mut TestContext,
    url: &str,
    active_session_id: &str,
    active_target_id: &str,
    background_session_id: &str,
    background_target_id: &str,
) {
    let background = PageTargetHost::with_url(
        background_target_id.to_owned(),
        Some(background_session_id.to_owned()),
        url.to_owned(),
    );

    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id(active_target_id.to_owned());
    bc.attach_active_session(active_session_id.to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(url, Some(background_session_id))
        .await;
}

async fn loaded_page_html_for_test(ctx: &mut TestContext) -> String {
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|bc| bc.active_page_state_mut().runtime_slot.loaded_page_mut())
        .expect("loaded page");
    page.serialize_html_async()
        .await
        .expect("loaded page should serialize HTML")
}

fn take_response_by_id(ctx: &mut TestContext, id: u64) -> Value {
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .expect("expected a response with the requested id");
    ctx.sent.remove(pos)
}

fn network_request_announced_before_fetch_pause(
    ctx: &TestContext,
    paused: &Value,
    expected_network_type: Option<&str>,
) -> Value {
    let fetch_request_id = paused["params"]["requestId"]
        .as_str()
        .expect("Fetch pause request id");
    let network_request_id = paused["params"]["networkId"]
        .as_str()
        .expect("Fetch pause network id");
    let matching_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matching_requests.len(),
        1,
        "a Fetch pause must have exactly one preceding Network.requestWillBeSent"
    );
    let request = matching_requests[0];
    if let Some(expected_network_type) = expected_network_type {
        assert_eq!(request["params"]["type"], json!(expected_network_type));
    }
    for field in ["url", "method", "postData"] {
        if let Some(expected) = paused["params"]["request"].get(field) {
            assert_eq!(
                request["params"]["request"].get(field),
                Some(expected),
                "Network and Fetch request projections disagree on {field}"
            );
        }
    }

    let request_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .expect("Network request position");
    let pause_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("Fetch pause position");
    assert!(
        request_index < pause_index,
        "Chromium emits Network.requestWillBeSent before Fetch.requestPaused"
    );
    request.clone()
}

async fn wait_for_network_loading_finished(
    ctx: &mut TestContext,
    session_id: &str,
    request_id: &str,
    description: &str,
) {
    wait_until_message(ctx, session_id, description, |message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    })
    .await;
}

async fn enable_runtime_async(ctx: &mut TestContext, session_id: &str, id: u64) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(id, json!({}), Some(session_id));
    ctx.sent.clear();
}

async fn evaluate_until_value_async(
    ctx: &mut TestContext,
    session_id: &str,
    mut next_id: u64,
    expression: &str,
    expected: &Value,
    description: &str,
) -> Value {
    let mut last = None;
    for _ in 0..64 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        ctx.process_async(json!({
            "id": next_id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": { "expression": expression }
        }))
        .await;
        let response = take_response_by_id(ctx, next_id);
        if response["result"]["result"]["value"] == *expected {
            return response;
        }
        last = Some(response);
        next_id += 1;
    }
    panic!(
        "timed out waiting for {description}; last={last:?}; sent={:?}",
        ctx.sent
    );
}

fn consume_main_document_navigation_start(ctx: &mut TestContext) {
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
}

async fn take_main_document_request_pause(ctx: &mut TestContext) -> Value {
    wait_until_scheduler_message(ctx, "main-document Fetch.requestPaused", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
    })
    .await;
    consume_main_document_navigation_start(ctx);
    loop {
        let message = ctx.take_one();
        match message["method"].as_str() {
            Some("Fetch.requestPaused") => return message,
            Some("Network.requestWillBeSent")
            | Some("Network.requestWillBeSentExtraInfo")
            | Some("Network.responseReceivedExtraInfo") => {}
            other => {
                panic!("expected main-document Fetch.requestPaused, got {other:?}: {message:?}")
            }
        }
    }
}

fn take_main_document_response_pause(ctx: &mut TestContext) -> Value {
    loop {
        let message = ctx.take_one();
        match message["method"].as_str() {
            Some("Fetch.requestPaused") => return message,
            Some("Network.requestWillBeSent")
            | Some("Network.requestWillBeSentExtraInfo")
            | Some("Network.responseReceivedExtraInfo") => {}
            other => {
                panic!(
                    "expected response-stage main-document Fetch.requestPaused, got {other:?}: {message:?}"
                )
            }
        }
    }
}

async fn child_frame_id_for_single_iframe_async(ctx: &mut TestContext, id: u64) -> String {
    ctx.process_async(json!({"id": id, "method": "Page.getFrameTree"}))
        .await;
    take_response_by_id(ctx, id)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned()
}

async fn spawn_digest_proxy(
    success_content_type: &'static str,
    success_body: &'static str,
    challenge_realm: &'static str,
    challenge_nonce: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                let request = String::from_utf8_lossy(&buf[..read]);
                let has_digest_proxy_auth = request.lines().any(|line| {
                    let (name, value) = match line.split_once(':') {
                        Some(parts) => parts,
                        None => return false,
                    };
                    name.eq_ignore_ascii_case("proxy-authorization")
                        && value.trim_start().starts_with("Digest ")
                });

                let response = if has_digest_proxy_auth {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {success_content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{success_body}",
                        success_body.len()
                    )
                } else {
                    let body = "proxy digest auth required";
                    format!(
                        "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Digest realm=\"{challenge_realm}\", nonce=\"{challenge_nonce}\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };

                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), handle)
}

mod basics;
mod command_correlation;
mod navigation_auth;
mod navigation_control;
mod navigation_response_stage;
mod navigation_subresource;
mod runtime_auth_response;
mod runtime_fetch;
mod runtime_websocket;
mod runtime_xhr;
