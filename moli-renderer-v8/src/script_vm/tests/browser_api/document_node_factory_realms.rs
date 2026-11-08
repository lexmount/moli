use super::*;

#[test]
fn document_node_factories_use_owner_realm_intrinsics_without_compatibility_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://document-node-factory-realms.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/document-node-factory-realms.js"
        ))
        .expect("Document node factory matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "880");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}
