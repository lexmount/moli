use super::*;

#[test]
fn attr_node_methods_use_native_identity_across_documents_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://attr-node-references.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("attr_node_references.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__attrNodeReferenceFailures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn attr_node_references_preserve_removal_identity_attached_values_and_clone_realms() {
    let mut vm = new_parsed_test_vm(
        "https://attr-node-reference-regressions.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    let result = vm
        .eval(include_str!("attr_node_reference_regressions.js"))
        .unwrap();
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(globalThis.__attrReferenceRegressionFailures)")
            .unwrap()
    );
}
