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
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
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

#[path = "svg_animated_number.rs"]
mod svg_animated_number;

#[test]
fn svg_viewport_element_uses_native_ancestors_and_receiver_brand() {
    let mut vm = new_parsed_test_vm(
        "https://svg-viewports.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(
        vm.eval(r#"(() => {
            const svgNS = 'http://www.w3.org/2000/svg';
            const outer = document.createElementNS(svgNS, 'svg');
            const symbol = outer.appendChild(document.createElementNS(svgNS, 'symbol'));
            const symbolLeaf = symbol.appendChild(document.createElementNS(svgNS, 'rect'));
            const foreign = outer.appendChild(document.createElementNS(svgNS, 'foreignObject'));
            const inner = foreign.appendChild(document.createElementNS(svgNS, 'svg'));
            const innerLeaf = inner.appendChild(document.createElementNS(svgNS, 'circle'));
            document.body.appendChild(outer);

            const child = document.querySelector('iframe').contentWindow;
            for (const realm of [window, child]) {
                const get = Object.getOwnPropertyDescriptor(realm.SVGElement.prototype, 'viewportElement').get;
                if (get.call(outer) !== null || get.call(symbolLeaf) !== symbol ||
                    get.call(inner) !== null || get.call(innerLeaf) !== inner) return false;
                let traps = 0, error;
                const proxy = new Proxy(innerLeaf, {get() { traps++; }, getPrototypeOf() { traps++; }});
                try { get.call(proxy); } catch (caught) { error = caught; }
                if (!(error instanceof realm.TypeError) || traps !== 0) return false;
            }
            return true;
        })()"#).unwrap(),
        "true",
    );
}
