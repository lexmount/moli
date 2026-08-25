use axum::{
    extract::{
        Path, State,
        ws::{WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use moli_protocol::{DevToolsTargetInfo, version};
use moli_protocol_cdp::CDP_PROTOCOL_JSON;
use percent_encoding::percent_decode_str;
use serde_json::json;

use super::{
    AppState, DEFAULT_BROWSER_ID, DEFAULT_TARGET_ID, DEFAULT_TARGET_URL,
    cdp_owner::SharedCdpOwnerRegistry,
    cdp_socket::{CdpFrontendSocketKind, run_cdp_frontend_socket},
};

pub(super) async fn json_version(State(state): State<AppState>) -> impl IntoResponse {
    axum::Json(json!({
        "Browser": version::PRODUCT,
        "Protocol-Version": version::PROTOCOL_VERSION,
        "User-Agent": state.fetch_config.user_agent(),
        "V8-Version": version::js_version(),
        "WebKit-Version": version::WEBKIT_VERSION,
        "webSocketDebuggerUrl": state.browser_ws_url,
    }))
}

pub(super) async fn json_list(State(state): State<AppState>) -> impl IntoResponse {
    let target_infos = state.cdp_agent_host_directory.page_target_infos();
    let targets = if target_infos.is_empty() {
        vec![target_json(&state, DEFAULT_TARGET_URL)]
    } else {
        target_infos
            .into_iter()
            .filter_map(|target_info| dynamic_target_json(&state, target_info))
            .collect()
    };
    axum::Json(json!(targets))
}

pub(super) async fn json_protocol() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        CDP_PROTOCOL_JSON,
    )
}

pub(super) async fn json_new_target(State(state): State<AppState>, uri: Uri) -> Response {
    let endpoint = match state.cdp_owner_registry.shared_owner() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Unable to create target owner: {error}"),
            )
                .into_response();
        }
    };
    let target_id = match endpoint
        .create_managed_target(requested_target_url(&uri))
        .await
    {
        Ok(target_id) => target_id,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Unable to create target: {error}"),
            )
                .into_response();
        }
    };
    let Some(route) = state.cdp_agent_host_directory.lookup_page(&target_id) else {
        rollback_unpublished_target(&endpoint, &target_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Created target was not published: {target_id}"),
        )
            .into_response();
    };
    let Some(target) = dynamic_target_json(&state, route.target_info) else {
        rollback_unpublished_target(&endpoint, &target_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Created target descriptor was invalid: {target_id}"),
        )
            .into_response();
    };
    axum::Json(target).into_response()
}

async fn rollback_unpublished_target(
    endpoint: &crate::cdp_frontend::CdpFrontendEndpoint,
    target_id: &str,
) {
    if let Err(error) = endpoint.close_target(target_id.to_owned()).await {
        tracing::warn!(
            target_id,
            ?error,
            "failed to roll back unpublished CDP target"
        );
    }
}

pub(super) async fn json_activate_target(
    Path(target_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let route = state.cdp_agent_host_directory.lookup_page(&target_id);
    if target_id == DEFAULT_TARGET_ID && route.is_none() {
        return (StatusCode::OK, "Target activated").into_response();
    }
    let Some(route) = route else {
        return no_such_target_response(&target_id);
    };
    match route.endpoint.activate_target(target_id.clone()).await {
        Ok(()) => (StatusCode::OK, "Target activated").into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to activate target {target_id}: {error}"),
        )
            .into_response(),
    }
}

pub(super) async fn json_close_target(
    Path(target_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if target_id == DEFAULT_TARGET_ID {
        return (StatusCode::OK, "Target is closing").into_response();
    }
    let Some(route) = state.cdp_agent_host_directory.lookup_page(&target_id) else {
        return no_such_target_response(&target_id);
    };
    match route.endpoint.close_target(target_id.clone()).await {
        Ok(()) => (StatusCode::OK, "Target is closing").into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to close target {target_id}: {error}"),
        )
            .into_response(),
    }
}

fn no_such_target_response(target_id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        format!("No such target id: {target_id}"),
    )
        .into_response()
}

fn target_json(state: &AppState, target_url: &str) -> serde_json::Value {
    json!({
        "description": "",
        "devtoolsFrontendUrl": &state.devtools_frontend_url,
        "id": DEFAULT_TARGET_ID,
        "title": target_url,
        "type": "page",
        "url": target_url,
        "webSocketDebuggerUrl": &state.page_ws_url,
    })
}

fn dynamic_target_json(
    state: &AppState,
    target_info: DevToolsTargetInfo,
) -> Option<serde_json::Value> {
    let target_id = target_info.target_id.as_ref()?.as_str();
    let page_ws_url_prefix = state.page_ws_url.strip_suffix(DEFAULT_TARGET_ID)?;
    let devtools_frontend_url_prefix = state
        .devtools_frontend_url
        .strip_suffix(DEFAULT_TARGET_ID)?;
    Some(json!({
        "description": "",
        "devtoolsFrontendUrl": format!("{devtools_frontend_url_prefix}{target_id}"),
        "id": target_id,
        "title": target_info.title,
        "type": "page",
        "url": target_info.url,
        "webSocketDebuggerUrl": format!("{page_ws_url_prefix}{target_id}"),
    }))
}

fn requested_target_url(uri: &Uri) -> String {
    let target = uri
        .query()
        .filter(|query| !query.is_empty())
        .and_then(|query| {
            query
                .split('&')
                .find_map(|part| part.strip_prefix("url="))
                .or_else(|| query.split('&').find(|part| !part.is_empty()))
        })
        .map(|target| percent_decode_str(target).decode_utf8_lossy().into_owned())
        .filter(|target| !target.is_empty());
    target
        .filter(|target| url::Url::parse(target).is_ok())
        .unwrap_or_else(|| DEFAULT_TARGET_URL.to_owned())
}

pub(super) async fn ws_browser_upgrade_handler(
    Path(browser_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if browser_id != DEFAULT_BROWSER_ID {
        return StatusCode::NOT_FOUND.into_response();
    }
    let owner_registry = state.cdp_owner_registry;
    ws.on_upgrade(move |socket| {
        run_shared_frontend_socket(socket, owner_registry, CdpFrontendSocketKind::Browser)
    })
}

pub(super) async fn ws_page_upgrade_handler(
    Path(target_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if target_id == DEFAULT_TARGET_ID {
        let owner_registry = state.cdp_owner_registry;
        return ws.on_upgrade(move |socket| {
            run_shared_frontend_socket(
                socket,
                owner_registry,
                CdpFrontendSocketKind::Page {
                    target_id: DEFAULT_TARGET_ID.to_owned(),
                },
            )
        });
    }
    let Some(route) = state.cdp_agent_host_directory.lookup_page(&target_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    page_ws_upgrade_response(ws, target_id, route.endpoint)
}

async fn run_shared_frontend_socket(
    socket: WebSocket,
    owner_registry: SharedCdpOwnerRegistry,
    kind: CdpFrontendSocketKind,
) {
    let endpoint = match owner_registry.shared_owner() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            tracing::warn!(?error, "failed to start shared CDP owner");
            return;
        }
    };
    run_cdp_frontend_socket(socket, endpoint, kind).await;
}

fn page_ws_upgrade_response(
    ws: WebSocketUpgrade,
    target_id: String,
    endpoint: crate::cdp_frontend::CdpFrontendEndpoint,
) -> Response {
    ws.on_upgrade(move |socket| async move {
        run_cdp_frontend_socket(socket, endpoint, CdpFrontendSocketKind::Page { target_id }).await;
    })
}
