use super::*;

#[test]
fn websocket_receivers_match_chromium_across_realms() {
    let mut vm = new_parsed_test_vm(
        "https://websocket-receivers.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let expression = format!(
        "JSON.stringify(({}).failures)",
        include_str!("../../../../tests/fixtures/websocket-receivers.js")
    );
    assert_eq!(vm.eval(&expression).unwrap(), "[]");
}

#[test]
fn websocket_registered_native_proxy_shares_state_and_handlers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://websocket-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'; globalThis.foreign = document.querySelector('iframe').contentWindow; globalThis.socket = new foreign.WebSocket('wss://example.test/socket');").unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "socket");
        let socket =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, socket, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::WebSocket::is_instance(
            scope,
            proxy.into()
        ));
        let key = crate::util::v8str(scope, "nativeSocket");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        function check(value, message) { if (!value) throw Error(message); }
        for (const realm of [globalThis, foreign]) {
            for (const name of ['url', 'readyState', 'bufferedAmount', 'extensions', 'protocol', 'binaryType']) {
                const getter = Object.getOwnPropertyDescriptor(realm.WebSocket.prototype, name).get;
                check(getter.call(nativeSocket) === socket[name], name + '/canonical state');
            }
            check(nativeSocket.readyState === WebSocket.CONNECTING, 'read real native state');
            const setter = Object.getOwnPropertyDescriptor(realm.WebSocket.prototype, 'binaryType').set;
            setter.call(nativeSocket, 'arraybuffer');
            check(socket.binaryType === 'arraybuffer', 'native setter updates target');
            socket.binaryType = 'blob';
            check(nativeSocket.binaryType === 'blob', 'native getter observes target updates');
            for (const name of ['onopen', 'onmessage', 'onerror', 'onclose']) {
                const descriptor = Object.getOwnPropertyDescriptor(realm.WebSocket.prototype, name);
                const callback = () => {};
                descriptor.set.call(nativeSocket, callback);
                check(socket[name] === callback && descriptor.get.call(nativeSocket) === callback, name + '/canonical handler');
                descriptor.set.call(nativeSocket, null);
                check(socket[name] === null, name + '/canonical handler removal');
            }
        }
        const calls = [], listener = () => calls.push('listener');
        nativeSocket.addEventListener('message', listener);
        nativeSocket.onmessage = function(event) {
            check(this === socket && event.target === socket, 'canonical event target');
            calls.push('handler');
        };
        check(socket.onmessage === nativeSocket.onmessage, 'handler slot is shared');
        socket.addEventListener('message', () => calls.push('tail'));
        socket.dispatchEvent(new MessageEvent('message'));
        check(calls.join() === 'listener,handler,tail', 'shared listener and handler dispatch order');
        nativeSocket.onmessage = () => calls.push('replacement');
        calls.length = 0;
        socket.dispatchEvent(new MessageEvent('message'));
        check(calls.join() === 'listener,replacement,tail', 'replacement preserves order');
        nativeSocket.onmessage = null;
        nativeSocket.removeEventListener('message', listener);
        calls.length = 0;
        socket.dispatchEvent(new MessageEvent('message'));
        check(calls.join() === 'tail' && socket.onmessage === null, 'canonical removal');
        let traps = 0, conversions = 0;
        const author = new Proxy(nativeSocket, {get() {traps++; throw 42;}});
        const revoked = Proxy.revocable(nativeSocket, {}); revoked.revoke();
        const setter = Object.getOwnPropertyDescriptor(WebSocket.prototype, 'binaryType').set;
        for (const receiver of [Object.create(nativeSocket), author, revoked.proxy]) {
            let error;
            try { setter.call(receiver, {toString() {conversions++; return 'blob';}}); }
            catch (caught) { error = caught; }
            check(error instanceof TypeError && conversions === 0 && traps === 0, 'author wrappers cannot inherit native identity');
        }
        nativeSocket.close();
        check(socket.readyState !== WebSocket.CONNECTING && nativeSocket.readyState === socket.readyState, 'close updates canonical state');
        return 'ok';
    })()"#).unwrap(), "ok");
}
