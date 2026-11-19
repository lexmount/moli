use super::*;
use moli_websocket::{
    spawn_connection_with_handshake_pause, spawn_standalone_connection_with_handshake_pause,
    spawn_synthetic_connection,
};

impl PageScriptHost {
    pub(crate) fn start_pending_websocket_connection(
        &mut self,
        pending: PendingWebSocketConnection,
        url: Url,
        headers: moli_fetch::RequestHeaders,
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
            let (connection, controller) = match pending.connector {
                Some(connector) => spawn_connection_with_handshake_pause(
                    connector,
                    pending.socket_id,
                    url.to_string(),
                    pending.protocols,
                    context,
                    event_sender,
                ),
                None => spawn_standalone_connection_with_handshake_pause(
                    pending.socket_id,
                    url.to_string(),
                    pending.protocols,
                    context,
                    event_sender,
                ),
            };
            (connection, Some(controller))
        } else {
            let connection = match pending.connector {
                Some(connector) => spawn_connection(
                    connector,
                    pending.socket_id,
                    url.to_string(),
                    pending.protocols,
                    context,
                    event_sender,
                ),
                None => spawn_standalone_connection(
                    pending.socket_id,
                    url.to_string(),
                    pending.protocols,
                    context,
                    event_sender,
                ),
            };
            (connection, None)
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
        request_headers: moli_fetch::RequestHeaders,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let event_sender = self.page_websocket_sender().event_sender();
        let Some(state) = self.websockets.get_mut(&pending.socket_id) else {
            return Err(format!("unknown pending WebSocket `{}`", pending.socket_id));
        };
        let (connection, peer) = spawn_synthetic_connection(
            pending.socket_id,
            request_headers,
            response_status,
            moli_fetch::ResponseHeaders::from_bytes(response_headers),
            event_sender,
        );
        state.url = request_url;
        state.connection = Some(connection);
        state.synthetic_peer = Some(peer);
        Ok(())
    }

    pub(crate) fn continue_websocket_handshake_response(
        &mut self,
        pending: PendingWebSocketResponseState,
        response_status: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
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
            .continue_open(
                response_status,
                response_headers.map(moli_fetch::ResponseHeaders::from_bytes),
            )
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
