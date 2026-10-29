use super::*;

#[test]
fn window_request_like_fetch_validates_inherited_native_signal_after_init() {
    let mut vm = new_storage_html_test_vm("https://request-like-signal.test/");
    let source = format!(
        "globalThis.requestLikeSignalResult='pending'; ({}).then(value=>requestLikeSignalResult=value,error=>requestLikeSignalResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/request-like-signal.js")
    );
    vm.eval(&source).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("requestLikeSignalResult").unwrap(), "true");
}

#[test]
fn request_and_fetch_share_complete_dictionary_conversion_before_materialization() {
    let mut vm = new_storage_html_test_vm("https://request-init-dictionary.test/");
    let source = format!(
        "globalThis.requestInitResult='pending'; ({}).then(value=>requestInitResult=value,error=>requestInitResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/request-init-dictionary.js")
    );
    vm.eval(&source).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("requestInitResult").unwrap(), "true");
}
