use super::*;

#[test]
fn secure_device_window_handlers_share_native_registration_and_receiver_checks() {
    let mut vm = new_storage_html_test_vm("https://device-window-handlers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("device_handlers.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn insecure_device_window_handler_names_remain_inert_author_properties() {
    let mut vm = new_storage_html_test_vm("http://device-window-handlers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("insecure_device_handlers.js"))
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
}
