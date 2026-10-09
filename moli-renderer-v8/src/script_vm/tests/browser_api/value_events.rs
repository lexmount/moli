use super::*;

#[test]
fn animation_transition_and_blob_events_preserve_native_payloads_and_conversions() {
    let mut vm = new_parsed_test_vm("https://value-events.test/", "<!doctype html><body></body>");
    let result = vm
        .eval(include_str!("value_events_v2.js"))
        .expect("event payload constructors should preserve Web IDL semantics");
    assert_eq!(result, "true");
}
