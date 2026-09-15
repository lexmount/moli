// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn worker_fetch_uses_worker_script_base_url_and_resolves_response_text() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "hello from worker fetch".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const response = await fetch("./data.txt");
            postMessage({
                ok: response.ok,
                status: response.status,
                url: response.url,
                text: await response.text()
            });
            close();
        })().catch((error) => {
            postMessage({ error: String(error) });
            close();
        });
        "#
        .into(),
        script_url.clone(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"ok":true,"status":200,"url":"{base_url}/assets/data.txt","text":"hello from worker fetch"}}"#
        )
    );
    server.await.expect("worker fetch server should finish");
}

#[tokio::test]
async fn worker_fetch_resolves_response_before_delayed_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed worker fetch server");
    let addr = listener.local_addr().expect("delayed worker fetch addr");
    let (release_body_tx, release_body_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept delayed worker fetch request");
        let _request = read_http_request_head(&mut stream)
            .await
            .expect("read delayed worker fetch request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write delayed worker fetch headers");
        let _ = release_body_rx.await;
        stream
            .write_all(b"hello world")
            .await
            .expect("write delayed worker fetch body");
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const response = await fetch("./delayed.txt");
            postMessage({ phase: "headers", status: response.status });
            postMessage({ phase: "body", text: await response.text() });
            close();
        })().catch((error) => {
            postMessage({ phase: "error", error: String(error), name: error && error.name });
            close();
        });
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
    );

    let headers = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for headers-first worker fetch")
        .expect("worker channel closed");
    assert_eq!(
        expect_post_json(headers),
        r#"{"phase":"headers","status":200}"#
    );
    release_body_tx
        .send(())
        .expect("delayed body receiver should still be waiting");
    let body = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for delayed worker fetch body")
        .expect("worker channel closed");
    assert_eq!(
        expect_post_json(body),
        r#"{"phase":"body","text":"hello world"}"#
    );
    server
        .await
        .expect("delayed worker fetch server should finish");
}

#[tokio::test]
async fn worker_fetch_streams_spooled_response_body_chunks() {
    ensure_v8();
    let body = "x".repeat(1024 * 1024 + 17);
    let expected_len = body.len();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/large.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        body,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const response = await fetch("./large.txt");
            const reader = response.body.getReader();
            let chunks = 0;
            let total = 0;
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;
                chunks++;
                total += value.byteLength;
            }
            postMessage({ status: response.status, chunks, total });
            close();
        })().catch((error) => {
            postMessage({ error: String(error) });
            close();
        });
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let payload = expect_post_json(msg);
    let payload = serde_json::from_str::<serde_json::Value>(&payload)
        .expect("worker fetch stream result should be JSON");
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["total"].as_u64(), Some(expected_len as u64));
    let chunks = payload["chunks"]
        .as_u64()
        .expect("worker fetch stream result should include chunk count");
    assert!(
        chunks > 1,
        "large response should be observed through multiple stream chunks: {payload}"
    );
    server.await.expect("worker fetch server should finish");
}

#[tokio::test]
async fn worker_fetch_applies_network_policy_extra_http_headers() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker header server");
    let addr = listener.local_addr().expect("worker header server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker fetch request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker fetch request");
        let received = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            received.len(),
            received
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker header response");
    });
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        (async () => {
            const response = await fetch("./api");
            postMessage(await response.text());
            close();
        })().catch((error) => {
            postMessage(String(error));
            close();
        });
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
        WorkerNetworkPolicy {
            extra_http_headers: vec![("x-cdp-test".to_owned(), "works-worker".to_owned())].into(),
            ..WorkerNetworkPolicy::default()
        },
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""works-worker""#);
    server.await.expect("worker header server should finish");
}

#[tokio::test]
async fn worker_fetch_request_stage_interception_can_fulfill_synthetic_response() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("http://example.test/intercepted-worker-fetch");
                postMessage({
                    status: response.status,
                    header: response.headers.get("x-worker-intercept"),
                    text: await response.text(),
                });
            } catch (error) {
                postMessage({ error: String(error) });
            }
            close();
        };
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader"),
        WorkerNetworkPolicy {
            network_partition_key: Some("credentialless-worker-fetch".to_owned()),
            ..WorkerNetworkPolicy::default()
        },
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch pause, got {pending:?}");
    };
    assert!(pending.info.network_request_handle.is_none());
    assert_eq!(pending.info.resource_type, SubresourceResourceType::Fetch);
    assert_eq!(
        pending.info.url.as_str(),
        "http://example.test/intercepted-worker-fetch"
    );
    assert_eq!(
        pending.network_partition_key.as_deref(),
        Some("credentialless-worker-fetch")
    );

    let request = pending_worker_fetch_continue(pending.fetch_id, 17, &pending.info, false);
    handle.fulfill_pending_fetch(
        request,
        202,
        vec![
            ("content-type".to_owned(), b"text/plain".to_vec()),
            ("x-worker-intercept".to_owned(), b"request-stage".to_vec()),
        ],
        RendererSyntheticResponseBody::from_bytes(b"fulfilled-worker-fetch".to_vec()),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":202,"header":"request-stage","text":"fulfilled-worker-fetch"}"#
    );
}

#[tokio::test]
async fn worker_subresource_request_handles_are_owner_unique() {
    ensure_v8();

    fn spawn_intercepted_fetch_worker() -> WorkerTestHandle {
        spawn_worker_with_request_client_and_network_policy(
            r#"
            onmessage = async () => {
                try {
                    await fetch("http://example.test/intercepted-worker-fetch");
                    postMessage("done");
                } catch (error) {
                    postMessage({ error: String(error) });
                }
                close();
            };
            "#
            .into(),
            "http://example.test/worker/main.js".into(),
            ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader"),
            WorkerNetworkPolicy {
                network_partition_key: Some("credentialless-worker-fetch".to_owned()),
                ..WorkerNetworkPolicy::default()
            },
        )
    }

    let mut first = spawn_intercepted_fetch_worker();
    let mut second = spawn_intercepted_fetch_worker();
    first.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    second.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    first.post_message(serialize_test_string("go"));
    second.post_message(serialize_test_string("go"));

    let first_pending = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for first worker fetch pause")
        .expect("first worker channel closed");
    let second_pending = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for second worker fetch pause")
        .expect("second worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(first_pending) = first_pending else {
        panic!("expected first worker fetch pause, got {first_pending:?}");
    };
    let WorkerToParentMessage::PendingSubresourceFetch(second_pending) = second_pending else {
        panic!("expected second worker fetch pause, got {second_pending:?}");
    };

    assert!(
        first_pending.info.network_request_handle.is_none(),
        "worker pause should not allocate a local request handle"
    );
    assert!(
        second_pending.info.network_request_handle.is_none(),
        "worker pause should not allocate a local request handle"
    );

    let first_request =
        pending_worker_fetch_continue(first_pending.fetch_id, 101, &first_pending.info, false);
    let second_request =
        pending_worker_fetch_continue(second_pending.fetch_id, 202, &second_pending.info, false);
    let first_handle = first_request
        .network_request_handle
        .expect("first owner-assigned worker fetch handle");
    let second_handle = second_request
        .network_request_handle
        .expect("second owner-assigned worker fetch handle");
    assert_ne!(
        first_handle, second_handle,
        "worker-owned request handles must carry owner identity"
    );

    first.fulfill_pending_fetch(
        first_request,
        200,
        vec![("content-type".to_owned(), b"text/plain".to_vec())],
        RendererSyntheticResponseBody::from_bytes(b"first-worker-body".to_vec()),
    );
    second.fulfill_pending_fetch(
        second_request,
        200,
        vec![("content-type".to_owned(), b"text/plain".to_vec())],
        RendererSyntheticResponseBody::from_bytes(b"second-worker-body".to_vec()),
    );

    let first_network = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for first worker network record")
        .expect("first worker channel closed");
    let first_record = expect_subresource_network_record(first_network);
    assert_eq!(first_record.request_handle(), Some(first_handle));

    let second_network = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for second worker network record")
        .expect("second worker channel closed");
    let second_record = expect_subresource_network_record(second_network);
    assert_eq!(second_record.request_handle(), Some(second_handle));

    assert_eq!(recv_post_json(&mut first).await, r#""done""#);
    assert_eq!(recv_post_json(&mut second).await, r#""done""#);

    first.terminate_and_join();
    second.terminate_and_join();
}

#[tokio::test]
async fn worker_fetch_continue_request_resolves_response_before_delayed_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind continued worker fetch delayed server");
    let addr = listener
        .local_addr()
        .expect("continued worker fetch delayed addr");
    let (release_body_tx, release_body_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept continued worker fetch request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read continued worker fetch request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("continued worker fetch request path");
        assert_eq!(path, "/worker/delayed.txt");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write continued worker fetch headers");
        let _ = release_body_rx.await;
        stream
            .write_all(b"hello world")
            .await
            .expect("write continued worker fetch body");
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./delayed.txt");
                postMessage({ phase: "headers", status: response.status });
                postMessage({ phase: "body", text: await response.text() });
            } catch (error) {
                postMessage({ phase: "error", error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for continued worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected continued worker fetch pause, got {pending:?}");
    };
    let request = pending_worker_fetch_continue(pending.fetch_id, 61, &pending.info, false);
    handle.continue_pending_fetch(request);

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"headers","status":200}"#
    );
    release_body_tx
        .send(())
        .expect("continued worker fetch body release should be received");
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"body","text":"hello world"}"#
    );
    server
        .await
        .expect("continued worker fetch delayed server should finish");
}

#[tokio::test]
async fn worker_fetch_response_stage_interception_pauses_before_resolving_response() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/api.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "origin-worker-body".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./api.txt");
                postMessage({
                    status: response.status,
                    header: response.headers.get("x-worker-response-stage"),
                    text: await response.text(),
                });
            } catch (error) {
                postMessage({ error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch pause, got {pending:?}");
    };
    let request = pending_worker_fetch_continue(pending.fetch_id, 23, &pending.info, true);
    handle.continue_pending_fetch(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected worker response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 23);
    assert_eq!(info.response_status, 200);
    assert_eq!(
        info.response_body.try_bytes().unwrap().as_ref(),
        b"origin-worker-body"
    );

    handle.continue_pending_fetch_response(
        request,
        Some(203),
        Some(vec![
            ("content-type".to_owned(), b"text/plain".to_vec()),
            ("x-worker-response-stage".to_owned(), b"continued".to_vec()),
        ]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":203,"header":"continued","text":"origin-worker-body"}"#
    );
    server
        .await
        .expect("worker response-stage server should finish");
}

#[tokio::test]
async fn worker_fetch_response_stage_continue_preserves_large_spooled_body_stream() {
    ensure_v8();
    let body = "x".repeat(1024 * 1024 + 17);
    let expected_len = body.len();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/large-fetch.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        body,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./large-fetch.txt");
                const reader = response.body.getReader();
                let chunks = 0;
                let total = 0;
                while (true) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    chunks++;
                    total += value.byteLength;
                }
                postMessage({ status: response.status, chunks, total });
            } catch (error) {
                postMessage({ error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage large worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected large worker fetch pause, got {pending:?}");
    };
    let request = pending_worker_fetch_continue(pending.fetch_id, 71, &pending.info, true);
    handle.continue_pending_fetch(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage large worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected large worker fetch response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 71);
    assert_eq!(
        info.response_body
            .read_chunk(expected_len - 1, 1)
            .expect("spooled body tail should be readable"),
        b"x"
    );

    handle.continue_pending_fetch_response(request, None, None);

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(r#"{{"status":200,"chunks":17,"total":{expected_len}}}"#)
    );
    server
        .await
        .expect("large worker fetch response-stage server should finish");
}

#[tokio::test]
async fn worker_fetch_auth_required_then_continue_with_auth_resolves() {
    ensure_v8();
    let (base_url, server) = spawn_basic_auth_http_server(
        "/worker/fetch-auth.txt",
        "worker-fetch-area",
        "fetch-secret",
        2,
    )
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./fetch-auth.txt");
                postMessage({ ok: true, status: response.status, text: await response.text() });
            } catch (error) {
                postMessage({ ok: false, error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker fetch auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_fetch_continue(pending.fetch_id, 47, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_fetch(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker fetch auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 47);
    assert_eq!(info.resource_type, SubresourceResourceType::Fetch);
    assert_eq!(info.challenge.source, "Server");
    assert_eq!(info.challenge.scheme, "basic");
    assert_eq!(info.challenge.realm, "worker-fetch-area");
    assert!(!info.intercept_response);
    assert_initial_worker_auth_network_headers(info.network_request_headers.as_deref());

    let expected_request_handle = Some(owner_assigned_request_handle(47));
    request.auth = Some(server_basic_auth_credentials());
    handle.continue_pending_fetch(request);

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth-success network record")
        .expect("worker channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.request_handle(), expected_request_handle);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert_initial_worker_auth_network_headers(record.network_request_headers());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Success { status, .. } if *status == 200
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"ok":true,"status":200,"text":"fetch-secret"}"#
    );
    server
        .await
        .expect("worker fetch auth server should finish");
}

#[tokio::test]
async fn worker_fetch_auth_required_then_fail_rejects_without_exposing_challenge_body() {
    ensure_v8();
    let (base_url, server) = spawn_basic_auth_http_server(
        "/worker/fetch-auth.txt",
        "worker-fetch-area",
        "fetch-secret",
        1,
    )
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./fetch-auth.txt");
                postMessage({ ok: true, status: response.status, text: await response.text() });
            } catch (error) {
                postMessage({ ok: false, error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker fetch auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_fetch_continue(pending.fetch_id, 53, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_fetch(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker fetch auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 53);
    assert_eq!(info.challenge.realm, "worker-fetch-area");

    let expected_request_handle = Some(owner_assigned_request_handle(53));
    handle.fail_pending_fetch_auth(request, "worker fetch auth aborted".to_owned());

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth-fail network record")
        .expect("worker channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.request_handle(), expected_request_handle);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "worker fetch auth aborted"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"ok":false,"error":"TypeError: worker fetch auth aborted"}"#
    );
    server
        .await
        .expect("worker fetch auth-fail server should finish");
}

#[tokio::test]
async fn worker_importscripts_applies_loader_network_policy_extra_http_headers() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker importScripts header server");
    let addr = listener
        .local_addr()
        .expect("worker importScripts header server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker importScripts request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker importScripts request");
        let received = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let body = format!("postMessage({received:?}); close();");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker importScripts response");
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        importScripts("./imported.js");
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
        WorkerNetworkPolicy {
            extra_http_headers: vec![(
                "x-cdp-test".to_owned(),
                "works-worker-importscripts".to_owned(),
            )]
            .into(),
            ..WorkerNetworkPolicy::default()
        },
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""works-worker-importscripts""#);
    server
        .await
        .expect("worker importScripts header server should finish");
}

#[tokio::test]
async fn module_worker_dependency_applies_loader_network_policy_extra_http_headers() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind module worker dependency header server");
    let addr = listener
        .local_addr()
        .expect("module worker dependency header server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept module worker dependency request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read module worker dependency request");
        let received = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let body = format!("export default {received:?};");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write module worker dependency response");
    });
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("module worker dependency loader");
    let mut handle = spawn_worker_with_request_client_and_kind_and_network_policy(
        r#"
        import headerValue from "./dep.js";
        postMessage(headerValue);
        close();
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
        WorkerNetworkPolicy {
            extra_http_headers: vec![(
                "x-cdp-test".to_owned(),
                "works-module-worker-dependency".to_owned(),
            )]
            .into(),
            ..WorkerNetworkPolicy::default()
        },
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""works-module-worker-dependency""#);
    server
        .await
        .expect("module worker dependency header server should finish");
}

#[tokio::test]
async fn worker_fetch_missing_input_rejects_with_type_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            try {
                await fetch();
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    isTypeError: error instanceof TypeError
                });
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","isTypeError":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_bad_port_rejects_before_transport() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            try {
                await fetch("http://example.test:25/blocked-port");
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    hasBadPortMessage: String(error && error.message).includes("blocked bad port")
                });
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(record.url().as_str(), "http://example.test:25/blocked-port");
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("blocked bad port")
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","hasBadPortMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_file_url_rejects_before_interception_or_transport() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                await fetch("file:///moli-policy-must-not-open");
                postMessage({ fulfilled: true });
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    message: error && error.message,
                    isTypeError: error instanceof TypeError,
                });
            }
            close();
        };
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        match timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker file fetch rejection")
            .expect("worker channel closed")
        {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unsupported worker fetch must not reach interception: {other:?}"),
        }
    }

    let record = network.expect("worker file fetch should record a network failure");
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert_eq!(
        record.outcome(),
        &SubresourceNetworkOutcome::Failure {
            error_text: "URL scheme \"file\" is not supported.".to_owned(),
        }
    );
    assert_eq!(
        post.expect("worker file fetch should expose a TypeError"),
        r#"{"name":"TypeError","message":"URL scheme \"file\" is not supported.","isTypeError":true}"#
    );
}

#[tokio::test]
async fn worker_network_offline_fetch_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        (async () => {
            try {
                await fetch("http://example.test/offline/worker-fetch");
                postMessage("unexpected");
            } catch (error) {
                postMessage(String(error));
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        WorkerNetworkPolicy {
            network_offline: true,
            ..WorkerNetworkPolicy::default()
        },
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/offline/worker-fetch"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "Network emulation offline"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""TypeError: Network emulation offline""#
    );
}

#[tokio::test]
async fn worker_fetch_blocked_url_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        (async () => {
            try {
                await fetch("http://example.test/blocked/worker-fetch");
                postMessage("unexpected");
            } catch (error) {
                postMessage(String(error));
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        vec!["http://example.test/blocked/*".to_owned()],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/blocked/worker-fetch"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "net::ERR_BLOCKED_BY_CLIENT"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""TypeError: net::ERR_BLOCKED_BY_CLIENT""#
    );
}

#[tokio::test]
async fn worker_fetch_connection_refused_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let (base_url, server) =
        spawn_connection_drop_http_server("/worker-fetch-connection-refused").await;
    let url = format!("{base_url}/worker-fetch-connection-refused");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasMessage: String(error && error.message).length > 0,
                    stringStartsWithTypeError: String(error).startsWith("TypeError"),
                }});
            }}
            close();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for worker fetch DNS failure output (post={}, network={})",
                    post.is_some(),
                    network.is_some()
                )
            })
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("worker fetch connection failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
    ));
    assert_eq!(
        post.expect("worker fetch connection failure should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasMessage":true,"stringStartsWithTypeError":true}"#
    );
    server
        .await
        .expect("worker fetch connection-drop server should finish");
}

#[tokio::test]
async fn worker_fetch_dns_failure_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let url = "http://moli-dns-failure.invalid./worker-fetch-dns-failure";
    let url_literal = serde_json::to_string(url).expect("serialize worker fetch url");
    let loader =
        ResourceRequestClient::new(&dns_failure_fetch_config()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasMessage: String(error && error.message).length > 0,
                    stringStartsWithTypeError: String(error).startsWith("TypeError"),
                }});
            }}
            close();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("worker fetch DNS failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
        panic!(
            "expected worker fetch DNS failure, got {:?}",
            record.outcome()
        );
    };
    assert!(
        error_text.to_ascii_lowercase().contains("resolv"),
        "expected DNS-resolution error text, got {error_text:?}"
    );
    assert_eq!(
        post.expect("worker fetch DNS failure should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasMessage":true,"stringStartsWithTypeError":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_redirect_error_rejects_before_following_redirect() {
    ensure_v8();
    let (base_url, server) =
        spawn_single_redirect_http_server("/worker-fetch-redirect-error", "/target").await;
    let url = format!("{base_url}/worker-fetch-redirect-error");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal}, {{ redirect: "error" }});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasRedirectModeMessage: String(error && error.message).includes("redirect mode error"),
                }});
            }}
            close();
        }})();
        "#
        ),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    server
        .await
        .expect("worker fetch redirect-error server should finish");
    let record = network.expect("worker fetch redirect error should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("redirect mode error")
    ));
    assert_eq!(
        post.expect("worker fetch redirect error should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasRedirectModeMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_manual_redirect_returns_opaqueredirect_filtered_response() {
    ensure_v8();
    let (base_url, server) =
        spawn_single_redirect_http_server("/worker-fetch-redirect-manual", "/target").await;
    let url = format!("{base_url}/worker-fetch-redirect-manual");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            const response = await fetch({url_literal}, {{ redirect: "manual" }});
            const clone = response.clone();
            const bodyUsedBefore = response.bodyUsed;
            const text = await response.text();
            const cloneText = await clone.text();
            postMessage({{
                type: response.type,
                status: response.status,
                ok: response.ok,
                statusText: response.statusText,
                redirected: response.redirected,
                urlMatchesRequest: response.url === {url_literal},
                bodyIsNull: response.body === null,
                headers: Array.from(response.headers),
                bodyUsedBefore,
                bodyUsedAfter: response.bodyUsed,
                text,
                cloneType: clone.type,
                cloneStatus: clone.status,
                cloneUrlMatchesRequest: clone.url === {url_literal},
                cloneBodyIsNull: clone.body === null,
                cloneText,
            }});
            close();
        }})().catch((error) => {{
            postMessage({{ error: String(error), name: error && error.name }});
            close();
        }});
        "#
        ),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let post = recv_post_json(&mut handle).await;
    server
        .await
        .expect("worker fetch manual-redirect server should finish");
    assert_eq!(
        post,
        r#"{"type":"opaqueredirect","status":0,"ok":false,"statusText":"","redirected":false,"urlMatchesRequest":true,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":false,"text":"","cloneType":"opaqueredirect","cloneStatus":0,"cloneUrlMatchesRequest":true,"cloneBodyIsNull":true,"cloneText":""}"#
    );
}

#[tokio::test]
async fn worker_fetch_no_cors_cross_origin_returns_opaque_filtered_response() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors fetch server");
    let addr = listener.local_addr().expect("worker no-cors fetch addr");
    let fetch_url = format!("http://{addr}/worker-no-cors-data");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors fetch url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors fetch request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors fetch request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsecret worker",
            )
            .await
            .expect("write worker no-cors fetch response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
            const clone = response.clone();
            const bodyUsedBefore = response.bodyUsed;
            const text = await response.text();
            const cloneText = await clone.text();
            postMessage({{
                type: response.type,
                status: response.status,
                ok: response.ok,
                statusText: response.statusText,
                url: response.url,
                redirected: response.redirected,
                bodyIsNull: response.body === null,
                headers: Array.from(response.headers),
                bodyUsedBefore,
                bodyUsedAfter: response.bodyUsed,
                text,
                cloneType: clone.type,
                cloneStatus: clone.status,
                cloneBodyIsNull: clone.body === null,
                cloneText,
            }});
            close();
        }})().catch((error) => {{
            postMessage({{ error: String(error), name: error && error.name }});
            close();
        }});
        "#
        ),
        worker_script_url,
        loader,
    );

    let post = recv_post_json(&mut handle).await;
    let request = request_rx
        .await
        .expect("worker no-cors server should capture request");
    server
        .await
        .expect("worker no-cors fetch server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post,
        r#"{"type":"opaque","status":0,"ok":false,"statusText":"","url":"","redirected":false,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":false,"text":"","cloneType":"opaque","cloneStatus":0,"cloneBodyIsNull":true,"cloneText":""}"#
    );
}

#[tokio::test]
async fn worker_fetch_no_cors_opaque_response_blocking_returns_empty_opaque_response() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors ORB server");
    let addr = listener.local_addr().expect("worker no-cors ORB addr");
    let fetch_url = format!("http://{addr}/worker-orb-data");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors ORB url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors ORB request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors ORB request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"secret\":true}",
            )
            .await
            .expect("write worker no-cors ORB response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{
                    fulfilled: true,
                    type: response.type,
                    status: response.status,
                    url: response.url,
                    bodyIsNull: response.body === null,
                    headerCount: Array.from(response.headers).length,
                }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasOrbMessage: String(error && error.message).includes("OpaqueResponseBlocking"),
                }});
            }}
        }})();
        "#
        ),
        worker_script_url,
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let request = request_rx
        .await
        .expect("worker no-cors ORB server should capture request");
    server
        .await
        .expect("worker no-cors ORB server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post.expect("worker no-cors ORB should post response"),
        r#"{"fulfilled":true,"type":"opaque","status":0,"url":"","bodyIsNull":true,"headerCount":0}"#
    );
    let record = network.expect("worker no-cors ORB should record network failure");
    assert_eq!(record.url().as_str(), fetch_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == crate::network_host::ABORTED_ERROR_TEXT
    ));
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_fetch_no_cors_image_rejects_when_worker_policy_requires_coep_corp() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors COEP server");
    let addr = listener.local_addr().expect("worker no-cors COEP addr");
    let fetch_url = format!("http://{addr}/worker-coep-image");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors COEP url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors COEP request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors COEP request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsecret worker",
            )
            .await
            .expect("write worker no-cors COEP response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        (async () => {{
            try {{
                await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCoepMessage: String(error && error.message).includes("Cross-Origin-Embedder-Policy"),
                }});
            }}
            close();
        }})();
        "#
            ),
            worker_script_url,
        )
        .with_request_client(loader)
        .with_policy_context(crate::types::SubresourcePolicyContext {
            cross_origin_embedder_policy:
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp,
            ..Default::default()
        }),
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let request = request_rx
        .await
        .expect("worker no-cors COEP server should capture request");
    server
        .await
        .expect("worker no-cors COEP server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post.expect("worker no-cors COEP should post rejection"),
        r#"{"name":"TypeError","isTypeError":true,"hasCoepMessage":true}"#
    );
    let record = network.expect("worker no-cors COEP should record network failure");
    assert_eq!(record.url().as_str(), fetch_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Cross-Origin-Embedder-Policy")
    ));
}

#[tokio::test]
async fn worker_fetch_no_cors_orb_allows_mislabeled_png_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors ORB image server");
    let addr = listener
        .local_addr()
        .expect("worker no-cors ORB image addr");
    let fetch_url = format!("http://{addr}/worker-image-as-html");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors ORB image url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors ORB image request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors ORB image request");
        let _ = request_tx.send(request);
        let body = b"\x89PNG\r\n\x1A\nworker";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker no-cors ORB image headers");
        stream
            .write_all(body)
            .await
            .expect("write worker no-cors ORB image body");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{
                    type: response.type,
                    status: response.status,
                    bodyIsNull: response.body === null,
                }});
            }} catch (error) {{
                postMessage({{ rejected: true, message: String(error && error.message) }});
            }}
            close();
        }})();
        "#
        ),
        worker_script_url,
        loader,
    );

    let post = recv_post_json(&mut handle).await;
    let request = request_rx
        .await
        .expect("worker no-cors ORB image server should capture request");
    server
        .await
        .expect("worker no-cors ORB image server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(post, r#"{"type":"opaque","status":0,"bodyIsNull":true}"#);
}

#[tokio::test]
async fn worker_fetch_no_cors_cross_origin_resource_policy_blocks_response() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors CORP server");
    let addr = listener.local_addr().expect("worker no-cors CORP addr");
    let fetch_url = format!("http://{addr}/worker-corp-data");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors CORP url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors CORP request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors CORP request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nCross-Origin-Resource-Policy: same-origin\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsecret worker",
            )
            .await
            .expect("write worker no-cors CORP response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCorpMessage: String(error && error.message).includes("Cross-Origin-Resource-Policy"),
                }});
            }}
            close();
        }})();
        "#
        ),
        worker_script_url,
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let request = request_rx
        .await
        .expect("worker no-cors CORP server should capture request");
    server
        .await
        .expect("worker no-cors CORP server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post.expect("worker no-cors CORP should post rejection"),
        r#"{"name":"TypeError","isTypeError":true,"hasCorpMessage":true}"#
    );
    let record = network.expect("worker no-cors CORP should record network failure");
    assert_eq!(record.url().as_str(), fetch_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Cross-Origin-Resource-Policy")
    ));
}

#[tokio::test]
async fn worker_fetch_redirect_loop_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let (base_url, server) = spawn_redirect_loop_http_server("/worker-fetch-loop").await;
    let url = format!("{base_url}/worker-fetch-loop");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasRedirectLimitMessage: String(error && error.message).includes("redirect limit exceeded"),
                }});
            }}
            close();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    server
        .await
        .expect("worker fetch redirect-loop server should finish");
    let record = network.expect("worker fetch redirect loop should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("redirect limit exceeded")
    ));
    assert_eq!(
        post.expect("worker fetch redirect loop should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasRedirectLimitMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_cross_origin_redirect_without_cors_rejects_and_reports_failure() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_without_cors_http_servers(
            "/worker-fetch-cors-redirect-deny",
            "/worker-fetch-cors-denied-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-fetch-cors-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCorsMessage: String(error && error.message).includes("CORS check failed"),
                }});
            }}
            close();
        }})();
        "#
        ),
        format!("{source_base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker CORS redirect source server should finish");
    target_server
        .await
        .expect("worker CORS redirect target server should finish");
    let record = network.expect("worker fetch CORS redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
        panic!("expected CORS network failure, got {:?}", record.outcome());
    };
    assert_eq!(error_text, crate::network_host::FAILED_ERROR_TEXT);
    assert_eq!(
        post.expect("worker fetch CORS redirect should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasCorsMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_cross_origin_redirect_final_url_obeys_connect_src() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_with_cors_http_servers(
            "/worker-fetch-csp-redirect-deny",
            "/worker-fetch-csp-target",
            "worker-csp-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-fetch-csp-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        (async () => {{
            const events = [];
            addEventListener("securitypolicyviolation", event => {{
                events.push({{
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition,
                    sourceFile: event.sourceFile,
                    lineNumber: event.lineNumber,
                    columnNumber: event.columnNumber,
                }});
            }});
            try {{
                const response = await fetch({url_literal});
                postMessage({{ fulfilled: true, text: await response.text() }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCspMessage: String(error && error.message).includes("Content Security Policy"),
                    events,
                }});
            }}
            close();
        }})();
        "#
            ),
            format!("{source_base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_content_security_policies(vec!["connect-src 'self'".to_owned()]),
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker CSP redirect source server should finish");
    target_server
        .await
        .expect("worker CSP redirect target server should finish");
    let record = network.expect("worker fetch CSP redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Content Security Policy")
    ));
    assert_eq!(
        post.expect("worker fetch CSP redirect should post rejection surface"),
        format!(
            r#"{{"name":"TypeError","isTypeError":true,"hasCspMessage":true,"events":[{{"blockedURI":"{url}","effectiveDirective":"connect-src","disposition":"enforce","sourceFile":"","lineNumber":0,"columnNumber":0}}]}}"#
        )
    );
}

#[tokio::test]
async fn worker_blocked_url_pattern_update_reaches_running_worker() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                await fetch("http://example.test/blocked/live-worker-fetch");
                postMessage("unexpected");
            } catch (error) {
                postMessage(String(error));
            }
            close();
        };
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );

    handle.set_blocked_url_patterns(&["http://example.test/blocked/*".to_owned()]);
    handle.post_message(serialize_test_string("go"));

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/blocked/live-worker-fetch"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "net::ERR_BLOCKED_BY_CLIENT"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""TypeError: net::ERR_BLOCKED_BY_CLIENT""#
    );
}
