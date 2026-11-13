use super::*;
use moli_inspector_json::InspectorJson;

#[test]
fn routed_output_keeps_raw_responses_opaque_and_container_results_equivalent() {
    let body = json!({"result":{"type":"object","objectId":"native",
        "value":{"objectId":"user-data","large":"x".repeat(1024 * 1024)},
        "deepSerializedValue":{"type":"array","value":[{"objectId":"deep"}]}},
        "exceptionDetails":{"exception":{"type":"object","objectId":"exception"}}});
    let envelope = json!({"id":45,"result":body});
    for container in 0..3 {
        let raw = InspectorJson::from_slice(envelope.to_string().as_bytes()).unwrap();
        let event = match container {
            0 => BackgroundProtocolEvent::inspector_message(raw.clone(), None),
            1 => BackgroundProtocolEvent::inspector_message(envelope.clone(), None),
            _ => BackgroundProtocolEvent::command_success(Some(45), None, body.clone()),
        };
        let output = RuntimeInspectorRoutedOutput {
            events: vec![event],
            ..Default::default()
        };
        let mut plan = CommandOutputPlan::default();
        output.push_ordered_into_plan(&mut plan, Some(45));
        let events = plan.into_background_events(Some(45), None);
        assert_eq!(events.len(), 1);
        assert!(
            !raw.is_materialized(),
            "routing must not walk either protocol handles or user data"
        );
        let actual = events
            .into_iter()
            .next()
            .unwrap()
            .into_wire_message()
            .unwrap();
        assert_eq!(actual, envelope);
    }
}

#[test]
fn routed_output_keeps_unknown_notifications_raw() {
    let notification = json!({"method":"Runtime.futureNotification","params":{
        "value":{"objectId":"user-data"},"nested":[{"promiseObjectId":"opaque"}]
    }});
    let raw = InspectorJson::from_slice(notification.to_string().as_bytes()).unwrap();
    let output = RuntimeInspectorRoutedOutput {
        events: vec![BackgroundProtocolEvent::inspector_message(
            raw.clone(),
            None,
        )],
        ..Default::default()
    };
    let mut plan = CommandOutputPlan::default();
    output.push_ordered_into_plan(&mut plan, None);
    let events = plan.into_background_events(None, None);
    assert!(!raw.is_materialized());
    assert_eq!(
        events.into_iter().next().unwrap().into_wire_message(),
        Some(notification)
    );
}
