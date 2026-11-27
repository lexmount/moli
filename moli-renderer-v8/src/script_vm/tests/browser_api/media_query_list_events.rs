use super::*;

const PROBE: &str = include_str!("media_query_list_events.js");

fn run_probe(function: &str) -> Vec<serde_json::Value> {
    [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::Lazy,
        moli_page_types::LayoutPolicy::Realtime,
    ]
    .into_iter()
    .map(|policy| {
        let mut vm = new_parsed_test_vm(
            "https://media-query-list-events.test/",
            "<!doctype html><body>",
        );
        vm.set_layout_policy(policy);
        let result = vm
            .eval(&format!("{PROBE}\nJSON.stringify(({function})())"))
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        result
    })
    .collect()
}

#[test]
fn media_query_list_event_constructor_preserves_webidl_and_receiver_semantics() {
    for result in run_probe("mediaQueryListEventConstructorProbe") {
        assert!(result["checks"].as_u64().unwrap() >= 35);
    }
}

#[test]
fn media_query_list_inherits_event_target_and_shares_ordered_listeners() {
    for result in run_probe("mediaQueryListEventTargetProbe") {
        assert_eq!(result["rows"], serde_json::json!(["main", "child"]));
    }
}

#[test]
fn media_query_list_generated_receiver_checks_precede_conversion() {
    for result in run_probe("mediaQueryListReceiverProbe") {
        assert_eq!(result["checks"], 90);
    }
}

#[test]
fn media_query_list_retained_target_dispatch_uses_callback_realm_lifetime() {
    for result in run_probe("() => retainedEventTargetLifetimeProbe(true)") {
        assert_eq!(result["calls"], 9);
        assert_eq!(result["dispatches"], 4);
        assert_eq!(result["retiredCalls"], 0);
    }
}

#[test]
fn event_target_retained_target_dispatch_uses_callback_realm_lifetime() {
    for result in run_probe("() => retainedEventTargetLifetimeProbe(false)") {
        assert_eq!(result["calls"], 9);
        assert_eq!(result["dispatches"], 4);
        assert_eq!(result["retiredCalls"], 0);
    }
}
