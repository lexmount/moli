//! P5 task-end contracts for child navigation commits.
//!
//! A navigation commit mutates child-frame ownership and publishes typed
//! follow-up work, but it is not allowed to end the surrounding HTML task from
//! inside the ScriptVm body helper. The selected Page dispatcher owns that
//! ordinary checkpoint.

use super::*;
use crate::runtime::{IntoPageTaskCompletion, PageTaskCompletion};

fn queue_child_navigation_commit(page_vm: &mut PageVm, element_id: &str) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
const frame = document.createElement("iframe");
frame.id = {element_id:?};
frame.srcdoc = "<!doctype html><body>child</body>";
document.body.appendChild(frame);
"queued"
"#,
    ))?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn child_navigation_commit_body_leaves_microtask_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-navigation-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_navigation_commit(&mut page_vm, "body-boundary-child")?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__childNavigationBodyCheckpoint = 0;
Promise.resolve().then(() => __childNavigationBodyCheckpoint = 1);
"reaction queued"
"#,
            )?;

        let outcome = page_vm
            .run_child_navigation_commit_body_for_test()?
            .expect("one exact child navigation commit should be ready");
        assert!(matches!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner
        ));
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__childNavigationBodyCheckpoint)",
            )?,
            "0",
            "the child-navigation body must leave the ordinary checkpoint to the selected dispatcher",
        );
        assert!(matches!(
            outcome.action.into_page_task_completion(),
            PageTaskCompletion::CheckpointOnly,
        ));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child-navigation body boundary witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_navigation_commit_checkpoints_without_draining_runtime_work() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/selected-child-navigation-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_navigation_commit(&mut page_vm, "selected-completion-child")?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__selectedChildNavigationCheckpoint = 0;
Promise.resolve().then(() => __selectedChildNavigationCheckpoint = 1);
"reaction queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildNavigationCommit
                )
                .await?,
            "one exact child-navigation task must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedChildNavigationCheckpoint)",
                )?,
            "1",
            "the selected child-navigation task must submit its ordinary task-end checkpoint",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "state-only checkpoint completion must not synchronously drain unrelated runtime work",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child-navigation completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_navigation_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/stale-child-navigation-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_navigation_commit(&mut page_vm, "retired-navigation-child")?;
        let retired_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner");
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildNavigationCommit,
            )
            .expect("the old Document must retain one opaque navigation claim");

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
globalThis.__staleChildNavigationCheckpoint = 0;
Promise.resolve().then(() => __staleChildNavigationCheckpoint = 1);
"replacement reaction queued"
"#,
            )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleChildNavigationCheckpoint)",
                )?,
            "0",
            "a stale child-navigation claim must not enter the replacement realm for a checkpoint",
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
    .expect("stale child-navigation completion witness should run");
}
