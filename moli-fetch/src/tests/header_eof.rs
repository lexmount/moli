use super::*;

async fn serve_responses(listener: TcpListener, responses: Vec<Vec<u8>>) -> Result<Vec<String>> {
    let mut requests = Vec::new();
    for response in responses {
        let (mut stream, _) =
            tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
        let mut request = Vec::new();
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let count =
                tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await??;
            anyhow::ensure!(
                count != 0 && request.len() < 64 * 1024,
                "incomplete request"
            );
            request.extend_from_slice(&chunk[..count]);
        }
        requests.push(String::from_utf8(request)?);
        stream.write_all(&response).await?;
        stream.shutdown().await?;
    }
    Ok(requests)
}

async fn fetch_in_mode(
    client: &FetchClientHandle,
    request: Request,
    mode: &str,
) -> Result<(ResponseHead, Vec<u8>)> {
    let (head, body) = match mode {
        "buffered" => client
            .fetch(request.with_follow_redirects(false))
            .await?
            .into_body(),
        "html" => client.fetch_html_stream(request).await?.into_body(),
        "raw" => client
            .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
            .await?
            .into_body(),
        _ => unreachable!(),
    };
    Ok((head, body.into_materialized_bytes().await?))
}

#[tokio::test]
async fn header_eof_delivers_complete_fields_on_every_transport() -> Result<()> {
    for (method, response, status, headers) in [
        ("GET", b"HTTP/1.1 200 OK\r\nX-Test: first\r\nX-Test: second\r\nX-Bytes: \xff\r\n".as_slice(), 200,
            vec![("x-test", "first"), ("x-test", "second"), ("x-bytes", "\u{ff}")]),
        ("GET", b"HTTP/1.0 200 NANANA\nCONTENT-LENGTH:  0\ncontent-length:\t 0\n", 200,
            vec![("content-length", "0"), ("content-length", "0")]),
        ("GET", b"HTTP/1.1 280 HELLO\nfoo-test: 1\nfoo-test: 2\nfoo-test: 3\n", 280,
            vec![("foo-test", "1"), ("foo-test", "2"), ("foo-test", "3")]),
        ("GET", b"HTTP/1.1 200 OK\r\n", 200, vec![]),
        ("GET", b"HTTP/1.1 204 No Content\r\nX-Test: empty\r\n", 204,
            vec![("x-test", "empty")]),
        ("GET", b"HTTP/1.1 304 Not Modified\r\nX-Test: empty\r\n", 304,
            vec![("x-test", "empty")]),
        ("GET", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n", 200,
            vec![("transfer-encoding", "chunked")]),
        ("HEAD", b"HTTP/1.1 200 OK\r\nContent-Length: 40\r\nX-Test: head\r\n", 200,
            vec![("content-length", "40"), ("x-test", "head")]),
        ("GET", b"HTTP/1.1 103 Early Hints\r\nX-Interim: hidden\r\n\r\nHTTP/1.1 200 OK\r\nX-Test: final\r\n", 200,
            vec![("x-test", "final")]),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/eof", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(listener, vec![response.to_vec(); 3]));
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        for mode in ["buffered", "html", "raw"] {
            let (head, body) = fetch_in_mode(&client, Request::new(method, &url, None, vec![])?, mode)
                .await.with_context(|| format!("{mode}: {response:?}"))?;
            assert_eq!(head.status, status, "{mode}");
            assert_eq!(head.headers, headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect::<Vec<_>>(), "{mode}");
            assert!(body.is_empty(), "{mode}: {body:?}");
        }
        assert_eq!(server.await??.len(), 3);
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn header_eof_does_not_recover_transport_failures() -> Result<()> {
    for response in [
        b"".as_slice(),
        b"HTTP/1.1 103 Early Hints\r\nX-Interim: hidden\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 4\r\nX-Test: conflict\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nx",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nx",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/failure", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(listener, vec![response.to_vec(); 3]));
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        for mode in ["buffered", "html", "raw"] {
            // Subresource failures must not trigger the navigation HTTPS-upgrade retry.
            let request =
                Request::get(&url)?.with_browser_request_metadata(BrowserRequestMetadata::Fetch);
            assert!(
                fetch_in_mode(&client, request, mode).await.is_err(),
                "{mode}: {response:?}"
            );
        }
        assert_eq!(server.await??.len(), 3);
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn header_eof_processes_cookies_before_following_redirects() -> Result<()> {
    for mode in ["html", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/start", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(
            listener,
            vec![
                b"HTTP/1.1 302 Found\r\nLocation: /final\r\nSet-Cookie: redirect=one; Path=/\r\n"
                    .to_vec(),
                b"HTTP/1.1 200 OK\r\nSet-Cookie: final=two; Path=/\r\nX-Test: done\r\n".to_vec(),
            ],
        ));
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let (head, body) = fetch_in_mode(&client, Request::get(&url)?, mode).await?;
        assert_eq!(head.status, 200, "{mode}");
        assert_eq!(head.final_url.path(), "/final", "{mode}");
        assert!(head.redirected);
        assert_eq!(head.redirect_chain.len(), 1);
        assert_eq!(head.redirect_chain[0].cookie_set_reports.len(), 1);
        assert_eq!(head.cookie_set_reports.len(), 1);
        assert!(body.is_empty());
        let requests = server.await??;
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("GET /final "));
        assert!(
            requests[1].contains("Cookie: redirect=one\r\n"),
            "{}",
            requests[1]
        );
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn header_eof_keeps_manual_raw_redirects_unfollowed() -> Result<()> {
    for headers_terminated in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/start", listener.local_addr()?);
        let mut response =
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nSet-Cookie: redirect=one; Path=/\r\n"
                .to_vec();
        if headers_terminated {
            response.extend_from_slice(b"\r\n");
        }
        let server = tokio::spawn(serve_responses(listener, vec![response]));
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let (head, body) = fetch_in_mode(
            &client,
            Request::get(&url)?.with_follow_redirects(false),
            "raw",
        )
        .await?;
        assert_eq!(head.status, 302);
        assert_eq!(head.final_url.path(), "/start");
        assert!(!head.redirected);
        assert_eq!(
            head.cookie_set_reports.len(),
            1,
            "headers_terminated={headers_terminated}"
        );
        assert!(body.is_empty());
        assert_eq!(server.await??.len(), 1);
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn header_eof_creates_reusable_empty_cache_entries() -> Result<()> {
    for mode in ["html", "raw"] {
        let cache_dir = unique_test_cache_dir();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/cached", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(
            listener,
            vec![b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nX-Test: cached\r\n".to_vec()],
        ));
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        for from_cache in [false, true] {
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let (head, body) = fetch_in_mode(&client, Request::get(&url)?, mode).await?;
            assert_eq!(head.from_cache, from_cache, "{mode}");
            assert!(head.headers.contains(&("x-test".into(), "cached".into())));
            assert!(body.is_empty());
            assert!(client.shutdown().is_clean());
        }
        assert_eq!(server.await??.len(), 1);
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn header_eof_revalidates_cached_raw_bodies() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/revalidate", listener.local_addr()?);
    let server = tokio::spawn(serve_responses(listener, vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: no-cache\r\nETag: \"eof\"\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody".to_vec(),
        b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nETag: \"eof\"\r\nX-Test: updated\r\nSet-Cookie: revalidated=one; Path=/\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nX-Test: cookie-check\r\n".to_vec(),
    ]));
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let (first, body) = fetch_in_mode(&client, Request::get(&url)?, "raw").await?;
    assert!(!first.from_cache);
    assert_eq!(body, b"body");
    let (revalidated, body) = fetch_in_mode(&client, Request::get(&url)?, "raw").await?;
    assert_eq!(revalidated.status, 200);
    assert!(revalidated.from_cache);
    assert_eq!(body, b"body");
    assert!(
        revalidated
            .headers
            .contains(&("x-test".into(), "updated".into()))
    );
    let (_, body) = fetch_in_mode(
        &client,
        Request::get(&format!("{url}/cookie-check"))?,
        "raw",
    )
    .await?;
    assert!(body.is_empty());
    let requests = server.await??;
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("if-none-match: \"eof\"\r\n")
    );
    assert!(requests[2].contains("Cookie: revalidated=one\r\n"));
    assert!(client.shutdown().is_clean());
    fs::remove_dir_all(cache_dir)?;
    Ok(())
}

#[tokio::test]
async fn header_eof_applies_critical_client_hints_before_publishing() -> Result<()> {
    for mode in ["html", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/navigation", listener.local_addr()?);
        let server = tokio::spawn(serve_responses(listener, vec![
            b"HTTP/1.1 403 Challenge\r\nAccept-CH: Sec-CH-UA-Arch\r\nCritical-CH: Sec-CH-UA-Arch\r\nSet-Cookie: challenge=one; Path=/\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nX-Test: restarted\r\n".to_vec(),
        ]));
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let (head, body) = fetch_in_mode(&client, Request::get(&url)?, mode).await?;
        assert_eq!(head.status, 200, "{mode}");
        assert_eq!(head.redirect_chain.len(), 1);
        let restart = &head.redirect_chain[0];
        assert_eq!(restart.from_url, restart.to_url);
        assert_eq!(restart.response_extra_info.as_ref().unwrap().status, 403);
        assert_eq!(
            restart
                .response_extra_info
                .as_ref()
                .unwrap()
                .cookie_set_reports
                .len(),
            1
        );
        assert!(body.is_empty());
        let requests = server.await??;
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].to_ascii_lowercase().contains("sec-ch-ua-arch:"));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("sec-ch-ua-arch: \"x86\"")
        );
        assert!(requests[1].contains("Cookie: challenge=one\r\n"));
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn header_eof_preserves_the_response_size_limit() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/limited", listener.local_addr()?);
    let server = tokio::spawn(serve_responses(
        listener,
        vec![b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nX-Test: oversized\r\n".to_vec(); 3],
    ));
    let mut config = FetchConfig::default();
    config.set_connection_limits(None, None, Some(4));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    for mode in ["buffered", "html", "raw"] {
        let error = fetch_in_mode(&client, Request::get(&url)?, mode)
            .await
            .unwrap_err();
        if mode != "buffered" {
            assert!(
                error
                    .to_string()
                    .contains("response exceeded configured limit of 4 bytes"),
                "{mode}: {error:#}"
            );
        }
    }
    assert_eq!(server.await??.len(), 3);
    assert!(client.shutdown().is_clean());
    Ok(())
}

#[tokio::test]
async fn header_eof_does_not_publish_a_cancelled_response() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/cancel", listener.local_addr()?);
    let (headers_tx, headers_rx) = oneshot::channel();
    let (close_tx, close_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).await?;
            request.push(byte[0]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nX-Test: pending\r\n")
            .await?;
        let _ = headers_tx.send(());
        close_rx.await?;
        stream.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let request = Request::get(&url)?;
    let mut response = Box::pin(client.fetch_raw_stream_with_cancel(request, cancel.clone()));
    tokio::select! {
        result = &mut response => panic!("response published before headers ended: {result:?}"),
        result = headers_rx => result?,
    }
    cancel.cancel();
    let _ = close_tx.send(());
    assert!(
        tokio::time::timeout(Duration::from_secs(5), response)
            .await?
            .is_err()
    );
    server.await??;
    assert!(client.shutdown().is_clean());
    Ok(())
}
