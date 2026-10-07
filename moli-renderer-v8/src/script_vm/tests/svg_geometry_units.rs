use super::*;

#[test]
fn svg_geometry_resolves_length_units_and_nested_viewports() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-geometry-units.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_geometry_units.js")).unwrap();
    assert_eq!(vm.eval("__svgUnitsResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__svgUnitsResults.total").unwrap(), "896");
    assert_eq!(
        vm.eval("JSON.stringify(__svgUnitsResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}
