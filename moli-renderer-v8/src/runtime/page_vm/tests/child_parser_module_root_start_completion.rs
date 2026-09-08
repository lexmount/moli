//! P5 task-end contracts for child parser-module graph starts.
//!
//! The body may register or compile an inline root, join an existing module-map
//! entry, start an external fetch, or publish a typed failure successor. It
//! never evaluates the module or dispatches a callback. Moli exposes this
//! owner/network handoff as a selected Page task, so only the production
//! dispatcher may submit its ordinary task-end checkpoint.

use super::*;

fn queue_inline_child_parser_module_root(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
globalThis.__childParserRootModuleBodyRuns = 0;
const frame = document.createElement("iframe");
frame.id = {element_id:?};
frame.srcdoc = `<script type="module">
  parent.__childParserRootModuleBodyRuns += 1;
<\/script>`;
document.body.appendChild(frame);
"queued"
"#,
    ))?;
    Ok(())
}

pub(super) async fn advance_to_child_parser_module_root(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildNavigationCommit,
                loader,
            )
            .await?,
        "child srcdoc must publish one exact navigation commit",
    );
    anyhow::ensure!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildRealmMaterialization,
                loader,
            )
            .await?,
        "the committed child Document must materialize its exact realm",
    );

    for _ in 0..4 {
        if page_vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::ParserModuleRootStart,
        ) {
            return Ok(());
        }
        anyhow::ensure!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentLifecycle,
                    loader,
                )
                .await?,
            "only bounded child lifecycle predecessors may stand before the parser-module root",
        );
    }
    anyhow::bail!("child parser-module root did not become the stable family head")
}

#[tokio::test(flavor = "current_thread")]
async fn child_parser_module_root_start_body_does_not_checkpoint_or_evaluate_module() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/child-parser-root-body-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_inline_child_parser_module_root(&mut page_vm, "parser-root-body-child")?;
        advance_to_child_parser_module_root(&mut page_vm, &loader).await?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__childParserRootCheckpoint = 0;
Promise.resolve().then(() => __childParserRootCheckpoint += 1);
"reaction queued"
"#,
            )?;

        let outcome = page_vm
            .run_child_parser_module_root_start_body_for_test()?
            .expect("one exact parser-module root body should be ready");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildParserModuleRootStartTargetEffect::ConsumedByCurrentOwner,
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__childParserRootCheckpoint)",
            )?,
            "0",
            "root graph admission must not discharge the selected task's checkpoint",
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__childParserRootModuleBodyRuns)",
            )?,
            "0",
            "root graph admission must publish later script work instead of evaluating the module inline",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child parser-module root body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_parser_module_root_start_submits_one_task_end_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/selected-child-parser-root-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_inline_child_parser_module_root(&mut page_vm, "selected-parser-root-child")?;
        advance_to_child_parser_module_root(&mut page_vm, &loader).await?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__selectedChildParserRootCheckpoint = 0;
Promise.resolve().then(() => __selectedChildParserRootCheckpoint += 1);
"reaction queued"
"#,
            )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildParserModuleRootStart,
                    &loader,
                )
                .await?,
            "one exact root-start task must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedChildParserRootCheckpoint)",
                )?,
            "1",
            "the production dispatcher must submit the state-only root-start task checkpoint",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__childParserRootModuleBodyRuns)",
                )?,
            "0",
            "task completion must not synchronously consume the published module-script successor",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child parser-module root completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_parser_module_root_does_not_checkpoint_current_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/stale-child-parser-root-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_inline_child_parser_module_root(&mut page_vm, "stale-parser-root-child")?;
        advance_to_child_parser_module_root(&mut page_vm, &loader).await?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildParserModuleRootStart,
            )
            .expect("the old child Document must retain one opaque root-start claim");

        page_vm.vm_mut().eval(
            r#"
document.getElementById("stale-parser-root-child").srcdoc =
  "<!doctype html><body>replacement</body>";
"replacement queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildNavigationCommit,
                    &loader,
                )
                .await?,
            "replacement navigation must install a new exact child Document",
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleChildParserRootCheckpoint = 0;
Promise.resolve().then(() => __staleChildParserRootCheckpoint += 1);
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
                    "String(globalThis.__staleChildParserRootCheckpoint)",
                )?,
            "0",
            "a stale child root-start claim must not checkpoint the current Document",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child parser-module root completion witness should run");
}
