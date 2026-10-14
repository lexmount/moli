use super::*;

#[test]
fn svg_animated_string_reflection_preserves_native_utf16_and_namespace_identity() {
    let mut vm = new_parsed_test_vm("https://svg-animated-string-values.test/", "<body></body>");
    assert_eq!(
        vm.eval(include_str!("svg_animated_string_native_values.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn svg_animated_strings_use_native_brands_and_registered_proxy_state() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-animated-string-proxies.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      globalThis.foreign = document.querySelector('iframe').contentWindow;
      globalThis.element = foreign.document.createElementNS('http://www.w3.org/2000/svg', 'use');
      globalThis.animated = element.href;
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "animated");
        let value = global.get(scope, key.into()).unwrap();
        let value = v8::Local::<v8::Object>::try_from(value).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, value, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeAnimatedProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name);};
      for (const callee of [window, foreign]) {
        const base = Object.getOwnPropertyDescriptor(callee.SVGAnimatedString.prototype, 'baseVal');
        const anim = Object.getOwnPropertyDescriptor(callee.SVGAnimatedString.prototype, 'animVal');
        base.set.call(nativeAnimatedProxy, '\uD800');
        assert(base.get.call(animated) === '\uD800' && anim.get.call(nativeAnimatedProxy) === '\uD800', 'native proxy slots');
        assert(element.getAttribute('href') === '\uD800', 'native attribute storage');
        element.setAttribute('href', '\uDC00');
        assert(base.get.call(nativeAnimatedProxy) === '\uDC00', 'native attribute mutation');
        let traps = 0;
        const author = new Proxy(animated, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
        const revoked = Proxy.revocable(animated, {}); revoked.revoke();
        for (const receiver of [{}, element, Object.create(animated), author, revoked.proxy]) {
          let conversions = 0, error;
          try {base.set.call(receiver, {toString() {conversions++; throw 42;}});} catch (caught) {error = caught;}
          assert(error && Object.getPrototypeOf(error) === callee.TypeError.prototype && conversions === 0 && traps === 0, 'receiver before conversion');
          error = undefined; try {anim.get.call(receiver);} catch (caught) {error = caught;}
          assert(error && Object.getPrototypeOf(error) === callee.TypeError.prototype && traps === 0, 'getter brand');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
