use super::*;

#[test]
fn scheduled_sources_have_native_interfaces_conversion_and_receiver_validation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-source-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__audioSourceDone = value, error => globalThis.__audioSourceDone = String(error));", include_str!("audio_source_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__audioSourceDone").unwrap(), "true");
}

#[test]
fn new_sources_do_not_reuse_the_oscillator_rendering_fingerprint() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-source-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__audioSourceDone = value, error => globalThis.__audioSourceDone = String(error));", include_str!("audio_source_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__audioSourceDone").unwrap(), "true");
}
