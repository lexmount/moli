//! P5 task-end contracts for child default-realm materialization.
//!
//! Realm construction, runtime-binding replay and every stored document-start
//! script belong to one selected child-frame task body. None of those nested
//! helpers may discharge the surrounding task's microtask checkpoint.

use super::*;

fn install_two_child_document_start_scripts(page_vm: &mut PageVm) {
    page_vm.vm_mut().set_stored_document_start_scripts(&[
        crate::DocumentStartScript {
            registry_key: Some("realm-completion-first".to_owned()),
            devtools_session: None,
            source: r#"
parent.__childRealmCompletionOrder.push("first-body");
Promise.resolve().then(() => {
  parent.__childRealmCompletionOrder.push("first-microtask");
});
"#
            .to_owned(),
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        },
        crate::DocumentStartScript {
            registry_key: Some("realm-completion-second".to_owned()),
            devtools_session: None,
            source: r#"
parent.__childRealmCompletionOrder.push("second-body");
"#
            .to_owned(),
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        },
    ]);
}

fn queue_child_realm_materialization(page_vm: &mut PageVm, element_id: &str) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
const frame = document.createElement("iframe");
frame.id = {element_id:?};
document.body.appendChild(frame);
void frame.contentWindow.Function;
"queued"
"#,
    ))?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn child_realm_materialization_body_does_not_checkpoint_runtime_binding_refresh() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-realm-binding-refresh-body").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_realm_materialization(&mut page_vm, "realm-binding-refresh-child")?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__childRealmRefreshCheckpoint = 0;
Promise.resolve().then(() => __childRealmRefreshCheckpoint = 1);
"reaction queued"
"#,
            )?;

        let outcome = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("one exact child realm materialization should be ready");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript,
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__childRealmRefreshCheckpoint)",
            )?,
            "0",
            "runtime-binding refresh must not discharge the selected realm task's checkpoint",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child realm runtime-binding body boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_realm_materialization_body_does_not_checkpoint_between_document_start_scripts() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-realm-materialization-body").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        install_two_child_document_start_scripts(&mut page_vm);
        page_vm
            .vm_mut()
            .eval("globalThis.__childRealmCompletionOrder = []; 'ready'")?;
        queue_child_realm_materialization(&mut page_vm, "realm-completion-body-child")?;

        let outcome = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("one exact child realm materialization should be ready");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerAfterDocumentStartScript,
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "JSON.stringify(globalThis.__childRealmCompletionOrder)",
            )?,
            r#"["first-body","second-body"]"#,
            "all document-start bodies must finish before the selected task submits one checkpoint",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child realm materialization body boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_realm_materialization_creates_named_preload_world_in_the_selected_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-named-world-materialization").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.set_stored_document_start_scripts(&[crate::DocumentStartScript {
            registry_key: Some("child-named-world-script".to_owned()),
            devtools_session: None,
            source: "globalThis.__childNamedWorldReady = 'ready';".to_owned(),
            world_name: Some("child-utility".to_owned()),
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        }]);
        page_vm.set_stored_runtime_bindings(&[
            crate::protocol_types::RuntimeBindingRegistration {
                devtools_session: None,
                name: "childNamedBinding".to_owned(),
                execution_context_name: Some("child-utility".to_owned()),
            },
        ]);
        queue_child_realm_materialization(&mut page_vm, "named-world-child")?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("named-world-child")
            .expect("child owner handle");
        let frame_id = page_vm
            .vm()
            .child_browsing_context_frame_id_by_owner_node_id(child_handle)
            .expect("child frame id");

        let outcome = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("one exact child realm materialization should be ready");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerAfterDocumentStartScript,
        );
        assert!(
            page_vm
                .has_isolated_world_named_for_frame(&frame_id, "child-utility"),
            "the selected child-realm turn must create its named preload world",
        );
        let execution_context_id =
            page_vm.create_isolated_world_for_frame(&frame_id, "child-utility", false)?;
        let state = page_vm.evaluate_expression_in_execution_context_with_await(
            execution_context_id,
            "JSON.stringify([globalThis.__childNamedWorldReady, typeof childNamedBinding])",
            false,
        )?;
        assert_eq!(
            state["value"],
            serde_json::json!("[\"ready\",\"function\"]"),
            "named preload script and binding must be installed before the materialization turn settles",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child named-world materialization witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn detached_session_binding_is_not_installed_in_a_later_child_realm() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/detached-session-child-binding").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let session_a =
            moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-a"));
        let session_b =
            moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-b"));
        let binding =
            |devtools_session, name: &str| crate::protocol_types::RuntimeBindingRegistration {
                devtools_session: Some(devtools_session),
                name: name.to_owned(),
                execution_context_name: None,
            };
        let bindings = [
            binding(session_a, "detachedChildBinding"),
            binding(session_b, "liveChildBinding"),
        ];
        page_vm.set_stored_runtime_bindings(&bindings);
        page_vm
            .vm_mut()
            .ensure_runtime_inspector_session(Some("session-a"));

        assert!(page_vm.detach_runtime_inspector_session(Some("session-a")));
        queue_child_realm_materialization(&mut page_vm, "post-detach-binding-child")?;
        page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("the post-detach child realm should materialize");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"JSON.stringify([
typeof document.getElementById("post-detach-binding-child").contentWindow.detachedChildBinding,
typeof document.getElementById("post-detach-binding-child").contentWindow.liveChildBinding,
])"#,
            )?,
            r#"["undefined","function"]"#,
            "a child realm created after detach must receive only live-session bindings",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("detached-session child binding witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_realm_materialization_checkpoints_after_all_document_start_scripts() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/selected-child-realm-materialization").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        install_two_child_document_start_scripts(&mut page_vm);
        page_vm
            .vm_mut()
            .eval("globalThis.__childRealmCompletionOrder = []; 'ready'")?;
        queue_child_realm_materialization(&mut page_vm, "selected-realm-completion-child")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildRealmMaterialization,
                    &loader,
                )
                .await?,
            "one exact realm task must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "JSON.stringify(globalThis.__childRealmCompletionOrder)",
            )?,
            r#"["first-body","second-body","first-microtask"]"#,
            "the selected task must checkpoint once after every synchronous document-start body",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "realm script completion may publish readiness but must not synchronously execute unrelated runtime work",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child realm completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_realm_document_start_reaction_publishes_nested_navigation_before_task_returns() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-realm-document-start-followup").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm
            .vm_mut()
            .set_stored_document_start_scripts(&[crate::DocumentStartScript {
                registry_key: Some("realm-completion-nested-child".to_owned()),
                devtools_session: None,
                source: r#"
Promise.resolve().then(() => {
  const nested = document.createElement("iframe");
  nested.srcdoc = "<!doctype html><body>nested</body>";
  document.body.appendChild(nested);
  parent.__childRealmNestedReaction = "ran";
});
"#
                .to_owned(),
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            }]);
        page_vm
            .vm_mut()
            .eval("globalThis.__childRealmNestedReaction = 'pending'; 'ready'")?;
        queue_child_realm_materialization(&mut page_vm, "realm-followup-parent")?;

        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildNavigationCommit,
                )
                .is_none(),
            "the nested child does not exist before the realm task checkpoint",
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildRealmMaterialization,
                    &loader,
                )
                .await?,
            "the exact realm task must run through the production dispatcher",
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "globalThis.__childRealmNestedReaction",
            )?,
            "ran",
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildNavigationCommit,
                )
                .is_some(),
            "task-end child-record synchronization must publish the reaction-created nested navigation",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child realm document-start follow-up witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_realm_materialization_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/stale-child-realm-materialization").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_realm_materialization(&mut page_vm, "retired-realm-child")?;
        let retired_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner");
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildRealmMaterialization,
            )
            .expect("the old Document must retain one opaque realm claim");

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"replaced"
"#,
        )?;
        let replacement_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner");
        assert_ne!(retired_document, replacement_document);
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleChildRealmCheckpoint = 0;
Promise.resolve().then(() => __staleChildRealmCheckpoint = 1);
"replacement reaction queued"
"#,
            )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleChildRealmCheckpoint)",
                )?,
            "0",
            "a stale realm claim must not enter the replacement realm for a checkpoint",
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("replacement owner must remain installed"),
            replacement_document,
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child realm completion witness should run");
}
