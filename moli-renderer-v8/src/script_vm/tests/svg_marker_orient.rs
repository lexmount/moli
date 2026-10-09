use super::*;

#[test]
fn svg_marker_orient_reflects_native_attributes_with_webidl_receiver_checks() {
    let mut vm = new_parsed_test_vm(
        "https://svg-marker-orient.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    vm.eval(include_str!("svg_marker_orient.js")).unwrap();
    assert_eq!(
        vm.eval("__svgMarkerOrientResults.complete").unwrap(),
        "true"
    );
    assert_eq!(vm.eval("__svgMarkerOrientResults.total").unwrap(), "376");
    assert_eq!(
        vm.eval("JSON.stringify(__svgMarkerOrientResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_marker_orient_accepts_registered_native_proxies_before_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-marker-proxy.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      globalThis.child = document.querySelector('iframe').contentWindow;
      globalThis.marker = child.document.implementation.createHTMLDocument('')
        .createElementNS('http://www.w3.org/2000/svg', 'marker');
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "marker");
        let target =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeMarker");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
      for (const realm of [window, child]) {
        const {get, set} = Object.getOwnPropertyDescriptor(realm.SVGMarkerElement.prototype, 'orient');
        let conversions = 0;
        if (set.call(nativeMarker, {toString() { conversions++; return '90deg'; }}) !== undefined ||
            conversions !== 1 || get.call(nativeMarker) !== '90deg' ||
            marker.getAttribute('orient') !== '90deg' || marker.orientAngle.baseVal.value !== 90)
          return false;
        let traps = 0;
        const author = new Proxy(nativeMarker, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const revoked = Proxy.revocable(nativeMarker, {}); revoked.revoke();
        for (const receiver of [author, revoked.proxy, Object.create(nativeMarker)]) {
          for (const invoke of [() => get.call(receiver), () => set.call(receiver,
            {toString() { conversions++; throw 42; }})]) {
            let error; try { invoke(); } catch (value) { error = value; }
            if (Object.getPrototypeOf(error) !== realm.TypeError.prototype || conversions !== 1 || traps !== 0)
              return false;
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
