use super::*;

#[test]
fn event_prototype_attributes_use_native_interface_brands_and_private_state() {
    let mut vm = new_storage_html_test_vm("https://event-prototype-webidl.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    let result = vm
        .eval(include_str!("event_prototype_attributes.js"))
        .unwrap();
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row=>!row.passed))")
            .unwrap()
    );
}
