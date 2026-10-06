use super::*;

#[test]
fn svg_transform_matrix_validation_respects_read_only_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-transform-matrix-order.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_transform_matrix_order.js"))
        .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "108");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.passed").unwrap(), "108");
}

#[test]
fn svg_transform_lists_synchronize_items_and_validate_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-transform-lists.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_transform_lists.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "7350");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed).slice(0, 20))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.passed").unwrap(), "7350");
}

#[test]
fn svg_transform_registered_proxies_preserve_list_and_matrix_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-transform-proxies.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      const child = document.querySelector('iframe').contentWindow;
      const detached = child.document.implementation.createHTMLDocument('');
      const owner = detached.createElementNS('http://www.w3.org/2000/svg', 'g');
      owner.setAttribute('transform', 'translate(2 3) scale(4 5)');
      const animated = owner.transform, base = animated.baseVal, anim = animated.animVal;
      const transform = base.getItem(0), matrix = transform.matrix;
      globalThis.__transformNativeEntries = {owner, animated, base, anim, transform, matrix};
      globalThis.__transformNativeProxies = {};
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__transformNativeEntries");
        let targets = global.get(scope, key.into()).unwrap();
        let targets = v8::Local::<v8::Object>::try_from(targets).unwrap();
        let key = crate::util::v8str(scope, "__transformNativeProxies");
        let proxies = global.get(scope, key.into()).unwrap();
        let proxies = v8::Local::<v8::Object>::try_from(proxies).unwrap();
        for name in ["owner", "animated", "base", "anim", "transform", "matrix"] {
            let key = crate::util::v8str(scope, name);
            let target = targets.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                proxies.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const p = __transformNativeProxies, t = __transformNativeEntries;
      if (p.owner.transform !== t.animated || p.animated.baseVal !== t.base || p.base[0] !== t.transform) return 'identity';
      p.transform.setTranslate(20,30);
      if (p.matrix.e !== 20 || p.matrix.f !== 30 || p.transform.matrix !== t.matrix) return 'matrix identity';
      p.matrix.a = 7;
      if (p.transform.type !== 1 || p.anim[0].matrix.a !== 7 || p.base[0].matrix.e !== 20) return 'matrix writeback';
      const copy = p.base.appendItem(p.transform);
      if (copy === t.transform || Object.getPrototypeOf(copy) !== child.SVGTransform.prototype) return 'copy producer';
      p.transform.setScale(8,9);
      if (copy.matrix.a !== 7 || p.base.length !== 3 || t.matrix.a !== 8) return 'copy isolation';
      let rejected = 0, conversions = 0;
      const value = {valueOf(){conversions++;return 1}};
      for (const receiver of [new Proxy(p.transform, {}), Object.create(p.transform)]) {
        try { child.SVGTransform.prototype.setTranslate.call(receiver, value, value); }
        catch(e) { if (e instanceof child.TypeError) rejected++; }
      }
      const revoked = Proxy.revocable(p.base, {}); revoked.revoke();
      try { child.SVGTransformList.prototype.removeItem.call(revoked.proxy, value); }
      catch(e) { if (e instanceof child.TypeError) rejected++; }
      const forged = child.document.createElementNS('http://www.w3.org/2000/svg','svg').createSVGTransform();
      Object.setPrototypeOf(forged, child.SVGTransformList.prototype);
      try { child.SVGTransformList.prototype.clear.call(forged); }
      catch(e) { if (e instanceof child.TypeError) rejected++; }
      try { p.anim.clear(); }
      catch(e) { if (e instanceof child.DOMException && e.name === 'NoModificationAllowedError') rejected++; }
      return rejected === 5 && conversions === 0 ? 'ok' : JSON.stringify({rejected, conversions});
    })()"#).unwrap(), "ok");
}
