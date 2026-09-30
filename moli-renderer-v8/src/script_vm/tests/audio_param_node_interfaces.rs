use super::*;

#[test]
fn audio_param_nodes_use_native_interfaces_options_and_receiver_checks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-param-nodes.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__paramNodesDone = value, error => globalThis.__paramNodesDone = String(error));", include_str!("audio_param_node_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__paramNodesDone").unwrap(), "true");
}

#[test]
fn audio_param_nodes_reject_unsupported_processing_without_committing_render_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-param-node-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__paramNodesDone = value, error => globalThis.__paramNodesDone = String(error));", include_str!("audio_param_node_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__paramNodesDone").unwrap(), "true");
}
