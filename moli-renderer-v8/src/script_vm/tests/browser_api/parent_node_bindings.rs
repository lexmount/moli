use super::*;

#[test]
fn parent_node_members_validate_their_owning_interface_across_realms() {
    let mut vm = new_parsed_test_vm(
        "https://parent-node-bindings.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/parent-node-owner-bindings.js"
        ))
        .expect("ParentNode owner matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "2142");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn parent_node_members_accept_registered_native_element_fragment_and_shadow_proxies() {
    let mut vm = new_parsed_test_vm(
        "https://parent-node-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.parentNodeRecords = [];
        globalThis.parentNodeTraps = 0;
        const nonNodeOwner = document.createElement('div');
        nonNodeOwner.valueOf = () => {parentNodeTraps++; throw new Error('non-Node conversion');};
        globalThis.nonNodeParentTarget = nonNodeOwner.attributes;
        globalThis.nonNodeParentHandler = {
          get() {parentNodeTraps++; throw new Error('non-Node get trap');},
          getPrototypeOf() {parentNodeTraps++; throw new Error('non-Node prototype trap');}
        };
        const other = document.querySelector('#child').contentWindow;
        for (const owner of [globalThis, other]) {
          for (const flavor of ['live', 'html', 'adopted']) {
            for (const kind of ['Element', 'DocumentFragment', 'ShadowRoot']) {
              let doc = flavor === 'live' ? owner.document : flavor === 'html' ?
                owner.document.implementation.createHTMLDocument('parent') :
                new owner.DOMParser().parseFromString('<!doctype html><title>parent</title>', 'text/html');
              const target = kind === 'Element' ? doc.createElement('div') : kind === 'DocumentFragment' ?
                doc.createDocumentFragment() : doc.createElement('div').attachShadow({mode: 'open'});
              const a = target.appendChild(doc.createElement('a')); a.id = 'inside';
              const b = target.appendChild(doc.createElement('b'));
              if (flavor === 'adopted') {
                owner.document.adoptNode(kind === 'ShadowRoot' ? target.host : target);
                doc = owner.document;
              }
              const handler = {
                get() {parentNodeTraps++; throw new Error('get trap');},
                getPrototypeOf() {parentNodeTraps++; throw new Error('prototype trap');}
              };
              parentNodeRecords.push({kind, doc, target, a, b, handler, owner,
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
        let key = v8::String::new(scope, "parentNodeRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 18);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "target").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let receiver = v8::Local::<v8::Object>::try_from(value).unwrap();
            let context = receiver.get_creation_context(scope).unwrap();
            let target = moli_webapi_declare::web_api_object_target(scope, receiver).unwrap();
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
                crate::web_api_interfaces::Element::is_instance(scope, proxy.into()),
                index % 3 == 0,
            );
            assert_eq!(
                crate::web_api_interfaces::DocumentFragment::is_instance(scope, proxy.into()),
                index % 3 != 0,
            );
            assert_eq!(
                crate::web_api_interfaces::ShadowRoot::is_instance(scope, proxy.into()),
                index % 3 == 2,
            );
            let key = v8::String::new(scope, "proxy").unwrap();
            assert!(record.set(scope, key.into(), proxy.into()).unwrap());
        }
        let key = v8::String::new(scope, "nonNodeParentTarget").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let target = v8::Local::<v8::Object>::try_from(value).unwrap();
        let key = v8::String::new(scope, "nonNodeParentHandler").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let handler = v8::Local::<v8::Object>::try_from(value).unwrap();
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::NamedNodeMap::is_instance(
            scope,
            proxy.into(),
        ));
        assert!(
            crate::native_bridge::node_runtime_and_handle_from_object(scope, proxy.into()).is_err(),
        );
        assert!(
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                scope,
                proxy.into(),
            )
            .is_err(),
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              for (const [index, record] of parentNodeRecords.entries()) {
                const {kind, proxy, doc, a, b, owner, callee} = record;
                const check = (passed, name) => {
                  if (!passed) throw new Error(`${index}/${kind}: ${name}`);
                };
                const prototype = callee[kind === 'Element' ? 'Element' : 'DocumentFragment'].prototype;
                const get = name => Object.getOwnPropertyDescriptor(prototype, name).get.call(proxy);
                const fails = body => {
                  try { body(); return false; }
                  catch (error) { return Object.getPrototypeOf(error) === callee.TypeError.prototype; }
                };
                const children = get('children'), list = prototype.querySelectorAll.call(proxy, '*');
                check(children.length === 2 && get('firstElementChild') === a && get('lastElementChild') === b && get('childElementCount') === 2, 'initial children');
                check(prototype.querySelector.call(proxy, '#inside') === a, 'querySelector identity');
                check(list.length === 2 && list[0] === a, 'querySelectorAll contents');
                check(Object.getPrototypeOf(list) === owner.NodeList.prototype, 'querySelectorAll realm');
                if (kind !== 'Element') check(prototype.getElementById.call(proxy, 'inside') === a, 'getElementById identity');
                const first = doc.createElement('first'), last = doc.createElement('last');
                prototype.prepend.call(proxy, first); prototype.append.call(proxy, last);
                check(get('firstElementChild') === first && get('lastElementChild') === last && children.length === 4 && list.length === 2, 'prepend and append');
                prototype.moveBefore.call(proxy, last, first);
                check(get('firstElementChild') === last && get('lastElementChild') === b && children.length === 4, 'moveBefore');
                const next = doc.createElement('next');
                prototype.replaceChildren.call(proxy, next, 'tail');
                check(children.length === 1 && get('firstElementChild') === next && get('lastElementChild') === next, 'replaceChildren');
                let conversions = 0;
                const poison = {toString() {conversions++; throw 1;}};
                const author = new Proxy(proxy, {});
                for (const name of ['children', 'firstElementChild', 'lastElementChild', 'childElementCount']) {
                  check(fails(() => Object.getOwnPropertyDescriptor(prototype, name).get.call(author)), `author proxy ${name}`);
                }
                for (const name of ['prepend', 'append', 'replaceChildren', 'querySelector', 'querySelectorAll']) {
                  check(fails(() => prototype[name].call(author, poison)), `author proxy ${name}`);
                }
                check(fails(() => prototype.moveBefore.call(author, next, null)), 'author proxy moveBefore');
                if (kind !== 'Element') check(fails(() => prototype.getElementById.call(author, poison)), 'author proxy getElementById');
                check(conversions === 0 && parentNodeTraps === 0, 'no conversions or traps');
              }
              return true;
            })()"#,
        )
        .unwrap(),
        "true",
    );
}
