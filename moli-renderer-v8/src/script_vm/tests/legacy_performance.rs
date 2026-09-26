use super::*;

#[tokio::test(flavor = "current_thread")]
async fn legacy_performance_bindings_preserve_native_state_across_realms_and_shadowing() {
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
        "legacy Performance WebIDL and native-state checks",
    )
    .await;
    let result = vm.eval("__legacyPerformanceResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["total"], 1158);
    assert_eq!(result["failures"], serde_json::json!([]));
}

#[test]
fn frozen_legacy_performance_timing_receives_lifecycle_updates() {
    let mut vm = new_storage_test_vm("https://frozen-legacy-performance.test/");
    let fixture = include_str!("../../../tests/fixtures/legacy-performance-frozen-lifecycle.js");
    vm.eval(&format!(
        "globalThis.__checkFrozenLegacyTiming = {}",
        fixture.trim().trim_end_matches(';')
    ))
    .unwrap();
    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .unwrap();
    vm.dispatch_window_load_event().unwrap();
    assert_eq!(vm.eval("__checkFrozenLegacyTiming()").unwrap(), "true");
}
