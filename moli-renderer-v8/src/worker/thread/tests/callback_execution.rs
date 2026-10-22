use super::*;

#[tokio::test]
async fn worker_performance_observer_drop_reporting_preserves_observe_boundaries() {
    ensure_v8();
    let source = format!(
        "const probe={};probe(globalThis,globalThis,globalThis,'worker').then(checks=>{{postMessage({{total:checks.length,failures:checks.filter(row=>!row.passed)}});close();}},error=>{{postMessage({{error:String(error)}});close();}});",
        include_str!("../../../script_vm/tests/performance_observer_dropped_entries.js"),
    );
    let mut handle = spawn_worker(source, "https://observer-drops.test/worker.js".into());
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("PerformanceObserver drop reporting timed out")
        .expect("worker closed before reporting drop semantics");
    assert_eq!(expect_post_json(message), r#"{"total":44,"failures":[]}"#);
}

#[tokio::test]
async fn worker_performance_observer_delivery_follows_registration_order_after_reobserve() {
    ensure_v8();
    let source = format!(
        "({}).then(result => {{ postMessage(result); close(); }}, error => {{ postMessage({{ error: String(error) }}); close(); }});",
        include_str!("../../../../tests/fixtures/performance-observer-registration-order.js"),
    );
    let mut handle = spawn_worker(
        source,
        "test://performance-observer-registration-order".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("registered observer delivery timed out")
        .expect("worker closed before reporting registration order");
    assert_eq!(
        expect_post_json(message),
        r#"["second:initial","first:initial","first:reobserved","second:reobserved"]"#,
    );
}

#[tokio::test]
async fn worker_webidl_callback_execution_preserves_api_exception_policies() {
    ensure_v8();
    let source = format!(
        "({}).then(result => {{ postMessage(result); close(); }}, error => {{ postMessage({{ error: String(error) }}); close(); }});",
        include_str!("../../../../tests/fixtures/webidl-callback-execution.js"),
    );
    let mut handle = spawn_worker(source, "https://callback-execution.test/worker.js".into());
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("shared callback policy probe timed out")
        .expect("worker closed before reporting callback policies");
    assert_eq!(
        expect_post_json(message),
        r#"{"policyThrow":true,"policyConversionThrow":true,"streamThrow":true,"streamConversionThrow":true,"notifications":["microtask"]}"#,
    );
}

#[tokio::test]
async fn worker_close_preserves_queued_microtasks_and_synchronous_callback_apis() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        queueMicrotask(() => {
            const events = [];
            const policy = trustedTypes.createPolicy('after-close', {
                createHTML(input) { return input; }
            });
            events.push(String(policy.createHTML('policy-after-close')));
            new ReadableStream({ start(controller) {
                events.push('stream-after-close');
                controller.close();
            }});
            postMessage(events);
        });
        close();
        "#
        .into(),
        "https://callback-execution.test/worker-close.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("post-close microtask probe timed out")
        .expect("worker closed before draining queued microtasks");
    assert_eq!(
        expect_post_json(message),
        r#"["policy-after-close","stream-after-close"]"#
    );
    assert!(timeout(TIMEOUT, handle.recv()).await.unwrap().is_none());
}

#[tokio::test]
async fn worker_performance_observer_delivery_preserves_batches_and_reentrant_registration() {
    ensure_v8();
    let source = format!(
        "({}).then(result => {{ postMessage(result); close(); }}, error => {{ postMessage({{ error: String(error) }}); close(); }});",
        include_str!("../../../../tests/fixtures/performance-observer-delivery.js"),
    );
    let mut handle = spawn_worker(source, "test://performance-observer-delivery".into());
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("PerformanceObserver delivery timed out")
        .expect("worker closed before reporting delivery");
    assert_eq!(
        expect_post_json(message),
        r#"{"batches":[["first","second"],["after-reobserve"]],"taken":["taken-during-callback"],"proxyCalls":2}"#,
    );
}

#[tokio::test]
async fn worker_performance_observer_exception_routes_through_worker_error_and_can_reobserve() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const events = [];
        const failure = new Error('performance-observer-error');
        let calls = 0;
        const observer = new PerformanceObserver(list => {
            events.push('callback:' + list.getEntries().map(entry => entry.name).join(','));
            if (++calls === 1) throw failure;
            observer.disconnect();
            postMessage(events);
            close();
        });
        self.addEventListener('error', event => {
            events.push('error:' + (event.error === failure) + ':' + (event.currentTarget === self));
            event.preventDefault();
            observer.disconnect();
            observer.observe({ type: 'mark' });
            performance.mark('after-error');
        });
        observer.observe({ type: 'mark' });
        performance.mark('before-error');
        "#
        .into(),
        "test://performance-observer-error".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("PerformanceObserver error timed out")
        .expect("worker closed before reporting error recovery");
    assert_eq!(
        expect_post_json(message),
        r#"["callback:before-error","error:true:true","callback:after-error"]"#,
    );
    assert!(
        timeout(TIMEOUT, handle.recv()).await.unwrap().is_none(),
        "preventDefault must suppress the parent error report",
    );
}

#[tokio::test]
async fn worker_performance_observer_unhandled_exception_is_reported_to_parent() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        new PerformanceObserver(() => {
            throw new Error('unhandled-performance-observer-error');
        }).observe({ type: 'mark' });
        performance.mark('before-error');
        "#
        .into(),
        "test://performance-observer-unhandled-error".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("PerformanceObserver parent error timed out")
        .expect("worker closed before reporting unhandled error");
    let WorkerToParentMessage::Error { message, phase, .. } = message else {
        panic!("expected worker error report, received {message:?}");
    };
    assert!(message.contains("unhandled-performance-observer-error"));
    assert_eq!(phase, WorkerErrorPhase::Runtime);
}

#[tokio::test]
async fn worker_performance_observer_does_not_deliver_after_close() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        new PerformanceObserver(() => postMessage('unexpected-delivery'))
            .observe({ type: 'mark' });
        performance.mark('queued-before-close');
        close();
        postMessage('closed');
        "#
        .into(),
        "test://performance-observer-close".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker close timed out")
        .expect("worker closed before reporting completion");
    assert_eq!(expect_post_json(message), r#""closed""#);
    assert!(
        timeout(TIMEOUT, handle.recv()).await.unwrap().is_none(),
        "closed workers must discard pending observer deliveries",
    );
}
