use super::*;

#[test]
fn service_worker_interfaces_share_native_prototypes_and_receiver_policy() {
    let mut vm = new_storage_page_task_executor_test_vm("https://service-worker-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__serviceWorkerInterfaceDone = value, error => globalThis.__serviceWorkerInterfaceDone = String(error));",
        include_str!("service_worker_interfaces.js")
    )).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__serviceWorkerInterfaceDone").unwrap(), "true");
}

#[test]
fn service_worker_interface_globals_require_secure_contexts() {
    let mut vm = new_storage_page_task_executor_test_vm("http://service-worker-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__serviceWorkerInterfaceDone = value);",
        include_str!("service_worker_interfaces.js")
    ))
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__serviceWorkerInterfaceDone").unwrap(), "true");
}
