use super::*;

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
async fn worker_global_prototype_chains_inherit_event_target_and_are_immutable() {
    ensure_v8();
    let script_url = url::Url::parse("https://example.test/app/worker-prototypes.js").unwrap();
    let fixture = include_str!("../../../../../tests/fixtures/worker-global-prototypes.js");
    for (interface, kind) in [
        (
            "DedicatedWorkerGlobalScope",
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
        ),
        (
            "SharedWorkerGlobalScope",
            WorkerGlobalKind::Shared {
                name: String::new(),
                storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(
                    &script_url,
                    None,
                ),
            },
        ),
        (
            "ServiceWorkerGlobalScope",
            WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url::Url::parse("https://example.test/app/").unwrap(),
            },
        ),
    ] {
        let (bootstrap_tx, mut bootstrap_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
        let handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                format!(
                    r#"
                    const result = ({fixture})({interface:?});
                    if (result.checks !== 46 || result.failures.length) {{
                        throw new Error(JSON.stringify(result));
                    }}
                    "#
                ),
                script_url.to_string(),
            )
            .with_global_kind(kind)
            .with_bootstrap_completion_sender(bootstrap_tx),
        );
        let bootstrap = timeout(TIMEOUT, bootstrap_rx.recv()).await;
        handle.terminate_and_join();
        bootstrap
            .expect("timed out waiting for worker prototype checks")
            .expect("worker bootstrap channel closed")
            .result
            .unwrap_or_else(|error| panic!("{interface} prototype checks failed: {error:?}"));
    }
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
        completion.owner.version_id(),
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
    let run = crate::runtime::RendererServiceWorkerRunIdentity::fresh();
    let request_url = url::Url::parse("https://example.test/app/navigation.html")
        .expect("navigation preload request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 901;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
            request_url,
            request_method: "GET".to_owned(),
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
                status_text: "Accepted".to_owned(),
                headers: vec![("x-preload".to_owned(), b"yes".to_vec())],
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
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
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
    let run = crate::runtime::RendererServiceWorkerRunIdentity::fresh();
    let request_url = url::Url::parse("https://example.test/app/preload-redirect.html")
        .expect("navigation preload redirect request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 904;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
            request_url: request_url.clone(),
            request_method: "GET".to_owned(),
            request_mode: moli_fetch::RequestMode::Navigate,
            body_source_id,
            response_head: MaterializedServiceWorkerFetchResponseHead {
                final_url: Some(request_url),
                response_type: "default".to_owned(),
                redirected: false,
                status: 302,
                status_text: "Found".to_owned(),
                headers: vec![("location".to_owned(), b"/app/final.html".to_vec())],
            },
        },
    );
    handle.finish_service_worker_navigation_preload_stream(
        ServiceWorkerNavigationPreloadStreamFinished {
            event_id,
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
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
    let run = crate::runtime::RendererServiceWorkerRunIdentity::fresh();
    let mut request = service_worker_fetch_request_for_test();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
        request,
        navigation_preload_sent: true,
    });
    handle.fail_service_worker_navigation_preload(ServiceWorkerNavigationPreloadFailure {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
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
    let run = crate::runtime::RendererServiceWorkerRunIdentity::fresh();
    let request_url = url::Url::parse("https://example.test/app/navigation.html")
        .expect("navigation preload request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 902;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
            request_url,
            request_method: "GET".to_owned(),
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
                status_text: "Accepted".to_owned(),
                headers: vec![("x-preload".to_owned(), b"yes".to_vec())],
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
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
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
    let run = crate::runtime::RendererServiceWorkerRunIdentity::fresh();
    let request_url = url::Url::parse("https://example.test/app/navigation.html")
        .expect("navigation preload request URL");
    let mut request = service_worker_fetch_request_for_test();
    request.url = request_url.clone();
    request.destination = ServiceWorkerRequestDestination::Document;
    request.request_mode = moli_fetch::RequestMode::Navigate;
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            run.clone(),
        ),
        request,
        navigation_preload_sent: true,
    });
    let body_source_id = 903;
    handle.start_service_worker_navigation_preload_response(
        ServiceWorkerNavigationPreloadResponseStarted {
            event_id,
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
            request_url,
            request_method: "GET".to_owned(),
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
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
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
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                run.clone(),
            ),
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
        completion.owner.version_id(),
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
