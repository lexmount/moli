use moli_protocol_cdp::CDP_PROTOCOL_JSON;
use serde_json::Value;

#[test]
fn embedded_protocol_preserves_the_complete_vendored_schema() {
    let source = include_str!("../../vendor/v8-152.2.0/v8/include/js_protocol-1.3.json");
    let original: Value = serde_json::from_str(source).expect("valid vendored CDP protocol");
    let embedded: Value =
        serde_json::from_str(CDP_PROTOCOL_JSON).expect("valid embedded CDP protocol");
    assert_eq!(embedded, original);
}
