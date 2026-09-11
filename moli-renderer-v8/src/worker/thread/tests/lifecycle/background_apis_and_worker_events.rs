use super::*;

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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
                getterForged: probe(() => abortedGetter.call(forged))
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
        r#"{"hasVisibleSlot":false,"ownNames":[],"anyForged":"TypeError","getterForged":"TypeError"}"#
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
                    setTimeout(() => {
                        postMessage({ calls, callableHandleEventLookups });
                        close();
                    }, 0);
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
                    // order. Use the next task: callback-cleanup microtasks
                    // run before later listeners in the same event dispatch.
                    setTimeout(() => {
                        primary.abort();
                        replacement.abort();
                        port2.postMessage("third");
                    }, 0);
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

#[tokio::test]
async fn worker_xmlhttprequest_abort_uses_internal_state_and_preserves_reopened_request() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const probe = new XMLHttpRequest();
            const state = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, "readyState").get;
            const states = [];
            let aborts = 0;
            probe.onreadystatechange = () => states.push(state.call(probe));
            probe.onabort = () => ++aborts;
            Object.defineProperty(probe, "readyState", { get() { throw new Error("public readyState read"); } });
            probe.abort();
            const unsent = state.call(probe);
            probe.open("POST", "data:text/plain,complete", false);
            probe.abort();
            const opened = state.call(probe);
            probe.send({ toString() { probe.abort(); return "payload"; } });
            const completed = [state.call(probe), probe.status, probe.responseText];
            probe.abort();
            const reset = [state.call(probe), probe.status, probe.statusText,
                probe.responseText, probe.responseURL, probe.getAllResponseHeaders()];

            const xhr = new XMLHttpRequest();
            const events = [];
            let restarted = false;
            xhr.onreadystatechange = () => {
                if (xhr.readyState === 4 && !restarted) {
                    restarted = true;
                    events.push(`abort-ready:${xhr.readyState}:${xhr.status}`);
                    xhr.open("GET", "data:text/plain,second");
                    xhr.send();
                    events.push(`reopened:${xhr.readyState}`);
                }
            };
            xhr.onloadstart = () => {
                if (!restarted) {
                    xhr.abort();
                    events.push(`after-abort:${xhr.readyState}`);
                }
            };
            xhr.onabort = () => events.push(`abort:${xhr.readyState}`);
            xhr.onload = () => events.push(`load:${xhr.responseText}`);
            xhr.onloadend = () => {
                events.push(`loadend:${xhr.readyState}`);
                if (xhr.readyState === 4) {
                    postMessage({ unsent, opened, completed, reset, states, aborts, events });
                    close();
                }
            };
            xhr.open("GET", "data:text/plain,first");
            xhr.send();
        })();
        "#
        .into(),
        "https://worker-xhr-abort-state.test/main.js".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"unsent":0,"opened":1,"completed":[4,200,"complete"],"reset":[0,0,"","","",""],"states":[1,4],"aborts":0,"events":["abort-ready:4:0","reopened:1","abort:1","loadend:1","after-abort:1","load:second","loadend:4"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_loadstart_handles_upload_abort_and_reopen() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const aborted = ["xhr", "upload", "empty"].map(abortAt => {
                const xhr = new XMLHttpRequest();
                const events = [];
                xhr.onloadstart = event => {
                    events.push(`xhr-start:${event.loaded}:${event.total}:${event.lengthComputable}`);
                    if (abortAt === "xhr") xhr.abort();
                };
                xhr.upload.onloadstart = event => {
                    events.push(`upload-start:${event.loaded}:${event.total}:${event.lengthComputable}`);
                    if (abortAt !== "xhr") xhr.abort();
                };
                for (const type of ["progress", "load", "abort", "loadend"]) {
                    xhr.upload.addEventListener(type, event => events.push(
                        `upload-${type}:${xhr.readyState}:${event.loaded}:${event.total}:${event.lengthComputable}`));
                }
                xhr.onabort = () => events.push(`xhr-abort:${xhr.readyState}`);
                xhr.onloadend = () => events.push(`xhr-loadend:${xhr.readyState}`);
                xhr.open("POST", "/unused");
                xhr.send(abortAt === "empty" ? "" : "é");
                events.push(`after-send:${xhr.readyState}:${xhr.status}`);
                return events;
            });

            const xhr = new XMLHttpRequest();
            const reopened = [];
            xhr.onloadstart = () => reopened.push(`xhr-start:${xhr.readyState}`);
            xhr.upload.onloadstart = event => {
                reopened.push(`upload-start:${event.loaded}:${event.total}`);
                xhr.open("GET", "data:text/plain,replacement");
                xhr.send();
                reopened.push(`after-reopen:${xhr.readyState}`);
            };
            for (const type of ["progress", "load", "abort", "loadend"]) {
                xhr.upload.addEventListener(type, () => reopened.push(`unexpected-upload-${type}`));
            }
            xhr.onload = () => reopened.push(`load:${xhr.responseText}`);
            xhr.onloadend = () => {
                reopened.push(`loadend:${xhr.readyState}`);
                postMessage({ aborted, reopened });
                close();
            };
            xhr.open("POST", "/unused");
            xhr.send("é");
        })();
        "#
        .into(),
        "https://worker-xhr-upload-start.test/main.js".into(),
    );
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"aborted":[["xhr-start:0:0:false","upload-abort:4:0:0:false","upload-loadend:4:0:0:false","xhr-abort:4","xhr-loadend:4","after-send:0:0"],["xhr-start:0:0:false","upload-start:0:2:true","upload-abort:4:0:0:false","upload-loadend:4:0:0:false","xhr-abort:4","xhr-loadend:4","after-send:0:0"],["xhr-start:0:0:false","upload-start:0:0:false","upload-abort:4:0:0:false","upload-loadend:4:0:0:false","xhr-abort:4","xhr-loadend:4","after-send:0:0"]],"reopened":["xhr-start:1","upload-start:0:2","xhr-start:1","after-reopen:1","load:replacement","loadend:4"]}"#
    );
}
