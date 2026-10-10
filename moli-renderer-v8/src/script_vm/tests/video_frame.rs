use super::*;

#[test]
fn video_frame_raw_planes_lifecycle_conversion_and_serialization() {
    for url in ["https://video-frame.test/", "http://video-frame.test/"] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval(include_str!("video_frame.js")).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("__uiEventResults.complete")
                .unwrap(),
            "true"
        );
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]"
        );
    }
}

#[test]
fn video_frame_registered_native_proxy_receivers_preserve_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-frame.test/");
    vm.eval("globalThis.frame=new VideoFrame(new Uint8Array([1,2,3,255]),{format:'RGBA',codedWidth:1,codedHeight:1,timestamp:7})").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "frame");
        let frame =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, frame, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("frame.timestamp+'|'+frame.clone().format+'|'+frame.allocationSize()")
            .unwrap(),
        "7|RGBA|4"
    );
    assert_eq!(
        vm.eval("try{new Proxy(frame,{}).clone();'accepted'}catch(e){e.name}")
            .unwrap(),
        "TypeError"
    );
    assert_eq!(
        vm.eval("frame.close();frame.format===null").unwrap(),
        "true"
    );
}

#[test]
fn video_frame_storage_rejection_keeps_transactions_and_message_ports_usable() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-frame-storage.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("video_frame_storage.js")).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("__uiEventResults.complete")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
