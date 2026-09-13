use super::*;

fn activation_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://user-activation.test/");
    vm.eval("if (!document.documentElement) document.appendChild(document.createElement('html')); if (!document.body) document.documentElement.appendChild(document.createElement('body'));").unwrap();
    vm
}

#[test]
fn user_activation_exposes_webidl_prototype_and_keeps_native_receiver_identity() {
    let mut vm = activation_vm();
    assert_eq!(vm.eval(r#"
      (() => {
        const check = (condition, label) => { if (!condition) throw Error(label); };
        const activation = navigator.userActivation;
        const proto = UserActivation.prototype;
        check(activation === navigator.userActivation, 'SameObject');
        check(Object.getPrototypeOf(activation) === proto, 'prototype');
        check(Object.getPrototypeOf(proto) === Object.prototype, 'base prototype');
        check(activation instanceof UserActivation, 'instance');
        check(Object.prototype.toString.call(activation) === '[object UserActivation]', 'tag');
        check(Reflect.ownKeys(activation).length === 0, 'no own properties');
        check(UserActivation.name === 'UserActivation' && UserActivation.length === 0, 'constructor metadata');
        for (const construct of [() => new UserActivation(), () => UserActivation()]) {
          let error; try { construct(); } catch (caught) { error = caught; }
          check(error instanceof TypeError, 'illegal constructor');
        }
        for (const name of ['hasBeenActive', 'isActive']) {
          const descriptor = Object.getOwnPropertyDescriptor(proto, name);
          check(descriptor.enumerable && descriptor.configurable && !descriptor.set, 'readonly accessor');
          check(descriptor.get.name === 'get ' + name && descriptor.get.length === 0, 'getter metadata');
          check(descriptor.get.call(activation) === false, 'initial state');
          for (const receiver of [null, undefined, {}, proto, Object.create(activation), new Proxy(activation, {})]) {
            let error; try { descriptor.get.call(receiver); } catch (caught) { error = caught; }
            check(error instanceof TypeError, 'receiver brand');
          }
        }
        const getActive = Object.getOwnPropertyDescriptor(proto, 'isActive').get;
        globalThis.UserActivation = function Replacement() { throw Error('public constructor'); };
        Object.setPrototypeOf(activation, null);
        check(getActive.call(activation) === false, 'native state survives prototype mutation');
        let error; try { structuredClone(activation); } catch (caught) { error = caught; }
        check(error?.name === 'DataCloneError', 'not serializable');
        return 'ok';
      })()
    "#).unwrap(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn user_activation_retains_detached_window_state_even_when_lazily_materialized() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for materialize_before_detach in [false, true] {
        let mut vm = new_storage_test_vm_with_loader("https://user-activation.test/", &loader);
        vm.eval(r#"
            if (!document.documentElement) document.appendChild(document.createElement('html'));
            if (!document.body) document.documentElement.appendChild(document.createElement('body'));
            globalThis.frame = document.createElement('iframe');
            frame.srcdoc = '<!doctype html><body>activation child';
            document.body.appendChild(frame);
        "#).unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "UserActivation frame").await;
        vm.eval("globalThis.savedNavigator = frame.contentWindow.navigator")
            .unwrap();
        if materialize_before_detach {
            assert_eq!(
                vm.eval("savedNavigator.userActivation.isActive").unwrap(),
                "false"
            );
        }
        vm.dispatch_key_event("keydown", "x", "KeyX", "x", 0, false, false)
            .unwrap();
        vm.eval("frame.remove()").unwrap();
        assert_eq!(vm.eval(r#"
          JSON.stringify([
            savedNavigator.userActivation.isActive,
            savedNavigator.userActivation.hasBeenActive,
            Object.getOwnPropertyDescriptor(UserActivation.prototype, 'isActive').get.call(savedNavigator.userActivation),
            savedNavigator.userActivation === savedNavigator.userActivation,
            frame.contentWindow === null
          ])
        "#).unwrap(), "[true,true,true,true,true]");
        vm.eval("document.body.appendChild(frame)").unwrap();
        run_child_navigation_commit_and_host_load_for_test(
            &mut vm,
            "replacement UserActivation frame",
        )
        .await;
        assert_eq!(
            vm.eval(
                r#"
          JSON.stringify([
            frame.contentWindow.navigator.userActivation.isActive,
            frame.contentWindow.navigator.userActivation.hasBeenActive,
            savedNavigator.userActivation.hasBeenActive,
            frame.contentWindow.navigator.userActivation !== savedNavigator.userActivation
          ])
        "#
            )
            .unwrap(),
            "[false,false,true,true]"
        );
    }
}

#[test]
fn user_activation_isolated_realm_shares_the_window_state() {
    let mut vm = activation_vm();
    let isolated = vm.create_isolated_world("activation", true).unwrap();
    vm.exec_in_execution_context(isolated, "globalThis.activation = navigator.userActivation; if (activation.isActive || activation.hasBeenActive) throw Error('initial state')").unwrap();
    vm.dispatch_key_event("keydown", "x", "KeyX", "x", 0, false, false)
        .unwrap();
    vm.exec_in_execution_context(isolated, "if (!activation.isActive || !activation.hasBeenActive || activation !== navigator.userActivation) throw Error('shared Window activation')").unwrap();
    assert_eq!(
        vm.eval("navigator.userActivation.isActive && navigator.userActivation.hasBeenActive")
            .unwrap(),
        "true"
    );
    vm.destroy_isolated_world_context(isolated);
    assert_eq!(
        vm.eval("navigator.userActivation.isActive && navigator.userActivation.hasBeenActive")
            .unwrap(),
        "true"
    );
}

#[test]
fn user_activation_popup_navigation_keeps_retained_state_separate_from_new_window() {
    let mut vm = activation_vm();
    vm.eval("globalThis.popup = open('about:blank'); globalThis.savedNavigator = popup.navigator; globalThis.savedActivation = savedNavigator.userActivation").unwrap();
    assert_eq!(vm.eval("typeof popup.UserActivation === 'function' && Object.getPrototypeOf(savedActivation) === popup.UserActivation.prototype").unwrap(), "true");
    let popup_id = vm._context_host.borrow().open_lightweight_popup_ids()[0];
    vm._context_host
        .borrow_mut()
        .notify_close_watcher_user_activation(
            crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id),
        );
    assert_eq!(vm.eval("JSON.stringify([navigator.userActivation.isActive, savedActivation.isActive, savedActivation.hasBeenActive])").unwrap(), "[false,true,true]");
    vm.eval("popup.location.href = 'about:blank?replacement'")
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
      JSON.stringify([
        popup.navigator !== savedNavigator,
        popup.navigator.userActivation.isActive,
        popup.navigator.userActivation.hasBeenActive,
        savedActivation.isActive,
        savedNavigator.userActivation === savedActivation
      ])
    "#
        )
        .unwrap(),
        "[true,false,false,true,true]"
    );
    vm.eval("popup.close()").unwrap();
    assert_eq!(
        vm.eval("savedActivation.isActive && savedActivation.hasBeenActive")
            .unwrap(),
        "true"
    );
}
