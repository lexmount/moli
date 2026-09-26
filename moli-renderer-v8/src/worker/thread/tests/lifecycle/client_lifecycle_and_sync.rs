use super::*;

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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
                        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                            ServiceWorkerVersionId::from_u64_for_test(1),
                            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
