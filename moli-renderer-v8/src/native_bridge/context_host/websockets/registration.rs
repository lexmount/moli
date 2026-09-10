use super::super::{JsContextHost, security_policy::DocumentCspOutcome};
use super::WebSocketConnectionState;
use crate::document_runtime::document_content_security_policy_error_message;
use crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT;
use crate::types::{
    PendingSubresourceContinueEvent, PendingSubresourceFetchInfo, PendingSubresourceResponseInfo,
    PendingWebSocketConnection, PendingWebSocketResponseState, SubresourceResourceType,
    SubresourceResponseBody,
};
use moli_websocket::{
    ConnectOptions as WebSocketConnectOptions, spawn_connection,
    spawn_connection_with_handshake_pause, spawn_failed_connection, spawn_synthetic_connection,
    websocket_cookie_url,
};
use url::Url;

impl JsContextHost {
    pub(crate) fn register_websocket_for_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        wrapper: v8::Local<'_, v8::Object>,
        owner: super::super::WindowExecutionContextBinding,
        frame_id: Option<String>,
        document_url: Url,
        url: Url,
        protocols: Vec<String>,
        csp_outcome: DocumentCspOutcome,
    ) -> u64 {
        let socket_id = self.next_websocket_id;
        self.next_websocket_id += 1;
        let resource_loader =
            self.document_resource_loader_for_dispatch_scope(owner.dispatch_scope());
        let loader = resource_loader
            .as_ref()
            .map(crate::network::context::DocumentResourceLoader::request_client);
        let cookie_url = websocket_cookie_url(&url);
        let cookie_context = moli_cookie_jar::NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&cookie_url, &document_url);
        let cookie_header = if let Some(loader) = loader {
            moli_fetch::cookie_header_for_request(
                &loader.cookie_store(),
                &cookie_url,
                cookie_context,
            )
        } else {
            Ok(None)
        };
        let cookie_header_for_context = cookie_header
            .as_ref()
            .ok()
            .and_then(|header| header.clone());
        let context = WebSocketConnectOptions {
            connector: loader.map(|loader| loader.websocket_connector()),
            origin: moli_url::origin_ascii_serialization(&document_url),
            user_agent: loader
                .map(|loader| loader.user_agent().to_owned())
                .unwrap_or_else(|| moli_fetch::FetchConfig::DEFAULT_USER_AGENT.to_owned()),
            extra_headers: self.extra_http_headers.clone(),
            http_proxy: loader.and_then(|loader| loader.http_proxy().map(ToOwned::to_owned)),
            http_no_proxy: loader.and_then(|loader| loader.http_no_proxy().map(ToOwned::to_owned)),
            proxy_bearer_token: loader
                .and_then(|loader| loader.proxy_bearer_token().map(ToOwned::to_owned)),
            tls: loader
                .map(|loader| loader.tls_config().clone())
                .unwrap_or_default(),
            cookie_header: cookie_header_for_context,
        };
        let dispatch_scope = owner.dispatch_scope();
        let csp_failure_message = csp_outcome.into_blocking_violation().map(|violation| {
            document_content_security_policy_error_message(&violation, "WebSocket")
        });
        let (connection, fetch_internal_id) = if let Some(message) = csp_failure_message {
            let connection = spawn_failed_connection(
                socket_id,
                message,
                self.page_websocket_sender().event_sender(),
            );
            (Some(connection), None)
        } else if self.is_url_blocked(&url) {
            let connection = spawn_failed_connection(
                socket_id,
                BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
                self.page_websocket_sender().event_sender(),
            );
            (Some(connection), None)
        } else if cookie_header.is_ok()
            && self.should_intercept_subresource(SubresourceResourceType::WebSocket)
        {
            let request_headers = websocket_interception_request_headers(&context);
            let internal_id = self.record_pending_websocket_fetch(
                v8::Global::new(scope, scope.get_current_context()),
                dispatch_scope,
                PendingWebSocketConnection {
                    socket_id,
                    protocols,
                    connect_options: context,
                },
                PendingSubresourceFetchInfo {
                    internal_id: 0,
                    network_request_handle: None,
                    frame_id: frame_id.clone(),
                    document_url: document_url.clone(),
                    url: url.clone(),
                    websocket_socket_id: Some(socket_id),
                    method: "GET".to_owned(),
                    request_headers,
                    request_body: None,
                    request_body_bytes: None,
                    resource_type: SubresourceResourceType::WebSocket,
                    request_cookie_report: None,
                },
            );
            (None, Some(internal_id))
        } else {
            let connection = match cookie_header {
                Ok(_) => spawn_connection(
                    socket_id,
                    url.to_string(),
                    protocols,
                    context,
                    self.page_websocket_sender().event_sender(),
                ),
                Err(error) => spawn_failed_connection(
                    socket_id,
                    format!("failed to build WebSocket cookie header: {error}"),
                    self.page_websocket_sender().event_sender(),
                ),
            };
            (Some(connection), None)
        };
        self.websockets.insert(
            socket_id,
            WebSocketConnectionState {
                owner,
                resource_loader,
                wrapper: v8::Global::new(scope, wrapper),
                connection,
                url,
                frame_id,
                document_url,
                opened: false,
                synthetic_peer: None,
                handshake_controller: None,
                fetch_internal_id,
                response_interception_pending: None,
            },
        );
        socket_id
    }

    pub(crate) fn start_pending_websocket_connection(
        &mut self,
        pending: PendingWebSocketConnection,
        url: Url,
        headers: Vec<(String, String)>,
        headers_overridden: bool,
        intercept_response: bool,
    ) -> Result<(), String> {
        let event_sender = self.page_websocket_sender().event_sender();
        let Some(state) = self.websockets.get_mut(&pending.socket_id) else {
            return Err(format!("unknown pending WebSocket `{}`", pending.socket_id));
        };
        let mut context = pending.connect_options;
        context.extra_headers = headers;
        if headers_overridden {
            context.cookie_header = None;
        }
        let (connection, handshake_controller) = if intercept_response {
            let (connection, controller) = spawn_connection_with_handshake_pause(
                pending.socket_id,
                url.to_string(),
                pending.protocols,
                context,
                event_sender,
            );
            (connection, Some(controller))
        } else {
            (
                spawn_connection(
                    pending.socket_id,
                    url.to_string(),
                    pending.protocols,
                    context,
                    event_sender,
                ),
                None,
            )
        };
        state.handshake_controller = handshake_controller;
        state.url = url;
        state.connection = Some(connection);
        state.response_interception_pending =
            if intercept_response {
                Some(state.fetch_internal_id.ok_or_else(|| {
                    "WebSocket response interception missing request id".to_owned()
                })?)
            } else {
                None
            };
        Ok(())
    }

    pub(crate) fn fail_pending_websocket_connection(
        &mut self,
        pending: PendingWebSocketConnection,
        error_text: String,
    ) -> Result<(), String> {
        let event_sender = self.page_websocket_sender().event_sender();
        let Some(state) = self.websockets.get_mut(&pending.socket_id) else {
            return Err(format!("unknown pending WebSocket `{}`", pending.socket_id));
        };
        let connection = spawn_failed_connection(pending.socket_id, error_text, event_sender);
        state.connection = Some(connection);
        Ok(())
    }

    pub(crate) fn fulfill_pending_websocket_connection(
        &mut self,
        pending: PendingWebSocketConnection,
        request_url: Url,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
    ) -> Result<(), String> {
        let event_sender = self.page_websocket_sender().event_sender();
        let Some(state) = self.websockets.get_mut(&pending.socket_id) else {
            return Err(format!("unknown pending WebSocket `{}`", pending.socket_id));
        };
        let (connection, peer) = spawn_synthetic_connection(
            pending.socket_id,
            request_headers,
            response_status,
            response_headers,
            event_sender,
        );
        state.url = request_url;
        state.connection = Some(connection);
        state.synthetic_peer = Some(peer);
        Ok(())
    }

    pub(crate) fn pause_websocket_handshake_response(
        &mut self,
        socket_id: u64,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
    ) -> bool {
        let Some((internal_id, url)) = self.websockets.get_mut(&socket_id).and_then(|state| {
            state
                .response_interception_pending
                .take()
                .map(|internal_id| (internal_id, state.url.clone()))
        }) else {
            return false;
        };
        self.record_pending_websocket_response(PendingWebSocketResponseState {
            internal_id,
            socket_id,
        });
        self.record_pending_subresource_continue_event(
            PendingSubresourceContinueEvent::ResponsePaused(PendingSubresourceResponseInfo {
                internal_id,
                url: url.clone(),
                final_url: url,
                method: "GET".to_owned(),
                request_headers,
                request_body: None,
                resource_type: SubresourceResourceType::WebSocket,
                request_cookie_report: None,
                network_request_headers: None,
                response_status,
                response_headers,
                response_body: SubresourceResponseBody::from_bytes(Vec::new()),
                from_cache: false,
            }),
        );
        true
    }

    pub(crate) fn continue_websocket_handshake_response(
        &mut self,
        pending: PendingWebSocketResponseState,
        response_status: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        let Some(state) = self.websockets.get_mut(&pending.socket_id) else {
            return Err(format!("unknown pending WebSocket `{}`", pending.socket_id));
        };
        let Some(controller) = state.handshake_controller.take() else {
            return Err(format!(
                "pending WebSocket `{}` has no handshake controller",
                pending.socket_id
            ));
        };
        controller
            .continue_open(response_status, response_headers)
            .map_err(|error| format!("pending WebSocket `{}`: {error}", pending.socket_id))
    }

    pub(crate) fn fail_websocket_handshake_response(
        &mut self,
        pending: PendingWebSocketResponseState,
        error_text: String,
    ) -> Result<(), String> {
        let Some(state) = self.websockets.get_mut(&pending.socket_id) else {
            return Err(format!("unknown pending WebSocket `{}`", pending.socket_id));
        };
        let Some(controller) = state.handshake_controller.take() else {
            return Err(format!(
                "pending WebSocket `{}` has no handshake controller",
                pending.socket_id
            ));
        };
        controller
            .fail(error_text)
            .map_err(|error| format!("pending WebSocket `{}`: {error}", pending.socket_id))
    }
}

fn websocket_interception_request_headers(
    context: &WebSocketConnectOptions,
) -> Vec<(String, String)> {
    let mut headers = context.extra_headers.clone();
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("origin"))
    {
        headers.push(("Origin".to_owned(), context.origin.clone()));
    }
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        headers.push(("User-Agent".to_owned(), context.user_agent.clone()));
    }
    if let Some(cookie) = context.cookie_header.as_ref()
        && !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("cookie"))
    {
        headers.push(("Cookie".to_owned(), cookie.clone()));
    }
    headers
}
