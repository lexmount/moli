use super::*;

#[tokio::test]
async fn worker_geometry_matrix_constructors_convert_before_rejecting_strings() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const result = probe(globalThis, true);
            postMessage({{total: result.total, failures: result.failures}});
            close();"#,
        include_str!("../../../script_vm/tests/geometry_matrix_constructor_union.js"),
    );
    let mut worker = spawn_worker(source, "test://geometry_matrix_constructor_union".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":64,"failures":[]}"#);
}
