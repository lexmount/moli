use super::*;

const PROBE: &str = include_str!("media_query_list_events.js");

fn run_probe(function: &str) -> serde_json::Value {
    let mut vm = new_parsed_test_vm(
        "https://media-query-list-events.test/",
        "<!doctype html><body>",
    );
    let result = vm
        .eval(&format!("{PROBE}\nJSON.stringify({function}())"))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    result
}

#[test]
fn media_query_list_event_constructor_preserves_webidl_and_receiver_semantics() {
    let result = run_probe("mediaQueryListEventConstructorProbe");
    assert!(result["checks"].as_u64().unwrap() >= 35);
}

#[test]
fn media_query_list_inherits_event_target_and_shares_ordered_listeners() {
    let result = run_probe("mediaQueryListEventTargetProbe");
    assert_eq!(result["rows"], serde_json::json!(["main", "child"]));
}

#[test]
fn media_query_list_generated_receiver_checks_precede_conversion() {
    let result = run_probe("mediaQueryListReceiverProbe");
    assert_eq!(result["checks"], 90);
}

#[test]
fn media_query_list_borrowed_members_preserve_target_realm_and_lifetime() {
    let result = run_probe("mediaQueryListLifetimeProbe");
    assert_eq!(result["calls"], 0);
}
