use super::*;

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
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            ),
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
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            ),
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
            owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
                ServiceWorkerVersionId::from_u64_for_test(1),
                crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            ),
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
            other => panic!("unexpected worker message: {other:?}"),
        }
    }
    handle.terminate_and_join();
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
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
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
    }
    handle.terminate_and_join();
}
