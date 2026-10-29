use super::*;

#[tokio::test]
async fn worker_close_preserves_decoder_backing_for_queued_microtasks() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        queueMicrotask(() => {
            const decoder = new TextDecoder();
            decoder.decode(new Uint8Array([0xE2, 0x82]), { stream: true });
            postMessage(decoder.decode(new Uint8Array([0xAC])));
        });
        close();
        "#
        .into(),
        "https://text-codec-runtime.test/after-close.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("post-close text codec probe timed out")
        .expect("worker closed before draining its codec microtask");
    assert_eq!(expect_post_json(message), r#""€""#);
    assert!(timeout(TIMEOUT, handle.recv()).await.unwrap().is_none());
}

#[tokio::test]
async fn worker_text_codec_runtime_preserves_streaming_bom_and_fatal_errors() {
    ensure_v8();
    let source = format!(
        "({}).then(result => {{ postMessage(result); close(); }}, error => {{ postMessage({{ error: String(error) }}); close(); }});",
        include_str!("../../../../tests/fixtures/text-codec-runtime.js"),
    );
    let mut handle = spawn_worker(source, "https://text-codec-runtime.test/worker.js".into());
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker text codec probe timed out")
        .expect("worker closed before reporting text codec results");
    assert_eq!(
        expect_post_json(message),
        r#"{"split":"€","utf8":"hé","legacy":"太平洋","bomKept":true,"fatal":["TypeError","TypeError"]}"#,
    );
}
