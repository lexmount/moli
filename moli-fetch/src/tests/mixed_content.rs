use super::*;
use crate::RequestMixedContentPolicy;

#[tokio::test]
async fn mixed_content_is_rejected_before_each_native_transport_connects() -> Result<()> {
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let request = Request::get("http://mixed-content.invalid/resource")?
        .with_mixed_content_policy(RequestMixedContentPolicy::Block);
    for mode in 0..3 {
        let error = match mode {
            0 => {
                let (tx, rx) = oneshot::channel();
                client.fetch_with_cancel_callback(
                    request.clone(),
                    FetchCancelHandle::new(),
                    move |result| {
                        let _ = tx.send(result);
                    },
                )?;
                rx.await?.unwrap_err()
            }
            1 => client
                .fetch_html_stream(request.clone())
                .await
                .err()
                .unwrap(),
            _ => client.fetch_raw(request.clone()).await.unwrap_err(),
        };
        assert!(
            error.to_string().contains("Mixed content"),
            "{mode}: {error:#}"
        );
    }
    assert!(client.shutdown().is_clean());
    Ok(())
}

#[tokio::test]
async fn loopback_clients_keep_http_access_but_downgrade_redirects_are_blocked() -> Result<()> {
    for mode in 0..3 {
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("loopback"),
            ScriptedResponse::status(302, "Found")
                .with_header("Location", "http://mixed-content.invalid/redirected"),
        ]);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let request = Request::get(&server.url())?
            .with_mixed_content_policy(RequestMixedContentPolicy::Block);
        let first = client.fetch_raw(request.clone()).await?;
        assert_eq!(first.body_bytes(), b"loopback");
        let error = match mode {
            0 => {
                let (tx, rx) = oneshot::channel();
                client.fetch_with_cancel_callback(
                    request,
                    FetchCancelHandle::new(),
                    move |result| {
                        let _ = tx.send(result);
                    },
                )?;
                rx.await?.unwrap_err()
            }
            1 => client.fetch_html_stream(request).await.err().unwrap(),
            _ => client.fetch_raw(request).await.unwrap_err(),
        };
        assert!(
            error.to_string().contains("Mixed content"),
            "{mode}: {error:#}"
        );
        assert_eq!(server.hits(), 2);
        server.shutdown();
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}
