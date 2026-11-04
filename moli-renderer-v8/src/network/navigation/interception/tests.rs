use super::*;
use crate::network::{RendererResourceTaskRunner, ResourceRequestClient};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn request_head(socket: &mut tokio::net::TcpStream) -> String {
    let mut head = Vec::new();
    let mut bytes = [0u8; 4096];
    while !head.ends_with(b"\r\n\r\n") {
        let length = socket.read(&mut bytes).await.unwrap();
        assert!(length > 0, "connection closed before request headers");
        head.extend_from_slice(&bytes[..length]);
    }
    String::from_utf8(head).unwrap()
}

#[tokio::test]
async fn redirect_body_cancellation_does_not_cancel_the_next_navigation_hop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        assert!(request_head(&mut first).await.starts_with("GET /redirect "));
        first
            .write_all(b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 1000000\r\n\r\n")
            .await
            .unwrap();
        let (mut second, _) = listener.accept().await.unwrap();
        assert!(request_head(&mut second).await.starts_with("GET /final "));
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let request = Request::get(&format!("http://{addr}/redirect")).unwrap();
    let origin = moli_url::WebOrigin::from_url(&request.url);
    let request = request.with_request_origin(origin);
    let loader = NavigationResourceLoader::new(
        owner.handle(),
        request.url.clone(),
        RendererResourceTaskRunner::from_current_tokio().unwrap(),
    );
    let mut navigation = NavigationInterception::new(loader.clone(), request);
    navigation.set_manual_redirects(true);
    let response = navigation.fetch_raw_stream().await.unwrap();
    let redirect = navigation
        .follow_redirect(&response.response().head())
        .unwrap()
        .unwrap();
    assert_eq!(redirect.to_url.path(), "/final");
    drop(response);
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), navigation.fetch_raw())
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_body();
    assert_eq!(body.try_into_materialized_bytes().unwrap(), b"ok");
    assert_eq!(
        loader.state(),
        super::super::NavigationResourceLoaderState::ResponseReady
    );
    server.await.unwrap();
}

#[tokio::test]
async fn dropping_a_request_future_cancels_transport_while_its_navigation_owner_survives() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (received, request_received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0u8; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        received.send(()).unwrap();
        let read =
            tokio::time::timeout(std::time::Duration::from_secs(10), socket.read(&mut bytes))
                .await
                .unwrap();
        assert!(
            matches!(read, Ok(0)) || read.is_err(),
            "cancelled navigation must close the transport"
        );
    });
    let owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let request = Request::get(&format!("http://{addr}/pending")).unwrap();
    let origin = moli_url::WebOrigin::from_url(&request.url);
    let request = request.with_request_origin(origin);
    let loader = NavigationResourceLoader::new(
        owner.handle(),
        request.url.clone(),
        RendererResourceTaskRunner::from_current_tokio().unwrap(),
    );
    let navigation = NavigationInterception::new(loader.clone(), request);
    let task = tokio::spawn(async move { navigation.fetch_raw_stream().await });
    request_received.await.unwrap();
    task.abort();
    assert!(task.await.is_err_and(|error| error.is_cancelled()));
    assert_eq!(
        loader.state(),
        super::super::NavigationResourceLoaderState::Cancelled
    );
    server.await.unwrap();
}

#[tokio::test]
async fn buffered_auth_redirect_does_not_wait_for_its_body_or_cancel_the_next_hop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        assert!(request_head(&mut first).await.starts_with("GET /redirect "));
        first.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"navigation\", nonce=\"deadbeef\", qop=\"auth\", algorithm=MD5\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        let authenticated = request_head(&mut first).await;
        assert!(authenticated.starts_with("GET /redirect "));
        assert!(
            authenticated
                .to_ascii_lowercase()
                .contains("authorization: digest ")
        );
        first
            .write_all(b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 1000000\r\n\r\n")
            .await
            .unwrap();
        // Keep this response body open until the navigation reaches its next hop.
        let (mut second, _) = listener.accept().await.unwrap();
        assert!(request_head(&mut second).await.starts_with("GET /final "));
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let request = Request::get(&format!("http://{addr}/redirect")).unwrap();
    let origin = moli_url::WebOrigin::from_url(&request.url);
    let request = request
        .with_request_origin(origin)
        .with_auth(moli_fetch::RequestAuth {
            target: moli_fetch::RequestAuthTarget::Server,
            scheme: moli_fetch::RequestAuthScheme::Digest,
            username: "user".to_owned(),
            password: "password".to_owned(),
        });
    let loader = NavigationResourceLoader::new(
        owner.handle(),
        request.url.clone(),
        RendererResourceTaskRunner::from_current_tokio().unwrap(),
    );
    let mut navigation = NavigationInterception::new(loader.clone(), request);
    navigation.set_manual_redirects(true);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        navigation.fetch_buffered(),
    )
    .await
    .expect("an authenticated redirect must complete at its headers")
    .unwrap();
    assert_eq!(response.response().status, 302);
    let redirect = navigation
        .follow_redirect(&response.response().head())
        .unwrap()
        .unwrap();
    assert_eq!(redirect.to_url.path(), "/final");
    drop(response);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        navigation.fetch_buffered(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.response().body_bytes(), b"ok");
    assert_eq!(
        loader.state(),
        super::super::NavigationResourceLoaderState::ResponseReady
    );
    server.await.unwrap();
}
