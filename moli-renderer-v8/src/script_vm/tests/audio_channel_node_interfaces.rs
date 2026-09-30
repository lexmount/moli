use super::*;

#[test]
fn audio_channel_nodes_use_native_interfaces_and_port_constraints() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-channel-nodes.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__channelNodesDone = value, error => globalThis.__channelNodesDone = String(error));", include_str!("audio_channel_node_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__channelNodesDone").unwrap(), "true");
}

#[test]
fn audio_channel_nodes_preserve_port_connections_and_reject_unsupported_processing() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-channel-node-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__channelNodesDone = value, error => globalThis.__channelNodesDone = String(error));", include_str!("audio_channel_node_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__channelNodesDone").unwrap(), "true");
}
