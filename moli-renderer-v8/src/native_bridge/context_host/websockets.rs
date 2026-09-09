mod commands;
mod events;
mod registration;

use moli_websocket::ConnectionHandle as WebSocketConnectionHandle;
use url::Url;

pub(super) struct WebSocketConnectionState {
    pub(super) owner: super::WindowExecutionContextBinding,
    pub(super) resource_loader: Option<crate::network::context::DocumentResourceLoader>,
    pub(super) wrapper: v8::Global<v8::Object>,
    pub(super) connection: Option<WebSocketConnectionHandle>,
    pub(super) url: Url,
    pub(super) frame_id: Option<String>,
    pub(super) document_url: Url,
    pub(super) opened: bool,
    pub(super) synthetic_peer: Option<moli_websocket::SyntheticPeer>,
    pub(super) handshake_controller: Option<moli_websocket::HandshakeController>,
    pub(super) fetch_internal_id: Option<u64>,
    pub(super) response_interception_pending: Option<u64>,
}
