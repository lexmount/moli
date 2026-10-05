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

#[test]
fn svg_filter_property_getters_and_windowless_writeback_use_native_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-properties.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_property_receivers.js"))
        .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3078");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_property_getters_accept_registered_native_proxies() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-filter-property-proxies.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      globalThis.childWindow = document.querySelector('iframe').contentWindow;
      globalThis.definitions = [
        ['feFuncR', 'SVGComponentTransferFunctionElement', ['type']],
        ['feBlend', 'SVGFEBlendElement', ['mode']],
        ['feColorMatrix', 'SVGFEColorMatrixElement', ['type']],
        ['feComposite', 'SVGFECompositeElement', ['operator']],
        ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', ['orderX','orderY','targetX','targetY','edgeMode','preserveAlpha']],
        ['feDisplacementMap', 'SVGFEDisplacementMapElement', ['xChannelSelector','yChannelSelector']],
        ['feMorphology', 'SVGFEMorphologyElement', ['operator']],
        ['feTurbulence', 'SVGFETurbulenceElement', ['numOctaves','stitchTiles','type']],
      ];
      for (const [tag] of definitions) {
        globalThis[tag] = childWindow.document.createElementNS('http://www.w3.org/2000/svg', tag);
      }
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (tag, proxy_name) in [
            ("feFuncR", "feFuncRProxy"),
            ("feBlend", "feBlendProxy"),
            ("feColorMatrix", "feColorMatrixProxy"),
            ("feComposite", "feCompositeProxy"),
            ("feConvolveMatrix", "feConvolveMatrixProxy"),
            ("feDisplacementMap", "feDisplacementMapProxy"),
            ("feMorphology", "feMorphologyProxy"),
            ("feTurbulence", "feTurbulenceProxy"),
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
      for (const [tag, name, properties] of definitions) {
        const element = globalThis[tag], native = globalThis[tag + 'Proxy'];
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

#[test]
fn svg_filter_set_std_deviation_uses_native_receivers_and_float_arguments() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-std-deviation.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_std_deviation.js"))
        .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "804");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_set_std_deviation_updates_retained_isolated_world_numbers() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-filter-deviation-worlds.test/");
    vm.eval(
        r#"
      globalThis.filters = ['feGaussianBlur', 'feDropShadow'].map(tag => {
        const element = document.createElementNS('http://www.w3.org/2000/svg', tag);
        element.id = tag; document.body.appendChild(element);
        return {element, x: element.stdDeviationX, y: element.stdDeviationY};
      });
    "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("svg-filter-deviation", false)
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
      globalThis.filters = ['feGaussianBlur', 'feDropShadow'].map(tag => {
        const element = document.getElementById(tag);
        const x = element.stdDeviationX, y = element.stdDeviationY;
        element.setStdDeviation(2.5, 3.25);
        return {element, x, y};
      });
      filters.every(({element, x, y}) => x.baseVal === 2.5 && y.baseVal === 3.25 &&
        x === element.stdDeviationX && y === element.stdDeviationY);
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"
      filters.every(({element, x, y}) => {
        if (x.baseVal !== 2.5 || y.animVal !== 3.25) return false;
        element.setStdDeviation(4.5, 5.25);
        return x.baseVal === 4.5 && y.baseVal === 5.25;
      });
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"
      filters.every(({element, x, y}) => x.baseVal === 4.5 && y.animVal === 5.25 &&
        x === element.stdDeviationX && y === element.stdDeviationY);
    "#
        )
        .unwrap(),
        "true"
    );
}
