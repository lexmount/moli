//! P5 task-end contracts for child classic-script source starts.
//!
//! The body only transfers one exact parser request to the owner network
//! bridge, or publishes a typed parser failure successor when admission cannot
//! start. It never evaluates the script or dispatches an event. Moli
//! exposes this owner/network handoff as a selected Page task, so only the
//! production dispatcher may submit its ordinary task-end checkpoint.

use super::*;

fn queue_child_classic_source_load(
    page_vm: &mut PageVm,
    element_id: &str,
    script_url: &str,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
globalThis.__childClassicSourceBodyRuns = 0;
const frame = document.createElement("iframe");
frame.id = {element_id:?};
frame.srcdoc = `<script src={script_url:?}><\/script>`;
document.body.appendChild(frame);
"queued"
"#,
    ))?;
    Ok(())
}

async fn advance_to_child_classic_source_load(page_vm: &mut PageVm) -> anyhow::Result<()> {
    anyhow::ensure!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildNavigationCommit
            )
            .await?,
        "child srcdoc must publish one exact navigation commit",
    );
    anyhow::ensure!(
        page_vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
        ),
        "the committed child Document must expose its exact classic source-start task",
    );
    Ok(())
}

async fn await_classic_response_server(server: JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("classic source request should reach the test server")
        .expect("classic source response server should finish");
}

#[tokio::test(flavor = "current_thread")]
async fn child_classic_source_load_body_does_not_checkpoint_or_evaluate_script() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/body-only-classic.js",
            "HTTP/1.1 200 OK",
            "parent.__childClassicSourceBodyRuns += 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_classic_source_load(
            &mut page_vm,
            "classic-source-body-child",
            &format!("{base_url}/body-only-classic.js"),
        )?;
        advance_to_child_classic_source_load(&mut page_vm).await?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__childClassicSourceCheckpoint = 0;
Promise.resolve().then(() => __childClassicSourceCheckpoint += 1);
"reaction queued"
"#,
            )?;

        let outcome = page_vm
            .run_child_classic_source_load_body_for_test()?
            .expect("one exact child classic source-start body should be ready");
        assert_eq!(
            outcome.action.target_effect,
            crate::page_task_queue::PageChildClassicScriptSourceLoadTargetEffect::NetworkRequestStartedForCurrentOwner,
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__childClassicSourceCheckpoint)",
            )?,
            "0",
            "network admission body must not discharge the selected task's checkpoint",
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "String(globalThis.__childClassicSourceBodyRuns)",
            )?,
            "0",
            "network admission must not execute the fetched script inline",
        );
        await_classic_response_server(server).await;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child classic source-start body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_classic_source_load_submits_task_end_checkpoint() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/selected-classic.js",
            "HTTP/1.1 200 OK",
            "parent.__childClassicSourceBodyRuns += 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_classic_source_load(
            &mut page_vm,
            "selected-classic-source-child",
            &format!("{base_url}/selected-classic.js"),
        )?;
        advance_to_child_classic_source_load(&mut page_vm).await?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__selectedChildClassicSourceCheckpoint = 0;
Promise.resolve().then(() => __selectedChildClassicSourceCheckpoint += 1);
"reaction queued"
"#,
            )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildClassicScriptSourceLoad
                )
                .await?,
            "one exact classic source-start task must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedChildClassicSourceCheckpoint)",
                )?,
            "1",
            "the production dispatcher must submit the state-only source-start checkpoint",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__childClassicSourceBodyRuns)",
                )?,
            "0",
            "task completion must not synchronously consume the later network terminal",
        );
        await_classic_response_server(server).await;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child classic source-start completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_classic_source_load_does_not_checkpoint_current_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.test/stale-child-classic-source-completion").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_child_classic_source_load(
            &mut page_vm,
            "stale-classic-source-child",
            "/retired-classic.js",
        )?;
        advance_to_child_classic_source_load(&mut page_vm).await?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildClassicScriptSourceLoad,
            )
            .expect("the old child Document must retain one opaque source-start claim");

        page_vm.vm_mut().eval(
            r#"
document.getElementById("stale-classic-source-child").srcdoc =
  "<!doctype html><body>replacement</body>";
"replacement queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildNavigationCommit
                )
                .await?,
            "replacement navigation must install a new exact child Document",
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleChildClassicSourceCheckpoint = 0;
Promise.resolve().then(() => __staleChildClassicSourceCheckpoint += 1);
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
                    "String(globalThis.__staleChildClassicSourceCheckpoint)",
                )?,
            "0",
            "a stale child source-start claim must not checkpoint the current Document",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child classic source-start completion witness should run");
}
