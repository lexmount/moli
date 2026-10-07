use super::*;

#[test]
fn svg_redraw_methods_convert_arguments_without_changing_dom_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-redraw-methods.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_redraw_compat.js")).unwrap();
    assert_eq!(vm.eval("__svgRedrawResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__svgRedrawResults.total").unwrap(), "2416");
    assert_eq!(
        vm.eval("JSON.stringify(__svgRedrawResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_redraw_methods_accept_registered_proxies_and_reject_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-redraw-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        const detached = childWindow.document.implementation.createHTMLDocument('');
        globalThis.svgRoot = detached.createElementNS('http://www.w3.org/2000/svg', 'svg');
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "svgRoot");
        let root = global.get(scope, key.into()).unwrap();
        let root = v8::Local::<v8::Object>::try_from(root).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, root, handler).unwrap();
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
                for (const [name, expected] of [['suspendRedraw', 1],
                        ['unsuspendRedraw', undefined], ['unsuspendRedrawAll', undefined],
                        ['forceRedraw', undefined]]) {
                    const fn = realm.SVGSVGElement.prototype[name];
                    if (fn.call(nativeProxy, 1) !== expected) throw Error(name + ' native proxy');
                    let traps = 0, conversions = 0;
                    const author = new Proxy(nativeProxy, {
                        get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 2;}
                    });
                    const revoked = Proxy.revocable(nativeProxy, {}); revoked.revoke();
                    for (const receiver of [author, revoked.proxy, Object.create(nativeProxy)]) {
                        let error;
                        try {fn.call(receiver, {valueOf() {conversions++; return 0;}});}
                        catch (caught) {error = caught;}
                        if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype ||
                            traps !== 0 || conversions !== 0) throw Error(name + ' receiver check');
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
