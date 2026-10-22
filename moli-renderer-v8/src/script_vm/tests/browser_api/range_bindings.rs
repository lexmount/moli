use super::*;

#[test]
fn range_contextual_fragments_use_native_context_and_preserve_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://range-contextual-fragments.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/range-contextual-fragment-native-context.js"
        ))
        .expect("Contextual fragment context matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "420");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn range_members_validate_native_receivers_before_argument_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://range-native-receivers.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/range-native-receivers.js"
        ))
        .expect("Range receiver matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "2380");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn range_registered_native_proxies_share_boundaries_and_selection_identity() {
    let mut vm = new_parsed_test_vm(
        "https://range-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.rangeProxyRecords = [];
        globalThis.rangeProxyTraps = 0;
        const other = document.querySelector('#child').contentWindow;
        for (const owner of [globalThis, other]) {
          for (const flavor of ['live', 'html', 'xml']) {
            for (const kind of ['Range', 'StaticRange']) {
              const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
                owner.document.implementation.createHTMLDocument('range') :
                owner.document.implementation.createDocument(null, 'r');
              const root = doc.createElement('section');
              (doc.body || doc.documentElement).appendChild(root);
              const text = root.appendChild(doc.createTextNode('abcdef'));
              let target;
              if (kind === 'Range') {
                target = doc.createRange(); target.setStart(text, 1); target.setEnd(text, 3);
              } else {
                target = new owner.StaticRange({startContainer: text, startOffset: 1,
                  endContainer: text, endOffset: 3});
              }
              const handler = {
                get() {rangeProxyTraps++; throw new Error('get trap');},
                getPrototypeOf() {rangeProxyTraps++; throw new Error('prototype trap');}
              };
              rangeProxyRecords.push({kind, flavor, owner, target, text, root, handler,
                callee: owner === globalThis ? other : globalThis});
            }
          }
        }
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "rangeProxyRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 12);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "target").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(value).unwrap();
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
            assert!(crate::web_api_interfaces::AbstractRange::is_instance(
                scope,
                proxy.into()
            ));
            assert_eq!(
                crate::web_api_interfaces::Range::is_instance(scope, proxy.into()),
                index % 2 == 0
            );
            assert_eq!(
                crate::web_api_interfaces::StaticRange::is_instance(scope, proxy.into()),
                index % 2 != 0
            );
            let key = v8::String::new(scope, "proxy").unwrap();
            assert!(record.set(scope, key.into(), proxy.into()).unwrap());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const [index, record] of rangeProxyRecords.entries()) {
        const {kind, flavor, owner, callee, proxy, target, root, text} = record;
        const check = (passed, name) => {if (!passed) throw Error(`${index}/${kind}: ${name}`);};
        const get = name => Object.getOwnPropertyDescriptor(callee.AbstractRange.prototype, name).get.call(proxy);
        const typeError = body => {
          try {body(); return false;}
          catch (error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}
        };
        check(get('startContainer') === text && get('endContainer') === text &&
          get('startOffset') === 1 && get('endOffset') === 3 && !get('collapsed'), 'initial boundaries');
        if (kind === 'Range') {
          const methods = callee.Range.prototype;
          const common = Object.getOwnPropertyDescriptor(methods, 'commonAncestorContainer').get;
          check(common.call(proxy) === text && methods.toString.call(proxy) === 'bc', 'native reads');
          const selected = flavor === 'live' ? owner.getSelection() : null;
          if (selected) {selected.removeAllRanges(); selected.addRange(target);}
          methods.setStart.call(proxy, text, 2);
          check(get('startOffset') === 2 && target.startOffset === 2 && methods.toString.call(proxy) === 'c', 'shared record');
          if (selected) check(selected.anchorNode === text && selected.anchorOffset === 2 &&
            selected.focusOffset === 3 && selected.getRangeAt(0) === target, 'selection association');
          const clone = methods.cloneRange.call(proxy);
          check(clone !== target && clone.startContainer === text && clone.startOffset === 2, 'cloneRange');
          const fragment = methods.cloneContents.call(proxy);
          check(fragment.textContent === 'c' && fragment.ownerDocument === text.ownerDocument, 'cloneContents');
          for (const name of ['nodeType', 'parentNode', 'ownerDocument'])
            Object.defineProperty(text, name, {configurable: true, get() {throw Error('public ' + name);}});
          const parsed = methods.createContextualFragment.call(proxy, '<b>parsed</b>');
          for (const name of ['nodeType', 'parentNode', 'ownerDocument']) delete text[name];
          check(parsed.firstChild.localName === 'b' && parsed.textContent === 'parsed' &&
            parsed.ownerDocument === text.ownerDocument, 'native contextual fragment');
          methods.collapse.call(proxy, false);
          check(get('collapsed') && get('startOffset') === 3 && target.startOffset === 3, 'collapse');
          if (selected) {
            check(selected.isCollapsed && selected.anchorOffset === 3, 'selection collapse');
            selected.removeAllRanges(); selected.addRange(clone);
            methods.setStart.call(proxy, text, 1);
            check(selected.anchorOffset === 2 && selected.focusOffset === 3 &&
              selected.getRangeAt(0) === clone, 'replacement selection association');
            methods.collapse.call(clone, true);
            check(selected.isCollapsed && selected.anchorOffset === 2 &&
              selected.getRangeAt(0) === clone, 'cross-realm clone selection association');
            selected.removeAllRanges();
            methods.setStart.call(proxy, text, 0); methods.setEnd.call(clone, text, 3);
            check(selected.rangeCount === 0, 'cleared selection association');
          }
          let conversions = 0;
          const poison = {valueOf() {conversions++; throw 1;}, toString() {conversions++; throw 1;}};
          const author = new Proxy(proxy, {});
          check(typeError(() => methods.setStart.call(author, text, poison)) &&
            typeError(() => methods.createContextualFragment.call(author, poison)) &&
            typeError(() => common.call(author)) && conversions === 0, 'author proxy rejects before conversion');
        }
        const author = new Proxy(proxy, {});
        for (const name of ['startContainer', 'startOffset', 'endContainer', 'endOffset', 'collapsed'])
          check(typeError(() => Object.getOwnPropertyDescriptor(callee.AbstractRange.prototype, name).get.call(author)), 'author proxy ' + name);
        check(rangeProxyTraps === 0, 'no traps');
        root.remove();
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn range_arguments_use_webidl_interface_conversion_and_native_node_state() {
    let mut vm = new_parsed_test_vm(
        "https://range-native-arguments.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/range-native-arguments.js"
        ))
        .expect("Range argument matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "2776");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn range_arguments_preserve_registered_native_proxy_identity_without_traps() {
    let mut vm = new_parsed_test_vm(
        "https://range-native-node-arguments.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.nodeArgumentRecords = [];
        globalThis.nodeArgumentTraps = 0;
        const other = document.querySelector('#child').contentWindow;
        for (const owner of [globalThis, other]) for (const flavor of ['live', 'html', 'xml']) {
          const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
            owner.document.implementation.createHTMLDocument('arguments') :
            owner.document.implementation.createDocument(null, 'r');
          for (const kind of ['Text', 'Element', 'Attr', 'DocumentType', 'Range']) {
            const root = doc.createElement('section'), text = root.appendChild(doc.createTextNode('abcdef'));
            (doc.body || doc.documentElement).appendChild(root);
            const range = doc.createRange(); range.setStart(text, 0); range.setEnd(text, 3);
            const target = kind === 'Text' ? text : kind === 'Element' ? doc.createElement('b') :
              kind === 'Attr' ? doc.createAttribute('x') : kind === 'DocumentType' ?
              doc.implementation.createDocumentType('r', '', '') : range;
            const handler = {
              get() {nodeArgumentTraps++; throw Error('get trap');},
              getPrototypeOf() {nodeArgumentTraps++; throw Error('prototype trap');}
            };
            nodeArgumentRecords.push({owner, doc, root, text, range, kind, target, handler,
              callee: owner === globalThis ? other : globalThis});
          }
        }
        true;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nodeArgumentRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 30);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "target").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(value).unwrap();
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
                crate::web_api_interfaces::Node::is_instance(scope, proxy.into()),
                index % 5 != 4
            );
            assert_eq!(
                crate::web_api_interfaces::Range::is_instance(scope, proxy.into()),
                index % 5 == 4
            );
            let reference = crate::native_bridge::NativeNodeReference::from_object(scope, target);
            if index % 5 == 4 {
                assert!(reference.is_none());
            } else {
                let reference = reference.unwrap();
                let alias =
                    crate::native_bridge::NativeNodeReference::from_object(scope, proxy.into())
                        .unwrap();
                assert!(reference.is_same(scope, &alias));
                assert_eq!(reference.tree_node().is_none(), index % 5 == 2);
            }
            let key = v8::String::new(scope, "proxy").unwrap();
            assert!(record.set(scope, key.into(), proxy.into()).unwrap());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const [index, record] of nodeArgumentRecords.entries()) {
        const {callee, doc, root, text, range, kind, target, proxy} = record;
        const methods = callee.Range.prototype;
        const check = (passed, name) => {if (!passed) throw Error(`${index}/${kind}: ${name}`);};
        const typeError = body => {
          try {body(); return false;}
          catch (error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}
        };
        if (kind === 'Range') {
          check(methods.compareBoundaryPoints.call(range, 0, proxy) === 0, 'Range source');
          check(typeError(() => methods.compareBoundaryPoints.call(range, 0, new Proxy(proxy, {}))), 'author Range proxy');
        } else {
          const init = {startContainer: proxy, startOffset: 0, endContainer: proxy, endOffset: 0};
          if (kind === 'Attr' || kind === 'DocumentType') {
            let caught;
            try {new callee.StaticRange(init);} catch (error) {caught = error;}
            check(caught && caught.name === 'InvalidNodeTypeError' &&
              Object.getPrototypeOf(caught) === callee.DOMException.prototype, 'native invalid endpoint type');
          } else {
            const staticRange = new callee.StaticRange(init);
            check(staticRange.startContainer === proxy && staticRange.endContainer === proxy, 'snapshot identity');
          }
          let conversions = 0;
          check(typeError(() => methods.setStart.call(range, new Proxy(proxy, {}), {
            valueOf() {conversions++; return 0;}
          })) && conversions === 0, 'author Node proxy before offset');
          if (kind === 'Text') {
            methods.setStart.call(range, proxy, 1); methods.setEnd.call(range, proxy, 2);
            check(range.startContainer === text && range.startOffset === 1 && range.endOffset === 2, 'native boundaries');
            check(methods.comparePoint.call(range, proxy, 1) === 0 &&
              methods.isPointInRange.call(range, proxy, 1) && methods.intersectsNode.call(range, proxy), 'native point comparison');
            for (const name of ['setStartBefore', 'setStartAfter', 'setEndBefore', 'setEndAfter', 'selectNode']) {
              methods[name].call(range, proxy);
              const side = name.includes('End') ? 'end' : 'start';
              check(range[side + 'Container'] === root && range[side + 'Offset'] === (name.endsWith('After') ? 1 : 0), name);
            }
            methods.selectNodeContents.call(range, proxy);
            check(range.startContainer === text && range.startOffset === 0 && range.endOffset === 6, 'selectNodeContents');
          } else if (kind === 'Element') {
            const insertion = doc.createRange();
            insertion.selectNodeContents(root); insertion.collapse(true);
            methods.insertNode.call(insertion, proxy);
            check(root.firstChild === target && target.nextSibling === text &&
              root.textContent === 'abcdef', 'insertNode');
            target.remove();
            methods.surroundContents.call(range, proxy);
            check(target.textContent === 'abc' && root.textContent === 'abcdef', 'surroundContents');
          }
        }
        check(nodeArgumentTraps === 0, 'no author traps');
        root.remove();
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn svg_length_specified_units_use_restricted_float_conversion() {
    let mut vm = new_storage_html_test_vm("https://svg-length-float.test/");
    let result = vm.eval(r#"
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        const length = svg.createSVGLength();
        length.newValueSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_NUMBER, 16777217);
        if (length.valueInSpecifiedUnits !== Math.fround(16777217)) throw new Error('float rounding');
        const original = length.valueInSpecifiedUnits;
        for (const value of [NaN, Infinity, -Infinity, 1e39, -1e39, Symbol()]) {
            let error;
            try {length.newValueSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_PX, value);}
            catch (caught) {error = caught;}
            if (!(error instanceof TypeError) || length.valueInSpecifiedUnits !== original)
                throw new Error('restricted float must reject before mutation');
        }
        const seen = [], sentinel = new Error('value');
        try {length.newValueSpecifiedUnits({valueOf() {seen.push('unit'); return 1;}},
            {valueOf() {seen.push('value'); throw sentinel;}});}
        catch (caught) {if (caught !== sentinel) throw caught;}
        if (seen.join() !== 'unit,value' || length.valueInSpecifiedUnits !== original)
            throw new Error('conversion order or exception identity');
        'ok';
    "#).unwrap();
    assert_eq!(result, "ok");
}
