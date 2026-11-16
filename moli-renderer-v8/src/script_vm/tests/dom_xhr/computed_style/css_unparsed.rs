use super::*;

#[test]
fn css_unparsed_values_preserve_indexed_receivers_brands_and_fallback_graphs() {
    let mut vm = new_parsed_test_vm(
        "https://css-unparsed.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(vm.eval(include_str!("css_unparsed.js")).unwrap(), "true");
}

#[test]
fn css_unparsed_values_use_usv_strings_and_css_identifier_serialization() {
    let mut vm = new_storage_test_vm("https://css-unparsed-strings.test/");
    assert_eq!(
        vm.eval(include_str!("css_unparsed_strings.js")).unwrap(),
        "true"
    );
}
