use super::*;

#[test]
fn user_timing_implicit_timestamps_share_the_performance_now_clock() {
    let mut vm = new_storage_page_task_executor_test_vm("https://user-timing-clock.test/");
    let source = format!(
        r#"(() => {{
            const probe = {};
            const privateStateProbe = {};
            const frame = document.createElement('iframe');
            document.body.appendChild(frame);
            try {{
                const checks = [globalThis, frame.contentWindow]
                    .flatMap(owner => [...probe(owner), ...privateStateProbe(owner)]);
                return JSON.stringify({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            }} finally {{ frame.remove(); }}
        }})()"#,
        include_str!("../../../tests/fixtures/user-timing-clock.js"),
        include_str!("../../../tests/fixtures/user-timing-private-state.js"),
    );
    let result: serde_json::Value = serde_json::from_str(&vm.eval(&source).unwrap()).unwrap();
    assert_eq!(result["total"], 400);
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
}
