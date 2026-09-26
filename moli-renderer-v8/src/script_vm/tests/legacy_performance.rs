use super::*;

#[tokio::test(flavor = "current_thread")]
async fn legacy_performance_bindings_preserve_native_state_through_shadowing_and_lifecycle() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://legacy-performance.test/");
    vm.eval("globalThis.__legacyPerformanceResult = null")
        .unwrap();
    let fixture = include_str!("../../../tests/fixtures/legacy-performance-webidl.js");
    vm.eval(&format!(
        "({}).then(result => __legacyPerformanceResult = result, error => __legacyPerformanceResult = String(error));",
        fixture.trim().trim_end_matches(';')
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__legacyPerformanceResult !== null)",
        "true",
        "legacy Performance WebIDL and lifecycle checks",
    )
    .await;
    let result = vm.eval("__legacyPerformanceResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["total"], 1159);
    assert_eq!(result["failures"], serde_json::json!([]));
}
