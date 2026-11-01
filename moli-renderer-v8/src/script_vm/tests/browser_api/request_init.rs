use super::*;

#[tokio::test(flavor = "current_thread")]
async fn request_constructor_resets_inherited_navigation_state_only_for_defined_members() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://request-init-navigation.test/",
        &loader,
    );
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "Request").unwrap();
        let source = moli_storage_service::StorageBucketCachedRequest {
            method: "GET".to_owned(),
            headers: vec![("x-native".to_owned(), "original".to_owned())],
            metadata: moli_storage_service::StorageBucketCachedRequestMetadata {
                destination: "document".to_owned(),
                mode: "navigate".to_owned(),
                referrer: "https://request-init-navigation.test/source".to_owned(),
                referrer_policy: "strict-origin".to_owned(),
                is_history_navigation: true,
                is_reload_navigation: true,
                ..Default::default()
            },
        };
        let request = crate::network_host::build_cached_request_object(
            scope,
            "https://request-init-navigation.test/target",
            &source,
        )
        .unwrap();
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeRequest");
        assert_eq!(
            global.create_data_property(scope, key.into(), request.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/request-init-navigation.js"
        ))
        .unwrap(),
        "true"
    );
}

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
