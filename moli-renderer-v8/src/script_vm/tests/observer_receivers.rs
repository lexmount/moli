use super::*;

#[tokio::test(flavor = "current_thread")]
async fn observer_bindings_validate_native_receivers_before_author_code() {
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-receivers.test/");
    let result = vm
        .eval(include_str!(
            "../../../tests/fixtures/observer-receiver-state.js"
        ))
        .unwrap();
    assert_eq!(result, r#"{"total":393,"failures":[]}"#);
}

#[tokio::test(flavor = "current_thread")]
async fn observer_callbacks_and_entry_lists_preserve_native_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-native-state.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/observer-native-state-lifecycle.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 5);
    for index in 0..count {
        let name = vm
            .eval(&format!("__observerNativeState.start({index})"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__observerNativeState.result !== null)",
            "true",
            &name,
        )
        .await;
        assert_eq!(
            vm.eval("__observerNativeState.result").unwrap(),
            "pass",
            "{name}"
        );
    }
}
