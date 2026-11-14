use super::*;
#[test]
fn media_reflection_preserves_foreign_namespace_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://media-namespace.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm
        .eval(include_str!("media_attribute_namespaces.js"))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["total"], 5760);
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
