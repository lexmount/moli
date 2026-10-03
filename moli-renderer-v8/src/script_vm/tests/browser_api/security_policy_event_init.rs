use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

#[tokio::test]
async fn security_policy_events_convert_dictionaries_and_validate_native_receivers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://security-policy-events.test/",
        &loader,
    );
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame = document.body.appendChild(document.createElement('iframe'));
        frame.id = 'child'; frame.srcdoc = '<head></head><body></body>'; void frame.contentWindow;
        'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    let source = format!(
        "globalThis.__cspEventProbe = {}; {}",
        include_str!("security_policy_event_core.js"),
        include_str!("security_policy_event_init.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[tokio::test]
async fn worker_security_policy_events_use_the_same_converted_payload_and_native_brands() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://worker-security-policy-events.test/",
            &loader,
        );
    let source = format!(
        "globalThis.__cspEventProbe = {}; ({}).then(value => globalThis.__cspWorker = value, error => globalThis.__cspWorker = String(error));",
        include_str!("security_policy_event_core.js"),
        include_str!("security_policy_event_worker.js")
    );
    vm.eval(&source).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm.eval("globalThis.__cspWorker !== undefined").unwrap() != "true" {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("worker SecurityPolicyViolationEvent probe should settle");
    assert_eq!(vm.eval("globalThis.__cspWorker").unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}
