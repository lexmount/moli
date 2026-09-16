use super::*;

const LIFECYCLE_PROBE: &str = include_str!("message_port_lifecycle.js");

#[test]
fn message_channel_retained_constructor_creates_detached_ports_without_endpoints() {
    let mut vm = new_parsed_test_vm(
        "https://retired-message-channel.test/",
        "<!doctype html><body>",
    );
    let registry = vm._context_host.borrow().message_port_registry();
    assert_eq!(registry.endpoint_count(), 0);
    let value = vm
        .eval(&format!(
            "{LIFECYCLE_PROBE}\nJSON.stringify(retiredChannelProbe())"
        ))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&value).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["rows"].as_array().unwrap().len(), 6, "{result}");
    assert_eq!(
        registry.endpoint_count(),
        0,
        "retired constructors must not allocate endpoints"
    );
}

#[test]
fn simple_event_targets_validate_events_before_rejecting_retired_receiver_realms() {
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

#[test]
fn message_port_transfer_disentangles_retired_owners_before_rebinding_endpoints() {
    for (mode, remaining) in [("before", 2), ("after", 0), ("getter", 0)] {
        let mut vm = new_parsed_test_vm(
            "https://retired-port-transfer.test/",
            "<!doctype html><body>",
        );
        let registry = vm._context_host.borrow().message_port_registry();
        let value = vm
            .eval(&format!(
                r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const channel = new frame.contentWindow.MessageChannel();
const ports = [channel.port1, channel.port2];
if ('{mode}' === 'after') frame.remove();
const clones = structuredClone({{get ports() {{
  if ('{mode}' === 'getter') frame.remove();
  return ports;
}}}}, {{transfer:ports}}).ports;
frame.remove();
String(clones.length === 2 && clones.every(port => port instanceof MessagePort))
"#
            ))
            .unwrap();
        assert_eq!(value, "true", "{mode}");
        assert_eq!(registry.endpoint_count(), remaining, "{mode}");
        vm.eval("for (const port of clones) port.close()").unwrap();
        assert_eq!(registry.endpoint_count(), 0, "{mode}");
    }
}

#[tokio::test]
async fn message_port_retirement_preserves_the_destination_selected_before_serialization() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://retired-port-delivery.test/",
        &loader,
    );
    vm.eval(&format!(
        r#"{LIFECYCLE_PROBE}
retiredPortTransferProbe().then(value => globalThis.retiredPortResult = value,
                              error => globalThis.retiredPortResult = String(error));
"#
    ))
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm
            .eval("globalThis.retiredPortResult !== undefined")
            .unwrap()
            != "true"
        {
            vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .unwrap();
        }
    })
    .await
    .expect("MessagePort lifecycle probe should settle");
    let value = vm.eval("JSON.stringify(retiredPortResult)").unwrap();
    let rows: serde_json::Value = serde_json::from_str(&value).unwrap();
    let rows = rows.as_array().expect("port lifecycle rows");
    assert_eq!(rows.len(), 8);
    for row in rows {
        assert_eq!(row["error"], serde_json::Value::Null, "{row}");
        let accepted = matches!(
            row["mode"].as_str().unwrap(),
            "transfer-before-removal"
                | "remove-during-post"
                | "close-during-post"
                | "remove-and-transfer-during-post"
        );
        assert_eq!(
            row["messages"],
            if accepted {
                serde_json::json!(["first"])
            } else {
                serde_json::json!([])
            },
            "{row}"
        );
    }
    assert_eq!(
        vm._context_host
            .borrow()
            .message_port_registry()
            .endpoint_count(),
        0
    );
}
