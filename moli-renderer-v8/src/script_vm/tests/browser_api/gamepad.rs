use super::*;

#[test]
fn navigator_get_gamepads_returns_fresh_empty_sequences_and_checks_its_receiver() {
    let mut vm = new_storage_test_vm("http://gamepad.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const descriptor = Object.getOwnPropertyDescriptor(Navigator.prototype, 'getGamepads');
        const first = navigator.getGamepads();
        const second = navigator.getGamepads();
        first.push('local mutation');
        const rejects = receiver => {
          try { navigator.getGamepads.call(receiver); return false; }
          catch (error) { return error instanceof TypeError; }
        };
        return [
          descriptor.value.name === 'getGamepads', descriptor.value.length === 0,
          descriptor.writable, descriptor.enumerable, descriptor.configurable,
          !Object.hasOwn(navigator, 'getGamepads'), Array.isArray(first),
          Array.isArray(second), first !== second, second.length === 0,
          navigator.getGamepads().length === 0,
          [null, undefined, {}, Navigator.prototype, Object.create(navigator)].every(rejects)
        ].every(Boolean);
      })()
    "#,
        )
        .expect("getGamepads surface should evaluate");
    assert_eq!(result, "true");
}

#[test]
fn navigator_get_gamepads_checks_the_current_global_policy_and_activity() {
    let mut vm = new_parsed_test_vm(
        "https://gamepad-policy.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
      (() => {
        const frame = document.createElement('iframe');
        frame.allow = "gamepad 'none'";
        document.body.appendChild(frame);
        const child = frame.contentWindow;
        const childNavigator = child.navigator;
        const childMethod = childNavigator.getGamepads;
        const denied = callback => {
          try { callback(); return false; }
          catch (error) { return error.name === 'SecurityError'; }
        };
        const results = [
          navigator.getGamepads().length === 0,
          denied(() => childMethod.call(childNavigator)),
          navigator.getGamepads.call(childNavigator).length === 0,
          denied(() => childMethod.call(navigator))
        ];
        frame.remove();
        const inactive = childMethod.call(childNavigator);
        results.push(Array.isArray(inactive), inactive.length === 0);
        return results.join('|');
      })()
    "#,
        )
        .expect("gamepad policy and activity should evaluate");
    assert_eq!(result, "true|true|true|true|true|true");
}
