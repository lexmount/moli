use super::*;

#[test]
fn event_and_ui_subclass_dictionaries_convert_before_initializing_state() {
    let mut vm = new_storage_html_test_vm("https://event-dictionary-conversion.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/event-dictionary-conversion.js"
        ))
        .unwrap(),
        "true"
    );
}
