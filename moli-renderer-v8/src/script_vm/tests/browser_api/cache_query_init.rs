use super::*;

#[test]
fn cache_query_converts_inherited_options_before_native_request_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://cache-dictionary.test/app/");
    vm.eval(&format!(
        "globalThis.cacheDictionaryResult='pending'; ({}).then(value=>cacheDictionaryResult=value,error=>cacheDictionaryResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/cache-query-dictionary.js"),
    ))
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(cacheDictionaryResult)")
            .unwrap(),
        "true"
    );
}
