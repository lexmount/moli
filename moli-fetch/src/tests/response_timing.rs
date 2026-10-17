use super::*;
use std::io::Write;

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
