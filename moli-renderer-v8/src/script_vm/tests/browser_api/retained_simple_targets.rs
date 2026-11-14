use super::*;

const LIFECYCLE_PROBE: &str = include_str!("message_port_lifecycle.js");

#[test]
fn simple_event_targets_dispatch_retained_objects_using_listener_realms() {
    let mut vm = new_parsed_test_vm(
        "https://retired-event-target.test/",
        "<!doctype html><body>",
    );
    let value = vm
        .eval(&format!(
            "{LIFECYCLE_PROBE}\nJSON.stringify(retiredTargetProbe())"
        ))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&value).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["rows"].as_array().unwrap().len(), 21, "{result}");
}
