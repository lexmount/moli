use super::*;

#[test]
fn performance_measurement_members_preserve_native_receiver_and_error_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://performance-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let objects = v8::Object::new(scope);
        let proxies = v8::Object::new(scope);
        for interface in [
            "LargestContentfulPaint",
            "PerformanceEventTiming",
            "PerformancePaintTiming",
            "PerformanceServerTiming",
        ] {
            let prototype =
                crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, interface)?;
            let object = v8::Object::new(scope);
            assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
            // Native identity fixtures, not published timing entries or fabricated measurements.
            moli_webapi_declare::initialize_web_api_object(scope, object, interface).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, interface);
            assert_eq!(
                objects.create_data_property(scope, key.into(), object.into()),
                Some(true)
            );
            assert_eq!(
                proxies.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        let objects_key = crate::util::v8str(scope, "nativeMeasurements");
        assert_eq!(
            global.create_data_property(scope, objects_key.into(), objects.into()),
            Some(true)
        );
        let proxies_key = crate::util::v8str(scope, "nativeMeasurementProxies");
        assert_eq!(
            global.create_data_property(scope, proxies_key.into(), proxies.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const foreign=document.getElementById('child').contentWindow;
      for(const [name,attribute] of [['LargestContentfulPaint','size'],['PerformanceEventTiming','processingStart'],['PerformancePaintTiming','paintTime'],['PerformanceServerTiming','name']]) {
        const native=nativeMeasurements[name], proxy=nativeMeasurementProxies[name];
        const getter=Object.getOwnPropertyDescriptor(foreign[name].prototype,attribute)?.get;
        const json=Object.getOwnPropertyDescriptor(foreign[name].prototype,'toJSON')?.value;
        if(typeof getter!=='function'||typeof json!=='function') throw Error('Missing member');
        Object.setPrototypeOf(native,null);
        globalThis[name]=function AuthorConstructor() {throw Error('author constructor');};
        for(const callback of [getter,json]) {
          for(const value of [native,proxy]) {
            let error;try{callback.call(value);}catch(e){error=e;}
            if(!(error instanceof foreign.DOMException)||error.name!=='NotSupportedError') throw Error('Native receiver/error realm');
          }
          let traps=0;const trap=()=>{traps++;throw Error('author trap');};
          const revoked=Proxy.revocable(native,{});revoked.revoke();
          for(const value of [{},Object.create(native),new Proxy(native,{get:trap,getPrototypeOf:trap}),revoked.proxy,new PerformanceMark('wrong-interface')]) {
            let error;try{callback.call(value);}catch(e){error=e;}
            if(!(error instanceof foreign.TypeError)) throw Error('Invalid receiver accepted');
          }
          if(traps!==0) throw Error('Brand check executed author trap');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
