use super::*;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn body(
    client: &FetchClientHandle,
    request: Request,
    transport: &str,
) -> Result<(ResponseHead, Vec<u8>)> {
    let (head, body) = match transport {
        "buffered" => client.fetch(request.with_single_hop()).await?.into_body(),
        "html" => client.fetch_html_stream(request).await?.into_body(),
        "raw" => client.fetch_raw(request).await?.into_body(),
        _ => unreachable!(),
    };
    Ok((head, body.into_materialized_bytes().await?))
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

async fn read_upload(stream: &mut tokio::net::TcpStream) -> Result<Option<(String, Vec<u8>)>> {
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(request.len() < 65536, "request headers exceed test limit");
        if stream.read(&mut byte).await? == 0 {
            anyhow::ensure!(request.is_empty(), "incomplete request headers");
            return Ok(None);
        }
        request.push(byte[0]);
    }
    let head = String::from_utf8(request)?;
    let length = header(&head, "Content-Length").unwrap_or("0").parse()?;
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(Some((head, bytes)))
}

#[tokio::test]
async fn misdirected_request_replays_bytes_on_a_fresh_connection_on_all_transports() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let attempts = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            for connection in 0..3 {
                let (mut stream, _) = listener.accept().await?;
                let barrier = Arc::clone(&barrier);
                let attempts = Arc::clone(&attempts);
                let recorded = Arc::clone(&recorded);
                connections.spawn(async move {
                    while let Some((head, bytes)) = read_upload(&mut stream).await? {
                        let warm = head.starts_with("GET /warm/");
                        recorded.lock().push((connection, head, bytes));
                        if warm {
                            // Leave two usable connections in the pool. A retry
                            // must open a third, not reuse the other idle one.
                            barrier.wait().await;
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nCache-Control: no-store\r\n\r\nwarm").await?;
                        } else if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            // There is deliberately no body. Retrying must not
                            // wait for this 421's advertised Content-Length.
                            stream.write_all(b"HTTP/1.1 421 Misdirected Request\r\nContent-Length: 99999\r\nSet-Cookie: misdirected=hidden; Path=/\r\nX-Stage: discarded\r\n\r\n").await?;
                        } else {
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nCache-Control: no-store\r\nX-Stage: final\r\nConnection: close\r\n\r\naccepted").await?;
                            break;
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                });
            }
            while let Some(result) = connections.join_next().await {
                result??;
            }
            Ok::<_, anyhow::Error>(())
        });
        let mut config = FetchConfig::default();
        config.set_request_timeout_ms(3000);
        config.set_connection_limits(None, None, Some(64));
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let (first, second) = tokio::join!(
            body(
                &client,
                Request::get(&format!("{origin}/warm/a"))?,
                transport
            ),
            body(
                &client,
                Request::get(&format!("{origin}/warm/b"))?,
                transport
            ),
        );
        assert_eq!(first?.1, b"warm");
        assert_eq!(second?.1, b"warm");
        let mut request = Request::get(&format!("{origin}/retry"))?;
        request.method = "POST".into();
        request.body = Some(bytes::Bytes::from_static(b"replay\0\xff"));
        let (response, bytes) = body(&client, request, transport).await?;
        assert_eq!(response.status, 200, "{transport}");
        assert_eq!(bytes, b"accepted");
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("x-stage"))
                .map(|(_, value)| value.as_slice())
                .collect::<Vec<_>>(),
            [b"final".as_slice()]
        );
        assert!(!response.redirected);
        assert!(response.redirect_chain.is_empty());
        assert!(response.cookie_set_reports.is_empty());
        assert!(client.shutdown().is_clean());
        tokio::time::timeout(Duration::from_secs(5), server).await???;
        let requests = requests.lock();
        assert_eq!(requests.len(), 4, "{transport}");
        let uploads = &requests[2..];
        assert!(
            uploads[0].0 < 2,
            "first attempt should reuse a warm connection"
        );
        assert_eq!(uploads[1].0, 2, "retry must open a fresh connection");
        for (_, head, bytes) in uploads {
            assert!(head.starts_with("POST /retry HTTP/1.1\r\n"));
            assert_eq!(bytes, b"replay\0\xff");
            assert_eq!(header(head, "Cookie"), None);
        }
    }
    Ok(())
}

#[tokio::test]
async fn misdirected_request_retries_once_but_never_replays_a_null_body_source() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        for null_source in [false, true] {
            let server = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(421, "Misdirected Request").with_body("first"),
                ScriptedResponse::status(421, "Misdirected Request").with_body("second"),
            ]);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let mut request = Request::get(&server.url())?.with_body_source_is_null(null_source);
            request.method = "POST".into();
            request.body = Some(bytes::Bytes::from_static(b"payload"));
            let (response, bytes) = body(&client, request, transport).await?;
            assert_eq!(
                response.status, 421,
                "{transport}, null source={null_source}"
            );
            assert_eq!(
                bytes,
                if null_source {
                    b"first".as_slice()
                } else {
                    b"second"
                }
            );
            assert_eq!(server.hits(), if null_source { 1 } else { 2 });
            assert!(!response.redirected);
            assert!(response.redirect_chain.is_empty());
            assert!(client.shutdown().is_clean());
            server.shutdown();
        }
    }
    Ok(())
}

#[tokio::test]
async fn misdirected_request_retry_retains_original_timeout_budget() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(421, "Misdirected Request").with_delay_ms(150),
            ScriptedResponse::ok("late").with_delay_ms(250),
        ]);
        let mut config = FetchConfig::default();
        config.set_request_timeout_ms(300);
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let error = body(&client, Request::get(&server.url())?, transport)
            .await
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<curl::Error>()
                .is_some_and(curl::Error::is_operation_timedout),
            "{transport}: {error:#}"
        );
        assert_eq!(server.hits(), 2);
        assert!(client.shutdown().is_clean());
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn misdirected_request_retry_remains_cancellable_before_final_headers() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/retry", listener.local_addr()?);
    let (second_tx, second_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        assert!(read_upload(&mut stream).await?.is_some());
        stream.write_all(b"HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        drop(stream);
        let (mut stream, _) = listener.accept().await?;
        assert!(read_upload(&mut stream).await?.is_some());
        second_tx.send(()).unwrap();
        release_rx.await?;
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await?, 0);
        Ok::<_, anyhow::Error>(())
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let fetch = client.fetch_raw_with_cancel(Request::get(&url)?, cancel.clone());
    let cancel_request = async {
        second_rx.await?;
        cancel.cancel();
        release_tx.send(()).unwrap();
        Ok::<_, anyhow::Error>(())
    };
    let (result, cancelled) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(fetch, cancel_request)
    })
    .await?;
    cancelled?;
    let error = result.unwrap_err();
    assert!(crate::is_fetch_cancelled(&error), "{error:#}");
    assert!(client.shutdown().is_clean());
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}

#[tokio::test]
async fn misdirected_request_retry_budget_resets_after_a_redirect() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(421, "Misdirected Request"),
        ScriptedResponse::status(302, "Found").with_header("Location", "/next"),
        ScriptedResponse::status(421, "Misdirected Request"),
        ScriptedResponse::ok("accepted"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let mut request = Request::get(&server.url())?;
    request.method = "POST".into();
    request.body = Some(bytes::Bytes::from_static(b"payload"));
    let response = client.fetch_raw(request).await?;
    assert_eq!(response.status, 200);
    assert_eq!(response.body_bytes(), b"accepted");
    assert!(response.redirected);
    assert_eq!(response.redirect_chain.len(), 1);
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("POST "));
    assert!(requests[1].starts_with("POST "));
    assert!(requests[2].starts_with("GET /next "));
    assert!(requests[3].starts_with("GET /next "));
    assert!(client.shutdown().is_clean());
    server.shutdown();
    Ok(())
}
