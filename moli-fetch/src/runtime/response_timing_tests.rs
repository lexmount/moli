use super::*;
use moli_cookie_jar::new_shared_browser_cookie_store;
use std::{io::Write, sync::mpsc as std_mpsc};

fn submit_html_request(
    config: &FetchConfig,
    request: Request,
) -> (
    FetchRuntimeOwner,
    FetchCancelHandle,
    PendingStreamingHtmlResponse,
) {
    let owner = FetchRuntimeOwner::new(config, new_shared_browser_cookie_store());
    let pending = owner.handle().submit_html_stream(request).unwrap();
    let cancel = pending.cancel_handle.clone();
    (owner, cancel, pending)
}

#[tokio::test]
async fn html_redirect_timing_belongs_to_the_final_transfer() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/redirect", listener.local_addr()?);
    let (release_redirect, ready) = std_mpsc::sync_channel(1);
    let server = thread::spawn(move || -> std::io::Result<()> {
        for path in ["/redirect", "/final"] {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte)?;
                request.extend_from_slice(&byte);
            }
            assert!(request.starts_with(format!("GET {path} ").as_bytes()));
            if path == "/redirect" {
                stream.write_all(
                    b"HTTP/1.1 103 Early Hints\r\n\r\n\
                      HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 8\r\nConnection: close\r\n\r\nold!",
                )?;
                ready.recv_timeout(Duration::from_secs(5)).unwrap();
                stream.write_all(b"tail")?;
            } else {
                stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )?;
            }
        }
        Ok(())
    });
    let (_owner, cancel, pending) =
        submit_html_request(&FetchConfig::default(), Request::get(&url)?);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cancel.response_timing().body_bytes_received() {
        anyhow::ensure!(Instant::now() < deadline, "redirect body was not observed");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let redirect_timing = cancel.response_timing();
    assert!(
        redirect_timing
            .first_interim_response_started_at()
            .is_some()
    );
    assert!(redirect_timing.response_started_at().is_some());
    let redirect_finished_after = Instant::now();
    release_redirect.send(())?;

    let mut response = pending.into_response().await?;
    assert_eq!(response.final_url.path(), "/final");
    assert!(response.next_chunk().await.is_none());
    response.finish().await?;
    let timing = cancel.response_timing();
    assert!(timing.request_started_at().unwrap() >= redirect_finished_after);
    assert!(timing.response_started_at().unwrap() >= redirect_finished_after);
    assert!(timing.first_interim_response_started_at().is_none());
    assert!(!timing.body_bytes_received());
    server.join().expect("server thread")?;
    Ok(())
}

#[tokio::test]
async fn html_response_timing_finishes_without_consumer_polling() -> Result<()> {
    for (framing, body) in [
        ("Content-Length: 4", b"body".as_slice()),
        (
            "Transfer-Encoding: chunked",
            b"4\r\nbody\r\n0\r\n\r\n".as_slice(),
        ),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/timed", listener.local_addr()?);
        let (release_body, ready) = std_mpsc::sync_channel(1);
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            read_request(&mut stream)?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\n{framing}\r\nConnection: close\r\n\r\n"
            )?;
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            stream.write_all(body)?;
            Ok(())
        });
        let (_owner, cancel, pending) =
            submit_html_request(&FetchConfig::default(), Request::get(&url)?);
        let mut response = pending.into_response().await?;
        let headers = cancel.response_timing();
        assert!(headers.response_started_at().is_some());
        assert!(headers.response_end_at().is_none());
        release_body.send(())?;
        // Keep the consumer's executor occupied while native threads finish.
        // Body completion can commit before curl publishes terminal statistics.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancel.response_completion_is_committed()
            || cancel
                .response_timing()
                .connection_timing()
                .and_then(|connection| connection.curl)
                .is_none()
        {
            anyhow::ensure!(
                Instant::now() < deadline,
                "HTML transfer did not record completion"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let native = cancel.response_timing();
        assert!(native.connection_timing().unwrap().curl.is_some());
        assert!(native.body_bytes_received());
        assert!(native.response_started_at().unwrap() <= native.response_end_at().unwrap());
        assert!(cancel.response_completion_is_committed());
        assert!(!cancel.cancel_if_response_pending());
        let mut text = String::new();
        while let Some(chunk) = response.next_chunk().await {
            text.push_str(&chunk);
        }
        response.finish().await?;
        assert_eq!(text, "body");
        assert_eq!(cancel.response_timing(), native);
        server.join().expect("server thread")?;
    }
    Ok(())
}

fn read_request(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        request.extend_from_slice(&byte);
    }
    Ok(())
}

#[tokio::test]
async fn html_response_timing_records_terminal_failures() -> Result<()> {
    for bytes in [
        b"".as_slice(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\npart".as_slice(),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/failure", listener.local_addr()?);
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            read_request(&mut stream)?;
            stream.write_all(bytes)?;
            Ok(())
        });
        let (_owner, cancel, pending) =
            submit_html_request(&FetchConfig::default(), Request::get(&url)?);
        match pending.into_response().await {
            Ok(mut response) => assert!(response.finish().await.is_err()),
            Err(_) => assert!(bytes.is_empty()),
        }
        let timing = cancel.response_timing();
        assert_eq!(timing.response_started_at().is_some(), !bytes.is_empty());
        assert!(timing.response_end_at().is_some());
        assert!(cancel.response_completion_is_committed());
        assert!(!cancel.cancel_if_response_pending());
        server.join().expect("server thread")?;
    }
    Ok(())
}

#[tokio::test]
async fn html_response_timing_finishes_bodyless_responses_at_headers() -> Result<()> {
    for (method, headers) in [
        ("GET", "HTTP/1.1 101 Switching Protocols\r\n"),
        ("GET", "HTTP/1.1 204 No Content\r\n"),
        ("GET", "HTTP/1.1 205 Reset Content\r\n"),
        ("GET", "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n"),
        ("HEAD", "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n"),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/empty", listener.local_addr()?);
        let (release, ready) = std_mpsc::sync_channel(1);
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            read_request(&mut stream)?;
            write!(stream, "{headers}Connection: close\r\n\r\n")?;
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        });
        let request = Request::new(method, &url, None, Vec::new())?;
        let (_owner, cancel, pending) = submit_html_request(&FetchConfig::default(), request);
        let mut response = pending.into_response().await?;
        tokio::time::timeout(Duration::from_secs(5), response.finish()).await??;
        let timing = cancel.response_timing();
        assert!(timing.response_end_at().is_some(), "{method}: {headers}");
        assert!(timing.response_started_at().unwrap() <= timing.response_end_at().unwrap());
        assert!(!timing.body_bytes_received());
        assert!(!cancel.cancel_if_response_pending());
        release.send(())?;
        server.join().expect("server thread")?;
    }
    Ok(())
}

#[tokio::test]
async fn html_response_timing_preserves_cancellation_before_body_completion() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/cancel", listener.local_addr()?);
    let (release, ready) = std_mpsc::sync_channel(1);
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        read_request(&mut stream)?;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\npart")?;
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(())
    });
    let (_owner, cancel, pending) =
        submit_html_request(&FetchConfig::default(), Request::get(&url)?);
    let mut response = pending.into_response().await?;
    assert_eq!(response.next_chunk().await.as_deref(), Some("part"));
    assert!(cancel.response_timing().response_end_at().is_none());
    assert!(cancel.cancel_if_response_pending());
    let error = tokio::time::timeout(Duration::from_secs(5), response.finish())
        .await?
        .expect_err("cancellation must terminate the held HTML response");
    assert!(crate::is_fetch_cancelled(&error), "{error:#}");
    assert!(cancel.response_timing().response_end_at().is_some());
    assert!(!cancel.response_completion_is_committed());
    release.send(())?;
    server.join().expect("server thread")?;
    Ok(())
}

#[tokio::test]
async fn html_cached_response_timing_does_not_invent_network_observations() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/cache", listener.local_addr()?);
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-html-timing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
    ));
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        read_request(&mut stream)?;
        stream.write_all(b"HTTP/1.1 200 OK\r\nCache-Control: max-age=600\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody")?;
        Ok(())
    });
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let (owner, _, pending) = submit_html_request(&config, Request::get(&url)?);
    pending.into_response().await?.finish().await?;
    server.join().expect("server thread")?;

    let pending = owner.handle().submit_html_stream(Request::get(&url)?)?;
    let cancel = pending.cancel_handle.clone();
    let mut response = pending.into_response().await?;
    assert!(response.from_cache);
    let mut text = String::new();
    while let Some(chunk) = response.next_chunk().await {
        text.push_str(&chunk);
    }
    response.finish().await?;
    assert_eq!(text, "body");
    let timing = cancel.response_timing();
    assert!(timing.request_started_at().is_none());
    assert!(timing.first_interim_response_started_at().is_none());
    assert!(!timing.body_bytes_received());
    assert!(timing.response_started_at().is_some());
    assert!(timing.response_started_at().unwrap() <= timing.response_end_at().unwrap());
    assert!(!cancel.cancel_if_response_pending());
    drop(owner);
    std::fs::remove_dir_all(cache_dir)?;
    Ok(())
}

#[tokio::test]
async fn html_declared_body_completion_precedes_delivery_of_the_last_chunk() -> Result<()> {
    for (encoding_headers, complete) in [
        (b"".as_slice(), true),
        (b"Content-Encoding: gzip\r\n".as_slice(), false),
        (b"Transfer-Encoding: chunked\r\n".as_slice(), false),
    ] {
        let cancel = FetchCancelHandle::new();
        let (start_tx, _start_rx) = oneshot::channel();
        let (body_tx, mut body_rx) = mpsc::unbounded_channel();
        let mut collector = StreamingResponseCollector::new(
            new_shared_browser_cookie_store(),
            start_tx,
            body_tx,
            cancel.clone(),
        );
        collector.begin_request(
            None,
            Url::parse("http://example.test/body")?,
            NetworkCookieRequestContext::top_level_navigation("GET"),
            None,
            true,
            Vec::new(),
            None,
        );
        let mut handler = FetchTransferHandler::new_streaming(collector);
        assert!(handler.header(b"HTTP/1.1 200 OK\r\n"));
        assert!(handler.header(b"Content-Length: 4\r\n"));
        if !encoding_headers.is_empty() {
            assert!(handler.header(encoding_headers));
        }
        assert!(handler.header(b"\r\n"));
        assert_eq!(handler.write(b"bo").unwrap(), 2);
        assert_eq!(body_rx.try_recv()?.as_str(), "bo");
        assert!(cancel.response_timing().body_bytes_received());
        assert!(cancel.response_timing().response_end_at().is_none());
        assert_eq!(handler.write(b"dy").unwrap(), 2);
        assert_eq!(body_rx.try_recv()?.as_str(), "dy");
        // Decoded or chunked lengths cannot be compared with Content-Length.
        assert_eq!(
            cancel.response_timing().response_end_at().is_some(),
            complete
        );
        assert_eq!(cancel.cancel_if_response_pending(), !complete);
    }
    Ok(())
}

#[test]
fn switching_protocols_timing_is_final_in_every_collector() {
    for interim in [false, true] {
        for mode in ["buffered", "html", "raw"] {
            let cancel = FetchCancelHandle::new();
            let mut handler = match mode {
                "buffered" => {
                    FetchTransferHandler::new_buffered(ResponseCollector::new(Some(cancel.clone())))
                }
                "html" => {
                    let (start_tx, _start_rx) = oneshot::channel();
                    let (body_tx, _body_rx) = mpsc::unbounded_channel();
                    FetchTransferHandler::new_streaming(StreamingResponseCollector::new(
                        new_shared_browser_cookie_store(),
                        start_tx,
                        body_tx,
                        cancel.clone(),
                    ))
                }
                "raw" => {
                    let (start_tx, _start_rx) = oneshot::channel();
                    let (body_tx, _body_rx) = mpsc::unbounded_channel();
                    FetchTransferHandler::new_raw_streaming(RawStreamingResponseCollector::new(
                        new_shared_browser_cookie_store(),
                        start_tx,
                        body_tx,
                        cancel.clone(),
                    ))
                }
                _ => unreachable!(),
            };
            if interim {
                assert!(handler.header(b"HTTP/1.1 103 Early Hints\r\n"));
                assert!(cancel.response_timing().response_started_at().is_none());
            }
            assert!(handler.header(b"HTTP/1.1 101 Switching Protocols\r\n"));
            let timing = cancel.response_timing();
            assert!(
                timing.response_started_at().is_some(),
                "{mode}: 101 must be final"
            );
            assert_eq!(
                timing.first_interim_response_started_at().is_some(),
                interim,
                "{mode}"
            );
            if let Some(first_interim) = timing.first_interim_response_started_at() {
                assert!(first_interim <= timing.response_started_at().unwrap());
            }
        }
    }
}

#[test]
fn transfer_progress_preserves_cancellation_and_body_observations_on_reuse() {
    let cancel = FetchCancelHandle::new();
    let mut handler =
        FetchTransferHandler::new_buffered(ResponseCollector::new(Some(cancel.clone())));
    for _ in 0..2 {
        cancel.begin_response_progress();
        handler.buffered_mut().unwrap().begin_request(None);
        handler.begin_transfer(None, false, None);
        assert!(handler.progress(4.0, 0.0, 0.0, 0.0));
        assert!(!cancel.response_timing().body_bytes_received());
        assert!(handler.progress(4.0, 1.0, 0.0, 0.0));
        assert!(cancel.response_timing().body_bytes_received());
        assert!(handler.progress(4.0, 4.0, 0.0, 0.0));
    }
    cancel.cancel();
    assert!(!handler.progress(4.0, 4.0, 0.0, 0.0));
}

#[tokio::test]
async fn html_redirect_to_a_cached_response_preserves_channels_and_timing() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = format!("http://{}", listener.local_addr()?);
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-html-redirect-timing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
    ));
    let server = thread::spawn(move || -> std::io::Result<()> {
        for response in [
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=600\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody".as_slice(),
            b"HTTP/1.1 103 Early Hints\r\n\r\nHTTP/1.1 302 Found\r\nLocation: /cache\r\nContent-Length: 4\r\nConnection: close\r\n\r\njunk".as_slice(),
        ] {
            let (mut stream, _) = listener.accept()?;
            read_request(&mut stream)?;
            stream.write_all(response)?;
        }
        Ok(())
    });
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let (owner, _, pending) =
        submit_html_request(&config, Request::get(&format!("{address}/cache"))?);
    pending.into_response().await?.finish().await?;

    let pending = owner
        .handle()
        .submit_html_stream(Request::get(&format!("{address}/redirect"))?)?;
    let cancel = pending.cancel_handle.clone();
    let mut response = pending.into_response().await?;
    assert!(response.from_cache);
    assert!(response.redirected);
    assert_eq!(response.final_url.path(), "/cache");
    assert_eq!(response.redirect_chain.len(), 1);
    assert!(!response.redirect_chain[0].from_cache);
    let mut body = String::new();
    while let Some(chunk) = response.next_chunk().await {
        body.push_str(&chunk);
    }
    response.finish().await?;
    assert_eq!(body, "body");
    let timing = cancel.response_timing();
    assert!(timing.request_started_at().is_none());
    assert!(timing.first_interim_response_started_at().is_none());
    assert!(!timing.body_bytes_received());
    assert!(timing.response_started_at().unwrap() <= timing.response_end_at().unwrap());
    assert!(!cancel.cancel_if_response_pending());
    server.join().expect("server thread")?;
    drop(owner);
    std::fs::remove_dir_all(cache_dir)?;
    Ok(())
}

#[tokio::test]
async fn html_failure_before_reusing_an_easy_preserves_the_original_error() -> Result<()> {
    let cancel = FetchCancelHandle::new();
    let (started_tx, started_rx) = oneshot::channel();
    let (body_tx, mut body_rx) = mpsc::unbounded_channel();
    let (completion_tx, completion_rx) = oneshot::channel();
    let mut job = StreamingRuntimeJob::new(
        Request::get("http://example.test/cache")?,
        started_tx,
        body_tx,
        completion_tx,
        cancel.clone(),
    );
    job.easy = Some(Easy2::new(FetchTransferHandler::new_streaming(
        StreamingResponseCollector::new(
            new_shared_browser_cookie_store(),
            job.started_tx.take().unwrap(),
            job.body_tx.take().unwrap(),
            cancel.clone(),
        ),
    )));
    fail_streaming_job_with_easy(job, None, anyhow!("cached redirect rejected"));
    let error = started_rx.await?.unwrap_err();
    assert!(format!("{error:#}").contains("cached redirect rejected"));
    assert!(body_rx.recv().await.is_none());
    assert!(completion_rx.await?.is_err());
    assert!(cancel.response_timing().response_end_at().is_some());
    Ok(())
}
