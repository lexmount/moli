use super::*;

#[test]
fn bounded_offline_audio_constructors_convert_overloads_and_render_size_in_each_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://offline-constructor.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__offlineConstructorDone = value, error => globalThis.__offlineConstructorDone = String(error));",
        include_str!("offline_audio_context_interfaces.js")
    ))
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__offlineConstructorDone").unwrap(), "true");
}
