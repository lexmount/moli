use super::*;

#[test]
fn node_tree_mutations_validate_arguments_before_hierarchy_across_realms() {
    let mut vm = new_parsed_test_vm(
        "https://node-tree-mutation-bindings.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/node-tree-mutation-bindings.js"
        ))
        .expect("Node tree mutation matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "16045");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed).slice(0, 30))")
            .unwrap(),
    );
}

#[test]
fn rejected_node_tree_mutations_preserve_source_document_and_parent() {
    let mut vm = new_parsed_test_vm(
        "https://node-tree-mutation-atomicity.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/node-tree-validation-atomicity.js"
        ))
        .expect("Rejected Node tree mutation matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "320");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed).slice(0, 30))")
            .unwrap(),
    );
}

#[test]
fn node_tree_mutations_preserve_author_properties_and_deep_native_identity() {
    let mut vm = new_parsed_test_vm(
        "https://node-tree-mutation-properties.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/node-tree-author-properties.js"
        ))
        .expect("Node mutation property preservation matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "41");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn node_tree_mutations_preserve_registered_native_proxy_arguments_and_receiver_owners() {
    let mut vm = new_parsed_test_vm(
        "https://node-tree-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(r#"
      globalThis.nodeTreeProxyRecords = [];
      globalThis.nodeTreeProxyTraps = 0;
      const other = document.querySelector('#child').contentWindow;
      for (const owner of [globalThis, other]) {
        const callee = owner === globalThis ? other : globalThis;
        for (const flavor of ['live', 'html', 'xml', 'parsed', 'new']) {
          const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
            owner.document.implementation.createHTMLDocument('mutations') : flavor === 'xml' ?
            owner.document.implementation.createDocument(null, 'root') : flavor === 'new' ?
            new owner.Document() : new owner.DOMParser().parseFromString('<root/>', 'application/xml');
          for (const kind of ['Element', 'DocumentFragment', 'ShadowRoot', 'Attr', 'Text']) {
            const parent = kind === 'Element' ? doc.createElement('parent') :
              kind === 'DocumentFragment' ? doc.createDocumentFragment() : kind === 'ShadowRoot' ?
              doc.createElementNS('http://www.w3.org/1999/xhtml', 'div').attachShadow({mode: 'open'}) :
              kind === 'Attr' ? doc.createAttribute('attr') : doc.createTextNode('parent');
            const a = doc.createComment('a'), b = doc.createTextNode('\ud800b\udfff'), c = doc.createComment('c');
            const aliases = [parent, a, b, c].map(target => {
              const handler = Object.create(null);
              handler.get = () => {nodeTreeProxyTraps++; throw Error('native get');};
              handler.getPrototypeOf = () => {nodeTreeProxyTraps++; throw Error('native prototype');};
              return {target, handler};
            });
            nodeTreeProxyRecords.push({kind, doc, parent, a, b, c, aliases, callee,
              calleePrototype: callee.Object.prototype});
          }
        }
      }
      true;
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nodeTreeProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 50);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "calleePrototype").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let prototype = v8::Local::<v8::Object>::try_from(value).unwrap();
            let context = prototype.get_creation_context(scope).unwrap();
            let key = v8::String::new(scope, "aliases").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let aliases = v8::Local::<v8::Array>::try_from(value).unwrap();
            assert_eq!(aliases.length(), 4);
            for index in 0..aliases.length() {
                let value = aliases.get_index(scope, index).unwrap();
                let alias = v8::Local::<v8::Object>::try_from(value).unwrap();
                let key = v8::String::new(scope, "target").unwrap();
                let value = alias.get(scope, key.into()).unwrap();
                let target = v8::Local::<v8::Object>::try_from(value).unwrap();
                let target = moli_webapi_declare::web_api_object_target(scope, target).unwrap();
                let key = v8::String::new(scope, "handler").unwrap();
                let value = alias.get(scope, key.into()).unwrap();
                let handler = v8::Local::<v8::Object>::try_from(value).unwrap();
                let proxy = {
                    // The alias realm intentionally differs from its native
                    // target's owner. Receiver data must come from the target.
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                    moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                    proxy
                };
                assert!(crate::web_api_interfaces::Node::is_instance(
                    scope,
                    proxy.into()
                ));
                let key = v8::String::new(scope, "proxy").unwrap();
                assert!(alias.set(scope, key.into(), proxy.into()).unwrap());
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const [index, record] of nodeTreeProxyRecords.entries()) {
        const {kind, doc, parent, a, b, c, aliases, callee} = record;
        const [receiver, aa, bb, cc] = aliases.map(alias => alias.proxy);
        const methods = callee.Node.prototype;
        const check = (passed, name) => {if (!passed) throw Error(`${index}/${kind}: ${name}`);};
        const typeError = body => {
          try {body(); return false;}
          catch (error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}
        };
        const domError = (name, body) => {
          try {body(); return false;}
          catch (error) {return error.name === name && Object.getPrototypeOf(error) === callee.DOMException.prototype;}
        };
        for (const method of ['appendChild', 'insertBefore', 'replaceChild', 'removeChild']) {
          const args = ['insertBefore', 'replaceChild'].includes(method) ? [aa, bb] : [aa];
          check(typeError(() => methods[method].apply(new Proxy(receiver, {}), args)), `author receiver ${method}`);
          args[0] = new Proxy(aa, {});
          check(typeError(() => methods[method].apply(receiver, args)), `author argument ${method}`);
        }
        for (const method of ['insertBefore', 'replaceChild']) {
          check(typeError(() => methods[method].call(receiver, aa, {})), `second argument ${method}`);
        }
        check(a.parentNode === null && b.parentNode === null && c.parentNode === null, 'invalid arguments are atomic');
        if (kind === 'Attr' || kind === 'Text') {
          check(domError('HierarchyRequestError', () => methods.appendChild.call(receiver, aa)), 'append hierarchy');
          check(domError('HierarchyRequestError', () => methods.insertBefore.call(receiver, aa, null)), 'insert hierarchy');
          check(domError('HierarchyRequestError', () => methods.replaceChild.call(receiver, aa, bb)), 'replace hierarchy');
          check(domError('NotFoundError', () => methods.removeChild.call(receiver, aa)), 'remove membership');
        } else {
          check(methods.appendChild.call(receiver, aa) === aa && parent.firstChild === a, 'append alias identity');
          check(methods.insertBefore.call(receiver, bb, aa) === bb && parent.firstChild === b, 'insert alias identity');
          check(methods.replaceChild.call(receiver, cc, aa) === aa && parent.lastChild === c, 'replace alias identity');
          check(methods.removeChild.call(receiver, bb) === bb && parent.firstChild === c, 'remove alias identity');
          check(a.parentNode === null && b.parentNode === null && c.parentNode === parent, 'native parent links');
          check([a, b, c].every(node => node.ownerDocument === doc) && b.data === '\ud800b\udfff', 'native owner and UTF-16');
        }
        check(nodeTreeProxyTraps === 0, 'no author traps');
      }
      return true;
    })()"#).unwrap(), "true");
}
