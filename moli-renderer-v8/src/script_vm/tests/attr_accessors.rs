use super::*;

#[test]
fn attr_accessors_share_branded_prototypes_and_node_value_operations() {
    let mut vm = new_parsed_test_vm(
        "https://attr-accessors.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("attr_accessors.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__attrAccessorFailures)")
            .unwrap(),
        "[]"
    );
}
