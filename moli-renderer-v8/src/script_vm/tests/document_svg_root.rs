use super::*;

#[test]
fn document_svg_root_tracks_native_document_root_across_realms_and_mutations() {
    let mut vm = new_storage_page_task_executor_test_vm("https://document-svg-root.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("document_svg_root.js")).unwrap();
    assert_eq!(
        vm.eval("__documentSvgRootResults.complete").unwrap(),
        "true"
    );
    assert_eq!(vm.eval("__documentSvgRootResults.total").unwrap(), "488");
    assert_eq!(
        vm.eval("JSON.stringify(__documentSvgRootResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn document_svg_root_uses_registered_document_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://document-svg-root-proxy.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        globalThis.svgDocument = childWindow.document.implementation.createDocument(
            'http://www.w3.org/2000/svg', 'svg');
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "svgDocument");
        let document = global.get(scope, key.into()).unwrap();
        let document = v8::Local::<v8::Object>::try_from(document).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, document, handler).unwrap();
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
            const original = svgDocument.documentElement;
            for (const realm of [window, childWindow]) {
                const get = Object.getOwnPropertyDescriptor(realm.Document.prototype, 'rootElement').get;
                if (get.call(nativeProxy) !== original) throw Error('native root identity');
                let traps = 0;
                const author = new Proxy(nativeProxy, {
                    get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 2;}
                });
                const revoked = Proxy.revocable(nativeProxy, {}); revoked.revoke();
                for (const receiver of [author, revoked.proxy, Object.create(nativeProxy)]) {
                    let error;
                    try {get.call(receiver);} catch (caught) {error = caught;}
                    if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype ||
                        traps !== 0) throw Error('author proxy root');
                }
                original.remove();
                if (get.call(nativeProxy) !== null) throw Error('removed root');
                svgDocument.appendChild(original);
            }
            return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
