use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

#[tokio::test]
async fn protocol_handlers_validate_parameters_and_the_receivers_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://protocol-handlers.test/",
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
    assert_eq!(vm.eval(include_str!("protocol_handlers.js")).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[test]
fn protocol_handlers_are_absent_in_insecure_contexts() {
    let mut vm = new_storage_test_vm("http://protocol-handlers.test/");
    assert_eq!(
        vm.eval(
            r#"
        const prototype = Navigator.prototype;
        const names = ['registerProtocolHandler', 'unregisterProtocolHandler'];
        const prototypeFirst = names.every(name => !(name in prototype));
        !isSecureContext && prototypeFirst && names.every(name => !(name in navigator))
    "#
        )
        .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn protocol_handlers_are_absent_from_worker_navigators() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://protocol-worker.test/",
            &loader,
        );
    let source = include_str!("protocol_worker.js");
    vm.eval(&format!("({source}).then(value => globalThis.__protocolWorker = value, error => globalThis.__protocolWorker = String(error));")).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm
            .eval("globalThis.__protocolWorker !== undefined")
            .unwrap()
            != "true"
        {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("protocol worker probe should settle");
    assert_eq!(vm.eval("globalThis.__protocolWorker").unwrap(), "true");
}

#[test]
fn protocol_handler_prototypes_are_finalized_before_navigator_materializes() {
    for (url, expected) in [
        ("http://protocol-handlers.test/", "[false,false,false]"),
        ("https://protocol-handlers.test/", "[true,true,true]"),
        ("http://localhost/", "[true,true,true]"),
    ] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(
            vm.eval(
                r#"
            const prototype = Navigator.prototype;
            const names = ['registerProtocolHandler', 'unregisterProtocolHandler'];
            const exposedBeforeInstance = names.every(name => Object.hasOwn(prototype, name));
            const exposedAfterInstance = names.every(name => name in navigator);
            JSON.stringify([isSecureContext, exposedBeforeInstance, exposedAfterInstance]);
        "#
            )
            .unwrap(),
            expected,
            "{url}"
        );
    }
}
