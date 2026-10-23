use super::*;

#[tokio::test]
async fn window_request_like_fetch_validates_inherited_native_signal_after_init() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://request-like-signal.test/",
        &loader,
    );
    let source = format!(
        "globalThis.requestLikeSignalResult='pending'; ({}).then(value=>requestLikeSignalResult=value,error=>requestLikeSignalResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/request-like-signal.js")
    );
    vm.eval(&source).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(requestLikeSignalResult!=='pending')",
        "true",
        "request-like-signal",
    )
    .await;
    assert_eq!(vm.eval("requestLikeSignalResult").unwrap(), "true");
}

#[tokio::test]
async fn request_and_fetch_share_complete_dictionary_conversion_before_materialization() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://request-init-dictionary.test/",
        &loader,
    );
    let source = format!(
        "globalThis.requestInitResult='pending'; ({}).then(value=>requestInitResult=value,error=>requestInitResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/request-init-dictionary.js")
    );
    vm.eval(&source).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(requestInitResult!=='pending')",
        "true",
        "request-init-dictionary",
    )
    .await;
    assert_eq!(vm.eval("requestInitResult").unwrap(), "true");
}
