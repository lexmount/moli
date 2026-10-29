use serde_json::{Value, json};

pub(crate) fn assert_debugger_defaults(mut dispatch: impl FnMut(&str) -> Vec<Value>) {
    let request = json!({"id": -2, "method": "Debugger.enable"});
    let enabled = dispatch(&request.to_string());
    let responses = enabled
        .iter()
        .filter(|message| message.get("id").is_some())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 1, "internal response escaped: {enabled:?}");
    assert_eq!(responses[0]["id"], -2);
    assert!(responses[0]["result"]["debuggerId"].is_string());
    for request in [
        json!({"id": 1, "method": "Debugger.setPauseOnExceptions", "params": {"state": "all"}}),
        json!({"id": 2, "method": "Debugger.setBreakpointByUrl", "params": {"lineNumber": 0, "url": "default-debugger-probe.js"}}),
        json!({"id": 3, "method": "Debugger.pause"}),
    ] {
        let messages = dispatch(&request.to_string());
        assert!(
            messages.iter().any(|message| {
                message["id"] == request["id"] && message.get("result").is_some()
            }),
            "{messages:?}"
        );
    }
    let evaluated = dispatch(
        &json!({
            "id": 4,
            "method": "Runtime.evaluate",
            "params": {"expression": "(() => { debugger; try { throw new Error('caught'); } catch {} return 42; })()\n//# sourceURL=default-debugger-probe.js"}
        }).to_string(),
    );
    assert!(
        evaluated
            .iter()
            .all(|message| message["method"] != "Debugger.paused"),
        "default session paused: {evaluated:?}"
    );
    let response = evaluated
        .iter()
        .find(|message| message["id"] == 4)
        .expect("evaluation response");
    assert_eq!(response["result"]["result"]["value"], 42, "{response}");
}

fn stack_probe() -> String {
    json!({
        "id": 100,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(function recurse(n) { if (n) return recurse(n - 1); throw new Error('stack depth'); })(80)"
        }
    })
    .to_string()
}

fn assert_frontend_response(messages: &[Value], request: &Value) {
    let responses = messages
        .iter()
        .filter(|message| message.get("id").is_some())
        .collect::<Vec<_>>();
    assert_eq!(
        responses,
        vec![&json!({"id": request["id"], "result": {}})],
        "only the frontend response may escape, including when its ID is negative: {request}"
    );
}

fn assert_stack_depth(messages: &[Value], expected: usize, request: &Value) {
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(100))
        .expect("stack probe response");
    let exception = &response["result"]["exceptionDetails"];
    assert!(exception.is_object(), "probe must throw: {response}");
    let depth = exception["stackTrace"]["callFrames"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(
        depth, expected,
        "Inspector exception stack depth after {request}: {response}"
    );
}

pub(crate) fn assert_session_lifecycle(mut dispatch: impl FnMut(&str) -> Vec<Value>) {
    for request in [
        json!({"id": 9, "method": "Runtime.enable", "params": "invalid"}),
        json!({"id": 9, "method": "Runtime.setMaxCallStackSizeToCapture", "params": {"size": 50}}),
    ] {
        let messages = dispatch(&request.to_string());
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == json!(9) && message.get("error").is_some()),
            "failed requests must retain their protocol error and must not enable Runtime: {messages:?}"
        );
    }
    let probe = stack_probe();
    for (request, expected_depth) in [
        (json!({"id": -1, "method": "Runtime.enable"}), Some(10)),
        (
            json!({"id": 2, "method": "Runtime.setMaxCallStackSizeToCapture", "params": {"size": 50}}),
            Some(50),
        ),
        (json!({"id": -1, "method": "Runtime.enable"}), Some(50)),
        (
            json!({"id": 3, "method": "Runtime.setMaxCallStackSizeToCapture", "params": {"size": 0}}),
            Some(0),
        ),
        (json!({"id": -1, "method": "Runtime.enable"}), Some(0)),
        (json!({"id": 4, "method": "Runtime.disable"}), None),
        (json!({"id": -1, "method": "Runtime.enable"}), Some(10)),
    ] {
        assert_frontend_response(&dispatch(&request.to_string()), &request);
        if let Some(expected_depth) = expected_depth {
            assert_stack_depth(&dispatch(&probe), expected_depth, &request);
        }
    }
}

pub(crate) fn assert_multiple_sessions(mut dispatch: impl FnMut(&str, &str) -> Vec<Value>) {
    let probe = stack_probe();
    for (session, request, expected_depth) in [
        ("A", json!({"id": -1, "method": "Runtime.enable"}), Some(10)),
        (
            "A",
            json!({"id": 2, "method": "Runtime.setMaxCallStackSizeToCapture", "params": {"size": 50}}),
            Some(50),
        ),
        ("B", json!({"id": -1, "method": "Runtime.enable"}), Some(50)),
        ("A", json!({"id": -1, "method": "Runtime.enable"}), Some(50)),
        ("A", json!({"id": 3, "method": "Runtime.disable"}), Some(10)),
        (
            "B",
            json!({"id": 4, "method": "Runtime.setMaxCallStackSizeToCapture", "params": {"size": 0}}),
            Some(0),
        ),
        ("B", json!({"id": -1, "method": "Runtime.enable"}), Some(0)),
        ("B", json!({"id": 5, "method": "Runtime.disable"}), None),
    ] {
        assert_frontend_response(&dispatch(session, &request.to_string()), &request);
        if let Some(expected_depth) = expected_depth {
            assert_stack_depth(&dispatch(session, &probe), expected_depth, &request);
        }
    }
}
