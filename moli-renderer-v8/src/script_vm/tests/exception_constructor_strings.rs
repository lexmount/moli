use super::*;

#[test]
fn exception_constructors_preserve_utf16_and_conversion_order() {
    let mut vm = new_parsed_test_vm(
        "https://exception-constructor-strings.test/",
        "<!doctype html><body></body>",
    );
    let result = vm.eval(&format!(
        r#"(() => {{
            {fixture}
            const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
            return JSON.stringify({{main:exceptionConstructorStringsProbe(window),child:exceptionConstructorStringsProbe(child)}});
        }})()"#,
        fixture = include_str!("../../../tests/fixtures/exception-constructor-strings.js"),
    )).expect("exception constructor argument checks should complete");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(
            result[realm]["rows"].as_array().unwrap().len(),
            202,
            "{result}"
        );
    }
}

#[test]
fn websocket_stream_close_info_stops_at_conversion_failures() {
    let mut vm = new_parsed_test_vm(
        "https://websocket-close-info.test/",
        "<!doctype html><body></body>",
    );
    let result = vm.eval(&format!(
        r#"(() => {{
            {fixture}
            const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
            return JSON.stringify({{main:websocketCloseInfoProbe(window),child:websocketCloseInfoProbe(child)}});
        }})()"#,
        fixture = include_str!("../../../tests/fixtures/websocket-close-info.js"),
    )).expect("WebSocketStream close dictionary checks should complete");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(
            result[realm]["rows"].as_array().unwrap().len(),
            22,
            "{result}"
        );
    }
}
