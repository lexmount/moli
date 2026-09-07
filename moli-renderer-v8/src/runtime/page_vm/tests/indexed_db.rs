use super::*;

use crate::{
    page_task_queue::{PageIndexedDbTaskTargetEffect, RendererPageIndexedDbTaskKind},
    runtime::{AuthorizedCurrentPageIndexedDbTask, PageTaskCompletion},
    script_vm::{IndexedDbStaleTaskCleanupEffect, IndexedDbTaskBodyEffect},
};

fn install_indexed_db_manager(page_vm: &mut PageVm, manager: &crate::SharedIndexedDbManager) {
    let manager = crate::downgrade_indexed_db_manager(manager);
    page_vm.indexed_db_manager = Some(manager.clone());
    page_vm.vm_mut().set_indexed_db_manager(Some(manager));
}

fn schedule_open(page_vm: &mut PageVm, database_name: &str, marker: &str) {
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
(() => {{
  globalThis[{marker:?}] = [];
  const request = indexedDB.open({database_name:?}, 1);
  request.onupgradeneeded = () => {{
    globalThis[{marker:?}].push("upgrade");
    Promise.resolve().then(() => globalThis[{marker:?}].push("upgrade-microtask"));
  }};
  request.onerror = () => {{
    globalThis[{marker:?}].push(`error:${{request.error && request.error.name}}`);
  }};
  request.onsuccess = () => {{
    globalThis[{marker:?}].push("success");
    Promise.resolve().then(() => globalThis[{marker:?}].push("microtask"));
    request.result.close();
  }};
  return "scheduled";
}})()
"#,
        ))
        .expect("IndexedDB open should schedule one typed Page task");
}

fn schedule_child_open(
    page_vm: &mut PageVm,
    execution_context_id: i64,
    database_name: &str,
    marker: &str,
) {
    page_vm
        .vm_mut()
        .eval_in_child_default_context(
            execution_context_id,
            &format!(
                r#"
(() => {{
  globalThis[{marker:?}] = [];
  const request = indexedDB.open({database_name:?}, 1);
  request.onupgradeneeded = () => {{
    globalThis[{marker:?}].push("upgrade");
    Promise.resolve().then(() => globalThis[{marker:?}].push("upgrade-microtask"));
  }};
  request.onerror = () => {{
    globalThis[{marker:?}].push(`error:${{request.error && request.error.name}}`);
  }};
  request.onsuccess = () => {{
    globalThis[{marker:?}].push("success");
    Promise.resolve().then(() => globalThis[{marker:?}].push("microtask"));
    request.result.close();
  }};
  return "scheduled";
}})()
"#,
            ),
        )
        .expect("child IndexedDB open should schedule one exact-realm Page task");
}

fn take_indexed_db_page_task_for_test(
    page_vm: &mut PageVm,
) -> crate::page_task_queue::RendererPageIndexedDbTask {
    page_vm
        .page_task_executor_sources_for_test()
        .take_indexed_db_task_for_executor_test()
        .expect("one exact IndexedDB task should be ready")
}

#[derive(Clone, Copy, Debug)]
struct IndexedDbSelectedTaskObservation {
    owner: crate::page_task_queue::RendererPageIndexedDbTaskOwner,
    kind: crate::page_task_queue::RendererPageIndexedDbTaskKind,
}

/// Execute one exact IndexedDB task without reproducing selected-task policy.
///
/// The domain metadata is observed while the generic claim remains opaque;
/// execution then returns through the production dispatcher owned by the
/// shared selected-task harness.
async fn run_selected_indexed_db_task_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<Option<IndexedDbSelectedTaskObservation>> {
    let Some(claimed) = page_vm
        .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::IndexedDbTask)
    else {
        return Ok(None);
    };
    assert_eq!(
        claimed.selector(),
        PageSelectedTaskTestSelector::IndexedDbTask
    );
    let (owner, kind) = claimed
        .indexed_db_owner_and_kind()
        .expect("exact IndexedDB selector must retain IndexedDB metadata");
    page_vm
        .run_claimed_selected_page_task_for_test(claimed, loader)
        .await?;
    Ok(Some(IndexedDbSelectedTaskObservation { owner, kind }))
}

/// Admission is now its own task. Advance it explicitly so tests of callback
/// checkpoints still select exactly one callback, rather than draining work.
async fn admit_indexed_db_connections_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    let admission = run_selected_indexed_db_task_for_test(page_vm, loader)
        .await?
        .expect("connection admission should have a selected task");
    assert_eq!(
        admission.kind,
        RendererPageIndexedDbTaskKind::DrainConnectionRequests
    );
    Ok(())
}

/// Set up an open-success task without treating upgrade/commit/success as one
/// task. The upgrade's complete listener is the boundary, not a queue length
/// or a timed wait. The caller still executes and checks exactly one task.
async fn complete_fixture_upgrade_before_open_success(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    for _ in 0..16 {
        if page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "globalThis.__indexedDbFixtureUpgradeComplete === true",
            )?
            == "true"
        {
            return Ok(());
        }
        run_selected_indexed_db_task_for_test(page_vm, loader)
            .await?
            .expect("upgrade must retain a ready IndexedDB task until complete");
    }
    anyhow::bail!("fixture upgrade did not complete within 16 selected IndexedDB tasks")
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_task_body_leaves_reactions_and_transaction_deactivation_for_selected_completion()
 {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url =
            Url::parse("https://example.com/indexed-db-task-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__indexedDbTaskBodyBoundary = [];
  const open = indexedDB.open("body-boundary", 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv").put("value", 1);
    __indexedDbTaskBodyBoundary.push("upgrade");
    open.transaction.oncomplete = () => globalThis.__indexedDbFixtureUpgradeComplete = true;
  };
  open.onsuccess = () => {
    __indexedDbTaskBodyBoundary.push("success");
    const transaction = open.result.transaction("kv");
    globalThis.__indexedDbTaskBodyStore = transaction.objectStore("kv");
    queueMicrotask(() => {
      try {
        __indexedDbTaskBodyStore.get(1);
        __indexedDbTaskBodyBoundary.push("microtask-active");
      } catch (error) {
        __indexedDbTaskBodyBoundary.push(`microtask-throw:${error && error.name}`);
      }
    });
  };
  return "scheduled";
})()
"#,
        )?;
        complete_fixture_upgrade_before_open_success(&mut page_vm, &loader).await?;
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        let task = take_indexed_db_page_task_for_test(&mut page_vm);
        let body_effect = page_vm.vm_mut().apply_current_indexed_db_task_body(
            AuthorizedCurrentPageIndexedDbTask::new_for_executor_test(task),
        )?;
        assert_eq!(
            body_effect,
            IndexedDbTaskBodyEffect::Applied,
            "the exact IndexedDB task body should consume its realm-local payload"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__indexedDbTaskBodyBoundary.join('|')",
                )?,
            "upgrade|success",
            "the IndexedDB body must leave Promise reactions and transaction deactivation for selected-task completion"
        );
        assert!(
            !has_ready_runtime_script_continuation_for_test(&page_vm),
            "the IndexedDB body must not publish runtime follow-up before selected-task completion"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__indexedDbTaskBodyBoundary.join('|')")?,
            "upgrade|success|microtask-active",
            "the selected task checkpoint must run reactions while the newly-created transaction is still active"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
(() => {
  try {
    __indexedDbTaskBodyStore.get(1);
    return "still-active";
  } catch (error) {
    return error && error.name;
  }
})()
"#,
            )?,
            "TransactionInactiveError",
            "the checkpoint-end IndexedDB batch must deactivate the transaction before the next task"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "callback completion must publish the established typed runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB body/completion boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_current_ticket_without_realm_payload_owns_only_a_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url =
            Url::parse("https://example.com/indexed-db-missing-current-payload").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        schedule_open(
            &mut page_vm,
            "missing-current-payload",
            "__indexedDbMissingCurrentPayload",
        );
        admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;

        let task = take_indexed_db_page_task_for_test(&mut page_vm);
        assert_eq!(
            page_vm
                .vm_mut()
                .discard_stale_indexed_db_task(task.owner(), task.kind())?,
            IndexedDbStaleTaskCleanupEffect::RemovedRealmLocalPayload,
            "the fixture must remove only the task's realm-local payload",
        );
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let outcome = page_vm.apply_selected_page_indexed_db_task_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageIndexedDbTaskTargetEffect::CurrentOwnerHadNoPendingTask
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__indexedDbMissingCurrentPayload.join('|')")?,
            "",
            "a missing realm-local payload must not synthesize an IndexedDB callback"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "checkpoint-only reconciliation must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB missing-current-payload completion should remain bounded");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_selected_callback_completion_reconciles_a_created_child() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url =
            Url::parse("https://example.com/indexed-db-selected-child-follow-up").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        page_vm.vm_mut().eval(
            r#"
(() => {
  const open = indexedDB.open("selected-child-follow-up", 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore("kv");
    open.transaction.oncomplete = () => globalThis.__indexedDbFixtureUpgradeComplete = true;
  };
  open.onsuccess = () => {
    const frame = document.createElement("iframe");
    frame.id = "indexed-db-selected-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
    open.result.close();
  };
  return "scheduled";
})()
"#,
        )?;
        complete_fixture_upgrade_before_open_success(&mut page_vm, &loader).await?;
        assert!(
            !page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "upgrade completion must not execute the open-success listener"
        );
        assert!(
            run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
                .await?
                .is_some(),
            "one selected IndexedDB task should be ready"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "the selected callback completion must reconcile the child created by the IndexedDB listener"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB selected callback child follow-up should run");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_task_applies_real_producer_work_and_one_microtask_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url = Url::parse("https://example.com/indexed-db-owner-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        schedule_open(&mut page_vm, "current-owner", "__indexedDbOwnerTurn");
        admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;

        let selected = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("real producer task should consume one typed Page turn");
        assert!(matches!(
            selected.kind,
            RendererPageIndexedDbTaskKind::RuntimeQueue(_)
        ));

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__indexedDbOwnerTurn.join('|')")?,
            "upgrade|upgrade-microtask",
            "one authorized IDB turn must include its host-task microtask checkpoint"
        );
        run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("upgrade commit should consume a separate turn");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__indexedDbOwnerTurn.join('|')")?,
            "upgrade|upgrade-microtask",
            "commit must not dispatch open success in its own task"
        );
        run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("open success should consume a separate turn");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__indexedDbOwnerTurn.join('|')")?,
            "upgrade|upgrade-microtask|success|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB current-owner task should run through the PageVm executor");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_source_consumes_exactly_one_runtime_task_per_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url = Url::parse("https://example.com/indexed-db-one-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        schedule_open(&mut page_vm, "first", "__firstIndexedDbTurn");
        schedule_open(&mut page_vm, "second", "__secondIndexedDbTurn");
        admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;

        let first = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("first IDB task should consume one turn");

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify([__firstIndexedDbTurn, __secondIndexedDbTurn])")?,
            r#"[["upgrade","upgrade-microtask"],[]]"#
        );

        let second = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("second IDB task should consume the next turn");
        assert_eq!(second.owner, first.owner);
        assert_ne!(second.kind, first.kind);

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify([__firstIndexedDbTurn, __secondIndexedDbTurn])")?,
            r#"[["upgrade","upgrade-microtask"],["upgrade","upgrade-microtask"]]"#
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB runtime tasks should obey the one-turn contract");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_task_survives_document_open_in_the_same_window_realm() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url = Url::parse("https://example.com/indexed-db-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        schedule_open(&mut page_vm, "document-open", "__documentOpenIndexedDbTurn");
        admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); \
             document.close(); 'replaced'",
        )?;

        run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("same-Window IDB task should remain runnable after document.open()");

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__documentOpenIndexedDbTurn.join('|')")?,
            "upgrade|upgrade-microtask",
            "document.open() must not retire work owned by its preserved Window realm"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB task should survive same-Window document.open()");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_rejects_a_replaced_child_realm_without_stealing_its_task() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
        let document_url = Url::parse("https://example.com/indexed-db-realm-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        install_indexed_db_manager(&mut page_vm, &manager);
        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'indexed-db-realm-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("indexed-db-realm-replacement")
            .expect("realm replacement fixture should install a child handle");
        let retired_execution_context_id =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "indexed-db-realm-replacement",
            )?;
        assert_eq!(
            page_vm.vm_mut().eval_in_child_default_context(
                retired_execution_context_id,
                "String(globalThis === top)",
            )?,
            "false",
            "the producer fixture must execute inside the child default realm"
        );
        assert_eq!(
            page_vm.vm_mut().eval_in_child_default_context(
                retired_execution_context_id,
                "String(indexedDB === top.indexedDB)",
            )?,
            "false",
            "each Window realm must expose its own IDBFactory wrapper"
        );
        schedule_child_open(
            &mut page_vm,
            retired_execution_context_id,
            "retired-child-realm",
            "__retiredRealmIndexedDbTurn",
        );
        admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        let current_execution_context_id =
            materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm,
                "indexed-db-realm-replacement",
            )?;
        schedule_child_open(
            &mut page_vm,
            current_execution_context_id,
            "replacement-child-realm",
            "__replacementRealmIndexedDbTurn",
        );

        let stale = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("retired child-realm task should consume one stale discard turn");

        assert_eq!(
            page_vm.vm_mut().eval_in_child_default_context(
                current_execution_context_id,
                "globalThis.__replacementRealmIndexedDbTurn.join('|')",
            )?,
            "",
            "discarding the retired realm must not consume the replacement realm's local task"
        );

        admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;
        let current = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
            .await?
            .expect("replacement child-realm task should consume the next turn");
        assert_eq!(
            stale.owner.root_document(),
            current.owner.root_document(),
            "child realm replacement remains inside one root Document namespace"
        );
        assert_eq!(
            stale.owner.execution_context().owner(),
            current.owner.execution_context().owner()
        );
        assert_ne!(
            stale.owner.execution_context(),
            current.owner.execution_context(),
            "realm replacement must change the exact execution-context identity"
        );
        assert_eq!(
            current.kind, stale.kind,
            "fresh realm-local IDB state should naturally reproduce the old task id"
        );
        assert_eq!(
            page_vm.vm_mut().eval_in_child_default_context(
                current_execution_context_id,
                "globalThis.__replacementRealmIndexedDbTurn.join('|')",
            )?,
            "upgrade|upgrade-microtask",
            "the central Agent checkpoint must drain reactions queued by the exact child realm"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("IndexedDB child-realm replacement should use exact realm ownership");
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_retired_connection_head_releases_the_next_live_realm() {
    run_page_vm_async_test(async move {
        // Cancel before admission, before upgradeneeded, with migration work
        // queued, and after commit but before open.success. No sleeps: each
        // boundary is a selected task or an observed transaction event.
        for phase in 0..4 {
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
            let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader, Url::parse("https://example.com/retired-connection-head")?,
                );
            install_indexed_db_manager(&mut page_vm, &manager);
            page_vm.vm_mut().eval(
                "const frame = document.createElement('iframe'); frame.id = 'idb-head'; document.body.append(frame);",
            )?;
            let handle = page_vm.vm().element_handle_by_id_for_test("idb-head").unwrap();
            let child_context = materialize_only_child_realm_execution_context_through_page_turn_for_test(
                &mut page_vm, "idb-head",
            )?;
            page_vm.vm_mut().eval("globalThis.__headEvents = []; globalThis.__survivor = 'pending';")?;
            page_vm.vm_mut().eval_in_child_default_context(child_context, r#"
                const head = indexedDB.open('retired-head', 1);
                head.onupgradeneeded = () => {
                    parent.__headEvents.push('upgrade');
                    head.result.createObjectStore('child').put('committed', 1);
                    head.transaction.oncomplete = () => parent.__headEvents.push('complete');
                };
                head.onsuccess = () => parent.__headEvents.push('unexpected-success');
                head.onerror = () => parent.__headEvents.push('unexpected-error');
            "#)?;
            // Accept this in the top realm before retiring the child; it must
            // share the child's FIFO instead of racing another initial upgrade.
            page_vm.vm_mut().eval(r#"
                const tail = indexedDB.open('retired-head');
                tail.onupgradeneeded = () => tail.result.createObjectStore('parent');
                tail.onsuccess = () => {
                    __survivor = Array.from(tail.result.objectStoreNames).join(',');
                    tail.result.close();
                };
                tail.onerror = () => __survivor = 'error:' + tail.error.name;
            "#)?;
            if phase >= 1 {
                admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;
            }
            if phase >= 2 {
                let upgrade = run_selected_indexed_db_task_for_test(&mut page_vm, &loader).await?.unwrap();
                assert!(matches!(upgrade.kind, RendererPageIndexedDbTaskKind::RuntimeQueue(_)));
                assert_eq!(page_vm.vm_mut().eval("__headEvents.join('|')")?, "upgrade");
            }
            if phase == 3 {
                for _ in 0..8 {
                    if page_vm.vm_mut().eval("__headEvents.includes('complete')")? == "true" {
                        break;
                    }
                    run_selected_indexed_db_task_for_test(&mut page_vm, &loader).await?
                        .expect("upgrade requests must progress to a commit event");
                }
                assert_eq!(page_vm.vm_mut().eval("__headEvents.join('|')")?, "upgrade|complete");
            }
            let retired_events = page_vm.vm_mut().eval("__headEvents.join('|')")?;
            page_vm.vm_mut().retire_child_frame_realm_for_test(handle);
            for _ in 0..16 {
                if page_vm.vm_mut().eval("__survivor !== 'pending'")? == "true" {
                    break;
                }
                run_selected_indexed_db_task_for_test(&mut page_vm, &loader).await?
                    .expect("retirement must wake the next live owner without a close() callback");
            }
            assert_eq!(page_vm.vm_mut().eval("__survivor")?, if phase == 3 { "child" } else { "parent" },
                "phase {phase}: only a committed upgrade may survive retirement");
            assert_eq!(page_vm.vm_mut().eval("__headEvents.join('|')")?, retired_events,
                "phase {phase}: stale tasks must not dispatch the retired head's callbacks");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("retiring a connection head must preserve FIFO progress in another realm");
}

#[test]
fn indexed_db_rejects_a_real_page_vm_replacement_identity_collision() {
    run_page_vm_large_stack_async_test(
        "indexed-db-real-page-vm-replacement-collision",
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
            let manager = crate::new_indexed_db_manager(None).expect("IndexedDB manager");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            install_indexed_db_manager(&mut page_vm, &manager);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    schedule_open(
                        &mut page_vm,
                        "retired-page-vm",
                        "__retiredIndexedDbTurn",
                    );
                    admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;
                    let retired_root = page_vm.document_lifecycle.identity().document;

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
                    schedule_open(
                        &mut page_vm,
                        "replacement-page-vm",
                        "__replacementIndexedDbTurn",
                    );

                    let stale = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
                        .await?
                        .expect("retired PageVm task should consume one stale discard turn");
                    assert_eq!(stale.owner.root_document(), retired_root);

                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("globalThis.__replacementIndexedDbTurn.join('|')")?,
                        "",
                        "discarding the old root namespace must not steal the colliding local task"
                    );

                    admit_indexed_db_connections_for_test(&mut page_vm, &loader).await?;
                    let current = run_selected_indexed_db_task_for_test(&mut page_vm, &loader)
                        .await?
                        .expect("replacement producer task should consume the next turn");
                    assert_ne!(stale.owner, current.owner);
                    assert_eq!(
                        stale.owner.execution_context(),
                        current.owner.execution_context(),
                        "fresh PageVm counters should naturally reuse the top Window/realm identity"
                    );
                    assert_eq!(current.owner.root_document(), current_root);
                    assert_eq!(
                        current.kind, stale.kind,
                        "new PageVm counters should naturally reproduce the old local task id"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("globalThis.__replacementIndexedDbTurn.join('|')")?,
                        "upgrade|upgrade-microtask"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("IndexedDB PageVm replacement should run through the task executor");
            server
                .await
                .expect("IndexedDB PageVm replacement server should finish");
        },
    );
}
