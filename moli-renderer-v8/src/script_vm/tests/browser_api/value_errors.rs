use super::*;

#[test]
fn media_and_transport_errors_preserve_native_values_and_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://value-errors.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("value_errors.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "239");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn media_and_transport_error_getters_accept_native_proxies_and_reject_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://value-error-proxies.test/");
    vm.eval(r#"document.body.innerHTML='<iframe></iframe>';
      globalThis.childWindow=document.querySelector('iframe').contentWindow;
      globalThis.errors=[new childWindow.OverconstrainedError('w\ud800','m\udfff'),
        new childWindow.WebTransportError('m\udfff',{source:'session',streamErrorCode:4294967295})];"#).unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "errors");
        let errors =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let proxies = v8::Array::new(scope, 2);
        for index in 0..2 {
            let object =
                v8::Local::<v8::Object>::try_from(errors.get_index(scope, index).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
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
      for(const realm of [window,childWindow]) {
        for(let index=0;index<errors.length;index++) {
          const original=errors[index],proxy=nativeProxies[index];
          const members=index===0?['constraint']:['source','streamErrorCode'];
          const interfaceName=index===0?'OverconstrainedError':'WebTransportError';
          for(const name of [...members,'message','name','code']) {
            const prototype=members.includes(name)?realm[interfaceName].prototype:realm.DOMException.prototype;
            const getter=Object.getOwnPropertyDescriptor(prototype,name).get;
            if(getter.call(proxy)!==getter.call(original))throw Error('native value identity');
            let traps=0;
            const author=new Proxy(proxy,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 43;}});
            const revoked=Proxy.revocable(proxy,{});revoked.revoke();
            for(const receiver of [author,revoked.proxy,Object.create(original)]) {
              let error;try{getter.call(receiver);}catch(caught){error=caught;}
              if(Object.getPrototypeOf(error)!==realm.TypeError.prototype||traps)throw Error('author wrapper and callee realm');
            }
          }
        }
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn web_transport_error_deserialization_respects_native_realm_exposure() {
    let mut source = new_storage_page_task_executor_test_vm("https://transport-error-source.test/");
    source.eval("globalThis.value=new WebTransportError('message',{source:'session',streamErrorCode:42})").unwrap();
    let context_ptr = &source.page_default_context as *const _;
    let mut payload = None;
    source
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "value");
            let value = global.get(scope, key.into()).unwrap();
            payload = crate::structured_clone::serialize_for_wire_for_storage(scope, value);
            Ok(())
        })
        .unwrap();
    let payload = payload.expect("WebTransportError should serialize in its exposed realm");
    drop(source);
    let mut target =
        new_storage_page_task_executor_test_vm("http://transport-error-insecure.test/");
    assert_eq!(
        target
            .eval("isSecureContext+'|'+typeof WebTransportError")
            .unwrap(),
        "false|undefined"
    );
    let context_ptr = &target.page_default_context as *const _;
    target
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let scope = std::pin::pin!(v8::TryCatch::new(scope));
            let scope = &mut scope.init();
            assert!(crate::structured_clone::deserialize_from_wire(scope, &payload).is_none());
            let exception = scope
                .exception()
                .expect("unexposed interface should throw DataCloneError");
            let exception = v8::Local::<v8::Object>::try_from(exception).unwrap();
            assert!(crate::web_api_interfaces::DOMException::is_instance(
                scope, exception
            ));
            let key = crate::util::v8str(scope, "name");
            assert_eq!(
                exception
                    .get(scope, key.into())
                    .unwrap()
                    .to_rust_string_lossy(scope),
                "DataCloneError"
            );
            Ok(())
        })
        .unwrap();
}
