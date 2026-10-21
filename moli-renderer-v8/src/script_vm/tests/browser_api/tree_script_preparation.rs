use super::*;

#[test]
fn tree_script_preparation_uses_the_current_document_across_owners_and_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tree-script-owners.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/tree-script-owners.js"
        ))
        .expect("tree script owner matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "378");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn tree_script_preparation_preserves_early_bailouts_and_consumes_disabled_scripts() {
    let mut vm = new_storage_page_task_executor_test_vm("https://script-eligibility.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/script-preparation-eligibility.js"
        ))
        .expect("script preparation state matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "30");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn tree_script_preparation_accepts_native_proxy_aliases_without_using_the_alias_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-script-alias.test/");
    vm.eval(r#"
      document.body.innerHTML = '<iframe id=child></iframe>';
      globalThis.scriptProxyRecords = [];
      globalThis.scriptProxyTraps = 0;
      const other = document.querySelector('#child').contentWindow, popup = open();
      for (const owner of [globalThis, other, popup]) {
        owner.__nativeScriptTrace = Object.create(null);
        for (const flavor of ['live', 'adopted']) {
          const source = flavor === 'live' ? owner.document : owner.document.implementation.createHTMLDocument('inert');
          for (const method of ['appendChild', 'insertBefore', 'replaceChild', 'append', 'prepend', 'replaceChildren', 'before', 'after', 'replaceWith']) {
            const parent = source.createElement('section'); owner.document.body.appendChild(parent);
            const pivot = parent.appendChild(owner.document.createElement('i'));
            const script = owner.document.createElement('script'), label = String(scriptProxyRecords.length);
            script.textContent = `globalThis.__nativeScriptTrace[${JSON.stringify(label)}] = (globalThis.__nativeScriptTrace[${JSON.stringify(label)}] || 0) + 1; globalThis.__nativeScriptCurrent = document.currentScript;`;
            const receiver = ['before', 'after', 'replaceWith'].includes(method) ? pivot : parent;
            const aliases = [receiver, script, pivot].map(target => {
              const handler = Object.create(null);
              handler.get = () => {scriptProxyTraps++; throw Error('native get trap');};
              handler.getPrototypeOf = () => {scriptProxyTraps++; throw Error('native prototype trap');};
              return {target, handler};
            });
            const callee = owner === globalThis ? other : globalThis;
            scriptProxyRecords.push({owner, parent, script, label, method, aliases, callee,
              calleePrototype: callee.Object.prototype});
          }
        }
      }
      true;
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "scriptProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 54);
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
            assert_eq!(aliases.length(), 3);
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
      for (const {owner, parent, script, label, method, aliases, callee} of scriptProxyRecords) {
        const [receiver, argument, pivot] = aliases.map(alias => alias.proxy);
        const operation = ['appendChild', 'insertBefore', 'replaceChild'].includes(method) ?
          callee.Node.prototype[method] : callee.Element.prototype[method];
        const args = ['insertBefore', 'replaceChild'].includes(method) ? [argument, pivot] : [argument];
        const check = (passed, message) => {if (!passed) throw Error(`${label}/${method}: ${message}`);};
        const invalidCalls = [[new Proxy(receiver, {}), args]];
        // ParentNode/ChildNode accept (Node or DOMString): an unbranded Node
        // argument enters DOMString conversion rather than a Node brand error.
        if (['appendChild', 'insertBefore', 'replaceChild'].includes(method)) {
          invalidCalls.push([receiver, [new Proxy(argument, {}), ...args.slice(1)]]);
        }
        for (const [target, values] of invalidCalls) {
          let rejected = false;
          try {operation.apply(target, values);}
          catch (error) {rejected = Object.getPrototypeOf(error) === callee.TypeError.prototype;}
          check(rejected && owner.__nativeScriptTrace[label] === undefined, 'author proxy rejected before execution');
        }
        const result = operation.apply(receiver, args);
        const expected = method === 'replaceChild' ? pivot : ['appendChild', 'insertBefore'].includes(method) ? argument : undefined;
        check(result === expected, 'argument alias return identity');
        check(owner.__nativeScriptTrace[label] === 1 && owner.__nativeScriptCurrent === script, 'synchronous owner currentScript');
        check(script.ownerDocument === owner.document && owner.document.currentScript === null, 'owner restored');
        script.remove(); parent.appendChild(script);
        check(owner.__nativeScriptTrace[label] === 1, 'already-started script is not repeated');
        check(scriptProxyTraps === 0, 'no alias traps'); parent.remove();
      }
      popup.close(); return true;
    })()"#).unwrap(), "true");
}
