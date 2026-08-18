use super::*;
use crate::runtime::RendererBrowserContextRuntime;
use crate::runtime::{ServiceWorkerNotificationMetadata, ServiceWorkerShowNotificationResult};
use crate::service_worker_runtime::ServiceWorkerNotificationEventKind;
use crate::worker::WorkerGlobalKind;

#[tokio::test]
async fn dedicated_worker_agent_allows_blocking_atomics_wait() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const values = new Int32Array(new SharedArrayBuffer(4));
        postMessage(Atomics.wait(values, 0, 0, 1));
        "#
        .into(),
        "test://dedicated-worker-atomics-wait".into(),
    );

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for dedicated Worker Atomics.wait result")
        .expect("worker channel closed before Atomics.wait result");
    match message {
        WorkerToParentMessage::Post(payload) => {
            assert_eq!(stringify_payload(&payload), r#""timed-out""#);
        }
        WorkerToParentMessage::Error { message, .. } => {
            panic!("unexpected worker Atomics.wait error: {message}");
        }
        other => panic!("unexpected worker Atomics.wait message: {other:?}"),
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_teardown_releases_context_owned_v8_finalizers_before_isolate_drop() {
    ensure_v8();

    for iteration in 0..8 {
        let mut handle = spawn_worker(
            r#"
            globalThis.__finalizerBlobs = [];
            for (let index = 0; index < 256; index += 1) {
                globalThis.__finalizerBlobs.push(
                    new Blob([`payload-${index}`], { type: "text/plain" })
                );
            }
            postMessage(globalThis.__finalizerBlobs.length);
            "#
            .into(),
            format!("test://context-owned-finalizer-teardown-{iteration}"),
        );

        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker finalizer setup")
            .expect("worker channel closed before finalizer setup completed");
        match message {
            WorkerToParentMessage::Post(payload) => {
                assert_eq!(stringify_payload(&payload), "256");
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected worker finalizer setup error: {message}");
            }
            other => panic!("unexpected worker finalizer setup message: {other:?}"),
        }
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn readable_stream_reader_cancel_notifies_underlying_source() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const stream = new ReadableStream({
            cancel(reason) {
                postMessage("reader-cancelled:" + String(reason));
            }
        });
        const reader = stream.getReader();
        reader.cancel("stop");
        "#
        .into(),
        "test://readable-stream-reader-cancel".into(),
    );

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for reader cancel notification")
        .expect("worker channel closed");
    match message {
        WorkerToParentMessage::Post(payload) => {
            assert_eq!(stringify_payload(&payload), r#""reader-cancelled:stop""#);
        }
        WorkerToParentMessage::Error { message, .. } => {
            panic!("unexpected worker error while waiting for reader cancel: {message}");
        }
        other => panic!("unexpected message while waiting for reader cancel: {other:?}"),
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn readable_stream_controller_error_rejects_pending_read() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let savedController;
        const stream = new ReadableStream({
            start(controller) {
                savedController = controller;
            }
        });
        const reader = stream.getReader();
        reader.read().then(
            () => postMessage("read-resolved"),
            reason => postMessage("read-rejected:" + reason.message)
        );
        savedController.error(new Error("stream-broken"));
        "#
        .into(),
        "test://readable-stream-controller-error".into(),
    );

    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for reader error notification")
        .expect("worker channel closed");
    match message {
        WorkerToParentMessage::Post(payload) => {
            assert_eq!(
                stringify_payload(&payload),
                r#""read-rejected:stream-broken""#
            );
        }
        WorkerToParentMessage::Error { message, .. } => {
            panic!("unexpected worker error while waiting for reader error: {message}");
        }
        other => panic!("unexpected message while waiting for reader error: {other:?}"),
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_bootstrap_reports_fetch_handler_presence() {
    ensure_v8();

    async fn fetch_handler_type(script_source: &str) -> WorkerFetchHandlerType {
        let (bootstrap_tx, mut bootstrap_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
        let handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                script_source.to_owned(),
                "https://example.test/app/sw.js".to_owned(),
            )
            .with_global_kind(crate::worker::WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url::Url::parse("https://example.test/app/").unwrap(),
            })
            .with_bootstrap_completion_sender(bootstrap_tx),
        );
        let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
            .await
            .expect("timed out waiting for service worker bootstrap")
            .expect("service worker bootstrap channel closed");
        let success = bootstrap
            .result
            .expect("service worker bootstrap should pass");
        handle.terminate_and_join();
        success.service_worker_fetch_handler_type
    }

    assert_eq!(
        fetch_handler_type("self.addEventListener('install', () => {});").await,
        WorkerFetchHandlerType::NoHandler,
        "non-fetch listeners must not mark the version as having a fetch handler"
    );
    assert_eq!(
        fetch_handler_type("self.addEventListener('fetch', () => {});").await,
        WorkerFetchHandlerType::EmptyFetchHandler,
        "empty addEventListener('fetch') should be skippable"
    );
    assert_eq!(
        fetch_handler_type("self.onfetch = () => {};").await,
        WorkerFetchHandlerType::EmptyFetchHandler,
        "empty onfetch should be skippable"
    );
    assert_eq!(
        fetch_handler_type("self.addEventListener('fetch', event => undefined);").await,
        WorkerFetchHandlerType::NotSkippable,
        "expression-body fetch listeners stay conservative without a V8 nop-function binding"
    );
    assert_eq!(
        fetch_handler_type(
            "self.addEventListener('fetch', event => { event.respondWith(fetch(event.request)); });"
        )
        .await,
        WorkerFetchHandlerType::NotSkippable,
        "non-empty fetch listeners must dispatch FetchEvent"
    );
}

#[tokio::test]
async fn service_worker_global_scope_does_not_expose_close() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            if ("close" in self) {
                throw new Error("ServiceWorkerGlobalScope must not expose close");
            }
            if (Object.hasOwn(ServiceWorkerGlobalScope.prototype, "close")) {
                throw new Error("ServiceWorkerGlobalScope.prototype must not own close");
            }
            "#
            .to_owned(),
            "https://example.test/app/no-close-sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );

    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    bootstrap
        .result
        .expect("service worker bootstrap should not expose close");
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_pause_evaluation_until_debugger_exposes_context_before_bootstrap() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"postMessage("bootstrapped");"#.to_owned(),
            "https://example.test/app/paused-worker.js".to_owned(),
        )
        .with_bootstrap_completion_sender(bootstrap_tx)
        .with_pause_evaluation_until_debugger(true),
    );

    async fn dispatch_runtime(
        handle: &WorkerHandle,
        id: i64,
        method: &str,
    ) -> Vec<serde_json::Value> {
        let (response_tx, response_rx) = oneshot::channel();
        let raw_json = serde_json::json!({
            "id": id,
            "method": method,
        })
        .to_string();
        assert!(
            handle.dispatch_runtime_protocol_message(
                Some("SID-worker-pause".to_owned()),
                raw_json,
                None,
                response_tx,
            ),
            "worker should accept Runtime protocol command while paused before bootstrap"
        );
        timeout(TIMEOUT, response_rx)
            .await
            .expect("timed out waiting for worker Runtime command response")
            .expect("worker Runtime response channel closed")
            .expect("worker Runtime command failed")
            .into_iter()
            .map(crate::runtime::RendererRuntimeInspectorMessage::into_v8_inspector_message)
            .collect()
    }

    let enable_messages = dispatch_runtime(&handle, 1, "Runtime.enable").await;
    let created_context = enable_messages
        .iter()
        .find(|message| message["method"] == "Runtime.executionContextCreated")
        .unwrap_or_else(|| {
            panic!(
                "paused worker should expose its execution context before top-level script evaluation: {enable_messages:?}"
            )
        });
    assert_eq!(
        created_context["params"]["context"]["origin"],
        "https://example.test/app/paused-worker.js"
    );
    assert!(
        matches!(
            bootstrap_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ),
        "top-level worker script must not bootstrap before Runtime.runIfWaitingForDebugger"
    );

    let run_if_messages = dispatch_runtime(&handle, 2, "Runtime.runIfWaitingForDebugger").await;
    assert!(
        run_if_messages
            .iter()
            .any(|message| message["id"] == 2 && message.get("result").is_some()),
        "Runtime.runIfWaitingForDebugger should complete through the worker inspector: {run_if_messages:?}"
    );

    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for worker bootstrap after debugger release")
        .expect("worker bootstrap channel closed after debugger release");
    bootstrap
        .result
        .expect("worker bootstrap should pass after debugger release");

    let mut saw_worker_script_loaded = false;
    let mut saw_bootstrapped_message = false;
    while !saw_worker_script_loaded || !saw_bootstrapped_message {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker postMessage after debugger release")
            .expect("worker channel closed after debugger release");
        match message {
            WorkerToParentMessage::Post(payload) => {
                assert_eq!(stringify_payload(&payload), r#""bootstrapped""#);
                saw_bootstrapped_message = true;
            }
            WorkerToParentMessage::RuntimeInspectorMessages(batches) => {
                saw_worker_script_loaded |= batches.iter().any(|batch| {
                    batch.inspector_session_id.as_deref() == Some("SID-worker-pause")
                        && batch.messages.iter().any(|message| {
                            message.clone().into_v8_inspector_message()
                                == serde_json::json!({
                                    "method": "Inspector.workerScriptLoaded",
                                    "params": {}
                                })
                        })
                });
            }
            other => panic!("unexpected worker message after debugger release: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_attach_before_runtime_enable_preserves_script_loaded_after_resume() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"postMessage("bootstrapped");"#.to_owned(),
            "https://example.test/app/attached-worker.js".to_owned(),
        )
        .with_bootstrap_completion_sender(bootstrap_tx)
        .with_pause_evaluation_until_debugger(true),
    );

    assert!(
        handle.attach_runtime_inspector_session(Some("SID-worker-attached".to_owned())),
        "worker should accept its Inspector session before the first command"
    );
    assert!(
        handle.run_if_waiting_for_debugger_for_devtools(),
        "worker should accept debugger resume after the Inspector session is attached"
    );

    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for worker bootstrap after debugger release")
        .expect("worker bootstrap channel closed after debugger release");
    bootstrap
        .result
        .expect("worker bootstrap should pass after debugger release");

    let mut saw_worker_script_loaded = false;
    let mut saw_bootstrapped_message = false;
    while !saw_worker_script_loaded || !saw_bootstrapped_message {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for attached worker output")
            .expect("worker channel closed before attached worker output");
        match message {
            WorkerToParentMessage::Post(payload) => {
                assert_eq!(stringify_payload(&payload), r#""bootstrapped""#);
                saw_bootstrapped_message = true;
            }
            WorkerToParentMessage::RuntimeInspectorMessages(batches) => {
                saw_worker_script_loaded |= batches.iter().any(|batch| {
                    batch.inspector_session_id.as_deref() == Some("SID-worker-attached")
                        && batch.messages.iter().any(|message| {
                            message.clone().into_v8_inspector_message()
                                == serde_json::json!({
                                    "method": "Inspector.workerScriptLoaded",
                                    "params": {}
                                })
                        })
                });
            }
            other => panic!("unexpected attached worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_inspector_interrupt_overtakes_js_running_command_during_active_javascript() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let deliveries = 0;
        self.onmessage = () => {
            deliveries += 1;
            postMessage(deliveries === 1 ? "entered" : "recovered");
            if (deliveries === 1) {
                while (true) {}
            }
        };
        postMessage("ready");
        "#
        .to_owned(),
        "https://example.test/app/interruptible-worker.js".to_owned(),
    );

    fn dispatch_runtime(
        handle: &WorkerHandle,
        id: i64,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> oneshot::Receiver<Result<Vec<crate::runtime::RendererRuntimeInspectorMessage>, String>>
    {
        let (response_tx, response_rx) = oneshot::channel();
        let mut message = serde_json::json!({
            "id": id,
            "method": method,
        });
        if let Some(params) = params {
            message["params"] = params;
        }
        assert!(
            handle.dispatch_runtime_protocol_message(
                Some("SID-worker-interrupt".to_owned()),
                message.to_string(),
                None,
                response_tx,
            ),
            "worker should accept {method}"
        );
        response_rx
    }

    let ready = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker readiness")
        .expect("worker closed before readiness");
    assert!(matches!(
        ready,
        WorkerToParentMessage::Post(ref payload) if stringify_payload(payload) == r#""ready""#
    ));

    let enable = dispatch_runtime(&handle, 1, "Runtime.enable", None);
    timeout(TIMEOUT, enable)
        .await
        .expect("timed out enabling worker Runtime")
        .expect("worker Runtime enable response channel closed")
        .expect("worker Runtime.enable failed");

    handle.post_message(serialize_test_string("start"));
    let entered = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for active worker JavaScript")
        .expect("worker closed before entering active JavaScript");
    assert!(matches!(
        entered,
        WorkerToParentMessage::Post(ref payload) if stringify_payload(payload) == r#""entered""#
    ));

    let mut evaluate = dispatch_runtime(
        &handle,
        2,
        "Runtime.evaluate",
        Some(serde_json::json!({
            "expression": "40 + 2",
            "returnByValue": true,
        })),
    );
    assert!(
        timeout(Duration::from_millis(100), &mut evaluate)
            .await
            .is_err(),
        "Runtime.evaluate must not interrupt active worker JavaScript"
    );

    let terminate = dispatch_runtime(&handle, 3, "Runtime.terminateExecution", None);
    timeout(TIMEOUT, terminate)
        .await
        .expect("Runtime.terminateExecution did not interrupt active worker JavaScript")
        .expect("worker Runtime termination response channel closed")
        .expect("worker Runtime.terminateExecution failed");

    let evaluate_messages = timeout(TIMEOUT, &mut evaluate)
        .await
        .expect("queued Runtime.evaluate did not run after termination")
        .expect("queued Runtime.evaluate response channel closed")
        .expect("queued Runtime.evaluate failed after termination");
    let evaluate_response = evaluate_messages
        .into_iter()
        .map(crate::runtime::RendererRuntimeInspectorMessage::into_v8_inspector_message)
        .find(|message| message["id"] == 2)
        .expect("queued Runtime.evaluate response");
    assert_eq!(evaluate_response["result"]["result"]["value"], 42);

    handle.post_message(serialize_test_string("again"));
    loop {
        let recovered = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker recovery")
            .expect("worker closed instead of recovering");
        match recovered {
            WorkerToParentMessage::Post(payload) => {
                assert_eq!(stringify_payload(&payload), r#""recovered""#);
                break;
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            WorkerToParentMessage::Error { message, .. } => {
                assert_eq!(
                    message, "null",
                    "worker emitted an unexpected error while recovering"
                );
            }
            other => panic!("unexpected worker output while checking recovery: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn real_workers_register_service_worker_clients_until_thread_exit() {
    ensure_v8();

    async fn assert_worker_client_lifetime(global_kind: WorkerGlobalKind, script_url: &str) {
        let browser_context_runtime = RendererBrowserContextRuntime::new();
        let service = browser_context_runtime.service_worker_runtime();
        let (bootstrap_tx, mut bootstrap_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
        let handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                "self.addEventListener('message', () => {});".to_owned(),
                script_url.to_owned(),
            )
            .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
            .with_service_worker_runtime(service.clone())
            .with_global_kind(global_kind)
            .with_bootstrap_completion_sender(bootstrap_tx),
        );
        let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
            .await
            .expect("timed out waiting for worker bootstrap")
            .expect("worker bootstrap channel closed");
        bootstrap.result.expect("worker bootstrap should pass");
        assert_eq!(service.diagnostics_snapshot().live_client_count, 1);
        handle.terminate_and_join();
        assert_eq!(service.diagnostics_snapshot().live_client_count, 0);
    }

    assert_worker_client_lifetime(
        WorkerGlobalKind::Dedicated {
            name: "dedicated".to_owned(),
        },
        "https://example.test/app/dedicated-worker.js",
    )
    .await;

    let shared_script_url = url::Url::parse("https://example.test/app/shared-worker.js").unwrap();
    let shared_storage_key =
        moli_storage_key::MoliStorageKey::first_party_from_url(&shared_script_url, None);
    assert_worker_client_lifetime(
        WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key: shared_storage_key,
        },
        shared_script_url.as_str(),
    )
    .await;
}

#[tokio::test]
async fn service_worker_install_wait_until_resolution_completes_lifecycle_event() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            event.waitUntil(Promise.resolve().then(() => "ok"));
        });
        "#,
    );

    let completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        1,
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(1)
    );
    assert_eq!(
        completion.version_id,
        ServiceWorkerVersionId::from_u64_for_test(1)
    );
    assert_eq!(completion.kind, ServiceWorkerLifecycleEventKind::Install);
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_preload_response_resolves_undefined_without_preload() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith((async () => {
                const promise = event.preloadResponse;
                const value = await promise;
                return new Response(JSON.stringify({
                    hasPromise: promise instanceof Promise,
                    samePromise: promise === event.preloadResponse,
                    valueType: typeof value,
                    isUndefined: value === undefined
                }), { status: 209 });
            })());
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 137).await;
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected preloadResponse probe response, got {completion:?}");
    };
    assert_eq!(response.status, 209);
    assert_eq!(
        String::from_utf8(response.body).expect("preloadResponse body should be UTF-8"),
        r#"{"hasPromise":true,"samePromise":true,"valueType":"undefined","isUndefined":true}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_preload_response_resolves_network_response() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith((async () => {
                const response = await event.preloadResponse;
                return new Response(JSON.stringify({
                    hasResponse: response instanceof Response,
                    status: response.status,
                    type: response.type,
                    url: response.url,
                    header: response.headers.get("x-preload"),
                    body: await response.text()
                }), { status: 210 });
            })());
        });
        "#,
    );

    let event_id = ServiceWorkerEventId::from_u64_for_worker(138);
    let request_url = url::Url::parse("https://example.test/app/navigation.html")
        .expect("navigation preload request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 901;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            request_url,
            request_mode: moli_fetch::RequestMode::Navigate,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(
                    url::Url::parse("https://example.test/app/navigation.html")
                        .expect("navigation preload response URL"),
                ),
                response_type: "default".to_owned(),
                redirected: false,
                status: 202,
                headers: vec![("x-preload".to_owned(), "yes".to_owned())],
            },
        },
    );
    handle.enqueue_service_worker_navigation_preload_chunk(
        ServiceWorkerNavigationPreloadStreamChunk {
            event_id,
            body_source_id,
            bytes: b"preloaded-".to_vec(),
        },
    );
    handle.enqueue_service_worker_navigation_preload_chunk(
        ServiceWorkerNavigationPreloadStreamChunk {
            event_id,
            body_source_id,
            bytes: b"body".to_vec(),
        },
    );
    handle.finish_service_worker_navigation_preload_stream(
        ServiceWorkerNavigationPreloadStreamFinished {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            body_source_id,
            result: Ok(()),
        },
    );

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for preloadResponse network response")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    };
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected preloadResponse network probe response, got {completion:?}");
    };
    assert_eq!(response.status, 210);
    assert_eq!(
        String::from_utf8(response.body).expect("preloadResponse probe body should be UTF-8"),
        r#"{"hasResponse":true,"status":202,"type":"basic","url":"https://example.test/app/navigation.html","header":"yes","body":"preloaded-body"}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_preload_response_opaqueredirect_exposes_request_url() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith((async () => {
                const response = await event.preloadResponse;
                return new Response(JSON.stringify({
                    hasResponse: response instanceof Response,
                    status: response.status,
                    type: response.type,
                    url: response.url,
                    ok: response.ok,
                    statusText: response.statusText,
                    redirected: response.redirected,
                    bodyIsNull: response.body === null,
                    headers: Array.from(response.headers)
                }), { status: 214 });
            })());
        });
        "#,
    );

    let event_id = ServiceWorkerEventId::from_u64_for_worker(142);
    let request_url = url::Url::parse("https://example.test/app/preload-redirect.html")
        .expect("navigation preload redirect request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 904;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            request_url: request_url.clone(),
            request_mode: moli_fetch::RequestMode::Navigate,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(request_url),
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                headers: vec![("location".to_owned(), "/app/final.html".to_owned())],
            },
        },
    );
    handle.finish_service_worker_navigation_preload_stream(
        ServiceWorkerNavigationPreloadStreamFinished {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            body_source_id,
            result: Ok(()),
        },
    );

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for preloadResponse opaqueredirect probe")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    };
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected preloadResponse opaqueredirect probe response, got {completion:?}");
    };
    assert_eq!(response.status, 214);
    assert_eq!(
        String::from_utf8(response.body).expect("preloadResponse probe body should be UTF-8"),
        r#"{"hasResponse":true,"status":0,"type":"opaqueredirect","url":"https://example.test/app/preload-redirect.html","ok":false,"statusText":"","redirected":false,"bodyIsNull":true,"headers":[]}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_preload_response_rejects_before_response() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith((async () => {
                let rejection = null;
                try {
                    await event.preloadResponse;
                } catch (error) {
                    rejection = {
                        name: error && error.name,
                        isDomException: error instanceof DOMException
                    };
                }
                return new Response(JSON.stringify(rejection), { status: 211 });
            })());
        });
        "#,
    );

    let event_id = ServiceWorkerEventId::from_u64_for_worker(139);
    let mut request = service_worker_fetch_request_for_test();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        request,
        navigation_preload_sent: true,
    });
    handle.fail_service_worker_navigation_preload(ServiceWorkerNavigationPreloadFailure {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        message: "navigation preload failed".to_owned(),
    });

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for preloadResponse rejection response")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    };
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected preloadResponse rejection probe response, got {completion:?}");
    };
    assert_eq!(response.status, 211);
    assert_eq!(
        String::from_utf8(response.body).expect("preloadResponse probe body should be UTF-8"),
        r#"{"name":"NetworkError","isDomException":true}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_preload_response_body_errors_after_response() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith((async () => {
                const response = await event.preloadResponse;
                let bodyError = null;
                try {
                    await response.text();
                } catch (error) {
                    bodyError = {
                        name: error && error.name,
                        message: error && error.message,
                        isTypeError: error instanceof TypeError
                    };
                }
                return new Response(JSON.stringify({
                    hasResponse: response instanceof Response,
                    status: response.status,
                    bodyError
                }), { status: 212 });
            })());
        });
        "#,
    );

    let event_id = ServiceWorkerEventId::from_u64_for_worker(140);
    let request_url = url::Url::parse("https://example.test/app/navigation.html")
        .expect("navigation preload request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 902;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            request_url,
            request_mode: moli_fetch::RequestMode::Navigate,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(
                    url::Url::parse("https://example.test/app/navigation.html")
                        .expect("navigation preload response URL"),
                ),
                response_type: "default".to_owned(),
                redirected: false,
                status: 202,
                headers: vec![("x-preload".to_owned(), "yes".to_owned())],
            },
        },
    );
    handle.enqueue_service_worker_navigation_preload_chunk(
        ServiceWorkerNavigationPreloadStreamChunk {
            event_id,
            body_source_id,
            bytes: b"partial".to_vec(),
        },
    );
    handle.finish_service_worker_navigation_preload_stream(
        ServiceWorkerNavigationPreloadStreamFinished {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            body_source_id,
            result: Err("navigation preload stream failed".to_owned()),
        },
    );

    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for preloadResponse body error response")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    };
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected preloadResponse body error probe response, got {completion:?}");
    };
    assert_eq!(response.status, 212);
    assert_eq!(
        String::from_utf8(response.body).expect("preloadResponse probe body should be UTF-8"),
        r#"{"hasResponse":true,"status":202,"bodyError":{"name":"TypeError","message":"navigation preload stream failed","isTypeError":true}}"#
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_preload_response_body_completes_after_fetch_event() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.preloadResponse
                .then(response => {
                    console.log(JSON.stringify({
                        phase: "got-response",
                        status: response.status
                    }));
                    return response.text();
                })
                .then(
                    text => console.log(JSON.stringify({
                        phase: "body",
                        text
                    })),
                    error => console.log(JSON.stringify({
                        phase: "body-error",
                        errorName: error && error.name,
                        errorMessage: error && error.message
                    }))
                );
            event.respondWith(new Response("immediate", { status: 213 }));
        });
        "#,
    );

    let event_id = ServiceWorkerEventId::from_u64_for_worker(141);
    let request_url = url::Url::parse("https://example.test/app/navigation.html")
        .expect("navigation preload request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 903;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            request_url,
            request_mode: moli_fetch::RequestMode::Navigate,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(
                    url::Url::parse("https://example.test/app/navigation.html")
                        .expect("navigation preload response URL"),
                ),
                response_type: "default".to_owned(),
                redirected: false,
                status: 200,
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            },
        },
    );

    let mut posts = Vec::new();
    let completion = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for immediate fetch response")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => break completion,
            WorkerToParentMessage::Console(message) => posts.push(message.message),
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    };
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected immediate fetch response, got {completion:?}");
    };
    assert_eq!(response.status, 213);
    assert_eq!(
        String::from_utf8(response.body).expect("immediate fetch body should be UTF-8"),
        "immediate"
    );

    handle.enqueue_service_worker_navigation_preload_chunk(
        ServiceWorkerNavigationPreloadStreamChunk {
            event_id,
            body_source_id,
            bytes: b"late-".to_vec(),
        },
    );
    handle.enqueue_service_worker_navigation_preload_chunk(
        ServiceWorkerNavigationPreloadStreamChunk {
            event_id,
            body_source_id,
            bytes: b"body".to_vec(),
        },
    );
    handle.finish_service_worker_navigation_preload_stream(
        ServiceWorkerNavigationPreloadStreamFinished {
            event_id,
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            body_source_id,
            result: Ok(()),
        },
    );
    let body_post = loop {
        if let Some(post) = posts.iter().find(|post| post.contains(r#""phase":"body""#)) {
            break post.clone();
        }
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .unwrap_or_else(|_| {
                panic!("timed out waiting for late preload body console message; posts={posts:?}")
            })
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::Console(message) => posts.push(message.message),
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            _ => {}
        }
    };
    assert!(
        posts
            .iter()
            .any(|post| post == r#"log: {"phase":"got-response","status":200}"#),
        "preloadResponse should resolve before body completion; posts={posts:?}"
    );
    assert_eq!(body_post, r#"log: {"phase":"body","text":"late-body"}"#);
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_install_wait_until_rejection_rejects_lifecycle_event() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            event.waitUntil(Promise.reject(new Error("install-boom")));
        });
        "#,
    );

    let completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        2,
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(2)
    );
    assert_eq!(completion.kind, ServiceWorkerLifecycleEventKind::Install);
    assert_eq!(
        completion.result,
        Err("service worker waitUntil promise rejected".to_owned())
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_wait_until_after_event_completion_throws_invalid_state() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        let captured;
        self.addEventListener("install", event => {
            captured = event;
        });
        self.addEventListener("activate", event => {
            try {
                captured.waitUntil(Promise.resolve());
                throw new Error("late waitUntil unexpectedly succeeded");
            } catch (error) {
                if (error.name !== "InvalidStateError" || !(error instanceof DOMException)) {
                    throw error;
                }
            }
        });
        "#,
    );

    let install_completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        3,
    )
    .await;
    assert_eq!(install_completion.result, Ok(()));

    let activate_completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Activate,
        4,
    )
    .await;
    assert_eq!(activate_completion.result, Ok(()));
    handle.terminate_and_join();
}

async fn dispatch_service_worker_lifecycle_event_and_console_for_test(
    handle: &mut crate::worker::WorkerHandle,
    kind: ServiceWorkerLifecycleEventKind,
    event_id: u64,
) -> (ServiceWorkerLifecycleCompletion, String) {
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        kind,
    });

    let mut completion = None;
    let mut console = None;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for lifecycle event and console")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(message) => {
                completion = Some(message);
            }
            WorkerToParentMessage::Console(message) => {
                console = Some(message.message);
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for lifecycle: {message}");
            }
            other => panic!("unexpected message while waiting for lifecycle console: {other:?}"),
        }
        if let (Some(completion), Some(console)) = (completion.as_ref(), console.as_ref()) {
            return (completion.clone(), console.clone());
        }
    }
}

async fn dispatch_service_worker_message_event_and_console_for_test(
    handle: &mut crate::worker::WorkerHandle,
    event_id: u64,
    data: V8StructuredClonePayload,
) -> (ServiceWorkerMessageCompletion, String) {
    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: data,
        window_interaction_allowed: false,
    });

    let mut completion = None;
    let mut console = None;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for message event and console")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(message) => {
                completion = Some(message);
            }
            WorkerToParentMessage::Console(message) => {
                console = Some(message.message);
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for message: {message}");
            }
            other => panic!("unexpected message while waiting for message console: {other:?}"),
        }
        if let (Some(completion), Some(console)) = (completion.as_ref(), console.as_ref()) {
            return (completion.clone(), console.clone());
        }
    }
}

#[tokio::test]
async fn service_worker_wait_until_in_microtask_extends_lifecycle_event() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            Promise.resolve().then(() => {
                let result = "unset";
                try {
                    event.waitUntil(Promise.resolve());
                    result = "OK";
                } catch (error) {
                    result = (error && error.name) + ":" + (error instanceof DOMException);
                }
                console.log("microtask:" + result);
            });
        });
        "#,
    );

    let (completion, console) = dispatch_service_worker_lifecycle_event_and_console_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        64,
    )
    .await;

    assert_eq!(completion.kind, ServiceWorkerLifecycleEventKind::Install);
    assert_eq!(completion.result, Ok(()));
    assert_eq!(console, "log: microtask:OK");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_wait_until_in_task_after_lifecycle_dispatch_throws_invalid_state() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            setTimeout(() => {
                let result = "unset";
                try {
                    event.waitUntil(Promise.resolve());
                    result = "OK";
                } catch (error) {
                    result = (error && error.name) + ":" + (error instanceof DOMException);
                }
                console.log("task:" + result);
            }, 0);
        });
        "#,
    );

    let (completion, console) = dispatch_service_worker_lifecycle_event_and_console_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        65,
    )
    .await;

    assert_eq!(completion.kind, ServiceWorkerLifecycleEventKind::Install);
    assert_eq!(completion.result, Ok(()));
    assert_eq!(console, "log: task:InvalidStateError:true");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_wait_until_in_microtask_extends_event() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            Promise.resolve().then(() => {
                let result = "unset";
                try {
                    event.waitUntil(Promise.resolve());
                    result = "OK";
                } catch (error) {
                    result = (error && error.name) + ":" + (error instanceof DOMException);
                }
                console.log("message-microtask:" + result);
            });
        });
        "#,
    );

    let (completion, console) = dispatch_service_worker_message_event_and_console_for_test(
        &mut handle,
        66,
        serialize_test_string("ping"),
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(66)
    );
    assert_eq!(completion.result, Ok(()));
    assert_eq!(console, "log: message-microtask:OK");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_wait_until_same_turn_after_pending_promise_settles_succeeds() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            let resolveFirst;
            const first = new Promise(resolve => {
                resolveFirst = resolve;
            });
            event.waitUntil(first);
            first.then(() => {
                let result = "unset";
                try {
                    event.waitUntil(Promise.resolve());
                    result = "OK";
                } catch (error) {
                    result = (error && error.name) + ":" + (error instanceof DOMException);
                }
                console.log("same-turn:" + result);
            });
            setTimeout(resolveFirst, 0);
        });
        "#,
    );

    let (completion, console) = dispatch_service_worker_message_event_and_console_for_test(
        &mut handle,
        67,
        serialize_test_string("ping"),
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(67)
    );
    assert_eq!(completion.result, Ok(()));
    assert_eq!(console, "log: same-turn:OK");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_wait_until_extra_microtask_after_pending_promise_settles_throws() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            let resolveFirst;
            const first = new Promise(resolve => {
                resolveFirst = resolve;
            });
            event.waitUntil(first);
            first.then(() => Promise.resolve().then(() => {
                let result = "unset";
                try {
                    event.waitUntil(Promise.resolve());
                    result = "OK";
                } catch (error) {
                    result = (error && error.name) + ":" + (error instanceof DOMException);
                }
                console.log("extra-turn:" + result);
            }));
            setTimeout(resolveFirst, 0);
        });
        "#,
    );

    let (completion, console) = dispatch_service_worker_message_event_and_console_for_test(
        &mut handle,
        68,
        serialize_test_string("ping"),
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(68)
    );
    assert_eq!(completion.result, Ok(()));
    assert_eq!(console, "log: extra-turn:InvalidStateError:true");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_keeps_event_active_for_async_wait_until_matrix() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        function waitUntilResult(event) {
            try {
                event.waitUntil(Promise.resolve());
                return "OK";
            } catch (error) {
                return (error && error.name) + ":" + (error instanceof DOMException);
            }
        }

        function newTaskResponse(body) {
            return new Promise(resolve => {
                setTimeout(() => resolve(new Response(body)), 0);
            });
        }

        self.addEventListener("fetch", event => {
            const step = new URL(event.request.url).pathname.split("/").pop();
            let response;
            if (step === "pending-respondwith-async-waituntil") {
                let resolveResponse;
                response = new Promise(resolve => {
                    resolveResponse = resolve;
                });
                event.respondWith(response);
                setTimeout(() => {
                    console.log(step + ":" + waitUntilResult(event));
                    resolveResponse(new Response(step));
                }, 0);
                return;
            }
            if (step === "during-event-dispatch-respondwith-microtask-sync-waituntil") {
                response = Promise.resolve(new Response(step));
                event.respondWith(response);
                response.then(() => {
                    console.log(step + ":" + waitUntilResult(event));
                });
                return;
            }
            if (step === "during-event-dispatch-respondwith-microtask-async-waituntil") {
                response = Promise.resolve(new Response(step));
                event.respondWith(response);
                response.then(() => Promise.resolve().then(() => {
                    console.log(step + ":" + waitUntilResult(event));
                }));
                return;
            }
            if (step === "after-event-dispatch-respondwith-microtask-sync-waituntil") {
                response = newTaskResponse(step);
                event.respondWith(response);
                response.then(() => {
                    console.log(step + ":" + waitUntilResult(event));
                });
                return;
            }
            if (step === "after-event-dispatch-respondwith-microtask-async-waituntil") {
                response = newTaskResponse(step);
                event.respondWith(response);
                response.then(() => Promise.resolve().then(() => {
                    console.log(step + ":" + waitUntilResult(event));
                }));
            }
        });
        "#,
    );

    async fn dispatch_case(
        handle: &mut crate::worker::WorkerHandle,
        event_id: u64,
        step: &str,
        expected_wait_until: &str,
    ) {
        let mut request = service_worker_fetch_request_for_test();
        request.url =
            url::Url::parse(&format!("https://example.test/app/{step}")).expect("case URL");

        let (completion, console) =
            dispatch_service_worker_fetch_event_and_handled_console_for_test(
                handle, event_id, request,
            )
            .await;

        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected respondWith response for {step}");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body, step.as_bytes().to_vec());
        assert_eq!(console, format!("log: {step}:{expected_wait_until}"));
    }

    dispatch_case(&mut handle, 69, "pending-respondwith-async-waituntil", "OK").await;
    dispatch_case(
        &mut handle,
        70,
        "during-event-dispatch-respondwith-microtask-sync-waituntil",
        "OK",
    )
    .await;
    dispatch_case(
        &mut handle,
        71,
        "during-event-dispatch-respondwith-microtask-async-waituntil",
        "OK",
    )
    .await;
    dispatch_case(
        &mut handle,
        72,
        "after-event-dispatch-respondwith-microtask-sync-waituntil",
        "OK",
    )
    .await;
    dispatch_case(
        &mut handle,
        73,
        "after-event-dispatch-respondwith-microtask-async-waituntil",
        "InvalidStateError:true",
    )
    .await;

    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_wait_until_uses_native_promise_adoption() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            Promise.resolve = () => {
                throw new Error("patched Promise.resolve should not run");
            };
            event.waitUntil("ok");
        });
        "#,
    );

    let completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        10,
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(10)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_wait_until_uses_native_promise_reactions() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            const promise = new Promise(resolve => resolve("ok"));
            promise.then = () => {
                throw new Error("patched promise.then should not run");
            };
            event.waitUntil(promise);
        });
        "#,
    );

    let completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        11,
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(11)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_without_respond_with_falls_back() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.waitUntil(Promise.resolve());
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 7).await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(7)
    );
    assert_eq!(
        completion.version_id,
        ServiceWorkerVersionId::from_u64_for_test(1)
    );
    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Fallback
    ));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_prevent_default_without_respond_with_fails_fetch() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.preventDefault();
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 57).await;

    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected preventDefault without respondWith to fail fetch");
    };
    assert_eq!(message, "FetchEvent was canceled without respondWith().");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_prevent_default_then_respond_with_uses_response() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.preventDefault();
            event.respondWith(new Response("prevented-response", {status: 201}));
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 58).await;

    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("expected preventDefault with respondWith to use response");
    };
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"prevented-response".to_vec());
    handle.terminate_and_join();
}

async fn dispatch_service_worker_fetch_event_and_handled_console_for_test(
    handle: &mut crate::worker::WorkerHandle,
    event_id: u64,
    request: ServiceWorkerFetchRequest,
) -> (ServiceWorkerFetchCompletion, String) {
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        request,
        navigation_preload_sent: false,
    });

    let mut completion = None;
    let mut handled_console = None;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for handled fetch event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(message) => {
                completion = Some(message);
            }
            WorkerToParentMessage::Console(message) => {
                handled_console = Some(message.message);
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for handled: {message}");
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
        if let (Some(completion), Some(handled_console)) =
            (completion.as_ref(), handled_console.as_ref())
        {
            return (completion.clone(), handled_console.clone());
        }
    }
}

#[tokio::test]
async fn service_worker_fetch_event_handled_resolves_for_fallback() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.handled.then(
                () => console.log("handled:RESOLVED"),
                () => console.log("handled:REJECTED")
            );
        });
        "#,
    );

    let (completion, message) = dispatch_service_worker_fetch_event_and_handled_console_for_test(
        &mut handle,
        59,
        service_worker_fetch_request_for_test(),
    )
    .await;

    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Fallback
    ));
    assert_eq!(message, "log: handled:RESOLVED");
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_handled_rejects_for_canceled_fallback() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.handled.then(
                () => console.log("handled:RESOLVED"),
                error => console.log(
                    "handled:REJECTED:" +
                    (error && error.name) + ":" +
                    (error instanceof DOMException) + ":" +
                    (error && error.message)
                )
            );
            event.preventDefault();
        });
        "#,
    );

    let (completion, handled_message) =
        dispatch_service_worker_fetch_event_and_handled_console_for_test(
            &mut handle,
            60,
            service_worker_fetch_request_for_test(),
        )
        .await;

    let ServiceWorkerFetchResult::Failure(failure_message) = completion.result else {
        panic!("expected canceled fallback to fail");
    };
    assert_eq!(
        failure_message,
        "FetchEvent was canceled without respondWith()."
    );
    assert_eq!(
        handled_message,
        "log: handled:REJECTED:NetworkError:true:FetchEvent was canceled without respondWith()."
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_event_handled_follows_respond_with_result() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            const search = new URL(event.request.url).search;
            event.handled.then(
                () => console.log(search + ":RESOLVED"),
                error => console.log(
                    search + ":REJECTED:" +
                    (error && error.name) + ":" +
                    (error instanceof DOMException)
                )
            );
            if (search === "?resolved") {
                event.respondWith(Promise.resolve(new Response("body")));
            } else if (search === "?invalid") {
                event.respondWith(Promise.resolve("invalid response"));
            } else if (search === "?rejected") {
                event.respondWith(Promise.reject(new Error("respondWith rejected")));
            }
        });
        "#,
    );

    async fn dispatch_and_console(
        handle: &mut crate::worker::WorkerHandle,
        event_id: u64,
        request_url: &str,
    ) -> (ServiceWorkerFetchCompletion, String) {
        let mut request = service_worker_fetch_request_for_test();
        request.url = url::Url::parse(request_url).expect("test service worker request URL");
        dispatch_service_worker_fetch_event_and_handled_console_for_test(handle, event_id, request)
            .await
    }

    let (completion, message) = dispatch_and_console(
        &mut handle,
        61,
        "https://example.test/app/data.txt?resolved",
    )
    .await;
    assert_eq!(message, "log: ?resolved:RESOLVED");
    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Response(_)
    ));

    let (completion, message) =
        dispatch_and_console(&mut handle, 62, "https://example.test/app/data.txt?invalid").await;
    assert_eq!(message, "log: ?invalid:REJECTED:NetworkError:true");
    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Failure(_)
    ));

    let (completion, message) = dispatch_and_console(
        &mut handle,
        63,
        "https://example.test/app/data.txt?rejected",
    )
    .await;
    assert_eq!(message, "log: ?rejected:REJECTED:NetworkError:true");
    assert!(matches!(
        completion.result,
        ServiceWorkerFetchResult::Failure(_)
    ));
    handle.terminate_and_join();
}

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
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
            .any(|(name, value)| name == "x-worker" && value == "yes")
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
            .any(|(name, value)| name == "x-stream" && value == "yes")
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
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
                    started.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(started.generation, 1);
                assert_eq!(started.response_head.response_type, "default");
                assert_eq!(started.response_head.status, 202);
                assert!(
                    started
                        .response_head
                        .headers
                        .iter()
                        .any(|(name, value)| name == "x-stream" && value == "yes")
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
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
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
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
async fn service_worker_fetch_respond_with_body_accessed_opaque_response_keeps_internal_body() {
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
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 15\r\nConnection: close\r\n\r\ncallback('OK');",
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
                      response.body !== null || response.bodyUsed) {{
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
            response.status, 0,
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
        assert_eq!(
            response.final_url.as_ref().map(url::Url::as_str),
            Some(fetch_url.as_str()),
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
        assert!(
            response.headers.is_empty(),
            "clone/cache mode {clone_mode}/{cache_mode}"
        );
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
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
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
        completion.version_id,
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
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
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
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
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
async fn service_worker_messageerror_dispatches_when_payload_cannot_deserialize() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            throw new Error("message should not dispatch for invalid payload");
        });
        self.addEventListener("messageerror", event => {
            if (event.type !== "messageerror") {
                throw new Error("unexpected type:" + event.type);
            }
            if (event.data !== null) {
                throw new Error("unexpected data:" + event.data);
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
            event.waitUntil(Promise.resolve().then(() => {}));
        });
        "#,
    );

    let completion =
        dispatch_service_worker_message_event_for_test(&mut handle, 23, Default::default()).await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(23)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_messageerror_dispatches_when_wasm_module_sender_origin_mismatches() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            throw new Error("message should not dispatch for disallowed WebAssembly.Module");
        });
        self.addEventListener("messageerror", event => {
            const actual = [
                event.type,
                event.data === null,
                event.source === null,
                event.origin
            ].join("|");
            if (actual !== "messageerror|true|true|https://sender.example") {
                throw new Error("unexpected messageerror:" + actual);
            }
        });
        "#,
    );

    let mut payload = serialize_test_post_message_value(
        "new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))",
    );
    payload.metadata.sender_origin = Some("https://sender.example".to_owned());
    let completion = dispatch_service_worker_message_event_object_for_test(
        &mut handle,
        ServiceWorkerMessageEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(24),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            source_client_id: None,
            source_client_url: None,
            source_client_snapshot: None,
            source_worker: None,
            source_origin: "https://sender.example".to_owned(),
            payload,
            window_interaction_allowed: false,
        },
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(24)
    );
    assert_eq!(completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_global_scope_handler_attributes_dispatch_functional_events() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        const handlerNames = [
            "oninstall",
            "onactivate",
            "onfetch",
            "onpush",
            "onsync",
            "onperiodicsync",
            "onmessage",
            "onmessageerror",
            "onnotificationclick",
            "onnotificationclose"
        ];
        const privateStateNames = [
            "__workerState",
            "__moliWorkerGlobalOnError",
            "__moliWorkerGlobalOnOffline",
            "__moliWorkerGlobalOnOnline",
            "__moliWorkerGlobalOnUnhandledRejection",
            "__moliWorkerGlobalOnRejectionHandled",
            "__moliWorkerGlobalOnInstall",
            "__moliWorkerGlobalOnActivate",
            "__moliWorkerGlobalOnFetch",
            "__moliWorkerGlobalOnPush",
            "__moliWorkerGlobalOnSync",
            "__moliWorkerGlobalOnPeriodicSync",
            "__moliWorkerGlobalOnMessage",
            "__moliWorkerGlobalOnMessageError",
            "__moliWorkerGlobalOnNotificationClick",
            "__moliWorkerGlobalOnNotificationClose"
        ];
        for (const name of privateStateNames) {
            if (Object.prototype.hasOwnProperty.call(globalThis, name)) {
                throw new Error("worker private state leaked as own property:" + name);
            }
        }
        globalThis.__workerState = "spoofed";
        globalThis.__moliWorkerGlobalOnInstall = "spoofed";
        if (oninstall !== null) {
            throw new Error("public property spoofing reached private handler state");
        }
        delete globalThis.__workerState;
        delete globalThis.__moliWorkerGlobalOnInstall;

        for (const name of handlerNames) {
            const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
            if (!descriptor ||
                typeof descriptor.get !== "function" ||
                typeof descriptor.set !== "function") {
                throw new Error("missing handler accessor:" + name);
            }
            if (globalThis[name] !== null) {
                throw new Error("initial handler should be null:" + name);
            }
            const marker = { name };
            globalThis[name] = marker;
            if (globalThis[name] !== marker) {
                throw new Error("object handler value was not retained:" + name);
            }
            globalThis[name] = undefined;
            if (globalThis[name] !== null) {
                throw new Error("undefined handler should reset to null:" + name);
            }
        }

        oninstall = event => {
            if (event.type !== "install") {
                throw new Error("unexpected install type:" + event.type);
            }
            event.waitUntil(Promise.resolve());
        };
        onactivate = event => {
            if (event.type !== "activate") {
                throw new Error("unexpected activate type:" + event.type);
            }
            event.waitUntil(Promise.resolve());
        };
        onfetch = event => {
            if (event.type !== "fetch" ||
                event.request.url !== "https://example.test/app/data.txt") {
                throw new Error("unexpected fetch event:" + event.type + "|" + event.request.url);
            }
            event.respondWith(new Response("from-onfetch:" + event.request.url, {
                status: 202,
                statusText: "Accepted"
            }));
        };
        onpush = event => {
            if (event.type !== "push" ||
                !event.data ||
                event.data.text() !== "handler-push") {
                throw new Error("unexpected push event");
            }
            event.waitUntil(Promise.resolve());
        };
        onsync = event => {
            if (event.type !== "sync" ||
                event.tag !== "handler-sync" ||
                event.lastChance !== false) {
                throw new Error("unexpected sync event:" + JSON.stringify({
                    type: event.type,
                    tag: event.tag,
                    lastChance: event.lastChance
                }));
            }
            event.waitUntil(Promise.resolve());
        };
        onperiodicsync = event => {
            if (event.type !== "periodicsync" ||
                event.tag !== "handler-periodic-sync" ||
                "lastChance" in event) {
                throw new Error("unexpected periodic sync event:" + JSON.stringify({
                    type: event.type,
                    tag: event.tag,
                    lastChance: event.lastChance
                }));
            }
            event.waitUntil(Promise.resolve());
        };
        onmessage = event => {
            if (event.type !== "message" || event.data !== "ping") {
                throw new Error("unexpected message event:" + event.type + "|" + event.data);
            }
            event.waitUntil(Promise.resolve());
        };
        onmessageerror = event => {
            if (event.type !== "messageerror" ||
                event.data !== null ||
                event.origin !== "" ||
                event.source !== null ||
                event.ports.length !== 0) {
                throw new Error("unexpected messageerror event");
            }
            event.waitUntil(Promise.resolve());
        };
        onnotificationclick = event => {
            if (event.type !== "notificationclick" ||
                event.notification.title !== "click-title" ||
                event.notification.data.answer !== 42 ||
                event.action !== "open") {
                throw new Error("unexpected notificationclick event");
            }
            event.waitUntil(Promise.resolve());
        };
        onnotificationclose = event => {
            if (event.type !== "notificationclose" ||
                event.notification.title !== "close-title" ||
                event.notification.data.answer !== 7 ||
                event.action !== "") {
                throw new Error("unexpected notificationclose event");
            }
            event.waitUntil(Promise.resolve());
        };
        "#,
    );

    let install_completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        40,
    )
    .await;
    assert_eq!(install_completion.result, Ok(()));

    let activate_completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Activate,
        41,
    )
    .await;
    assert_eq!(activate_completion.result, Ok(()));

    let fetch_completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 42).await;
    let ServiceWorkerFetchResult::Response(response) = fetch_completion.result else {
        panic!("expected onfetch response");
    };
    assert_eq!(response.status, 202);
    assert_eq!(response.status_text, "Accepted");
    assert_eq!(
        response.body,
        b"from-onfetch:https://example.test/app/data.txt".to_vec()
    );

    let push_completion = dispatch_service_worker_push_event_for_test(
        &mut handle,
        47,
        Some(b"handler-push".to_vec()),
    )
    .await;
    assert_eq!(push_completion.result, Ok(()));

    let sync_completion =
        dispatch_service_worker_sync_event_for_test(&mut handle, 50, "handler-sync").await;
    assert_eq!(sync_completion.result, Ok(()));

    let periodic_sync_completion = dispatch_service_worker_periodic_sync_event_for_test(
        &mut handle,
        51,
        "handler-periodic-sync",
    )
    .await;
    assert_eq!(periodic_sync_completion.result, Ok(()));

    let message_completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        43,
        serialize_test_string("ping"),
    )
    .await;
    assert_eq!(message_completion.result, Ok(()));

    let messageerror_completion =
        dispatch_service_worker_message_event_for_test(&mut handle, 44, Default::default()).await;
    assert_eq!(messageerror_completion.result, Ok(()));

    let click_completion = dispatch_service_worker_notification_event_for_test(
        &mut handle,
        ServiceWorkerNotificationEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(45),
            kind: ServiceWorkerNotificationEventKind::Click,
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            notification_id: 1,
            title: "click-title".to_owned(),
            tag: String::new(),
            metadata: ServiceWorkerNotificationMetadata::default(),
            actions: vec![ServiceWorkerNotificationAction {
                action: "open".to_owned(),
                title: "Open".to_owned(),
                icon: String::new(),
                navigate: None,
            }],
            action: "open".to_owned(),
            data: serialize_test_value("({ answer: 42 })"),
        },
    )
    .await;
    assert_eq!(click_completion.result, Ok(()));

    let close_completion = dispatch_service_worker_notification_event_for_test(
        &mut handle,
        ServiceWorkerNotificationEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(46),
            kind: ServiceWorkerNotificationEventKind::Close,
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            generation: 1,
            notification_id: 2,
            title: "close-title".to_owned(),
            tag: String::new(),
            metadata: ServiceWorkerNotificationMetadata::default(),
            actions: Vec::new(),
            action: String::new(),
            data: serialize_test_value("({ answer: 7 })"),
        },
    )
    .await;
    assert_eq!(close_completion.result, Ok(()));

    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_push_event_data_methods_and_wait_until_complete() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        let seenPushEvents = 0;
        self.addEventListener("push", event => {
            seenPushEvents += 1;
            if (seenPushEvents === 1) {
                event.waitUntil((async () => {
                    if (event.type !== "push" || event.data === null) {
                        throw new Error("missing push data");
                    }
                    const text = event.data.text();
                    if (text !== "{\"answer\":42,\"message\":\"hi\"}") {
                        throw new Error("unexpected push text:" + text);
                    }
                    const json = event.data.json();
                    if (json.answer !== 42 || json.message !== "hi") {
                        throw new Error("unexpected push json:" + JSON.stringify(json));
                    }
                    const expectedBytes = Array.from(text)
                        .map(ch => ch.charCodeAt(0))
                        .join(",");
                    const bufferBytes = Array.from(new Uint8Array(event.data.arrayBuffer())).join(",");
                    const bytes = Array.from(event.data.bytes()).join(",");
                    if (bufferBytes !== expectedBytes || bytes !== expectedBytes) {
                        throw new Error("unexpected push bytes:" + bufferBytes + "|" + bytes);
                    }
                })());
                return;
            }
            if (seenPushEvents === 2) {
                if (event.type !== "push" || event.data !== null) {
                    throw new Error("unexpected null push data");
                }
                event.waitUntil(Promise.resolve());
                return;
            }
            if (seenPushEvents === 3) {
                if (event.type !== "push" || event.data === null) {
                    throw new Error("missing invalid push data");
                }
                let caught = null;
                try {
                    event.data.json();
                } catch (error) {
                    caught = error;
                }
                if (!caught || caught.name !== "SyntaxError") {
                    throw new Error("invalid push json did not throw SyntaxError:" + (caught && caught.name));
                }
                event.waitUntil(Promise.resolve());
                return;
            }
            throw new Error("unexpected extra push event");
        });
        "#,
    );

    let payload_completion = dispatch_service_worker_push_event_for_test(
        &mut handle,
        48,
        Some(br#"{"answer":42,"message":"hi"}"#.to_vec()),
    )
    .await;
    assert_eq!(payload_completion.result, Ok(()));

    let null_completion = dispatch_service_worker_push_event_for_test(&mut handle, 49, None).await;
    assert_eq!(null_completion.result, Ok(()));

    let invalid_json_completion =
        dispatch_service_worker_push_event_for_test(&mut handle, 50, Some(b"{".to_vec())).await;
    assert_eq!(invalid_json_completion.result, Ok(()));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_message_wait_until_delays_completion_until_reaction_runs() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        let settled = false;
        self.addEventListener("message", event => {
            event.waitUntil(Promise.resolve().then(() => {
                settled = true;
            }));
            event.waitUntil(Promise.resolve().then(() => {
                if (!settled) {
                    throw new Error("waitUntil reaction did not run before completion");
                }
            }));
        });
        "#,
    );

    let completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        21,
        serialize_test_string("ping"),
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
async fn service_worker_message_wait_until_rejection_reports_failure() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("message", event => {
            event.waitUntil(Promise.reject(new Error("message-boom")));
        });
        "#,
    );

    let completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        22,
        serialize_test_string("ping"),
    )
    .await;

    assert_eq!(
        completion.event_id,
        ServiceWorkerEventId::from_u64_for_worker(22)
    );
    assert_eq!(
        completion.result,
        Err("service worker waitUntil promise rejected".to_owned())
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_respond_with_rejection_fails_fetch() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("fetch", event => {
            event.respondWith(Promise.reject(new Error("fetch-boom")));
        });
        "#,
    );

    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 9).await;

    let ServiceWorkerFetchResult::Failure(message) = completion.result else {
        panic!("expected service worker fetch failure");
    };
    assert!(message.contains("fetch-boom"));
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_skip_waiting_posts_runtime_request() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            event.waitUntil(self.skipWaiting());
        });
        "#,
    );
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(5),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        kind: ServiceWorkerLifecycleEventKind::Install,
    });

    let mut saw_skip_waiting = false;
    while !saw_skip_waiting {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker skipWaiting")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerSkipWaiting {
                registration_id,
                version_id,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(1));
                saw_skip_waiting = true;
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(5)
                );
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_) => {}
            WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. } => {}
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_clients_claim_posts_runtime_request() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("activate", event => {
            event.waitUntil(clients.claim());
        });
        "#,
    );
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(6),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        kind: ServiceWorkerLifecycleEventKind::Activate,
    });

    let mut saw_clients_claim = false;
    while !saw_clients_claim {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker clients.claim")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientsClaim {
                registration_id,
                version_id,
            } => {
                assert_eq!(
                    registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(version_id, ServiceWorkerVersionId::from_u64_for_test(1));
                saw_clients_claim = true;
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(6)
                );
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_) => {}
            WorkerToParentMessage::ServiceWorkerSkipWaiting { .. } => {}
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_clients_match_all_and_get_resolve_from_parent_query_result() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            event.waitUntil((async () => {
                const clientsFromMatchAll = await clients.matchAll({
                    includeUncontrolled: true,
                    type: "all"
                });
                if (clientsFromMatchAll.length !== 1) {
                    throw new Error("unexpected matchAll length:" + clientsFromMatchAll.length);
                }
                if (typeof clients.openWindow !== "function") {
                    throw new Error("clients.openWindow should be a function");
                }
                const first = clientsFromMatchAll[0];
                if (first.id !== "client-000000000000002a" ||
                    first.url !== "https://example.test/app/page.html" ||
                    first.type !== "window" ||
                    first.frameType !== "top-level" ||
                    first.lifecycleState !== "active" ||
                    first.visibilityState !== "visible" ||
                    first.focused !== false ||
                    typeof first.postMessage !== "function" ||
                    typeof first.focus !== "function" ||
                    typeof first.navigate !== "function") {
                    throw new Error("unexpected matchAll client:" + JSON.stringify({
                        id: first.id,
                        url: first.url,
                        type: first.type,
                        frameType: first.frameType,
                        lifecycleState: first.lifecycleState,
                        visibilityState: first.visibilityState,
                        focused: first.focused,
                        postMessage: typeof first.postMessage,
                        focus: typeof first.focus,
                        navigate: typeof first.navigate
                    }));
                }
                const fetched = await clients.get(first.id);
                if (fetched.id !== first.id ||
                    fetched.url !== first.url ||
                    fetched.type !== first.type ||
                    typeof fetched.postMessage !== "function") {
                    throw new Error("unexpected get client");
                }
                try {
                    await clients.openWindow("https://example.test/app/opened.html");
                    throw new Error("openWindow should reject without window interaction");
                } catch (error) {
                    if (error.name !== "InvalidAccessError" ||
                        !(error instanceof DOMException) ||
                        error.message !== "Not allowed to open a window.") {
                        throw new Error("unexpected openWindow rejection:" + JSON.stringify({
                            name: error && error.name,
                            message: error && error.message,
                            isDomException: error instanceof DOMException
                        }));
                    }
                }
                try {
                    await first.focus();
                    throw new Error("focus should reject without window interaction");
                } catch (error) {
                    if (error.name !== "InvalidAccessError" ||
                        !(error instanceof DOMException) ||
                        error.message !== "Not allowed to focus a window.") {
                        throw new Error("unexpected focus rejection:" + JSON.stringify({
                            name: error && error.name,
                            message: error && error.message,
                            isDomException: error instanceof DOMException
                        }));
                    }
                }
                const navigated = await first.navigate("./next.html");
                if (!navigated ||
                    navigated.id !== "client-000000000000002b" ||
                    navigated.url !== "https://example.test/app/next.html" ||
                    navigated.type !== "window" ||
                    navigated.frameType !== "top-level" ||
                    navigated.lifecycleState !== "active" ||
                    navigated.visibilityState !== "visible" ||
                    typeof navigated.postMessage !== "function" ||
                    typeof navigated.focus !== "function" ||
                    typeof navigated.navigate !== "function") {
                    throw new Error("unexpected navigate result:" + JSON.stringify({
                        id: navigated && navigated.id,
                        url: navigated && navigated.url,
                        type: navigated && navigated.type,
                        frameType: navigated && navigated.frameType,
                        lifecycleState: navigated && navigated.lifecycleState,
                        visibilityState: navigated && navigated.visibilityState,
                        postMessage: typeof (navigated && navigated.postMessage),
                        focus: typeof (navigated && navigated.focus),
                        navigate: typeof (navigated && navigated.navigate)
                    }));
                }
            })());
        });
        "#,
    );
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(30),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        kind: ServiceWorkerLifecycleEventKind::Install,
    });

    let mut saw_match_all_query = false;
    let mut saw_get_query = false;
    let mut saw_navigate = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker clients query")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                assert_eq!(
                    query.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    query.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                match query.kind {
                    crate::runtime::ServiceWorkerClientQueryKind::MatchAll { options } => {
                        assert!(!saw_match_all_query);
                        assert_eq!(query.request_id, 1);
                        assert!(options.include_uncontrolled);
                        assert_eq!(
                            options.client_type,
                            crate::runtime::ServiceWorkerClientQueryType::All
                        );
                        saw_match_all_query = true;
                        handle.dispatch_service_worker_client_query_result(
                            crate::runtime::ServiceWorkerClientQueryResult {
                                request_id: query.request_id,
                                clients: vec![
                                    crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                        crate::runtime::ServiceWorkerClientId::from_u64_for_test(
                                            42,
                                        ),
                                        url::Url::parse("https://example.test/app/page.html")
                                            .unwrap(),
                                        false,
                                    ),
                                ],
                            },
                        );
                    }
                    crate::runtime::ServiceWorkerClientQueryKind::Get { exposed_client_id } => {
                        assert!(saw_match_all_query);
                        assert!(!saw_get_query);
                        assert_eq!(query.request_id, 2);
                        assert_eq!(exposed_client_id, "client-000000000000002a");
                        saw_get_query = true;
                        handle.dispatch_service_worker_client_query_result(
                            crate::runtime::ServiceWorkerClientQueryResult {
                                request_id: query.request_id,
                                clients: vec![
                                    crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                        crate::runtime::ServiceWorkerClientId::from_u64_for_test(
                                            42,
                                        ),
                                        url::Url::parse("https://example.test/app/page.html")
                                            .unwrap(),
                                        true,
                                    ),
                                ],
                            },
                        );
                    }
                }
            }
            WorkerToParentMessage::ServiceWorkerClientNavigate(navigate) => {
                assert!(saw_match_all_query);
                assert!(saw_get_query);
                assert!(!saw_navigate);
                assert_eq!(navigate.request_id, 1);
                assert_eq!(
                    navigate.source_version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(
                    navigate.target_client_id,
                    crate::runtime::ServiceWorkerClientId::from_u64_for_worker(42)
                );
                assert_eq!(
                    navigate.url,
                    url::Url::parse("https://example.test/app/next.html").unwrap()
                );
                saw_navigate = true;
                handle.dispatch_service_worker_client_navigate_result(
                    crate::runtime::ServiceWorkerClientNavigateResult {
                        request_id: navigate.request_id,
                        result: Ok(Some(
                            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(43),
                                url::Url::parse("https://example.test/app/next.html").unwrap(),
                                true,
                            ),
                        )),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(completion) => {
                assert!(saw_match_all_query);
                assert!(saw_get_query);
                assert!(saw_navigate);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(30)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerClientFocus(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_worker_client_query_builds_base_client_object() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener("install", event => {
            event.waitUntil((async () => {
                const workerClients = await clients.matchAll({
                    includeUncontrolled: true,
                    type: "worker"
                });
                if (workerClients.length !== 1) {
                    throw new Error("unexpected worker client length:" + workerClients.length);
                }
                const first = workerClients[0];
                if (first.id !== "client-000000000000004d" ||
                    first.url !== "https://example.test/app/dedicated-worker.js" ||
                    first.type !== "worker" ||
                    typeof first.postMessage !== "function" ||
                    typeof first.focus !== "undefined" ||
                    typeof first.navigate !== "undefined" ||
                    "frameType" in first ||
                    "visibilityState" in first ||
                    "focused" in first) {
                    throw new Error("unexpected worker client:" + JSON.stringify({
                        id: first.id,
                        url: first.url,
                        type: first.type,
                        postMessage: typeof first.postMessage,
                        focus: typeof first.focus,
                        navigate: typeof first.navigate,
                        hasFrameType: "frameType" in first,
                        hasVisibilityState: "visibilityState" in first,
                        hasFocused: "focused" in first
                    }));
                }
                const fetched = await clients.get(first.id);
                if (fetched.id !== first.id ||
                    fetched.url !== first.url ||
                    fetched.type !== first.type ||
                    typeof fetched.postMessage !== "function" ||
                    typeof fetched.focus !== "undefined" ||
                    "frameType" in fetched) {
                    throw new Error("unexpected fetched worker client");
                }
            })());
        });
        "#,
    );
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(31),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        kind: ServiceWorkerLifecycleEventKind::Install,
    });

    let mut saw_match_all_query = false;
    let mut saw_get_query = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker worker client query")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => match query.kind {
                crate::runtime::ServiceWorkerClientQueryKind::MatchAll { options } => {
                    assert!(!saw_match_all_query);
                    assert_eq!(query.request_id, 1);
                    assert!(options.include_uncontrolled);
                    assert_eq!(
                        options.client_type,
                        crate::runtime::ServiceWorkerClientQueryType::Worker
                    );
                    saw_match_all_query = true;
                    handle.dispatch_service_worker_client_query_result(
                        crate::runtime::ServiceWorkerClientQueryResult {
                            request_id: query.request_id,
                            clients: vec![worker_client_snapshot_for_test(false)],
                        },
                    );
                }
                crate::runtime::ServiceWorkerClientQueryKind::Get { exposed_client_id } => {
                    assert!(saw_match_all_query);
                    assert!(!saw_get_query);
                    assert_eq!(query.request_id, 2);
                    assert_eq!(exposed_client_id, "client-000000000000004d");
                    saw_get_query = true;
                    handle.dispatch_service_worker_client_query_result(
                        crate::runtime::ServiceWorkerClientQueryResult {
                            request_id: query.request_id,
                            clients: vec![worker_client_snapshot_for_test(true)],
                        },
                    );
                }
            },
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(completion) => {
                assert!(saw_match_all_query);
                assert!(saw_get_query);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(31)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

fn worker_client_snapshot_for_test(
    controlled: bool,
) -> crate::runtime::ServiceWorkerClientSnapshot {
    let mut snapshot = crate::runtime::ServiceWorkerClientSnapshot::dedicated_worker_for_test(
        crate::runtime::ServiceWorkerClientId::from_u64_for_test(77),
        url::Url::parse("https://example.test/app/dedicated-worker.js").unwrap(),
        controlled,
    );
    snapshot.exposed_id = "client-000000000000004d".to_owned();
    snapshot
}

#[tokio::test]
async fn service_worker_open_window_consumes_window_interaction_before_focus() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
    event.waitUntil((async () => {
        const clientsFromMatchAll = await clients.matchAll({
            includeUncontrolled: true
        });
        if (clientsFromMatchAll.length !== 1) {
            throw new Error("unexpected matchAll length:" + clientsFromMatchAll.length);
        }
        const opened = await clients.openWindow("./opened.html");
        if (opened !== null) {
            throw new Error("unexpected openWindow result:" + opened);
        }
        try {
            await clientsFromMatchAll[0].focus();
            throw new Error("focus should reject after openWindow consumed interaction");
        } catch (error) {
            if (error.name !== "InvalidAccessError" ||
                !(error instanceof DOMException) ||
                error.message !== "Not allowed to focus a window.") {
                throw new Error("unexpected consumed focus rejection:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(31),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("ping"),
        window_interaction_allowed: true,
    });

    let mut saw_match_all_query = false;
    let mut saw_open_window = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker openWindow")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                assert!(!saw_match_all_query);
                assert_eq!(query.request_id, 1);
                saw_match_all_query = true;
                handle.dispatch_service_worker_client_query_result(
                    crate::runtime::ServiceWorkerClientQueryResult {
                        request_id: query.request_id,
                        clients: vec![
                            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(42),
                                url::Url::parse("https://example.test/app/page.html").unwrap(),
                                true,
                            ),
                        ],
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerClientsOpenWindow(open_window) => {
                assert!(saw_match_all_query);
                assert!(!saw_open_window);
                assert_eq!(open_window.request_id, 1);
                assert_eq!(
                    open_window.source_version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(
                    open_window.url,
                    url::Url::parse("https://example.test/app/opened.html").unwrap()
                );
                saw_open_window = true;
                handle.dispatch_service_worker_clients_open_window_result(
                    crate::runtime::ServiceWorkerClientsOpenWindowResult {
                        request_id: open_window.request_id,
                        result: Ok(None),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerClientFocus(focus) => {
                panic!(
                    "focus request should not be sent after openWindow consumed interaction: {focus:?}"
                );
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert!(saw_match_all_query);
                assert!(saw_open_window);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(31)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_open_window_rejects_non_http_urls_before_parent_request() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
    event.waitUntil((async () => {
        try {
            await clients.openWindow("about:blank");
            throw new Error("about:blank openWindow should reject");
        } catch (error) {
            if (error.name !== "TypeError" ||
                error instanceof DOMException ||
                error.message !== "'about:blank' cannot be opened.") {
                throw new Error("unexpected about:blank openWindow rejection:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
        const opened = await clients.openWindow("./opened.html");
        if (opened !== null) {
            throw new Error("unexpected openWindow result:" + opened);
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(32),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("ping"),
        window_interaction_allowed: true,
    });

    let mut saw_open_window = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker openWindow rejection")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientsOpenWindow(open_window) => {
                assert!(!saw_open_window);
                assert_eq!(open_window.request_id, 1);
                assert_eq!(
                    open_window.url,
                    url::Url::parse("https://example.test/app/opened.html").unwrap()
                );
                saw_open_window = true;
                handle.dispatch_service_worker_clients_open_window_result(
                    crate::runtime::ServiceWorkerClientsOpenWindowResult {
                        request_id: open_window.request_id,
                        result: Ok(None),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert!(saw_open_window);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(32)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_window_client_navigate_rejects_typed_failures() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
    event.waitUntil((async () => {
        const clientsFromMatchAll = await clients.matchAll({
            includeUncontrolled: true
        });
        if (clientsFromMatchAll.length !== 1) {
            throw new Error("unexpected matchAll length:" + clientsFromMatchAll.length);
        }
        const client = clientsFromMatchAll[0];
        try {
            await client.navigate("about:blank");
            throw new Error("about:blank navigate should reject");
        } catch (error) {
            if (error.name !== "TypeError" ||
                error instanceof DOMException ||
                error.message !== "Failed to execute 'navigate' on 'WindowClient': URL is invalid.") {
                throw new Error("unexpected local navigate failure:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
        try {
            await client.navigate("file:///tmp/moli-window-client-denied.html");
            throw new Error("file navigate should reject");
        } catch (error) {
            if (error.name !== "TypeError" ||
                error instanceof DOMException ||
                error.message !== "'file:///tmp/moli-window-client-denied.html' cannot navigate.") {
                throw new Error("unexpected display-gated navigate failure:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
        try {
            await client.navigate("javascript:1");
            throw new Error("javascript navigate should reject");
        } catch (error) {
            if (error.name !== "TypeError" ||
                error instanceof DOMException ||
                error.message !== "'javascript:1' cannot navigate.") {
                throw new Error("unexpected javascript navigate failure:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
        try {
            await client.navigate("./already-navigating.html");
            throw new Error("parent navigate failure should reject");
        } catch (error) {
            if (error.name !== "TypeError" ||
                error instanceof DOMException ||
                error.message !== "The client is already navigating.") {
                throw new Error("unexpected parent navigate failure:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(33),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("ping"),
        window_interaction_allowed: false,
    });

    let mut saw_match_all_query = false;
    let mut saw_navigate = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker navigate failure")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                assert!(!saw_match_all_query);
                assert_eq!(query.request_id, 1);
                saw_match_all_query = true;
                handle.dispatch_service_worker_client_query_result(
                    crate::runtime::ServiceWorkerClientQueryResult {
                        request_id: query.request_id,
                        clients: vec![
                            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(42),
                                url::Url::parse("https://example.test/app/page.html").unwrap(),
                                true,
                            ),
                        ],
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerClientNavigate(navigate) => {
                assert!(saw_match_all_query);
                assert!(!saw_navigate);
                assert_eq!(navigate.request_id, 1);
                assert_eq!(
                    navigate.url,
                    url::Url::parse("https://example.test/app/already-navigating.html").unwrap()
                );
                saw_navigate = true;
                handle.dispatch_service_worker_client_navigate_result(
                    crate::runtime::ServiceWorkerClientNavigateResult {
                        request_id: navigate.request_id,
                        result: Err(
                            crate::runtime::ServiceWorkerClientNavigateError::type_error(
                                "The client is already navigating.",
                            ),
                        ),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert!(saw_match_all_query);
                assert!(saw_navigate);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(33)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerClientFocus(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_open_window_rejects_typed_parent_failure() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
    event.waitUntil((async () => {
        try {
            await clients.openWindow("./opened.html");
            throw new Error("openWindow should reject parent failure");
        } catch (error) {
            if (error.name !== "TypeError" ||
                error instanceof DOMException ||
                error.message !== "No live window client is available to host openWindow().") {
                throw new Error("unexpected openWindow parent failure:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(32),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("ping"),
        window_interaction_allowed: true,
    });

    let mut saw_open_window = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker openWindow failure")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientsOpenWindow(open_window) => {
                assert!(!saw_open_window);
                assert_eq!(open_window.request_id, 1);
                assert_eq!(
                    open_window.url,
                    url::Url::parse("https://example.test/app/opened.html").unwrap()
                );
                saw_open_window = true;
                handle.dispatch_service_worker_clients_open_window_result(
                    crate::runtime::ServiceWorkerClientsOpenWindowResult {
                        request_id: open_window.request_id,
                        result: Err(
                            crate::runtime::ServiceWorkerClientsOpenWindowError::type_error(
                                "No live window client is available to host openWindow().",
                            ),
                        ),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert!(saw_open_window);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(32)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_window_client_focus_rejects_typed_parent_failures() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
    event.waitUntil((async () => {
        const clientsFromMatchAll = await clients.matchAll({
            includeUncontrolled: true
        });
        if (clientsFromMatchAll.length !== 1) {
            throw new Error("unexpected matchAll length:" + clientsFromMatchAll.length);
        }
        let expected;
        if (event.data === "not-found") {
            expected = {
                name: "NotFoundError",
                message: "The client was not found.",
                isDomException: true,
                isTypeError: false
            };
        } else if (event.data === "inactive") {
            expected = {
                name: "TypeError",
                message: "The client is inactive.",
                isDomException: false,
                isTypeError: true
            };
        } else {
            throw new Error("unexpected message data:" + event.data);
        }
        try {
            await clientsFromMatchAll[0].focus();
            throw new Error("focus unexpectedly resolved");
        } catch (error) {
            const actual = {
                name: error && error.name,
                message: error && error.message,
                isDomException: error instanceof DOMException,
                isTypeError: error instanceof TypeError
            };
            if (JSON.stringify(actual) !== JSON.stringify(expected)) {
                throw new Error("unexpected focus failure:" + JSON.stringify(actual));
            }
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(32),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("not-found"),
        window_interaction_allowed: true,
    });

    let mut client_queries = 0;
    let mut focus_requests = 0;
    let mut completions = 0;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker focus rejection")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                client_queries += 1;
                assert_eq!(query.request_id, client_queries);
                handle.dispatch_service_worker_client_query_result(
                    crate::runtime::ServiceWorkerClientQueryResult {
                        request_id: query.request_id,
                        clients: vec![
                            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(42),
                                url::Url::parse("https://example.test/app/page.html").unwrap(),
                                true,
                            ),
                        ],
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerClientFocus(focus) => {
                assert_eq!(client_queries, focus_requests + 1);
                focus_requests += 1;
                assert_eq!(focus.request_id, focus_requests);
                assert_eq!(
                    focus.source_version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(
                    focus.target_client_id,
                    crate::runtime::ServiceWorkerClientId::from_u64_for_worker(42)
                );
                let result = if focus_requests == 1 {
                    Err(crate::runtime::ServiceWorkerClientFocusError::not_found())
                } else {
                    Err(crate::runtime::ServiceWorkerClientFocusError::type_error(
                        "The client is inactive.",
                    ))
                };
                handle.dispatch_service_worker_client_focus_result(
                    crate::runtime::ServiceWorkerClientFocusResult {
                        request_id: focus.request_id,
                        result,
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                completions += 1;
                assert_eq!(completion.result, Ok(()));
                if completions == 1 {
                    assert_eq!(client_queries, 1);
                    assert_eq!(focus_requests, 1);
                    assert_eq!(
                        completion.event_id,
                        ServiceWorkerEventId::from_u64_for_worker(32)
                    );
                    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
                        event_id: ServiceWorkerEventId::from_u64_for_worker(33),
                        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                        generation: 1,
                        source_client_id: None,
                        source_client_url: None,
                        source_client_snapshot: None,
                        source_worker: None,
                        source_origin: String::new(),
                        payload: serialize_test_string("inactive"),
                        window_interaction_allowed: true,
                    });
                } else {
                    assert_eq!(completions, 2);
                    assert_eq!(client_queries, 2);
                    assert_eq!(focus_requests, 2);
                    assert_eq!(
                        completion.event_id,
                        ServiceWorkerEventId::from_u64_for_worker(33)
                    );
                    break;
                }
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
            WorkerToParentMessage::ServiceWorkerClientsOpenWindow(open_window) => {
                panic!(
                    "openWindow request should not be sent after focus consumed interaction: {open_window:?}"
                );
            }
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_show_notification_requires_notification_permission() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
self.addEventListener("message", event => {
  event.waitUntil(self.registration.showNotification("denied").then(
    () => { throw new Error("showNotification unexpectedly resolved"); },
    error => {
      if (!error || error.name !== "TypeError") {
        throw new Error("unexpected rejection:" + (error && error.name));
      }
    }
  ));
});
"#,
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(33),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("show"),
        window_interaction_allowed: false,
    });

    let mut saw_completion = false;
    while !saw_completion {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for denied showNotification")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(33)
                );
                assert_eq!(completion.result, Ok(()));
                saw_completion = true;
            }
            WorkerToParentMessage::ServiceWorkerShowNotification(_) => {
                panic!("denied showNotification must not reach runtime owner");
            }
            WorkerToParentMessage::ServiceWorkerGetNotifications(_)
            | WorkerToParentMessage::ServiceWorkerCloseNotification(_) => {}
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_show_notification_sends_runtime_record_request_when_granted() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil(self.registration.showNotification("hello", {
    body: "Worker body",
    icon: "/worker-icon.png",
    image: "/worker-image.png",
    badge: "/worker-badge.png",
    dir: "rtl",
    lang: "fr",
    vibrate: [5, 6],
    timestamp: 777,
    renotify: true,
    silent: false,
    requireInteraction: true,
    data: { answer: 42 }
  }));
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_network_policy(WorkerNetworkPolicy {
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("notifications".to_owned()),
                setting: "granted".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            ..WorkerNetworkPolicy::default()
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(34),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("show"),
        window_interaction_allowed: false,
    });

    let request_id = loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for showNotification")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerShowNotification(request) => {
                assert_eq!(request.request_id, 1);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(request.title, "hello");
                assert_eq!(request.tag, "");
                assert_eq!(request.metadata.body, "Worker body");
                assert_eq!(request.metadata.icon, "/worker-icon.png");
                assert_eq!(request.metadata.image, "/worker-image.png");
                assert_eq!(request.metadata.badge, "/worker-badge.png");
                assert_eq!(request.metadata.dir, "rtl");
                assert_eq!(request.metadata.lang, "fr");
                assert_eq!(request.metadata.vibrate, vec![5, 6]);
                assert_eq!(request.metadata.timestamp, Some(777));
                assert!(request.metadata.renotify);
                assert_eq!(request.metadata.silent, Some(false));
                assert!(request.metadata.require_interaction);
                assert_eq!(
                    inspect_payload(&request.data, "JSON.stringify(__wire)"),
                    r#"{"answer":42}"#
                );
                break request.request_id;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                panic!("showNotification completed before owner ack: {completion:?}");
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    };

    assert!(
        timeout(std::time::Duration::from_millis(50), handle.recv())
            .await
            .is_err(),
        "showNotification promise should wait for owner result"
    );

    handle.dispatch_service_worker_show_notification_result(ServiceWorkerShowNotificationResult {
        request_id,
        result: Ok(()),
    });

    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for showNotification owner result")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(34)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_get_notifications_resolves_from_parent_and_close_posts_request() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    const notifications = await self.registration.getNotifications({ tag: "same" });
    if (notifications.length !== 1 ||
        notifications[0].title !== "stored" ||
        notifications[0].tag !== "same" ||
        notifications[0].actions.length !== 1 ||
        notifications[0].actions[0].action !== "reply" ||
        notifications[0].actions[0].title !== "Reply" ||
        notifications[0].actions[0].icon !== "/reply.png" ||
        notifications[0].data.answer !== 7 ||
        typeof notifications[0].close !== "function") {
      throw new Error("unexpected notifications:" + JSON.stringify({
        length: notifications.length,
        title: notifications[0] && notifications[0].title,
        tag: notifications[0] && notifications[0].tag,
        actions: notifications[0] && notifications[0].actions,
        answer: notifications[0] && notifications[0].data && notifications[0].data.answer,
        close: notifications[0] && typeof notifications[0].close
      }));
    }
    notifications[0].close();
  })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(35),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("get"),
        window_interaction_allowed: false,
    });

    let mut saw_get_request = false;
    let mut saw_close_request = false;
    let mut saw_completion = false;
    while !(saw_get_request && saw_close_request && saw_completion) {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for getNotifications")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerGetNotifications(request) => {
                assert!(!saw_get_request);
                assert_eq!(request.request_id, 1);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(request.tag.as_deref(), Some("same"));
                handle.dispatch_service_worker_get_notifications_result(
                    ServiceWorkerGetNotificationsResult {
                        request_id: request.request_id,
                        result: Ok(vec![ServiceWorkerNotificationSnapshot {
                            id: 17,
                            registration_id: request.registration_id,
                            title: "stored".to_owned(),
                            tag: "same".to_owned(),
                            metadata: ServiceWorkerNotificationMetadata::default(),
                            actions: vec![ServiceWorkerNotificationAction {
                                action: "reply".to_owned(),
                                title: "Reply".to_owned(),
                                icon: "/reply.png".to_owned(),
                                navigate: None,
                            }],
                            data: serialize_test_value("({ answer: 7 })"),
                        }]),
                    },
                );
                saw_get_request = true;
            }
            WorkerToParentMessage::ServiceWorkerCloseNotification(request) => {
                assert!(saw_get_request);
                assert!(!saw_close_request);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(request.notification_id, 17);
                saw_close_request = true;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(35)
                );
                assert_eq!(
                    completion.result,
                    Ok(()),
                    "saw_get_request={saw_get_request} saw_close_request={saw_close_request}"
                );
                saw_completion = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_registration_sync_resolves_from_parent_results() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    if (!self.registration.sync ||
        typeof self.registration.sync.register !== "function" ||
        typeof self.registration.sync.getTags !== "function") {
      throw new Error("missing registration.sync surface");
    }
    const before = await self.registration.sync.getTags();
    if (before.length !== 0) {
      throw new Error("unexpected sync tags before register:" + before.join(","));
    }
    const registerValue = await self.registration.sync.register("worker-sync");
    if (registerValue !== undefined) {
      throw new Error("sync.register should resolve undefined");
    }
    const after = await self.registration.sync.getTags();
    if (after.length !== 1 || after[0] !== "worker-sync") {
      throw new Error("unexpected sync tags after register:" + after.join(","));
    }
  })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(36),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("sync"),
        window_interaction_allowed: false,
    });

    let mut saw_first_get_tags = false;
    let mut saw_register = false;
    let mut saw_second_get_tags = false;
    let mut saw_completion = false;
    while !(saw_first_get_tags && saw_register && saw_second_get_tags && saw_completion) {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker registration sync")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerSyncGetTags(request) => {
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                if !saw_first_get_tags {
                    assert_eq!(request.request_id, 1);
                    handle.dispatch_service_worker_sync_get_tags_result(
                        crate::runtime::ServiceWorkerSyncGetTagsResult {
                            request_id: request.request_id,
                            result: Ok(Vec::new()),
                        },
                    );
                    saw_first_get_tags = true;
                } else {
                    assert!(saw_register);
                    assert!(!saw_second_get_tags);
                    assert_eq!(request.request_id, 2);
                    handle.dispatch_service_worker_sync_get_tags_result(
                        crate::runtime::ServiceWorkerSyncGetTagsResult {
                            request_id: request.request_id,
                            result: Ok(vec!["worker-sync".to_owned()]),
                        },
                    );
                    saw_second_get_tags = true;
                }
            }
            WorkerToParentMessage::ServiceWorkerSyncRegistration(request) => {
                assert!(saw_first_get_tags);
                assert!(!saw_register);
                assert_eq!(request.request_id, 1);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(request.tag, "worker-sync");
                handle.dispatch_service_worker_sync_registration_result(
                    crate::runtime::ServiceWorkerSyncRegistrationResult {
                        request_id: request.request_id,
                        result: Ok(()),
                    },
                );
                saw_register = true;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(36)
                );
                assert_eq!(
                    completion.result,
                    Ok(()),
                    "first_get={saw_first_get_tags} register={saw_register} second_get={saw_second_get_tags}"
                );
                saw_completion = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_registration_navigation_preload_surface_is_exposed() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    const manager = self.registration.navigationPreload;
    event.source.postMessage(JSON.stringify({
      hasManager: !!manager,
      instance: manager instanceof NavigationPreloadManager,
      enable: typeof manager.enable,
      disable: typeof manager.disable,
      setHeaderValue: typeof manager.setHeaderValue,
      getState: typeof manager.getState
    }));
  })());
});
"#,
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(136),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: Some(crate::runtime::ServiceWorkerClientId::from_u64_for_test(1)),
        source_client_url: Some(url::Url::parse("https://example.test/app/page.html").unwrap()),
        source_client_snapshot: Some(
            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                crate::runtime::ServiceWorkerClientId::from_u64_for_test(1),
                url::Url::parse("https://example.test/app/page.html").unwrap(),
                true,
            ),
        ),
        source_worker: None,
        source_origin: "https://example.test".to_owned(),
        payload: serialize_test_string("navigation-preload"),
        window_interaction_allowed: false,
    });

    let mut saw_client_message = false;
    let mut saw_completion = false;
    while !(saw_client_message && saw_completion) {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for navigation preload surface")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientMessage(request) => {
                assert_eq!(
                    stringify_payload(&request.payload),
                    r#""{\"hasManager\":true,\"instance\":true,\"enable\":\"function\",\"disable\":\"function\",\"setHeaderValue\":\"function\",\"getState\":\"function\"}""#
                );
                saw_client_message = true;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(136)
                );
                assert_eq!(completion.result, Ok(()));
                saw_completion = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker navigation preload error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected navigation preload message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_navigation_preload_get_state_rejects_without_runtime() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    try {
      await self.registration.navigationPreload.getState();
      event.source.postMessage("resolved");
    } catch (error) {
      event.source.postMessage(JSON.stringify({
        name: error && error.name,
        isDomException: error instanceof DOMException,
        message: error && error.message
      }));
    }
  })());
});
"#,
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(137),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: Some(crate::runtime::ServiceWorkerClientId::from_u64_for_test(1)),
        source_client_url: Some(url::Url::parse("https://example.test/app/page.html").unwrap()),
        source_client_snapshot: Some(
            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                crate::runtime::ServiceWorkerClientId::from_u64_for_test(1),
                url::Url::parse("https://example.test/app/page.html").unwrap(),
                true,
            ),
        ),
        source_worker: None,
        source_origin: "https://example.test".to_owned(),
        payload: serialize_test_string("navigation-preload"),
        window_interaction_allowed: false,
    });

    let mut saw_client_message = false;
    let mut saw_completion = false;
    while !(saw_client_message && saw_completion) {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for navigation preload getState rejection")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientMessage(request) => {
                assert_eq!(
                    stringify_payload(&request.payload),
                    r#""{\"name\":\"InvalidStateError\",\"isDomException\":true,\"message\":\"Registration failed - no active Service Worker\"}""#
                );
                saw_client_message = true;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(137)
                );
                assert_eq!(completion.result, Ok(()));
                saw_completion = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker navigation preload error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected navigation preload rejection message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_periodic_sync_resolves_from_parent_results() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    if (!self.registration.periodicSync ||
        typeof self.registration.periodicSync.register !== "function" ||
        typeof self.registration.periodicSync.getTags !== "function" ||
        typeof self.registration.periodicSync.unregister !== "function") {
      throw new Error("missing registration.periodicSync surface");
    }
    const before = await self.registration.periodicSync.getTags();
    if (before.length !== 0) {
      throw new Error("unexpected periodic sync tags before register:" + before.join(","));
    }
    const registerValue = await self.registration.periodicSync.register("periodic-worker", {
      minInterval: 60000
    });
    if (registerValue !== undefined) {
      throw new Error("periodicSync.register should resolve undefined");
    }
    const mid = await self.registration.periodicSync.getTags();
    if (mid.length !== 1 || mid[0] !== "periodic-worker") {
      throw new Error("unexpected periodic sync tags after register:" + mid.join(","));
    }
    const unregisterValue = await self.registration.periodicSync.unregister("periodic-worker");
    if (unregisterValue !== undefined) {
      throw new Error("periodicSync.unregister should resolve undefined");
    }
    const after = await self.registration.periodicSync.getTags();
    if (after.length !== 0) {
      throw new Error("unexpected periodic sync tags after unregister:" + after.join(","));
    }
  })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(39),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("periodic-sync"),
        window_interaction_allowed: false,
    });

    let mut get_tags_count = 0;
    let mut saw_register = false;
    let mut saw_unregister = false;
    let mut saw_completion = false;
    while !(get_tags_count == 3 && saw_register && saw_unregister && saw_completion) {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker periodic sync")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(request) => {
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                get_tags_count += 1;
                let tags = match get_tags_count {
                    1 => Vec::new(),
                    2 => {
                        assert!(saw_register);
                        vec!["periodic-worker".to_owned()]
                    }
                    3 => {
                        assert!(saw_unregister);
                        Vec::new()
                    }
                    other => panic!("unexpected periodic sync getTags request #{other}"),
                };
                handle.dispatch_service_worker_periodic_sync_get_tags_result(
                    crate::runtime::ServiceWorkerPeriodicSyncGetTagsResult {
                        request_id: request.request_id,
                        result: Ok(tags),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(request) => {
                assert_eq!(get_tags_count, 1);
                assert!(!saw_register);
                assert_eq!(request.request_id, 1);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(request.tag, "periodic-worker");
                assert_eq!(request.min_interval_ms, 60_000);
                handle.dispatch_service_worker_periodic_sync_registration_result(
                    crate::runtime::ServiceWorkerPeriodicSyncRegistrationResult {
                        request_id: request.request_id,
                        result: Ok(()),
                    },
                );
                saw_register = true;
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(request) => {
                assert_eq!(get_tags_count, 2);
                assert!(saw_register);
                assert!(!saw_unregister);
                assert_eq!(request.request_id, 1);
                assert_eq!(request.tag, "periodic-worker");
                handle.dispatch_service_worker_periodic_sync_unregistration_result(
                    crate::runtime::ServiceWorkerPeriodicSyncUnregistrationResult {
                        request_id: request.request_id,
                        result: Ok(()),
                    },
                );
                saw_unregister = true;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(39)
                );
                assert_eq!(
                    completion.result,
                    Ok(()),
                    "get_tags={get_tags_count} register={saw_register} unregister={saw_unregister}"
                );
                saw_completion = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_sync_register_rejects_when_background_sync_denied() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    let caught = null;
    try {
      await self.registration.sync.register("denied-sync");
    } catch (error) {
      caught = {
        name: error && error.name,
        message: error && error.message,
        isDomException: error instanceof DOMException
      };
    }
    if (!caught ||
        caught.name !== "NotAllowedError" ||
        caught.message !== "Background Sync permission has not been granted." ||
        caught.isDomException !== true) {
      throw new Error("unexpected sync permission result:" + JSON.stringify(caught));
    }
  })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_network_policy(WorkerNetworkPolicy {
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("background-sync".to_owned()),
                setting: "denied".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            ..WorkerNetworkPolicy::default()
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(38),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("sync"),
        window_interaction_allowed: false,
    });

    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for denied service worker sync register")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(38)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::ServiceWorkerSyncRegistration(request) => {
                panic!("denied background sync should not reach parent: {request:?}");
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_periodic_sync_register_rejects_when_permission_denied() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    let caught = null;
    try {
      await self.registration.periodicSync.register("denied-periodic", {
        minInterval: 60000
      });
    } catch (error) {
      caught = {
        name: error && error.name,
        message: error && error.message,
        isDomException: error instanceof DOMException
      };
    }
    if (!caught ||
        caught.name !== "NotAllowedError" ||
        caught.message !== "Periodic Background Sync permission has not been granted." ||
        caught.isDomException !== true) {
      throw new Error("unexpected periodic sync permission result:" + JSON.stringify(caught));
    }
  })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_network_policy(WorkerNetworkPolicy {
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("periodic-background-sync".to_owned()),
                setting: "denied".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            ..WorkerNetworkPolicy::default()
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(40),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("periodic-sync"),
        window_interaction_allowed: false,
    });

    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for denied service worker periodic sync register")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(40)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(request) => {
                panic!("denied periodic sync should not reach parent: {request:?}");
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_push_manager_subscription_resolves_from_parent_results() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    if (!self.registration.pushManager ||
        typeof self.registration.pushManager.subscribe !== "function" ||
        typeof self.registration.pushManager.getSubscription !== "function" ||
        typeof self.registration.pushManager.permissionState !== "function") {
      throw new Error("missing registration.pushManager surface");
    }
    const permission = await self.registration.pushManager.permissionState();
    if (permission !== "granted") {
      throw new Error("unexpected push permission:" + permission);
    }
    const before = await self.registration.pushManager.getSubscription();
    if (before !== null) {
      throw new Error("unexpected subscription before subscribe");
    }
    const sub = await self.registration.pushManager.subscribe({ userVisibleOnly: true });
    const options = sub.options;
    const userVisibleOnlyDescriptor =
      Object.getOwnPropertyDescriptor(options, "userVisibleOnly");
    const applicationServerKeyDescriptor =
      Object.getOwnPropertyDescriptor(options, "applicationServerKey");
    const readonlyWriteErrors = [];
    for (const [key, value] of [
      ["userVisibleOnly", false],
      ["applicationServerKey", "mutated"]
    ]) {
      try {
        (() => {
          "use strict";
          options[key] = value;
        })();
      } catch (error) {
        readonlyWriteErrors.push(`${key}:${error && error.name}`);
      }
    }
    const setterHits = [];
    for (const key of ["endpoint", "expirationTime", "options"]) {
      Object.defineProperty(Object.prototype, key, {
        configurable: true,
        set(value) { setterHits.push(`${key}:${typeof value}`); }
      });
    }
    let json;
    try {
      json = sub.toJSON();
    } finally {
      for (const key of ["endpoint", "expirationTime", "options"]) {
        delete Object.prototype[key];
      }
    }
    const endpointDescriptor = Object.getOwnPropertyDescriptor(json, "endpoint");
    if (sub.endpoint !== "https://moli.invalid/service-worker/push/1" ||
        sub.expirationTime !== null ||
        sub.options.userVisibleOnly !== true ||
        sub.options.applicationServerKey !== null ||
        !userVisibleOnlyDescriptor ||
        userVisibleOnlyDescriptor.writable !== false ||
        !applicationServerKeyDescriptor ||
        applicationServerKeyDescriptor.writable !== false ||
        readonlyWriteErrors.join(",") !==
          "userVisibleOnly:TypeError,applicationServerKey:TypeError" ||
        typeof sub.unsubscribe !== "function" ||
        json.endpoint !== sub.endpoint ||
        json.expirationTime !== null ||
        json.options.userVisibleOnly !== true ||
        !endpointDescriptor ||
        endpointDescriptor.value !== sub.endpoint ||
        endpointDescriptor.writable !== true ||
        endpointDescriptor.enumerable !== true ||
        endpointDescriptor.configurable !== true ||
        setterHits.length !== 0) {
      throw new Error("unexpected push subscription:" + JSON.stringify({
        endpoint: sub && sub.endpoint,
        expirationTime: sub && sub.expirationTime,
        options: sub && sub.options,
        userVisibleOnlyDescriptor,
        applicationServerKeyDescriptor,
        readonlyWriteErrors,
        json,
        endpointDescriptor,
        setterHits
      }));
    }
    const unsubscribed = await sub.unsubscribe();
    if (unsubscribed !== true) {
      throw new Error("unexpected unsubscribe result:" + unsubscribed);
    }
    const after = await self.registration.pushManager.getSubscription();
    if (after !== null) {
      throw new Error("unexpected subscription after unsubscribe");
    }
  })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_network_policy(WorkerNetworkPolicy {
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("notifications".to_owned()),
                setting: "granted".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            ..WorkerNetworkPolicy::default()
        }),
    );

    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(37),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("push-manager"),
        window_interaction_allowed: false,
    });

    let subscription = crate::runtime::ServiceWorkerPushSubscriptionSnapshot {
        endpoint: "https://moli.invalid/service-worker/push/1".to_owned(),
        user_visible_only: true,
    };
    let mut saw_first_get = false;
    let mut saw_subscribe = false;
    let mut saw_unsubscribe = false;
    let mut saw_second_get = false;
    let mut saw_completion = false;
    while !(saw_first_get && saw_subscribe && saw_unsubscribe && saw_second_get && saw_completion) {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker push manager")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerPushGetSubscription(request) => {
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                if !saw_first_get {
                    assert_eq!(request.request_id, 1);
                    handle.dispatch_service_worker_push_get_subscription_result(
                        crate::runtime::ServiceWorkerPushGetSubscriptionResult {
                            request_id: request.request_id,
                            result: Ok(None),
                        },
                    );
                    saw_first_get = true;
                } else {
                    assert!(saw_subscribe);
                    assert!(saw_unsubscribe);
                    assert!(!saw_second_get);
                    assert_eq!(request.request_id, 2);
                    handle.dispatch_service_worker_push_get_subscription_result(
                        crate::runtime::ServiceWorkerPushGetSubscriptionResult {
                            request_id: request.request_id,
                            result: Ok(None),
                        },
                    );
                    saw_second_get = true;
                }
            }
            WorkerToParentMessage::ServiceWorkerPushSubscribe(request) => {
                assert!(saw_first_get);
                assert!(!saw_subscribe);
                assert_eq!(request.request_id, 1);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert!(request.user_visible_only);
                handle.dispatch_service_worker_push_subscribe_result(
                    crate::runtime::ServiceWorkerPushSubscribeResult {
                        request_id: request.request_id,
                        result: Ok(subscription.clone()),
                    },
                );
                saw_subscribe = true;
            }
            WorkerToParentMessage::ServiceWorkerPushUnsubscribe(request) => {
                assert!(saw_subscribe);
                assert!(!saw_unsubscribe);
                assert_eq!(request.request_id, 1);
                assert_eq!(
                    request.registration_id,
                    ServiceWorkerRegistrationId::from_u64_for_test(1)
                );
                assert_eq!(
                    request.version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                handle.dispatch_service_worker_push_unsubscribe_result(
                    crate::runtime::ServiceWorkerPushUnsubscribeResult {
                        request_id: request.request_id,
                        result: Ok(true),
                    },
                );
                saw_unsubscribe = true;
            }
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(37)
                );
                assert_eq!(
                    completion.result,
                    Ok(()),
                    "first_get={saw_first_get} subscribe={saw_subscribe} unsubscribe={saw_unsubscribe} second_get={saw_second_get}"
                );
                saw_completion = true;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_notificationclick_grants_window_interaction_for_focus() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("notificationclick", event => {
    event.waitUntil((async () => {
        if (event.type !== "notificationclick" ||
            event.notification.title !== "hello" ||
            event.notification.data.answer !== 42 ||
            event.notification.actions.length !== 1 ||
            event.notification.actions[0].action !== "open" ||
            event.notification.actions[0].title !== "Open" ||
            event.notification.actions[0].icon !== "/open.png" ||
            event.notification.body !== "Click body" ||
            event.notification.icon !== "/click-icon.png" ||
            event.notification.image !== "/click-image.png" ||
            event.notification.badge !== "/click-badge.png" ||
            event.notification.dir !== "ltr" ||
            event.notification.lang !== "en" ||
            Array.from(event.notification.vibrate).join("/") !== "9/10" ||
            event.notification.timestamp !== 888 ||
            event.notification.renotify !== true ||
            event.notification.silent !== true ||
            event.notification.requireInteraction !== true ||
            event.action !== "open") {
            throw new Error("unexpected notificationclick event:" + JSON.stringify({
                type: event.type,
                title: event.notification && event.notification.title,
                data: event.notification && event.notification.data,
                actions: event.notification && event.notification.actions,
                body: event.notification && event.notification.body,
                icon: event.notification && event.notification.icon,
                image: event.notification && event.notification.image,
                badge: event.notification && event.notification.badge,
                dir: event.notification && event.notification.dir,
                lang: event.notification && event.notification.lang,
                vibrate: event.notification && Array.from(event.notification.vibrate).join("/"),
                timestamp: event.notification && event.notification.timestamp,
                renotify: event.notification && event.notification.renotify,
                silent: event.notification && event.notification.silent,
                requireInteraction: event.notification && event.notification.requireInteraction,
                action: event.action
            }));
        }
        const clientsFromMatchAll = await clients.matchAll({
            includeUncontrolled: true
        });
        if (clientsFromMatchAll.length !== 1) {
            throw new Error("unexpected matchAll length:" + clientsFromMatchAll.length);
        }
        const focused = await clientsFromMatchAll[0].focus();
        if (focused.id !== "client-000000000000002a" || focused.focused !== true) {
            throw new Error("unexpected focus result:" + JSON.stringify({
                id: focused && focused.id,
                focused: focused && focused.focused
            }));
        }
        try {
            await clients.openWindow("./after-focus.html");
            throw new Error("openWindow should reject after focus consumed interaction");
        } catch (error) {
            if (error.name !== "InvalidAccessError" ||
                !(error instanceof DOMException) ||
                error.message !== "Not allowed to open a window.") {
                throw new Error("unexpected consumed openWindow error:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_notification_event(ServiceWorkerNotificationEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(32),
        kind: ServiceWorkerNotificationEventKind::Click,
        registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        notification_id: 1,
        title: "hello".to_owned(),
        tag: String::new(),
        metadata: ServiceWorkerNotificationMetadata {
            dir: "ltr".to_owned(),
            lang: "en".to_owned(),
            body: "Click body".to_owned(),
            icon: "/click-icon.png".to_owned(),
            image: "/click-image.png".to_owned(),
            badge: "/click-badge.png".to_owned(),
            vibrate: vec![9, 10],
            timestamp: Some(888),
            renotify: true,
            silent: Some(true),
            require_interaction: true,
        },
        actions: vec![ServiceWorkerNotificationAction {
            action: "open".to_owned(),
            title: "Open".to_owned(),
            icon: "/open.png".to_owned(),
            navigate: None,
        }],
        action: "open".to_owned(),
        data: serialize_test_value("({ answer: 42 })"),
    });

    let mut saw_match_all_query = false;
    let mut saw_focus = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker notificationclick")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                assert!(!saw_match_all_query);
                assert_eq!(query.request_id, 1);
                saw_match_all_query = true;
                handle.dispatch_service_worker_client_query_result(
                    crate::runtime::ServiceWorkerClientQueryResult {
                        request_id: query.request_id,
                        clients: vec![
                            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(42),
                                url::Url::parse("https://example.test/app/page.html").unwrap(),
                                true,
                            ),
                        ],
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerClientFocus(focus) => {
                assert!(saw_match_all_query);
                assert!(!saw_focus);
                assert_eq!(focus.request_id, 1);
                assert_eq!(
                    focus.source_version_id,
                    ServiceWorkerVersionId::from_u64_for_test(1)
                );
                assert_eq!(
                    focus.target_client_id,
                    crate::runtime::ServiceWorkerClientId::from_u64_for_worker(42)
                );
                saw_focus = true;
                handle.dispatch_service_worker_client_focus_result(
                    crate::runtime::ServiceWorkerClientFocusResult {
                        request_id: focus.request_id,
                        result: Ok(
                            crate::runtime::ServiceWorkerClientSnapshot::focused_window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(42),
                                url::Url::parse("https://example.test/app/page.html").unwrap(),
                                true,
                            ),
                        ),
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerNotificationCompleted(completion) => {
                assert!(saw_match_all_query);
                assert!(saw_focus);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(32)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
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
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_notificationclose_does_not_grant_window_interaction() {
    ensure_v8();
    let (bootstrap_tx, mut bootstrap_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
self.addEventListener("notificationclose", event => {
    event.waitUntil((async () => {
        if (event.type !== "notificationclose" ||
            event.notification.title !== "bye" ||
            event.notification.data.answer !== 7 ||
            event.action !== "") {
            throw new Error("unexpected notificationclose event:" + JSON.stringify({
                type: event.type,
                title: event.notification && event.notification.title,
                data: event.notification && event.notification.data,
                action: event.action
            }));
        }
        const windows = await clients.matchAll({ includeUncontrolled: true });
        if (windows.length !== 1) {
            throw new Error("unexpected matchAll length:" + windows.length);
        }
        try {
            await windows[0].focus();
            throw new Error("focus unexpectedly resolved");
        } catch (error) {
            if (error.name !== "InvalidAccessError" ||
                !(error instanceof DOMException) ||
                error.message !== "Not allowed to focus a window.") {
                throw new Error("unexpected focus error:" + JSON.stringify({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                }));
            }
        }
    })());
});
"#
            .to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        })
        .with_bootstrap_completion_sender(bootstrap_tx),
    );
    let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv())
        .await
        .expect("timed out waiting for service worker bootstrap")
        .expect("service worker bootstrap channel closed");
    assert!(
        bootstrap.result.is_ok(),
        "bootstrap failed: {:?}",
        bootstrap.result
    );

    handle.dispatch_service_worker_notification_event(ServiceWorkerNotificationEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(33),
        kind: ServiceWorkerNotificationEventKind::Close,
        registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
        version_id: ServiceWorkerVersionId::from_u64_for_test(1),
        generation: 1,
        notification_id: 2,
        title: "bye".to_owned(),
        tag: String::new(),
        metadata: ServiceWorkerNotificationMetadata::default(),
        actions: Vec::new(),
        action: String::new(),
        data: serialize_test_value("({ answer: 7 })"),
    });

    let mut saw_match_all_query = false;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker notificationclose")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerClientQuery(query) => {
                assert!(!saw_match_all_query);
                saw_match_all_query = true;
                handle.dispatch_service_worker_client_query_result(
                    crate::runtime::ServiceWorkerClientQueryResult {
                        request_id: query.request_id,
                        clients: vec![
                            crate::runtime::ServiceWorkerClientSnapshot::window_for_test(
                                crate::runtime::ServiceWorkerClientId::from_u64_for_test(42),
                                url::Url::parse("https://example.test/app/page.html").unwrap(),
                                true,
                            ),
                        ],
                    },
                );
            }
            WorkerToParentMessage::ServiceWorkerClientFocus(_) => {
                panic!("notificationclose must not grant focus permission");
            }
            WorkerToParentMessage::ServiceWorkerNotificationCompleted(completion) => {
                assert!(saw_match_all_query);
                assert_eq!(
                    completion.event_id,
                    ServiceWorkerEventId::from_u64_for_worker(33)
                );
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error: {message}");
            }
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
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
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_fetch_rejects_preaborted_signal_with_dom_exception() {
    ensure_v8();
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const controller = new AbortController();
            controller.abort();
            try {
                await fetch("https://example.com/data.txt", { signal: controller.signal });
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                });
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"AbortError","message":"The operation was aborted.","isDomException":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_rejects_inflight_abort_signal_and_ignores_late_completion() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "slow worker fetch".to_owned(),
        Duration::from_millis(150),
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const controller = new AbortController();
            const events = [];
            const pending = fetch("./data.txt", { signal: controller.signal });
            setTimeout(() => controller.abort(), 0);
            try {
                await pending;
                events.push("unexpected");
            } catch (error) {
                events.push(`error:${error && error.name}:${error instanceof DOMException}:${error && error.message}`);
            }
            await new Promise((resolve) => setTimeout(resolve, 250));
            postMessage(events);
            close();
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"["error:AbortError:true:The operation was aborted."]"#
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn worker_fetch_body_consumption_after_stream_abort_preserves_abort_reason() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed worker fetch abort server");
    let addr = listener
        .local_addr()
        .expect("delayed worker fetch abort addr");
    let (release_body_tx, release_body_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept delayed worker fetch abort request");
        let _request = read_http_request_head(&mut stream)
            .await
            .expect("read delayed worker fetch abort request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/wasm\r\nContent-Length: 11\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write delayed worker fetch abort headers");
        let _ = release_body_rx.await;
        let _ = stream.write_all(b"hello world").await;
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const controller = new AbortController();
            const response = await fetch("./delayed.wasm", { signal: controller.signal });
            controller.abort();
            await Promise.resolve();
            try {
                await response.arrayBuffer();
                postMessage({ phase: "unexpected" });
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    message: error && error.message,
                    isDomException: error instanceof DOMException
                });
            }
            close();
        })();
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"name":"AbortError","message":"The operation was aborted.","isDomException":true}"#
    );
    let _ = release_body_tx.send(());
    server
        .await
        .expect("delayed worker fetch abort server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_abort_cancels_inflight_request_and_ignores_late_completion() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "slow worker xhr".to_owned(),
        Duration::from_millis(150),
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('abort', () => events.push('abort'));
            xhr.addEventListener('error', () => events.push(`error:${xhr.readyState}:${xhr.status}`));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => events.push(`loadend:${xhr.readyState}:${xhr.status}`));
            xhr.addEventListener('loadend', () => {
                setTimeout(() => {
                    postMessage({
                        readyState: xhr.readyState,
                        status: xhr.status,
                        responseURL: xhr.responseURL,
                        events,
                    });
                    close();
                }, 250);
            });
            xhr.open('GET', './data.txt');
            xhr.send();
            setTimeout(() => xhr.abort(), 0);
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":0,"status":0,"responseURL":"","events":["abort","loadend:4:0"]}"#
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn worker_xmlhttprequest_timeout_cancels_inflight_request_and_ignores_late_completion() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "slow worker xhr".to_owned(),
        Duration::from_millis(150),
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push(`readystatechange:${xhr.readyState}`));
            xhr.addEventListener('timeout', () => events.push('timeout'));
            xhr.addEventListener('error', () => events.push(`error:${xhr.readyState}:${xhr.status}`));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {
                events.push(`loadend:${xhr.readyState}:${xhr.status}`);
                setTimeout(() => {
                    postMessage({
                        readyState: xhr.readyState,
                        status: xhr.status,
                        statusText: xhr.statusText,
                        responseText: xhr.responseText,
                        responseURL: xhr.responseURL,
                        contentType: xhr.getResponseHeader('Content-Type'),
                        allHeaders: xhr.getAllResponseHeaders(),
                        events,
                    });
                    close();
                }, 250);
            });
            xhr.open('GET', './data.txt');
            xhr.timeout = 1000;
            xhr.send();
            xhr.timeout = 20;
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":"","events":["readystatechange:1","readystatechange:4","timeout","loadend:4:0"]}"#
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn worker_shared_event_targets_honor_abort_signal_options() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const targets = [
                self,
                new FileReader(),
                new XMLHttpRequest(),
            ];
            const calls = [];
            const controller = new AbortController();
            const alreadyAborted = new AbortController();
            alreadyAborted.abort("before-registration");

            // EventTarget registration uses the signal's internal abort
            // algorithms, not its overridable public addEventListener method.
            controller.signal.addEventListener = () => {
                throw new Error("public signal.addEventListener must not be called");
            };
            targets.forEach((target, index) => {
                target.addEventListener(
                    "probe",
                    () => calls.push(`aborted:${index}`),
                    { signal: controller.signal }
                );
                target.addEventListener(
                    "probe",
                    () => calls.push(`already-aborted:${index}`),
                    { signal: alreadyAborted.signal }
                );
                target.addEventListener(
                    "probe",
                    () => calls.push(`live:${index}`)
                );
            });

            let invalidSignalError = null;
            try {
                targets[0].addEventListener("probe", () => {}, { signal: {} });
            } catch (error) {
                invalidSignalError = error.name;
            }

            controller.abort("after-registration");
            targets.forEach(target => target.dispatchEvent(new Event("probe")));
            postMessage({ calls, invalidSignalError });
            close();
        })();
        "#
        .into(),
        "test://worker-shared-event-target-abort-signal".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"calls":["live:0","live:1","live:2"],"invalidSignalError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_abort_controller_dispatches_abort_and_throwifaborted() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const controller = new AbortController();
            const events = [];
            controller.signal.addEventListener('abort', () => events.push('listener'));
            controller.signal.onabort = () => events.push('onabort');
            controller.abort('worker-abort');
            let thrown = null;
            let thrownMatchesReason = false;
            try {
                controller.signal.throwIfAborted();
            } catch (error) {
                thrown = error;
                thrownMatchesReason = error === controller.signal.reason;
            }
            postMessage({
                aborted: controller.signal.aborted,
                reason: String(controller.signal.reason),
                events,
                thrown: String(thrown),
                thrownType: typeof thrown,
                thrownMatchesReason,
                isEventTarget: controller.signal instanceof EventTarget,
                tag: Object.prototype.toString.call(controller.signal)
            });
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
        r#"{"aborted":true,"reason":"worker-abort","events":["listener","onabort"],"thrown":"worker-abort","thrownType":"string","thrownMatchesReason":true,"isEventTarget":true,"tag":"[object AbortSignal]"}"#
    );
}

#[tokio::test]
async fn worker_abort_controller_default_reason_is_dom_exception_on_live_prototype() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const controller = new AbortController();
            const protoAborted = Object.getOwnPropertyDescriptor(AbortSignal.prototype, 'aborted');
            const protoReason = Object.getOwnPropertyDescriptor(AbortSignal.prototype, 'reason');
            const protoOnabort = Object.getOwnPropertyDescriptor(AbortSignal.prototype, 'onabort');
            const protoSignal =
                Object.getOwnPropertyDescriptor(AbortController.prototype, 'signal');
            let forgedSignal;
            try {
                protoSignal.get.call({});
                forgedSignal = "ok";
            } catch (error) {
                forgedSignal = error && error.name;
            }
            controller.abort();
            let thrown = null;
            try {
                controller.signal.throwIfAborted();
            } catch (error) {
                thrown = error;
            }
            postMessage({
                aborted: controller.signal.aborted,
                ownAborted: Object.prototype.hasOwnProperty.call(controller.signal, 'aborted'),
                ownReason: Object.prototype.hasOwnProperty.call(controller.signal, 'reason'),
                ownOnabort: Object.prototype.hasOwnProperty.call(controller.signal, 'onabort'),
                ownSignal: Object.prototype.hasOwnProperty.call(controller, 'signal'),
                protoAbortedGetter: typeof protoAborted.get,
                protoReasonGetter: typeof protoReason.get,
                protoOnabortGetter: typeof protoOnabort.get,
                protoOnabortSetter: typeof protoOnabort.set,
                protoSignalGetter: `${protoSignal.get.name}:${protoSignal.get.length}`,
                protoSignalSetter: typeof protoSignal.set,
                borrowedSignal: protoSignal.get.call(controller) === controller.signal,
                forgedSignal,
                reasonName: controller.signal.reason && controller.signal.reason.name,
                reasonMessage: controller.signal.reason && controller.signal.reason.message,
                reasonIsDomException: controller.signal.reason instanceof DOMException,
                thrownName: thrown && thrown.name,
                thrownMessage: thrown && thrown.message,
                thrownIsDomException: thrown instanceof DOMException
            });
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
        r#"{"aborted":true,"ownAborted":false,"ownReason":false,"ownOnabort":false,"ownSignal":false,"protoAbortedGetter":"function","protoReasonGetter":"function","protoOnabortGetter":"function","protoOnabortSetter":"function","protoSignalGetter":"get signal:0","protoSignalSetter":"undefined","borrowedSignal":true,"forgedSignal":"TypeError","reasonName":"AbortError","reasonMessage":"The operation was aborted.","reasonIsDomException":true,"thrownName":"AbortError","thrownMessage":"The operation was aborted.","thrownIsDomException":true}"#
    );
}

#[tokio::test]
async fn worker_readable_stream_pipe_to_honors_abort_signal() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const events = [];
            const reason = new Error("worker-pipe-abort");
            const abortController = new AbortController();
            const readable = new ReadableStream({
                pull() {
                    events.push("pull");
                },
                cancel(value) {
                    events.push("cancel:" + (value === reason));
                }
            }, { highWaterMark: 0 });
            const writable = new WritableStream({
                abort(value) {
                    events.push("abort:" + (value === reason));
                }
            });
            readable.pipeTo(writable, { signal: abortController.signal }).then(
                () => events.push("pipe:fulfilled"),
                value => {
                    events.push(
                        "pipe:" + (value === reason) + ":" + readable.locked + ":" + writable.locked
                    );
                    postMessage(events.join("|"));
                    close();
                }
            );
            Promise.resolve().then(() => abortController.abort(reason));
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
        r#""pull|abort:true|cancel:true|pipe:true:false:false""#
    );
}

#[tokio::test]
async fn worker_abort_signal_timeout_fires_abort_event() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const signal = AbortSignal.timeout(0);
            signal.addEventListener('abort', () => {
                postMessage({
                    aborted: signal.aborted,
                    reasonName: signal.reason && signal.reason.name,
                    reasonMessage: signal.reason && signal.reason.message,
                    reasonIsDomException: signal.reason instanceof DOMException,
                    tag: Object.prototype.toString.call(signal)
                });
                close();
            });
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
        r#"{"aborted":true,"reasonName":"TimeoutError","reasonMessage":"signal timed out","reasonIsDomException":true,"tag":"[object AbortSignal]"}"#
    );
}

#[tokio::test]
async fn worker_abort_signal_any_tracks_source_abort() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const first = new AbortController();
            const second = new AbortController();
            const composite = AbortSignal.any([first.signal, second.signal]);
            composite.onabort = () => {
                postMessage({
                    aborted: composite.aborted,
                    reason: String(composite.reason),
                    isEventTarget: composite instanceof EventTarget
                });
                close();
            };
            second.abort('second-abort');
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
        r#"{"aborted":true,"reason":"second-abort","isEventTarget":true}"#
    );
}

#[tokio::test]
async fn worker_abort_signal_internal_id_is_not_page_visible_or_forgeable() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const controller = new AbortController();
            const signal = controller.signal;
            const forged = { __lmWorkerAbortSignalId: signal.__lmWorkerAbortSignalId ?? 1 };
            const abortedGetter = Object.getOwnPropertyDescriptor(AbortSignal.prototype, "aborted").get;
            const probe = callback => {
                try {
                    callback();
                    return "ok";
                } catch (error) {
                    return error && error.name;
                }
            };
            postMessage({
                hasVisibleSlot: "__lmWorkerAbortSignalId" in signal,
                ownNames: Object.getOwnPropertyNames(signal),
                anyForged: probe(() => AbortSignal.any([forged])),
                getterForged: abortedGetter.call(forged)
            });
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
        r#"{"hasVisibleSlot":false,"ownNames":[],"anyForged":"TypeError","getterForged":false}"#
    );
}

#[tokio::test]
async fn worker_abort_listener_can_abort_another_controller_reentrantly() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const first = new AbortController();
            const second = new AbortController();
            const events = [];
            second.signal.addEventListener('abort', () => events.push('second-listener'));
            first.signal.addEventListener('abort', () => {
                events.push('first-listener');
                second.abort('second-abort');
                events.push(`second-state:${second.signal.aborted}:${String(second.signal.reason)}`);
            });
            first.abort('first-abort');
            postMessage({
                firstAborted: first.signal.aborted,
                secondAborted: second.signal.aborted,
                secondReason: String(second.signal.reason),
                events,
            });
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
        r#"{"firstAborted":true,"secondAborted":true,"secondReason":"second-abort","events":["first-listener","second-listener","second-state:true:second-abort"]}"#
    );
}

#[tokio::test]
async fn worker_abort_signal_dispatch_event_listener_can_mutate_listeners_reentrantly() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const controller = new AbortController();
            const signal = controller.signal;
            let status = 'start';
            function original() {
                status = 'listener-ran';
                signal.removeEventListener('custom', original);
                signal.addEventListener('custom', () => {
                    status += '|late';
                });
                controller.abort('custom-abort');
                status += `|after-abort:${signal.aborted}`;
            }
            signal.addEventListener('custom', original);
            signal.dispatchEvent(new Event('custom'));
            postMessage({
                status,
                aborted: signal.aborted,
                reason: String(signal.reason),
            });
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
        r#"{"status":"listener-ran|after-abort:true","aborted":true,"reason":"custom-abort"}"#
    );
}

#[tokio::test]
async fn worker_abort_signal_dispatch_event_honors_once_listener_option() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const signal = new AbortController().signal;
            let count = 0;
            signal.addEventListener('custom', () => {
                count += 1;
            }, { once: true });
            signal.dispatchEvent(new Event('custom'));
            signal.dispatchEvent(new Event('custom'));
            postMessage(count);
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
    assert_eq!(expect_post_json(msg), "1");
}

#[tokio::test]
async fn worker_abort_signal_listeners_use_event_listener_callback_interface_semantics() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const signal = new AbortController().signal;
            const calls = [];
            let callableHandleEventLookups = 0;
            function callable(event) {
                "use strict";
                calls.push(`callable:${this === signal}:${event.currentTarget === signal}`);
            }
            Object.defineProperty(callable, "handleEvent", {
                get() {
                    callableHandleEventLookups += 1;
                    throw new Error("callable handleEvent must not be read");
                }
            });

            let objectVersion = 1;
            const objectListener = {
                get handleEvent() {
                    const version = objectVersion;
                    return function(event) {
                        calls.push(
                            `object:${version}:${this === objectListener}:` +
                            `${event.target === signal}`
                        );
                    };
                }
            };

            signal.addEventListener("custom", callable);
            // A duplicate registration must not replace the first record's options.
            signal.addEventListener("custom", callable, { once: true });
            signal.addEventListener("custom", objectListener);
            signal.dispatchEvent(new Event("custom"));
            objectVersion = 2;
            signal.dispatchEvent(new Event("custom"));
            signal.removeEventListener("custom", callable);
            signal.removeEventListener("custom", objectListener);

            let captureCalls = 0;
            function captureListener() {
                captureCalls += 1;
            }
            signal.addEventListener("capture", captureListener, false);
            signal.addEventListener("capture", captureListener, true);
            signal.removeEventListener("capture", captureListener, false);
            signal.dispatchEvent(new Event("capture"));

            const mutationSignal = new AbortController().signal;
            const mutationCalls = [];
            function removedBeforeTurn() {
                mutationCalls.push("removed");
            }
            mutationSignal.addEventListener("mutation", () => {
                mutationCalls.push("first");
                mutationSignal.removeEventListener("mutation", removedBeforeTurn);
                mutationSignal.addEventListener("mutation", () => mutationCalls.push("late"));
            });
            mutationSignal.addEventListener("mutation", removedBeforeTurn);
            mutationSignal.dispatchEvent(new Event("mutation"));

            const nestedSignal = new AbortController().signal;
            let onceCalls = 0;
            nestedSignal.addEventListener("nested", () => {
                onceCalls += 1;
                nestedSignal.dispatchEvent(new Event("nested"));
            }, { once: true });
            nestedSignal.dispatchEvent(new Event("nested"));

            const passiveSignal = new AbortController().signal;
            const passiveEvent = new Event("passive", { cancelable: true });
            passiveSignal.addEventListener("passive", event => event.preventDefault(), {
                passive: true
            });
            const passiveDispatchResult = passiveSignal.dispatchEvent(passiveEvent);

            const stopController = new AbortController();
            const stopped = [];
            stopController.signal.addEventListener("abort", event => {
                stopped.push("first");
                event.stopImmediatePropagation();
            });
            stopController.signal.addEventListener("abort", () => stopped.push("second"));
            stopController.signal.onabort = () => stopped.push("onabort");
            stopController.abort();

            let primitiveError = null;
            try {
                signal.addEventListener("primitive", 1);
            } catch (error) {
                primitiveError = error && error.name;
            }
            signal.addEventListener("nullable", null);

            postMessage({
                calls,
                callableHandleEventLookups,
                captureCalls,
                mutationCalls,
                onceCalls,
                passiveDispatchResult,
                passiveDefaultPrevented: passiveEvent.defaultPrevented,
                stopped,
                primitiveError,
            });
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
        r#"{"calls":["callable:true:true","object:1:true:true","callable:true:true","object:2:true:true"],"callableHandleEventLookups":0,"captureCalls":1,"mutationCalls":["first"],"onceCalls":1,"passiveDispatchResult":true,"passiveDefaultPrevented":false,"stopped":["first"],"primitiveError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_abort_signal_listener_errors_use_worker_error_reporting() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const errors = [];
            self.onerror = message => {
                errors.push(String(message));
                return true;
            };
            const signal = new AbortController().signal;
            const lookupFailure = {
                get handleEvent() {
                    throw new RangeError("worker-abort-lookup");
                }
            };
            signal.addEventListener("lookup", lookupFailure);
            signal.dispatchEvent(new Event("lookup"));

            const revocable = Proxy.revocable(() => {}, {});
            signal.addEventListener("revoked", revocable.proxy);
            revocable.revoke();
            signal.dispatchEvent(new Event("revoked"));

            postMessage({
                count: errors.length,
                lookup: errors.some(value => value.includes("worker-abort-lookup")),
                revoked: errors.some(value => value.toLowerCase().includes("revoked")),
            });
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
        r#"{"count":2,"lookup":true,"revoked":true}"#
    );
}

#[tokio::test]
async fn worker_message_port_listeners_use_event_listener_callback_interface_semantics() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const calls = [];
            const channel = new MessageChannel();
            const { port1, port2 } = channel;

            const objectListener = {
                handleEvent() {
                    throw new Error("MessagePort must resolve the replacement operation");
                }
            };
            port1.addEventListener("message", objectListener);
            port1.addEventListener("message", objectListener, { once: true });
            objectListener.handleEvent = function(event) {
                calls.push(`object:${event.data}:${this === objectListener}`);
            };

            let callableHandleEventLookups = 0;
            function callable(event) {
                "use strict";
                calls.push(
                    `callable:${event.data}:${this === port1}:` +
                    `${event.currentTarget === port1}`
                );
            }
            Object.defineProperty(callable, "handleEvent", {
                get() {
                    callableHandleEventLookups += 1;
                    throw new Error("callable MessagePort listeners must not resolve handleEvent");
                }
            });
            port1.addEventListener("message", callable);
            port1.addEventListener("message", event => {
                calls.push(`once:${event.data}`);
            }, { once: true });

            function removedBeforeTurn(event) {
                calls.push(`removed:${event.data}`);
            }
            port1.addEventListener("message", event => {
                calls.push(`remove:${event.data}`);
                port1.removeEventListener("message", removedBeforeTurn);
            });
            port1.addEventListener("message", removedBeforeTurn);

            const late = event => calls.push(`late:${event.data}`);
            port1.addEventListener("message", event => {
                calls.push(`add:${event.data}`);
                port1.addEventListener("message", late);
            });
            port1.onmessage = event => {
                calls.push(`handler:${event.data}`);
                if (event.data === "second") {
                    Promise.resolve().then(() => {
                        postMessage({ calls, callableHandleEventLookups });
                        close();
                    });
                }
            };
            port1.start();
            port2.postMessage("first");
            port2.postMessage("second");
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/message-port-callback-interface.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        concat!(
            r#"{"calls":["object:first:true","callable:first:true:true","once:first","#,
            r#""remove:first","add:first","handler:first","object:second:true","#,
            r#""callable:second:true:true","remove:second","add:second","handler:second","#,
            r#""late:second"],"callableHandleEventLookups":0}"#
        )
    );
}

#[tokio::test]
async fn worker_message_port_listener_signal_controls_the_exact_registration() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const calls = [];
            const channel = new MessageChannel();
            const { port1, port2 } = channel;
            const primary = new AbortController();
            const duplicate = new AbortController();
            const onceSignal = new AbortController();
            let replacement;

            const signaled = event => calls.push(`signal:${event.data}`);
            const once = event => calls.push(`once:${event.data}`);
            primary.signal.addEventListener = () => {
                throw new Error("public AbortSignal.addEventListener must not be consulted");
            };
            port1.addEventListener("message", signaled, { signal: primary.signal });
            port1.addEventListener("message", signaled, { signal: duplicate.signal });
            port1.addEventListener("message", once, {
                once: true,
                signal: onceSignal.signal
            });

            const alreadyAborted = new AbortController();
            alreadyAborted.abort();
            port1.addEventListener("message", () => {
                calls.push("already-aborted");
            }, { signal: alreadyAborted.signal });

            let invalidSignalThrew = false;
            try {
                port1.addEventListener("message", () => {}, { signal: {} });
            } catch (error) {
                invalidSignalThrew = error instanceof TypeError;
            }

            port1.onmessage = event => {
                calls.push(`base:${event.data}`);
                if (event.data === "first") {
                    duplicate.abort();
                    onceSignal.abort();
                    replacement = new AbortController();
                    port1.addEventListener("message", once, {
                        once: true,
                        signal: replacement.signal
                    });
                    port2.postMessage("second");
                } else if (event.data === "second") {
                    // This handler precedes the listener added above in event
                    // order. Abort after the current dispatch so that the
                    // replacement once-listener observes this event first.
                    Promise.resolve().then(() => {
                        primary.abort();
                        replacement.abort();
                        port2.postMessage("third");
                    });
                } else {
                    postMessage({ calls, invalidSignalThrew });
                    close();
                }
            };
            port1.start();
            port2.postMessage("first");
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/message-port-listener-signal.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        concat!(
            r#"{"calls":["signal:first","once:first","base:first","signal:second","#,
            r#""base:second","once:second","base:third"],"invalidSignalThrew":true}"#
        )
    );
}

#[tokio::test]
async fn worker_message_port_listener_errors_use_worker_error_reporting() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            self.onerror = message => {
                postMessage({
                    isMessagePortError: String(message).includes("worker-message-port-lookup")
                });
                close();
                return true;
            };
            const channel = new MessageChannel();
            channel.port1.addEventListener("message", {
                get handleEvent() {
                    throw new RangeError("worker-message-port-lookup");
                }
            });
            channel.port1.start();
            channel.port2.postMessage("trigger");
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/message-port-callback-error.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"isMessagePortError":true}"#);
}

// ─── setInterval ────────────────────────────────────────────────────

#[tokio::test]
async fn worker_setinterval() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let count = 0;
        let id = setInterval(function() {
            count++;
            postMessage(count);
            if (count >= 3) {
                clearInterval(id);
                close();
            }
        }, 50);
        "#
        .into(),
        "test://setinterval".into(),
    );

    for expected in 1..=3 {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(expect_post_json(msg), expected.to_string());
    }
}

#[tokio::test]
async fn worker_message_listener_receives_data_without_onmessage() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        addEventListener("message", function(event) {
            postMessage(`listener:${event.data}`);
            close();
        });
        "#
        .into(),
        "test://message_listener".into(),
    );

    handle.post_message(serialize_test_string("ping"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""listener:ping""#);
}

#[tokio::test]
async fn worker_message_listener_receives_messageevent_instance() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        addEventListener("message", function(event) {
            postMessage({
                isMessageEvent: event instanceof MessageEvent,
                typeString: Object.prototype.toString.call(event),
                data: event.data
            });
            close();
        });
        "#
        .into(),
        "test://message_event_instance".into(),
    );

    handle.post_message(serialize_test_string("ping"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"isMessageEvent":true,"typeString":"[object MessageEvent]","data":"ping"}"#
    );
}

#[tokio::test]
async fn file_reader_sync_declared_methods_preserve_descriptors_and_reads() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const methods = [
            "readAsText",
            "readAsDataURL",
            "readAsArrayBuffer",
            "readAsBinaryString"
        ];
        const descriptors = methods.map(name => {
            const descriptor =
                Object.getOwnPropertyDescriptor(FileReaderSync.prototype, name);
            return [
                name,
                typeof descriptor?.value,
                descriptor?.value?.name,
                descriptor?.value?.length,
                descriptor?.enumerable,
                descriptor?.writable,
                descriptor?.configurable
            ].join(":");
        });
        const reader = new FileReaderSync();
        const blob = new Blob(["abc"], { type: "text/plain" });
        const buffer = reader.readAsArrayBuffer(blob);
        postMessage({
            descriptors,
            text: reader.readAsText(blob),
            dataUrlOk:
                reader.readAsDataURL(blob) === "data:text/plain;base64,YWJj",
            binary: reader.readAsBinaryString(blob),
            byteLength: buffer.byteLength
        });
        close();
        "#
        .into(),
        "test://filereadersync-declared-methods".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"descriptors":["readAsText:function:readAsText:1:true:true:true","readAsDataURL:function:readAsDataURL:1:true:true:true","readAsArrayBuffer:function:readAsArrayBuffer:1:true:true:true","readAsBinaryString:function:readAsBinaryString:1:true:true:true"],"text":"abc","dataUrlOk":true,"binary":"abc","byteLength":3}"#
    );
}

#[tokio::test]
async fn worker_unsupported_worker_related_constructors_throw_not_supported() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const describeUnsupported = (name, construct) => {
            let errorName = "";
            let errorCode = 0;
            let errorMessage = "";
            try {
                construct();
            } catch (error) {
                errorName = error && error.name;
                errorCode = error && error.code;
                errorMessage = error && error.message;
            }
            const ctor = globalThis[name];
            return [
                typeof ctor,
                ctor && ctor.name,
                ctor && ctor.length,
                Object.getPrototypeOf(ctor) === EventTarget,
                Object.getPrototypeOf(ctor.prototype) === EventTarget.prototype,
                Object.prototype.toString.call(Object.create(ctor.prototype)),
                errorName,
                errorCode,
                errorMessage
            ].join("|");
        };

        postMessage({
            eventSource: describeUnsupported(
                "EventSource",
                () => new EventSource("/events")
            ),
            sharedWorker: typeof SharedWorker
        });
        close();
        "#
        .into(),
        "test://unsupported_worker_related_constructors".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"eventSource":"function|EventSource|1|true|true|[object EventSource]|NotSupportedError|9|This constructor is not implemented in dedicated workers yet.","sharedWorker":"undefined"}"#
    );
}

#[tokio::test]
async fn nested_worker_drops_message_dispatched_before_listener_is_added() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const child = new Worker(
            "data:text/javascript," +
            encodeURIComponent(`
                onmessage = () => postMessage('after-listener');
                postMessage('early');
                throw new Error('early-dispatched');
            `)
        );
        child.onerror = event => {
            event.preventDefault();
            child.onerror = null;
            child.onmessage = event => {
                postMessage(`child:${event.data}`);
                close();
            };
            child.postMessage("go");
        };
        "#
        .into(),
        "test://nested_worker_no_replay".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""child:after-listener""#);
}

#[tokio::test]
async fn worker_global_unhandledrejection_event_dispatches_after_microtask_checkpoint() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onunhandledrejection = event => {
            const defaultPreventedBefore = event.defaultPrevented;
            event.preventDefault();
            postMessage({
                type: event.type,
                reason: event.reason,
                promise: event.promise instanceof Promise,
                cancelable: event.cancelable,
                defaultPreventedBefore,
                defaultPreventedAfter: event.defaultPrevented
            });
            close();
        };
        Promise.reject("worker-boom");
        "#
        .into(),
        "test://worker_unhandledrejection".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"type":"unhandledrejection","reason":"worker-boom","promise":true,"cancelable":true,"defaultPreventedBefore":false,"defaultPreventedAfter":true}"#
    );
}

#[tokio::test]
async fn worker_global_unhandledrejection_fallback_event_preserves_event_shape() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        self.PromiseRejectionEvent = undefined;
        onunhandledrejection = event => {
            const defaultPreventedBefore = event.defaultPrevented;
            event.preventDefault();
            postMessage({
                type: event.type,
                reason: event.reason,
                promise: event.promise instanceof Promise,
                defaultPreventedBefore,
                defaultPreventedAfter: event.defaultPrevented,
                preventDefaultType: typeof event.preventDefault,
                preventDefaultEnumerable: Object.prototype.propertyIsEnumerable.call(event, "preventDefault"),
                defaultPreventedEnumerable: Object.prototype.propertyIsEnumerable.call(event, "defaultPrevented"),
                typeEnumerable: Object.prototype.propertyIsEnumerable.call(event, "type")
            });
            close();
        };
        Promise.reject("fallback-worker-boom");
        "#
        .into(),
        "test://worker_unhandledrejection_fallback_event".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"type":"unhandledrejection","reason":"fallback-worker-boom","promise":true,"defaultPreventedBefore":false,"defaultPreventedAfter":true,"preventDefaultType":"function","preventDefaultEnumerable":true,"defaultPreventedEnumerable":true,"typeEnumerable":true}"#
    );
}

#[tokio::test]
async fn worker_global_rejectionhandled_event_dispatches_for_late_handler() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const rejected = Promise.reject("late-worker-boom");
        onunhandledrejection = event => {
            event.preventDefault();
            setTimeout(() => rejected.catch(() => {}), 0);
        };
        onrejectionhandled = event => {
            postMessage({
                type: event.type,
                reason: event.reason,
                samePromise: event.promise === rejected,
                cancelable: event.cancelable
            });
            close();
        };
        "#
        .into(),
        "test://worker_rejectionhandled".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"type":"rejectionhandled","reason":"late-worker-boom","samePromise":true,"cancelable":false}"#
    );
}

#[tokio::test]
async fn worker_unhandledrejection_notification_allows_one_message_port_turn() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const events = [];
        let rejected;
        addEventListener("unhandledrejection", event => {
            if (event.promise === rejected) {
                event.preventDefault();
                events.push(event.type);
            }
        });
        const channel = new MessageChannel();
        channel.port1.onmessage = () => {
            rejected.catch(() => {});
            setTimeout(() => {
                postMessage(events);
                close();
            }, 10);
        };
        rejected = Promise.reject("handled-before-notification");
        channel.port2.postMessage("attach");
        "#
        .into(),
        "test://worker_unhandledrejection_one_port_turn".into(),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#"[]"#);
}

#[tokio::test]
async fn worker_unhandledrejection_notification_precedes_nested_message_port_turn() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let rejected;
        addEventListener("unhandledrejection", event => {
            if (event.promise === rejected) {
                event.preventDefault();
                postMessage(event.type);
                close();
            }
        });
        const first = new MessageChannel();
        first.port1.onmessage = () => {
            const second = new MessageChannel();
            second.port1.onmessage = () => rejected.catch(() => {});
            second.port2.postMessage("too-late");
        };
        rejected = Promise.reject("nested-turn");
        first.port2.postMessage("queue-nested");
        "#
        .into(),
        "test://worker_unhandledrejection_nested_port_turn".into(),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""unhandledrejection""#);
}

#[tokio::test]
async fn worker_create_image_bitmap_rejects_invalid_blob_with_dom_exception() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            try {
                await createImageBitmap(new Blob());
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    isDomException: error instanceof DOMException
                });
            }
            close();
        })();
        "#
        .into(),
        "test://worker_create_image_bitmap".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"name":"InvalidStateError","isDomException":true}"#
    );
}

#[tokio::test]
async fn worker_message_port_transfers_readable_stream() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const stream = new ReadableStream({
            start(controller) {
                controller.enqueue("a");
                controller.close();
            }
        });
        const channel = new MessageChannel();
        channel.port1.onmessage = async event => {
            const reader = event.data.getReader();
            const first = await reader.read();
            const second = await reader.read();
            postMessage({
                originalLocked: stream.locked,
                instance: event.data instanceof ReadableStream,
                value: first.value,
                firstDone: first.done,
                secondDone: second.done
            });
            close();
        };
        channel.port2.postMessage(stream, [stream]);
        "#
        .into(),
        "test://worker_message_port_readable_stream_transfer".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"originalLocked":true,"instance":true,"value":"a","firstDone":false,"secondDone":true}"#
    );
}

#[tokio::test]
async fn shared_worker_global_unhandledrejection_event_dispatches_after_connect_task() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                onunhandledrejection = event => {
                    if (event.type === "unhandledrejection" &&
                        event.reason === "shared-boom" &&
                        event.promise instanceof Promise &&
                        event.cancelable) {
                        event.preventDefault();
                        close();
                    }
                };
                Promise.reject("shared-boom");
            };
            "#
            .into(),
            "https://app.example/shared-worker.js".into(),
        )
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        }),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn worker_fetch_csp_block_dispatches_securitypolicyviolation_event() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            fetch("https://api.example/data").catch(error => {
                postMessage({
                    events,
                    error: error && error.message
                });
                close();
            });
            "#
            .into(),
            "https://app.example/worker.js".into(),
        )
        .with_content_security_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"events":[{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"https://api.example/data","documentURI":"https://app.example/worker.js","originalPolicy":"connect-src 'none'","disposition":"enforce","instance":true}],"error":"fetch: blocked by Content Security Policy for `https://api.example/data`."}"#
    );
}

#[tokio::test]
async fn worker_fetch_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "report-only fetch ok".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("worker fetch report-only loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const events = [];
                addEventListener("securitypolicyviolation", event => {
                    events.push({
                        type: event.type,
                        effectiveDirective: event.effectiveDirective,
                        violatedDirective: event.violatedDirective,
                        blockedURI: event.blockedURI,
                        documentURI: event.documentURI,
                        originalPolicy: event.originalPolicy,
                        disposition: event.disposition,
                        instance: event instanceof SecurityPolicyViolationEvent
                    });
                });
                const response = await fetch("./data.txt");
                postMessage({ events, text: await response.text() });
                close();
            })().catch(error => {
                postMessage({ error: String(error), stack: error && error.stack });
                close();
            });
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader)
        .with_content_security_report_only_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"{base_url}/worker/data.txt","documentURI":"{script_url}","originalPolicy":"connect-src 'none'","disposition":"report","instance":true}}],"text":"report-only fetch ok"}}"#
        )
    );
    server
        .await
        .expect("worker fetch report-only server should finish");
}

#[tokio::test]
async fn worker_fetch_csp_report_uri_posts_violation_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker CSP report server");
    let addr = listener.local_addr().expect("worker CSP report addr");
    let base_url = format!("http://{addr}");
    let (request_tx, request_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker CSP report request");
        let request = read_http_request_with_body(&mut stream)
            .await
            .expect("read worker CSP report request");
        let _ = request_tx.send(request);
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write worker CSP report response");
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker CSP report loader");
    let script_url = format!("{base_url}/worker/main.js");
    let blocked_url = format!("{base_url}/worker/blocked.txt");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                try {
                    await fetch("./blocked.txt");
                    postMessage({ status: "unexpected" });
                } catch (error) {
                    postMessage({ name: error && error.name });
                }
                close();
            })();
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader.clone())
        .with_content_security_policies(vec![
            "connect-src 'none'; report-uri /csp-report".to_owned(),
        ]),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#"{"name":"TypeError"}"#);
    let request = timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for worker CSP report")
        .expect("worker CSP report capture channel closed");
    server
        .await
        .expect("worker CSP report server should finish");
    assert!(request.starts_with("POST /csp-report HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/csp-report")
    );
    let body = request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("worker CSP report request should contain body");
    let body: serde_json::Value =
        serde_json::from_str(body).expect("worker CSP report body should be JSON");
    assert_eq!(body["csp-report"]["document-uri"], script_url);
    assert_eq!(body["csp-report"]["blocked-uri"], blocked_url);
    assert_eq!(body["csp-report"]["effective-directive"], "connect-src");
    assert_eq!(body["csp-report"]["violated-directive"], "connect-src");
    assert_eq!(body["csp-report"]["disposition"], "enforce");
}

#[tokio::test]
async fn worker_fetch_csp_report_to_posts_reporting_api_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker CSP report-to server");
    let addr = listener.local_addr().expect("worker CSP report-to addr");
    let base_url = format!("http://{addr}");
    let (request_tx, request_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker CSP report-to request");
        let request = read_http_request_with_body(&mut stream)
            .await
            .expect("read worker CSP report-to request");
        let _ = request_tx.send(request);
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write worker CSP report-to response");
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker CSP report-to loader");
    let script_url = format!("{base_url}/worker/main.js");
    let blocked_url = format!("{base_url}/worker/blocked.txt");
    let reporting_endpoints =
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &[(
                "Reporting-Endpoints".to_owned(),
                "csp=\"/report-to\"".to_owned(),
            )],
            &url::Url::parse(&script_url).expect("script url"),
        );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                try {
                    await fetch("./blocked.txt");
                    postMessage({ status: "unexpected" });
                } catch (error) {
                    postMessage({ name: error && error.name });
                }
                close();
            })();
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader.clone())
        .with_content_security_policies(vec![
            "connect-src 'none'; report-uri /legacy; report-to csp".to_owned(),
        ])
        .with_content_security_reporting_endpoints(reporting_endpoints),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#"{"name":"TypeError"}"#);
    let request = timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for worker CSP report-to")
        .expect("worker CSP report-to capture channel closed");
    server
        .await
        .expect("worker CSP report-to server should finish");
    assert!(request.starts_with("POST /report-to HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/reports+json")
    );
    let body = request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("worker CSP report-to request should contain body");
    let body: serde_json::Value =
        serde_json::from_str(body).expect("worker CSP report-to body should be JSON");
    assert_eq!(body[0]["type"], "csp-violation");
    assert_eq!(body[0]["url"], script_url);
    assert_eq!(body[0]["body"]["documentURL"], script_url);
    assert_eq!(body[0]["body"]["blockedURL"], blocked_url);
    assert_eq!(body[0]["body"]["effectiveDirective"], "connect-src");
    assert_eq!(
        body[0]["body"]["originalPolicy"],
        "connect-src 'none'; report-uri /legacy; report-to csp"
    );
    assert_eq!(body[0]["body"]["disposition"], "enforce");
}

#[tokio::test]
async fn worker_xhr_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "report-only xhr ok".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr report-only loader");
    let script_url = format!("{base_url}/worker/main.js");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                });
            });
            const xhr = new XMLHttpRequest();
            xhr.onload = () => {
                postMessage({ events, status: xhr.status, text: xhr.responseText });
                close();
            };
            xhr.onerror = () => {
                postMessage({ events, error: "xhr-error" });
                close();
            };
            xhr.open("GET", "./data.txt");
            xhr.send();
            "#
            .into(),
            script_url.clone(),
        )
        .with_request_client(loader)
        .with_content_security_report_only_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"{base_url}/worker/data.txt","documentURI":"{script_url}","originalPolicy":"connect-src 'none'","disposition":"report","instance":true}}],"status":200,"text":"report-only xhr ok"}}"#
        )
    );
    server
        .await
        .expect("worker xhr report-only server should finish");
}

#[tokio::test]
async fn shared_worker_fetch_csp_block_dispatches_securitypolicyviolation_event() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            onconnect = () => {
                addEventListener("securitypolicyviolation", event => {
                    if (event.type === "securitypolicyviolation" &&
                        event.effectiveDirective === "connect-src" &&
                        event.blockedURI === "https://api.example/data" &&
                        event instanceof SecurityPolicyViolationEvent) {
                        close();
                    }
                });
                fetch("https://api.example/data").catch(() => {});
            };
            "#
            .into(),
            "https://app.example/shared-worker.js".into(),
        )
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        })
        .with_content_security_policies(vec!["connect-src 'none'".to_owned()]),
    );

    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(0))
        .expect("connect shared worker");
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        if matches!(msg, WorkerToParentMessage::SharedWorkerClosed) {
            break;
        }
    }
}

#[tokio::test]
async fn nested_worker_unhandled_error_routes_through_parent_worker_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesBoom: String(message).includes("child-boom"),
                filenameIncludesDataUrl: String(filename).includes("data:text/javascript"),
                errorIsUndefined: error === undefined
            });
            close();
            return true;
        };
        new Worker("data:text/javascript,throw%20new%20Error('child-boom')");
        "#
        .into(),
        "test://nested_worker_error_parent_onerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"messageIncludesBoom":true,"filenameIncludesDataUrl":true,"errorIsUndefined":true}"#
    );
}

#[tokio::test]
async fn nested_worker_script_load_failure_is_async_error_event() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let result = "not-run";
        try {
            const child = new Worker("missing-child.js");
            child.onerror = event => {
                event.preventDefault();
                postMessage({
                    constructed: result === "constructed",
                    type: event.type,
                    messageIsNonEmpty: event.message.length > 0,
                    filename: event.filename
                });
                close();
            };
            result = "constructed";
        } catch (error) {
            postMessage({ threw: error.name });
            close();
        }
        "#
        .into(),
        "http://example.test/parent.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"constructed":true,"type":"error","messageIsNonEmpty":true,"filename":"http://example.test/missing-child.js"}"#
    );
}

#[tokio::test]
async fn worker_onmessage_exception_routes_through_worker_global_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesBoom: String(message).includes("boom-message"),
                filename,
                lineno,
                colno,
                errorMessage: error && error.message
            });
            close();
            return true;
        };
        onmessage = function() {
            throw new Error("boom-message");
        };
        "#
        .into(),
        "test://message_onerror".into(),
    );

    handle.post_message(serialize_test_string("go"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(msg);
    assert!(
        json.contains(r#""messageIncludesBoom":true"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""errorMessage":"boom-message""#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""filename":"test://message_onerror""#),
        "json: {json}"
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn worker_error_listener_receives_errorevent_instance() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        addEventListener("error", function(event) {
            postMessage({
                isErrorEvent: event instanceof ErrorEvent,
                typeString: Object.prototype.toString.call(event),
                isTrusted: event.isTrusted,
                message: event.message,
                filename: event.filename
            });
            event.preventDefault();
            close();
        });
        onmessage = function() {
            throw new Error("boom-message");
        };
        "#
        .into(),
        "test://error_event_instance".into(),
    );

    handle.post_message(serialize_test_string("go"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"isErrorEvent":true,"typeString":"[object ErrorEvent]","isTrusted":true,"message":"Uncaught Error: boom-message","filename":"test://error_event_instance"}"#
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn worker_domexception_error_event_message_preserves_name_and_message() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesName: String(message).includes("TypeError"),
                messageIncludesMessage: String(message).includes("dom-boom"),
                errorName: error && error.name,
                errorMessage: error && error.message
            });
            close();
            return true;
        };
        onmessage = function() {
            throw new DOMException("dom-boom", "TypeError");
        };
        "#
        .into(),
        "test://domexception_onerror".into(),
    );

    handle.post_message(serialize_test_string("go"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"messageIncludesName":true,"messageIncludesMessage":true,"errorName":"TypeError","errorMessage":"dom-boom"}"#
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn worker_error_report_ignores_throwing_accessors() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        throw {
            get name() { throw new Error("name getter should stay local"); },
            get message() { throw new Error("message getter should stay local"); },
            get stack() { throw new Error("stack getter should stay local"); },
        };
        "#
        .into(),
        "test://throwing_error_accessors".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error {
            message, filename, ..
        } => {
            assert!(
                message.contains("Uncaught"),
                "message should come from the original report: {message}"
            );
            assert_eq!(filename, "test://throwing_error_accessors");
        }
        WorkerToParentMessage::Post(_) => panic!("expected worker error"),
        WorkerToParentMessage::SubresourceNetwork(_)
        | WorkerToParentMessage::PendingSubresourceFetch(_)
        | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
        | WorkerToParentMessage::SubresourceContinue(_)
        | WorkerToParentMessage::WebSocketSubresource(_)
        | WorkerToParentMessage::WebSocketLifecycle(_)
        | WorkerToParentMessage::WebSocketFrame(_)
        | WorkerToParentMessage::Console(_)
        | WorkerToParentMessage::RuntimeInspectorMessages(_)
        | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
        | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
        | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
        | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
        | WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(_)
        | WorkerToParentMessage::SharedWorkerClosed => panic!("expected worker error"),
    }
}

#[tokio::test]
async fn worker_timer_exception_routes_through_worker_global_onerror_and_interval_continues() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let tick = 0;
        let intervalId = setInterval(function() {
            tick += 1;
            if (tick === 1) {
                throw new Error("tick-boom");
            }
            postMessage(`tick:${tick}`);
            clearInterval(intervalId);
            close();
        }, 10);

        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                phase: "onerror",
                messageIncludesBoom: String(message).includes("tick-boom"),
                filename,
                lineno,
                colno,
                errorMessage: error && error.message
            });
            return true;
        };
        "#
        .into(),
        "test://timer_onerror".into(),
    );

    let first = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(first);
    assert!(json.contains(r#""phase":"onerror""#), "json: {json}");
    assert!(
        json.contains(r#""messageIncludesBoom":true"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""errorMessage":"tick-boom""#),
        "json: {json}"
    );

    let second = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(second), r#""tick:2""#);

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected interval worker to close cleanly");
}

#[tokio::test]
async fn worker_queue_microtask_runs_after_current_stack() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const order = [];
        queueMicrotask(() => {
            order.push("microtask");
            postMessage(order.join(","));
        });
        order.push("sync");
        "#
        .into(),
        "test://queue_microtask".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""sync,microtask""#);
}

#[tokio::test]
async fn worker_queue_microtask_uses_typed_callback_fifo_and_error_reporting() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const events = [];
        const conversionErrors = [];
        for (const invoke of [
            () => queueMicrotask(),
            () => queueMicrotask(null),
            () => queueMicrotask({})
        ]) {
            try {
                invoke();
                conversionErrors.push("missing");
            } catch (error) {
                conversionErrors.push(error.name);
            }
        }

        onerror = (_message, _source, _line, _column, error) => {
            events.push(`error:${error && error.name}`);
            return true;
        };
        const callback = new Proxy(
            function() {
                "use strict";
                events.push(`callback:${this === undefined}:${arguments.length}`);
                queueMicrotask(() => events.push("nested"));
            },
            {
                apply(target, receiver, args) {
                    events.push(`apply:${receiver === undefined}:${args.length}`);
                    return Reflect.apply(target, receiver, args);
                }
            }
        );
        queueMicrotask(callback);
        Promise.resolve().then(() => events.push("promise"));
        queueMicrotask(() => events.push("second"));

        const revoked = Proxy.revocable(function() {}, {});
        revoked.revoke();
        let revokedAccepted = true;
        try {
            queueMicrotask(revoked.proxy);
        } catch {
            revokedAccepted = false;
        }
        queueMicrotask(() => {
            queueMicrotask(() => {
                postMessage({ conversionErrors, revokedAccepted, events });
                close();
            });
        });
        events.push("sync");
        "#
        .into(),
        "test://queue_microtask_webidl".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"conversionErrors":["TypeError","TypeError","TypeError"],"revokedAccepted":true,"events":["sync","apply:true:0","callback:true:0","promise","second","error:TypeError","nested"]}"#
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(
        next.is_none(),
        "worker should close after the exact microtask run"
    );
}

#[tokio::test]
async fn worker_top_level_exception_routes_through_worker_global_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, filename, lineno, colno, error) {
            postMessage({
                messageIncludesBoom: String(message).includes("top-boom"),
                filename,
                lineno,
                colno,
                errorMessage: error && error.message
            });
            close();
            return true;
        };
        throw new Error("top-boom");
        "#
        .into(),
        "test://top_level_onerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(msg);
    assert!(
        json.contains(r#""messageIncludesBoom":true"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""errorMessage":"top-boom""#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""filename":"test://top_level_onerror""#),
        "json: {json}"
    );

    let next = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(next.is_none(), "expected handled worker to close cleanly");
}

#[tokio::test]
async fn unhandled_classic_worker_top_level_exception_is_runtime_phase() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        throw new Error("top-level-runtime");
        "#
        .into(),
        "test://top_level_runtime_phase".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error { message, phase, .. } => {
            assert!(
                message.contains("top-level-runtime"),
                "expected top-level-runtime, got {message:?}"
            );
            assert_eq!(phase, WorkerErrorPhase::Runtime);
        }
        other => panic!("expected worker error, got {other:?}"),
    }
}

// ─── Multiple concurrent workers ────────────────────────────────────

#[tokio::test]
async fn multiple_concurrent_workers() {
    ensure_v8();
    let mut handles: Vec<_> = (0..3)
        .map(|i| {
            spawn_worker(
                format!(r#"postMessage("worker_{i}");"#),
                format!("test://concurrent_{i}"),
            )
        })
        .collect();

    let mut received = Vec::new();
    for h in handles.iter_mut() {
        let msg = timeout(TIMEOUT, h.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        received.push(expect_post_json(msg));
    }

    received.sort();
    assert_eq!(
        received,
        vec![r#""worker_0""#, r#""worker_1""#, r#""worker_2""#,]
    );
}

// ─── Console ────────────────────────────────────────────────────────

#[tokio::test]
async fn worker_console_does_not_crash() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        console.log("hello");
        console.warn("warning");
        console.error("error");
        console.info("info");
        console.debug("debug");
        console.trace("trace");
        console.time("t");
        console.timeEnd("t");
        postMessage("done");
        "#
        .into(),
        "test://console".into(),
    );

    let mut console_messages = Vec::new();
    loop {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match msg {
            WorkerToParentMessage::Console(message) => console_messages.push(message.message),
            WorkerToParentMessage::Post(_) => break,
            other => panic!("unexpected worker console probe message: {other:?}"),
        }
    }
    assert_eq!(
        console_messages,
        [
            "log: hello",
            "warn: warning",
            "error: error",
            "info: info",
            "debug: debug",
            "trace: trace"
        ]
    );
}

// ─── Self reference ─────────────────────────────────────────────────

#[tokio::test]
async fn worker_self_reference() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        self.postMessage(self === globalThis);
        "#
        .into(),
        "test://self_ref".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "true");
}

// ─── Empty worker stays alive until parent drops handle ────────────

#[tokio::test]
async fn worker_empty_script_exits_on_drop() {
    ensure_v8();
    let handle = spawn_worker(
        r#"
        // intentionally empty — no onmessage, no timers
        "#
        .into(),
        "test://empty".into(),
    );

    // Worker stays alive (per spec) until the parent drops the handle.
    // Drop sends Terminate, then thread exits.
    drop(handle);
    // If we reach here without hanging, the test passes.
}

// ─── Syntax error ───────────────────────────────────────────────────

#[tokio::test]
async fn worker_syntax_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        function( { broken syntax
        "#
        .into(),
        "test://syntax_error".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(matches!(msg, WorkerToParentMessage::Error { .. }));
}

// ─── Drop terminates ────────────────────────────────────────────────

#[tokio::test]
async fn worker_drop_terminates() {
    ensure_v8();
    let handle = spawn_worker(
        r#"
        onmessage = function(e) {
            postMessage("alive");
        };
        "#
        .into(),
        "test://drop_terminate".into(),
    );

    drop(handle);
    // If we reach here without hanging, the test passes.
}
