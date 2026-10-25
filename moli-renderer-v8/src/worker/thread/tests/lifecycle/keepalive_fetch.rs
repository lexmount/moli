use super::*;
use tokio::io::AsyncReadExt;

fn close_test_kind(shared: bool, script_url: &url::Url) -> WorkerGlobalKind {
    if shared {
        WorkerGlobalKind::Shared {
            name: "keepalive-close".to_owned(),
            storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(script_url, None),
        }
    } else {
        WorkerGlobalKind::Dedicated {
            name: "keepalive-close".to_owned(),
        }
    }
}

async fn wait_for_close(handle: &mut WorkerTestHandle) {
    timeout(TIMEOUT, async {
        while let Some(message) = handle.recv().await {
            if let WorkerToParentMessage::Error { message, .. } = message {
                panic!("worker failed before self.close(): {message}");
            }
        }
    })
    .await
    .expect("self.close() must retire the Worker before the response body is released");
}

async fn wait_for_detached_requests_to_finish(runtime: &crate::network::BrowserResourceRuntime) {
    timeout(TIMEOUT, async {
        while runtime.detached_keepalive_diagnostics().active_load_count != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("completed transport must release the detached keepalive record");
}

async fn assert_immediate_close_keepalive(shared: bool, post: bool, nested: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (request_tx, request_rx) = oneshot::channel();
    let (response_tx, response_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_request_with_body(&mut stream).await.unwrap();
        request_tx.send(request).unwrap();
        response_rx.await.unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone")
            .await
            .unwrap();
    });
    let script_url = url::Url::parse(&format!("{base}/worker.js")).unwrap();
    let target = format!("{base}/keepalive");
    let body = if post { "keepalive body" } else { "" };
    let init = if post {
        format!("{{keepalive: true, method: 'POST', body: {body:?}}}")
    } else {
        "{keepalive: true}".to_owned()
    };
    let source = if nested {
        let child = worker_data_url(&format!(
            "fetch({target:?}, {init}); postMessage('queued'); close();"
        ));
        format!("const child = new Worker({child:?}); child.onmessage = () => close();")
    } else {
        format!("fetch({target:?}, {init}); close();")
    };
    let client = worker_test_request_client();
    let runtime = client.browser_resource_runtime();
    let mut worker = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source, script_url.to_string())
            .with_request_client(client.handle())
            .with_global_kind(close_test_kind(shared, &script_url)),
    );
    wait_for_close(&mut worker).await;
    worker.terminate_and_join();

    let request = timeout(TIMEOUT, request_rx)
        .await
        .expect("keepalive must reach the server after the Worker has joined")
        .unwrap();
    let method = if post { "POST" } else { "GET" };
    assert!(request.starts_with(&format!("{method} /keepalive HTTP/1.1\r\n")));
    assert!(request.ends_with(&format!("\r\n\r\n{body}")));
    assert_eq!(
        runtime.detached_keepalive_diagnostics().active_load_count,
        1
    );
    response_tx.send(()).unwrap();
    timeout(TIMEOUT, server).await.unwrap().unwrap();
    wait_for_detached_requests_to_finish(&runtime).await;
}

#[tokio::test]
async fn worker_keepalive_fetch_survives_immediate_close() {
    ensure_v8();
    for shared in [false, true] {
        for post in [false, true] {
            assert_immediate_close_keepalive(shared, post, false).await;
        }
    }
}

#[tokio::test]
async fn nested_worker_keepalive_fetch_survives_both_workers_closing() {
    ensure_v8();
    assert_immediate_close_keepalive(false, true, true).await;
}

#[tokio::test]
async fn worker_close_preserves_keepalive_bodies_and_cancels_ordinary_or_aborted_fetches() {
    ensure_v8();
    for shared in [false, true] {
        for (keepalive, abort) in [(false, false), (true, false), (true, true)] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let (body_tx, body_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                assert!(request.starts_with("GET /stream HTTP/1.1\r\n"));
                let body = vec![b'x'; 1024 * 1024];
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                body_rx.await.unwrap();
                if keepalive && !abort {
                    stream.write_all(&body).await.unwrap();
                } else {
                    let mut byte = [0];
                    let result = timeout(TIMEOUT, stream.read(&mut byte)).await.expect(
                        "ordinary/explicitly aborted transport must close without its body",
                    );
                    match result {
                        Ok(0) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::ConnectionAborted
                            ) => {}
                        other => panic!("expected cancelled connection, got {other:?}"),
                    }
                }
            });
            let script_url = url::Url::parse(&format!("{base}/worker.js")).unwrap();
            let source = format!(
                "const controller = new AbortController(); fetch('./stream', {{keepalive: {keepalive}, signal: controller.signal}}).then(response => {{ if (response.status !== 200) throw new Error('bad status'); if ({abort}) controller.abort(); close(); }});"
            );
            let client = worker_test_request_client();
            let runtime = client.browser_resource_runtime();
            let mut worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(source, script_url.to_string())
                    .with_request_client(client.handle())
                    .with_global_kind(close_test_kind(shared, &script_url)),
            );
            wait_for_close(&mut worker).await;
            worker.terminate_and_join();
            if keepalive && !abort {
                assert_eq!(
                    runtime.detached_keepalive_diagnostics().active_load_count,
                    1
                );
            }
            body_tx.send(()).unwrap();
            timeout(TIMEOUT, server).await.unwrap().unwrap();
            wait_for_detached_requests_to_finish(&runtime).await;
        }
    }
}
