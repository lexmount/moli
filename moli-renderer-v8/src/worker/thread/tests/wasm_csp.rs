use super::*;

fn wasm_csp_options() -> WorkerSpawnOptions {
    WorkerSpawnOptions::new(
        format!(
            "{}.then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/worker-wasm-csp.js"),
        ),
        "https://app.test/worker/main.js".to_owned(),
    )
}

async fn wasm_csp_result(options: WorkerSpawnOptions) -> serde_json::Value {
    let mut handle = spawn_test_worker_with_options(options);
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    serde_json::from_str(&expect_post_json(message)).unwrap()
}

fn assert_wasm_compilation_result(result: &serde_json::Value, allowed: bool) {
    let status = if allowed { "allowed" } else { "CompileError" };
    assert_eq!(
        result["results"],
        serde_json::json!([
            ["Module", status],
            ["compile", status],
            ["instantiate", status],
            ["compileStreaming", status],
            ["instantiateStreaming", status],
        ])
    );
    assert_eq!(result["synchronousEvents"], 0);
    assert_eq!(result["valid"], true, "validation does not compile code");
    for event in result["events"].as_array().unwrap() {
        assert_eq!(event["blocked"], "wasm-eval");
        assert_eq!(event["directive"], "script-src");
        assert_eq!(event["document"], "https://app.test/worker/main.js");
        assert_eq!(event["sample"], "");
        assert_eq!(event["native"], true);
        assert_eq!(event["afterMicrotask"], true);
    }
}

#[tokio::test]
async fn worker_wasm_csp_covers_sync_async_and_streaming_compilation() {
    ensure_v8();
    for (policy, allowed) in [
        ("", true),
        ("connect-src 'none'", true),
        ("default-src 'none'", false),
        ("script-src 'self' 'report-sample'", false),
        ("script-src 'unsafe-eval'", true),
        ("script-src 'wasm-unsafe-eval'", true),
        ("default-src 'WASM-UNSAFE-EVAL'", true),
        ("default-src 'none'; script-src 'wasm-unsafe-eval'", true),
        ("default-src 'wasm-unsafe-eval'; script-src 'none'", false),
        (
            "default-src 'none'; script-src-elem 'wasm-unsafe-eval'",
            false,
        ),
        (
            "require-trusted-types-for 'script'; script-src 'trusted-types-eval'",
            false,
        ),
        (
            "default-src 'none'; script-src \u{000b}'wasm-unsafe-eval'",
            false,
        ),
        (
            "default-src 'none'; script-src 'unsafe-eval'\u{000b}",
            false,
        ),
    ] {
        let result =
            wasm_csp_result(wasm_csp_options().with_content_security_policies(vec![policy.into()]))
                .await;
        assert_wasm_compilation_result(&result, allowed);
        let events = result["events"].as_array().unwrap();
        assert_eq!(
            events.len(),
            if allowed { 0 } else { 5 },
            "{policy}: {result}"
        );
        for event in events {
            assert_eq!(event["policy"], policy);
            assert_eq!(event["disposition"], "enforce");
        }
    }
}

#[tokio::test]
async fn worker_wasm_csp_reports_every_policy_without_enforcing_report_only() {
    ensure_v8();
    let policies = ["script-src 'none'", "default-src 'self'"]
        .map(crate::content_security_policy::ContentSecurityPolicy::from);
    for enforce in [false, true] {
        let result = wasm_csp_result(
            wasm_csp_options()
                .with_content_security_policies(if enforce { policies.to_vec() } else { vec![] })
                .with_content_security_report_only_policies(policies.to_vec()),
        )
        .await;
        assert_wasm_compilation_result(&result, !enforce);
        let events = result["events"].as_array().unwrap();
        assert_eq!(events.len(), if enforce { 20 } else { 10 });
        for policy in &policies {
            for disposition in if enforce {
                vec!["enforce", "report"]
            } else {
                vec!["report"]
            } {
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event["policy"] == policy.as_str()
                            && event["disposition"] == disposition)
                        .count(),
                    5
                );
            }
        }
    }
}
