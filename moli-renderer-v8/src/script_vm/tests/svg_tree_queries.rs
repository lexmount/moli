use super::*;

#[test]
fn svg_tree_queries_use_native_descendants_and_validate_receivers_before_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-tree-queries.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_tree_queries.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1076");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_tree_queries_accept_registered_native_proxies_and_preserve_node_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-tree-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        const doc = childWindow.document.implementation.createHTMLDocument('');
        globalThis.root = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
        globalThis.leaf = doc.createElementNS('http://www.w3.org/2000/svg', 'rect');
        leaf.id = 'leaf'; root.appendChild(leaf);
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("root", "nativeRoot"), ("leaf", "nativeLeaf")] {
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
            for (const realm of [window, childWindow]) {
                const method = realm.SVGSVGElement.prototype.getElementById;
                const getter = Object.getOwnPropertyDescriptor(realm.SVGElement.prototype, 'viewportElement').get;
                if (method.call(nativeRoot, 'leaf') !== leaf || getter.call(nativeLeaf) !== root) {
                    throw Error('registered proxy lookup changed node identity');
                }
                for (const [fn, native] of [[method, nativeRoot], [getter, nativeLeaf]]) {
                    let traps = 0, conversions = 0;
                    const author = new Proxy(native, {
                        get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
                    });
                    const revoked = Proxy.revocable(native, {}); revoked.revoke();
                    for (const receiver of [author, revoked.proxy, Object.create(native)]) {
                        let error;
                        try {fn.call(receiver, {toString() {conversions++; return 'leaf';}});}
                        catch (caught) {error = caught;}
                        if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype ||
                            traps !== 0 || conversions !== 0) throw Error('invalid proxy receiver');
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
