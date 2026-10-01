use super::*;

#[test]
fn rtc_error_and_event_preserve_native_identity_and_webidl_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://rtc-error.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!("rtc_error.js"))
        .expect("RTCError and RTCErrorEvent should use native WebIDL and Event semantics");
    assert_eq!(result, "true");
}
