use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

mod handshake;
use handshake::browser_client_handshake;

use crate::{
    ConnectOptions,
    handshake::HandshakeResponse,
    proxy::{
        connect_websocket_via_http_proxy_tunnel, websocket_proxy_url, websocket_target_authority,
    },
    tls::wrap_websocket_stream,
};

pub(crate) async fn open_websocket_stream(
    request: http::Request<()>,
    context: &ConnectOptions,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        HandshakeResponse,
    ),
    String,
> {
    let proxy_url = websocket_proxy_url(request.uri(), context)?;
    let tcp_stream = match proxy_url {
        Some(proxy_url) => {
            connect_websocket_via_http_proxy_tunnel(request.uri(), &proxy_url, context).await?
        }
        None => {
            let authority = websocket_target_authority(request.uri())?;
            TcpStream::connect(authority)
                .await
                .map_err(|error| format!("failed to connect WebSocket server: {error}"))?
        }
    };
    let stream = wrap_websocket_stream(request.uri(), tcp_stream, context).await?;
    browser_client_handshake(request, stream).await
}
