use super::*;

fn assert_svg_element_receiver_probe(vm: &mut ScriptVm, expected_checks: usize) {
    vm.eval(include_str!("svg_element_receivers.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("__uiEventResults.total").unwrap(),
        expected_checks.to_string()
    );
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_element_members_validate_native_receivers_before_argument_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-element-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_svg_element_receiver_probe(&mut vm, 10812);
}

#[test]
fn svg_element_members_accept_registered_proxies_with_shared_producer_state() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-native-element-receivers.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      const childWindow = document.querySelector('iframe').contentWindow;
      const detached = childWindow.document.implementation.createHTMLDocument('');
      globalThis.__nativeSvgReceivers = Object.create(null);
      for (const tag of ['text','tspan','textPath','pattern','linearGradient',
          'radialGradient','mask','clipPath','image','a']) {
        __nativeSvgReceivers[tag] = {
          target: detached.createElementNS('http://www.w3.org/2000/svg', tag),
        };
      }
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__nativeSvgReceivers");
        let receivers = global.get(scope, key.into()).unwrap();
        let receivers = v8::Local::<v8::Object>::try_from(receivers).unwrap();
        for tag in [
            "text",
            "tspan",
            "textPath",
            "pattern",
            "linearGradient",
            "radialGradient",
            "mask",
            "clipPath",
            "image",
            "a",
        ] {
            let key = crate::util::v8str(scope, tag);
            let entry = receivers.get(scope, key.into()).unwrap();
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
    assert_svg_element_receiver_probe(&mut vm, 10982);
    assert_eq!(
        vm.eval(
            r#"(() => {
          const methods = ['getNumberOfChars','getComputedTextLength','getSubStringLength',
            'getStartPositionOfChar','getEndPositionOfChar','getExtentOfChar',
            'getRotationOfChar','getCharNumAtPosition','selectSubString'];
          for (const tag of ['text','tspan','textPath']) {
            const {target, proxy} = __nativeSvgReceivers[tag];
            let traps = 0;
            const author = new Proxy(proxy, {
              get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;},
            });
            const revoked = Proxy.revocable(proxy, {}); revoked.revoke();
            for (const name of methods) {
              const method = SVGTextContentElement.prototype[name];
              if (name === 'getNumberOfChars' || name === 'getComputedTextLength') {
                if (method.call(proxy) !== method.call(target)) throw Error(name + ' native proxy rejected');
              } else {
                const sentinel = {}; let error, conversions = 0;
                const value = {
                  valueOf() {conversions++; throw sentinel;},
                  get x() {conversions++; throw sentinel;},
                };
                try {method.call(proxy, value, value);} catch (caught) {error = caught;}
                if (error !== sentinel || conversions !== 1) throw Error(name + ' native conversion');
              }
              for (const receiver of [author, revoked.proxy, Object.create(proxy)]) {
                let error, conversions = 0;
                const value = {
                  valueOf() {conversions++; throw 42;}, get x() {conversions++; throw 42;},
                };
                try {method.call(receiver, value, value);} catch (caught) {error = caught;}
                if (!(error instanceof TypeError) || conversions !== 0 || traps !== 0) {
                  throw Error(name + ' author proxy/conversion accepted');
                }
              }
            }
          }
          return true;
        })()"#,
        )
        .unwrap(),
        "true"
    );
}
