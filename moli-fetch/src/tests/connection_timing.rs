use super::*;
use std::io::Write;

#[tokio::test]
async fn discarding_completed_response_preserves_connection_and_cancellation_authority()
-> Result<()> {
    for (status, head) in [
        (200, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n"),
        (204, "HTTP/1.1 204 No Content\r\n"),
        (
            302,
            "HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\n",
        ),
        (304, "HTTP/1.1 304 Not Modified\r\nContent-Length: 123\r\n"),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/", listener.local_addr()?);
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut socket, _) = listener.accept()?;
            socket.set_read_timeout(Some(Duration::from_secs(5)))?;
            for second in [false, true] {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte)?;
                    request.extend_from_slice(&byte);
                }
                if second {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nCache-Control: no-store\r\nConnection: close\r\n\r\ndone")?;
                } else {
                    write!(
                        socket,
                        "{head}Cache-Control: no-store\r\nConnection: keep-alive\r\n\r\n"
                    )?;
                }
            }
            Ok(())
        });
        let mut config = FetchConfig::default();
        config.set_http_proxy(Some(String::new()));
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let first = FetchCancelHandle::new();
        let response = client
            .fetch_raw_stream_with_cancel(
                Request::get(&url)?.with_redirect_handling(RequestRedirectHandling::FollowByCaller),
                first.clone(),
            )
            .await?;
        assert_eq!(response.status, status);
        assert!(first.response_completion_is_committed());
        let connection = response
            .response_timing()
            .connection_timing()
            .unwrap()
            .connection;
        assert!(connection.is_some());
        tokio::time::timeout(Duration::from_secs(5), response.discard_unused()).await?;
        assert!(
            !first.is_cancelled(),
            "discarding completed IO is not a cancellation request"
        );
        let second = FetchCancelHandle::new();
        let response = client
            .fetch_with_cancel(Request::get(&url)?, second.clone())
            .await?;
        assert_eq!(response.body_text(), "done");
        let reused = second.response_timing().connection_timing().unwrap();
        assert_eq!(reused.connection, connection);
        assert_eq!(reused.curl.unwrap().new_connections, Some(0));
        server.join().unwrap()?;
    }
    Ok(())
}

#[tokio::test]
async fn native_connection_timing_covers_all_shared_dns_transports() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("timed"); 3]);
    let port = Url::parse(&server.url())?.port().context("server port")?;
    for transport in ["buffered", "html", "raw"] {
        let mut config = FetchConfig::default();
        config.set_http_proxy(Some(String::new()));
        let owner = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());
        let handle = owner.handle();
        let cancel = FetchCancelHandle::new();
        let request = Request::get(&format!("http://localhost:{port}/{transport}"))?;
        let before = Instant::now();
        let timing = match transport {
            "buffered" => {
                let response = handle
                    .submit_with_cancel(request, cancel.clone())?
                    .await??;
                assert_eq!(response.body_text(), "timed");
                cancel.response_timing()
            }
            "html" => {
                let mut response = handle.submit_html_stream(request)?.into_response().await?;
                let mut body = String::new();
                while let Some(chunk) = response.next_chunk().await {
                    body.push_str(&chunk);
                }
                response.finish().await?;
                assert_eq!(body, "timed");
                response.response_timing()
            }
            _ => {
                let mut response = handle
                    .submit_raw_stream(request, cancel.clone())?
                    .into_response()
                    .await?;
                let mut body = Vec::new();
                while let Some(chunk) = response.next_chunk().await {
                    body.extend_from_slice(&chunk);
                }
                response.finish().await?;
                assert_eq!(body, b"timed");
                response.response_timing()
            }
        };
        let after = Instant::now();
        let connection = timing
            .connection_timing()
            .context("native connection observations")?;
        let dns = connection.shared_dns.context("shared DNS observations")?;
        let prepared = connection
            .request_prepared_at
            .context("request preparation")?;
        assert!(before <= dns.started_at);
        assert!(dns.started_at <= dns.completed_at);
        assert!(dns.completed_at <= prepared);
        assert!(prepared <= timing.response_started_at().unwrap());
        assert!(timing.response_end_at().unwrap() <= after);
        assert!(connection.tls_context_started_at.is_none());
        let selected = connection
            .connection
            .context("selected physical connection")?;
        assert_eq!(selected.domain_lookup, Some(dns));
        assert!(dns.completed_at <= selected.connect_started_at);
        assert!(selected.connect_started_at <= selected.connect_completed_at);
        assert!(selected.connect_completed_at <= prepared);
        assert!(selected.secure_started_at.is_none());
        let curl = connection.curl.context("terminal curl statistics")?;
        assert_eq!(curl.new_connections, Some(1));
        assert_eq!(curl.app_connect, Some(Duration::ZERO));
        assert!(curl.name_lookup.unwrap() <= curl.connect.unwrap());
        assert!(curl.connect.unwrap() <= curl.pretransfer.unwrap());
        assert!(curl.pretransfer.unwrap() <= curl.total.unwrap());
    }
    assert_eq!(server.hits(), 3);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn native_connection_timing_does_not_leak_across_redirects_or_cache_hits() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("network"),
        ScriptedResponse::ok("cached").with_header("Cache-Control", "max-age=60"),
    ]);
    let redirect = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found").with_header("Location", &server.url()),
    ]);
    let mut source_url = Url::parse(&redirect.url())?;
    source_url.set_host(Some("localhost"))?;
    let mut config = FetchConfig::default();
    config.set_http_proxy(Some(String::new()));
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let first = client
        .fetch_with_cancel(Request::get(source_url.as_str())?, cancel.clone())
        .await?;
    assert_eq!(first.body_text(), "network");
    let connection = cancel
        .response_timing()
        .connection_timing()
        .context("final redirect attempt")?;
    assert!(connection.shared_dns.is_none());
    assert!(connection.curl.is_some());
    let url = format!("{}/cached", server.url());
    let response = client
        .fetch_with_cancel(Request::get(&url)?, cancel.clone())
        .await?;
    assert_eq!(response.body_text(), "cached");
    assert!(
        cancel
            .response_timing()
            .connection_timing()
            .unwrap()
            .curl
            .is_some()
    );
    let response = client
        .fetch_with_cancel(Request::get(&url)?, cancel.clone())
        .await?;
    assert_eq!(response.body_text(), "cached");
    assert_eq!(response.cache_state, crate::ResponseCacheState::Local);
    assert!(cancel.response_timing().connection_timing().is_none());
    assert_eq!(server.hits(), 2);
    assert_eq!(redirect.hits(), 1);
    drop(client);
    server.shutdown();
    redirect.shutdown();
    fs::remove_dir_all(cache_dir)?;
    Ok(())
}
