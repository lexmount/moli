use super::*;

#[test]
fn document_import_preserves_registry_scopes_when_cloning_shadow_trees() {
    let mut vm = new_storage_page_task_executor_test_vm("https://document-clone-registry.test/");
    vm.eval(include_str!("shadow_clone_registry_matrix.js"))
        .expect("Document clone registry matrix should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(__documentCloneRegistryResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__documentCloneRegistryResults.complete && __documentCloneRegistryResults.total === 3960 && __documentCloneRegistryResults.passed === 3960")
            .unwrap(),
        "true"
    );
}
