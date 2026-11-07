use super::*;

#[test]
fn audio_buffers_use_native_channel_storage_and_webidl_bindings() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-buffers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__audioBufferDone = value, error => globalThis.__audioBufferDone = String(error));", include_str!("audio_buffer_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__audioBufferDone").unwrap(), "true");
}
