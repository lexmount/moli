use super::*;

#[test]
fn encrypted_media_events_preserve_payload_identity_and_webidl_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://encrypted-media-events.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("encrypted_media_events.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "628");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn value_event_payload_getters_accept_registered_proxies_without_author_traps() {
    let mut vm = new_storage_page_task_executor_test_vm("https://value-event-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        globalThis.buffer = new childWindow.ArrayBuffer(3);
        globalThis.dataBlob = new childWindow.Blob(['payload']);
        globalThis.encrypted = new childWindow.MediaEncryptedEvent('encrypted', {initData: buffer, initDataType: 'cenc'});
        globalThis.message = new childWindow.MediaKeyMessageEvent('message', {message: buffer, messageType: 'license-renewal'});
        globalThis.animation = new childWindow.AnimationEvent('animation', {animationName: '\ud800'});
        globalThis.blob = new childWindow.BlobEvent('blob', {data: dataBlob});
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("encrypted", "encryptedProxy"),
            ("message", "messageProxy"),
            ("animation", "animationProxy"),
            ("blob", "blobProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let event = global.get(scope, key.into()).unwrap();
            let event = v8::Local::<v8::Object>::try_from(event).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, event, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert!(crate::web_api_interfaces::Event::is_instance(
                scope,
                proxy.into()
            ));
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
            const payloads = [
                ['MediaEncryptedEvent', encryptedProxy, [['initData', buffer], ['initDataType', 'cenc']]],
                ['MediaKeyMessageEvent', messageProxy, [['message', buffer], ['messageType', 'license-renewal']]],
                ['AnimationEvent', animationProxy, [['animationName', '\ud800']]],
                ['BlobEvent', blobProxy, [['data', dataBlob]]],
            ];
            for (const realm of [window, childWindow]) {
                for (const [name, nativeProxy, members] of payloads) {
                    let traps = 0;
                    const author = new Proxy(nativeProxy, {
                        get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
                    });
                    const revoked = Proxy.revocable(nativeProxy, {}); revoked.revoke();
                    for (const [member, expected] of members) {
                        const getter = Object.getOwnPropertyDescriptor(realm[name].prototype, member).get;
                        if (getter.call(nativeProxy) !== expected) throw Error('registered proxy payload: ' + member);
                        for (const receiver of [author, revoked.proxy, Object.create(nativeProxy)]) {
                            let error;
                            try {getter.call(receiver);} catch (caught) {error = caught;}
                            if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype || traps !== 0) {
                                throw Error('author proxy accepted: ' + member);
                            }
                        }
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
