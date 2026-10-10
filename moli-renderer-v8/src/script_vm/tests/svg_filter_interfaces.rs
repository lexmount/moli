use super::*;

#[test]
fn svg_filter_interfaces_preserve_native_inheritance_and_realm_contracts() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.getElementById('child').contentWindow;
  const detached = document.implementation.createHTMLDocument('');
  const xml = new DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml');
  for (const [name, parentName] of [["SVGFEComponentTransferElement","SVGElement"],["SVGFEFloodElement","SVGElement"],["SVGFEImageElement","SVGElement"],["SVGFEMergeElement","SVGElement"],["SVGFEMergeNodeElement","SVGElement"],["SVGFETileElement","SVGElement"]]) {
    for (const w of [window, child]) {
      const C = w[name], parent = w[parentName];
      assert(typeof C === 'function' && C.name === name, name + ' exposed');
      assert(Object.getPrototypeOf(C.prototype) === parent.prototype, name + ' prototype inheritance');
      assert(Object.getPrototypeOf(C) === parent, name + ' interface inheritance');
      let error;
      try { new C(); } catch (caught) { error = caught; }
      assert(error instanceof w.TypeError, name + ' illegal constructor realm');
    }
  }
  const nodeType = Object.getOwnPropertyDescriptor(Node.prototype, 'nodeType').get;
  for (const [tag, name] of [["feComponentTransfer","SVGFEComponentTransferElement"],["feFlood","SVGFEFloodElement"],["feImage","SVGFEImageElement"],["feMerge","SVGFEMergeElement"],["feMergeNode","SVGFEMergeNodeElement"],["feTile","SVGFETileElement"]]) {
    for (const [doc, w] of [[document, window], [child.document, child], [detached, window], [xml, window]]) {
      const element = doc.createElementNS(ns, tag), C = w[name];
      assert(element instanceof C && element instanceof w.SVGElement && element instanceof w.Node, tag + ' native factory brand');
      assert(Object.getPrototypeOf(element) === C.prototype && element.constructor === C, tag + ' factory prototype');
      assert(Object.prototype.toString.call(element) === '[object ' + name + ']', tag + ' native tag');
      assert(element.cloneNode(false) instanceof C, tag + ' cloned brand');
      assert(doc.importNode(element, false) instanceof C, tag + ' imported brand');
      element.setAttribute('data-test', 'value');
      assert(element.getAttribute('data-test') === 'value' && nodeType.call(element) === 1, tag + ' inherited DOM behavior');
      for (const receiver of [Object.create(element), new Proxy(element, {})]) {
        let error;
        try { nodeType.call(receiver); } catch (caught) { error = caught; }
        assert(error instanceof TypeError, tag + ' rejects forged native identity');
      }
      assert(!(doc.createElement(tag) instanceof C), tag + ' namespace-sensitive factory');
    }
    const parsed = new DOMParser().parseFromString('<svg xmlns="' + ns + '"><' + tag + '/></svg>', 'image/svg+xml');
    assert(parsed.documentElement.firstElementChild instanceof window[name], tag + ' parsed XML brand');
  }
  return true;
})()"#).unwrap(), "true");
}

#[test]
fn svg_filter_and_marker_getters_validate_native_receivers() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-filter-marker-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_marker_receivers.js"))
        .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1872");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_and_marker_getters_accept_registered_native_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-marker-proxies.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      globalThis.childWindow = document.querySelector('iframe').contentWindow;
      globalThis.foreignFilter = childWindow.document.createElementNS('http://www.w3.org/2000/svg', 'filter');
      globalThis.foreignMarker = childWindow.document.createElementNS('http://www.w3.org/2000/svg', 'marker');
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("foreignFilter", "nativeFilterProxy"),
            ("foreignMarker", "nativeMarkerProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let object = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(object).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
      for (const [name, element, native, properties] of [
        ['SVGFilterElement', foreignFilter, nativeFilterProxy, ['x','y','width','height','filterUnits','primitiveUnits']],
        ['SVGMarkerElement', foreignMarker, nativeMarkerProxy, ['refX','refY','markerWidth','markerHeight','markerUnits','orientType','orientAngle']],
      ]) {
        let traps = 0;
        const author = new Proxy(native, {get() {traps++;throw 42;}, getPrototypeOf() {traps++;throw 42;}});
        const revoked = Proxy.revocable(native, {}); revoked.revoke();
        for (const property of properties) {
          const getter = Object.getOwnPropertyDescriptor(window[name].prototype, property).get;
          const value = getter.call(native);
          if (value !== element[property] || value !== getter.call(element) ||
              Object.getPrototypeOf(value) !== childWindow[value.constructor.name].prototype) {
            throw Error(name + '.' + property + ' native proxy state/realm');
          }
          for (const receiver of [author, revoked.proxy, Object.create(native)]) {
            let error;
            try {getter.call(receiver);} catch (caught) {error = caught;}
            if (!(error instanceof TypeError) || traps !== 0) throw Error(property + ' author proxy accepted');
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
