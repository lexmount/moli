use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocketUpgrade},
    },
    http::{
        HeaderMap, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_TYPE, LOCATION,
            SET_COOKIE,
        },
    },
    response::IntoResponse,
    routing::{get, post},
};

use crate::testing::{
    TestContext, spawn_connection_drop_server, wait_until_messages,
    wait_until_renderer_document_load,
};
use crate::{
    conn::{
        BrowserContext, CapturedBody, CdpConnection, DocumentBodySource, NETWORK_ERROR_PAGE_URL,
        NavigationDispatchState, PageTargetHost, TargetIdentityState, TargetPageSlot,
    },
    domains::page::LOADER_ID,
};
use moli_core::{OptionalResourceFetchMask, runtime::NavigationEngine};
use moli_fetch::{FetchConfig, RawResponse, ResponseHead};
use parking_lot::Mutex;
use serde_json::json;
use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{Duration, sleep};
use url::Url;

fn consume_main_document_navigation_start(ctx: &mut TestContext) {
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
}
async fn flush_until_subresource_finished(
    ctx: &mut TestContext,
    resource_type: &str,
    expected_request_count: usize,
    description: &str,
) {
    wait_until_messages(ctx, Some("SID-1"), description, |messages| {
        let requests = messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!(resource_type)
            })
            .collect::<Vec<_>>();
        let Some(request_id) = requests
            .first()
            .and_then(|message| message["params"]["requestId"].as_str())
        else {
            return false;
        };
        requests.len() >= expected_request_count
            && messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
    })
    .await;
}
async fn flush_until_subresource_failed(
    ctx: &mut TestContext,
    resource_type: &str,
    description: &str,
) {
    wait_until_messages(ctx, Some("SID-1"), description, |messages| {
        let Some(request_id) = messages.iter().find_map(|message| {
            if message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!(resource_type)
            {
                message["params"]["requestId"].as_str()
            } else {
                None
            }
        }) else {
            return false;
        };
        messages.iter().any(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
    })
    .await;
}
async fn read_raw_http_request_path(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await.unwrap();
        assert_ne!(read, 0, "client closed before request headers");
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&request);
    request.split_whitespace().nth(1).unwrap_or("/").to_owned()
}
async fn websocket_echo_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                Message::Text(text) => {
                    let _ = socket.send(Message::Text(text)).await;
                }
                Message::Binary(bytes) => {
                    let _ = socket.send(Message::Binary(bytes)).await;
                }
                Message::Close(frame) => {
                    let _ = socket.send(Message::Close(frame)).await;
                    break;
                }
                Message::Ping(bytes) => {
                    let _ = socket.send(Message::Pong(bytes)).await;
                }
                Message::Pong(_) => {}
            }
        }
    })
}
async fn plain_page() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><html><body>ready</body></html>",
    )
}

mod blocked_urls;
mod cache;
mod cookies;
mod load_resource;
mod navigation;
mod response_body;
mod runtime;
mod service_worker;
mod session;
mod subresource;
