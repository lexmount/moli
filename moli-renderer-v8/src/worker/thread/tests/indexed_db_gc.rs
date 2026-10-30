use super::*;

async fn cursor_gc_command(
    handle: &WorkerHandle,
    id: i64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let call_id = i32::try_from(id).expect("test call id");
    let (dispatch_tx, dispatch_rx) = oneshot::channel();
    let (response_tx, response_rx) = oneshot::channel();
    assert!(handle.dispatch_runtime_protocol_message(
        Some("SID-idb-gc".to_owned()),
        serde_json::json!({"id": id, "method": method, "params": params}).to_string(),
        Some(crate::runtime::RendererRuntimeInspectorResponseSender::new(
            call_id,
            response_tx
        )),
        dispatch_tx,
    ));
    timeout(TIMEOUT, dispatch_rx)
        .await
        .expect("worker GC dispatch timeout")
        .expect("worker GC dispatch closed")
        .expect("worker GC dispatch failed");
    // HeapProfiler.collectGarbage can answer after the Inspector dispatch
    // returns. Use the production deferred-response channel for both modes.
    let completion = timeout(TIMEOUT, response_rx)
        .await
        .expect("worker GC response timeout")
        .expect("worker GC response closed");
    let response = completion
        .output
        .into_protocol_response(call_id)
        .expect("worker GC command response")
        .into_json()
        .expect("GC probes return scalar JSON responses");
    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    response["result"].clone()
}

#[tokio::test]
async fn indexed_db_worker_cursor_graphs_collect_and_retained_cursors_survive_gc() {
    ensure_v8();
    let probe = include_str!("../../../script_vm/tests/indexed_db_cursor_gc.js");
    let manager = crate::new_indexed_db_manager(None).expect("worker IndexedDB manager");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!("{probe}\nprepareIndexedDbCursorGc().then(value => postMessage(value), error => postMessage(String(error)));"),
            "https://indexeddb-cursor-gc.test/worker.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker IndexedDB GC setup timeout")
        .expect("worker IndexedDB GC setup closed");
    assert_eq!(expect_post_json(message), "\"prepared\"");
    cursor_gc_command(
        &handle,
        1,
        "HeapProfiler.collectGarbage",
        serde_json::json!({}),
    )
    .await;
    let checked = cursor_gc_command(&handle, 2, "Runtime.evaluate", serde_json::json!({
        "expression": "checkIndexedDbCursorGc(); delete globalThis.cursorGcHeld; true", "returnByValue": true
    })).await;
    assert_eq!(checked["result"]["value"], true, "{checked}");
    cursor_gc_command(
        &handle,
        3,
        "HeapProfiler.collectGarbage",
        serde_json::json!({}),
    )
    .await;
    let checked = cursor_gc_command(
        &handle,
        4,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": "checkReleasedIndexedDbCursorGc()", "returnByValue": true
        }),
    )
    .await;
    assert_eq!(checked["result"]["value"], true, "{checked}");
}
