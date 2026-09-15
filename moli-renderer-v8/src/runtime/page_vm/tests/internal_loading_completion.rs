//! P5 task-end contracts for declarative-refresh internal-loading tasks.
//!
//! WHATWG requires a declarative refresh to become due no earlier than the
//! Document's completely-loaded time. Chromium then posts a cancellable
//! `kInternalLoading` task after `LoadEventFinished()` and relies on the main
//! scheduler's ordinary task-end checkpoint. A detached Document cancels its
//! task instead of checkpointing a replacement realm.

use super::*;

async fn loaded_page_with_ready_meta_refresh(path: &str) -> PageVm {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
document.head.innerHTML = '<meta http-equiv="refresh" content="0;{path}">';
'ready'
"#,
        ))
        .expect("install declarative refresh");
    let local_executor = page_vm.local_executor.clone();

    local_executor
        .run(async move {
            let outcome = page_vm
                .finish_post_parse_execution_on_named_owner_lane(
                    Vec::new(),
                    PageVmInitStage::Load,
                    Instant::now(),
                )
                .await?;
            let PageVmNavigationTurnOutcome::Completed(page_vm) = outcome else {
                anyhow::bail!("load must complete before a zero-delay meta refresh is published")
            };
            assert!(
                !page_vm.vm().has_pending_location_navigation(),
                "load completion must publish rather than execute the internal-loading task"
            );
            Ok(*page_vm)
        })
        .await
        .expect("post-parse load completion should succeed")
}

async fn loaded_page_with_delayed_meta_refresh(path: &str, delay_seconds: &str) -> PageVm {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
document.head.innerHTML = '<meta http-equiv="refresh" content="{delay_seconds};{path}">';
'ready'
"#,
        ))
        .expect("install delayed declarative refresh");
    let local_executor = page_vm.local_executor.clone();

    local_executor
        .run(async move {
            let outcome = page_vm
                .finish_post_parse_execution_on_named_owner_lane(
                    Vec::new(),
                    PageVmInitStage::Load,
                    Instant::now(),
                )
                .await?;
            let PageVmNavigationTurnOutcome::Completed(page_vm) = outcome else {
                anyhow::bail!("load must complete before a delayed meta refresh is scheduled")
            };
            assert!(
                !page_vm.vm().has_pending_location_navigation(),
                "load completion must not activate a delayed meta refresh"
            );
            Ok(*page_vm)
        })
        .await
        .expect("post-parse delayed load completion should succeed")
}

async fn loaded_page_without_meta_refresh() -> PageVm {
    let page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();
    local_executor
        .run(async move {
            let outcome = page_vm
                .finish_post_parse_execution_on_named_owner_lane(
                    Vec::new(),
                    PageVmInitStage::Load,
                    Instant::now(),
                )
                .await?;
            let PageVmNavigationTurnOutcome::Completed(page_vm) = outcome else {
                anyhow::bail!("plain page load must complete")
            };
            Ok(*page_vm)
        })
        .await
        .expect("plain post-parse load completion should succeed")
}

fn queue_checkpoint_marker(page_vm: &mut PageVm, marker: &str) {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!(
            r#"
globalThis.{marker} = 0;
Promise.resolve().then(() => globalThis.{marker} += 1);
"reaction queued"
"#,
        ))
        .expect("queue checkpoint marker");
}

fn checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> String {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!("String(globalThis.{marker})"))
        .expect("read checkpoint marker")
}

#[test]
fn internal_loading_body_activates_refresh_without_checkpointing() {
    run_page_vm_large_stack_async_test("internal-loading-body-completion", || async move {
        let mut page_vm = loaded_page_with_ready_meta_refresh("body.html").await;
        queue_checkpoint_marker(&mut page_vm, "__internalLoadingBodyCheckpoint");

        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("load completion should publish one internal-loading body");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            },
        );
        assert_eq!(
            checkpoint_marker(&mut page_vm, "__internalLoadingBodyCheckpoint"),
            "0",
            "the internal-loading body must leave task-end checkpoint authority to the selected dispatcher",
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("the body should activate the declarative refresh navigation");
        assert_eq!(pending.url.as_str(), "https://example.com/body.html");
        assert_eq!(
            pending
                .entry_seed
                .as_ref()
                .and_then(|seed| seed.activation.as_ref())
                .and_then(|activation| activation.navigation_type.as_deref()),
            Some("replace"),
            "a refresh due within one second must replace the current entry"
        );
    });
}

#[test]
fn same_url_meta_refresh_uses_reload_navigation() {
    run_page_vm_large_stack_async_test("same-url-meta-refresh-reload", || async move {
        let mut page_vm = loaded_page_with_ready_meta_refresh("").await;
        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("load completion should publish the reload task");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            },
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("same-URL declarative refresh should activate a reload");
        assert_eq!(pending.url.as_str(), "https://example.com/");
        assert_eq!(
            pending.browser_navigation_kind,
            moli_fetch::BrowserNavigationRequestKind::Reload,
            "same-URL meta refresh must preserve browser reload semantics"
        );
    });
}

#[test]
fn fragment_meta_refresh_stays_in_the_current_document() {
    run_page_vm_large_stack_async_test("fragment-meta-refresh", || async move {
        let mut page_vm = loaded_page_with_ready_meta_refresh("#done").await;
        let original_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("test page should have a document owner");
        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("load completion should publish the fragment refresh task");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            },
        );
        assert!(
            !page_vm.vm().has_pending_location_navigation(),
            "a fragment refresh must not hand a cross-document request to the browser"
        );
        assert_eq!(
            page_vm.vm().current_main_document_task_owner(),
            Some(original_owner),
            "a fragment refresh must retain the exact Document"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("location.href")
                .expect("current URL should remain observable"),
            "https://example.com/#done"
        );
    });
}

#[test]
fn selected_internal_loading_task_checkpoints_without_draining_runtime_work() {
    run_page_vm_large_stack_async_test("selected-internal-loading-completion", || async move {
        let mut page_vm = loaded_page_with_ready_meta_refresh("selected.html").await;
        queue_checkpoint_marker(&mut page_vm, "__selectedInternalLoadingCheckpoint");
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::InternalLoading
                )
                .await
                .expect("selected internal-loading dispatcher should succeed"),
            "one exact internal-loading task must enter the production selected dispatcher",
        );
        assert_eq!(
            checkpoint_marker(&mut page_vm, "__selectedInternalLoadingCheckpoint"),
            "1",
            "a current internal-loading task must submit its ordinary task-end checkpoint",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "state-only internal-loading completion must not drain unrelated runtime work",
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("the selected task should activate the declarative refresh navigation");
        assert_eq!(pending.url.as_str(), "https://example.com/selected.html");
    });
}

#[test]
fn selected_internal_loading_task_blocked_by_competing_navigation_still_checkpoints() {
    run_page_vm_large_stack_async_test(
        "not-activated-internal-loading-completion",
        || async move {
            let mut page_vm = loaded_page_with_ready_meta_refresh("superseded.html").await;
            page_vm
                .vm_mut()
                .eval("location.href = 'competing.html'; 'competing navigation queued'")
                .expect("queue competing navigation");
            queue_checkpoint_marker(&mut page_vm, "__suppressedInternalLoadingCheckpoint");

            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::InternalLoading
                    )
                    .await
                    .expect("suppressed internal-loading dispatcher should succeed"),
                "the superseded internal-loading body remains one selected Page task",
            );
            assert_eq!(
                checkpoint_marker(&mut page_vm, "__suppressedInternalLoadingCheckpoint"),
                "1",
                "a selected no-op body still reaches the ordinary task-end checkpoint",
            );
            let pending = page_vm
                .vm_mut()
                .take_pending_location_navigation_with_seed()
                .expect("the competing navigation must remain authoritative");
            assert_eq!(pending.url.as_str(), "https://example.com/competing.html");
        },
    );
}

#[test]
fn stale_claimed_internal_loading_task_does_not_checkpoint_replacement_document() {
    run_page_vm_large_stack_async_test("stale-internal-loading-completion", || async move {
        let mut page_vm = loaded_page_with_ready_meta_refresh("stale.html").await;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::InternalLoading)
            .expect("the old Document should retain one opaque internal-loading claim");

        page_vm
            .vm_mut()
            .eval("document.open(); document.write('<p>replacement</p>'); document.close();")
            .expect("replace Document before the claimed task executes");
        queue_checkpoint_marker(&mut page_vm, "__staleInternalLoadingCheckpoint");
        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await
            .expect("stale internal-loading dispatcher should succeed");

        assert_eq!(
            checkpoint_marker(&mut page_vm, "__staleInternalLoadingCheckpoint"),
            "0",
            "a detached Document's task must not checkpoint its replacement realm",
        );
        assert!(
            !page_vm.vm().has_pending_location_navigation(),
            "a stale declarative refresh must not navigate the replacement Document",
        );
    });
}

#[test]
fn delayed_meta_refresh_becomes_one_internal_loading_turn_at_its_deadline() {
    run_page_vm_large_stack_async_test("delayed-meta-refresh-deadline", || async move {
        let mut page_vm = loaded_page_with_delayed_meta_refresh("delayed.html", "1").await;
        assert!(
            page_vm.run_internal_loading_body_for_test().is_none(),
            "a delayed refresh must not enter the internal-loading source at load completion"
        );
        let deadline = page_vm
            .next_internal_loading_deadline_for_test()
            .expect("delayed refresh must register an internal-loading deadline");
        assert_eq!(
            page_vm.vm().next_timeout_deadline(),
            None,
            "a declarative refresh must not register a Window/V8 timer"
        );

        super::super::wait_for_page_timer_deadline(Some(deadline)).await;
        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("the due source must expose one internal-loading body directly");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            },
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("the internal-loading body should activate delayed refresh navigation");
        assert_eq!(pending.url.as_str(), "https://example.com/delayed.html");
    });
}

#[test]
fn post_load_meta_insertion_schedules_the_document_owned_refresh_task() {
    run_page_vm_large_stack_async_test("dynamic-delayed-meta-refresh", || async move {
        let mut page_vm = loaded_page_without_meta_refresh().await;
        assert!(
            page_vm.next_internal_loading_deadline_for_test().is_none(),
            "a page without a refresh must not own an internal-loading deadline"
        );

        page_vm
            .vm_mut()
            .eval(
                r#"
const meta = document.createElement('meta');
meta.setAttribute('http-equiv', 'refresh');
meta.setAttribute('content', '60;must-not-win.html');
document.head.append(meta);
'scheduled'
"#,
            )
            .expect("insert a refresh meta after load");
        let original_deadline = page_vm
            .next_internal_loading_deadline_for_test()
            .expect("the connected insertion should post its first refresh candidate");
        page_vm
            .vm_mut()
            .eval(
                r#"
document.querySelector('meta[http-equiv="refresh"]')
    .setAttribute('content', '1;dynamic.html');
'rescheduled'
"#,
            )
            .expect("replace the connected meta refresh through an attribute mutation");
        let deadline = page_vm
            .next_internal_loading_deadline_for_test()
            .expect("the live Document scheduler should replace the inserted refresh");
        assert!(
            deadline < original_deadline,
            "a shorter attribute-mutation candidate must replace the posted task"
        );
        assert_eq!(
            page_vm.vm().next_timeout_deadline(),
            None,
            "dynamic declarative refresh must remain outside the Window timer heap"
        );

        super::super::wait_for_page_timer_deadline(Some(deadline)).await;
        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("the inserted refresh should become one due internal-loading task");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            },
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("the inserted refresh should activate navigation");
        assert_eq!(pending.url.as_str(), "https://example.com/dynamic.html");
    });
}

#[test]
fn equivalent_meta_mutation_preserves_the_original_refresh_deadline() {
    run_page_vm_large_stack_async_test("equivalent-meta-refresh-mutation", || async move {
        let mut page_vm = loaded_page_with_delayed_meta_refresh("stable.html", "60").await;
        let original_deadline = page_vm
            .next_internal_loading_deadline_for_test()
            .expect("load should arm the original delayed refresh");

        page_vm
            .vm_mut()
            .eval(
                r#"
document.querySelector('meta[http-equiv="refresh"]')
    .setAttribute('content', '60;./stable.html');
'rediscovered'
"#,
            )
            .expect("mutate the serialized content to the same parsed candidate");

        assert_eq!(
            page_vm.next_internal_loading_deadline_for_test(),
            Some(original_deadline),
            "rediscovering the same delay and resolved URL must not postpone the refresh"
        );
    });
}

#[test]
fn document_open_preserves_an_active_delayed_meta_refresh_deadline() {
    run_page_vm_large_stack_async_test("document-open-delayed-meta-refresh", || async move {
        let mut page_vm = loaded_page_with_delayed_meta_refresh("must-run.html", "1").await;
        let original_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("test page should have a document owner");
        let original_deadline = page_vm
            .next_internal_loading_deadline_for_test()
            .expect("the original Document must own a delayed refresh task");

        page_vm
            .vm_mut()
            .eval("document.open(); document.write('<p>replacement</p>'); document.close();")
            .expect("open a replacement parser before the refresh deadline");
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(original_owner),
            "document.open should rotate the exact Document owner"
        );
        assert_eq!(
            page_vm.next_internal_loading_deadline_for_test(),
            Some(original_deadline),
            "Blink keeps an already armed nonzero refresh at its original deadline"
        );
        super::super::wait_for_page_timer_deadline(Some(original_deadline)).await;
        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("the rebound refresh should become ready at its original deadline");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            },
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("the active refresh must navigate after document.open()");
        assert_eq!(pending.url.as_str(), "https://example.com/must-run.html");
    });
}
