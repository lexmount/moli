use super::*;

#[test]
fn shadow_root_members_validate_receivers_before_argument_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-root-receivers.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/shadow-root-native-receivers.js"
        ))
        .expect("ShadowRoot receiver matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3892");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn shadow_root_members_accept_registered_native_proxies_without_author_traps() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-root-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.shadowProxyRecords = [];
        globalThis.shadowProxyTraps = 0;
        const other = document.querySelector('#child').contentWindow;
        for (const owner of [globalThis, other]) for (const flavor of ['live', 'html', 'xml']) {
          const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
            owner.document.implementation.createHTMLDocument('shadow') :
            owner.document.implementation.createDocument(null, 'r');
          for (const mode of ['open', 'closed']) {
            const host = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
            const root = host.attachShadow({mode, delegatesFocus: true, clonable: true, serializable: true});
            const text = host.appendChild(doc.createTextNode('text'));
            const slot = root.appendChild(doc.createElementNS('http://www.w3.org/1999/xhtml', 'slot'));
            (doc.body || doc.documentElement).appendChild(host);
            for (const kind of ['ShadowRoot', 'Text']) shadowProxyRecords.push({
              owner, doc, host, root, text, slot, mode, kind, target: kind === 'Text' ? text : root,
              callee: owner === globalThis ? other : globalThis,
              handler: {
                get() {shadowProxyTraps++; throw Error('get trap');},
                getPrototypeOf() {shadowProxyTraps++; throw Error('prototype trap');}
              }
            });
          }
        }
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "shadowProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 24);
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
            let proxy = {
                let scope = &mut v8::ContextScope::new(scope, context);
                let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                proxy
            };
            assert_eq!(
                crate::web_api_interfaces::ShadowRoot::is_instance(scope, proxy.into()),
                index % 2 == 0,
            );
            assert_eq!(
                crate::web_api_interfaces::Text::is_instance(scope, proxy.into()),
                index % 2 != 0,
            );
            let key = v8::String::new(scope, "proxy").unwrap();
            assert!(record.set(scope, key.into(), proxy.into()).unwrap());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const [index, record] of shadowProxyRecords.entries()) {
        const {callee, target, proxy, kind, root, host, mode, slot} = record;
        const check = (passed, name) => {if (!passed) throw Error(`${index}/${kind}: ${name}`);};
        const proto = callee[kind].prototype;
        const descriptor = name => Object.getOwnPropertyDescriptor(proto, name);
        const typeError = body => {
          try {body(); return false;}
          catch (error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}
        };
        const author = new Proxy(proxy, {});
        if (kind === 'Text') {
          check(descriptor('assignedSlot').get.call(proxy) === (mode === 'open' ? slot : null), 'native slot');
          check(typeError(() => descriptor('assignedSlot').get.call(author)), 'author Text proxy');
        } else {
          for (const name of ['host', 'mode', 'delegatesFocus', 'slotAssignment', 'clonable',
            'serializable', 'activeElement', 'innerHTML', 'styleSheets', 'adoptedStyleSheets', 'onslotchange']) {
            const get = descriptor(name).get;
            check(get.call(proxy) === get.call(target), 'native getter ' + name);
            check(typeError(() => get.call(author)), 'author root proxy ' + name);
          }
          check(descriptor('host').get.call(proxy) === host && descriptor('mode').get.call(proxy) === mode,
            'native shadow state');
          let conversions = 0;
          const poison = {toString() {conversions++; return '';},
            get [Symbol.iterator]() {conversions++; throw 1;},
            get serializableShadowRoots() {conversions++; throw 1;}};
          check(typeError(() => descriptor('innerHTML').set.call(author, poison)) &&
            typeError(() => descriptor('adoptedStyleSheets').set.call(author, poison)) &&
            typeError(() => proto.getHTML.call(author, poison)) &&
            typeError(() => proto.setHTMLUnsafe.call(author, poison)) && conversions === 0,
            'author proxy before conversion');
          let calls = 0;
          const handler = () => calls++;
          descriptor('onslotchange').set.call(proxy, handler);
          callee.EventTarget.prototype.dispatchEvent.call(proxy, new callee.Event('slotchange'));
          check(calls === 1 && descriptor('onslotchange').get.call(target) === handler, 'native event handler');
          descriptor('onslotchange').set.call(proxy, null);
          descriptor('adoptedStyleSheets').set.call(proxy, []);
          check(descriptor('adoptedStyleSheets').get.call(root).length === 0, 'native stylesheet setter');
          check(proto.getHTML.call(proxy) === proto.getHTML.call(target), 'native serialization');
          check(proto.getSelection.call(proxy) === proto.getSelection.call(target), 'native selection');
        }
        check(shadowProxyTraps === 0, 'no author traps');
      }
      return true;
    })()"#).unwrap(), "true");
}
