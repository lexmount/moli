use super::*;

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_transfer_moves_pixels_and_preserves_state_in_windows_and_workers() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut()
            .eval("document.body.innerHTML = '<iframe id=child></iframe>';")?;
        page.vm_mut().eval(&format!(
            "globalThis.__offscreenTransferChecks = {};",
            include_str!("../../../../tests/fixtures/canvas-offscreen-transfer-checks.js"),
        ))?;
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/canvas-offscreen-transfer.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            for selector in [
                PageSelectedTaskTestSelector::WorkerHostBridge,
                PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "OffscreenCanvas transfer fixture did not finish"
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
            result["total"], 147,
            "all realms and worker checks must run"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("OffscreenCanvas bitmap ownership, state and receiver fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_transfer_accepts_registered_native_proxies() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        page.vm_mut().eval(
            r#"
            document.body.innerHTML = '<iframe id=child></iframe>';
            globalThis.nativeOffscreen = new OffscreenCanvas(2, 1);
            const context = nativeOffscreen.getContext('2d');
            context.fillStyle = '#00ff00'; context.fillRect(0, 0, 2, 1);
            "#,
        )?;
        page.vm_mut().register_offscreen_canvas_proxy_for_test()?;
        assert_eq!(
            page.vm_mut().eval(
                r#"(() => {
                const other = document.getElementById('child').contentWindow;
                const transfer = other.OffscreenCanvas.prototype.transferToImageBitmap;
                let traps = 0;
                const trap = () => { traps++; throw Error('author trap'); };
                const snapshot = transfer.call(offscreenProxy, new Proxy({}, {get: trap}));
                if (Object.getPrototypeOf(snapshot) !== ImageBitmap.prototype || snapshot.width !== 2) throw Error('native receiver identity and realm');
                const revoked = Proxy.revocable(offscreenProxy, {}); revoked.revoke();
                for (const receiver of [new Proxy(offscreenProxy, {get: trap, getPrototypeOf: trap}), revoked.proxy, Object.create(offscreenProxy)]) {
                    let caught;
                    try { transfer.call(receiver); } catch (error) { caught = error; }
                    if (!(caught instanceof other.TypeError)) throw Error('author receiver accepted');
                }
                const context = nativeOffscreen.getContext('2d');
                context.fillStyle = '#0000ff'; context.fillRect(0, 0, 2, 1);
                const next = transfer.call(offscreenProxy);
                const read = new OffscreenCanvas(2, 1).getContext('2d');
                read.drawImage(snapshot, 0, 0);
                if (Array.from(read.getImageData(0, 0, 1, 1).data).join(',') !== '0,255,0,255') throw Error('old storage changed');
                read.drawImage(next, 0, 0);
                if (Array.from(read.getImageData(0, 0, 1, 1).data).join(',') !== '0,0,255,255') throw Error('new storage not owned');
                snapshot.close(); next.close();
                if (traps !== 0) throw Error('proxy traps invoked');
                return true;
            })()"#,
            )?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("registered native OffscreenCanvas proxy preserves transfer identity");
}

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_transfer_preserves_premultiplied_bitmap_metadata() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let loader = page.request_client.clone();
        page.vm_mut().eval(
            r#"
            globalThis.__premultipliedTransfer = null;
            (async () => {
                const image = new ImageData(new Uint8ClampedArray([200, 100, 50, 128]), 1, 1);
                for (const alpha of [true, false]) {
                    const source = await createImageBitmap(image, {premultiplyAlpha: 'premultiply'});
                    const canvas = new OffscreenCanvas(1, 1);
                    canvas.getContext('bitmaprenderer', {alpha}).transferFromImageBitmap(source);
                    const read = new OffscreenCanvas(1, 1).getContext('2d');
                    read.drawImage(canvas, 0, 0);
                    const presentation = Array.from(read.getImageData(0, 0, 1, 1).data).join(',');
                    const expectedPresentation = alpha ? '199,100,50,128' : '100,50,25,255';
                    if (presentation !== expectedPresentation) throw Error('unexpected source presentation: ' + presentation);
                    const bitmap = canvas.transferToImageBitmap();
                    read.clearRect(0, 0, 1, 1); read.drawImage(bitmap, 0, 0);
                    if (Array.from(read.getImageData(0, 0, 1, 1).data).join(',') !== '199,100,50,128') throw Error('premultiplication metadata or raw pixel data lost');
                    const blank = canvas.transferToImageBitmap();
                    read.clearRect(0, 0, 1, 1); read.drawImage(blank, 0, 0);
                    const expectedBlank = alpha ? '0,0,0,0' : '0,0,0,255';
                    if (Array.from(read.getImageData(0, 0, 1, 1).data).join(',') !== expectedBlank) throw Error('replacement metadata not reset');
                    bitmap.close(); blank.close();
                }
                __premultipliedTransfer = true;
            })().catch(error => { __premultipliedTransfer = String(error); });
            "#,
        )?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.vm_mut().eval("__premultipliedTransfer === null")? == "true" {
            let _ = page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::BitmapTask,
                    &loader,
                )
                .await?;
            anyhow::ensure!(Instant::now() < deadline, "bitmap source did not finish");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(page.vm_mut().eval("__premultipliedTransfer")?, "true");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("transferred bitmap must retain its pixel representation");
}
