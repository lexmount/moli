use super::*;

#[test]
fn geometry_point_dictionaries_follow_webidl_member_and_exception_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://geometry-point-order.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("geometry_point_conversion_order.js"))
        .unwrap();
    assert_eq!(
        vm.eval("__geometryPointOrderResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("__geometryPointOrderResults.total").unwrap(),
        "1344"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__geometryPointOrderResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_text_point_argument_is_optional_and_converted_after_receiver_validation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-text-point-args.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_text_point_args.js")).unwrap();
    assert_eq!(vm.eval("__svgTextPointResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__svgTextPointResults.total").unwrap(), "896");
    assert_eq!(
        vm.eval("JSON.stringify(__svgTextPointResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}
