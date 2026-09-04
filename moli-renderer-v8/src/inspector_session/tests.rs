use serde_json::{Value, json};

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
