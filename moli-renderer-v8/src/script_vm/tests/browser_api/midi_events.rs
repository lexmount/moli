use super::*;

#[test]
fn midi_events_preserve_payloads_and_webidl_buffer_policy() {
    let mut vm = new_storage_page_task_executor_test_vm("https://midi-events.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("midi_events.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "342");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn midi_event_payloads_accept_native_ports_and_registered_event_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://midi-event-proxies.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'; globalThis.childWindow = document.querySelector('iframe').contentWindow;").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "childWindow");
        let child =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        for (interface, name, proxy_name) in [
            ("MIDIPort", "nativePort", "nativePortProxy"),
            ("MIDIInput", "nativeInput", "nativeInputProxy"),
            ("MIDIOutput", "nativeOutput", "nativeOutputProxy"),
        ] {
            let key = crate::util::v8str(scope, interface);
            let constructor =
                v8::Local::<v8::Object>::try_from(child.get(scope, key.into()).unwrap()).unwrap();
            let owner = constructor.get_creation_context(scope).unwrap();
            let port = {
                let scope = &mut v8::ContextScope::new(scope, owner);
                let port = v8::Object::new(scope);
                crate::web_api_interfaces::initialize(scope, port, interface).unwrap();
                let key = crate::util::v8str(scope, "prototype");
                let prototype = constructor.get(scope, key.into()).unwrap();
                assert_eq!(port.set_prototype(scope, prototype), Some(true));
                port
            };
            assert!(crate::web_api_interfaces::MIDIPort::is_instance(
                scope, port
            ));
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, port, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert!(crate::web_api_interfaces::MIDIPort::is_instance(
                scope,
                proxy.into()
            ));
            for (key, value) in [(name, port.into()), (proxy_name, proxy.into())] {
                let key = crate::util::v8str(scope, key);
                assert_eq!(
                    global.create_data_property(scope, key.into(), value),
                    Some(true)
                );
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for (const realm of [window, childWindow]) {
        for (const port of [nativePort, nativeInput, nativeOutput, nativePortProxy, nativeInputProxy, nativeOutputProxy]) {
          const event = new realm.MIDIConnectionEvent('test', {port});
          const getter = Object.getOwnPropertyDescriptor(realm.MIDIConnectionEvent.prototype, 'port').get;
          if (event.port !== port || getter.call(event) !== port) throw Error('port identity');
          let traps = 0;
          const author = new Proxy(port, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 43;}});
          const revoked = Proxy.revocable(port, {}); revoked.revoke();
          for (const bad of [author, revoked.proxy, Object.create(port)]) {
            let error; try {new realm.MIDIConnectionEvent('test', {port: bad});} catch (caught) {error = caught;}
            if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype || traps) throw Error('port brand');
          }
        }
      }
      globalThis.message = new childWindow.MIDIMessageEvent('test', {data: new Uint8Array([0x90,60,127])});
      globalThis.connection = new childWindow.MIDIConnectionEvent('test', {port: nativeInputProxy});
      return true;
    })()"#).unwrap(), "true");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("message", "messageProxy"),
            ("connection", "connectionProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let event =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
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
    assert_eq!(vm.eval(r#"(() => {
      for (const realm of [window, childWindow]) {
        for (const [kind, member, proxy, value] of [
          ['MIDIMessageEvent', 'data', messageProxy, message.data],
          ['MIDIConnectionEvent', 'port', connectionProxy, nativeInputProxy]
        ]) {
          const getter = Object.getOwnPropertyDescriptor(realm[kind].prototype, member).get;
          if (getter.call(proxy) !== value) throw Error('registered event proxy');
          let traps = 0;
          const author = new Proxy(proxy, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 43;}});
          const revoked = Proxy.revocable(proxy, {}); revoked.revoke();
          for (const bad of [author, revoked.proxy, Object.create(proxy)]) {
            let error; try {getter.call(bad);} catch (caught) {error = caught;}
            if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype || traps) throw Error('event receiver');
          }
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
