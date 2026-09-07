use super::*;

#[test]
fn media_devices_enumeration_and_devicechange_live_on_a_branded_event_target_prototype() {
    let mut vm = new_storage_test_vm("https://media-devices.test/");
    let result = vm.eval(r#"
      (() => {
        const devices = navigator.mediaDevices;
        const prototype = MediaDevices.prototype;
        const method = Object.getOwnPropertyDescriptor(prototype, 'enumerateDevices');
        const handler = Object.getOwnPropertyDescriptor(prototype, 'ondevicechange');
        const checks = [
          devices instanceof MediaDevices, devices instanceof EventTarget,
          devices === navigator.mediaDevices,
          Object.getOwnPropertyNames(devices).length === 0,
          method.value.name === 'enumerateDevices', method.value.length === 0,
          method.enumerable, method.writable, method.configurable,
          handler.get.name === 'get ondevicechange', handler.get.length === 0,
          handler.set.name === 'set ondevicechange', handler.set.length === 1,
          handler.enumerable, handler.configurable, devices.ondevicechange === null
        ];
        const events = [];
        devices.addEventListener('devicechange', () => events.push('first'));
        devices.ondevicechange = event => events.push('old');
        devices.addEventListener('devicechange', () => events.push('last'));
        devices.ondevicechange = event => events.push(event.target === devices ? 'handler' : 'bad target');
        devices.dispatchEvent(new Event('devicechange'));
        checks.push(events.join(',') === 'first,handler,last');
        devices.ondevicechange = {};
        checks.push(devices.ondevicechange === null);
        for (const fake of [{}, prototype, Object.create(devices)]) {
          for (const callback of [() => handler.get.call(fake), () => handler.set.call(fake, null)]) {
            try { callback(); checks.push(false); } catch (error) { checks.push(error instanceof TypeError); }
          }
        }
        return checks.every(Boolean);
      })()
    "#).expect("MediaDevices prototype and event dispatch should evaluate");
    assert_eq!(result, "true");
}

#[test]
fn media_devices_enumeration_resolves_fresh_empty_lists_and_rejects_fake_receivers() {
    let mut vm = new_storage_test_vm("https://media-devices-enumeration.test/");
    vm.exec(r#"
      const devices = navigator.mediaDevices;
      const fake = Object.create(devices);
      fake.__moliMediaDevicesBrand = true;
      const calls = [devices.enumerateDevices(), devices.enumerateDevices(),
        ...[null, undefined, {}, fake].map(receiver => devices.enumerateDevices.call(receiver))];
      globalThis.__enumerationPromises = calls.every(value => value instanceof Promise);
      Promise.allSettled(calls).then(results => {
        globalThis.__enumerationResult = [
          ...results.slice(0, 2).map(result => result.status === 'fulfilled' &&
            Array.isArray(result.value) && result.value.length === 0),
          results[0].value !== results[1].value,
          ...results.slice(2).map(result => result.status === 'rejected' && result.reason instanceof TypeError)
        ].every(Boolean);
      });
    "#, None).expect("enumeration promises should be created without synchronous throws");
    assert_eq!(
        vm.eval("__enumerationPromises && __enumerationResult")
            .unwrap(),
        "true"
    );
}

#[test]
fn media_devices_enumeration_keeps_discarded_receivers_pending_even_with_borrowed_methods() {
    let mut vm = new_parsed_test_vm(
        "https://media-devices-discard.test/",
        "<!doctype html><body></body>",
    );
    vm.exec(
        r#"
      const iframe = document.createElement('iframe');
      document.body.appendChild(iframe);
      const devices = iframe.contentWindow.navigator.mediaDevices;
      const foreignEnumerate = devices.enumerateDevices;
      iframe.remove();
      globalThis.__discardedSettled = false;
      globalThis.__activeSettled = false;
      const markDiscarded = () => { __discardedSettled = true; };
      devices.enumerateDevices().then(markDiscarded, markDiscarded);
      navigator.mediaDevices.enumerateDevices.call(devices).then(markDiscarded, markDiscarded);
      foreignEnumerate.call(navigator.mediaDevices).then(result => {
        __activeSettled = Array.isArray(result) && result.length === 0;
      });
    "#,
        None,
    )
    .expect("enumeration should retain its receiver's document activity");
    assert_eq!(
        vm.eval("!__discardedSettled && __activeSettled").unwrap(),
        "true"
    );
}

#[test]
fn media_devices_is_only_exposed_in_secure_windows() {
    for (url, expected) in [
        ("https://media-devices.test/", "true|true"),
        ("http://media-devices.test/", "false|false"),
    ] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(
            vm.eval("['MediaDevices' in globalThis, 'mediaDevices' in navigator].join('|')")
                .unwrap(),
            expected
        );
    }
}
