use super::*;

#[test]
fn image_size_presentation_hints_follow_cascade_and_live_attribute_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://image-presentation-hints.test/",
        "<!doctype html><title>Image presentation hints</title><body>",
    );
    let source = include_str!("../../../../../tests/fixtures/image-presentation-hints.js");
    let result = vm
        .eval(&format!("JSON.stringify({source})"))
        .expect("image presentation fixture");
    let result: serde_json::Value = serde_json::from_str(&result).expect("fixture JSON");
    assert_eq!(result["checks"], 26);
    assert_eq!(
        result["failures"],
        serde_json::json!([]),
        "HTML image dimensions must enter the style cascade, not bypass CSS"
    );
}
