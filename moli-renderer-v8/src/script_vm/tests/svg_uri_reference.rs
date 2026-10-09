use super::*;

#[test]
fn svg_uri_reference_getters_validate_receivers_and_reflect_native_utf16_attributes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-uri-reference.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_uri_reference.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3168");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_uri_reference_getters_share_registered_native_proxy_state_in_the_producer_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-uri-native-proxy.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      globalThis.childWindow = document.querySelector('iframe').contentWindow;
      globalThis.definitions = [
        ['a','SVGAElement'], ['image','SVGImageElement'], ['use','SVGUseElement'],
        ['textPath','SVGTextPathElement'], ['pattern','SVGPatternElement'],
        ['script','SVGScriptElement'], ['linearGradient','SVGGradientElement'],
        ['radialGradient','SVGGradientElement'], ['filter','SVGFilterElement'],
        ['feImage','SVGFEImageElement'], ['mpath','SVGMPathElement'],
      ];
      const detached = childWindow.document.implementation.createHTMLDocument('');
      for (const [tag] of definitions) {
        globalThis[tag] = detached.createElementNS('http://www.w3.org/2000/svg', tag);
      }
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (tag, proxy_name) in [
            ("a", "aProxy"),
            ("image", "imageProxy"),
            ("use", "useProxy"),
            ("textPath", "textPathProxy"),
            ("pattern", "patternProxy"),
            ("script", "scriptProxy"),
            ("linearGradient", "linearGradientProxy"),
            ("radialGradient", "radialGradientProxy"),
            ("filter", "filterProxy"),
            ("feImage", "feImageProxy"),
            ("mpath", "mpathProxy"),
        ] {
            let key = crate::util::v8str(scope, tag);
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
      for (const [tag, name] of definitions) {
        const element = globalThis[tag], native = globalThis[tag + 'Proxy'];
        const getter = Object.getOwnPropertyDescriptor(window[name].prototype, 'href').get;
        const value = getter.call(native);
        if (value !== element.href || value !== getter.call(element) ||
            Object.getPrototypeOf(value) !== childWindow.SVGAnimatedString.prototype) {
          throw Error(name + ' native proxy state/realm');
        }
        let traps = 0;
        const author = new Proxy(native, {get() {traps++;throw 42;}, getPrototypeOf() {traps++;throw 42;}});
        const revoked = Proxy.revocable(native, {}); revoked.revoke();
        for (const receiver of [author, revoked.proxy, Object.create(native)]) {
          let error;
          try {getter.call(receiver);} catch (caught) {error = caught;}
          if (!(error instanceof TypeError) || traps !== 0) throw Error(name + ' author proxy accepted');
        }
        element.setAttributeNS('http://www.w3.org/1999/xlink', 'xlink:href', '#legacy');
        value.baseVal = '#written\ud800';
        if (value !== getter.call(native) || value.baseVal !== '#written\ud800' ||
            value.animVal !== '#written\ud800' || element.hasAttribute('href') ||
            element.getAttributeNS('http://www.w3.org/1999/xlink', 'href') !== '#written\ud800') {
          throw Error(name + ' native proxy UTF-16/xlink writeback');
        }
      }
      return true;
    })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn svg_script_uri_preserves_domstring_units_without_bypassing_trusted_types() {
    for mode in ["none", "enforced", "report-only"] {
        let mut vm = new_storage_test_vm("https://svg-script-uri-trusted-types.test/");
        let policies = ["require-trusted-types-for 'script'".to_owned()];
        match mode {
            "enforced" => vm.set_response_content_security_policies(&policies),
            "report-only" => vm.set_response_content_security_report_only_policies(&policies),
            _ => {}
        }
        vm.eval(&format!("globalThis.policyMode = '{mode}'"))
            .unwrap();
        assert_eq!(
            vm.eval(
                r#"(() => {
          const assert = (condition, message) => {if (!condition) throw Error(message);};
          const xlink = 'http://www.w3.org/1999/xlink';
          const doc = document.implementation.createHTMLDocument('');
          const explicit = trustedTypes.createPolicy('uri-explicit', {
            createScriptURL: value => value, createHTML: value => value,
          });
          const setters = [
            ['attribute', element => element.setAttribute('href', explicit.createScriptURL('#initial')),
              (element, value) => element.setAttribute('href', value)],
            ['xlink', element => element.setAttributeNS(xlink, 'xlink:href', explicit.createScriptURL('#initial')),
              (element, value) => element.setAttributeNS(xlink, 'xlink:href', value)],
            ['baseVal', element => element.setAttributeNS(xlink, 'xlink:href', explicit.createScriptURL('#initial')),
              (element, value) => {element.href.baseVal = value;}],
          ];
          for (const [name, seed, set] of setters) {
            const element = doc.createElementNS('http://www.w3.org/2000/svg', 'script'); seed(element);
            const animated = element.href;
            let conversions = 0, error;
            try {set(element, {toString() {conversions++;return '#raw\ud800';}});}
            catch (caught) {error = caught;}
            assert(conversions === 1, name + ' single conversion');
            if (policyMode === 'enforced') {
              assert(error instanceof TypeError && animated.baseVal === '#initial', name + ' enforcement');
              try {set(element, explicit.createHTML('#wrong'));} catch (caught) {error = caught;}
              assert(error instanceof TypeError && animated.baseVal === '#initial', name + ' wrong trusted brand');
            } else {
              assert(!error && animated.baseVal === '#raw\ud800' && animated.animVal === '#raw\ud800', name + ' raw UTF-16');
            }
            set(element, explicit.createScriptURL('#trusted'));
            assert(animated.baseVal === '#trusted', name + ' explicit TrustedScriptURL');
            const sentinel = {}; error = undefined;
            try {set(element, {toString() {throw sentinel;}});} catch (caught) {error = caught;}
            assert(error === sentinel && animated.baseVal === '#trusted', name + ' conversion exception');
          }
          const calls = [], sentinel = {};
          trustedTypes.createPolicy('default', {createScriptURL(value, type, sink) {
            calls.push([value, type, sink]);
            if (value === '#throw') throw sentinel;
            if (value === '#reject') return null;
            return value;
          }});
          for (const [name, seed, set] of setters) {
            const element = doc.createElementNS('http://www.w3.org/2000/svg', 'script'); seed(element);
            const count = calls.length;
            set(element, '#policy\ud800');
            if (policyMode === 'none') {
              assert(calls.length === count && element.href.baseVal === '#policy\ud800', name + ' unused default policy');
            } else {
              const call = calls[calls.length - 1];
              assert(calls.length === count + 1 && call[0] === '#policy\ud800' &&
                call[1] === 'TrustedScriptURL' && call[2] === 'SVGScriptElement href', name + ' lossless policy input/sink');
              assert(element.href.baseVal === '#policy\ufffd', name + ' ScriptURL policy USVString result');
              const previous = element.href.baseVal;
              let error;
              try {set(element, '#throw');} catch (caught) {error = caught;}
              assert(error === sentinel && element.href.baseVal === previous, name + ' policy exception');
              error = undefined;
              try {set(element, '#reject');} catch (caught) {error = caught;}
              assert(policyMode === 'enforced' ? error instanceof TypeError && element.href.baseVal === previous :
                !error && element.href.baseVal === '#reject', name + ' rejected policy outcome');
            }
          }
          return true;
        })()"#,
            )
            .unwrap(),
            "true",
            "{mode}"
        );
    }
}
