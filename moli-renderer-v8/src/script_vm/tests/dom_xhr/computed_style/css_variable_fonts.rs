use super::*;

#[test]
fn css_variable_fonts_parse_native_declarations_in_live_and_windowless_documents() {
    let mut vm = new_parsed_test_vm("https://variable-fonts.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_variable_font_declarations.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn css_variable_fonts_resolve_cascade_and_reset_with_font_shorthand() {
    let mut vm = new_parsed_test_vm("https://variable-fonts.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_variable_font_computation.js"))
            .unwrap(),
        "true"
    );
}
