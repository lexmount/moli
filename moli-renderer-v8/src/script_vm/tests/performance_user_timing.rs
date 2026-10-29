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

#[test]
fn performance_measure_converts_arguments_and_members_in_webidl_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://measure-conversion.test/");
    let source = format!(
        r#"(() => {{
            const probe = {};
            const frame = document.createElement('iframe');
            document.body.appendChild(frame);
            try {{
                const realms = [globalThis, frame.contentWindow];
                const checks = [];
                for (const owner of realms) for (const callee of realms)
                    checks.push(...probe(owner, callee));
                return JSON.stringify({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            }} finally {{ frame.remove(); }}
        }})()"#,
        include_str!("../../../tests/fixtures/user-timing-measure-conversion.js"),
    );
    let result: serde_json::Value = serde_json::from_str(&vm.eval(&source).unwrap()).unwrap();
    assert_eq!(result["total"], 128);
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
}

#[tokio::test(flavor = "current_thread")]
async fn performance_names_retain_utf16_identity_across_realms_and_observers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://performance-names.test/");
    let source = format!(
        r#"(async () => {{
            const probe = {};
            const frame = document.createElement('iframe');
            document.body.appendChild(frame);
            try {{
                const checks = [];
                const realms = [globalThis, frame.contentWindow];
                for (const owner of realms) for (const callee of realms)
                    checks.push(...await probe(owner, callee));
                globalThis.__performanceNameResult = {{total: checks.length, failures: checks.filter(row => !row.passed)}};
            }} finally {{ frame.remove(); }}
        }})()"#,
        include_str!("../../../tests/fixtures/performance-name-utf16.js"),
    );
    vm.eval(&source).unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(globalThis.__performanceNameResult)")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["total"], 912);
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
}
