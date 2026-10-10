use super::*;

#[test]
fn document_import_and_adopt_use_webidl_node_and_dictionary_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://document-import-adopt.test/");
    vm.eval(include_str!("document_import_adopt_webidl.js"))
        .expect("Document import/adopt WebIDL matrix should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(__documentImportAdoptResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__documentImportAdoptResults.complete && __documentImportAdoptResults.total === 1350 && __documentImportAdoptResults.passed === 1350")
            .unwrap(),
        "true"
    );
}

#[test]
fn document_import_and_adopt_accept_registered_native_proxy_arguments_and_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://document-import-proxy.test/");
    vm.eval(
        r#"
        globalThis.__importElement = document.createElement('section');
        __importElement.appendChild(document.createTextNode('text'));
        globalThis.__importDocument = document;
        globalThis.__importRegistry = customElements;
        globalThis.__importTraps = 0;
        globalThis.__importTrap = () => { __importTraps++; throw Error('author trap'); };
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__importTrap");
        let trap = global.get(scope, key.into()).unwrap();
        for name in ["Element", "Document", "Registry"] {
            let key = crate::util::v8_string(scope, &format!("__import{name}")).unwrap();
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
            let key = crate::util::v8_string(scope, &format!("__import{name}Proxy")).unwrap();
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const clone = Document.prototype.importNode.call(__importDocumentProxy, __importElementProxy,
            { customElementRegistry: __importRegistryProxy, selfOnly: true });
        if (clone === __importElement || clone.ownerDocument !== document || clone.childNodes.length !== 0) return 'clone';
        if (Document.prototype.adoptNode.call(__importDocumentProxy, __importElementProxy) !== __importElement) return 'adopt';
        for (const [method, receiver, node] of [
            ['importNode', __importDocumentProxy, new Proxy(__importElementProxy, {})],
            ['adoptNode', __importDocumentProxy, new Proxy(__importElementProxy, {})],
            ['importNode', new Proxy(__importDocumentProxy, {}), __importElementProxy],
            ['adoptNode', new Proxy(__importDocumentProxy, {}), __importElementProxy],
        ]) {
            let reads = 0, error;
            try { Document.prototype[method].call(receiver, node, { get customElementRegistry() { reads++; return undefined; } }); }
            catch (caught) { error = caught; }
            if (!(error instanceof TypeError) || reads !== 0) return 'author proxy';
        }
        return __importTraps === 0 ? 'ok' : 'trap';
    })()"#).unwrap(), "ok");
}
