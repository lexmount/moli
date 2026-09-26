use super::*;

#[tokio::test(flavor = "current_thread")]
async fn performance_bindings_validate_receivers_before_author_code_in_windows_and_workers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://performance-receivers.test/");
    let window = vm
        .eval(include_str!(
            "../../../tests/fixtures/performance-receivers.js"
        ))
        .unwrap();
    assert_eq!(window, r#"{"total":436,"failures":[]}"#);

    vm.eval("globalThis.__performanceReceiverWorkerResult = null")
        .unwrap();
    let worker = include_str!("../../../tests/fixtures/performance-receivers-worker.js");
    vm.eval(&format!(
        "({}).then(result => __performanceReceiverWorkerResult = result, error => __performanceReceiverWorkerResult = String(error));",
        worker.trim().trim_end_matches(';')
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__performanceReceiverWorkerResult !== null)",
        "true",
        "Performance worker receiver checks",
    )
    .await;
    assert_eq!(
        vm.eval("__performanceReceiverWorkerResult").unwrap(),
        r#"{"total":155,"failures":[]}"#
    );
}
