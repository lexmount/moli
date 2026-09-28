use super::*;

#[test]
fn css_style_value_parse_validates_property_grammar_and_preserves_native_realms() {
    let mut vm = new_parsed_test_vm(
        "https://css-style-value.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_style_value_parse.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_value_parse_reifies_unparsed_shorthands_and_empty_fallbacks() {
    let mut vm = new_storage_test_vm("https://css-style-value-variables.test/");
    assert_eq!(
        vm.eval(include_str!("css_style_value_variables.js"))
            .unwrap(),
        "true"
    );
}
