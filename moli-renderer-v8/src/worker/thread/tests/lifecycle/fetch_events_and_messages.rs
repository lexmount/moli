use super::*;

#[tokio::test]
async fn service_worker_fetch_handler_throw_without_respond_with_still_falls_back() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", () => {
            throw new Error("boom");
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(59);
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker fetch completion")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                assert!(
                    message.contains("boom"),
                    "unexpected error message: {message}"
                );
            }
            WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_) => {}
            other => panic!("unexpected message while waiting for fetch completion: {other:?}"),
        }
    };

    assert_eq!(completion.event_id, event_id);
    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Fallback
    ));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_materialized_response() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith(new Response("worker-body:" + event.request.url, {
                status: 201,
                statusText: "Created by worker",
                headers: {"x-worker": "yes"}
            }));
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 8).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.response_type, "default");
    assert_eq!(response.status, 201);
    assert_eq!(response.status_text, "Created by worker");
    assert_eq!(
        response.body,
        b"worker-body:https://example.test/app/data.txt".to_vec()
    );
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| name == "x-worker" && value == b"yes")
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_readable_stream_body_materializes() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const stream = new ReadableStream({
                start(controller) {
                    Promise.resolve()
                        .then(() => controller.enqueue(new Uint8Array([65, 66])))
                        .then(() => controller.enqueue(new Uint8Array([67])))
                        .then(() => controller.close());
                }
            });
            event.respondWith(new Response(stream, {
                status: 202,
                statusText: "Accepted Stream",
                headers: {"x-stream": "yes"}
            }));
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 18).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.response_type, "default");
    assert_eq!(response.status, 202);
    assert_eq!(response.status_text, "Accepted Stream");
    assert_eq!(response.body, b"ABC".to_vec());
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| name == "x-stream" && value == b"yes")
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_readable_stream_body_posts_stream_chunks() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const stream = new ReadableStream({
                start(controller) {
                    Promise.resolve()
                        .then(() => controller.enqueue(new Uint8Array([65, 66])))
                        .then(() => controller.enqueue(new Uint8Array([67])))
                        .then(() => controller.close());
                }
            });
            event.respondWith(new Response(stream, {
                status: 202,
                headers: {"x-stream": "yes"}
            }));
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(19);
    let run = crate::runtime::RendererServiceWorkerRunIdentity::fresh();
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });

    let mut body_source_id = None;
    let mut chunks = Vec::new();
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker fetch stream")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchStreamStarted(started) => {
                assert_eq!(started.event_id, event_id);
                assert_eq!(
                    started.owner.version_id(),
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(started.owner.run_identity(), &run);
                assert_eq!(started.response_head.response_type, "default");
                assert_eq!(started.response_head.status, 202);
                assert!(
                    started
                        .response_head
                        .headers
                        .iter()
                        .any(|(name, value)| name == "x-stream" && value == b"yes")
                );
                body_source_id = Some(started.body_source_id);
            }
            WorkerToParentMessage::ServiceWorkerFetchStreamChunk(chunk) => {
                let id = body_source_id.expect("stream chunks must follow stream start");
                assert_eq!(chunk.event_id, event_id);
                assert_eq!(chunk.body_source_id, id);
                chunks.extend(chunk.bytes);
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => {
                assert_eq!(completion.event_id, event_id);
                let ServiceWorkerFetchResult::Response(response) = completion.result else {
                    panic!("expected final service worker response");
                };
                assert_eq!(response.status, 202);
                assert_eq!(response.body, b"ABC".to_vec());
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for stream: {message}");
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
            | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerShowNotification(_)
            | WorkerToParentMessage::ServiceWorkerGetNotifications(_)
            | WorkerToParentMessage::ServiceWorkerSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_)
            | WorkerToParentMessage::ServiceWorkerCloseNotification(_)
            | WorkerToParentMessage::ServiceWorkerClientMessage(_)
            | WorkerToParentMessage::ServiceWorkerWorkerMessage(_)
            | WorkerToParentMessage::ServiceWorkerClientQuery(_)
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerClientFocus(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }

    assert_eq!(chunks, b"ABC".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_readable_stream_error_fails_open_stream() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const stream = new ReadableStream({
                start(controller) {
                    Promise.resolve()
                        .then(() => controller.enqueue(new Uint8Array([65])))
                        .then(() => controller.error(new Error("stream-broken")));
                }
            });
            event.respondWith(new Response(stream, {status: 206}));
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(21);
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });

    let mut body_source_id = None;
    let mut chunks = Vec::new();
    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker stream error")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchStreamStarted(started) => {
                assert_eq!(started.event_id, event_id);
                assert_eq!(started.response_head.status, 206);
                body_source_id = Some(started.body_source_id);
            }
            WorkerToParentMessage::ServiceWorkerFetchStreamChunk(chunk) => {
                let id = body_source_id.expect("stream chunks must follow stream start");
                assert_eq!(chunk.event_id, event_id);
                assert_eq!(chunk.body_source_id, id);
                chunks.extend(chunk.bytes);
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for stream error: {message}");
            }
            other => panic!("unexpected message while waiting for stream error: {other:?}"),
        }
    };

    assert!(
        body_source_id.is_some(),
        "stream must start before body read failure"
    );
    assert_eq!(chunks, b"A".to_vec());
    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected service worker fetch failure after stream error");
    };
    assert!(
        message.contains("failed to materialize Response body"),
        "unexpected failure message: {message}"
    );
    assert!(
        message.contains("stream-broken"),
        "unexpected failure message: {message}"
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_invalid_readable_stream_chunk_fails_open_stream() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const stream = new ReadableStream({
                start(controller) {
                    Promise.resolve()
                        .then(() => controller.enqueue(new Uint8Array([65])))
                        .then(() => controller.enqueue("not-bytes"));
                }
            });
            event.respondWith(new Response(stream, {status: 207}));
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(22);
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });

    let mut body_source_id = None;
    let mut chunks = Vec::new();
    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker invalid stream chunk")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchStreamStarted(started) => {
                assert_eq!(started.event_id, event_id);
                assert_eq!(started.response_head.status, 207);
                body_source_id = Some(started.body_source_id);
            }
            WorkerToParentMessage::ServiceWorkerFetchStreamChunk(chunk) => {
                let id = body_source_id.expect("stream chunks must follow stream start");
                assert_eq!(chunk.event_id, event_id);
                assert_eq!(chunk.body_source_id, id);
                chunks.extend(chunk.bytes);
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for invalid chunk: {message}"
                );
            }
            other => panic!("unexpected message while waiting for invalid chunk: {other:?}"),
        }
    };

    assert!(
        body_source_id.is_some(),
        "stream must start before invalid chunk"
    );
    assert_eq!(chunks, b"A".to_vec());
    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected service worker fetch failure after invalid stream chunk");
    };
    assert!(
        message.contains("failed to materialize Response body"),
        "unexpected failure message: {message}"
    );
    assert!(
        message.contains("ReadableStream body chunks must be Uint8Array"),
        "unexpected failure message: {message}"
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_stream_cancel_from_parent_notifies_source_and_fails_stream() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const stream = new ReadableStream({
                cancel(reason) {
                    console.log("stream-cancelled:" + String(reason));
                }
            });
            event.respondWith(new Response(stream, {status: 203}));
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(20);
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for service worker fetch stream start")
        .expect("service worker channel closed");
    let body_source_id = match message {
        WorkerToParentMessage::ServiceWorkerFetchStreamStarted(started) => {
            assert_eq!(started.event_id, event_id);
            started.body_source_id
        }
        WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
        | WorkerToParentMessage::ServiceWorkerFetchCompleted(_) => {
            panic!("stream body completed before stream start was reported");
        }
        WorkerToParentMessage::Error { message, .. } => {
            panic!("unexpected service worker error while waiting for stream start: {message}");
        }
        other => panic!("unexpected message while waiting for stream start: {other:?}"),
    };
    handle.cancel_service_worker_fetch_stream(event_id, body_source_id);

    let mut saw_cancel_notification = false;
    let mut saw_failure_completion = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for stream cancel notification")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::Console(message) => {
                assert_eq!(
                    message.message,
                    "log: stream-cancelled:The operation was aborted."
                );
                saw_cancel_notification = true;
                if saw_failure_completion {
                    break;
                }
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => {
                assert_eq!(completion.event_id, event_id);
                let ServiceWorkerFetchResult::Failure(message) = completion.result else {
                    panic!("expected stream cancel to fail service worker fetch completion");
                };
                assert_eq!(
                    message,
                    "FetchEvent.respondWith stream body was canceled: The operation was aborted."
                );
                saw_failure_completion = true;
                if saw_cancel_notification {
                    break;
                }
            }
            WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_) => {}
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for stream cancel: {message}"
                );
            }
            other => panic!("unexpected message while waiting for stream cancel: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_error_response_fails_fetch() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith(Response.error());
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 16).await;

    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected service worker fetch failure");
    };
    assert_eq!(
        message,
        "FetchEvent.respondWith rejected an error Response."
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_used_response_body_fails_fetch() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const response = new Response("used-body");
            response.text();
            event.respondWith(response);
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 17).await;

    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected service worker fetch failure");
    };
    assert_eq!(
        message,
        "FetchEvent.respondWith rejected a Response whose body is already used."
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_locked_response_body_fails_fetch() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const response = new Response(new ReadableStream({
                start(controller) {
                    controller.enqueue(new Uint8Array([65]));
                }
            }));
            globalThis.__lockedResponseReader = response.body.getReader();
            event.respondWith(response);
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 18).await;

    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected service worker fetch failure");
    };
    assert_eq!(
        message,
        "FetchEvent.respondWith rejected a Response whose body is locked."
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_request_exposes_destination_metadata() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith(new Response(JSON.stringify({
                destination: event.request.destination,
                mode: event.request.mode,
                credentials: event.request.credentials,
                redirect: event.request.redirect,
                cache: event.request.cache,
                referrer: event.request.referrer,
                referrerPolicy: event.request.referrerPolicy,
                integrity: event.request.integrity,
                keepalive: event.request.keepalive,
                isReload: event.isReload,
                requestIsReloadNavigation: event.request.isReloadNavigation,
                clientId: event.clientId,
                resultingClientId: event.resultingClientId,
                accept: event.request.headers.get("accept")
            })));
        });
        "#,
    );

    let request = ServiceWorkerFetchRequest {
        client_id: crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(7),
        resulting_client_id: None,
        url: url::Url::parse("https://example.test/app/script.js").unwrap(),
        method: "GET".to_owned(),
        headers: vec![("accept".to_owned(), "application/javascript".to_owned())],
        body: None,
        destination: ServiceWorkerRequestDestination::Script,
        request_mode: moli_fetch::RequestMode::NoCors,
        credentials_mode: moli_fetch::RequestCredentialsMode::Include,
        redirect_mode: moli_fetch::RequestRedirectMode::Error,
        priority: None,
        is_reload: true,
        metadata: ServiceWorkerFetchRequestMetadata {
            cache: "reload".to_owned(),
            referrer: "https://example.test/app/referrer.html".to_owned(),
            referrer_policy: "origin".to_owned(),
            integrity: "sha256-test".to_owned(),
            keepalive: true,
            use_cors_preflight: false,
        },
    };
    let completion =
        dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, 14, request).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        r#"{"destination":"script","mode":"no-cors","credentials":"include","redirect":"error","cache":"reload","referrer":"https://example.test/app/referrer.html","referrerPolicy":"origin","integrity":"sha256-test","keepalive":true,"isReload":true,"requestIsReloadNavigation":true,"clientId":"client-0000000000000007","resultingClientId":"","accept":"application/javascript"}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_exposes_resulting_client_metadata() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith(new Response(JSON.stringify({
                clientId: event.clientId,
                resultingClientId: event.resultingClientId,
                destination: event.request.destination
            })));
        });
        "#,
    );

    let client_id = crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(7);
    let resulting_client_id =
        crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(8);
    let request = ServiceWorkerFetchRequest {
        client_id,
        resulting_client_id: Some(resulting_client_id),
        url: url::Url::parse("https://example.test/app/").unwrap(),
        method: "GET".to_owned(),
        headers: Vec::new(),
        body: None,
        destination: ServiceWorkerRequestDestination::Document,
        request_mode: moli_fetch::RequestMode::Navigate,
        credentials_mode: moli_fetch::RequestCredentialsMode::Include,
        redirect_mode: moli_fetch::RequestRedirectMode::Manual,
        priority: None,
        is_reload: false,
        metadata: Default::default(),
    };
    let completion =
        dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, 15, request).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        r#"{"clientId":"client-0000000000000007","resultingClientId":"client-0000000000000008","destination":"document"}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_request_exposes_abort_signal_surface() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const request = event.request;
            const inherited = new Request(request);
            const clone = request.clone();
            const controller = new AbortController();
            const fromInit = new Request(request, { signal: controller.signal });
            const events = [];
            fromInit.signal.addEventListener("abort", () => events.push("fromInit"));
            controller.abort("sw-abort");
            event.respondWith(new Response(JSON.stringify({
                tag: Object.prototype.toString.call(request.signal),
                isEventTarget: request.signal instanceof EventTarget,
                aborted: request.signal.aborted,
                reasonType: typeof request.signal.reason,
                inheritedSignalDifferent: inherited.signal !== request.signal,
                cloneSignalDifferent: clone.signal !== request.signal,
                inheritedAborted: inherited.signal.aborted,
                cloneAborted: clone.signal.aborted,
                fromInitSignalDifferent: fromInit.signal !== controller.signal,
                fromInitAborted: fromInit.signal.aborted,
                fromInitReason: String(fromInit.signal.reason),
                events
            })));
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 19).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        r#"{"tag":"[object AbortSignal]","isEventTarget":true,"aborted":false,"reasonType":"undefined","inheritedSignalDifferent":true,"cloneSignalDifferent":true,"inheritedAborted":false,"cloneAborted":false,"fromInitSignalDifferent":true,"fromInitAborted":true,"fromInitReason":"sw-abort","events":["fromInit"]}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_request_signal_aborts_with_parent_reason() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const signal = event.request.signal;
            const events = [];
            event.respondWith(new Promise(resolve => {
                signal.addEventListener("abort", () => {
                    events.push("abort");
                    resolve(new Response(JSON.stringify({
                        aborted: signal.aborted,
                        reasonName: signal.reason && signal.reason.name,
                        reasonMessage: signal.reason && signal.reason.message,
                        events
                    })));
                });
            }));
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(27);
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });
    handle.abort_service_worker_fetch_request_signal(
        event_id,
        Some(serialize_test_value("new Error('caller-abort')")),
    );

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker fetch abort signal completion")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_) => {}
            other => {
                panic!("unexpected message while waiting for fetch abort signal: {other:?}");
            }
        }
    };

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response after request signal abort");
    };
    assert_eq!(completion.event_id, event_id);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        r#"{"aborted":true,"reasonName":"Error","reasonMessage":"caller-abort","events":["abort"]}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_request_body_used_guards_inherited_body() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith((async () => {
                const request = event.request;
                const probe = callback => {
                    try {
                        return String(callback());
                    } catch (error) {
                        return `throw:${error && error.name}`;
                    }
                };
                const cloneBefore = request.clone();
                const inheritedBefore = new Request(request);
                const locked = request.clone();
                locked.body.getReader();

                const lockedNew = probe(() => new Request(locked));
                const lockedClone = probe(() => locked.clone());
                const requestText = await request.text();
                const cloneText = await cloneBefore.text();
                const inheritedText = await inheritedBefore.text();
                const secondRead = await request.text().then(
                    value => `resolve:${value}`,
                    error => `reject:${error && error.name}`
                );

                return new Response(JSON.stringify({
                    bodyIsStream: request.body instanceof ReadableStream,
                    bodyUsedAfter: request.bodyUsed,
                    lockedNew,
                    lockedClone,
                    requestText,
                    cloneText,
                    inheritedText,
                    secondRead,
                    afterReplacement: probe(() => new Request(request, {
                        body: "replacement",
                        duplex: "half"
                    }).bodyUsed),
                    afterNew: probe(() => new Request(request)),
                    afterClone: probe(() => request.clone())
                }));
            })());
        });
        "#,
    );

    let request = ServiceWorkerFetchRequest {
        client_id: crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(7),
        resulting_client_id: None,
        url: url::Url::parse("https://example.test/app/post").unwrap(),
        method: "POST".to_owned(),
        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
        body: Some(b"fetch-body".to_vec()),
        destination: ServiceWorkerRequestDestination::Empty,
        request_mode: moli_fetch::RequestMode::Cors,
        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
        priority: None,
        is_reload: false,
        metadata: Default::default(),
    };
    let completion =
        dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, 20, request).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        r#"{"bodyIsStream":true,"bodyUsedAfter":true,"lockedNew":"throw:TypeError","lockedClone":"throw:TypeError","requestText":"fetch-body","cloneText":"fetch-body","inheritedText":"fetch-body","secondRead":"reject:TypeError","afterReplacement":"false","afterNew":"throw:TypeError","afterClone":"throw:TypeError"}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_uses_native_promise_adoption() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            Promise.resolve = () => {
                throw new Error("patched Promise.resolve should not run");
            };
            event.respondWith(new Response("native-adoption"));
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 12).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"native-adoption".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_uses_native_promise_reactions() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const promise = new Promise(resolve => resolve(new Response("native-reaction")));
            promise.then = () => {
                throw new Error("patched promise.then should not run");
            };
            event.respondWith(promise);
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 13).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"native-reaction".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_stops_fetch_event_propagation() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        const order = [];
        self.addEventListener("fetch", event => {
            order.push("first");
            event.respondWith(Promise.resolve().then(() => new Response(order.join(","))));
        });
        self.addEventListener("fetch", () => {
            order.push("second");
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 24).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"first".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_keeps_response_when_handler_throws_afterward() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith(Promise.resolve(new Response("intercepted")));
            throw new Error("after-respond");
        });
        "#,
    );
    let event_id = ServiceWorkerEventId::from_u64_for_worker(25);
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request: service_worker_fetch_request_for_test(),
        navigation_preload_sent: false,
    });

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker fetch completion")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                assert!(
                    message.contains("after-respond"),
                    "unexpected error message: {message}"
                );
            }
            WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_) => {}
            other => panic!("unexpected message while waiting for fetch completion: {other:?}"),
        }
    };

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected service worker response");
    };
    assert_eq!(completion.event_id, event_id);
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"intercepted".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_twice_rejects_second_call() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            let secondResult = "unset";
            event.respondWith(Promise.resolve().then(() => new Response(secondResult)));
            try {
                event.respondWith(new Response("second"));
                secondResult = "resolved";
            } catch (error) {
                secondResult = error && error.name;
            }
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 26).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected first respondWith response");
    };
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"InvalidStateError".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_in_microtask_uses_response() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            Promise.resolve().then(() => {
                event.respondWith(new Response("microtask-response", {status: 217}));
            });
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 27).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected microtask respondWith response");
    };
    assert_eq!(response.status, 217);
    assert_eq!(response.body, b"microtask-response".to_vec());
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_in_task_throws_invalid_state() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            setTimeout(() => {
                let result = "unset";
                try {
                    event.respondWith(new Response("task-response"));
                    result = "resolved";
                } catch (error) {
                    result = (error && error.name) + ":" + (error instanceof DOMException);
                }
                console.log("task:" + result);
            }, 0);
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 28).await;
    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Fallback
    ));

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for task respondWith result")
        .expect("service worker channel closed");
    let WorkerToParentMessage::Console(message) = message else {
        panic!("unexpected message while waiting for task respondWith result: {message:?}");
    };
    assert_eq!(message.message, "log: task:InvalidStateError:true");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_event_receives_structured_data() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            const eventShape = {
                constructor: event.constructor.name,
                instance: event instanceof ExtendableMessageEvent,
                extendable: event instanceof ExtendableEvent,
                eventBase: event instanceof Event,
                tag: Object.prototype.toString.call(event),
                lastEventId: event.lastEventId
            };
            const expectedShape = {
                constructor: "ExtendableMessageEvent",
                instance: true,
                extendable: true,
                eventBase: true,
                tag: "[object ExtendableMessageEvent]",
                lastEventId: ""
            };
            if (JSON.stringify(eventShape) !== JSON.stringify(expectedShape)) {
                throw new Error("unexpected event shape:" + JSON.stringify(eventShape));
            }
            if (event.type !== "message") {
                throw new Error("unexpected type:" + event.type);
            }
            if (event.data.text !== "ping" || event.data.count !== 3) {
                throw new Error("unexpected data:" + JSON.stringify(event.data));
            }
            if (event.source !== null) {
                throw new Error("unexpected source");
            }
            if (event.origin !== "") {
                throw new Error("unexpected origin:" + event.origin);
            }
            if (event.ports.length !== 0) {
                throw new Error("unexpected ports");
            }
            if (typeof event.waitUntil !== "function") {
                throw new Error("missing waitUntil");
            }
        });
        "#,
    );

    let completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        20,
        serialize_test_value(r#"({ text: "ping", count: 3 })"#),
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(20)
    );
    assert_eq!(
        completion.owner.version_id(),
        ServiceWorkerVersionId::from_u64_for_test(1)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_extendable_message_event_constructor_surface() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", () => {
            const channel = new MessageChannel();
            const dataObject = { value: 7 };
            const defaultEvent = new ExtendableMessageEvent("default");
            const initialized = new ExtendableMessageEvent("custom", {
                bubbles: true,
                cancelable: true,
                composed: true,
                data: dataObject,
                origin: null,
                lastEventId: 123,
                source: channel.port1,
                ports: [channel.port1, channel.port2]
            });
            let invalidSource = "";
            try {
                new ExtendableMessageEvent("bad", { source: self });
            } catch (error) {
                invalidSource = error && error.name;
            }
            let invalidPorts = "";
            try {
                new ExtendableMessageEvent("bad", { ports: [1] });
            } catch (error) {
                invalidPorts = error && error.name;
            }
            let missingNew = "";
            try {
                ExtendableMessageEvent("bad");
            } catch (error) {
                missingNew = error && error.name;
            }
            let waitUntil = "";
            try {
                defaultEvent.waitUntil(Promise.resolve());
            } catch (error) {
                waitUntil = (error && error.name) + ":" + (error instanceof DOMException);
            }

            const actual = {
                constructorType: typeof ExtendableMessageEvent,
                constructorName: ExtendableMessageEvent.name,
                constructorLength: ExtendableMessageEvent.length,
                prototypeParent: Object.getPrototypeOf(ExtendableMessageEvent.prototype) === ExtendableEvent.prototype,
                constructorParent: Object.getPrototypeOf(ExtendableMessageEvent) === ExtendableEvent,
                defaultEvent: {
                    type: defaultEvent.type,
                    bubbles: defaultEvent.bubbles,
                    cancelable: defaultEvent.cancelable,
                    data: defaultEvent.data,
                    origin: defaultEvent.origin,
                    lastEventId: defaultEvent.lastEventId,
                    source: defaultEvent.source,
                    portsLength: defaultEvent.ports.length,
                    portsFrozen: Object.isFrozen(defaultEvent.ports),
                    instance: defaultEvent instanceof ExtendableMessageEvent,
                    extendable: defaultEvent instanceof ExtendableEvent,
                    eventBase: defaultEvent instanceof Event,
                    tag: Object.prototype.toString.call(defaultEvent)
                },
                initialized: {
                    type: initialized.type,
                    bubbles: initialized.bubbles,
                    cancelable: initialized.cancelable,
                    composed: initialized.composed,
                    dataSame: initialized.data === dataObject,
                    origin: initialized.origin,
                    lastEventId: initialized.lastEventId,
                    sourceSame: initialized.source === channel.port1,
                    portsLength: initialized.ports.length,
                    port0Same: initialized.ports[0] === channel.port1,
                    port1Same: initialized.ports[1] === channel.port2,
                    portsFrozen: Object.isFrozen(initialized.ports)
                },
                errors: {
                    invalidSource,
                    invalidPorts,
                    missingNew,
                    waitUntil
                }
            };
            const expected = {
                constructorType: "function",
                constructorName: "ExtendableMessageEvent",
                constructorLength: 1,
                prototypeParent: true,
                constructorParent: true,
                defaultEvent: {
                    type: "default",
                    bubbles: false,
                    cancelable: false,
                    data: null,
                    origin: "",
                    lastEventId: "",
                    source: null,
                    portsLength: 0,
                    portsFrozen: true,
                    instance: true,
                    extendable: true,
                    eventBase: true,
                    tag: "[object ExtendableMessageEvent]"
                },
                initialized: {
                    type: "custom",
                    bubbles: true,
                    cancelable: true,
                    composed: true,
                    dataSame: true,
                    origin: "null",
                    lastEventId: "123",
                    sourceSame: true,
                    portsLength: 2,
                    port0Same: true,
                    port1Same: true,
                    portsFrozen: true
                },
                errors: {
                    invalidSource: "TypeError",
                    invalidPorts: "TypeError",
                    missingNew: "TypeError",
                    waitUntil: "InvalidStateError:true"
                }
            };
            if (JSON.stringify(actual) !== JSON.stringify(expected)) {
                throw new Error("unexpected constructor surface:" + JSON.stringify(actual));
            }
        });
        "#,
    );

    let completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        29,
        serialize_test_string("go"),
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(29)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_script_created_extendable_events_support_dispatch() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", () => {
            for (const Constructor of [ExtendableEvent, ExtendableMessageEvent]) {
                const event = new Constructor(Constructor.name, {
                    bubbles: true, cancelable: true, composed: true
                });
                let calls = 0;
                self.addEventListener(Constructor.name, received => {
                    calls++;
                    if (received !== event || received.isTrusted ||
                        received.target !== self || received.currentTarget !== self ||
                        received.eventPhase !== Event.AT_TARGET ||
                        received.composedPath()[0] !== self) {
                        throw new Error("incorrect dispatched event state");
                    }
                    for (const action of [
                        () => self.dispatchEvent(received),
                        () => received.waitUntil(Promise.resolve())
                    ]) {
                        let errorName;
                        try { action(); } catch (error) { errorName = error.name; }
                        if (errorName !== "InvalidStateError") {
                            throw new Error("expected InvalidStateError, got " + errorName);
                        }
                    }
                    received.preventDefault();
                });
                for (let attempt = 1; attempt <= 2; attempt++) {
                    const result = self.dispatchEvent(event);
                    if (result !== false || calls !== attempt || !event.defaultPrevented ||
                        !event.bubbles || !event.composed || event.isTrusted ||
                        event.currentTarget !== null || event.eventPhase !== Event.NONE ||
                        event.composedPath().length !== 0) {
                        throw new Error("incorrect completed event state");
                    }
                }
            }
            let fakeError;
            try { self.dispatchEvent({type: "fake"}); }
            catch (error) { fakeError = error.name; }
            if (fakeError !== "TypeError") {
                throw new Error("dispatchEvent accepted a non-Event");
            }
        });
        "#,
    );

    let completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        30,
        serialize_test_string("go"),
    )
    .await;
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_event_source_uses_client_snapshot() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            const source = event.source;
            if (!source) {
                throw new Error("missing source");
            }
            const actual = {
                eventConstructor: event.constructor.name,
                eventInstance: event instanceof ExtendableMessageEvent,
                type: event.type,
                data: event.data,
                origin: event.origin,
                sourceConstructor: source.constructor.name,
                sourceWindowClient: source instanceof WindowClient,
                sourceClient: source instanceof Client,
                id: source.id,
                url: source.url,
                clientType: source.type,
                frameType: source.frameType,
                lifecycleState: source.lifecycleState,
                visibilityState: source.visibilityState,
                focused: source.focused,
                postMessage: typeof source.postMessage,
                focus: typeof source.focus,
                navigate: typeof source.navigate
            };
            const expected = {
                eventConstructor: "ExtendableMessageEvent",
                eventInstance: true,
                type: "message",
                data: "ping",
                origin: "https://example.test",
                sourceConstructor: "WindowClient",
                sourceWindowClient: true,
                sourceClient: true,
                id: "client-000000000000002a",
                url: "https://example.test/app/page.html",
                clientType: "window",
                frameType: "top-level",
                lifecycleState: "active",
                visibilityState: "visible",
                focused: true,
                postMessage: "function",
                focus: "function",
                navigate: "function"
            };
            if (JSON.stringify(actual) !== JSON.stringify(expected)) {
                throw new Error("unexpected source snapshot:" + JSON.stringify(actual));
            }
        });
        "#,
    );
    let source_client_id = crate::runtime::ServiceWorkerClientId::from_u64_for_test(42);
    let source_client_url = url::Url::parse("https://example.test/app/page.html").unwrap();
    let completion = dispatch_service_worker_message_event_object_for_test(
        &mut handle,
        ServiceWorkerMessageEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(21),
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            ),
            source_client_id: Some(source_client_id),
            source_client_url: Some(source_client_url.clone()),
            source_client_snapshot: Some(
                crate::runtime::ServiceWorkerClientSnapshot::focused_window_for_test(
                    source_client_id,
                    source_client_url,
                    true,
                ),
            ),
            source_worker: None,
            source_origin: "https://example.test".to_owned(),
            payload: serialize_test_string("ping"),
            window_interaction_allowed: false,
        },
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(21)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_event_source_uses_service_worker_snapshot() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            const source = event.source;
            if (!source) {
                throw new Error("missing source");
            }
            const actual = {
                eventConstructor: event.constructor.name,
                eventInstance: event instanceof ExtendableMessageEvent,
                type: event.type,
                data: event.data,
                origin: event.origin,
                constructor: source.constructor.name,
                sourceServiceWorker: source instanceof ServiceWorker,
                scriptURL: source.scriptURL,
                state: source.state,
                postMessage: typeof source.postMessage
            };
            const expected = {
                eventConstructor: "ExtendableMessageEvent",
                eventInstance: true,
                type: "message",
                data: "ping",
                origin: "https://example.test",
                constructor: "ServiceWorker",
                sourceServiceWorker: true,
                scriptURL: "https://example.test/app/source-sw.js",
                state: "activated",
                postMessage: "function"
            };
            if (JSON.stringify(actual) !== JSON.stringify(expected)) {
                throw new Error("unexpected service worker source:" + JSON.stringify(actual));
            }
        });
        "#,
    );
    let completion = dispatch_service_worker_message_event_object_for_test(
        &mut handle,
        ServiceWorkerMessageEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(22),
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            ),
            source_client_id: None,
            source_client_url: None,
            source_client_snapshot: None,
            source_worker: Some(
                crate::service_worker_runtime::ServiceWorkerVersionSnapshot::new(
                    ServiceWorkerVersionId::from_u64_for_test(2),
                    url::Url::parse("https://example.test/app/source-sw.js").unwrap(),
                    "activated",
                ),
            ),
            source_origin: "https://example.test".to_owned(),
            payload: serialize_test_string("ping"),
            window_interaction_allowed: false,
        },
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(22)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_opaque_headers_precede_orb_validation_and_cache_preserves_checked_body() {
    ensure_v8();
    for (mime, bytes, expected) in [
        (
            "application/json",
            &b"globalThis.value = 1;"[..],
            &b"globalThis.value = 1;"[..],
        ),
        ("application/json", &b"{\"secret\":true}"[..], &b""[..]),
        (
            "text/html",
            &b"\x89PNG\r\n\x1a\nimage data"[..],
            &b"\x89PNG\r\n\x1a\nimage data"[..],
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fetch_url = format!("http://{}/body", listener.local_addr().unwrap());
        let (release, released) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            stream.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len(),
            ).as_bytes()).await.unwrap();
            stream.write_all(&bytes[..bytes.len() - 1]).await.unwrap();
            if timeout(TIMEOUT, released)
                .await
                .is_ok_and(|result| result.is_ok())
            {
                stream.write_all(&bytes[bytes.len() - 1..]).await.unwrap();
            }
        });
        let source = format!(
            r#"
            addEventListener('fetch', event => {{
              event.respondWith((async () => {{
                const response = await fetch({}, {{mode: 'no-cors'}});
                const clone = response.clone();
                const cacheName = event.request.url;
                const cache = await caches.open(cacheName);
                let settled = false;
                const write = cache.put(event.request, clone).then(() => settled = true);
                await new Promise(resolve => setTimeout(resolve, 30));
                console.log(JSON.stringify({{type: response.type, bodyNull: response.body === null,
                  cloneNull: clone.body === null, settled}}));
                await write;
                const cached = await cache.match(event.request);
                await caches.delete(cacheName);
                return cached;
              }})());
            }});
        "#,
            serde_json::to_string(&fetch_url).unwrap()
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, "https://example.test/app/sw.js".to_owned())
                .with_request_client(loader)
                .with_global_kind(crate::worker::WorkerGlobalKind::Service {
                    registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                    version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                    scope_url: url::Url::parse("https://example.test/app/").unwrap(),
                }),
        );
        let mut request = service_worker_fetch_request_for_test();
        request.request_mode = moli_fetch::RequestMode::NoCors;
        request.destination = ServiceWorkerRequestDestination::Script;
        handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(31),
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            ),
            request,
            navigation_preload_sent: false,
        });
        let early = loop {
            match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
                WorkerToParentMessage::Console(message) => break message.message,
                WorkerToParentMessage::SubresourceNetwork(_)
                | WorkerToParentMessage::SubresourceContinue(_) => {}
                other => panic!("expected opaque headers before EOF for {mime}: {other:?}"),
            }
        };
        let early: serde_json::Value = serde_json::from_str(
            early
                .strip_prefix("log: ")
                .unwrap_or_else(|| panic!("unexpected opaque response console message: {early}")),
        )
        .unwrap_or_else(|error| panic!("invalid opaque response diagnostic {early}: {error}"));
        assert_eq!(early["type"], "opaque", "{mime}");
        assert_eq!(early["bodyNull"], true, "{mime}");
        assert_eq!(early["cloneNull"], true, "{mime}");
        if !expected.is_empty() {
            assert_eq!(
                early["settled"], false,
                "allowed body ended before its last byte"
            );
        }
        release.send(()).unwrap();
        let response = loop {
            match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
                WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => {
                    match completion.result {
                        ServiceWorkerFetchResult::Response(response) => break response,
                        other => panic!("expected cached opaque response for {mime}: {other:?}"),
                    }
                }
                WorkerToParentMessage::Error { message, .. } => panic!("{mime}: {message}"),
                _ => {}
            }
        };
        assert_eq!(response.response_type, "opaque", "{mime}");
        assert_eq!(response.body, expected, "{mime}");
        server.await.unwrap();
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn service_worker_fetch_respond_with_body_accessed_opaque_response_keeps_internal_head_and_body()
 {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker opaque body server");
    let addr = listener
        .local_addr()
        .expect("service worker opaque body server addr");
    let fetch_url = format!("http://{addr}/app/respond-with-body-accessed-response.jsonp");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize opaque body fetch URL");
    let server = tokio::spawn(async move {
        for _ in 0..6 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept service worker opaque body request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker opaque body request");
            assert!(
                request
                    .starts_with("GET /app/respond-with-body-accessed-response.jsonp HTTP/1.1\r\n")
            );
            assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCross-Origin-Resource-Policy: cross-origin\r\nVary: *\r\nSet-Cookie: hidden=secret\r\nContent-Length: 15\r\nConnection: close\r\n\r\ncallback('OK');",
                )
                .await
                .expect("write service worker opaque body response");
        }
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("service worker fetch loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
                function assertOpaqueResponse(response, label) {{
                  response.body;
                  if (response.type !== "opaque" || response.status !== 0 ||
                      response.body !== null || response.bodyUsed ||
                      response.statusText !== "" || response.url !== "" ||
                      [...response.headers].length !== 0) {{
                    throw new Error(label + ":" + [
                      response.type,
                      response.status,
                      response.body === null,
                      response.bodyUsed
                    ].join("/"));
                  }}
                }}

                function maybeClone(response, cloneMode) {{
                  if (cloneMode === "clone-response") {{
                    const clone = response.clone();
                    assertOpaqueResponse(clone, "clone-response");
                    return clone;
                  }}
                  if (cloneMode === "clone-unused") {{
                    const unused = response.clone();
                    assertOpaqueResponse(unused, "clone-unused");
                  }}
                  return response;
                }}

                async function passThroughCacheIfNeeded(event, response, cacheMode) {{
                  if (cacheMode !== "pass-through") {{
                    return response;
                  }}
                  const cacheName = event.request.url;
                  await self.caches.delete(cacheName);
                  const cache = await self.caches.open(cacheName);
                  await cache.put(event.request, response);
                  const cached = await cache.match(event.request.url);
                  assertOpaqueResponse(cached, "cached");
                  await self.caches.delete(cacheName);
                  return cached;
                }}

                self.addEventListener("fetch", event => {{
                  const url = new URL(event.request.url);
                  const cloneMode = url.searchParams.get("clone");
                  const cacheMode = url.searchParams.get("cache");
                  event.respondWith(fetch({fetch_url_literal}, {{ mode: "no-cors" }})
                    .then(async response => {{
                      assertOpaqueResponse(response, "original");
                      const selected = maybeClone(response, cloneMode);
                      assertOpaqueResponse(selected, "selected");
                      const finalResponse =
                        await passThroughCacheIfNeeded(event, selected, cacheMode);
                      assertOpaqueResponse(finalResponse, "final");
                      return finalResponse;
                    }}));
                }});
                "#
            ),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_request_client(loader)
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        }),
    );

    for (index, (clone_mode, cache_mode)) in [
        ("none", "none"),
        ("clone-response", "none"),
        ("clone-unused", "none"),
        ("none", "pass-through"),
        ("clone-response", "pass-through"),
        ("clone-unused", "pass-through"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = service_worker_fetch_request_for_test();
        request.url = url::Url::parse(&format!(
            "https://example.test/app/TestRequest?clone={clone_mode}&cache={cache_mode}"
        ))
        .unwrap();
        request.headers = vec![("accept".to_owned(), "application/javascript".to_owned())];
        request.destination = ServiceWorkerRequestDestination::Script;
        request.request_mode = moli_fetch::RequestMode::NoCors;
        request.credentials_mode = moli_fetch::RequestCredentialsMode::Include;

        let completion = dispatch_service_worker_fetch_event_with_request_for_test(
            &mut handle,
            30 + index as u64,
            request,
        )
        .await;

        let response = match completion.result {
            ServiceWorkerFetchResult::Response(response) => response,
            other => {
                panic!(
                    "expected opaque service worker response for {clone_mode}/{cache_mode}, got {other:?}"
                );
            }
        };
        assert_eq!(
            response.response_type, "opaque",
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
        assert_eq!(
            response.status, 200,
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
        assert_eq!(
            response.final_url.as_ref().map(url::Url::as_str),
            Some(fetch_url.as_str()),
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
        assert_eq!(response.status_text, "OK");
        for (name, value) in [
            ("content-type", "application/javascript"),
            ("cross-origin-resource-policy", "cross-origin"),
            ("vary", "*"),
            ("set-cookie", "hidden=secret"),
        ] {
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(key, entry)| key.eq_ignore_ascii_case(name) && entry == value.as_bytes()),
                "missing internal {name} for {clone_mode}/{cache_mode}: {:?}",
                response.headers,
            );
        }
        assert_eq!(
            response.body,
            b"callback('OK');".to_vec(),
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
    }

    server
        .await
        .expect("service worker opaque body server should finish");
    handle.terminate_and_join();
}
