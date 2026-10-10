use super::*;

#[tokio::test(flavor = "current_thread")]
async fn webrtc_local_description_validates_offer_provenance_through_operations_chain() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://local-sdp.test/",
        "http://local-sdp.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/webrtc-local-description.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm, &loader, "String(globalThis.__uiEventResults?.complete)", "true",
            "local description operations must finish through the production selected Page dispatcher",
        ).await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===32 && new Set(__uiEventResults.checks.map(row=>row.name)).size===32").unwrap(), "true");
    }
}

#[test]
fn webrtc_sdp_error_uses_intrinsic_rtc_brand_and_original_line_number() {
    let mut vm = new_parsed_test_vm("https://sdp-error.test/", "<!doctype html><body>");
    vm.eval("globalThis.originalRTCError=RTCError;globalThis.originalDOMException=DOMException;RTCError=function(){throw Error('author constructor')};DOMException=function(){throw Error('author constructor')}").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let error =
            crate::context_bootstrap::build_rtc_sdp_error(scope, "Invalid media port", 6).unwrap();
        assert!(crate::web_api_interfaces::RTCError::is_instance(
            scope, error
        ));
        assert!(crate::web_api_interfaces::DOMException::is_instance(
            scope, error
        ));
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "nativeSdpError").unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), error.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("nativeSdpError instanceof originalRTCError && nativeSdpError instanceof originalDOMException && nativeSdpError.name==='OperationError' && nativeSdpError.code===0 && nativeSdpError.message==='Invalid media port' && nativeSdpError.errorDetail==='sdp-syntax-error' && nativeSdpError.sdpLineNumber===6 && nativeSdpError.sctpCauseCode===null && nativeSdpError.receivedAlert===null && nativeSdpError.sentAlert===null && typeof nativeSdpError.stack==='string'").unwrap(), "true");
}
