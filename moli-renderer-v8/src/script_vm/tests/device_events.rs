use super::*;

#[test]
fn device_event_payloads_preserve_webidl_conversions_native_brands_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://device-events.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("device_events.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__deviceEventFailures)").unwrap(),
        "[]"
    );
}

#[test]
fn device_event_exposure_errors_use_the_callee_realm() {
    let mut vm = new_parsed_test_vm(
        "http://device-events.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("device_event_realms.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn device_event_exposure_and_legacy_creation_use_native_realm_metadata() {
    for (url, exposed) in [
        ("https://device-events.test/", true),
        ("http://device-events.test/", false),
        ("http://localhost/", true),
    ] {
        let mut vm = new_parsed_test_vm(url, "<!doctype html><body></body>");
        let types = vm
            .eval(
                r#"['DeviceMotionEvent', 'DeviceOrientationEvent',
          'DeviceMotionEventAcceleration', 'DeviceMotionEventRotationRate']
          .map(name => typeof globalThis[name]).join(',')"#,
            )
            .unwrap();
        assert_eq!(
            types,
            if exposed {
                "function,function,function,function"
            } else {
                "undefined,undefined,undefined,undefined"
            },
            "{url}"
        );
        let result = vm.eval(r#"(() => {
          const rows = [];
          for (const name of ['Event', 'BeforeUnloadEvent', 'TextEvent', 'DeviceMotionEvent', 'DeviceOrientationEvent']) {
            Object.defineProperty(globalThis, name, {configurable: true, get() {throw new Error('public constructor');}});
            try {
              const event = document.createEvent(name);
              rows.push(Object.prototype.toString.call(event) + ':' + event.type);
            } catch(error) { rows.push(error.name); }
          }
          return JSON.stringify(rows);
        })()"#).unwrap();
        assert_eq!(
            result,
            if exposed {
                r#"["[object Event]:","[object BeforeUnloadEvent]:","[object TextEvent]:","[object DeviceMotionEvent]:","[object DeviceOrientationEvent]:"]"#
            } else {
                r#"["[object Event]:","[object BeforeUnloadEvent]:","[object TextEvent]:","NotSupportedError","NotSupportedError"]"#
            },
            "{url}"
        );
    }
}
