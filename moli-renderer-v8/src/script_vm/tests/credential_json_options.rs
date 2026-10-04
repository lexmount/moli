use super::*;

#[test]
fn credential_json_options_preserve_dictionary_conversion_and_binary_error_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://credential-json.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("credential_json_options.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
}
