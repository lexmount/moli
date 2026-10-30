use super::*;

async fn assert_worker_stream_clone_failures(poison_bridge: bool) {
    ensure_v8();
    let registry = crate::message_port_runtime::new_message_port_registry();
    let runtime = crate::runtime::RendererBrowserContextRuntime::new_with_registries_for_test(
        registry.clone(),
        crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
    );
    let source = format!(
        "{}; postMessage(probeStreamCloneFailures({poison_bridge}));",
        include_str!("stream_clone_failures.js"),
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source, "https://stream-clone-failure.test/worker.js".into())
            .with_worker_context_runtime(runtime.worker_context_runtime()),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("stream transfer failure probe timed out")
        .expect("worker closed before reporting transfer failures");
    let observed: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(
        observed,
        serde_json::json!({
            "results": vec![serde_json::json!(["DataCloneError", true]); 6],
            "sameMarker": true,
            "functionError": ["DataCloneError", true],
            "bridgeReads": 0,
        }),
    );
    // Assert rollback while the Worker is still alive, before teardown could
    // hide leaked private transfer channels.
    assert_eq!(registry.endpoint_count(), 0, "failed transfer leaked ports");
    handle.terminate_and_join();
}

#[tokio::test]
async fn worker_failed_stream_transfer_preserves_exception_and_discards_channels() {
    assert_worker_stream_clone_failures(false).await;
}

#[tokio::test]
async fn worker_failed_stream_transfer_does_not_read_author_window_bridge() {
    assert_worker_stream_clone_failures(true).await;
}
