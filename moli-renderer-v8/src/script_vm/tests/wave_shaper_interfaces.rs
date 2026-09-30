use super::*;

#[test]
fn wave_shapers_use_native_curves_and_webidl_bindings() {
    let mut vm = new_storage_page_task_executor_test_vm("https://wave-shapers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__waveShaperDone = value, error => globalThis.__waveShaperDone = String(error));", include_str!("wave_shaper_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__waveShaperDone").unwrap(), "true");
}

#[test]
fn wave_shaper_processing_and_dc_output_require_a_backend_before_rendering() {
    let mut vm = new_storage_page_task_executor_test_vm("https://wave-shaper-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__waveShaperBackendDone = value, error => globalThis.__waveShaperBackendDone = String(error));", include_str!("wave_shaper_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__waveShaperBackendDone").unwrap(), "true");
}
