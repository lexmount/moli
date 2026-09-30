use super::*;

#[test]
fn css_generated_images_reify_native_lists_computed_values_and_typed_writes() {
    let mut vm = new_parsed_test_vm(
        "https://css-generated-images.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_generated_images.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_image_values_reify_native_urls_and_preserve_brands_realms_and_writes() {
    let mut vm = new_parsed_test_vm(
        "https://css-image-values.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_image_values.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_all_keywords_preserve_native_realms_and_property_independent_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-all-keywords.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_all_keywords.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_value_parse_reifies_declared_math_with_native_brands_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://css-declared-math.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_declared_math.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_math_declaration_writes_preserve_double_precision_and_mutation_snapshots() {
    let mut vm = new_parsed_test_vm(
        "https://css-math-write-precision.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_math_write_precision.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn css_declared_math_keeps_calculations_until_computed_value_resolution() {
    let mut vm = new_parsed_test_vm(
        "https://css-declared-math-ranges.test/",
        "<!doctype html><body><div id=target></div>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const check = (ok, label) => { if (!ok) throw new Error(label); };
        const scalar = (value, expected, unit) => {
            check(value instanceof CSSMathSum && value.values.length === 1, 'declared calculation wrapper');
            check(value.values[0] instanceof CSSUnitValue, 'native scalar');
            check(value.values[0].unit === unit && value.values[0].value === expected, 'scalar magnitude and unit');
        };
        for (const [property, text, expected, unit] of [
            ['width', 'calc(1in + 2px)', 98, 'px'],
            ['width', 'calc(16777217px + 2px)', 16777219, 'px'],
            ['width', 'calc(-3.14%)', -3.14, 'percent'],
            ['width', 'min(1px, 2px)', 1, 'px'],
            ['width', 'round(3.3px, 1px)', 3, 'px'],
            ['opacity', 'calc(0% + 0%)', 0, 'percent'],
            ['z-index', 'calc(3.14)', 3.14, 'number'],
        ]) scalar(CSSStyleValue.parse(property, text), expected, unit);
        const list = CSSStyleValue.parseAll('transition-duration', 'calc(0.1234567890123456s + 1s), 2s');
        check(list.length === 2, 'property controls list cardinality');
        scalar(list[0], 1.1234567890123457, 's');
        check(list[1] instanceof CSSUnitValue && list[1].value === 2, 'second item');
        const element = document.getElementById('target');
        const map = element.attributeStyleMap;
        for (const property of ['width', 'height', 'padding-left']) {
            map.set(property, CSS.percent(-12.5));
            scalar(map.get(property), -12.5, 'percent');
            const computed = element.computedStyleMap().get(property);
            check(computed instanceof CSSUnitValue && computed.value === 0 && computed.unit === 'percent', 'computed range clamp');
        }
        map.set('z-index', CSS.number(3.14));
        scalar(map.get('z-index'), 3.14, 'number');
        check(element.computedStyleMap().get('z-index').value === 3, 'computed integer rounding');
        element.style.width = 'calc(1in + 2px)';
        scalar(map.get('width'), 98, 'px');
        const computed = element.computedStyleMap().get('width');
        check(computed instanceof CSSUnitValue && computed.value === 98 && computed.unit === 'px', 'computed scalar has no sum wrapper');
        return true;
    })()"#).unwrap(), "true");
}

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
