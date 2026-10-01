use super::*;

#[test]
fn legacy_event_initializers_preserve_creation_state_and_convert_before_dispatch_guard() {
    let mut vm = new_parsed_test_vm(
        "https://legacy-event-init.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm.eval(include_str!("legacy_event_init.js")).expect(
        "legacy event initializers should preserve native Event state and WebIDL conversion",
    );
    assert_eq!(result, "true");
}
