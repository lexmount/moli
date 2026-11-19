use super::PageScriptHost;

impl PageScriptHost {
    pub(crate) fn receive_synthetic_websocket_text(&self, socket_id: u64, data: String) -> bool {
        self.websockets
            .get(&socket_id)
            .and_then(|state| state.synthetic_peer.as_ref())
            .map(|connection| connection.send_text(data).is_ok())
            .unwrap_or(false)
    }

    pub(crate) fn receive_synthetic_websocket_binary(&self, socket_id: u64, data: Vec<u8>) -> bool {
        self.websockets
            .get(&socket_id)
            .and_then(|state| state.synthetic_peer.as_ref())
            .map(|connection| connection.send_binary(data).is_ok())
            .unwrap_or(false)
    }

    pub(crate) fn close_synthetic_websocket_from_server(
        &self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> bool {
        self.websockets
            .get(&socket_id)
            .and_then(|state| state.synthetic_peer.as_ref())
            .map(|connection| connection.close(code, reason).is_ok())
            .unwrap_or(false)
    }
}
