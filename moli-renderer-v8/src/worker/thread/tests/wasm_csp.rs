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
        ("script-src \u{000b}'wasm-unsafe-eval'", false),
        ("script-src 'unsafe-eval'\u{000b}", false),
    ] {
        let result = wasm_csp_result(
            wasm_csp_options().with_content_security_policies(vec![policy.to_owned()]),
        )
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
    let policies = ["script-src 'none'", "default-src 'self'"].map(str::to_owned);
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
                        .filter(|event| event["policy"] == *policy
                            && event["disposition"] == disposition)
                        .count(),
                    5
                );
            }
        }
    }
}

#[tokio::test]
async fn worker_wasm_csp_uses_inherited_header_meta_and_report_only_policies() {
    ensure_v8();
    let result = wasm_csp_result(wasm_csp_options().with_content_security_policy_snapshot(
        crate::content_security_policy::InheritedContentSecurityPolicy {
            self_url: Some(url::Url::parse("https://creator.test/").unwrap()),
            header_policies: vec!["script-src 'wasm-unsafe-eval'".to_owned()],
            meta_policies: vec!["script-src 'none'".to_owned()],
            report_only_policies: vec!["default-src 'none'".to_owned()],
            ..Default::default()
        },
    ))
    .await;
    assert_wasm_compilation_result(&result, false);
    let events = result["events"].as_array().unwrap();
    assert_eq!(events.len(), 10);
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["policy"] == "script-src 'none'" && event["disposition"] == "enforce"
            )
            .count(),
        5
    );
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["policy"] == "default-src 'none'" && event["disposition"] == "report"
            )
            .count(),
        5
    );
}

#[tokio::test]
async fn worker_wasm_csp_allows_instantiating_a_cloned_compiled_module() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
        r#"
        onmessage = async event => {
          const reports = [];
          addEventListener('securitypolicyviolation', event => reports.push(event.blockedURI));
          const sync = new WebAssembly.Instance(event.data);
          const asyncInstance = await WebAssembly.instantiate(event.data);
          let blocked = false;
          try { new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])); }
          catch (error) { blocked = error instanceof WebAssembly.CompileError; }
          setTimeout(() => {
            postMessage([event.data instanceof WebAssembly.Module, sync instanceof WebAssembly.Instance,
              asyncInstance instanceof WebAssembly.Instance, blocked, reports]);
            close();
          }, 30);
        };
        "#.to_owned(), "test://wasm_module_csp".to_owned(),
    ).with_content_security_policies(vec!["script-src 'none'".to_owned()]));
    handle.post_message(serialize_test_post_message_value(
        "new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))",
    ));
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        "[true,true,true,true,[\"wasm-eval\"]]"
    );
}

#[tokio::test]
async fn worker_compilation_csp_shared_path_reports_each_string_violation() {
    ensure_v8();
    let policies = ["script-src 'none' 'report-sample'", "default-src 'none'"].map(str::to_owned);
    let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
        r#"
        const events = [];
        addEventListener('securitypolicyviolation', event => events.push([
          event.blockedURI, event.originalPolicy, event.disposition, event.sample, event.lineNumber > 0
        ]));
        let blocked = false;
        try { eval('42'); } catch (error) { blocked = error instanceof EvalError; }
        setTimeout(() => { postMessage({blocked, events}); close(); }, 30);
        "#.to_owned(), "https://app.test/worker.js".to_owned(),
    ).with_content_security_policies(policies.to_vec()).with_content_security_report_only_policies(policies.to_vec()));
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["blocked"], true);
    let events = result["events"].as_array().unwrap();
    assert_eq!(events.len(), 4);
    for disposition in ["report", "enforce"] {
        for (index, policy) in policies.iter().enumerate() {
            assert!(events.contains(&serde_json::json!([
                "eval",
                policy,
                disposition,
                if index == 0 { "42" } else { "" },
                true
            ])));
        }
    }
}

#[tokio::test]
async fn shared_and_service_worker_isolates_install_the_wasm_csp_callback() {
    ensure_v8();
    for kind in [
        crate::worker::WorkerGlobalKind::Shared {
            name: "wasm-csp".to_owned(),
            storage_key: moli_storage_key::MoliStorageKey::new(
                "https://app.test".to_owned(),
                "https://app.test".to_owned(),
                None,
                moli_storage_key::StoragePartitionRelation::FirstParty,
            ),
        },
        crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://app.test/").unwrap(),
        },
    ] {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                format!(
                    "console.log(JSON.stringify({}));",
                    include_str!("../../../../tests/fixtures/csp-eval-source-tokens.js")
                ),
                "https://app.test/worker.js".to_owned(),
            )
            .with_global_kind(kind.clone())
            .with_content_security_policies(vec!["script-src 'none'".to_owned()]),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let WorkerToParentMessage::Console(message) = message else {
            panic!("expected console result for {kind:?}, got {message:?}");
        };
        assert_eq!(
            message.message, "log: [\"EvalError\",\"EvalError\",\"CompileError\"]",
            "{kind:?}"
        );
    }
}
