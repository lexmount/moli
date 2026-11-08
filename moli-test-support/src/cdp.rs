use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, ensure};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// Discover the browser endpoint instead of assuming a server-specific path.
pub async fn discover_browser_websocket_url(addr: SocketAddr) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = TcpStream::connect(addr).await?;
        stream
            .write_all(
                format!("GET /json/version HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .context("discovery response has no HTTP headers")?;
        ensure!(
            headers.starts_with("HTTP/1.1 200 ") || headers.starts_with("HTTP/1.0 200 "),
            "discovery failed: {headers}"
        );
        let version: serde_json::Value = serde_json::from_str(body)?;
        let endpoint = version["webSocketDebuggerUrl"]
            .as_str()
            .filter(|value| !value.is_empty())
            .context("discovery response has no webSocketDebuggerUrl")?;
        Ok(endpoint.to_owned())
    })
    .await
    .context("browser endpoint discovery timed out")?
}
