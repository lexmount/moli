use super::*;

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_blob_exports_snapshots_in_windows_and_workers() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut()
            .eval("document.body.innerHTML = '<iframe id=child></iframe>';")?;
        page.vm_mut().eval(&format!(
            "globalThis.__offscreenBlobChecks = {};",
            include_str!("../../../../tests/fixtures/canvas-offscreen-blob-checks.js"),
        ))?;
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/canvas-offscreen-blob.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            for selector in [
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
                PageSelectedTaskTestSelector::BitmapTask,
                PageSelectedTaskTestSelector::WorkerHostBridge,
                PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "OffscreenCanvas export fixture did not finish"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let result: serde_json::Value =
            serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["passed"] != true)
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(
            result["total"], 488,
            "all realms and worker checks must run"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("OffscreenCanvas serialization and Blob clone fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_blob_retires_with_its_removed_window() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resources, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/offscreen-blob-retired")?,
        );
        page.vm_mut().eval("document.body.innerHTML = '<iframe id=canvas-frame></iframe>'; globalThis.__retiredExportCalls = 0;")?;
        materialize_only_child_realm_execution_context_through_page_turn_for_test(&mut page, "canvas-frame")?;
        page.vm_mut().eval("new (document.getElementById('canvas-frame').contentWindow.OffscreenCanvas)(1, 1).convertToBlob().then(() => __retiredExportCalls++, () => __retiredExportCalls++);")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let claimed = loop {
            if let Some(claimed) = page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
            ) {
                break claimed;
            }
            assert!(Instant::now() < deadline, "encoder result did not become ready");
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        page.vm_mut().eval("document.getElementById('canvas-frame').remove();")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("globalThis.__retiredExportCheckpoint = 0; queueMicrotask(() => __retiredExportCheckpoint++);")?;
        page.run_claimed_selected_page_task_for_test(claimed, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("`${__retiredExportCalls}:${__retiredExportCheckpoint}`")?, "0:0");
        Ok::<_, anyhow::Error>(())
    }).await.expect("retired OffscreenCanvas Window must not settle a parent reaction");
}

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_blob_preserves_window_ownership_across_document_open() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resources, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/offscreen-blob-document")?,
        );
        page.vm_mut().eval("globalThis.__oldExportCalls = 0; new OffscreenCanvas(1, 1).convertToBlob().then(blob => { if (blob instanceof Blob && blob.type === 'image/png' && blob.size > 0) __oldExportCalls++; });")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let claimed = loop {
            if let Some(claimed) = page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
            ) {
                break claimed;
            }
            assert!(Instant::now() < deadline, "encoder result did not become ready");
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        page.vm_mut().eval("document.open(); document.write('<body>replacement</body>'); document.close();")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("globalThis.__replacementExportCheckpoint = 0; queueMicrotask(() => __replacementExportCheckpoint++);")?;
        page.run_claimed_selected_page_task_for_test(claimed, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("`${__oldExportCalls}:${__replacementExportCheckpoint}`")?, "1:1");
        Ok::<_, anyhow::Error>(())
    }).await.expect("document.open preserves OffscreenCanvas Window and promise task");
}

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_blob_accepts_registered_native_proxies_before_conversion() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<iframe id=child></iframe>';
            globalThis.nativeOffscreen = new OffscreenCanvas(2, 1);
            const context = nativeOffscreen.getContext('2d');
            context.fillStyle = '#00ff00';
            context.fillRect(0, 0, 2, 1);
        "#)?;
        page.vm_mut().register_offscreen_canvas_proxy_for_test()?;
        page.vm_mut().eval(r#"
            globalThis.__proxyExportResult = null;
            (async () => {
                const other = document.getElementById('child').contentWindow;
                const exportBlob = other.OffscreenCanvas.prototype.convertToBlob;
                let conversions = 0;
                const options = { get type() { conversions++; return 'image/png'; } };
                const pending = exportBlob.call(offscreenProxy, options);
                if (!(pending instanceof other.Promise) || conversions !== 1) throw Error('native proxy conversion');
                let caught;
                try { await exportBlob.call(new Proxy(offscreenProxy, {}), options); } catch(error) { caught = error; }
                if (!(caught instanceof other.TypeError) || conversions !== 1) throw Error('author proxy must fail before conversion');
                const blob = await pending;
                if (!(blob instanceof Blob) || blob instanceof other.Blob) throw Error('canvas relevant Blob realm');
                const bitmap = await createImageBitmap(blob);
                const read = new OffscreenCanvas(2, 1).getContext('2d');
                read.drawImage(bitmap, 0, 0);
                if (bitmap.width !== 2 || JSON.stringify(Array.from(read.getImageData(0, 0, 1, 1).data)) !== '[0,255,0,255]') throw Error('native proxy snapshot');
                bitmap.close();
                __proxyExportResult = true;
            })().catch(error => { __proxyExportResult = String(error); });
        "#)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.vm_mut().eval("__proxyExportResult === null")? == "true" {
            for selector in [
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
                PageSelectedTaskTestSelector::BitmapTask,
            ] {
                let _ = page.run_exact_selected_page_task_for_test(selector, &loader).await?;
            }
            anyhow::ensure!(Instant::now() < deadline, "native proxy export did not finish");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(page.vm_mut().eval("__proxyExportResult")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.expect("native OffscreenCanvas proxy must preserve identity and pixels");
}
