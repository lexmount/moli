use super::*;

#[test]
fn attribute_node_methods_share_native_dispatch_across_documents_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://attr-dispatch.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("../fixtures/attr-node-native-dispatch.js"))
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__attrDispatchFailures)").unwrap(),
        "[]"
    );
}
