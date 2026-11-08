use super::*;

#[test]
fn abort_controller_receivers_use_native_brands_and_callee_realms() {
    let mut vm = new_parsed_test_vm(
        "https://abort-controller-receivers.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/abort-controller-receivers.js"
        ))
        .expect("AbortController receiver matrix should complete");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn abort_controller_receivers_accept_registered_native_proxies_and_preserve_reasons() {
    let mut vm = new_storage_html_test_vm("https://abort-controller-native-proxy.test/");
    vm.eval(
        r#"
        globalThis.nativeControllerTarget = new AbortController();
        globalThis.nativeControllerTraps = 0;
        globalThis.nativeController = new Proxy(nativeControllerTarget, {
          get() {nativeControllerTraps++; throw new Error('get trap');},
          getPrototypeOf() {nativeControllerTraps++; throw new Error('prototype trap');}
        });
        globalThis.nativeAbortReason = document.implementation.createHTMLDocument('').createElement('select');
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeController").unwrap();
        let proxy = global.get(scope, key.into()).unwrap();
        let proxy = v8::Local::<v8::Proxy>::try_from(proxy).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::AbortController::is_instance(
            scope,
            proxy.into()
        ));
        let key = v8::String::new(scope, "nativeAbortReason").unwrap();
        let reason = global.get(scope, key.into()).unwrap();
        assert!(reason.is_proxy(), "reason must exercise a native Proxy");
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const get = Object.getOwnPropertyDescriptor(AbortController.prototype, 'signal').get;
              const signal = get.call(nativeController);
              if (signal !== nativeControllerTarget.signal) return false;
              AbortController.prototype.abort.call(nativeController, nativeAbortReason);
              if (!signal.aborted || signal.reason !== nativeAbortReason) return false;
              if (get.call(nativeController) !== signal || nativeControllerTraps !== 0) return false;
              const author = new Proxy(nativeController, {});
              for (const call of [() => get.call(author), () => AbortController.prototype.abort.call(author)]) {
                try { call(); return false; } catch (error) { if (!(error instanceof TypeError)) return false; }
              }
              return nativeControllerTraps === 0;
            })()"#,
        )
        .unwrap(),
        "true",
    );
}
