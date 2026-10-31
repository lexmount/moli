use super::*;
use std::io::Write;

const EMPTY_GZIP: &[u8] = &[
    31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

fn compressed_response_server(
    gzip: &'static [u8],
    chunked: bool,
) -> Result<(String, thread::JoinHandle<std::io::Result<()>>)> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/compressed", listener.local_addr()?);
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            request.extend_from_slice(&byte);
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nCache-Control: max-age=600\r\nConnection: close\r\n"
        )?;
        if chunked {
            write!(
                stream,
                "Transfer-Encoding: chunked\r\n\r\n{:x}\r\n",
                gzip.len()
            )?;
            stream.write_all(gzip)?;
            stream.write_all(b"\r\n0\r\n\r\n")?;
        } else {
            write!(stream, "Content-Length: {}\r\n\r\n", gzip.len())?;
            stream.write_all(gzip)?;
        }
        Ok(())
    });
    Ok((url, server))
}

#[tokio::test]
async fn encoded_body_size_survives_compression_and_response_materialization() -> Result<()> {
    const TEXT_GZIP: &[u8] = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff\x4b\x49\x4d\xce\x4f\x49\x4d\x01\x00\xf6\x9a\xf0\x1a\x07\x00\x00\x00";
    for (gzip, decoded) in [(EMPTY_GZIP, ""), (TEXT_GZIP, "decoded")] {
        for chunked in [false, true] {
            for transport in ["buffered", "raw", "html"] {
                let (url, server) = compressed_response_server(gzip, chunked)?;
                let config = FetchConfig::default();
                let request = Request::get(&url)?;
                let response = if transport == "buffered" {
                    let owner = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());
                    let response = owner
                        .handle()
                        .submit_with_cancel(request, FetchCancelHandle::new())?
                        .await??;
                    assert!(owner.shutdown().is_clean());
                    response
                } else {
                    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
                    let response = if transport == "raw" {
                        client.fetch(request).await?
                    } else {
                        client
                            .fetch_html_stream(request)
                            .await?
                            .into_materialized_text_response()
                            .await?
                    };
                    assert!(client.shutdown().is_clean());
                    response
                };
                assert_eq!(
                    response.body_text(),
                    decoded,
                    "{transport}, chunked={chunked}"
                );
                assert_eq!(
                    response.timing_info.encoded_body_size(),
                    Some(gzip.len() as u64),
                    "{transport}, chunked={chunked}"
                );
                assert!(
                    response
                        .timing_info
                        .transport()
                        .response_started_at()
                        .is_some()
                );
                let raw = response.into_materialized_raw_response();
                assert_eq!(raw.timing_info.encoded_body_size(), Some(gzip.len() as u64));
                let text = raw.into_lossy_materialized_text_response();
                assert_eq!(
                    text.head().timing_info.encoded_body_size(),
                    Some(gzip.len() as u64)
                );
                server.join().expect("compressed response server")?;
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn disk_cache_retains_encoded_body_size_after_client_restart() -> Result<()> {
    for html in [false, true] {
        let root = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(root.display().to_string()));
        let (url, server) = compressed_response_server(EMPTY_GZIP, true)?;
        let mut original_start = None;
        for cached in [false, true] {
            let client = FetchClient::new(&config, new_shared_browser_cookie_store());
            let request = Request::get(&url)?;
            let response = if html {
                client
                    .fetch_html_stream(request)
                    .await?
                    .into_materialized_text_response()
                    .await?
            } else {
                client.fetch(request).await?
            };
            assert_eq!(response.from_cache, cached);
            assert_eq!(response.body_bytes().len(), 0);
            assert_eq!(
                response.timing_info.encoded_body_size(),
                Some(EMPTY_GZIP.len() as u64)
            );
            let start = response
                .timing_info
                .transport()
                .response_started_at()
                .unwrap();
            if cached {
                assert!(start > original_start.unwrap());
                assert!(
                    response
                        .timing_info
                        .transport()
                        .request_started_at()
                        .is_none()
                );
            } else {
                original_start = Some(start);
            }
            assert!(client.shutdown().is_clean());
        }
        server.join().expect("cache response server")?;
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[tokio::test]
async fn native_response_timing_distinguishes_encoded_empty_bodies_from_head_headers() -> Result<()>
{
    // A complete gzip member with an empty decoded payload.
    const GZIP: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    for method in ["GET", "HEAD"] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/empty", listener.local_addr()?);
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte)?;
                request.extend_from_slice(&byte);
            }
            assert!(request.starts_with(method.as_bytes()));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                GZIP.len()
            )?;
            if method == "GET" {
                stream.write_all(GZIP)?;
            }
            Ok(())
        });
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let cancel = FetchCancelHandle::new();
        let request = Request::new(method, &url, None, crate::RequestHeaders::default())?;
        let mut response = client
            .fetch_raw_stream_with_cancel(request, cancel.clone())
            .await?;
        while let Some(chunk) = response.next_chunk().await {
            assert!(chunk.is_empty(), "decoded payload must remain empty");
        }
        response.finish().await?;
        assert_eq!(
            cancel.response_timing().body_bytes_received(),
            method == "GET"
        );
        server.join().expect("server thread")?;
    }
    Ok(())
}

#[tokio::test]
async fn native_response_timing_separates_first_interim_and_final_headers() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/interim", listener.local_addr()?);
    let (release_final, ready) = std_mpsc::sync_channel(1);
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            request.extend_from_slice(&byte);
        }
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </a>;rel=preload\r\n\r\n")?;
        ready.recv().expect("consumer releases final headers");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody")?;
        Ok(())
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let fetch = client.fetch_raw_stream_with_cancel(Request::get(&url)?, cancel.clone());
    let release = async {
        let deadline = Instant::now() + Duration::from_secs(5);
        while cancel
            .response_timing()
            .first_interim_response_started_at()
            .is_none()
        {
            assert!(Instant::now() < deadline, "interim status was not observed");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(cancel.response_timing().response_started_at().is_none());
        release_final.send(()).unwrap();
    };
    let (response, ()) = tokio::join!(fetch, release);
    let mut response = response?;
    while response.next_chunk().await.is_some() {}
    response.finish().await?;
    let timing = cancel.response_timing();
    assert!(
        timing.request_started_at().unwrap() <= timing.first_interim_response_started_at().unwrap()
    );
    assert!(
        timing.first_interim_response_started_at().unwrap()
            <= timing.response_started_at().unwrap()
    );
    assert!(timing.response_started_at().unwrap() <= timing.response_end_at().unwrap());
    server.join().expect("server thread")?;
    Ok(())
}

#[tokio::test]
async fn raw_response_timing_is_native_even_when_the_consumer_does_not_poll() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/timed", listener.local_addr()?);
    let (release_body, ready) = std_mpsc::sync_channel(1);
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            request.extend_from_slice(&byte);
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")?;
        ready.recv().expect("consumer releases the body");
        stream.write_all(b"body")?;
        Ok(())
    });
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let mut response = client
        .fetch_raw_stream_with_cancel(Request::get(&url)?, cancel.clone())
        .await?;
    let headers = cancel.response_timing();
    assert!(headers.response_started_at().is_some());
    assert!(headers.response_end_at().is_none());
    release_body.send(())?;
    // Intentionally keep this Tokio thread occupied: the curl semantic owner
    // must capture completion without the response consumer being polled.
    let deadline = Instant::now() + Duration::from_secs(5);
    while cancel.response_timing().response_end_at().is_none() {
        assert!(
            Instant::now() < deadline,
            "native transfer did not complete"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let native = cancel.response_timing();
    let observed_at = Instant::now();
    assert_eq!(native.response_started_at(), headers.response_started_at());
    assert!(native.response_end_at().unwrap() >= native.response_started_at().unwrap());
    assert!(native.response_end_at().unwrap() <= observed_at);
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert_eq!(body, b"body");
    assert_eq!(cancel.response_timing(), native);
    server.join().expect("server thread")?;
    Ok(())
}

#[tokio::test]
async fn native_request_timing_is_transfer_scoped_on_multiplexed_http2() -> Result<()> {
    let server = ScriptedH2Server::spawn_with_response_barrier(
        vec![ScriptedResponse::ok("h2"); 2],
        Arc::new(tokio::sync::Barrier::new(2)),
    );
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_transport_connection_limits(Some(1), Some(1), Some(2));
    let owner = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());
    let handle = owner.handle();
    let buffered_cancel = FetchCancelHandle::new();
    let raw_cancel = FetchCancelHandle::new();
    let before = Instant::now();
    let buffered = handle.submit_with_cancel(
        Request::get(&server.url_path("/buffered"))?,
        buffered_cancel.clone(),
    )?;
    let raw =
        handle.submit_raw_stream(Request::get(&server.url_path("/raw"))?, raw_cancel.clone())?;
    let requests = async {
        let buffered = async {
            let response = buffered.await??;
            assert_eq!(
                response.negotiated_http_version,
                Some(NegotiatedHttpVersion::Http2)
            );
            assert_eq!(response.body_text(), "h2");
            Ok::<_, anyhow::Error>(())
        };
        let raw = async {
            let mut response = raw.into_response().await?;
            assert_eq!(
                response.negotiated_http_version,
                Some(NegotiatedHttpVersion::Http2)
            );
            let mut body = Vec::new();
            while let Some(chunk) = response.next_chunk().await {
                body.extend_from_slice(&chunk);
            }
            response.finish().await?;
            assert_eq!(body, b"h2");
            Ok::<_, anyhow::Error>(())
        };
        tokio::try_join!(buffered, raw)
    };
    tokio::time::timeout(Duration::from_secs(5), requests).await??;
    let after = Instant::now();
    for cancel in [buffered_cancel, raw_cancel] {
        let timing = cancel.response_timing();
        assert!(timing.request_started_at().unwrap() >= before);
        assert!(timing.request_started_at().unwrap() <= timing.response_started_at().unwrap());
        assert!(timing.response_started_at().unwrap() <= timing.response_end_at().unwrap());
        assert!(timing.response_end_at().unwrap() <= after);
    }
    // The barrier withholds all responses until both streams have arrived.
    // A single connection therefore proves concurrent multiplexing, not just
    // sequential reuse of an HTTP/2 connection.
    assert_eq!(server.connection_stream_counts(), [2]);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn native_request_timing_waits_for_the_proxy_tunnel() -> Result<()> {
    let server = ScriptedH2Server::spawn(vec![ScriptedResponse::ok("tunneled")]);
    let url = server.url_path("/timing");
    let origin = Url::parse(&url)?;
    let origin_port = origin.port().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let proxy_url = format!("http://{}", listener.local_addr()?);
    let (connect_seen_tx, connect_seen) = oneshot::channel();
    let (release_connect, release_connect_rx) = oneshot::channel();
    let proxy = thread::spawn(move || -> Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let listener = TcpListener::from_std(listener)?;
                let (mut downstream, _) = listener.accept().await?;
                let mut head = Vec::new();
                let mut byte = [0];
                while !head.ends_with(b"\r\n\r\n") {
                    downstream.read_exact(&mut byte).await?;
                    head.extend_from_slice(&byte);
                }
                assert!(head.starts_with(b"CONNECT "));
                connect_seen_tx.send(()).unwrap();
                release_connect_rx.await?;
                let mut upstream =
                    tokio::net::TcpStream::connect(("127.0.0.1", origin_port)).await?;
                downstream
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await?;
                // Dropping the client after completion closes the tunnel.
                let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
                Ok(())
            })
    });
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_http_proxy(Some(proxy_url));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let cancel = FetchCancelHandle::new();
    let fetch = client.fetch_raw_stream_with_cancel(Request::get(&url)?, cancel.clone());
    let release = async {
        tokio::time::timeout(Duration::from_secs(5), connect_seen).await??;
        assert!(cancel.response_timing().request_started_at().is_none());
        let released_at = Instant::now();
        release_connect.send(()).unwrap();
        Ok::<_, anyhow::Error>(released_at)
    };
    let (response, released_at) = tokio::join!(fetch, release);
    let mut response = response?;
    assert_eq!(
        response.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http2)
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert_eq!(body, b"tunneled");
    let timing = cancel.response_timing();
    assert!(timing.request_started_at().unwrap() >= released_at?);
    assert!(timing.request_started_at().unwrap() <= timing.response_started_at().unwrap());
    drop(response);
    drop(client);
    proxy.join().unwrap()?;
    server.shutdown();
    Ok(())
}
