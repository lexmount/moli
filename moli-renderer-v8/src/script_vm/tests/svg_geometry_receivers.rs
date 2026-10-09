use super::*;

#[test]
fn svg_geometry_getters_validate_native_receivers_and_reflect_windowless_values() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-geometry-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_geometry_receivers.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "7368");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_geometry_getters_accept_registered_native_proxies_in_the_producer_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-geometry-proxies.test/");
    vm.eval(
        r#"
      document.body.innerHTML = '<iframe></iframe>';
      globalThis.childWindow = document.querySelector('iframe').contentWindow;
      globalThis.definitions = [
        ['g', 'SVGGraphicsElement', ['transform','requiredExtensions','systemLanguage']],
        ['path', 'SVGGeometryElement', ['pathLength']],
        ['svg', 'SVGSVGElement', ['x','y','width','height']],
        ['rect', 'SVGRectElement', ['x','y','width','height','rx','ry']],
        ['circle', 'SVGCircleElement', ['cx','cy','r']],
        ['ellipse', 'SVGEllipseElement', ['cx','cy','rx','ry']],
        ['line', 'SVGLineElement', ['x1','y1','x2','y2']],
        ['image', 'SVGImageElement', ['x','y','width','height']],
        ['use', 'SVGUseElement', ['x','y','width','height']],
        ['foreignObject', 'SVGForeignObjectElement', ['x','y','width','height']],
      ];
      const detached = childWindow.document.implementation.createHTMLDocument('');
      for (const [tag] of definitions) {
        globalThis[tag] = detached.createElementNS('http://www.w3.org/2000/svg', tag);
      }
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (tag, proxy_name) in [
            ("g", "gProxy"),
            ("path", "pathProxy"),
            ("svg", "svgProxy"),
            ("rect", "rectProxy"),
            ("circle", "circleProxy"),
            ("ellipse", "ellipseProxy"),
            ("line", "lineProxy"),
            ("image", "imageProxy"),
            ("use", "useProxy"),
            ("foreignObject", "foreignObjectProxy"),
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
      for (const property of ['transform','requiredExtensions','systemLanguage']) {
        const getter = Object.getOwnPropertyDescriptor(SVGGraphicsElement.prototype, property).get;
        if (getter.call(rectProxy) !== rect[property]) throw Error('native graphics subclass');
      }
      const pathLength = Object.getOwnPropertyDescriptor(SVGGeometryElement.prototype, 'pathLength').get;
      if (pathLength.call(rectProxy) !== rect.pathLength) throw Error('native geometry subclass');
      const number = pathLength.call(pathProxy);
      number.baseVal = 2.5;
      if (path.getAttribute('pathLength') !== '2.5' || number !== path.pathLength) throw Error('native number writeback');
      const transform = Object.getOwnPropertyDescriptor(SVGGraphicsElement.prototype, 'transform').get;
      g.setAttribute('transform', 'translate(1 2)');
      const animated = transform.call(gProxy), base = animated.baseVal;
      const item = base.getItem(0);
      item.setTranslate(3, 4);
      const reparsed = g.cloneNode(false).transform.baseVal.getItem(0).matrix;
      if (!g.getAttribute('transform') || reparsed.a !== 1 || reparsed.b !== 0 || reparsed.c !== 0 ||
          reparsed.d !== 1 || reparsed.e !== 3 || reparsed.f !== 4 || transform.call(gProxy) !== animated ||
          animated.baseVal !== base || base.getItem(0) !== item || animated.animVal.getItem(0).matrix.f !== 4) {
        throw Error('native transform writeback or retained base item');
      }
      return true;
    })()"#,
        )
        .unwrap(),
        "true"
    );
}
