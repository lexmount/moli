use super::*;

#[tokio::test(flavor = "current_thread")]
async fn worker_fetch_body_abort_preserves_signal_reason_for_all_body_methods() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker fetch abort server");
    let address = listener.local_addr().expect("worker fetch abort address");
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut response_stream = None;
        loop {
            let (mut stream, _) = tokio::select! {
                _ = &mut stop_rx => break,
                result = listener.accept() => result.expect("accept worker fetch abort request"),
            };
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker fetch abort request");
            if request.starts_with("GET /release?") {
                // The assertion finishes before this request. Keep the original
                // response incomplete until abort has rejected its Body promise.
                drop(response_stream.take());
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nreleased",
                    )
                    .await
                    .expect("write worker fetch release response");
            } else {
                assert!(request.starts_with("GET /stream?"), "{request}");
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 9\r\nConnection: close\r\n\r\nfirst",
                    )
                    .await
                    .expect("write incomplete worker fetch body");
                response_stream = Some(stream);
            }
        }
    });
    let script = format!(
        "{}\n{}",
        include_str!("../../../../../tests/fixtures/fetch-abort-reasons.js"),
        r#"
runFetchAbortReasonContract(location.href,'worker').then(result => {
  postMessage(result);close();
},error => {
  postMessage({checks:[{name:'fixture completion',passed:false,detail:String(error.stack || error)}]});
  close();
});
"#,
    );
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch abort loader");
    let mut handle = spawn_worker_with_request_client(
        script,
        format!("http://{address}/worker/main.js"),
        loader,
    );
    let result = timeout(TIMEOUT, recv_post_json(&mut handle)).await;
    handle.terminate_and_join();
    let _ = stop_tx.send(());
    server.await.expect("worker fetch abort server should stop");
    let facts: serde_json::Value =
        serde_json::from_str(&result.expect("worker fetch abort fixture should finish"))
            .expect("worker fetch abort results should be JSON");
    let checks = facts["checks"].as_array().expect("abort checks");
    let failures = checks
        .iter()
        .filter(|check| check["passed"] != true)
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(checks.len(), 486);
}
