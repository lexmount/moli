use super::*;

use crate::{
    page_task_queue::{
        PageServiceWorkerClientMessageTargetEffect, ServiceWorkerClientMessageCallbackEffect,
        ServiceWorkerClientMessageEventKind,
    },
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
    service_worker_runtime::ServiceWorkerVersionId,
    types::ServiceWorkerClientMessageCompletion,
};

fn service_worker_client_message(
    target: crate::types::ServiceWorkerWindowClientTarget,
    payload: crate::structured_clone::V8StructuredClonePayload,
) -> ServiceWorkerClientMessageCompletion {
    ServiceWorkerClientMessageCompletion {
        target,
        source_version_id: ServiceWorkerVersionId::from_u64_for_test(41),
        source_script_url: Url::parse("https://service-worker-client-message.test/worker.js")
            .expect("ServiceWorker test script URL"),
        source_state: "activated",
        payload,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_body_leaves_reactions_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/current").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerClientMessageEvents = [];
navigator.serviceWorker.onmessage = event => {
  __serviceWorkerClientMessageEvents.push(event.type + ":" + event.data);
  Promise.resolve().then(() => {
    __serviceWorkerClientMessageEvents.push("microtask");
    const script = document.createElement("script");
    script.textContent =
      "__serviceWorkerClientMessageEvents.push('runtime-script')";
    document.body.appendChild(script);
  });
};
"listener-ready"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        let payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("payload")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(target, payload))
            .expect("current ServiceWorker client message should enter its typed source");

        let task = page_vm
            .take_service_worker_client_message_body_task_for_test()
            .expect("one ServiceWorker client message should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_client_message_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerClientMessageTargetEffect::EventDispatchedToCurrentTarget {
                event_kind: ServiceWorkerClientMessageEventKind::Message,
                callback_effect: ServiceWorkerClientMessageCallbackEffect::CallbackDispatched,
            }
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerClientMessageEvents.join('|')")?,
            "message:payload",
            "the ServiceWorker message body must leave its Promise reaction pending"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the message body must not consume unrelated runtime residence"
        );
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerClientMessageEvents.join('|')")?,
            "message:payload|microtask|runtime-script",
            "selected completion must own the checkpoint and runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker client-message body probe should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_messageerror_uses_the_same_selected_completion_boundary() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/messageerror").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerClientMessageErrorEvents = [];
navigator.serviceWorker.onmessageerror = event => {
  __serviceWorkerClientMessageErrorEvents.push(event.type);
  Promise.resolve().then(() => {
    __serviceWorkerClientMessageErrorEvents.push("microtask");
  });
};
"listener-ready"
"#,
        )?;
        let target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(
                target,
                crate::structured_clone::V8StructuredClonePayload::default(),
            ))
            .expect("invalid structured clone should enter the typed source");

        let task = page_vm
            .take_service_worker_client_message_body_task_for_test()
            .expect("one ServiceWorker messageerror task should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_client_message_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerClientMessageTargetEffect::EventDispatchedToCurrentTarget {
                event_kind: ServiceWorkerClientMessageEventKind::MessageError,
                callback_effect: ServiceWorkerClientMessageCallbackEffect::CallbackDispatched,
            }
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerClientMessageErrorEvents.join('|')")?,
            "messageerror",
            "the messageerror body must leave its reaction pending"
        );

        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerClientMessageErrorEvents.join('|')")?,
            "messageerror|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker messageerror completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn current_service_worker_client_without_a_dispatchable_container_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/tampered-container").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        page_vm.vm_mut().eval(
            r#"
Object.defineProperty(navigator, "serviceWorker", {
  configurable: true,
  value: null,
});
"container-hidden"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("payload")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(target, payload))
            .expect("current exact target should still enter the typed source");

        let task = page_vm
            .take_service_worker_client_message_body_task_for_test()
            .expect("container-free current target should consume one bounded turn");
        let outcome = page_vm.apply_selected_page_service_worker_client_message_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerClientMessageTargetEffect::CurrentTargetProducedNoDispatchableEvent
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a callback-free current task must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker current-target no-event completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_without_listener_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/no-listener").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("payload")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(target, payload))
            .expect("listener-free current target should enter the typed source");

        let task = page_vm
            .take_service_worker_client_message_body_task_for_test()
            .expect("one listener-free ServiceWorker message should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_client_message_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerClientMessageTargetEffect::EventDispatchedToCurrentTarget {
                event_kind: ServiceWorkerClientMessageEventKind::Message,
                callback_effect:
                    ServiceWorkerClientMessageCallbackEffect::CurrentTargetHadNoCallback,
            }
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a listener-free ServiceWorker task must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker no-listener completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_completion_syncs_a_reaction_created_child() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerClientMessageChildOrder = [];
navigator.serviceWorker.onmessage = () => {
  __serviceWorkerClientMessageChildOrder.push("callback");
  Promise.resolve().then(() => {
    __serviceWorkerClientMessageChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "service-worker-message-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
};
"listener-ready"
"#,
        )?;
        let target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        let payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("payload")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(target, payload))
            .expect("current ServiceWorker message should enter the typed source");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ServiceWorkerClientMessage,
                    &loader
                )
                .await?,
            "the message must return through the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerClientMessageChildOrder.join('|')")?,
            "callback|microtask",
            "the selected checkpoint must precede child-record synchronization"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "the reaction-created srcdoc frame must publish its later typed navigation turn"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit)
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_completion_finishes_after_listener_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let root_document = page_vm.document_lifecycle.identity().document;
        let retired_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial exact Document owner");

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerDocumentOpenEvents = [];
navigator.serviceWorker.onmessage = () => {
  __serviceWorkerDocumentOpenEvents.push("callback");
  document.open();
  document.write("<!doctype html><body>replacement</body>");
  document.close();
  Promise.resolve().then(() => {
    __serviceWorkerDocumentOpenEvents.push("microtask");
  });
};
"listener-ready"
"#,
        )?;
        let target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        let payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("payload")?;
        page_vm
            .service_worker_task_sender_for_root_for_test(root_document)
            .send_service_worker_client_message(service_worker_client_message(target, payload))
            .expect("ServiceWorker message should enter the typed source");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ServiceWorkerClientMessage,
                    &loader
                )
                .await?,
            "listener-triggered replacement must return through selected completion"
        );
        let current_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open replacement owner");
        assert_ne!(
            current_document_owner, retired_document_owner,
            "document.open must rotate the exact ScriptVm Document owner"
        );
        assert_eq!(
            page_vm.document_lifecycle.identity().document,
            root_document,
            "document.open retains the PageVm/root-navigation identity"
        );
        let current_target = page_vm.vm().service_worker_client_message_target_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
        )?;
        assert_eq!(current_target.client_id, target.client_id);
        assert_ne!(
            current_target.document_owner, target.document_owner,
            "document.open must retire the authorized ServiceWorker client document owner"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerDocumentOpenEvents.join('|')")?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker listener document.open completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_dispatches_to_the_exact_child_window() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/child-target").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "service-worker-message-child-target";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "service-worker-message-child-target",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("service-worker-message-child-target")
            .expect("child frame handle");
        let child_client_id = page_vm
            .vm_mut()
            .register_service_worker_child_client_for_test(child_handle)?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerTopClientMessages = [];
navigator.serviceWorker.onmessage = event => {
  __serviceWorkerTopClientMessages.push(event.data);
};
const child =
  document.getElementById("service-worker-message-child-target").contentWindow;
child.__serviceWorkerChildClientMessages = [];
child.navigator.serviceWorker.onmessage = child.Function(
  "event",
  "__serviceWorkerChildClientMessages.push(event.data)"
);
"listeners-ready"
"#,
        )?;

        let target = page_vm
            .vm()
            .service_worker_client_message_target_for_test(
                crate::native_bridge::OwnerDispatchScope::Child(child_handle),
            )?;
        assert_eq!(
            target.client_id, child_client_id,
            "the test target must name the registered child client, not the top fallback"
        );
        let payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("child-payload")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(target, payload))
            .expect("child message should enter the typed Page source");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ServiceWorkerClientMessage, &loader)
                .await?,
            "the exact child task should run through the production dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval(
                    "document.getElementById('service-worker-message-child-target').contentWindow.__serviceWorkerChildClientMessages.join('|')",
                )?,
            "child-payload"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerTopClientMessages.join('|')")?,
            "",
            "a child-targeted message must not leak into the top Window client"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("exact-child ServiceWorker client-message test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_discards_a_retired_child_document_target() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-client-message.test/child-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "service-worker-message-replacement-child";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "service-worker-message-replacement-child",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("service-worker-message-replacement-child")
            .expect("child frame handle");
        let retired_client_id = page_vm
            .vm_mut()
            .register_service_worker_child_client_for_test(child_handle)?;
        let retired_target = page_vm
            .vm()
            .service_worker_client_message_target_for_test(
                crate::native_bridge::OwnerDispatchScope::Child(child_handle),
            )?;
        assert_eq!(retired_target.client_id, retired_client_id);
        let retired_payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("retired")?;
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(
                retired_target,
                retired_payload,
            ))
            .expect("retired child message should remain resident in the typed source");

        page_vm.vm_mut().eval(
            r#"
document.getElementById("service-worker-message-replacement-child").srcdoc =
  "<!doctype html><body>replacement</body>";
"replacement-queued"
"#,
        )?;
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "child replacement must rotate the exact Document before task selection"
        );
        let replacement_client_id = page_vm
            .vm_mut()
            .register_service_worker_child_client_for_test(child_handle)?;
        let current_target = page_vm
            .vm()
            .service_worker_client_message_target_for_test(
                crate::native_bridge::OwnerDispatchScope::Child(child_handle),
            )?;
        assert_eq!(
            retired_target.client_id, replacement_client_id,
            "child replacement should exercise the real reused client-id collision"
        );
        assert_eq!(retired_target.client_id, current_target.client_id);
        assert_ne!(
            retired_target.document_owner, current_target.document_owner,
            "exact document owner must distinguish reused child client ids"
        );
        page_vm.vm_mut().eval(
            r#"
const replacementChild =
  document.getElementById("service-worker-message-replacement-child").contentWindow;
replacementChild.__replacementServiceWorkerChildMessages = [];
replacementChild.navigator.serviceWorker.onmessage = replacementChild.Function(
  "event",
  "__replacementServiceWorkerChildMessages.push(event.data)"
);
"replacement-listener-ready"
"#,
        )?;
        let current_payload = page_vm
            .vm_mut()
            .service_worker_client_message_payload_for_test("current")?;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_client_message(
                current_target,
                current_payload,
            ))
            .expect("current child message should follow the stale task");

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ServiceWorkerClientMessage,
            )
            .expect("retired child task should remain a bounded stale turn");
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval(
                    "document.getElementById('service-worker-message-replacement-child').contentWindow.__replacementServiceWorkerChildMessages.join('|')",
                )?,
            "",
            "the retired child task must not enter the replacement Window"
        );

        let current = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ServiceWorkerClientMessage,
            )
            .expect("the current child task should remain behind the stale source head");
        page_vm
            .run_claimed_selected_page_task_for_test(current, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval(
                    "document.getElementById('service-worker-message-replacement-child').contentWindow.__replacementServiceWorkerChildMessages.join('|')",
                )?,
            "current"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker child replacement authorization test should run");
}

#[test]
fn service_worker_client_message_rejects_a_real_root_document_replacement() {
    run_page_vm_large_stack_async_test(
        "service-worker-client-message-real-root-replacement",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let retired_root = page_vm.document_lifecycle.identity().document;
                    let retired_target = page_vm
                        .vm()
                        .service_worker_client_message_target_for_test(
                            crate::native_bridge::OwnerDispatchScope::Top,
                        )?;
                    let retired_payload = page_vm
                        .vm_mut()
                        .service_worker_client_message_payload_for_test("retired")?;
                    page_vm
                        .service_worker_task_sender_for_root_for_test(retired_root)
                        .send_service_worker_client_message(service_worker_client_message(
                            retired_target,
                            retired_payload,
                        ))
                        .expect("retired-root message should remain resident");

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                    let mut pending_document_lifecycle_turn = None;
                    let navigation = page_vm
                        .follow_pending_location_navigation_one_turn_async(
                            &mut pending_document_lifecycle_turn,
                            PageVmInitStage::Load,
                        )
                        .await?;
                    assert!(matches!(
                        navigation,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                            | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                ..
                            }
                    ));

                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);
                    let current_target = page_vm
                        .vm()
                        .service_worker_client_message_target_for_test(
                            crate::native_bridge::OwnerDispatchScope::Top,
                        )?;
                    assert_ne!(
                        retired_target.client_id, current_target.client_id,
                        "a top-level network replacement installs a new browser ServiceWorker client"
                    );
                    assert_ne!(
                        retired_target, current_target,
                        "the replacement must install a distinct exact ServiceWorker client target"
                    );
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__replacementServiceWorkerRootMessages = [];
navigator.serviceWorker.onmessage = event => {
  __replacementServiceWorkerRootMessages.push(event.data);
};
"replacement-listener-ready"
"#,
                    )?;
                    let current_payload = page_vm
                        .vm_mut()
                        .service_worker_client_message_payload_for_test("current")?;
                    page_vm
                        .service_worker_task_sender_for_root_for_test(current_root)
                        .send_service_worker_client_message(service_worker_client_message(
                            current_target,
                            current_payload,
                        ))
                        .expect("replacement-root message should follow the stale task");

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ServiceWorkerClientMessage,
                        )
                        .expect("retired-root task should remain a bounded stale turn");
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementServiceWorkerRootMessages.join('|')")?,
                        "",
                        "the retired task must not inspect or dispatch into the replacement PageVm"
                    );

                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ServiceWorkerClientMessage,
                        )
                        .expect("the current-root message should remain behind the stale source head");
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementServiceWorkerRootMessages.join('|')")?,
                        "current"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("ServiceWorker root replacement proof should run");
            server.await.expect("replacement HTTP server");
        },
    );
}
