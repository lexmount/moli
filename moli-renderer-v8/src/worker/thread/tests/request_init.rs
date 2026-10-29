use super::*;

#[tokio::test]
async fn worker_request_like_fetch_validates_inherited_native_signal_after_init() {
    ensure_v8();
    let source = format!(
        "({}).then(value=>postMessage(value),error=>postMessage(String(error.stack||error)));",
        include_str!("../../../../tests/fixtures/request-like-signal.js")
    );
    let mut handle = spawn_worker(source, "https://request-like-signal.test/worker.js".into());
    assert_eq!(recv_post_json(&mut handle).await, "true");
}

#[tokio::test]
async fn worker_request_and_fetch_use_the_shared_dictionary_conversion() {
    ensure_v8();
    let source = format!(
        "({}).then(value=>postMessage(value),error=>postMessage(String(error.stack||error)));",
        include_str!("../../../../tests/fixtures/request-init-dictionary.js")
    );
    let mut handle = spawn_worker(source, "https://request-init-worker.test/worker.js".into());
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker dictionary conversion must settle")
        .expect("worker must report its result");
    assert_eq!(expect_post_json(message), "true");
}
