use super::*;

#[test]
fn svg_geometry_methods_validate_native_receivers_before_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-geometry-methods.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_geometry_methods.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "2174");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_geometry_methods_use_registered_proxy_geometry_without_author_traps() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://svg-geometry-method-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        const detached = childWindow.document.implementation.createHTMLDocument('');
        globalThis.rect = detached.createElementNS('http://www.w3.org/2000/svg', 'rect');
        rect.setAttribute('x', '2'); rect.setAttribute('y', '3');
        rect.setAttribute('width', '10'); rect.setAttribute('height', '10');
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "rect");
        let rect = global.get(scope, key.into()).unwrap();
        let rect = v8::Local::<v8::Object>::try_from(rect).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, rect, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
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
            for (const realm of [window, childWindow]) {
                const graphics = realm.SVGGraphicsElement.prototype;
                const geometry = realm.SVGGeometryElement.prototype;
                const point = geometry.getPointAtLength.call(nativeProxy, 5);
                if (point.x !== 7 || point.y !== 3 ||
                    Object.getPrototypeOf(point) !== realm.DOMPoint.prototype ||
                    geometry.getTotalLength.call(nativeProxy) !== 40 ||
                    !geometry.isPointInFill.call(nativeProxy, {x: 5, y: 5})) {
                    throw Error('native proxy geometry or result realm');
                }
                const box = graphics.getBBox.call(nativeProxy);
                if (Object.getPrototypeOf(box) !== realm.DOMRect.prototype) throw Error('box realm');
                graphics.getCTM.call(nativeProxy);
                graphics.getScreenCTM.call(nativeProxy);
                geometry.isPointInStroke.call(nativeProxy, {x: 5, y: 5});
                let traps = 0, conversions = 0;
                const author = new Proxy(nativeProxy, {
                    get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
                });
                const revoked = Proxy.revocable(nativeProxy, {}); revoked.revoke();
                for (const receiver of [author, revoked.proxy, Object.create(nativeProxy)]) {
                    for (const method of [graphics.getBBox, graphics.getCTM, graphics.getScreenCTM,
                        geometry.isPointInFill, geometry.isPointInStroke, geometry.getTotalLength,
                        geometry.getPointAtLength]) {
                        let error;
                        const argument = {get x() {conversions++; return 0;},
                            valueOf() {conversions++; return 0;}};
                        try {method.call(receiver, argument);} catch (caught) {error = caught;}
                        if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype ||
                            traps !== 0 || conversions !== 0) throw Error('author proxy accepted');
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
fn svg_geometric_fill_uses_the_cascade_and_ignores_visual_paint() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-geometric-fill.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
            document.body.innerHTML = '<style>.rings {fill-rule: evenodd}</style>' +
              '<svg xmlns="http://www.w3.org/2000/svg"><path class="rings" fill="none" ' +
              'd="M0 0H10V10H0Z M2 2H8V8H2Z"/></svg>';
            const path = document.querySelector('path');
            const center = {x: 5, y: 5};
            if (path.isPointInFill(center) || !path.isPointInFill({x: 2, y: 5})) throw Error('CSS evenodd');
            path.style.fillRule = 'nonzero';
            if (!path.isPointInFill(center)) throw Error('inline rule overrides stylesheet');
            path.removeAttribute('class'); path.style.fillRule = '';
            path.parentNode.setAttribute('fill-rule', 'evenodd');
            if (path.isPointInFill(center)) throw Error('inherited rule');
            path.setAttribute('fill-rule', 'nonzero');
            for (const fill of ['none', 'transparent', 'red']) {
                path.setAttribute('fill', fill);
                if (!path.isPointInFill(center)) throw Error('paint affected geometry');
            }
            for (const value of [NaN, Infinity, -Infinity]) {
                if (path.isPointInFill({x: value, y: 5}) || path.isPointInFill({x: 5, y: value})) {
                    throw Error('nonfinite coordinate');
                }
            }
            return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
