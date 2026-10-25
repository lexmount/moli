use super::*;

#[test]
fn main_and_child_window_global_prototype_chains_are_immutable() {
    let mut vm = new_storage_test_vm("https://window-proxy-prototype.test/");
    let script = include_str!("../../../tests/fixtures/window-global-prototypes.js");
    let result = vm
        .eval(&format!("JSON.stringify({script})"))
        .expect("Window global prototype probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 186);
    assert_eq!(result["failures"], serde_json::json!([]));
}
