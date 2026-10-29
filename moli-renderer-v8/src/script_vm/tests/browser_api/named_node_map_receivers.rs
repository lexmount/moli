use super::*;

#[test]
fn named_node_map_receivers_validate_native_identity_before_attr_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://named-node-map-receivers.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/named-node-map-receivers.js"
        ))
        .expect("NamedNodeMap receiver matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1058");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn named_node_map_receivers_and_attr_arguments_accept_registered_native_proxies() {
    let mut vm = new_parsed_test_vm(
        "https://named-node-map-native-proxies.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.nativeMapRecords = [];
        globalThis.nativeMapTraps = 0;
        const other = document.querySelector('#child').contentWindow;
        function handler() {
          return {
            get() {nativeMapTraps++; throw new Error('get trap');},
            getPrototypeOf() {nativeMapTraps++; throw new Error('prototype trap');}
          };
        }
        for (const owner of [globalThis, other]) {
          for (const flavor of ['live', 'html', 'xml', 'parsed']) {
            const doc = flavor === 'live' ? owner.document :
              flavor === 'html' ? owner.document.implementation.createHTMLDocument('attributes') :
              flavor === 'xml' ? owner.document.implementation.createDocument(null, 'root') :
              new owner.DOMParser().parseFromString('<root/>', 'application/xml');
            const root = flavor === 'xml' || flavor === 'parsed' ? doc.documentElement : doc.createElement('div');
            root.setAttribute('data-x', 'one');
            const map = root.attributes;
            const attr = doc.createAttributeNS('urn:native', 'n:data-native'); attr.value = 'native';
            const nativeMap = Proxy.revocable(map, handler());
            const nativeAttr = Proxy.revocable(attr, handler());
            nativeMapRecords.push({root, map, attr, mapProxy: nativeMap.proxy, attrProxy: nativeAttr.proxy,
              mapRevoke: nativeMap.revoke, attrRevoke: nativeAttr.revoke,
              callee: owner === globalThis ? other : globalThis});
          }
        }
        true;
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeMapRecords").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let records = v8::Local::<v8::Array>::try_from(value).unwrap();
        assert_eq!(records.length(), 8);
        for index in 0..records.length() {
            let value = records.get_index(scope, index).unwrap();
            let record = v8::Local::<v8::Object>::try_from(value).unwrap();
            for (name, is_map) in [("mapProxy", true), ("attrProxy", false)] {
                let key = v8::String::new(scope, name).unwrap();
                let value = record.get(scope, key.into()).unwrap();
                let proxy = v8::Local::<v8::Proxy>::try_from(value).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                assert_eq!(
                    crate::web_api_interfaces::NamedNodeMap::is_instance(scope, proxy.into()),
                    is_map,
                );
                assert_eq!(
                    crate::web_api_interfaces::Attr::is_instance(scope, proxy.into()),
                    !is_map,
                );
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              for (const record of nativeMapRecords) {
                const {root, map, attr, mapProxy, attrProxy, mapRevoke, attrRevoke, callee} = record;
                const prototype = callee.NamedNodeMap.prototype;
                const length = Object.getOwnPropertyDescriptor(prototype, 'length').get;
                const fails = body => {
                  try { body(); return false; }
                  catch (error) { return Object.getPrototypeOf(error) === callee.TypeError.prototype; }
                };
                if (length.call(mapProxy) !== 1 || prototype.item.call(mapProxy, 0) !== root.getAttributeNode('data-x')) return false;
                if (prototype.setNamedItem.call(mapProxy, attrProxy) !== null || attr.ownerElement !== root) return false;
                if (prototype.getNamedItem.call(mapProxy, 'n:data-native') !== attr) return false;
                if (prototype.removeNamedItem.call(mapProxy, 'n:data-native') !== attr || attr.ownerElement !== null) return false;
                if (prototype.setNamedItemNS.call(mapProxy, attrProxy) !== null || attr.ownerElement !== root) return false;
                if (prototype.getNamedItemNS.call(mapProxy, 'urn:native', 'data-native') !== attr) return false;
                if (prototype.removeNamedItemNS.call(mapProxy, 'urn:native', 'data-native') !== attr || attr.ownerElement !== null) return false;
                if (length.call(mapProxy) !== 1 || root.attributes !== map) return false;
                let conversions = 0;
                const name = {toString() {conversions++; throw 1;}};
                if (!fails(() => length.call(new Proxy(mapProxy, {})))) return false;
                if (!fails(() => prototype.getNamedItem.call(new Proxy(mapProxy, {}), name))) return false;
                if (!fails(() => prototype.setNamedItem.call(mapProxy, new Proxy(attrProxy, {})))) return false;
                if (!fails(() => prototype.setNamedItemNS.call(mapProxy, root))) return false;
                if (conversions !== 0 || nativeMapTraps !== 0) return false;
                attrRevoke();
                if (!fails(() => prototype.setNamedItem.call(mapProxy, attrProxy))) return false;
                mapRevoke();
                if (!fails(() => length.call(mapProxy))) return false;
              }
              return nativeMapTraps === 0;
            })()"#,
        )
        .unwrap(),
        "true",
    );
}
