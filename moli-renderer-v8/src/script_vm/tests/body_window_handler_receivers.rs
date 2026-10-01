use super::*;

#[test]
fn body_and_frameset_window_handlers_validate_native_interface_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://body-handler-brands.test/");
    let result = vm
        .eval(include_str!("body_window_handler_receivers.js"))
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(result, "true");
}

#[test]
fn body_window_handlers_accept_registered_native_proxies_and_reject_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://body-handler-proxy.test/");
    vm.eval("globalThis.nativeBody = document.createElement('body')")
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let body = global
            .get(scope, crate::util::v8str(scope, "nativeBody").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("native body fixture");
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, body, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::HTMLBodyElement::is_instance(
            scope,
            proxy.into()
        ));
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeBodyProxy").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const assert = (ok, message) => { if (!ok) throw Error(message); };
              const descriptor = Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, 'onmessage');
              const callback = () => {};
              try {
                descriptor.set.call(nativeBodyProxy, callback);
                assert(descriptor.get.call(nativeBodyProxy) === callback && window.onmessage === callback, 'native Proxy forwards to Window');
                let traps = 0;
                const trap = () => { traps++; throw Error('author trap'); };
                for (const receiver of [new Proxy(nativeBodyProxy, {get: trap, getPrototypeOf: trap}), Object.create(nativeBodyProxy)]) {
                  let error;
                  try { descriptor.set.call(receiver, null); } catch (value) { error = value; }
                  assert(error instanceof TypeError, 'author wrapper rejects');
                }
                assert(traps === 0 && window.onmessage === callback, 'receiver validation has no author side effects');
                return true;
              } finally { window.onmessage = null; }
            })()"#
        )
        .unwrap(),
        "true"
    );
}
