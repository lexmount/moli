use super::*;

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
