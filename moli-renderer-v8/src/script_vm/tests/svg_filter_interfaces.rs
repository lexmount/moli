use super::*;

#[test]
fn svg_filter_primitive_reflection_and_receiver_brands() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-reflection.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_reflection.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_registered_native_proxies_share_receiver_state_and_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-proxies.test/");
    vm.eval(r#"document.body.innerHTML = '<iframe id=child></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        globalThis.foreignBlend = childWindow.document.createElementNS('http://www.w3.org/2000/svg', 'feBlend');
        globalThis.foreignImage = childWindow.document.createElementNS('http://www.w3.org/2000/svg', 'feImage');
        globalThis.foreignInput = foreignBlend.in1;"#).unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("foreignBlend", "nativeBlendProxy"),
            ("foreignImage", "nativeImageProxy"),
            ("foreignInput", "nativeInputProxy"),
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
    assert_eq!(vm.eval(r#"(() => {
        const descriptor = (prototype, name) => Object.getOwnPropertyDescriptor(prototype, name);
        for (const name of ['x','y','width','height','in1','in2','result']) {
          const getter = descriptor(SVGFEBlendElement.prototype, name).get;
          const value = getter.call(nativeBlendProxy);
          const C = ['x','y','width','height'].includes(name) ? childWindow.SVGAnimatedLength : childWindow.SVGAnimatedString;
          if (value !== foreignBlend[name] || Object.getPrototypeOf(value) !== C.prototype) throw Error(name + ' native proxy state/realm');
        }
        for (const name of ['href','preserveAspectRatio']) {
          const getter = descriptor(SVGFEImageElement.prototype, name).get;
          if (getter.call(nativeImageProxy) !== foreignImage[name]) throw Error(name + ' native image state');
        }
        const base = descriptor(SVGAnimatedString.prototype, 'baseVal'), anim = descriptor(SVGAnimatedString.prototype, 'animVal');
        base.set.call(nativeInputProxy, 'native\ud800value');
        if (base.get.call(nativeInputProxy) !== 'native\ud800value' || anim.get.call(nativeInputProxy) !== 'native\ud800value' || foreignBlend.getAttribute('in') !== 'native\ud800value') throw Error('native string proxy');
        let traps = 0, conversions = 0, error;
        const author = new Proxy(nativeInputProxy, {get() {traps++;throw 42;}});
        try {base.set.call(author, {toString() {conversions++;return 'bad';}});} catch (caught) {error = caught;}
        if (!(error instanceof TypeError) || traps !== 0 || conversions !== 0) throw Error('author proxy brand/order');
        return true;
    })()"#).unwrap(), "true");
}

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
