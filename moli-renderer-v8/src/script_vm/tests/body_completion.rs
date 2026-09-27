use super::*;

#[tokio::test(flavor = "current_thread")]
async fn body_completion_uses_fetch_tasks_in_window() {
    run_body_completion_probe(None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn body_completion_uses_fetch_tasks_in_classic_worker() {
    run_body_completion_probe(Some("classic")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn body_completion_can_release_module_worker_top_level_await() {
    run_body_completion_probe(Some("module")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn body_completion_queues_native_network_success_and_errors() {
    let server = super::fetch_resource_timing::timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", server.origin), &loader);
    let fixture = include_str!("../../../tests/fixtures/body-completion-network.js")
        .trim()
        .trim_end_matches(';');
    vm.eval(&format!(
        "globalThis.__fetchOrigins = [{}]; globalThis.bodyNetworkResult = null; ({fixture}).then(value => bodyNetworkResult = value, error => bodyNetworkResult = JSON.stringify({{error:String(error)}}));",
        serde_json::to_string(&server.origin).unwrap(),
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(bodyNetworkResult !== null)",
        "true",
        "native Body completion task probe",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("bodyNetworkResult").unwrap()).unwrap();
    assert_eq!(result["total"], 12, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[tokio::test(flavor = "current_thread")]
async fn body_completion_tasks_reach_popups_opened_by_child_windows() {
    let server = super::fetch_resource_timing::timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", server.origin), &loader);
    let fixture = include_str!("../../../tests/fixtures/body-completion-popups.js")
        .trim()
        .trim_end_matches(';');
    vm.eval(&format!(
        "globalThis.bodyPopupResult = null; ({fixture}).then(value => bodyPopupResult = value, error => bodyPopupResult = JSON.stringify({{error:String(error)}}));"
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(bodyPopupResult !== null)",
        "true",
        "popup Body completion task probe",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("bodyPopupResult").unwrap()).unwrap();
    assert_eq!(result["total"], 12, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[tokio::test(flavor = "current_thread")]
async fn body_completion_tasks_follow_receiver_realm_lifetime() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://body-completion.test/", &loader);
    let fixture = include_str!("../../../tests/fixtures/body-completion-realms.js")
        .trim()
        .trim_end_matches(';');
    vm.eval(&format!(
        "globalThis.bodyRealmResult = null; ({fixture}).then(value => bodyRealmResult = value, error => bodyRealmResult = JSON.stringify({{error:String(error)}}));"
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(bodyRealmResult !== null)",
        "true",
        "Body completion realm probe",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("bodyRealmResult").unwrap()).unwrap();
    assert_eq!(result["total"], 28, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

async fn run_body_completion_probe(worker_kind: Option<&str>) {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://body-completion.test/", &loader);
    let fixture = include_str!("../../../tests/fixtures/body-completion-tasks.js")
        .trim()
        .trim_end_matches(';');
    vm.eval("globalThis.bodyCompletionResult = null;").unwrap();
    let script = if let Some(kind) = worker_kind {
        let source = if kind == "module" {
            format!("postMessage(await ({fixture}));")
        } else {
            format!(
                "({fixture}).then(postMessage, error => postMessage(JSON.stringify({{error:String(error)}})));"
            )
        };
        format!(
            r#"
            const workerURL = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
            const worker = new Worker(workerURL, {{type: '{}'}});
            const finish = value => {{
                bodyCompletionResult = value;
                worker.terminate();
                URL.revokeObjectURL(workerURL);
            }};
            worker.onmessage = event => finish(event.data);
            worker.onerror = event => {{
                finish(JSON.stringify({{error:event.message}}));
                event.preventDefault();
            }};
            "#,
            serde_json::to_string(&source).unwrap(),
            kind,
        )
    } else {
        format!(
            "({fixture}).then(value => bodyCompletionResult = value, error => bodyCompletionResult = JSON.stringify({{error:String(error)}}));"
        )
    };
    vm.eval(&script).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(bodyCompletionResult !== null)",
        "true",
        "Body completion task probe",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("bodyCompletionResult").unwrap()).unwrap();
    assert_eq!(result["total"], 108, "worker={worker_kind:?}: {result}");
    assert_eq!(
        result["failures"],
        serde_json::json!([]),
        "worker={worker_kind:?}: {result}"
    );
}
