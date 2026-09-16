use super::*;

#[tokio::test]
async fn worker_abort_signal_static_factories_preserve_conversion_and_timeout_semantics() {
    ensure_v8();
    let probe = include_str!("../../../script_vm/tests/browser_api/abort_signal_statics.js");
    let mut handle = spawn_worker(
        format!(
            r#"{probe}
            (async () => {{
                postMessage({{statics: abortSignalStaticsProbe(), timeout: await abortSignalTimeoutProbe()}});
                close();
            }})().catch(error => {{ postMessage({{error: error.name + ': ' + error.message}}); close(); }});"#
        ),
        "https://abort-signal-statics.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let row: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(row["statics"]["failures"], serde_json::json!([]), "{row}");
    assert_eq!(row["statics"]["scenarios"].as_array().unwrap().len(), 4);
    assert_eq!(row["timeout"]["failures"], serde_json::json!([]), "{row}");
    assert_eq!(row["timeout"]["calls"], 1);
}
