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

#[test]
fn equality_keeps_surrogates_and_wire_payloads_distinct_without_a_business_view() {
    let raw = |value: &str| {
        InspectorJson::from_slice(format!(r#"{{"id":7,"result":{value}}}"#).as_bytes()).unwrap()
    };
    let high = raw(r#""\ud800""#);
    let low = raw(r#""\udfff""#);
    let replacement = raw(r#""\ufffd""#);
    let literal = raw(r#""\\ud800""#);
    assert_ne!(high, low);
    assert_ne!(high, replacement);
    assert_ne!(high, literal);
    assert_eq!(high, high.clone());
    assert_eq!(high, raw(r#""\ud800""#));
    let mut changed = high.clone();
    changed.set_id(8);
    assert_ne!(high, changed);
    changed.set_id(7);
    changed.set_session_id(Some("worker"));
    assert_ne!(high, changed);
    for message in [&high, &low, &replacement, &literal, &changed] {
        assert!(!message.is_materialized());
    }
    let parsed = json!({"id":7,"result":"�"});
    assert_ne!(high, InspectorJson::from(parsed));
    assert!(!high.is_materialized());
}

#[test]
fn equality_is_consistent_across_raw_parsed_and_depth_limited_views() {
    let raw = InspectorJson::from_slice(br#"{"id":7,"result":{"value":"ok"}}"#).unwrap();
    let parsed = json!({"id":7,"result":{"value":"ok"}});
    assert_eq!(raw, InspectorJson::from(parsed.clone()));
    assert_eq!(
        InspectorJson::from(parsed.clone()),
        InspectorJson::from(parsed)
    );
    assert!(!raw.is_materialized());
    let spaced = InspectorJson::from_slice(br#"{"id":7,"result":{ "value": "ok" }}"#).unwrap();
    assert_ne!(raw, spaced, "wire equality preserves payload spelling");
    let mut messages = (0..2).map(|number| {
        let body = format!("{}{number}{}", "[".repeat(150), "]".repeat(150));
        InspectorJson::from_slice(format!(r#"{{"id":7,"result":{body}}}"#).as_bytes()).unwrap()
    });
    let left = messages.next().unwrap();
    let right = messages.next().unwrap();
    assert_ne!(left, right);
    assert!(!left.is_materialized());
    assert!(!right.is_materialized());
}

#[test]
fn raw_construction_rejects_reserved_params_but_retains_user_payload_fields() {
    for key in [
        "__moliFetchRequestId",
        "__moliBlockedInterceptors",
        r"\u005f_moliFetchRequestId",
        r"__moliBlockedInterceptor\u0073",
    ] {
        let raw = format!(
            r#"{{"method":"Network.requestWillBeSent","params":{{"{key}":"private","value":"\ud800"}}}}"#
        );
        let error = InspectorJson::from_slice(raw.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("Parsed message"));
    }
    let raw = br#"{"method":"Runtime.consoleAPICalled","params":{"args":[{"type":"object","value":{"__moliFetchRequestId":"\ud800"}}],"text":"__moliBlockedInterceptors"}}"#;
    let message = InspectorJson::from_slice(raw).unwrap();
    assert!(
        message
            .to_string()
            .contains(r#""__moliFetchRequestId":"\ud800""#)
    );
    assert!(!message.is_materialized());
}

#[test]
fn params_keys_preserve_every_isolated_surrogate_without_a_business_view() {
    for unit in 0xd800..=0xdfff {
        let params = format!(r#"{{"\u{unit:04x}":1,"value":"\ud800"}}"#);
        let raw = format!(r#"{{"method":"Example.event","params":{params}}}"#);
        let message = InspectorJson::from_slice(raw.as_bytes()).unwrap();
        let encoded = message.to_string();
        let envelope: BTreeMap<String, &RawValue> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(envelope["params"].get(), params);
        assert!(!message.is_materialized());
    }
    for params in [
        r#"{"\ud83d\ude00":1,"\\ud800":2,"\u0000":3}"#.to_owned(),
        r#"{"\\u005f_moliFetchRequestId":1,"\ud800":{"__moliFetchRequestId":2}}"#.to_owned(),
        format!(r#"{{"\ud800":{}0{}}}"#, "[".repeat(150), "]".repeat(150)),
    ] {
        let raw = format!(r#"{{"method":"Example.event","params":{params}}}"#);
        let message = InspectorJson::from_slice(raw.as_bytes()).unwrap();
        assert!(message.to_string().contains(&params));
        assert!(!message.is_materialized());
    }
}

#[test]
fn surrogate_params_keys_cannot_bypass_reserved_field_validation() {
    for key in [
        "__moliFetchRequestId",
        "__moliBlockedInterceptors",
        r"\u005f_moliFetchRequestId",
        r"__moliBlockedInterceptor\u0073",
    ] {
        for params in [
            format!(r#"{{"\ud800":1,"{key}":"private"}}"#),
            format!(r#"{{"{key}":"private","\udfff":1}}"#),
        ] {
            let raw = format!(r#"{{"method":"Example.event","params":{params}}}"#);
            let error = InspectorJson::from_slice(raw.as_bytes()).unwrap_err();
            assert!(error.to_string().contains("Parsed message"), "{error}");
        }
    }
}
