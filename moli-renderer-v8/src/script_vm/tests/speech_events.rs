use super::*;

#[test]
fn speech_events_construct_native_payloads_and_validate_dictionaries() {
    for url in [
        "https://shell-interfaces.test/",
        "http://shell-interfaces.test/",
        "http://localhost/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        assert_eq!(
            vm.eval(include_str!("speech_events.js")).unwrap(),
            "ok",
            "{url}"
        );
    }
}
