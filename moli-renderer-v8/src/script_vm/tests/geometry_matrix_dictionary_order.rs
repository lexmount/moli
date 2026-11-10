use super::*;

#[test]
fn matrix_dictionaries_convert_ancestors_before_own_members() {
    let mut vm = new_storage_page_task_executor_test_vm("https://matrix-dictionary-order.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("geometry_matrix_dictionary_order.js"))
        .unwrap();
    assert_eq!(
        vm.eval("__matrixDictionaryOrderResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("__matrixDictionaryOrderResults.total").unwrap(),
        "1440"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__matrixDictionaryOrderResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}
