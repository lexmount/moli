use super::*;

#[tokio::test(flavor = "current_thread")]
async fn bitmap_renderer_transfers_real_pixels_and_exports_snapshots_across_realms() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut()
            .eval("document.body.innerHTML = '<iframe id=child></iframe>';")?;
        page.vm_mut().eval(&format!(
            "globalThis.__bitmapRendererSurface = {};",
            include_str!("../../../../tests/fixtures/canvas-bitmap-renderer-surface.js"),
        ))?;
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/canvas-bitmap-renderer.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            for selector in [
                PageSelectedTaskTestSelector::BitmapTask,
                PageSelectedTaskTestSelector::CanvasBlobSerialization,
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "bitmap renderer fixture did not finish"
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
            result["total"], 340,
            "all realm and canvas combinations must run"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("bitmap renderer ownership and export fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn bitmap_renderer_transfers_registered_native_bitmap_proxies_and_shares_detachment() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<iframe id=child></iframe>';
            globalThis.nativeRenderer = new OffscreenCanvas(3, 2).getContext('bitmaprenderer');
            createImageBitmap(new ImageData(new Uint8ClampedArray([255, 0, 0, 255]), 1, 1)).then(bitmap => globalThis.nativeBitmap = bitmap);
        "#)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.vm_mut().eval("typeof nativeBitmap")? == "undefined" {
            let _ = page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::BitmapTask, &loader).await?;
            anyhow::ensure!(Instant::now() < deadline, "native bitmap was not decoded");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        page.vm_mut().register_bitmap_renderer_proxies_for_test()?;
        assert_eq!(page.vm_mut().eval(r#"(() => {
            const other = document.getElementById('child').contentWindow;
            const get = Object.getOwnPropertyDescriptor(other.ImageBitmapRenderingContext.prototype, 'canvas').get;
            const transfer = other.ImageBitmapRenderingContext.prototype.transferFromImageBitmap;
            const width = Object.getOwnPropertyDescriptor(other.ImageBitmap.prototype, 'width').get;
            if (get.call(rendererProxy) !== nativeRenderer.canvas || width.call(bitmapProxy) !== 1) throw Error('native receiver identity');
            let traps = 0, caught;
            try { transfer.call(rendererProxy, new Proxy(bitmapProxy, {getPrototypeOf() { traps++; }})); } catch(error) { caught = error; }
            if (!(caught instanceof other.TypeError) || traps !== 0 || nativeBitmap.width !== 1) throw Error('author argument proxy');
            transfer.call(rendererProxy, bitmapProxy);
            if (nativeBitmap.width !== 0 || width.call(bitmapProxy) !== 0) throw Error('shared source detachment');
            const read = new OffscreenCanvas(1, 1).getContext('2d');
            read.drawImage(get.call(rendererProxy), 0, 0);
            if (JSON.stringify(Array.from(read.getImageData(0, 0, 1, 1).data)) !== '[255,0,0,255]') throw Error('native bitmap data');
            other.ImageBitmap.prototype.close.call(bitmapProxy);
            transfer.call(rendererProxy, null);
            return true;
        })()"#)?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.expect("registered native context and ImageBitmap proxy transfer");
}
