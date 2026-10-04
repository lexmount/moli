use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

#[tokio::test]
async fn error_events_convert_dictionaries_and_validate_native_receivers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://error-events.test/", &loader);
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
        "globalThis.__errorEventProbe = {}; {}",
        include_str!("error_event_core.js"),
        include_str!("error_event_init.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[tokio::test]
async fn worker_error_events_use_the_same_converted_payload_and_native_brands() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://worker-error-events.test/",
            &loader,
        );
    let source = format!(
        "globalThis.__errorEventProbe = {}; ({}).then(value => globalThis.__errorWorker = value, error => globalThis.__errorWorker = String(error));",
        include_str!("error_event_core.js"),
        include_str!("error_event_worker.js")
    );
    vm.eval(&source).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm.eval("globalThis.__errorWorker !== undefined").unwrap() != "true" {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("worker ErrorEvent probe should settle");
    assert_eq!(vm.eval("globalThis.__errorWorker").unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[tokio::test]
async fn worker_native_error_reporting_uses_protected_payloads_and_onerror_cancellation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://worker-native-errors.test/",
            &loader,
        );
    let source = format!(
        "({}).then(value => globalThis.__nativeErrorReports = value);",
        include_str!("error_event_worker_reporting.js")
    );
    vm.eval(&source).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while vm
            .eval("globalThis.__nativeErrorReports !== undefined")
            .unwrap()
            != "true"
        {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("native Worker error reports should settle");
    let rows: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__nativeErrorReports)").unwrap()).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 8);
    for row in rows.as_array().unwrap() {
        let forwarded = row["mode"] == "forward";
        assert_eq!(row["parentErrors"], u64::from(forwarded), "{row}");
        assert_eq!(
            row["handler"],
            serde_json::json!({"count":5,"receiver":true,"message":true,"filename":true,"location":true,"identity":true}),
            "{row}"
        );
        assert_eq!(
            row["after"],
            serde_json::json!({"errorEvent":true,"cancelable":true,"trusted":true,"prevented":!forwarded}),
            "{row}"
        );
        for field in ["calls", "gets", "fieldReads"] {
            assert_eq!(row[field], 0, "{row}");
        }
    }
}
