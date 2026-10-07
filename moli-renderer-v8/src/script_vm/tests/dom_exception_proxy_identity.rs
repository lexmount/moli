use super::*;

#[test]
fn dom_exception_getters_read_registered_native_proxy_targets() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-error-receiver.test/");
    vm.eval("globalThis.errors = [new DOMException('message','AbortError'),new QuotaExceededError('full',{quota:12})]").unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "errors");
        let errors =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let proxies = v8::Array::new(scope, 2);
        for index in 0..2 {
            let error =
                v8::Local::<v8::Object>::try_from(errors.get_index(scope, index).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, error, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert!(crate::web_api_interfaces::DOMException::is_instance(
                scope,
                proxy.into()
            ));
            assert_eq!(proxies.set_index(scope, index, proxy.into()), Some(true));
        }
        let key = crate::util::v8str(scope, "nativeProxies");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxies.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for(let index=0;index<errors.length;index++) {
        const original=errors[index],proxy=nativeProxies[index];
        for(const name of ['name','message','code']) {
          const getter=Object.getOwnPropertyDescriptor(DOMException.prototype,name).get;
          if(getter.call(proxy)!==getter.call(original))throw Error('native target '+name);
          let traps=0,error;
          const author=new Proxy(proxy,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 43;}});
          try{getter.call(author);}catch(caught){error=caught;}
          if(!(error instanceof TypeError)||traps)throw Error('author proxy');
        }
      }
      if(nativeProxies[1].quota!==12)throw Error('derived native target');
      return true;
    })()"#).unwrap(), "true");
}
