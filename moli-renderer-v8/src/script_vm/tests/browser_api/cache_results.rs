use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cache_queries_preserve_creation_order_owner_realm_and_immutable_results() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://cache-results.test/", &loader);
    vm.exec(include_str!("cache_results.js"), None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete===true)",
        "true",
        "cache-query-results-fixture",
    )
    .await;
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "145");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
