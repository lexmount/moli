mod commands;
mod connection;
mod cookie;
mod events;
mod frames;
mod handle;
mod handshake;
mod headers;
mod limits;
mod protocol;
mod proxy;
mod request;
mod runtime;
mod session;
mod synthetic;
mod transport;
mod types;

pub use cookie::websocket_cookie_url;
pub use events::EventSender;
pub use handle::{CommandSendError, ConnectionHandle};
pub use protocol::{
    WebSocketCloseRequest, WebSocketCloseValidationError, WebSocketSubprotocolError,
    WebSocketUrlError, close_info_code_from_number, default_close_code_for_reason,
    is_valid_close_code, is_valid_close_reason, is_valid_subprotocol,
    normalize_websocket_close_info, normalize_websocket_url, validate_subprotocols,
    validate_websocket_close_request, websocket_url_is_potentially_trustworthy,
};
pub use runtime::{spawn_connection, spawn_failed_connection, spawn_synthetic_connection};
pub use types::{Command, CommandSender, ConnectOptions, Event, FrameOpcode};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod native_tests;
