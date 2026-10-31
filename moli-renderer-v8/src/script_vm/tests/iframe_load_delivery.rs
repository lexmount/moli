use super::*;

const FIXTURE: &str = include_str!("../../../tests/fixtures/iframe-load-delivery.js");
const CHROMIUM: &str = include_str!("../../../tests/fixtures/iframe-load-delivery.chromium.json");

fn snapshot(vm: &mut ScriptVm) -> serde_json::Value {
    serde_json::from_str(
        &vm.eval("JSON.stringify(iframeLoadDelivery.snapshot())")
            .expect("iframe load trace should evaluate"),
    )
    .expect("iframe load trace should be JSON")
}

#[tokio::test]
async fn iframe_load_delivery_matches_chromium_before_and_after_owner_tasks() {
    let oracle: serde_json::Value = serde_json::from_str(CHROMIUM).unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut mismatches = Vec::new();
    for (name, expected) in oracle["cases"].as_object().unwrap() {
        let mut vm = new_storage_test_vm_with_loader("https://iframe-load-delivery.test/", &loader);
        vm.eval(FIXTURE)
            .expect("shared load fixture should install");
        vm.eval(&format!(
            "iframeLoadDelivery.start({})",
            serde_json::to_string(name).unwrap()
        ))
        .unwrap_or_else(|error| panic!("{name}: setup failed: {error}"));
        let immediate = snapshot(&mut vm);
        if immediate != expected["immediate"] {
            mismatches.push(format!(
                "{name} immediate: expected {}, got {immediate}",
                expected["immediate"]
            ));
        }
        // Exercise real owner sources through quiescence, including any
        // navigation created by the synchronous initial load callback.
        vm.drain_pending_child_frame_work_for_test_on_owner_sources()
            .await;
        let settled = snapshot(&mut vm);
        if settled != expected["settled"] {
            mismatches.push(format!(
                "{name} settled: expected {}, got {settled}",
                expected["settled"]
            ));
        }
        assert!(
            vm.run_next_child_frame_semantic_turn_for_test()
                .await
                .is_none(),
            "{name}: load delivery must not leave a duplicate owner task"
        );
        if matches!(name.as_str(), "pending-replacement" | "navigate-in-onload") {
            assert_eq!(
                vm.eval("iframeLoadDelivery.bodyText()").unwrap(),
                "replacement",
                "{name}: only the replacement Document should finish"
            );
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[tokio::test]
async fn iframe_load_delivery_discards_queued_load_for_replaced_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_test_vm_with_loader("https://iframe-load-replacement.test/", &loader);
    vm.eval(FIXTURE).unwrap();
    vm.eval("iframeLoadDelivery.start('srcdoc')").unwrap();
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "original Document should commit",
    )
    .await;
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            transition,
        )
        .await;
    }
    // Complete has reserved HostLoad, but it has not called author JS yet.
    vm.eval("iframeLoadDelivery.replace()").unwrap();
    assert_eq!(
        vm.run_child_host_load_body_for_test().unwrap(),
        Some(crate::script_vm::ChildHostLoadRunOutcome::ConsumedWithoutCallback),
        "the superseded HostLoad must consume its reservation without calling author JS"
    );
    assert_eq!(snapshot(&mut vm), serde_json::json!(["before", "after"]));
    assert_eq!(vm.completed_child_frame_navigation_load_count(), 0);

    vm.drain_pending_child_frame_work_for_test_on_owner_sources()
        .await;
    assert_eq!(
        snapshot(&mut vm),
        serde_json::json!(["before", "after", "frame:handler", "frame:listener"])
    );
    assert_eq!(
        vm.eval("iframeLoadDelivery.bodyText()").unwrap(),
        "replacement"
    );
    assert_eq!(vm.completed_child_frame_navigation_load_count(), 1);
    assert!(
        vm.run_next_child_frame_semantic_turn_for_test()
            .await
            .is_none()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn iframe_reentrant_load_callbacks_preserve_descendant_completion_boundaries() {
    const SOURCE: &str = include_str!("../../../tests/fixtures/iframe-reentrant-load.js");
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/iframe-reentrant-load.chromium.json"
    ))
    .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://iframe-load-delivery.test/").unwrap(),
        &loader,
    );
    vm.eval(&format!(
        "({SOURCE}).then(value => globalThis.reentrantLoadResult = value, \
         error => globalThis.reentrantLoadError = error.stack || String(error));"
    ))
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while vm.eval("typeof reentrantLoadResult").unwrap() == "undefined" {
        assert_eq!(
            vm.eval("typeof reentrantLoadError").unwrap(),
            "undefined",
            "reentrant load fixture failed: {}",
            vm.eval("globalThis.reentrantLoadError").unwrap()
        );
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "reentrant iframe load fixture did not complete"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    let actual: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(reentrantLoadResult)").unwrap()).unwrap();
    let actual = actual.as_array().unwrap();
    let expected = oracle["cases"].as_array().unwrap();
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual["phase"], expected["phase"]);
        assert_eq!(actual["complete"], true);
        if actual["phase"] != "window-microtask" {
            assert_eq!(actual, expected, "synchronous callback ordering");
            continue;
        }
        // This topic changes load-dependency delivery. Main's separate
        // callback-microtask cleanup policy can place these two markers later
        // than Chromium; retain their own order and the load completion order.
        let events = actual["events"].as_array().unwrap();
        let position = |name: &str| events.iter().position(|event| event == name).unwrap();
        assert!(position("window-microtask") < position("descendant-insert"));
        assert!(position("descendant-insert") < position("descendant-load"));
        let lifecycle = |value: &serde_json::Value| {
            value["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| *event != "window-microtask" && *event != "descendant-insert")
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lifecycle(actual),
            lifecycle(expected),
            "microtask descendant must retain load completion boundaries"
        );
    }
}
