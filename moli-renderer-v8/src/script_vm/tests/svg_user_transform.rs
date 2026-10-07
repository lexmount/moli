use super::*;

#[test]
fn svg_user_transform_preserves_native_state_identity_and_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-user-transform.test/");
    vm.eval(include_str!("svg_user_transform.js"))
        .expect("SVG user transform matrix should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(__svgUserTransformResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__svgUserTransformResults.complete && __svgUserTransformResults.total === 1035 && __svgUserTransformResults.passed === 1035")
            .unwrap(),
        "true"
    );
    vm.eval(include_str!("svg_user_transform_document.js"))
        .expect("SVG document replacement and signed-zero matrix should evaluate");
    assert_eq!(
        vm.eval(
            "JSON.stringify(__svgUserTransformDocumentResults.checks.filter(row => !row.passed))"
        )
        .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__svgUserTransformDocumentResults.complete && __svgUserTransformDocumentResults.total === 36 && __svgUserTransformDocumentResults.passed === 36")
            .unwrap(),
        "true"
    );
}

#[test]
fn svg_user_transform_accepts_registered_native_proxies_without_author_traps() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-transform-proxies.test/");
    vm.eval(
        r#"
        globalThis.__transformRoot = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        globalThis.__transformPoint = __transformRoot.currentTranslate;
        globalThis.__transformTraps = 0;
        globalThis.__transformTrap = function() { __transformTraps++; throw Error('author trap'); };
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__transformTrap");
        let trap = global.get(scope, key.into()).unwrap();
        for (source, destination) in [
            ("__transformRoot", "__transformRootProxy"),
            ("__transformPoint", "__transformPointProxy"),
        ] {
            let key = crate::util::v8str(scope, source);
            let target = global.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let key = crate::util::v8str(scope, "get");
            assert_eq!(
                handler.create_data_property(scope, key.into(), trap),
                Some(true)
            );
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, destination);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const scale = Object.getOwnPropertyDescriptor(SVGSVGElement.prototype, 'currentScale');
        const translate = Object.getOwnPropertyDescriptor(SVGSVGElement.prototype, 'currentTranslate');
        const x = Object.getOwnPropertyDescriptor(DOMPoint.prototype, 'x');
        scale.set.call(__transformRootProxy, 3);
        x.set.call(__transformPointProxy, 7);
        if (scale.get.call(__transformRootProxy) !== 3 || __transformRoot.currentScale !== 3 ||
            translate.get.call(__transformRootProxy) !== __transformPoint ||
            x.get.call(__transformPointProxy) !== 7 || __transformPoint.x !== 7) return 'native identity';
        for (const [fn, receiver] of [[scale.set, __transformRootProxy], [x.set, __transformPointProxy]]) {
            let conversions = 0, error;
            try { fn.call(new Proxy(receiver, {}), {valueOf() { conversions++; return 8; }}); }
            catch (caught) { error = caught; }
            if (!(error instanceof TypeError) || conversions !== 0) return 'author proxy';
        }
        return __transformTraps === 0 ? 'ok' : 'trap';
    })()"#).unwrap(), "ok");
}
