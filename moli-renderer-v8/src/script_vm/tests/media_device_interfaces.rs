use super::*;

#[test]
fn media_device_interfaces_preserve_inheritance_brands_and_secure_exposure() {
    for url in [
        "https://media-device-interfaces.test/",
        "http://media-device-interfaces.test/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        vm.eval(&format!("({}).then(value => globalThis.__mediaDeviceDone = value, error => globalThis.__mediaDeviceDone = String(error));", include_str!("media_device_interfaces.js"))).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__mediaDeviceDone").unwrap(), "true", "{url}");
    }
}

#[test]
fn media_device_serialization_reads_native_fields_and_preserves_utf16() {
    let mut vm = new_storage_page_task_executor_test_vm("https://media-device-values.test/");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
                scope,
                "InputDeviceInfo",
            )?;
            // A native test fixture, not a fake device published by enumerateDevices.
            let object = v8::Object::new(scope);
            assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
            moli_webapi_declare::initialize_web_api_object(scope, object, "InputDeviceInfo")
                .unwrap();
            for (slot, value) in [
                ("__moliMediaDeviceId", "native-id"),
                ("__moliMediaDeviceKind", "audioinput"),
                ("__moliMediaDeviceGroupId", "native-group"),
            ] {
                let value = crate::util::v8str(scope, value);
                crate::util::set_private_value(scope, object, slot, value.into());
            }
            let label =
                v8::String::new_from_two_byte(scope, &[0xd800], v8::NewStringType::Normal).unwrap();
            crate::util::set_private_value(scope, object, "__moliMediaDeviceLabel", label.into());
            assert_eq!(
                context.global(scope).create_data_property(
                    scope,
                    crate::util::v8str(scope, "nativeDevice").into(),
                    object.into()
                ),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, message) => {if (!ok) throw Error(message);};
      const device = nativeDevice, prototype = MediaDeviceInfo.prototype;
      assert(device instanceof InputDeviceInfo && device instanceof MediaDeviceInfo, 'native inherited brand');
      assert(device.deviceId === 'native-id' && device.kind === 'audioinput' && device.groupId === 'native-group' && device.label.charCodeAt(0) === 0xd800, 'native values');
      const expected = device.toJSON();
      let reads = 0, writes = 0;
      for (const name of ['deviceId','kind','label','groupId']) {
        Object.defineProperty(device,name,{configurable:true,get(){reads++;throw Error('author getter');}});
        Object.defineProperty(Object.prototype,name,{configurable:true,set(){writes++;throw Error('inherited setter');}});
      }
      let json;
      try {json = prototype.toJSON.call(device);} finally {for (const name of ['deviceId','kind','label','groupId']) delete Object.prototype[name];}
      assert(reads === 0 && writes === 0 && JSON.stringify(json) === JSON.stringify(expected), 'private native serialization and data properties');
      assert(json !== expected && Object.getPrototypeOf(json) === Object.prototype, 'fresh ordinary JSON object');
      const first = device.getCapabilities(), second = device.getCapabilities();
      assert(first !== second && Object.keys(first).length === 0, 'fresh empty capabilities shim');
      Object.setPrototypeOf(device,null);
      assert(prototype.toJSON.call(device).deviceId === 'native-id', 'brand independent of public prototype');
      for (const receiver of [Object.create(device), new Proxy(device,{})]) {
        let error;try {prototype.toJSON.call(receiver);} catch(e){error=e;}
        assert(error instanceof TypeError, 'author objects do not acquire native brand');
      }
      return true;
    })()"#).unwrap(), "true");
}
