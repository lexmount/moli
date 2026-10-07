use super::*;

#[test]
fn cache_storage_dictionary_inheritance_preserves_promise_errors_and_utf16_names() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://cache-dictionary-inheritance.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__cacheDictionaryDone = value, error => globalThis.__cacheDictionaryDone = String(error));",
        include_str!("cache_dictionary_inheritance.js"),
    )).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("__cacheDictionaryInheritanceResults.complete")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("__cacheDictionaryInheritanceResults.total")
            .unwrap(),
        "52"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify(__cacheDictionaryInheritanceResults.checks.filter(row => !row.passed))"
        )
        .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__cacheDictionaryDone").unwrap(), "true");
}
