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

#[test]
fn websocket_connector_inherits_private_network_policy() {
    let mut config = FetchConfig::default();
    config.set_network_blocking(true, Vec::new());
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let error = client
        .handle()
        .websocket_connector()
        .connect(CurlWebSocketRequest::new(
            "ws://127.0.0.1/private".to_owned(),
        ))
        .expect_err("a WebSocket IP literal must use the fetch address policy");

    assert!(
        error
            .to_string()
            .contains("blocked private network address `127.0.0.1`")
    );
}

#[test]
fn websocket_connector_applies_blocked_cidr_to_fixed_direct_target() {
    let mut config = FetchConfig::default();
    config.set_network_blocking(false, vec!["198.18.0.0/15".parse().unwrap()]);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut request = CurlWebSocketRequest::new("ws://example.test/socket".to_owned());
    request.resolve_entries = vec!["example.test:80:198.18.0.1".to_owned()];

    let error = client
        .handle()
        .websocket_connector()
        .connect(request)
        .expect_err("a fixed WebSocket target in a blocked CIDR must be rejected");

    assert!(error.to_string().contains("matches `198.18.0.0/15`"));
}
