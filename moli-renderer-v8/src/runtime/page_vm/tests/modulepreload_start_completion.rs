//! P5 task-end contracts for child modulepreload fetch starts.
//!
//! WHATWG starts the modulepreload graph from link processing and reports the
//! later result through a separate load/error event. Blink likewise starts
//! `FetchSingle()` from `ModulePreloadIfNeeded()` and relies on the enclosing
//! renderer task's ordinary task-end checkpoint. Moli adds an explicit
//! owner/network handoff: the start itself is independently scheduler-visible.
//! These tests keep that internal carrier's body, completion, and stale-owner
//! boundaries distinct without claiming that the carrier exists in HTML.

use super::*;

use anyhow::Context;

use crate::page_task_queue::PageModulepreloadStartDocumentEffect;

async fn queue_real_child_modulepreload_start(
    page_vm: &mut PageVm,
    base_url: &str,
    frame_id: &str,
) -> anyhow::Result<ChildDocumentModuleFetchTarget> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  frame.srcdoc = `<link rel="modulepreload" href="{base_url}/preload.js">`;
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(frame_id)
        .context("modulepreload-start fixture should retain its child handle")?;

    let mut startup_sources = Vec::new();
    for _ in 0..12 {
        if page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
            .has_ready_task()
        {
            break;
        }
        let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        else {
            break;
        };
        startup_sources.push(source);
    }
    anyhow::ensure!(
        page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
            .has_ready_task(),
        "real child link must publish one typed modulepreload start; observed {startup_sources:?}",
    );
    page_vm
        .vm()
        .current_child_document_module_fetch_target(child_handle)
        .context("modulepreload-start producer should retain an exact child realm")
}

fn queue_checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> anyhow::Result<()> {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!(
            r#"
globalThis.{marker} = 0;
Promise.resolve().then(() => globalThis.{marker} += 1);
"reaction queued"
"#,
        ))?;
    Ok(())
}

fn checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> anyhow::Result<String> {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!("String(globalThis.{marker})"))
}

#[tokio::test(flavor = "current_thread")]
async fn child_modulepreload_start_body_does_not_checkpoint_or_consume_terminal() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/preload.js",
            "HTTP/1.1 200 OK",
            "export const value = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_real_child_modulepreload_start(
            &mut page_vm,
            &base_url,
            "modulepreload-start-body-child",
        )
        .await?;
        queue_checkpoint_marker(&mut page_vm, "__modulepreloadStartBodyCheckpoint")?;

        let outcome = page_vm
            .run_page_modulepreload_start_body_for_test()
            .context("one exact child modulepreload-start body should be ready")?;
        assert!(matches!(
            outcome.action.document_effect,
            PageModulepreloadStartDocumentEffect::AppliedToCurrentOwner { outcome }
                if outcome.fetch_was_scheduled()
        ));
        assert_eq!(
            checkpoint_marker(&mut page_vm, "__modulepreloadStartBodyCheckpoint")?,
            "0",
            "modulepreload-start body must leave task-end checkpoint authority to the selected dispatcher",
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "starting the request must not synchronously consume its later Networking terminal",
        );
        server
            .await
            .expect("modulepreload response server should observe the started request");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child modulepreload-start body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_modulepreload_start_submits_checkpoint_without_runtime_drain() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/preload.js",
            "HTTP/1.1 200 OK",
            "export const value = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_real_child_modulepreload_start(
            &mut page_vm,
            &base_url,
            "selected-modulepreload-start-child",
        )
        .await?;
        queue_checkpoint_marker(&mut page_vm, "__selectedModulepreloadStartCheckpoint")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ModulepreloadStart
                )
                .await?,
            "one exact modulepreload-start task must enter the production selected dispatcher",
        );
        assert_eq!(
            checkpoint_marker(&mut page_vm, "__selectedModulepreloadStartCheckpoint")?,
            "1",
            "the current selected start must submit its ordinary task-end checkpoint",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "state-only start completion must not drain unrelated runtime residence",
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "task completion must not consume the later modulepreload terminal",
        );
        server
            .await
            .expect("modulepreload response server should observe the selected start");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child modulepreload-start witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_modulepreload_start_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/page").expect("page URL"),
            );
        queue_real_child_modulepreload_start(
            &mut page_vm,
            "https://stale-modulepreload.invalid",
            "stale-modulepreload-start-child",
        )
        .await?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ModulepreloadStart,
            )
            .context("the retired realm must retain one opaque modulepreload-start claim")?;

        page_vm
            .vm_mut()
            .eval("document.open(); document.write('<p>replacement</p>'); document.close();")?;
        queue_checkpoint_marker(&mut page_vm, "__staleModulepreloadStartCheckpoint")?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            checkpoint_marker(&mut page_vm, "__staleModulepreloadStartCheckpoint")?,
            "0",
            "a stale start claim must not checkpoint the replacement realm",
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "discarding a stale start must not synthesize a Network terminal",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child modulepreload-start completion witness should run");
}
