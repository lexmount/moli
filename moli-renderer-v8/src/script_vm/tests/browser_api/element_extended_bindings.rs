use super::*;

#[test]
fn element_extended_members_convert_interfaces_and_preserve_native_values() {
    let mut vm = new_parsed_test_vm(
        "https://element-extended-bindings.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/element-extended-native-bindings.js"
        ))
        .expect("Element extended binding matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "2232");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap()
    );
}

#[test]
fn element_extended_members_preserve_registered_native_proxy_identity() {
    let mut vm = new_parsed_test_vm(
        "https://element-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(r#"
      globalThis.elementProxyRecords = [];
      globalThis.elementProxyTraps = 0;
      const other = document.querySelector('#child').contentWindow;
      for (const owner of [globalThis, other]) for (const flavor of ['live', 'html', 'xml']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('native') : owner.document.implementation.createDocument(null, 'r');
        const root = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        const node = doc.createElementNS('http://www.w3.org/1999/xhtml', 'select');
        const attr = doc.createAttribute('data-native'); attr.value = '\ud800x\udfff';
        (doc.body || doc.documentElement).appendChild(root);
        elementProxyRecords.push({root, node, attr, callee: owner === globalThis ? other : globalThis,
          handler: {get() {elementProxyTraps++; throw Error('get');},
            getPrototypeOf() {elementProxyTraps++; throw Error('getPrototypeOf');}}});
      }
      true;
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "elementProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 6);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "handler").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let handler = v8::Local::<v8::Object>::try_from(value).unwrap();
            for name in ["root", "node", "attr"] {
                let key = v8::String::new(scope, name).unwrap();
                let value = record.get(scope, key.into()).unwrap();
                let object = v8::Local::<v8::Object>::try_from(value).unwrap();
                let target = moli_webapi_declare::web_api_object_target(scope, object).unwrap();
                let context = target.get_creation_context(scope).unwrap();
                let proxy = {
                    let scope = &mut v8::ContextScope::new(scope, context);
                    // Registration pairs one private handler with one Proxy.
                    let private_handler = crate::util::new_null_prototype_object(scope);
                    for trap in ["get", "getPrototypeOf"] {
                        let key = v8::String::new(scope, trap).unwrap();
                        let callback = handler.get(scope, key.into()).unwrap();
                        assert!(
                            private_handler
                                .create_data_property(scope, key.into(), callback)
                                .unwrap()
                        );
                    }
                    let proxy = v8::Proxy::new(scope, target, private_handler).unwrap();
                    moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                    proxy
                };
                assert_eq!(
                    crate::web_api_interfaces::Element::is_instance(scope, proxy.into()),
                    name != "attr"
                );
                assert_eq!(
                    crate::web_api_interfaces::Attr::is_instance(scope, proxy.into()),
                    name == "attr"
                );
                let key = v8::String::new(scope, &format!("{name}Proxy")).unwrap();
                assert!(record.set(scope, key.into(), proxy.into()).unwrap());
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const [index, record] of elementProxyRecords.entries()) {
        const {callee, root, node, attr, rootProxy, nodeProxy, attrProxy} = record;
        const methods = callee.Element.prototype;
        const check = (value, name) => {if (!value) throw Error(`${index}: ${name}`);};
        const typeError = body => {try {body(); return false;} catch(error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}};
        check(methods.setAttributeNode.call(rootProxy, attrProxy) === null, 'attach');
        check(methods.getAttributeNode.call(root, 'data-native') === attrProxy, 'shared Attr cache');
        check(root.getAttribute('data-native') === '\ud800x\udfff', 'native value');
        check(methods.setAttributeNode.call(root, attrProxy) === attrProxy, 'same owner through alias');
        check(methods.removeAttributeNode.call(root, attrProxy) === attrProxy, 'remove identity');
        check(attr.ownerElement === null && attr.value === '\ud800x\udfff', 'detach state');
        check(methods.insertAdjacentElement.call(rootProxy, 'beforeend', nodeProxy) === nodeProxy, 'insert identity');
        check(node.parentNode === root, 'native insertion');
        methods.insertAdjacentText.call(rootProxy, 'afterbegin', '\ud800');
        check(root.firstChild.data === '\ud800', 'native text');
        for (const method of ['setAttributeNode', 'setAttributeNodeNS', 'removeAttributeNode']) {
          check(typeError(() => methods[method].call(root, new Proxy(attrProxy, {}))), 'author Attr proxy');
          check(typeError(() => methods[method].call(new Proxy(rootProxy, {}), attr)), 'author Element proxy');
        }
        check(typeError(() => methods.insertAdjacentElement.call(root, 'beforeend', new Proxy(nodeProxy, {}))), 'author inserted proxy');
        check(elementProxyTraps === 0, 'no author traps');
        root.remove();
      }
      return true;
    })()"#).unwrap(), "true");
}
