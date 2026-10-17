use super::*;

#[tokio::test]
async fn buffered_response_commits_completion_before_delivery() -> Result<()> {
    let responses = [
        ScriptedResponse::ok("body"),
        ScriptedResponse::status(204, "No Content"),
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "/final")
            .with_body("redirect body"),
    ];
    let server = ScriptedHttpServer::spawn(responses.iter().chain(&responses).cloned().collect());
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    for use_callback in [false, true] {
        for (status, body) in [(200, "body"), (204, ""), (302, "redirect body")] {
            let parent = FetchCancelHandle::new();
            let child = parent.child_for_subrequest();
            let request = Request::get(&server.url())?.with_single_hop();
            let response = if use_callback {
                let (tx, rx) = oneshot::channel();
                let observer = child.clone();
                client.fetch_with_cancel_callback(request, child.clone(), move |result| {
                    // The token must be committed before invoking consumers.
                    assert!(observer.response_completion_is_committed());
                    let _ = tx.send(result);
                })?;
                rx.await??
            } else {
                client.fetch_with_cancel(request, child.clone()).await?
            };
            parent.adopt_response_progress(&child);
            assert_eq!(response.status, status);
            assert_eq!(response.body_text(), body);
            assert!(parent.response_completion_is_committed());
            assert!(!parent.cancel_if_response_pending());
            assert!(!parent.is_cancelled());
        }
    }
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn buffered_redirect_body_wait_is_cancellable_without_committing_completion() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/redirect", listener.local_addr()?);
    let (headers_tx, headers_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            anyhow::ensure!(stream.read(&mut byte).await? != 0, "incomplete request");
            request.push(byte[0]);
        }
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 302 Found\r\n",
                    "Location: /final\r\n",
                    "Content-Length: 100\r\n",
                    "Connection: close\r\n\r\n",
                )
                .as_bytes(),
            )
            .await?;
        let _ = headers_tx.send(());
        assert_eq!(stream.read(&mut byte).await?, 0);
        Ok::<_, anyhow::Error>(())
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let (tx, mut rx) = oneshot::channel();
    client.fetch_with_cancel_callback(
        Request::get(&url)?.with_single_hop(),
        cancel.clone(),
        move |result| {
            let _ = tx.send(result);
        },
    )?;
    tokio::time::timeout(Duration::from_secs(5), headers_rx).await??;
    // Known compatibility limitation: the buffered transport retains redirect
    // bodies, so headers alone cannot deliver its materialized response. Keep
    // cancellation working until that fallback can retire bodies at headers.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut rx)
            .await
            .is_err()
    );
    assert!(!cancel.response_completion_is_committed());
    assert!(cancel.cancel_if_response_pending());
    let error = tokio::time::timeout(Duration::from_secs(5), rx)
        .await??
        .unwrap_err();
    assert!(error.is::<crate::FetchCancelled>());
    assert!(cancel.is_cancelled());
    assert!(!cancel.response_completion_is_committed());
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}
