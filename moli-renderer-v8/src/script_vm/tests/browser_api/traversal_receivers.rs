use super::*;

#[test]
fn traversal_receivers_preserve_native_state_across_realms_and_document_types() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-receivers.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/traversal-receivers.js"
        ))
        .expect("traversal receiver matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1102");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn traversal_receivers_accept_registered_native_proxies_without_author_traps() {
    let mut vm = new_parsed_test_vm(
        "https://traversal-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.nativeTraversalRecords = [];
        globalThis.nativeTraversalTraps = 0;
        const other = document.querySelector('#child').contentWindow;
        for (const owner of [globalThis, other]) {
          for (const kind of ['NodeIterator', 'TreeWalker']) {
            const root = owner.document.createElement('root');
            const a = root.appendChild(owner.document.createElement('a'));
            const b = root.appendChild(owner.document.createElement('b'));
            const target = owner.document['create' + kind](root, 1);
            const {proxy, revoke} = Proxy.revocable(target, {
              get() {nativeTraversalTraps++; throw new Error('get trap');},
              getPrototypeOf() {nativeTraversalTraps++; throw new Error('prototype trap');}
            });
            nativeTraversalRecords.push({kind, root, a, b, target, proxy, revoke,
              callee: owner === globalThis ? other : globalThis});
          }
        }
        true;
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeTraversalRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 4);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            let key = v8::String::new(scope, "proxy").unwrap();
            let value = record.get(scope, key.into()).unwrap();
            let proxy = v8::Local::<v8::Proxy>::try_from(value).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                crate::web_api_interfaces::NodeIterator::is_instance(scope, proxy.into()),
                index % 2 == 0,
            );
            assert_eq!(
                crate::web_api_interfaces::TreeWalker::is_instance(scope, proxy.into()),
                index % 2 == 1,
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              for (const record of nativeTraversalRecords) {
                const {kind, root, a, b, proxy, revoke, callee} = record;
                const prototype = callee[kind].prototype;
                const get = name => Object.getOwnPropertyDescriptor(prototype, name).get.call(proxy);
                const invoke = name => prototype[name].call(proxy);
                const fails = body => {
                  try { body(); return false; }
                  catch (error) { return Object.getPrototypeOf(error) === callee.TypeError.prototype; }
                };
                if (get('root') !== root || get('whatToShow') !== 1 || get('filter') !== null) return false;
                let members;
                if (kind === 'NodeIterator') {
                  if (get('referenceNode') !== root || !get('pointerBeforeReferenceNode')) return false;
                  if (invoke('nextNode') !== root || invoke('previousNode') !== root) return false;
                  if (invoke('nextNode') !== root || invoke('nextNode') !== a) return false;
                  if (invoke('detach') !== undefined || invoke('nextNode') !== b) return false;
                  members = ['root', 'whatToShow', 'filter', 'referenceNode', 'pointerBeforeReferenceNode'];
                } else {
                  const set = Object.getOwnPropertyDescriptor(prototype, 'currentNode').set;
                  if (get('currentNode') !== root || invoke('firstChild') !== a) return false;
                  if (invoke('nextSibling') !== b || invoke('previousSibling') !== a) return false;
                  if (invoke('parentNode') !== root || invoke('lastChild') !== b) return false;
                  if (invoke('previousNode') !== a || invoke('nextNode') !== b) return false;
                  if (set.call(proxy, a) !== undefined || get('currentNode') !== a) return false;
                  if (!fails(() => set.call(new Proxy(proxy, {}), root))) return false;
                  members = ['root', 'whatToShow', 'filter', 'currentNode'];
                }
                for (const name of members) {
                  const getter = Object.getOwnPropertyDescriptor(prototype, name).get;
                  if (!fails(() => getter.call(new Proxy(proxy, {})))) return false;
                }
                for (const name of Object.getOwnPropertyNames(prototype)) {
                  if (name === 'constructor' || members.includes(name)) continue;
                  if (!fails(() => prototype[name].call(new Proxy(proxy, {})))) return false;
                }
                const opposite = callee[kind === 'NodeIterator' ? 'TreeWalker' : 'NodeIterator'].prototype;
                for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(opposite))) {
                  if (name === 'constructor') continue;
                  for (const receiver of [record.target, proxy]) {
                    if (descriptor.get && !fails(() => descriptor.get.call(receiver))) return false;
                    if (descriptor.set && !fails(() => descriptor.set.call(receiver, root))) return false;
                    if (typeof descriptor.value === 'function' && !fails(() => descriptor.value.call(receiver))) return false;
                  }
                }
                const rootGetter = Object.getOwnPropertyDescriptor(prototype, 'root').get;
                revoke();
                if (!fails(() => rootGetter.call(proxy))) return false;
              }
              return nativeTraversalTraps === 0;
            })()"#,
        )
        .unwrap(),
        "true",
    );
}
