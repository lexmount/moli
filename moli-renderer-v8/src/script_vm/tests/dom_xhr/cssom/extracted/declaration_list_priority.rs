use super::*;

#[test]
fn cssom_declaration_lists_preserve_importance_without_changing_imperative_writes() {
    let mut vm = new_storage_html_test_vm("https://declaration-list-priority.test/");
    let source = include_str!("declaration_list_priority.js");
    let result = vm
        .eval(&format!("JSON.stringify({source})"))
        .expect("declaration-list priority contract should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(result["checks"].as_u64().unwrap() > 300, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
