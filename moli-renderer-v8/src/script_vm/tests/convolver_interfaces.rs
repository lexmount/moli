use super::*;

#[test]
fn convolvers_use_native_impulse_responses_and_webidl_bindings() {
    let mut vm = new_storage_page_task_executor_test_vm("https://convolvers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__convolverDone = value, error => globalThis.__convolverDone = String(error));", include_str!("convolver_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__convolverDone").unwrap(), "true");
}

#[test]
fn convolver_processing_requires_a_backend_before_rendering() {
    let mut vm = new_storage_page_task_executor_test_vm("https://convolver-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__convolverBackendDone = value, error => globalThis.__convolverBackendDone = String(error));", include_str!("convolver_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__convolverBackendDone").unwrap(), "true");
}
