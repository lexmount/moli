use super::*;

#[test]
fn window_handler_accessors_use_the_receiver_owner_and_native_brand() {
    let mut vm = new_storage_html_test_vm("https://window-handler-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("window_receivers.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
}
