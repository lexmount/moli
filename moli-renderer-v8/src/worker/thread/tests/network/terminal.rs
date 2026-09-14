use super::*;
use crate::runtime::RendererNetworkOutputItem;
use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};

#[tokio::test]
async fn accepted_fetch_response_streams_without_interception() {
    accepted_response(SubresourceResourceType::Fetch, false, false).await;
}

#[tokio::test]
async fn accepted_xhr_response_streams_without_interception() {
    accepted_response(SubresourceResourceType::Xhr, false, false).await;
}

#[tokio::test]
async fn accepted_fetch_auth_response_streams_before_eof() {
    accepted_response(SubresourceResourceType::Fetch, true, false).await;
}

#[tokio::test]
async fn accepted_xhr_auth_response_streams_before_eof() {
    accepted_response(SubresourceResourceType::Xhr, true, false).await;
}

#[tokio::test]
async fn accepted_fetch_auth_response_retains_partial_body() {
    accepted_response(SubresourceResourceType::Fetch, true, true).await;
}

#[tokio::test]
async fn accepted_xhr_auth_response_retains_partial_body() {
    accepted_response(SubresourceResourceType::Xhr, true, true).await;
}

async fn accepted_response(resource_type: SubresourceResourceType, auth: bool, partial: bool) {
    ensure_v8();
    let is_fetch = resource_type == SubresourceResourceType::Fetch;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (prefix_tx, prefix_rx) = tokio::sync::oneshot::channel();
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        for authenticated in if auth { vec![false, true] } else { vec![false] } {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut stream).await.unwrap();
            assert!(request.starts_with("POST /probe "));
            assert_eq!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: basic "),
                authenticated
            );
            let mut body = [0; 4];
            tokio::io::AsyncReadExt::read_exact(&mut stream, &mut body)
                .await
                .unwrap();
            assert_eq!(body, [0, 128, 255, 65]);
            if auth && !authenticated {
                let length = if is_fetch { 4 } else { 0 };
                stream.write_all(format!("HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"worker-stage\"\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                if is_fetch {
                    // The challenge body stays withheld. Providing credentials
                    // must close this transport before the retry can proceed.
                    let mut byte = [0];
                    match tokio::io::AsyncReadExt::read(&mut stream, &mut byte).await {
                        Ok(0) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                        other => panic!("challenge transport must close: {other:?}"),
                    }
                }
                continue;
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
            // Each gate requires a receipt from the exact request. No body byte
            // can make a buffered response look like a streaming response.
            if prefix_rx.await.is_err() {
                return;
            }
            stream.write_all(b"bo").await.unwrap();
            if tail_rx.await.is_err() {
                return;
            }
            if !partial {
                stream.write_all(b"dy").await.unwrap();
            }
            return;
        }
    });
    let script = if is_fetch {
        "onmessage=async()=>{try{const r=await fetch('/probe',{method:'POST',body:new Uint8Array([0,128,255,65])});postMessage('headers');postMessage(await r.text());}catch(e){postMessage('rejected');}close();};"
    } else {
        "onmessage=()=>{const x=new XMLHttpRequest();x.open('POST','/probe');x.onload=()=>{postMessage(x.responseText);close();};x.onerror=()=>{postMessage('rejected');close();};x.send(new Uint8Array([0,128,255,65]));};"
    };
    let mut worker = spawn_worker_with_request_client(
        script.into(),
        format!("{origin}/worker.js"),
        ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
    );
    worker.set_fetch_subresource_interception(auth, Some(resource_type));
    worker.post_message(serialize_test_string("go"));
    let mut prefix_tx = Some(prefix_tx);
    let mut tail_tx = Some(tail_tx);
    let mut handle = None;
    let mut heads = 0;
    let mut bytes = 0;
    let mut terminals = 0;
    let mut posts = Vec::new();
    timeout(TIMEOUT, async {
        while let Some(message) = worker.recv().await {
            match message {
                WorkerToParentMessage::Network(observation) => {
                    let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                        panic!("resource receipt")
                    };
                    match item.as_ref() {
                        ScriptNetworkOutputItem::SubresourceRequestStarted(start) => {
                            assert!(
                                handle.replace(start.handle()).is_none(),
                                "one admission across auth rounds"
                            );
                        }
                        ScriptNetworkOutputItem::SubresourceResponseStarted(head) => {
                            assert_eq!(Some(head.handle()), handle);
                            assert_eq!(
                                head.status(),
                                200,
                                "the challenge remains private to the auth decision"
                            );
                            assert_initial_worker_auth_network_headers(
                                head.network_request_headers(),
                            );
                            heads += 1;
                            assert_eq!(heads, 1);
                        }
                        ScriptNetworkOutputItem::SubresourceDataReceived(data) => {
                            assert_eq!(Some(data.handle()), handle);
                            assert_eq!(heads, 1);
                            assert_eq!(terminals, 0);
                            assert_eq!(data.data_length(), 2);
                            bytes += data.data_length();
                            if let Some(release) = tail_tx.take() {
                                release.send(()).unwrap();
                            }
                        }
                        ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) => {
                            assert_eq!(Some(terminal.handle()), handle);
                            assert_eq!(heads, 1);
                            terminals += 1;
                            assert_eq!(terminals, 1);
                            match terminal.result() {
                                SubresourceBodyFinishedResult::Ready(body) if !partial => {
                                    assert_eq!(body.clone_body_bytes(), b"body");
                                    assert!(terminal.data_was_streamed());
                                }
                                SubresourceBodyFinishedResult::FailedWithPartialBody {
                                    partial_body,
                                    ..
                                } if partial => {
                                    assert_eq!(partial_body.clone_body_bytes(), b"bo");
                                }
                                result => panic!("unexpected terminal: {result:?}"),
                            }
                        }
                        other => panic!("unexpected resource stage: {other:?}"),
                    }
                }
                WorkerToParentMessage::FetchInterception(pause) => {
                    assert!(auth);
                    match pause.stage() {
                        crate::runtime::RendererWorkerFetchStage::Request(_) => {
                            continue_worker_request(&pause, false, true).await
                        }
                        crate::runtime::RendererWorkerFetchStage::Auth(info) => {
                            assert_eq!(info.challenge.realm, "worker-stage");
                            assert_initial_worker_auth_network_headers(
                                info.network_request_headers.as_deref(),
                            );
                            decide_worker_pause(
                                &pause,
                                crate::runtime::WorkerFetchDecision::ProvideAuth(
                                    server_basic_auth_credentials(),
                                ),
                            )
                            .await;
                        }
                        other => panic!("accepted response must not pause: {other:?}"),
                    }
                }
                WorkerToParentMessage::Post(payload) => posts.push(stringify_payload(&payload)),
                other => panic!("unexpected Worker output: {other:?}"),
            }
            if heads == 1
                && (!is_fetch || posts.iter().any(|post| post == "\"headers\""))
                && let Some(release) = prefix_tx.take()
            {
                release.send(()).unwrap();
            }
        }
    })
    .await
    .expect("native head and fetch JS headers must arrive before body release");
    worker.terminate_and_join();
    server.await.unwrap();
    assert_eq!(
        (heads, bytes, terminals),
        (1, if partial { 2 } else { 4 }, 1)
    );
    let mut expected = if is_fetch {
        vec!["\"headers\""]
    } else {
        Vec::new()
    };
    expected.push(if partial { "\"rejected\"" } else { "\"body\"" });
    assert_eq!(posts, expected);
}

#[derive(Clone, Copy, Debug)]
enum HeldAuthAction {
    Cancel,
    Release,
    Fail,
    Fulfill,
    Abort,
    Retire,
    RetireKeepalive,
}

#[tokio::test]
async fn worker_auth_retirement_closes_the_challenge_transport() {
    held_auth_response(HeldAuthAction::Retire).await;
}

#[tokio::test]
async fn worker_auth_retirement_releases_the_keepalive_challenge() {
    held_auth_response(HeldAuthAction::RetireKeepalive).await;
}

#[tokio::test]
async fn worker_auth_cancel_streams_the_challenge_body() {
    held_auth_response(HeldAuthAction::Cancel).await;
}

#[tokio::test]
async fn worker_auth_release_streams_the_challenge_body() {
    held_auth_response(HeldAuthAction::Release).await;
}

#[tokio::test]
async fn worker_auth_failure_closes_the_challenge_transport() {
    held_auth_response(HeldAuthAction::Fail).await;
}

#[tokio::test]
async fn worker_auth_fulfillment_closes_the_challenge_transport() {
    held_auth_response(HeldAuthAction::Fulfill).await;
}

#[tokio::test]
async fn worker_auth_abort_closes_the_challenge_transport() {
    held_auth_response(HeldAuthAction::Abort).await;
}

async fn held_auth_response(action: HeldAuthAction) {
    use tokio::io::AsyncReadExt;
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (prefix_tx, prefix_rx) = tokio::sync::oneshot::channel();
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request_head(&mut socket).await.unwrap();
        assert!(request.starts_with("POST /probe "));
        let mut upload = [0; 4];
        socket.read_exact(&mut upload).await.unwrap();
        assert_eq!(upload, [0, 128, 255, 65]);
        socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"held-worker\"\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut byte = [0];
        tokio::select! {
            closed = socket.read(&mut byte) => {
                match closed {
                    Ok(0) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {},
                    other => panic!("challenge must close without another upload: {other:?}"),
                }
                return true;
            }
            ready = prefix_rx => ready.unwrap(),
        }
        socket.write_all(b"bo").await.unwrap();
        tail_rx.await.unwrap();
        socket.write_all(b"dy").await.unwrap();
        assert_eq!(
            socket.read(&mut byte).await.unwrap(),
            0,
            "completed transport closes after the whole challenge body"
        );
        false
    });
    // The surrounding Context survives Worker retirement, including its
    // transport executor. Retiring the test handle must not destroy it too.
    let client = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let mut worker = spawn_worker_with_request_client(
        "const controller=new AbortController();onmessage=async e=>{if(e.data==='abort'){controller.abort();return;}try{const r=await fetch('/probe',{method:'POST',body:new Uint8Array([0,128,255,65]),signal:controller.signal,keepalive:KEEPALIVE});postMessage('headers:'+r.status);postMessage(await r.text());}catch(e){postMessage('rejected');}close();};".replace("KEEPALIVE", if matches!(action, HeldAuthAction::RetireKeepalive) { "true" } else { "false" }),
        format!("{origin}/worker.js"),
        client.handle(),
    );
    worker.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    worker.post_message(serialize_test_string("go"));
    let mut records = WorkerNetworkRecords::default();
    let request = records.recv_pause(&mut worker).await;
    continue_worker_request(&request, false, true).await;
    let auth = records.recv_pause(&mut worker).await;
    let crate::runtime::RendererWorkerFetchStage::Auth(info) = auth.stage() else {
        panic!("challenge before body")
    };
    assert_eq!(info.challenge.realm, "held-worker");
    assert_eq!(auth.handle(), request.handle());
    use crate::runtime::WorkerFetchDecision;
    let decision = match action {
        HeldAuthAction::Retire | HeldAuthAction::RetireKeepalive => {
            worker.terminate_and_join();
            let keepalive = matches!(action, HeldAuthAction::RetireKeepalive);
            if keepalive {
                prefix_tx.send(()).unwrap();
                tail_tx.send(()).unwrap();
            }
            assert_eq!(timeout(TIMEOUT, server).await.unwrap().unwrap(), !keepalive);
            return;
        }
        HeldAuthAction::Cancel => Some(WorkerFetchDecision::CancelAuth),
        HeldAuthAction::Release => Some(WorkerFetchDecision::Release),
        HeldAuthAction::Fail => Some(WorkerFetchDecision::Fail("held auth rejected".into())),
        HeldAuthAction::Fulfill => Some(WorkerFetchDecision::Fulfill {
            response_code: 202,
            response_headers: vec![("content-type".into(), "text/plain".into())],
            response_body: crate::RendererSyntheticResponseBody::from_bytes(b"mock".to_vec()),
        }),
        HeldAuthAction::Abort => {
            worker.post_message(serialize_test_string("abort"));
            None
        }
    };
    if let Some(decision) = decision {
        decide_worker_pause(&auth, decision).await;
    }
    let resumed = matches!(action, HeldAuthAction::Cancel | HeldAuthAction::Release);
    let mut prefix_tx = Some(prefix_tx);
    let mut tail_tx = Some(tail_tx);
    let mut posts = Vec::new();
    let mut heads = 0;
    let mut bytes = 0;
    let mut terminals = 0;
    timeout(TIMEOUT, async {
        while let Some(message) = worker.recv().await {
            match message {
                WorkerToParentMessage::Post(payload) => posts.push(stringify_payload(&payload)),
                WorkerToParentMessage::Network(observation) => {
                    let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                        panic!("resource fact")
                    };
                    match item.as_ref() {
                        ScriptNetworkOutputItem::SubresourceResponseStarted(head) => {
                            assert_eq!(head.handle(), request.handle());
                            assert_eq!(
                                head.status(),
                                if matches!(action, HeldAuthAction::Fulfill) {
                                    202
                                } else {
                                    401
                                }
                            );
                            heads += 1;
                        }
                        ScriptNetworkOutputItem::SubresourceDataReceived(data) => {
                            assert_eq!(data.handle(), request.handle());
                            assert_eq!((heads, terminals), (1, 0));
                            assert_eq!(data.data_length(), 2);
                            bytes += data.data_length();
                            if let Some(release) = tail_tx.take() {
                                release.send(()).unwrap();
                            }
                        }
                        ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) => {
                            assert_eq!(terminal.handle(), request.handle());
                            terminals += 1;
                            match terminal.result() {
                                SubresourceBodyFinishedResult::Ready(body) => {
                                    assert!(resumed || matches!(action, HeldAuthAction::Fulfill));
                                    assert_eq!(
                                        body.clone_body_bytes(),
                                        if resumed { b"body" } else { b"mock" }
                                    );
                                    assert_eq!(terminal.data_was_streamed(), resumed);
                                }
                                SubresourceBodyFinishedResult::FailedWithPartialBody {
                                    partial_body,
                                    ..
                                } => {
                                    assert!(matches!(
                                        action,
                                        HeldAuthAction::Fail | HeldAuthAction::Abort
                                    ));
                                    assert!(partial_body.is_empty());
                                }
                                other => panic!("unexpected terminal for {action:?}: {other:?}"),
                            }
                        }
                        other => panic!("no repeated request admission: {other:?}"),
                    }
                }
                other => panic!("unexpected output for {action:?}: {other:?}"),
            }
            if resumed
                && heads == 1
                && posts.iter().any(|post| post == "\"headers:401\"")
                && let Some(release) = prefix_tx.take()
            {
                release.send(()).unwrap();
            }
        }
    })
    .await
    .expect("decision must settle or resume before the held body is released");
    worker.terminate_and_join();
    assert_eq!(timeout(TIMEOUT, server).await.unwrap().unwrap(), !resumed);
    assert_eq!(
        (heads, bytes, terminals),
        (1, if resumed { 4 } else { 0 }, 1)
    );
    assert_eq!(
        posts,
        if resumed {
            vec!["\"headers:401\"", "\"body\""]
        } else if matches!(action, HeldAuthAction::Fulfill) {
            vec!["\"headers:202\"", "\"mock\""]
        } else {
            vec!["\"rejected\""]
        }
    );
}

#[tokio::test]
async fn worker_body_cancellation_preserves_live_clones_and_closes_the_last_consumer() {
    use tokio::io::AsyncReadExt;
    ensure_v8();
    for cancel_last in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (prefix_tx, prefix_rx) = tokio::sync::oneshot::channel();
        let (tail_tx, tail_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut stream).await.unwrap();
            assert!(request.starts_with("GET /probe "));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            prefix_rx.await.unwrap();
            stream.write_all(b"bo").await.unwrap();
            let mut byte = [0];
            tokio::select! {
                result = stream.read(&mut byte) => {
                    assert!(cancel_last, "a surviving clone must retain its transport");
                    match result {
                        Ok(0) => {},
                        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {},
                        other => panic!("last consumer must close its transport: {other:?}"),
                    }
                }
                result = tail_rx => {
                    result.unwrap();
                    assert!(!cancel_last, "cancellation must precede the held tail");
                    stream.write_all(b"dy").await.unwrap();
                }
            }
        });
        let script = format!(
            r#"
            (async()=>{{
                const response = await fetch('/probe');
                const clone = response.clone();
                const canceled = response.body.cancel('first');
                postMessage('first-canceled');
                const reader = clone.body.getReader();
                const decoder = new TextDecoder();
                const prefix = decoder.decode((await reader.read()).value);
                onmessage = async()=>{{
                    if ({cancel_last}) {{
                        await reader.cancel('last');
                        await canceled;
                        postMessage('last-canceled');
                    }} else {{
                        let body = prefix;
                        for (;;) {{
                            const part = await reader.read();
                            if (part.done) break;
                            body += decoder.decode(part.value);
                        }}
                        postMessage(body);
                    }}
                }};
                postMessage(prefix);
            }})().catch(error=>postMessage(String(error)));
        "#
        );
        let mut worker = spawn_worker_with_request_client(
            script,
            format!("{origin}/worker.js"),
            ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
        );
        let mut prefix_tx = Some(prefix_tx);
        let mut tail_tx = Some(tail_tx);
        let mut terminal = false;
        let mut posts = Vec::new();
        timeout(TIMEOUT, async {
            while let Some(message) = worker.recv().await {
                match message {
                    WorkerToParentMessage::Post(payload) => {
                        let value = stringify_payload(&payload);
                        if value == "\"first-canceled\"" {
                            prefix_tx.take().unwrap().send(()).unwrap();
                        } else if value == "\"bo\"" {
                            worker.post_message(serialize_test_string("finish"));
                            if !cancel_last {
                                tail_tx.take().unwrap().send(()).unwrap();
                            }
                        }
                        posts.push(value);
                    }
                    WorkerToParentMessage::Network(observation) => {
                        if let RendererNetworkOutputItem::Resource(item) = observation.item()
                            && let ScriptNetworkOutputItem::SubresourceBodyFinished(finished) =
                                item.as_ref()
                        {
                            assert!(!terminal, "only one terminal result");
                            terminal = true;
                            match finished.result() {
                                SubresourceBodyFinishedResult::Ready(body) if !cancel_last => {
                                    assert_eq!(body.clone_body_bytes(), b"body")
                                }
                                SubresourceBodyFinishedResult::FailedWithPartialBody {
                                    partial_body,
                                    ..
                                } if cancel_last => {
                                    assert_eq!(partial_body.clone_body_bytes(), b"bo")
                                }
                                other => panic!("unexpected body result: {other:?}"),
                            }
                        }
                    }
                    other => panic!("unexpected Worker result: {other:?}"),
                }
                if terminal && posts.len() == 3 {
                    break;
                }
            }
            assert_eq!(
                posts,
                [
                    "\"first-canceled\"",
                    "\"bo\"",
                    if cancel_last {
                        "\"last-canceled\""
                    } else {
                        "\"body\""
                    }
                ]
            );
            assert!(terminal);
            server.await.unwrap();
        })
        .await
        .expect("body cancellation must release only the last consumer's transport");
        worker.terminate_and_join();
    }
}

#[tokio::test]
async fn intercepted_fetch_failure_retains_physical_head_and_prefix() {
    intercepted_failure(SubresourceResourceType::Fetch).await;
}

#[tokio::test]
async fn intercepted_xhr_failure_retains_physical_head_and_prefix() {
    intercepted_failure(SubresourceResourceType::Xhr).await;
}

async fn intercepted_failure(resource_type: SubresourceResourceType) {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (release_failure, failed_body) = tokio::sync::oneshot::channel();
    let mut release_failure = Some(release_failure);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_request_head(&mut stream).await.unwrap();
        assert!(request.starts_with("GET /partial "));
        stream.write_all(b"HTTP/1.1 200 OK\r\nX-Physical: retained\r\nContent-Length: 6\r\nConnection: close\r\n\r\n").await.unwrap();
        failed_body.await.unwrap();
        stream.write_all(b"pre").await.unwrap();
    });
    let script = match resource_type {
        SubresourceResourceType::Fetch => {
            "onmessage=async()=>{try{const r=await fetch('/partial');await r.text();postMessage('resolved');}catch(e){postMessage('rejected');}close();};"
        }
        SubresourceResourceType::Xhr => {
            "onmessage=()=>{const x=new XMLHttpRequest();x.open('GET','/partial');x.onload=()=>{postMessage('resolved');close();};x.onerror=()=>{postMessage('rejected');close();};x.send();};"
        }
        _ => unreachable!(),
    };
    let mut worker = spawn_worker_with_request_client(
        script.into(),
        format!("{origin}/worker.js"),
        ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
    );
    worker.set_fetch_subresource_interception(true, Some(resource_type));
    worker.post_message(serialize_test_string("go"));
    let mut items = Vec::new();
    let mut posts = Vec::new();
    timeout(TIMEOUT, async {
        while let Some(message) = worker.recv().await {
            match message {
                WorkerToParentMessage::Network(observation) => {
                    let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                        panic!("resource receipt")
                    };
                    items.push(item.clone());
                }
                WorkerToParentMessage::FetchInterception(pause) => match pause.stage() {
                    crate::runtime::RendererWorkerFetchStage::Request(_) => {
                        continue_worker_request(&pause, true, false).await;
                    }
                    crate::runtime::RendererWorkerFetchStage::Response(info) => {
                        assert_eq!(info.response_status, 200);
                        continue_worker_response(&pause, None, None).await;
                        release_failure
                            .take()
                            .expect("one response decision")
                            .send(())
                            .unwrap();
                    }
                    _ => panic!("unexpected response decision"),
                },
                WorkerToParentMessage::Post(payload) => posts.push(stringify_payload(&payload)),
                other => panic!("unexpected Worker output: {other:?}"),
            }
        }
    })
    .await
    .expect("Worker must settle the failed request and close");
    worker.terminate_and_join();
    server.await.unwrap();
    assert_eq!(posts, ["\"rejected\""]);
    assert_eq!(
        items.len(),
        4,
        "admission, physical head, actual prefix and one failed terminal: {items:?}"
    );
    let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = items[0].as_ref() else {
        panic!("request admission first")
    };
    let ScriptNetworkOutputItem::SubresourceResponseStarted(head) = items[1].as_ref() else {
        panic!("the failed stream must retain its physical head")
    };
    assert_eq!(head.handle(), start.handle());
    assert_eq!(head.status(), 200);
    assert!(
        head.response_headers()
            .iter()
            .any(|(name, value)| name == "x-physical" && value == "retained")
    );
    let ScriptNetworkOutputItem::SubresourceDataReceived(data) = items[2].as_ref() else {
        panic!("the accepted response reports its actual prefix before failure")
    };
    assert_eq!(data.handle(), start.handle());
    assert_eq!(data.data_length(), 3);
    let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) = items[3].as_ref() else {
        panic!("one terminal last")
    };
    assert_eq!(terminal.handle(), start.handle());
    let SubresourceBodyFinishedResult::FailedWithPartialBody { partial_body, .. } =
        terminal.result()
    else {
        panic!("the failed stream must retain its received bytes")
    };
    assert_eq!(partial_body.clone_body_bytes(), b"pre");
}
