use super::*;

#[test]
fn video_color_space_preserves_conversion_native_brands_and_json_dictionaries() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-color-space.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("video_color_space.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn video_color_space_accepts_registered_native_proxy_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-color-space.test/");
    vm.eval("globalThis.nativeColorSpace = new VideoColorSpace({primaries:'smpte432', transfer:'iec61966-2-1', matrix:'rgb', fullRange:true})").unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeColorSpace");
        let target =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("nativeColorSpace.primaries + '|' + nativeColorSpace.transfer + '|' + nativeColorSpace.matrix + '|' + nativeColorSpace.fullRange").unwrap(),
        "smpte432|iec61966-2-1|rgb|true"
    );
    assert_eq!(
        vm.eval("JSON.stringify(nativeColorSpace.toJSON())")
            .unwrap(),
        r#"{"fullRange":true,"matrix":"rgb","primaries":"smpte432","transfer":"iec61966-2-1"}"#
    );
    assert_eq!(vm.eval("try { VideoColorSpace.prototype.toJSON.call(new Proxy(nativeColorSpace, {})); 'accepted' } catch(error) { error.name }").unwrap(), "TypeError");
}

#[tokio::test]
async fn video_color_space_clone_rejection_preserves_message_ports_and_indexed_db_transactions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://video-color-space-storage.test/",
        &loader,
    );
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "globalThis.__videoColorSpaceStorageRun = {};",
        include_str!("video_color_space_storage.js")
    ))
    .unwrap();
    let completion = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while vm.eval("__uiEventResults.complete")? != "true" {
            wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader).await?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await;
    let progress = vm.eval("JSON.stringify(__uiEventResults)").unwrap();
    assert!(
        matches!(completion, Ok(Ok(()))),
        "{completion:?}; {progress}"
    );
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "8");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}
