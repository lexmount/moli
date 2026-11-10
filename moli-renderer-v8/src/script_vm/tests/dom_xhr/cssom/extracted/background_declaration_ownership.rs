use super::*;

#[test]
fn background_declarations_keep_native_priority_and_pending_cascade_ownership() {
    let mut vm = new_parsed_test_vm(
        "https://background-ownership.test/path/",
        "<!doctype html><body>",
    );
    let source = include_str!("background_declaration_ownership.js");
    let result = vm.eval(&format!("JSON.stringify({source})")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(result["checks"].as_u64().unwrap() > 1000, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
