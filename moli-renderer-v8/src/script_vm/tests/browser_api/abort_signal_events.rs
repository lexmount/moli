use super::*;

const PROBE: &str = include_str!("abort_signal_events.js");

fn run_probe(expression: &str) -> serde_json::Value {
    let mut vm = new_parsed_test_vm("https://abort-signal-events.test/", "<!doctype html><body>");
    let result = vm
        .eval(&format!("{PROBE}\nJSON.stringify({expression})"))
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[test]
fn abort_signal_shares_event_target_listeners_across_realms() {
    let result = run_probe(
        r#"(() => {
        const frame = document.body.appendChild(document.createElement('iframe'));
        const realm = frame.contentWindow;
        try {
            return [abortSignalEventTargetProbe(),
                abortSignalEventTargetProbe(realm, EventTarget.prototype),
                abortSignalEventTargetProbe(window, realm.EventTarget.prototype)];
        } finally { frame.remove(); }
    })()"#,
    );
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert_eq!(row["failures"], serde_json::json!([]), "{row}");
        assert_eq!(row["scenarios"].as_array().unwrap().len(), 6);
    }
}

#[test]
fn abort_signal_generated_receiver_checks_use_callee_realm_before_conversion() {
    let result = run_probe(
        r#"(() => {
        const frame = document.body.appendChild(document.createElement('iframe'));
        try { return [abortSignalReceiverProbe(), abortSignalReceiverProbe(frame.contentWindow)]; }
        finally { frame.remove(); }
    })()"#,
    );
    for row in result.as_array().unwrap() {
        assert_eq!(row["failures"], serde_json::json!([]), "{row}");
        assert_eq!(row["checks"], 40);
    }
}

#[test]
fn abort_signal_inherited_dispatch_preserves_retired_target_validation() {
    let result = run_probe("abortSignalLifetimeProbe()");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["calls"], 0);
}
