use super::*;
use std::io::Write;

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
