use super::*;

async fn assert_promise_job_fixture(script: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://promise-job-retirement.test/index.html",
        &loader,
    );
    vm.exec(
        &format!(
            r#"globalThis.__promiseJobRetirement = null;
({script})().then(
    result => {{ __promiseJobRetirement = result; }},
    error => {{ __promiseJobRetirement = {{error: String(error)}}; }}
);"#
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__promiseJobRetirement !== null)",
        "true",
        "Promise job retirement fixture should finish",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__promiseJobRetirement)").unwrap()).unwrap();
    assert_eq!(result["completed"], true, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[tokio::test(flavor = "current_thread")]
async fn promise_jobs_continue_after_child_window_retirement() {
    assert_promise_job_fixture(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/promise-job-retirement.js"
    )))
    .await;
}
