use super::*;

#[test]
fn node_normalization_updates_ranges_iterators_and_selection_across_documents() {
    let mut vm = new_parsed_test_vm(
        "https://node-normalization.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("node_normalization.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__nodeNormalizationFailures)")
            .unwrap(),
        "[]"
    );
}
