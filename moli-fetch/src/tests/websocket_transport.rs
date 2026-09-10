use super::*;
use moli_curl::websocket::{CurlWebSocketEvent, CurlWebSocketRequest};

#[tokio::test]
async fn websocket_connector_follows_fetch_owner_lifetime() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}/pending", listener.local_addr()?);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let handle = client.handle();
    let connector = handle.websocket_connector();
    let mut connection = connector.connect(CurlWebSocketRequest::new(url.clone()))?;
    let (mut peer, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await??;
    tokio::time::timeout(Duration::from_secs(3), read_http_request_head(&mut peer)).await??;

    let other = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    drop(client);
    // Request-side handles and the connector do not keep the native owner alive.
    assert!(connector.connect(CurlWebSocketRequest::new(url)).is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), peer.read(&mut [0])).await??,
        0
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), connection.recv()).await?,
        Some(CurlWebSocketEvent::Closed { result: Err(_) })
    ));

    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("other runtime")]);
    let response = other.fetch(Request::get(&server.url())?).await?;
    assert_eq!(response.body_text(), "other runtime");
    server.shutdown();
    Ok(())
}
