use super::super::JsContextHost;
use crate::types::{
    ScriptNetworkOutputItem, SubresourceNetworkRecord, SubresourceResourceType,
    WebSocketFrameDirection, WebSocketFrameOpcode, WebSocketLifecycleEvent, WebSocketNetworkEvent,
};
use http::HeaderName;
use moli_cookie_jar::StoredCookieSetReport;
use moli_websocket::websocket_cookie_url;

impl JsContextHost {
    pub(crate) fn record_websocket_open(
        &mut self,
        socket_id: u64,
        request_headers: moli_fetch::RequestHeaders,
        response_status: u16,
        response_headers: moli_fetch::ResponseHeaders,
    ) {
        let response_headers = response_headers.into_iter().collect::<Vec<_>>();
        let cookie_set_reports =
            self.store_websocket_response_cookies(socket_id, &response_headers);
        let Some(state) = self.websockets.get_mut(&socket_id) else {
            return;
        };
        let document_url = state.document_url.clone();
        let frame_id = state.frame_id.clone();
        let url = state.url.clone();
        state.opened = true;
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketLifecycleEvent(
            WebSocketLifecycleEvent::open(socket_id, document_url.clone(), url.clone()),
        ));
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            SubresourceNetworkRecord::success(
                frame_id,
                document_url,
                url.clone(),
                "GET".to_owned(),
                request_headers,
                None,
                SubresourceResourceType::WebSocket,
                None,
                Default::default(),
                url,
                response_status,
                response_headers,
                String::new(),
                cookie_set_reports,
            )
            .with_websocket_socket_id(socket_id),
        )));
        self.note_subresource_activity();
    }

    pub(crate) fn record_websocket_failure(&mut self, socket_id: u64, error_text: String) {
        let Some(state) = self.websockets.get(&socket_id) else {
            return;
        };
        let document_url = state.document_url.clone();
        let frame_id = state.frame_id.clone();
        let url = state.url.clone();
        let opened = state.opened;
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketLifecycleEvent(
            WebSocketLifecycleEvent::error(
                socket_id,
                document_url.clone(),
                url.clone(),
                error_text.clone(),
            ),
        ));
        if opened {
            self.note_subresource_activity();
            return;
        }
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            SubresourceNetworkRecord::failure(
                frame_id,
                document_url,
                url,
                "GET".to_owned(),
                Default::default(),
                None,
                SubresourceResourceType::WebSocket,
                error_text,
            )
            .with_websocket_socket_id(socket_id),
        )));
        self.note_subresource_activity();
    }

    pub(crate) fn record_websocket_closing(&mut self, socket_id: u64) {
        let Some(state) = self.websockets.get(&socket_id) else {
            return;
        };
        let document_url = state.document_url.clone();
        let url = state.url.clone();
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketLifecycleEvent(
            WebSocketLifecycleEvent::closing(socket_id, document_url, url),
        ));
        self.note_subresource_activity();
    }

    pub(crate) fn record_websocket_close(
        &mut self,
        socket_id: u64,
        code: u16,
        reason: String,
        was_clean: bool,
    ) {
        let Some(state) = self.websockets.get(&socket_id) else {
            return;
        };
        let document_url = state.document_url.clone();
        let url = state.url.clone();
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketLifecycleEvent(
            WebSocketLifecycleEvent::close(socket_id, document_url, url, code, reason, was_clean),
        ));
        self.note_subresource_activity();
    }

    pub(crate) fn record_websocket_frame(
        &mut self,
        socket_id: u64,
        direction: WebSocketFrameDirection,
        opcode: WebSocketFrameOpcode,
        payload_length: usize,
    ) {
        let Some(state) = self.websockets.get(&socket_id) else {
            return;
        };
        let document_url = state.document_url.clone();
        let url = state.url.clone();
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketNetworkEvent(
            WebSocketNetworkEvent::new(
                socket_id,
                document_url,
                url,
                direction,
                opcode,
                payload_length,
            ),
        ));
        self.note_subresource_activity();
    }

    pub(crate) fn record_websocket_network_event(&mut self, event: WebSocketNetworkEvent) {
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketNetworkEvent(event));
        self.note_subresource_activity();
    }

    pub(crate) fn record_websocket_lifecycle_event(&mut self, event: WebSocketLifecycleEvent) {
        self.push_network_output_item(ScriptNetworkOutputItem::WebSocketLifecycleEvent(event));
        self.note_subresource_activity();
    }

    fn store_websocket_response_cookies(
        &self,
        socket_id: u64,
        response_headers: &[(String, Vec<u8>)],
    ) -> Vec<StoredCookieSetReport> {
        if !response_headers
            .iter()
            .any(|(name, _)| response_header_name_is(name, &HeaderName::from_static("set-cookie")))
        {
            return Vec::new();
        }
        let Some(state) = self.websockets.get(&socket_id) else {
            return Vec::new();
        };
        let Some(loader) = state.resource_loader.as_ref() else {
            return Vec::new();
        };
        let response_cookie_url = websocket_cookie_url(&state.url);
        let cookie_store = loader.request_client().cookie_store();
        let mut store = cookie_store.lock();
        store.store_response_headers_with_reports(&response_cookie_url, response_headers)
    }
}

fn response_header_name_is(candidate: &str, expected: &HeaderName) -> bool {
    HeaderName::from_bytes(candidate.as_bytes()).is_ok_and(|candidate| candidate == *expected)
}
