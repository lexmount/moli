use super::*;

#[test]
fn navigation_event_dictionaries_validate_required_native_payloads_before_state() {
    let mut vm = new_storage_html_test_vm("https://navigation-event-dictionary.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/navigation-event-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
}
