use super::*;

#[test]
fn native_dom_strings_preserve_utf16_across_documents_and_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://native-dom-strings.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("native_dom_strings.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__nativeDomStringFailures)")
            .unwrap(),
        "[]"
    );
}
