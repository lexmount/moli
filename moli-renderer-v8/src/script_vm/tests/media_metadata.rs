use super::*;

#[test]
fn media_metadata_conversion_snapshots_and_native_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://media-metadata.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("media_metadata.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn media_metadata_and_session_accept_registered_native_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://media-metadata-proxy.test/");
    vm.eval("globalThis.nativeMetadata = new MediaMetadata({title: 'initial'}); globalThis.nativeSession = navigator.mediaSession;")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("nativeMetadata", "nativeMetadataProxy"),
            ("nativeSession", "nativeSessionProxy"),
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
        const title = Object.getOwnPropertyDescriptor(MediaMetadata.prototype, 'title');
        const metadata = Object.getOwnPropertyDescriptor(MediaSession.prototype, 'metadata');
        title.set.call(nativeMetadataProxy, 'changed');
        metadata.set.call(nativeSessionProxy, nativeMetadataProxy);
        if (title.get.call(nativeMetadataProxy) !== 'changed' || nativeMetadata.title !== 'changed') throw Error('native metadata proxy');
        if (metadata.get.call(nativeSessionProxy) !== nativeMetadataProxy) throw Error('native session proxy');
        let reads = 0, error;
        try { metadata.set.call(nativeSession, new Proxy(nativeMetadataProxy, {get() {reads++;}})); } catch (caught) { error = caught; }
        if (!(error instanceof TypeError) || reads !== 0 || nativeSession.metadata !== nativeMetadataProxy) throw Error('author proxy');
        return true;
    })()"#).unwrap(), "true");
}
