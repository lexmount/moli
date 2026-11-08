use super::*;

#[tokio::test]
async fn worker_interfaces_inherit_canonical_event_target_operations() {
    ensure_v8();
    let probe = include_str!("../../../../tests/fixtures/event-target-inheritance.js");
    let mut handle = spawn_worker(
        format!(
            "{probe}; postMessage({{pending: __eventTargetInheritance.pending, rows: __eventTargetInheritance.rows}}); close();"
        ),
        "https://event-target-inheritance.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["pending"], false);
    assert!(
        result["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row[1] == true),
        "{result}"
    );
}

#[tokio::test]
async fn worker_event_target_prototype_methods_preserve_native_state() {
    ensure_v8();
    let probe = include_str!("../../../../tests/fixtures/event-target-prototype.js");
    let mut handle = spawn_worker(
        format!("{probe}\npostMessage(__uiEventResults); close();"),
        "https://event-target-prototype.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["complete"], true, "{result}");
    assert_eq!(result["total"], 56, "{result}");
    assert_eq!(result["passed"], result["total"], "{result}");
}

#[tokio::test]
async fn native_promise_rejection_events_finish_shared_dispatch_state() {
    ensure_v8();
    let mut handle = spawn_worker(r#"
        let retained;
        let calls = 0;
        let identity = false;
        addEventListener('unhandledrejection', function(event) {
            retained = event;
            identity = this === self && event.target === self && event.currentTarget === self &&
                event.composedPath()[0] === self && event.isTrusted;
            calls++;
            event.preventDefault();
            setTimeout(() => {
                const finished = retained.currentTarget === null && retained.composedPath().length === 0;
                self.dispatchEvent(retained);
                postMessage([identity, finished, calls, retained.isTrusted]);
                close();
            }, 0);
        }, {once: true});
        addEventListener('unhandledrejection', () => calls += 10, {once: true});
        Promise.reject('dispatch-state-probe');
    "#.into(), "https://worker.test/native-rejection.js".into());
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), "[true,true,11,false]");
}

#[tokio::test]
async fn service_worker_install_listeners_survive_worker_navigator_materialization() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        void navigator;
        addEventListener('install', event => {
            event.waitUntil(Promise.reject('install listener reached'));
        });
        "#,
    );
    let completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        1,
    )
    .await;
    handle.terminate_and_join();
    assert_eq!(
        completion.result,
        Err("service worker waitUntil promise rejected".to_owned())
    );
}

#[tokio::test]
async fn service_worker_message_listeners_survive_worker_navigator_materialization() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        const retainedNavigator = navigator;
        addEventListener('message', event => {
            const identity = retainedNavigator === navigator && event.target === self;
            if (event.data === 'payload' && identity) {
                event.waitUntil(Promise.reject('message listener reached'));
            }
        });
        "#,
    );
    let completion = dispatch_service_worker_message_event_for_test(
        &mut handle,
        1,
        serialize_test_post_message_value("'payload'"),
    )
    .await;
    handle.terminate_and_join();
    assert_eq!(
        completion.result,
        Err("service worker waitUntil promise rejected".to_owned())
    );
}

#[tokio::test]
async fn service_worker_dispatch_survives_navigator_materialization_in_an_earlier_listener() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        addEventListener('install', () => { void navigator; });
        addEventListener('install', event => {
            event.waitUntil(Promise.reject('later install listener reached'));
        });
        "#,
    );
    let completion = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        1,
    )
    .await;
    handle.terminate_and_join();
    assert_eq!(
        completion.result,
        Err("service worker waitUntil promise rejected".to_owned())
    );
}
