use super::*;

async fn assert_readable_transfer_cancels_source(mode: &str, behavior: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-cancel-failure.test/",
        &loader,
    );
    let registry = vm._context_host.borrow().message_port_registry();
    vm.eval(include_str!("cancellation.js"))
        .expect("install transferable stream cancellation probe");
    let config = serde_json::json!({
        "name": format!("{mode}-{behavior}"),
        "mode": mode,
        "cancel": behavior,
    });
    vm.eval(&format!("runMessageSourceProbe({config})"))
        .expect("start transferred readable stream cancellation");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.probeResult?.complete === true)",
        "true",
        "source cancellation and remote stream must settle",
    )
    .await;
    let consumer = if mode == "bad-chunk" {
        serde_json::json!({
            "operation": {"status": "rejected", "name": "DataCloneError", "realm": true},
            "closed": {"status": "rejected", "name": "DataCloneError", "realm": true},
            "sameError": true,
        })
    } else {
        serde_json::json!({
            "operation": {"status": "fulfilled"},
            "closed": {"status": "fulfilled"},
            "sameError": null,
        })
    };
    let reentry = if behavior == "reenter" {
        serde_json::json!({"status": "TypeError", "realm": true})
    } else {
        serde_json::Value::Null
    };
    let observed = vm
        .eval("JSON.stringify([probeResult.settled, probeResult.cancels, probeResult.reason, probeResult.sourceDesiredSize, probeResult.consumer, probeResult.reentry, probeResult.unhandled])")
        .expect("inspect source cancellation");
    let observed: serde_json::Value =
        serde_json::from_str(&observed).expect("cancellation observation JSON");
    assert_eq!(
        observed,
        serde_json::json!([
            true,
            1,
            {"kind": "exception", "name": "DataCloneError", "realm": true},
            0,
            consumer,
            reentry,
            [],
        ]),
        "{mode} with {behavior}"
    );
    for _ in 0..32 {
        if registry.endpoint_count() == 0
            && vm
                ._context_host
                .borrow()
                .message_port_execution_context_owners_for_test()
                .is_empty()
        {
            break;
        }
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("release canceled stream transfer endpoints");
    }
    assert_eq!(registry.endpoint_count(), 0, "native endpoint leak");
    assert!(
        vm._context_host
            .borrow()
            .message_port_execution_context_owners_for_test()
            .is_empty(),
        "native wrapper leak"
    );
}

#[tokio::test]
async fn blocked_cancel_reason_cancels_the_original_readable() {
    assert_readable_transfer_cancels_source("blocked-cancel", "resolve").await;
}

#[tokio::test]
async fn uncloneable_chunk_cancels_source_before_reentrant_enqueue() {
    assert_readable_transfer_cancels_source("bad-chunk", "reenter").await;
}

#[tokio::test]
async fn uncloneable_chunk_handles_rejected_and_throwing_cancel_callbacks() {
    for behavior in ["reject", "throw"] {
        assert_readable_transfer_cancels_source("bad-chunk", behavior).await;
    }
}

#[tokio::test]
async fn blocked_cancel_reason_releases_ports_without_awaiting_source_cancel() {
    assert_readable_transfer_cancels_source("blocked-cancel", "pending").await;
}
