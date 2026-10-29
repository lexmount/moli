use super::*;

fn check_file_value_receivers(vm: &mut ScriptVm) {
    vm.eval(include_str!("file_value_receivers.js")).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("__fileValueResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__fileValueResults.failures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn file_value_interfaces_use_native_receiver_and_promise_error_adapters() {
    let mut vm = new_storage_page_task_executor_test_vm("https://file-value-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    check_file_value_receivers(&mut vm);
}

#[test]
fn file_value_interfaces_preserve_registered_proxy_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://file-value-native-proxies.test/");
    vm.eval(r#"
      document.body.innerHTML = '<iframe></iframe>';
      const foreign = document.querySelector('iframe').contentWindow;
      const blob = new foreign.Blob(['contents'], {type: 'text/plain'});
      const file = new foreign.File(['contents'], 'file.txt', {lastModified: 123});
      const transfer = new foreign.DataTransfer(); transfer.items.add(file);
      globalThis.__fileNativeValues = {Blob: {target: blob}, File: {target: file}, FileList: {target: transfer.files}};
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__fileNativeValues");
        let values = global.get(scope, key.into()).unwrap();
        let values = v8::Local::<v8::Object>::try_from(values).unwrap();
        for name in ["Blob", "File", "FileList"] {
            let key = crate::util::v8str(scope, name);
            let entry = values.get(scope, key.into()).unwrap();
            let entry = v8::Local::<v8::Object>::try_from(entry).unwrap();
            let key = crate::util::v8str(scope, "target");
            let target = entry.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, "proxy");
            assert_eq!(
                entry.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    check_file_value_receivers(&mut vm);
}
