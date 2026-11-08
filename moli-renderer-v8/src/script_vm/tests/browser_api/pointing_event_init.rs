use super::*;

#[test]
fn pointing_events_convert_inherited_dictionaries_and_interface_sequences_once() {
    let mut vm = new_storage_html_test_vm("https://pointing-event-dictionaries.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/pointing-event-dictionaries.js"
        ))
        .unwrap(),
        "true"
    );
}
