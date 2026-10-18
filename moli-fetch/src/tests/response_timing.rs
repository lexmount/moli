use super::*;
use std::io::Write;

#[tokio::test]
async fn native_response_timing_records_encoded_bytes_before_decoding_failures() -> Result<()> {
    for (coding, chunked, method, body, should_fail) in [
        ("gzip", false, "GET", b"abc".as_slice(), true),
        ("unrecognizedname", false, "GET", b"abc".as_slice(), true),
        ("gzip, deflate,Apple", false, "GET", b"abc".as_slice(), true),
        ("gzip", true, "GET", b"abc".as_slice(), true),
        ("unrecognizedname", true, "GET", b"abc".as_slice(), true),
        ("gzip, deflate,Apple", true, "GET", b"abc".as_slice(), true),
        ("unrecognizedname", true, "GET", b"".as_slice(), false),
        ("gzip", false, "HEAD", b"abc".as_slice(), false),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/encoded", listener.local_addr()?);
        let (release, ready) = std_mpsc::sync_channel(1);
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte)?;
                request.push(byte[0]);
            }
            let framing = if chunked {
                "Transfer-Encoding: chunked".to_owned()
            } else {
                format!("Content-Length: {}", body.len())
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Encoding: {coding}\r\n{framing}\r\nConnection: close\r\n\r\n"
            )?;
            ready
                .recv_timeout(Duration::from_secs(5))
                .expect("release encoded body");
            if method == "GET" {
                let mut encoded = Vec::new();
                if chunked {
                    if !body.is_empty() {
                        write!(encoded, "{:x}\r\n", body.len())?;
                        encoded.extend_from_slice(body);
                        encoded.extend_from_slice(b"\r\n");
                    }
                    encoded.extend_from_slice(b"0\r\nX-Trailer: present\r\n\r\n");
                } else {
                    encoded.extend_from_slice(body);
                }
                stream.write_all(&encoded)?;
            }
            Ok(())
        });
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let cancel = FetchCancelHandle::new();
        let request = Request::new(method, &url, None, crate::RequestHeaders::default())?;
        let mut response = client
            .fetch_raw_stream_with_cancel(request, cancel.clone())
            .await?;
        assert_eq!(response.status, 200);
        assert!(
            !cancel.response_timing().body_bytes_received(),
            "headers are not body receipt"
        );
        release.send(())?;
        while let Some(chunk) = response.next_chunk().await {
            assert!(chunk.is_empty(), "these responses have no decoded payload");
        }
        let result = response.finish().await;
        assert_eq!(
            result.is_err(),
            should_fail,
            "{method}/{coding}/chunked={chunked}"
        );
        let timing = cancel.response_timing();
        assert_eq!(
            timing.body_bytes_received(),
            method == "GET" && !body.is_empty(),
            "{method}/{coding}/chunked={chunked}"
        );
        assert!(timing.response_end_at().is_some());
        assert_eq!(
            timing.body_sizes(),
            Some(crate::FetchResponseBodySizes {
                encoded: if method == "GET" {
                    body.len() as u64
                } else {
                    0
                },
                decoded: 0,
            })
        );
        server.join().expect("encoded response server")?;
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
        assert_eq!(
            cancel.response_timing().body_sizes(),
            Some(crate::FetchResponseBodySizes {
                encoded: if method == "GET" {
                    GZIP.len() as u64
                } else {
                    0
                },
                decoded: 0,
            })
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

#[tokio::test]
async fn native_response_body_sizes_survive_cache_replay_revalidation_and_legacy_entries()
-> Result<()> {
    const GZIP: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 2, 3, 75, 76, 74, 6, 0, 194, 65, 36, 53, 3, 0, 0, 0,
    ];
    for coding in ["identity", "gzip"] {
        for chunked in [false, true] {
            for legacy in [false, true] {
                let payload = if coding == "gzip" {
                    GZIP
                } else {
                    b"abc".as_slice()
                };
                let cache_dir = unique_test_cache_dir();
                let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
                let url = format!("http://{}/compressed", listener.local_addr()?);
                let expected_hits = if legacy && coding == "gzip" { 3 } else { 2 };
                let server = thread::spawn(move || -> std::io::Result<()> {
                    for _ in 0..expected_hits {
                        let (mut stream, _) = listener.accept()?;
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") {
                            stream.read_exact(&mut byte)?;
                            request.push(byte[0]);
                        }
                        if String::from_utf8_lossy(&request)
                            .to_ascii_lowercase()
                            .contains("if-none-match: \"v1\"")
                        {
                            stream.write_all(b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=600\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                            continue;
                        }
                        let framing = if chunked {
                            "Transfer-Encoding: chunked".to_owned()
                        } else {
                            format!("Content-Length: {}", payload.len())
                        };
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Encoding: {coding}\r\nContent-Type: text/plain\r\nETag: \"v1\"\r\nCache-Control: max-age=600\r\n{framing}\r\nConnection: close\r\n\r\n"
                        )?;
                        if chunked {
                            write!(stream, "{:x};test=value\r\n", payload.len())?;
                            stream.write_all(payload)?;
                            stream.write_all(b"\r\n0\r\nX-Trailer: present\r\n\r\n")?;
                        } else {
                            stream.write_all(payload)?;
                        }
                    }
                    Ok(())
                });
                let mut config = FetchConfig::default();
                config.set_http_cache_dir(Some(cache_dir.display().to_string()));
                for (index, mode) in [
                    RequestCacheMode::Default,
                    RequestCacheMode::Default,
                    RequestCacheMode::Validate,
                    RequestCacheMode::Default,
                ]
                .into_iter()
                .enumerate()
                {
                    if legacy && index == 1 {
                        let mut pending = vec![cache_dir.clone()];
                        let mut changed = 0;
                        while let Some(directory) = pending.pop() {
                            for entry in fs::read_dir(directory)? {
                                let entry = entry?;
                                if entry.file_type()?.is_dir() {
                                    pending.push(entry.path());
                                } else if entry.file_name() == "meta.json" {
                                    let mut metadata: serde_json::Value =
                                        serde_json::from_slice(&fs::read(entry.path())?)?;
                                    metadata
                                        .as_object_mut()
                                        .unwrap()
                                        .remove("encoded_body_size");
                                    fs::write(entry.path(), serde_json::to_vec(&metadata)?)?;
                                    changed += 1;
                                }
                            }
                        }
                        assert_eq!(changed, 1);
                    }
                    // A new client proves that the counts are stored in the cache,
                    // rather than retained only by the first transfer's owner.
                    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
                    let cancel = FetchCancelHandle::new();
                    let mut response = client
                        .fetch_raw_stream_with_cancel(
                            Request::get(&url)?.with_cache_mode(mode),
                            cancel.clone(),
                        )
                        .await?;
                    let expected_state = match index {
                        0 => crate::ResponseCacheState::None,
                        1 if legacy && coding == "gzip" => crate::ResponseCacheState::None,
                        2 => crate::ResponseCacheState::Validated,
                        _ => crate::ResponseCacheState::Local,
                    };
                    assert_eq!(
                        response.cache_state, expected_state,
                        "{coding}/chunked={chunked}/legacy={legacy}/index={index}"
                    );
                    let mut decoded = Vec::new();
                    while let Some(chunk) = response.next_chunk().await {
                        decoded.extend_from_slice(&chunk);
                    }
                    response.finish().await?;
                    assert_eq!(decoded, b"abc");
                    assert_eq!(
                        cancel.response_timing().body_sizes(),
                        Some(crate::FetchResponseBodySizes {
                            encoded: payload.len() as u64,
                            decoded: 3
                        }),
                        "{coding}/chunked={chunked}/legacy={legacy}/index={index}"
                    );
                }
                server.join().expect("compressed cache server")?;
                fs::remove_dir_all(cache_dir)?;
            }
        }
    }
    Ok(())
}
