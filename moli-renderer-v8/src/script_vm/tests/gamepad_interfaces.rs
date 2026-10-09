use super::*;

#[test]
fn gamepad_interfaces_use_native_brands_and_shared_event_payloads() {
    let mut vm = new_storage_page_task_executor_test_vm("https://gamepad-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__gamepadDone = value, error => globalThis.__gamepadDone = String(error));", include_str!("gamepad_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__nodeReplacementResults.total").unwrap(), "28");
    assert_eq!(vm.eval("__gamepadDone").unwrap(), "true");
}

#[test]
fn gamepad_events_preserve_native_payload_identity_and_registered_proxy_brands() {
    let mut vm = new_storage_page_task_executor_test_vm("https://gamepad-payload.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        // Native identity fixtures only; these are not devices published by
        // navigator.getGamepads and contain no fabricated hardware snapshots.
        for (name, interface) in [
            ("nativeGamepad", "Gamepad"),
            ("nativeActuator", "GamepadHapticActuator"),
        ] {
            let prototype =
                crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, interface)?;
            let object = v8::Object::new(scope);
            assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
            moli_webapi_declare::initialize_web_api_object(scope, object, interface).unwrap();
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, name).into(),
                    object.into()
                ),
                Some(true)
            );
            if interface == "Gamepad" {
                let handler = crate::util::new_null_prototype_object(scope);
                let proxy = v8::Proxy::new(scope, object, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                assert!(crate::web_api_interfaces::Gamepad::is_instance(
                    scope,
                    proxy.into()
                ));
                assert_eq!(
                    global.create_data_property(
                        scope,
                        crate::util::v8str(scope, "nativeGamepadProxy").into(),
                        proxy.into()
                    ),
                    Some(true)
                );
            }
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"(async () => {
      const assert = (ok, message) => { if (!ok) throw Error(message); };
      const foreign = document.getElementById('child').contentWindow;
      const getter = Object.getOwnPropertyDescriptor(foreign.GamepadEvent.prototype, 'gamepad').get;
      for (const gamepad of [nativeGamepad, nativeGamepadProxy]) {
        let reads = 0;
        Object.defineProperty(gamepad, 'id', {configurable: true, get() { reads++; throw Error('author lookup'); }});
        const event = new foreign.GamepadEvent('gamepadconnected', {gamepad});
        assert(event.gamepad === gamepad && getter.call(event) === gamepad && reads === 0, 'native payload is retained by identity without public field lookup');
        const receiver = new EventTarget(); let observed;
        receiver.addEventListener('gamepadconnected', value => { observed = getter.call(value); });
        receiver.dispatchEvent(event); assert(observed === gamepad, 'cross-realm dispatch keeps payload');
        event.initEvent('reused', false, false); assert(getter.call(event) === gamepad, 'initEvent retains the subclass payload');
        let traps = 0; const trap = () => { traps++; throw Error('trap'); };
        for (const forged of [Object.create(gamepad), new Proxy(gamepad, {get: trap, getPrototypeOf: trap})]) {
          let error; try { new foreign.GamepadEvent('test', {gamepad: forged}); } catch (value) { error = value; }
          assert(error instanceof foreign.TypeError, 'author objects do not acquire the native payload brand');
        }
        assert(traps === 0, 'payload validation does not execute author traps');
        delete gamepad.id;
      }
      Object.setPrototypeOf(nativeGamepad, null);
      assert(new GamepadEvent('test', {gamepad: nativeGamepad}).gamepad === nativeGamepad, 'native payload brand survives prototype removal');
      nativeActuator.__moliGamepadEffects = ['dual-rumble'];
      for (const name of ['playEffect','reset']) {
        const promise = foreign.GamepadHapticActuator.prototype[name].call(nativeActuator, 'dual-rumble', {});
        assert(promise instanceof foreign.Promise, 'unsupported operation returns callee Promise');
        let error; try { await promise; } catch (value) { error = value; }
        assert(error instanceof foreign.DOMException && error.name === 'NotSupportedError', 'no fabricated haptic success');
      }
      const effects = Object.getOwnPropertyDescriptor(GamepadHapticActuator.prototype, 'effects').get;
      let error; try { effects.call(nativeActuator); } catch (value) { error = value; }
      assert(error instanceof DOMException && error.name === 'NotSupportedError', 'public shadow cannot fabricate a hardware capability list');
      globalThis.__gamepadNativeDone = true;
    })().catch(error => globalThis.__gamepadNativeDone = String(error))"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("__gamepadNativeDone")
            .unwrap(),
        "true"
    );
}
