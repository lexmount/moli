use moli_protocol_cdp::CDP_PROTOCOL_JSON;
use serde_json::Value;

#[test]
fn embedded_protocol_preserves_the_complete_pinned_schemas() {
    let mut original: Value =
        serde_json::from_str(include_str!("../protocol/browser_protocol.json")).unwrap();
    let mut javascript: Value =
        serde_json::from_str(include_str!("../protocol/js_protocol.json")).unwrap();
    original["domains"]
        .as_array_mut()
        .unwrap()
        .append(javascript["domains"].as_array_mut().unwrap());
    let embedded: Value =
        serde_json::from_str(CDP_PROTOCOL_JSON).expect("valid embedded CDP protocol");
    assert_eq!(embedded, original);
}
