use super::*;

#[test]
fn speech_synthesis_events_preserve_payloads_and_webidl_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://speech-synthesis-events.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("speech_synthesis_events.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "666");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn speech_synthesis_events_accept_registered_proxies_without_author_traps() {
    let mut vm = new_storage_page_task_executor_test_vm("https://speech-synthesis-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        globalThis.utterance = new childWindow.SpeechSynthesisUtterance('native');
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "utterance");
        let utterance = global.get(scope, key.into()).unwrap();
        let utterance = v8::Local::<v8::Object>::try_from(utterance).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, utterance, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(
            crate::web_api_interfaces::SpeechSynthesisUtterance::is_instance(scope, proxy.into())
        );
        let key = crate::util::v8str(scope, "nativeUtterance");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(
        r#"
        globalThis.speechEvent = new childWindow.SpeechSynthesisEvent('speech', {
            utterance: nativeUtterance, charIndex: -1, charLength: 4294967297,
            elapsedTime: 1 / 3, name: '\ud800'
        });
        globalThis.speechError = new childWindow.SpeechSynthesisErrorEvent('speech', {
            utterance: nativeUtterance, charIndex: -1, charLength: 4294967297,
            elapsedTime: 1 / 3, name: '\ud800', error: 'not-allowed'
        });
        "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("speechEvent", "speechEventProxy"),
            ("speechError", "speechErrorProxy"),
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
            for (const realm of [window, childWindow]) {
                for (const [name, nativeEvent] of [
                    ['SpeechSynthesisEvent', speechEventProxy],
                    ['SpeechSynthesisErrorEvent', speechErrorProxy],
                ]) {
                    const expected = {
                        utterance: nativeUtterance, charIndex: 4294967295, charLength: 1,
                        elapsedTime: Math.fround(1 / 3), name: '\ud800', error: 'not-allowed'
                    };
                    let traps = 0;
                    const author = new Proxy(nativeEvent, {
                        get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
                    });
                    const revoked = Proxy.revocable(nativeEvent, {}); revoked.revoke();
                    for (const member of ['utterance', 'charIndex', 'charLength', 'elapsedTime', 'name', ...(name === 'SpeechSynthesisErrorEvent' ? ['error'] : [])]) {
                        const owner = member === 'error' ? realm.SpeechSynthesisErrorEvent : realm.SpeechSynthesisEvent;
                        const getter = Object.getOwnPropertyDescriptor(owner.prototype, member).get;
                        if (!Object.is(getter.call(nativeEvent), expected[member])) throw Error('native event payload: ' + member);
                        for (const receiver of [author, revoked.proxy, Object.create(nativeEvent)]) {
                            let error;
                            try {getter.call(receiver);} catch (caught) {error = caught;}
                            if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype || traps !== 0) throw Error('invalid receiver: ' + member);
                        }
                    }
                    const event = new realm[name]('speech', {utterance: nativeUtterance, error: 'network'});
                    if (event.utterance !== nativeUtterance || event.utterance === utterance) throw Error('native utterance identity');
                    const authorUtterance = new Proxy(nativeUtterance, {
                        get() {traps++; throw 43;}, getPrototypeOf() {traps++; throw 43;}
                    });
                    const revokedUtterance = Proxy.revocable(nativeUtterance, {}); revokedUtterance.revoke();
                    for (const value of [authorUtterance, revokedUtterance.proxy, Object.create(nativeUtterance)]) {
                        let error, errorReads = 0;
                        try {new realm[name]('speech', {utterance: value, get error() {errorReads++; return 'network';}});} catch (caught) {error = caught;}
                        if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype || traps !== 0 || errorReads !== 0) throw Error('invalid utterance');
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
