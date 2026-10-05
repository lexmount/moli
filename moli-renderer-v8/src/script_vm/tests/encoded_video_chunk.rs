use super::*;

#[test]
fn encoded_video_chunk_preserves_native_identity_and_callee_error_realms() {
    for url in [
        "https://shell-interfaces.test/",
        "http://shell-interfaces.test/",
        "http://localhost/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        assert_eq!(
            vm.eval(include_str!("encoded_video_chunk_shell.js"))
                .unwrap(),
            "ok",
            "{url}"
        );
    }
}

#[test]
fn encoded_video_chunk_preserves_bytes_transfer_conversion_and_serialization() {
    let mut vm = new_storage_page_task_executor_test_vm("https://encoded-video-chunk.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("encoded_video_chunk.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn encoded_video_chunk_accepts_registered_native_proxy_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://encoded-video-chunk.test/");
    vm.eval("globalThis.nativeChunk = new EncodedVideoChunk({type:'delta', timestamp:-5, data:new Uint8Array([4,5])})").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeChunk");
        let chunk =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, chunk, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("nativeChunk.type + '|' + nativeChunk.timestamp + '|' + nativeChunk.duration + '|' + nativeChunk.byteLength").unwrap(), "delta|-5|null|2");
    assert_eq!(
        vm.eval(
            "const dest = new Uint8Array(2); nativeChunk.copyTo(dest); Array.from(dest).join(',')"
        )
        .unwrap(),
        "4,5"
    );
    assert_eq!(vm.eval("try { new Proxy(nativeChunk, {}).copyTo(dest); 'accepted' } catch(error) { error.name }").unwrap(), "TypeError");
}

#[test]
fn encoded_video_chunk_rejects_indexed_db_storage_and_roundtrips_message_ports() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://encoded-video-chunk-storage.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("encoded_video_chunk_storage.js"))
        .unwrap();
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
