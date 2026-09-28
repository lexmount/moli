use super::*;

#[test]
fn css_computed_map_enumeration_uses_native_declarations_across_all_iterators() {
    let mut vm = new_parsed_test_vm(
        "https://computed-map-enumeration.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_computed_map_enumeration.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn css_computed_map_custom_names_follow_unicode_code_point_order() {
    let mut vm = new_parsed_test_vm(
        "https://computed-map-unicode.test/",
        "<!doctype html><body>",
    );
    // The specification and css-houdini-drafts#700 require U+F900 before
    // U+1F4A9. The upstream iterable WPT currently asserts the opposite,
    // UTF-16 code-unit order; do not adopt that ordering to pass the test.
    assert_eq!(
        vm.eval(
            r#"(() => {
            const el = document.createElement('div');
            el.style.cssText = '--💩: astral; --豈: bmp; --z: z; --A: A; --a: a;';
            document.body.append(el);
            const names = [...el.computedStyleMap().keys()].filter(name => name.startsWith('--'));
            el.remove();
            return JSON.stringify(names);
        })()"#
        )
        .unwrap(),
        r#"["--A","--a","--z","--豈","--💩"]"#
    );
}
