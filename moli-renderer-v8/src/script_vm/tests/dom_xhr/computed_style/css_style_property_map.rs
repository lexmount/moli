use super::*;

#[test]
fn css_style_declarations_commit_before_custom_element_reactions() {
    let mut vm = new_parsed_test_vm("https://css-style-reactions.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_style_reactions.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_retain_declared_units_until_the_declaration_changes() {
    let mut vm = new_parsed_test_vm(
        "https://css-declared-units.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_declared_units.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_accept_percentage_units_without_weakening_type_checks() {
    let mut vm = new_parsed_test_vm(
        "https://css-percentages.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_percentage_writes.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_defer_percentage_ranges_until_computed_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-percentage-ranges.test/",
        "<!doctype html><body><div id=target></div>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
                const element = document.getElementById('target');
                for (const property of ['width', 'height', 'padding-left']) {
                    const input = CSS.percent(-12.5);
                    element.attributeStyleMap.set(property, input);
                    if (element.style.getPropertyValue(property) !== 'calc(-12.5%)')
                        throw new Error(property + ': range must be deferred in the declaration');
                    if (input.value !== -12.5) throw new Error('input was mutated');
                    const computed = element.computedStyleMap().get(property);
                    if (!(computed instanceof CSSUnitValue) || computed.unit !== 'percent' || computed.value !== 0)
                        throw new Error(property + ': computed percentage was not clamped: ' + computed);
                }
                return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_share_declarations_and_validate_native_types() {
    let mut vm = new_parsed_test_vm(
        "https://css-style-map.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_style_property_map.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_reject_unsupported_unparsed_storage_atomically() {
    let mut vm = new_storage_test_vm("https://css-style-map-unparsed.test/");
    assert_eq!(vm.eval(r#"(() => {
        const sheet = new CSSStyleSheet(); sheet.replaceSync('div { width: 2px; --x: old; }');
        const el = document.createElement('div'); el.style.cssText = 'width: 2px; --x: old;';
        for (const [style, map] of [[el.style, el.attributeStyleMap], [sheet.cssRules[0].style, sheet.cssRules[0].styleMap]]) {
            for (const [property, parts] of [['width', ['red']], ['width', ['10px']], ['width', []], ['--x', []]]) {
                const before = style.cssText;
                try { map.set(property, new CSSUnparsedValue(parts)); return false; }
                catch (e) { if (e.name !== 'NotSupportedError') throw e; }
                if (style.cssText !== before) return false;
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
