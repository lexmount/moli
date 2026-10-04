use super::*;

#[test]
fn shell_media_recorder_preserves_native_identity_and_callee_error_realms() {
    for url in [
        "https://shell-interfaces.test/",
        "http://shell-interfaces.test/",
        "http://localhost/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        assert_eq!(
            vm.eval(include_str!("media_recorder_shell.js")).unwrap(),
            "ok",
            "{url}"
        );
    }
}
