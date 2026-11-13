use super::*;
use serde_json::json;

#[test]
fn response_projection_agrees_for_known_schema_fields() {
    for (body, expected) in [
        (
            r#"{"result":{"value":{"objectId":"user"},"type":"object","objectId":"result"},"exceptionDetails":{"exception":{"type":"object","objectId":"exception","value":{"objectId":"user-exception"}}}}"#,
            vec!["exception", "result"],
        ),
        (
            r#"{"result":[{"name":"\ud800","value":{"type":"object","objectId":"value","value":{"objectId":"user"}}},{"name":"accessor","get":{"objectId":"get"},"set":{"objectId":"set"},"symbol":{"objectId":"symbol"}}],"internalProperties":[{"value":{"objectId":"internal"},"get":{"objectId":"not-internal"}}],"privateProperties":[{"value":{"objectId":"private-value"},"get":{"objectId":"private-get"},"set":{"objectId":"private-set"},"symbol":{"objectId":"not-private"}}]}"#,
            vec![
                "get",
                "internal",
                "private-get",
                "private-set",
                "private-value",
                "set",
                "symbol",
                "value",
            ],
        ),
        (
            r#"{"object":{"objectId":"node"},"objects":{"objectId":"query"}}"#,
            vec!["node", "query"],
        ),
        (
            r#"{"result":{"objectId":"same"},"object":{"objectId":"same"},"exceptionDetails":{"exception":{"objectId":"same"}}}"#,
            vec!["same"],
        ),
        (
            r#"{"result":{"value":{"objectId":"no-type-user"}}}"#,
            vec![],
        ),
        (
            r#"{"result":{"objectId":{"objectId":"not-a-string"}},"object":{"objectId":42}}"#,
            vec![],
        ),
    ] {
        let wire = format!(r#"{{"id":1,"result":{body}}}"#);
        let raw = InspectorJson::from_slice(wire.as_bytes()).unwrap();
        let envelope = decode_lossy_json(wire.as_bytes()).unwrap();
        let parsed = InspectorJson::from(envelope.clone());
        assert_eq!(raw.response_remote_object_ids().unwrap(), expected);
        assert_eq!(parsed.response_remote_object_ids().unwrap(), expected);
        assert_eq!(response_remote_object_ids_in_value(&envelope), expected);
        assert_eq!(
            response_result_remote_object_ids(&envelope["result"]),
            expected
        );
        assert!(!raw.is_materialized());
        assert_eq!(raw.to_string(), wire);
    }
}

#[test]
fn response_projection_ignores_user_deep_and_unknown_handles() {
    let body = json!({
        "result": {
            "type":"object", "objectId":"actual",
            "value":{"objectId":"user","promiseObjectId":"user-promise","errorObjectId":"user-error"},
            "deepSerializedValue":{"type":"array","objectId":"deep-root","value":[{"objectId":"deep-child"}]},
            "futureExtension":{"objectId":"remote-extension"}
        },
        "objectId":"body-extension",
        "extension":{"objectId":"future","nested":{"errorObjectId":"future-error"}},
        "exceptionDetails":{"exception":{"objectId":"exception","deepSerializedValue":{"objectId":"deep-exception"}}}
    });
    let envelope = json!({"id":1,"result":body,"objectId":"envelope-extension"});
    let raw = InspectorJson::from_slice(envelope.to_string().as_bytes()).unwrap();
    assert_eq!(
        raw.response_remote_object_ids().unwrap(),
        ["actual", "exception"]
    );
    assert_eq!(
        response_remote_object_ids_in_value(&envelope),
        ["actual", "exception"]
    );
    assert_eq!(
        response_result_remote_object_ids(&body),
        ["actual", "exception"]
    );
    assert!(!raw.is_materialized());
    assert_eq!(
        decode_lossy_json(raw.to_string().as_bytes()).unwrap(),
        envelope
    );
}

#[test]
fn response_projection_preserves_wire_escapes_and_lossy_handles() {
    let wire = br#"{"id":7,"result":{"result":{"value":{"objectId":"user","text":"\ud800","number":1e400,"quoted":"\"\\[{]}"},"t\u0079pe":"object","object\u0049d":"actual"},"exceptionDetails":{"exception":{"type":"object","objectId":"\ud800"}},"extension":{"\ud800":{"objectId":"future"}}}}"#;
    let raw = InspectorJson::from_slice(wire).unwrap();
    assert_eq!(raw.response_remote_object_ids().unwrap(), ["actual", "�"]);
    assert!(!raw.is_materialized());
    assert_eq!(raw.to_string(), std::str::from_utf8(wire).unwrap());
}

#[test]
fn response_projection_forwards_deep_opaque_payloads_without_collecting_them() {
    for depth in [0, 40, 2000] {
        let deep = format!(
            "{}{{\"objectId\":\"nested-ignored\",\"text\":\"{}\"}}{}",
            "[".repeat(depth),
            "x".repeat(1024 * 1024),
            "]".repeat(depth)
        );
        for field in ["value", "deepSerializedValue", "futureExtension"] {
            let body = format!(
                r#"{{"result":{{"objectId":"root","{field}":{deep}}},"exceptionDetails":{{"exception":{{"objectId":"exception"}}}}}}"#
            );
            let mut raw =
                InspectorJson::from_slice(format!(r#"{{"id":1,"result":{body}}}"#).as_bytes())
                    .unwrap();
            assert_eq!(
                raw.response_remote_object_ids().unwrap(),
                ["exception", "root"]
            );
            raw.set_id(9);
            raw.set_session_id(Some("SID-opaque"));
            assert!(raw.to_string().contains(&body));
            assert!(!raw.is_materialized());
        }
    }
}

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
fn repeated_id_preserves_raw_cache_and_shared_envelope() {
    for id in [0, 7, u64::MAX] {
        for materialized in [false, true] {
            for shared in [false, true] {
                let mut message = InspectorJson::from_slice(
                    format!(r#"{{"id":{id},"result":{{"value":"\ud800"}}}}"#).as_bytes(),
                )
                .unwrap();
                if materialized {
                    let _ = message.value();
                }
                let original = shared.then(|| message.clone());
                let encoded = message.to_string();

                message.set_id(id);

                assert_eq!(message.is_materialized(), materialized);
                assert_eq!(message.to_string(), encoded);
                if let Some(InspectorJson::Raw(original)) = original {
                    let InspectorJson::Raw(current) = &message else {
                        panic!("expected raw data")
                    };
                    assert!(Arc::ptr_eq(&original, current));
                }
            }
        }
    }
}

#[test]
fn repeated_session_preserves_raw_cache_and_shared_envelope() {
    for session_id in [None, Some(""), Some("SID-\"quoted")] {
        for materialized in [false, true] {
            for shared in [false, true] {
                let mut message =
                    InspectorJson::from_slice(br#"{"id":7,"result":{"value":"\ud800"}}"#).unwrap();
                message.set_session_id(session_id);
                if materialized {
                    let _ = message.value();
                }
                let original = shared.then(|| message.clone());
                let encoded = message.to_string();

                message.set_session_id(session_id);

                assert_eq!(message.is_materialized(), materialized);
                assert_eq!(message.to_string(), encoded);
                if let Some(InspectorJson::Raw(original)) = original {
                    let InspectorJson::Raw(current) = &message else {
                        panic!("expected raw data")
                    };
                    assert!(Arc::ptr_eq(&original, current));
                }
            }
        }
    }
}

#[test]
fn changed_session_invalidates_only_the_updated_raw_view() {
    let original =
        InspectorJson::from_slice(br#"{"id":7,"sessionId":"old","result":{"value":"\ud800"}}"#)
            .unwrap();
    assert_eq!(original.value()["sessionId"], "old");
    let mut updated = original.clone();

    updated.set_session_id(Some("new"));
    assert!(!updated.is_materialized());
    assert_eq!(original.value()["sessionId"], "old");
    assert_eq!(updated.value()["sessionId"], "new");

    updated.set_session_id(None);
    assert!(!updated.is_materialized());
    assert!(updated.value().get("sessionId").is_none());
    assert_eq!(original.value()["sessionId"], "old");
    assert!(updated.to_string().contains(r"\ud800"));
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

fn assert_lossy_json(raw: &str, expected: serde_json::Value) {
    let decoded = decode_lossy_json(raw.as_bytes()).expect("UTF-8 compatibility decoding");
    assert_eq!(decoded, expected, "input: {raw}");
}

#[test]
fn inspector_protocol_lone_surrogates_preserve_pairs_literals_and_structure() {
    for (escaped, expected) in [
        (r"\ud800", "�"),
        (r"\udfff", "�"),
        (r"\uD800\uD800\uDC00", "�𐀀"),
        (r"\uDC00\uD800\uDC00", "�𐀀"),
        (r"\uD83D\uDE00\ud800", "😀�"),
        (r"\ud800a\udfff", "�a�"),
        (r"\ud800\u0061", "�a"),
        (r"\u0000\ud800", "\0�"),
    ] {
        assert_lossy_json(
            &format!(r#"{{"id":7,"result":{{"value":"{escaped}"}}}}"#),
            json!({"id": 7, "result": {"value": expected}}),
        );
    }
    assert_lossy_json(
        r#"{"\ud800":["\udfff",{"literal":"\\ud800","paired":"\uD83D\uDE00"}]}"#,
        json!({"�": ["�", {"literal": r"\ud800", "paired": "😀"}]}),
    );
    assert_lossy_json(
        r#"{"value":"a\"\\\ud800\\udfff"}"#,
        json!({"value": "a\"\\�\\udfff"}),
    );
}

#[test]
fn inspector_protocol_replaces_every_isolated_surrogate_code_unit() {
    for unit in 0xd800_u16..=0xdfff {
        assert_lossy_json(
            &format!(r#"{{"value":"\u{unit:04x}"}}"#),
            json!({"value": "�"}),
        );
    }
}

#[test]
fn inspector_protocol_mixed_escapes_match_standard_utf16_lossy_decoding() {
    use std::fmt::Write as _;
    let alphabet = [
        0xd800, 0xdbff, 0xdc00, 0xdfff, 0x5c, 0x22, 0x41, 0xa, 0, 0xffff, 0xfeff,
    ];
    let mut seed = 0xc0de_2026_u64;
    for _ in 0..256 {
        let mut units = Vec::new();
        let mut raw = String::from(r#"{"value":""#);
        for _ in 0..64 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let unit = if seed & 1 == 0 {
                alphabet[(seed >> 32) as usize % alphabet.len()]
            } else {
                (seed >> 32) as u16
            };
            units.push(unit);
            write!(&mut raw, "\\u{unit:04x}").unwrap();
        }
        raw.push_str(r#"","literal":"\\ud800"}"#);
        assert_lossy_json(
            &raw,
            json!({"value": String::from_utf16_lossy(&units), "literal": r"\ud800"}),
        );
    }
}

#[test]
fn inspector_protocol_surrogate_recovery_keeps_malformed_json_invalid() {
    for raw in [
        r#"{"value":"\ud800\q"}"#,
        r#"{"value":"\ud800","other":}"#,
        r#"{"value":"\uD80X"}"#,
        r#"{"value":"\uD800\uDE0X"}"#,
        r#"{"value":"\ud800"} trailing"#,
        r#"{"value":\ud800}"#,
        "{\"value\":\"\\ud800\n\"}",
    ] {
        assert!(decode_lossy_json(raw.as_bytes()).is_err(), "{raw}");
    }
}
