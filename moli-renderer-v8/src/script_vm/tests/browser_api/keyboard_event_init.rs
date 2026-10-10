use super::*;

#[test]
fn keyboard_event_initializers_preserve_dom_strings_and_convert_complete_dictionaries() {
    let mut vm = new_storage_html_test_vm("https://keyboard-event-init-webidl.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    let result = vm.eval(include_str!("keyboard_event_init.js")).unwrap();
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__keyboardEventResults.rows.filter(row=>!row.passed))")
            .unwrap()
    );
}
