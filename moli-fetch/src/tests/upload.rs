use super::*;
use crate::{UploadEvent, UploadObserver};
use std::time::Instant;

#[tokio::test]
async fn browser_upload_reports_progress_while_response_is_held() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/browser-upload", listener.local_addr()?);
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_http_request_head(&mut stream).await.unwrap();
            let mut body = [0; 12];
            stream.read_exact(&mut body).await.unwrap();
            assert_eq!(&body, b"Test Message");
            release_rx.await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let (event_tx, mut events) = mpsc::unbounded_channel();
        let started = Instant::now();
        let observer = UploadObserver::new_browser(12, move |report| {
            let _ = event_tx.send((report.event, Instant::now()));
        });
        let request = Request::new_bytes(
            "POST",
            &url,
            Some(b"Test Message".to_vec().into()),
            Vec::new(),
        )?
        .with_upload_observer(observer);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let fetch = upload_response(&client, request, transport, FetchCancelHandle::new());
        let observe = async {
            let (event, at) = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                event,
                UploadEvent::Complete {
                    loaded: 12,
                    total: 12
                }
            );
            assert!(
                at.duration_since(started) >= Duration::from_millis(90),
                "browser progress must follow the reporting cadence: {transport}"
            );
            release_tx.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(fetch, observe);
        assert_eq!(result?, 200);
        assert!(
            events.try_recv().is_err(),
            "duplicate final progress: {transport}"
        );
        server.await?;
    }
    Ok(())
}

#[tokio::test]
async fn browser_upload_holds_reports_until_owner_acknowledges() -> Result<()> {
    for flush_at_response_start in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/backpressure", listener.local_addr()?);
        let total = 16 * 1024 * 1024;
        let (received_tx, received_rx) = oneshot::channel();
        let (read_tx, read_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_http_request_head(&mut stream).await.unwrap();
            // Keep the receive window closed until a partial report is observed.
            // This also makes the first report independent of test scheduling.
            read_rx.await.unwrap();
            let mut body = vec![0; total];
            stream.read_exact(&mut body).await.unwrap();
            if flush_at_response_start {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
            received_tx.send(()).unwrap();
            release_rx.await.unwrap();
            if !flush_at_response_start {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
            stream.write_all(b"ok").await.unwrap();
        });
        let (event_tx, mut events) = mpsc::unbounded_channel();
        let observer = UploadObserver::new_browser(total as u64, move |report| {
            let _ = event_tx.send(report);
        });
        let request = Request::new_bytes(
            "POST",
            &url,
            Some(vec![b'x'; total].into()),
            vec![("Expect".into(), String::new())],
        )?
        .with_upload_observer(observer);
        let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
        let fetch = upload_response(&client, request, "raw", FetchCancelHandle::new());
        let observe = async {
            let report = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(report.event, UploadEvent::Progress { loaded, total } if loaded > 0 && loaded < total)
            );
            read_tx.send(()).unwrap();
            received_rx.await.unwrap();
            let report = if flush_at_response_start {
                // Response headers must flush the final byte counts even while
                // the previous report is unacknowledged and the body is held.
                Some(report)
            } else {
                tokio::time::sleep(Duration::from_millis(220)).await;
                assert!(
                    events.try_recv().is_err(),
                    "pending owner must coalesce further upload reports"
                );
                drop(report);
                None
            };
            let final_report = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                final_report.event,
                UploadEvent::Complete {
                    loaded: total as u64,
                    total: total as u64
                }
            );
            drop(report);
            drop(final_report);
            release_tx.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(fetch, observe);
        assert_eq!(result?, 200);
        server.await?;
    }
    Ok(())
}

#[tokio::test]
async fn browser_upload_does_not_report_empty_or_absent_bodies() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        for body in [None, Some(Vec::new())] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}/empty-upload", listener.local_addr()?);
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let _ = read_http_request_head(&mut stream).await.unwrap();
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            });
            let (event_tx, mut events) = mpsc::unbounded_channel();
            let observer = UploadObserver::new_browser(0, move |report| {
                let _ = event_tx.send(report.event);
            });
            let request = Request::new_bytes("POST", &url, body.map(Into::into), Vec::new())?
                .with_upload_observer(observer);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
            assert_eq!(
                upload_response(&client, request, transport, FetchCancelHandle::new()).await?,
                200
            );
            assert!(
                events.try_recv().is_err(),
                "empty upload reported progress: {transport}"
            );
            server.await?;
        }
    }
    Ok(())
}

async fn upload_response(
    client: &FetchClient,
    request: Request,
    transport: &str,
    cancel: FetchCancelHandle,
) -> Result<u16> {
    match transport {
        "buffered" => Ok(client
            .fetch_with_cancel(request.with_single_hop(), cancel)
            .await?
            .status),
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
            let request =
                Request::new_bytes("POST", &url, body.clone().map(Into::into), Vec::new())?
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
        for browser in [false, true] {
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
            let observer = if browser {
                UploadObserver::new_browser(body.len() as u64, move |report| {
                    let _ = event_tx.send(report.event);
                })
            } else {
                UploadObserver::new(body.len() as u64, move |event| {
                    let _ = event_tx.send(event);
                })
            };
            let request = Request::new_bytes(
                "POST",
                &url,
                Some(body.into()),
                vec![("Expect".into(), "100-continue".into())],
            )?
            .with_upload_observer(observer);
            let client =
                FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
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
            Some(vec![b'x'; total as usize].into()),
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
