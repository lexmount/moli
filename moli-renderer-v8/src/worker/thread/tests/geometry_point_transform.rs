use super::*;

#[tokio::test]
async fn worker_geometry_point_transforms_preserve_2d_and_3d_arithmetic() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const result = probe(globalThis);
            postMessage({{total: result.total, failures: result.failures}});
            close();"#,
        include_str!("../../../script_vm/tests/geometry_point_transform.js"),
    );
    let mut worker = spawn_worker(source, "test://geometry_point_transform".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":88,"failures":[]}"#);
}
