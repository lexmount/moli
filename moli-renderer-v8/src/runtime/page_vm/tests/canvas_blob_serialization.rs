use super::*;

#[tokio::test(flavor = "current_thread")]
async fn canvas_to_blob_retires_the_removed_canvas_window() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resources, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.com/canvas-blob-retired")?,
        );
        page.vm_mut().eval("document.body.innerHTML = '<iframe id=canvas-frame></iframe>'; globalThis.__retiredCanvasCalls = 0;")?;
        materialize_only_child_realm_execution_context_through_page_turn_for_test(&mut page, "canvas-frame")?;
        page.vm_mut().eval("document.getElementById('canvas-frame').contentWindow.document.createElement('canvas').toBlob(() => __retiredCanvasCalls++);")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let claimed = loop {
            if let Some(claimed) = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::CanvasBlobSerialization) { break claimed; }
            assert!(std::time::Instant::now() < deadline, "encoder result did not become ready");
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        };
        page.vm_mut().eval("document.getElementById('canvas-frame').remove();")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("globalThis.__retiredCanvasCheckpoint = 0; queueMicrotask(() => __retiredCanvasCheckpoint++);")?;
        page.run_claimed_selected_page_task_for_test(claimed, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("`${__retiredCanvasCalls}:${__retiredCanvasCheckpoint}`")?, "0:0");
        Ok::<_, anyhow::Error>(())
    }).await.expect("a retired canvas Window must not invoke a live parent callback");
}

#[tokio::test(flavor = "current_thread")]
async fn canvas_to_blob_exports_snapshots_through_its_own_task_source() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resources, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/canvas-to-blob")?,
        );
        page.vm_mut()
            .eval("document.body.innerHTML = '<iframe id=child></iframe>';")?;
        page.vm_mut().eval(include_str!(
            "../../../script_vm/tests/browser_api/canvas_to_blob.js"
        ))?;
        assert_eq!(
            page.vm_mut().eval("typeof __canvasToBlobResults")?,
            "undefined",
            "callbacks must not finish inside the invoking script or its microtasks"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            for selector in [
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
                PageSelectedTaskTestSelector::BitmapTask,
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            if page.vm_mut().eval("typeof __canvasToBlobResults")? != "undefined" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "canvas callback fixture did not finish"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        let result: serde_json::Value = serde_json::from_str(
            &page
                .vm_mut()
                .eval("JSON.stringify(__canvasToBlobResults)")?,
        )?;
        assert_eq!(result["passed"], result["total"], "{result}");
        assert_eq!(result["total"], 49, "the complete export fixture must run");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("canvas toBlob fixture should pass");
}

#[tokio::test(flavor = "current_thread")]
async fn canvas_to_blob_reports_callback_errors_and_completes_the_task() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resources, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/canvas-blob-errors")?,
        );
        page.vm_mut().eval(r#"
globalThis.__canvasMarks = [];
window.onerror = (message) => { __canvasMarks.push(message.includes('canvas-marker') ? 'reported' : 'other'); return true; };
document.createElement('canvas').toBlob(() => {
  __canvasMarks.push('callback');
  queueMicrotask(() => __canvasMarks.push('checkpoint'));
  throw new Error('canvas-marker');
});
"#)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !page.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::CanvasBlobSerialization, &loader,
        ).await? {
            assert!(std::time::Instant::now() < deadline, "encoder result did not become ready");
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert_eq!(page.vm_mut().eval("__canvasMarks.join(',')")?, "callback,reported,checkpoint");
        Ok::<_, anyhow::Error>(())
    }).await.expect("toBlob callback errors should be reported with a task checkpoint");
}

#[tokio::test(flavor = "current_thread")]
async fn canvas_to_blob_preserves_the_window_callback_across_document_open() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resources, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/canvas-blob-document")?,
        );
        page.vm_mut().eval(r#"
globalThis.__canvasOldCalls = 0;
document.createElement('canvas').toBlob(() => { __canvasOldCalls++; });
"#)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let claimed = loop {
            if let Some(claimed) = page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
            ) {
                break claimed;
            }
            assert!(std::time::Instant::now() < deadline, "encoder result did not become ready");
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        };
        page.vm_mut().eval("document.open(); document.write('<body>replacement</body>'); document.close();")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test(
            "globalThis.__canvasReplacementCheckpoint = 0; queueMicrotask(() => __canvasReplacementCheckpoint++);",
        )?;
        page.run_claimed_selected_page_task_for_test(claimed, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test(
            "`${__canvasOldCalls}:${__canvasReplacementCheckpoint}`",
        )?, "1:1", "document.open preserves the relevant Window and its canvas callback task");
        Ok::<_, anyhow::Error>(())
    }).await.expect("canvas blob task must retain exact Window ownership");
}
