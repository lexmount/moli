use super::*;

#[test]
fn native_exception_stacks_ignore_mutated_javascript_capture_helpers() {
    let mut vm = new_parsed_test_vm(
        "https://dom-exception-native-stacks.test/",
        "<!doctype html><body></body>",
    );
    let actual = vm
        .eval(&format!(
            r#"(() => {{
        {fixture}
        const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
        return JSON.stringify({{
            main: domExceptionStackBoundariesProbe(window),
            child: domExceptionStackBoundariesProbe(child),
        }});
    }})()"#,
            fixture = include_str!("../../../tests/fixtures/dom-exception-stack-boundaries.js"),
        ))
        .expect("native exception stack boundaries should evaluate");
    let actual: serde_json::Value = serde_json::from_str(&actual).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(actual[realm]["failures"], serde_json::json!([]), "{actual}");
        assert_eq!(
            actual[realm]["rows"].as_array().unwrap().len(),
            37,
            "{actual}"
        );
    }
}
