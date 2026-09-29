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
