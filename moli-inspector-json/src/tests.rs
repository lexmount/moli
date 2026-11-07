use super::*;
use serde_json::json;

#[test]
fn every_isolated_surrogate_survives_id_and_session_rewrites() {
    for unit in 0xd800..=0xdfff {
        let body = format!(r#"{{"result":{{"type":"string","value":"\u{unit:04x}"}}}}"#);
        let mut message =
            InspectorJson::from_slice(format!(r#"{{"id":7,"result":{body}}}"#).as_bytes()).unwrap();
        message.set_id(u64::MAX);
        message.set_session_id(Some("SID-\"quoted"));
        assert!(!message.is_materialized());
        let encoded = message.to_string();
        assert!(encoded.contains(&body));
        assert!(!message.is_materialized());
        let envelope: BTreeMap<String, Box<RawValue>> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(envelope["result"].get(), body);
        assert_eq!(message.id_u64(), Some(u64::MAX));
        assert_eq!(message.session_id(), Some("SID-\"quoted"));
        assert_eq!(message.value()["result"]["result"]["value"], "�");
        assert_eq!(
            message.to_string(),
            encoded,
            "reading a UTF-8 view must not rewrite the wire"
        );
    }
}

#[test]
fn payload_pairs_literals_keys_and_numbers_remain_opaque() {
    let body = r#"{"\ud800":"\ud83d\ude00","literal":"\\ud800","text":"a\"b\\c","id":7,"sessionId":"inner","number":1.2300e+4}"#;
    let mut message =
        InspectorJson::from_slice(format!(r#"{{"id":7,"result":{body}}}"#).as_bytes()).unwrap();
    message.set_id(100);
    message.set_session_id(Some("outer"));
    let encoded = message.to_string();
    let envelope: BTreeMap<String, Box<RawValue>> = serde_json::from_str(&encoded).unwrap();
    assert_eq!(envelope["result"].get(), body);
    assert_eq!(message.value()["result"]["�"], "😀");
    assert_eq!(message.value()["result"]["literal"], r"\ud800");
    assert_eq!(message.to_string(), encoded);
}

#[test]
fn clones_share_payload_but_envelope_updates_are_independent() {
    let original = InspectorJson::from_slice(br#"{"id":7,"result":{"text":"\ud800"}}"#).unwrap();
    let mut copy = original.clone();
    copy.set_id(9);
    copy.set_session_id(Some("SID"));
    let (InspectorJson::Raw(left), InspectorJson::Raw(right)) = (&original, &copy) else {
        panic!("expected raw data")
    };
    assert!(Arc::ptr_eq(&left.source, &right.source));
    assert_eq!(original.id_u64(), Some(7));
    assert_eq!(original.session_id(), None);
    assert!(!original.is_materialized());
    assert!(!copy.is_materialized());
    assert_eq!(copy.value()["id"], 9);
    copy.set_id(10);
    assert!(!copy.is_materialized());
    assert_eq!(copy.value()["id"], 10);
}

#[test]
fn session_removal_and_error_routing_do_not_materialize_payload() {
    let mut message = InspectorJson::from_slice(
        br#"{"id":7,"sessionId":"old","error":{"code":-32603,"message":"\udfff"}}"#,
    )
    .unwrap();
    assert!(message.has_field("error"));
    assert!(!message.has_field("result"));
    message.set_session_id(None);
    message.set_id(100);
    assert!(!message.is_materialized());
    let envelope: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(&message.to_string()).unwrap();
    assert!(!envelope.contains_key("sessionId"));
    assert_eq!(
        envelope["error"].get(),
        r#"{"code":-32603,"message":"\udfff"}"#
    );
}

#[test]
fn notifications_keep_method_metadata_and_raw_params() {
    let params = r#"{"args":[{"type":"string","value":"\ud800"}],"executionContextId":1}"#;
    let mut message = InspectorJson::from_slice(
        format!(r#"{{"method":"Runtime.consoleAPICalled","params":{params}}}"#).as_bytes(),
    )
    .unwrap();
    message.set_session_id(Some("worker"));
    assert_eq!(message.method(), Some("Runtime.consoleAPICalled"));
    assert_eq!(message.id_i64(), None);
    assert!(message.to_string().contains(params));
    assert!(!message.is_materialized());
}

#[test]
fn malformed_syntax_and_non_envelopes_are_rejected() {
    for raw in [
        r#"{"id":7,"result":"\q"}"#,
        r#"{"result":"\uD80X"}"#,
        r#"{"id":7,"result":]}"#,
        r#"{"id":7} trailing"#,
        "[]",
        "null",
    ] {
        assert!(InspectorJson::from_slice(raw.as_bytes()).is_err(), "{raw}");
    }
}

#[test]
fn value_mutation_explicitly_materializes_only_that_copy() {
    let original = InspectorJson::from_slice(br#"{"id":7,"result":{"value":"\ud800"}}"#).unwrap();
    let mut copy = original.clone();
    copy.value_mut()["result"]["value"] = json!("changed");
    assert_eq!(copy.value()["result"]["value"], "changed");
    assert!(original.to_string().contains(r"\ud800"));
    assert!(!original.is_materialized());
}

#[test]
fn structured_depth_limit_returns_an_error_without_destroying_raw_json() {
    let body = format!("{}0{}", "[".repeat(150), "]".repeat(150));
    let message =
        InspectorJson::from_slice(format!(r#"{{"id":7,"result":{body}}}"#).as_bytes()).unwrap();
    assert!(message.to_string().contains(&body));
    assert_eq!(message.value()["error"]["code"], -32603);
    assert_eq!(message.value()["id"], 7);
    assert!(message.to_string().contains(&body));
}
