use super::*;

#[test]
fn iir_filters_use_native_coefficients_webidl_and_analytic_frequency_responses() {
    let mut vm = new_storage_page_task_executor_test_vm("https://iir-filters.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__iirDone = value, error => globalThis.__iirDone = String(error));", include_str!("iir_filter_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__iirDone").unwrap(), "true");
}

#[test]
fn iir_processing_requires_a_backend_before_rendering() {
    let mut vm = new_storage_page_task_executor_test_vm("https://iir-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__iirBackendDone = value, error => globalThis.__iirBackendDone = String(error));", include_str!("iir_filter_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__iirBackendDone").unwrap(), "true");
}
