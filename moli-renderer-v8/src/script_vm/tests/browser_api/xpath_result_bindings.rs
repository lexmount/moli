use super::*;

#[test]
fn xpath_result_members_validate_receivers_and_convert_indices_before_algorithms() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-result-bindings.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/xpath-result-native-bindings.js"
        ))
        .expect("XPathResult binding matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3240");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap()
    );
}

#[test]
fn xpath_result_registered_native_aliases_share_values_cursor_and_mutation_state() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-result-native-aliases.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(r#"
      globalThis.xpathProxyRecords = [];
      globalThis.xpathProxyTraps = 0;
      const other = document.querySelector('#child').contentWindow;
      for (const owner of [globalThis, other]) for (const flavor of ['live', 'html', 'xml']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('xpath') : owner.document.implementation.createDocument(null, 'r');
        for (let type=1;type<=9;type++) {
          const root = doc.createElement('root');
          const a = root.appendChild(doc.createElement('a'));
          const b = root.appendChild(doc.createElement('b'));
          (doc.body || doc.documentElement).appendChild(root);
          xpathProxyRecords.push({owner, doc, type, root, a, b,
            callee: owner === globalThis ? other : globalThis,
            handler: {get() {xpathProxyTraps++; throw Error('get');},
              getPrototypeOf() {xpathProxyTraps++; throw Error('getPrototypeOf');}}});
        }
      }
      // Materialize the trees before taking any iterator mutation baseline.
      for (const record of xpathProxyRecords) {
        const {doc,root,type} = record;
        record.target = doc.evaluate(type===1?'7':type===2?"'value'":type===3?'true()':'child::*',root,null,type,null);
      }
      true;
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "xpathProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 54);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "target").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(value).unwrap();
            let target = moli_webapi_declare::web_api_object_target(scope, target).unwrap();
            let context = target.get_creation_context(scope).unwrap();
            let key = v8::String::new(scope, "handler").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let handler = v8::Local::<v8::Object>::try_from(value).unwrap();
            for name in ["firstAlias", "secondAlias"] {
                let proxy = {
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let private_handler = crate::util::new_null_prototype_object(scope);
                    for trap in ["get", "getPrototypeOf"] {
                        let key = v8::String::new(scope, trap).unwrap();
                        let value = handler.get(scope, key.into()).unwrap();
                        assert!(
                            private_handler
                                .create_data_property(scope, key.into(), value)
                                .unwrap()
                        );
                    }
                    let proxy = v8::Proxy::new(scope, target, private_handler).unwrap();
                    moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                    proxy
                };
                assert!(crate::web_api_interfaces::XPathResult::is_instance(
                    scope,
                    proxy.into()
                ));
                let key = v8::String::new(scope, name).unwrap();
                assert!(record.set(scope, key.into(), proxy.into()).unwrap());
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const [index, record] of xpathProxyRecords.entries()) {
        const {callee,target,firstAlias,secondAlias,type,a,b} = record;
        const methods = callee.XPathResult.prototype;
        const get = (name,receiver) => Object.getOwnPropertyDescriptor(methods,name).get.call(receiver);
        const check = (value,name) => {if (!value) throw Error(`${index}/${type}: ${name}`);};
        const typeError = body => {try {body();return false;} catch(error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}};
        for (const receiver of [target,firstAlias,secondAlias]) {
          check(get('resultType',receiver) === type && !get('invalidIteratorState',receiver), 'initial state');
          if (type===1) check(get('numberValue',receiver) === 7, 'number');
          if (type===2) check(get('stringValue',receiver) === 'value', 'string');
          if (type===3) check(get('booleanValue',receiver) === true, 'boolean');
          if (type===8 || type===9) check(get('singleNodeValue',receiver) === a, 'single node');
          if (type===6 || type===7) check(get('snapshotLength',receiver) === 2 && methods.snapshotItem.call(receiver,0) === a && methods.snapshotItem.call(receiver,1) === b, 'snapshot');
        }
        if (type===4 || type===5) check(methods.iterateNext.call(firstAlias) === a &&
          methods.iterateNext.call(target) === b && methods.iterateNext.call(secondAlias) === null &&
          methods.iterateNext.call(firstAlias) === null, 'shared iterator cursor');
        let converted = 0;const sentinel = {};
        try {methods.snapshotItem.call(firstAlias,{valueOf(){converted++;throw sentinel;}});throw Error('missing exception');}
        catch(error) {check(error === sentinel && converted === 1,'index conversion before result kind');}
        const author = new Proxy(firstAlias,{});
        for (const member of ['resultType','invalidIteratorState','numberValue','stringValue','booleanValue','singleNodeValue','snapshotLength'])
          check(typeError(() => get(member,author)), 'author alias ' + member);
        converted = 0;
        check(typeError(() => methods.snapshotItem.call(author,{valueOf(){converted++;return 0;}})) && converted === 0, 'author alias before conversion');
        check(xpathProxyTraps === 0, 'no author traps');
      }
      // Mutations invalidate a whole Document. Test all cursors before mutating
      // any tree so another record's legitimate invalidation cannot mask a bug.
      for (const [index, record] of xpathProxyRecords.entries()) {
        const {callee,target,firstAlias,secondAlias,type,root,a,b} = record;
        if (type<4 || type>7) continue;
        root.setAttribute('mutation','changed');
        const methods = callee.XPathResult.prototype;
        const get = (name,receiver) => Object.getOwnPropertyDescriptor(methods,name).get.call(receiver);
        for (const receiver of [target,firstAlias,secondAlias]) {
          if (type===4 || type===5) {
            if (!get('invalidIteratorState',receiver)) throw Error(`${index}: missing invalidation`);
            let error;try {methods.iterateNext.call(receiver);} catch(caught) {error=caught;}
            if (error?.name !== 'InvalidStateError' || error.code !== 11 || Object.getPrototypeOf(error) !== callee.DOMException.prototype)
              throw Error(`${index}: invalid iterator error`);
          } else if (get('invalidIteratorState',receiver) || get('snapshotLength',receiver) !== 2 ||
            methods.snapshotItem.call(receiver,0) !== a || methods.snapshotItem.call(receiver,1) !== b)
            throw Error(`${index}: mutated snapshot changed`);
        }
      }
      for (const record of xpathProxyRecords) record.root.remove();
      return true;
    })()"#).unwrap(), "true");
}
