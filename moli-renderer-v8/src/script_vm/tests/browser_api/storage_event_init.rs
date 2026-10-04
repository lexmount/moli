use super::*;

#[test]
fn storage_event_initializers_preserve_dom_strings_and_validate_storage_interfaces() {
    let mut vm = new_storage_html_test_vm("https://storage-event-init-webidl.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    assert_eq!(
        vm.eval(include_str!("storage_event_init.js")).unwrap(),
        "true"
    );
}
