use super::*;

#[test]
fn ice_error_event_converts_payloads_and_preserves_native_event_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://ice-error-event.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!("ice_error_event.js"))
        .expect("ICE error events should retain WebIDL conversion and native Event identity");
    assert_eq!(result, "true");
}
