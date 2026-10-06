use super::*;

#[tokio::test(flavor = "current_thread")]
async fn storage_result_arrays_ignore_inherited_numeric_properties() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-array-results.test/",
        &loader,
    );
    vm.exec(include_str!("cache_array_results.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete===true)",
        "true",
        "storage-result-array-prototype-fixture",
    )
    .await;
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "488");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
