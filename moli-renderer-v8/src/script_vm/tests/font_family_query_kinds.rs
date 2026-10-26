use super::*;

#[test]
fn font_set_queries_distinguish_generic_families_from_literal_names_in_each_realm() {
    let mut vm = new_storage_test_vm("https://font-query-family-kinds.test/");
    let bytes = include_bytes!("../../../../moli-layout/tests/fixtures/moli-ahem.ttf");
    vm.eval(&format!(
        r#"(async () => {{
            {}
            const frame = (document.body || document.documentElement || document)
                .appendChild(document.createElement('iframe'));
            const child = frame.contentWindow;
            const bytes = {};
            return {{
                main: await fontFaceSetFamilyQueriesProbe(window, document.fonts, bytes),
                child: await fontFaceSetFamilyQueriesProbe(child, child.document.fonts, bytes),
            }};
        }})().then(
            result => {{ globalThis.familyQueryResult = JSON.stringify(result); }},
            error => {{ globalThis.familyQueryResult = JSON.stringify({{error: String(error.stack || error)}}); }}
        )"#,
        include_str!("../../../tests/fixtures/fontfaceset-family-queries.js"),
        serde_json::to_string(bytes.as_slice()).unwrap(),
    ))
    .unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("familyQueryResult").unwrap()).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(
            result[realm]["rows"].as_array().unwrap().len(),
            69,
            "{result}"
        );
    }
}
