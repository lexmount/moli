use super::*;

#[test]
fn child_node_arguments_convert_before_tree_checks_or_mutation() {
    let mut vm = new_parsed_test_vm(
        "https://child-conversion.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!(
        "../fixtures/child-node-argument-conversion.js"
    ))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__childConversionFailures)")
            .unwrap(),
        "[]"
    );
}
