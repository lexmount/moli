use super::*;
use crate::UploadBody;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
struct RecordedUpload {
    method: http::Method,
    headers: http::HeaderMap,
    bytes: Vec<u8>,
    ended: bool,
}

struct UploadServer {
    url: String,
    records: Arc<Mutex<Vec<RecordedUpload>>>,
    errors: Arc<Mutex<Vec<String>>>,
    progress: mpsc::UnboundedReceiver<Vec<u8>>,
    stop: CancellationToken,
    end: Arc<tokio::sync::Notify>,
    release_response: Arc<tokio::sync::Notify>,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl UploadServer {
    async fn spawn(status: u16) -> Result<Self> {
        Self::spawn_with_response_timing(status, false, false).await
    }

    async fn spawn_with_response_timing(
        status: u16,
        early_response: bool,
        hold_response_end: bool,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("https://{}/upload", listener.local_addr()?);
        let records = Arc::new(Mutex::new(Vec::new()));
        let recorded = records.clone();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let server_errors = errors.clone();
        let (progress, received) = mpsc::unbounded_channel();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let end = Arc::new(tokio::sync::Notify::new());
        let received_end = end.clone();
        let release_response = Arc::new(tokio::sync::Notify::new());
        let response_release = release_response.clone();
        let acceptor = support::h2_tls_acceptor();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let stream = tokio::select! {
                    _ = stopped.cancelled() => break,
                    accepted = listener.accept() => accepted?.0,
                };
                let acceptor = acceptor.clone();
                let records = recorded.clone();
                let progress = progress.clone();
                let stopped = stopped.clone();
                let response_release = response_release.clone();
                let end = received_end.clone();
                let errors = server_errors.clone();
                connections.spawn(async move {
                    let result = async {
                        let stream = acceptor.accept(stream).await?;
                        let mut connection = h2::server::handshake(stream).await?;
                        let mut requests = tokio::task::JoinSet::new();
                        loop {
                            let request = tokio::select! {
                                _ = stopped.cancelled() => break,
                                request = connection.accept() => request,
                            };
                            let Some(Ok((request, mut respond))) = request else {
                                break;
                            };
                            let index = {
                                let mut rows = records.lock();
                                let index = rows.len();
                                rows.push(RecordedUpload {
                                    method: request.method().clone(),
                                    headers: request.headers().clone(),
                                    bytes: Vec::new(),
                                    ended: false,
                                });
                                index
                            };
                            let records = records.clone();
                            let progress = progress.clone();
                            let response_release = response_release.clone();
                            let end = end.clone();
                            requests.spawn(async move {
                                let mut early = if early_response {
                                    let head = http::Response::builder()
                                        .status(status)
                                        .header("cache-control", "no-store")
                                        .body(())?;
                                    let mut send = respond.send_response(head, false)?;
                                    send.send_data(
                                        bytes::Bytes::from_static(b"uploaded"),
                                        !hold_response_end,
                                    )?;
                                    Some(send)
                                } else {
                                    None
                                };
                                let mut body = request.into_body();
                                while let Some(chunk) = body.data().await {
                                    let chunk = chunk?;
                                    records.lock()[index].bytes.extend_from_slice(&chunk);
                                    let _ = progress.send(chunk.to_vec());
                                    body.flow_control().release_capacity(chunk.len())?;
                                }
                                records.lock()[index].ended = true;
                                end.notify_one();
                                if let Some(send) = early.as_mut() {
                                    if hold_response_end {
                                        response_release.notified().await;
                                        send.send_data(bytes::Bytes::new(), true)?;
                                    }
                                } else {
                                    let head = http::Response::builder()
                                        .status(status)
                                        .header("cache-control", "no-store")
                                        .body(())?;
                                    respond
                                        .send_response(head, false)?
                                        .send_data(bytes::Bytes::from_static(b"uploaded"), true)?;
                                }
                                Ok::<_, anyhow::Error>(())
                            });
                        }
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    if let Err(error) = &result {
                        errors.lock().push(format!("{error:#}"));
                    }
                    result
                });
            }
            Ok(())
        });
        Ok(Self {
            url,
            records,
            errors,
            progress: received,
            stop,
            end,
            release_response,
            task: Some(task),
        })
    }

    async fn received_chunk(&mut self) -> Result<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(5), self.progress.recv())
            .await
            .with_context(|| format!("upload server errors: {:?}", self.errors.lock()))?
            .context("upload server closed before receiving a chunk")
    }

    async fn shutdown(mut self) -> Result<()> {
        self.stop.cancel();
        self.task.take().unwrap().await??;
        Ok(())
    }

    async fn received_end(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), self.end.notified())
            .await
            .with_context(|| format!("upload server errors: {:?}", self.errors.lock()))
    }
}

impl Drop for UploadServer {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn upload_client() -> FetchClient {
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_request_timeout_ms(5000);
    FetchClient::new(&config, new_shared_browser_cookie_store())
}

async fn response(
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

#[tokio::test]
async fn upload_body_sends_h2_chunks_before_eof_on_all_transports() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let mut server = UploadServer::spawn(200).await?;
        let client = upload_client();
        let (sender, body) = UploadBody::channel();
        let mut request = Request::get(&server.url)?.with_upload_body(body);
        request.method = "POST".into();
        let first = bytes::Bytes::from_static(b"first\0\xff");
        let second = bytes::Bytes::from(vec![b'x'; 196_613]);
        let expected = [first.as_ref(), second.as_ref()].concat();
        let fetch = response(&client, request, transport);
        let produce = async {
            sender.send(first.clone()).await?;
            // The server sees data while the producer remains open. A
            // read-all-bytes implementation cannot pass this handshake.
            assert_eq!(server.received_chunk().await?, first);
            sender.send(bytes::Bytes::new()).await?;
            sender.send(second).await?;
            sender.finish();
            Ok::<_, anyhow::Error>(())
        };
        let (fetch, produce) = tokio::join!(fetch, produce);
        produce?;
        let (head, bytes) = fetch?;
        server.received_end().await?;
        assert_eq!(head.status, 200, "{transport}");
        assert_eq!(
            head.negotiated_http_version,
            Some(NegotiatedHttpVersion::Http2)
        );
        assert_eq!(bytes, b"uploaded");
        let rows = server.records.lock().clone();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].method, http::Method::POST);
        assert_eq!(rows[0].bytes, expected);
        for name in [
            "content-length",
            "content-type",
            "expect",
            "transfer-encoding",
        ] {
            assert!(!rows[0].headers.contains_key(name), "{transport}: {name}");
        }
        assert!(client.shutdown().is_clean());
        server.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn upload_body_drop_aborts_an_active_h2_upload_on_all_transports() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let mut server = UploadServer::spawn(200).await?;
        let client = upload_client();
        let (sender, body) = UploadBody::channel();
        let mut request = Request::get(&server.url)?.with_upload_body(body);
        request.method = "POST".into();
        let fetch = response(&client, request, transport);
        let produce = async {
            sender.send(bytes::Bytes::from_static(b"partial")).await?;
            assert_eq!(server.received_chunk().await?, b"partial");
            drop(sender);
            Ok::<_, anyhow::Error>(())
        };
        let (fetch, produce) = tokio::join!(fetch, produce);
        produce?;
        assert!(fetch.is_err(), "{transport}: incomplete upload must fail");
        assert_eq!(server.records.lock().len(), 1);
        assert!(client.shutdown().is_clean());
        server.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn upload_body_421_never_replays_on_all_transports() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let server = UploadServer::spawn(421).await?;
        let client = upload_client();
        let (sender, body) = UploadBody::channel();
        let mut request = Request::get(&server.url)?
            .with_upload_body(body)
            .with_body_source_is_null(false);
        request.method = "POST".into();
        assert!(request.body_source_is_null());
        sender.send(bytes::Bytes::from_static(b"once")).await?;
        sender.finish();
        let (head, bytes) = response(&client, request, transport).await?;
        assert_eq!(head.status, 421, "{transport}");
        assert_eq!(bytes, b"uploaded");
        assert_eq!(server.records.lock().len(), 1);
        assert!(client.shutdown().is_clean());
        server.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn upload_body_refuses_http1_including_a_warm_connection() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        let server = ScriptedHttps11Server::spawn(vec![
            ScriptedResponse::ok("warm"),
            ScriptedResponse::ok("must not upload"),
        ]);
        let client = upload_client();
        let warm = client.fetch(Request::get(&server.url())?).await?;
        assert_eq!(
            warm.negotiated_http_version,
            Some(NegotiatedHttpVersion::Http11)
        );
        let (sender, body) = UploadBody::channel();
        let mut request = Request::get(&server.url())?.with_upload_body(body);
        request.method = "POST".into();
        sender.send(bytes::Bytes::from_static(b"not HTTP1")).await?;
        sender.finish();
        assert!(response(&client, request, transport).await.is_err());
        assert_eq!(server.hits(), 1, "{transport}: must not reuse HTTP/1");
        assert!(client.shutdown().is_clean());
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn upload_body_early_responses_keep_sending_until_eof_on_all_transports() -> Result<()> {
    for transport in ["buffered", "html", "raw"] {
        for status in [200, 303, 401] {
            let mut server = UploadServer::spawn_with_response_timing(status, true, false).await?;
            let client = upload_client();
            let (sender, body) = UploadBody::channel();
            let mut request = Request::get(&server.url)?
                .with_upload_body(body)
                .with_redirect_mode(RequestRedirectMode::Manual);
            request.method = "POST".into();
            let transmitted = sender.transmission();
            let fetch = response(&client, request, transport);
            let produce = async {
                sender
                    .send(bytes::Bytes::from_static(b"first"))
                    .await
                    .context("first chunk")?;
                assert_eq!(server.received_chunk().await?, b"first");
                sender
                    .send(bytes::Bytes::from_static(b"second"))
                    .await
                    .context("second chunk")?;
                sender.finish();
                transmitted.await.context("upload transmission")?;
                Ok::<_, anyhow::Error>(())
            };
            let (fetch, produce) = tokio::join!(fetch, produce);
            produce.with_context(|| {
                format!(
                    "{transport}, early {status}, server records: {:?}",
                    server.records.lock()
                )
            })?;
            let (head, bytes) =
                fetch.with_context(|| format!("{transport}, early {status}: response"))?;
            server.received_end().await?;
            assert_eq!(head.status, status, "{transport}");
            assert_eq!(bytes, b"uploaded");
            assert_eq!(server.records.lock()[0].bytes, b"firstsecond");
            assert!(server.records.lock()[0].ended);
            assert!(client.shutdown().is_clean());
            server.shutdown().await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn upload_body_transmission_completes_before_response_eof() -> Result<()> {
    let server = UploadServer::spawn_with_response_timing(200, true, true).await?;
    let client = upload_client();
    let (sender, body) = UploadBody::channel();
    let mut request = Request::get(&server.url)?.with_upload_body(body);
    request.method = "POST".into();
    sender.send(bytes::Bytes::from_static(b"first")).await?;
    let transmitted = sender.transmission();
    let mut response = client
        .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
        .await?;
    assert_eq!(
        response
            .next_chunk()
            .await
            .context("early response chunk")?,
        b"uploaded"
    );
    sender.send(bytes::Bytes::from_static(b"second")).await?;
    sender.finish();
    // The server waits for this notification before ending its response.
    // Waiting for CURLMSG_DONE instead of upload progress would deadlock.
    tokio::time::timeout(Duration::from_secs(3), transmitted).await??;
    server.received_end().await?;
    server.release_response.notify_one();
    assert!(response.next_chunk().await.is_none());
    response.finish().await?;
    assert_eq!(server.records.lock()[0].bytes, b"firstsecond");
    assert!(client.shutdown().is_clean());
    server.shutdown().await?;
    Ok(())
}
