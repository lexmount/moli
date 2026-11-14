use super::*;

#[test]
fn svg_rect_preserves_legacy_values_in_live_and_windowless_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-rect-geometry.test/");
    vm.eval(include_str!("svg_rect_geometry.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__uiEventResults.complete && __uiEventResults.total === 750 && __uiEventResults.passed === 750")
            .unwrap(),
        "true"
    );
}
