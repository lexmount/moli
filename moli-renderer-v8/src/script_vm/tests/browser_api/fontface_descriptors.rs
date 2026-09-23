use super::*;

#[test]
fn font_face_descriptors_parse_css_and_convert_dictionaries_in_main_and_child_realms() {
    let mut vm = new_storage_test_vm("https://font-face-descriptors.test/");
    vm.eval(
        &format!(
            r#"(async () => {{
                {}
                const frame = (document.body || document.documentElement || document)
                    .appendChild(document.createElement('iframe'));
                const child = frame.contentWindow;
                const mainFace = new FontFace('Main', 'url(unused.ttf)');
                const childFace = new child.FontFace('Child', 'url(unused.ttf)');
                const mainSetter = Object.getOwnPropertyDescriptor(FontFace.prototype, 'ascentOverride').set;
                const childSetter = Object.getOwnPropertyDescriptor(child.FontFace.prototype, 'ascentOverride').set;
                childSetter.call(mainFace, '20%');
                mainSetter.call(childFace, '30%');
                let borrowedError = false;
                try {{ childSetter.call(mainFace, 'invalid'); }}
                catch (error) {{
                    borrowedError = error instanceof child.DOMException &&
                        !(error instanceof DOMException) && error.name === 'SyntaxError';
                }}
                return JSON.stringify({{
                    main: await fontFaceDescriptorsProbe(window),
                    child: await fontFaceDescriptorsProbe(child),
                    borrowedValues: [mainFace.ascentOverride, childFace.ascentOverride],
                    borrowedError,
                }});
            }})().then(result => {{ globalThis.descriptorResult = result; }})"#,
            include_str!("../../../../tests/fixtures/fontface-descriptors.js"),
        ),
    ).expect("descriptor probe should evaluate");
    let result = vm.eval("descriptorResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(result[realm]["checks"], 420, "{result}");
    }
    assert_eq!(result["borrowedValues"], serde_json::json!(["20%", "30%"]));
    assert_eq!(result["borrowedError"], true, "{result}");
}
