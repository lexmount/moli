use super::*;

#[test]
fn value_event_dictionaries_convert_inherited_members_before_payload_state() {
    let mut vm = new_storage_html_test_vm("https://value-event-dictionary.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/value-event-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
}
