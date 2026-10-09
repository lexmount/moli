use super::*;

#[test]
fn svg_detached_font_lengths_rebase_with_list_ownership() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-detached-lengths.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_detached_lengths.js")).unwrap();
    assert_eq!(
        vm.eval("__svgDetachedLengthResults.complete").unwrap(),
        "true"
    );
    assert_eq!(vm.eval("__svgDetachedLengthResults.total").unwrap(), "484");
    assert_eq!(
        vm.eval("JSON.stringify(__svgDetachedLengthResults.checks.filter(row => !row.passed).slice(0, 8))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__svgDetachedLengthResults.passed").unwrap(), "484");
}
