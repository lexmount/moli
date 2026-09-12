use super::*;
use crate::{UploadEvent, UploadObserver};

async fn upload_response(
    client: &FetchClient,
    mut request: Request,
    transport: &str,
    cancel: FetchCancelHandle,
) -> Result<u16> {
    match transport {
        "buffered" => {
            request.follow_redirects = false;
            Ok(client.fetch_with_cancel(request, cancel).await?.status)
        }
        "html" => {
            let mut response = client.fetch_html_stream(request).await?;
            while response.next_chunk().await.is_some() {}
            response.finish().await?;
            Ok(response.status)
        }
        "raw" => {
            let mut response = client.fetch_raw_stream_with_cancel(request, cancel).await?;
            while response.next_chunk().await.is_some() {}
            response.finish().await?;
            Ok(response.status)
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn upload_observer_finishes_before_response_and_distinguishes_empty_from_absent() -> Result<()>
{
    for transport in ["buffered", "html", "raw"] {
        for body in [None, Some(Vec::new()), Some(b"\x00body\xff".to_vec())] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}/upload", listener.local_addr()?);
            let expected = body.clone().unwrap_or_default();
            let total = expected.len() as u64;
            let (received_tx, received_rx) = oneshot::channel();
            let (release_tx, release_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let head = read_http_request_head(&mut stream).await.unwrap();
                assert!(head.starts_with("POST /upload HTTP/1.1\r\n"));
                let mut bytes = vec![0; expected.len()];
                stream.read_exact(&mut bytes).await.unwrap();
                assert_eq!(bytes, expected);
                received_tx.send(()).unwrap();
                release_rx.await.unwrap();
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            });
            let (event_tx, mut events) = mpsc::unbounded_channel();
            let observer = UploadObserver::new(total, move |event| {
                let _ = event_tx.send(event);
            });
            let request = Request::new_bytes("POST", &url, body.clone(), Vec::new())?
                .with_upload_observer(observer);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            let fetch = upload_response(&client, request, transport, FetchCancelHandle::new());
            let observe = async {
                received_rx.await.unwrap();
                if body.is_some() {
                    loop {
                        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
                            .await
                            .unwrap()
                            .unwrap();
                        if let UploadEvent::Complete {
                            loaded,
                            total: observed_total,
                        } = event
                        {
                            assert_eq!((loaded, observed_total), (total, total), "{transport}");
                            break;
                        }
                    }
                } else {
                    assert!(events.try_recv().is_err());
                }
                release_tx.send(()).unwrap();
            };
            let (result, ()) = tokio::join!(fetch, observe);
            assert_eq!(result?, 200);
            assert!(
                events.try_recv().is_err(),
                "duplicate upload completion: {transport}"
            );
            server.await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn upload_observer_does_not_infer_completion_from_an_early_response() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/reject", listener.local_addr()?);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let head = read_http_request_head(&mut stream).await.unwrap();
            assert!(
                head.to_ascii_lowercase()
                    .contains("expect: 100-continue\r\n")
            );
            stream.write_all(b"HTTP/1.1 413 Content Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let (event_tx, mut events) = mpsc::unbounded_channel();
        let body = vec![b'x'; 1024 * 1024];
        let observer = UploadObserver::new(body.len() as u64, move |event| {
            let _ = event_tx.send(event);
        });
        let request = Request::new_bytes(
            "POST",
            &url,
            Some(body),
            vec![("Expect".into(), "100-continue".into())],
        )?
        .with_upload_observer(observer);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        assert_eq!(
            upload_response(&client, request, transport, FetchCancelHandle::new()).await?,
            413
        );
        assert!(
            events.try_recv().is_err(),
            "unsent body reported as uploaded: {transport}"
        );
        server.await?;
    }
    Ok(())
}

#[tokio::test]
async fn upload_observer_reports_partial_bytes_and_stops_after_cancellation() -> Result<()> {
    for transport in ["buffered", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/partial", listener.local_addr()?);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_http_request_head(&mut stream).await.unwrap();
            // Hold the receive window closed long enough to expose a partial
            // upload, then let transmission advance to the next progress task.
            tokio::time::sleep(Duration::from_millis(150)).await;
            let mut received = 0;
            let mut chunk = [0; 65536];
            while let Ok(count) = stream.read(&mut chunk).await {
                if count == 0 {
                    break;
                }
                received += count;
            }
            received
        });
        let cancel = FetchCancelHandle::new();
        let cancel_upload = cancel.clone();
        let (event_tx, mut events) = mpsc::unbounded_channel();
        let total = 16 * 1024 * 1024;
        let observer = UploadObserver::new(total, move |event| {
            if matches!(event, UploadEvent::Progress { loaded, total } if loaded > 0 && loaded < total)
            {
                cancel_upload.cancel();
            }
            let _ = event_tx.send(event);
        });
        let request = Request::new_bytes(
            "POST",
            &url,
            Some(vec![b'x'; total as usize]),
            vec![("Expect".into(), String::new())],
        )?
        .with_upload_observer(observer);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            upload_response(&client, request, transport, cancel),
        )
        .await?;
        assert!(
            result.is_err(),
            "partial cancellation must abort {transport}"
        );
        let mut partial = false;
        while let Ok(event) = events.try_recv() {
            match event {
                UploadEvent::Progress {
                    loaded,
                    total: observed_total,
                } => {
                    assert_eq!(observed_total, total);
                    assert!(loaded > 0 && loaded < total);
                    partial = true;
                }
                UploadEvent::Complete { .. } => panic!("cancelled upload completed: {transport}"),
            }
        }
        assert!(partial, "missing partial upload: {transport}");
        assert!(tokio::time::timeout(Duration::from_secs(3), server).await?? < total as usize);
    }
    Ok(())
}
