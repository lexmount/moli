use super::*;

#[tokio::test]
async fn worker_json_data_module_parse_errors_keep_syntax_error_identity() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const url = 'data:application/json,%7Bbad';
        const capture = () => import(url, {with:{type:'json'}}).catch(error => error);
        Promise.all([capture(), capture()]).then(async errors => {
            const repeated = await capture();
            postMessage([errors[0] instanceof SyntaxError,
                errors[0] === errors[1], errors[0] === repeated]);
        });
    "#
        .into(),
        "https://json-errors.test/worker.js".into(),
    );
    assert_eq!(recv_post_json(&mut handle).await, "[true,true,true]");
    handle.terminate_and_join();
}
