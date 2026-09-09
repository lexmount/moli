use super::*;

#[tokio::test(flavor = "current_thread")]
async fn bitmap_argument_errors_reject_promises_without_queuing_decode_tasks() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(r#"
        globalThis.outcomes = [];
        globalThis.getters = [];
        const source = new OffscreenCanvas(2, 3);
        const marker = {};
        const probes = [
            () => createImageBitmap(),
            () => createImageBitmap({width: 2, height: 3}),
            () => createImageBitmap(source, 0, 0),
            () => createImageBitmap(source, 0, 0, 0, 1),
            () => createImageBitmap(source, {resizeWidth: 0}),
            () => createImageBitmap(source, {resizeHeight: Infinity}),
            () => createImageBitmap(source, {resizeWidth: -1}),
            () => createImageBitmap(source, {imageOrientation: 'invalid'}),
            () => createImageBitmap(new OffscreenCanvas(0, 3)),
            () => createImageBitmap(source, {
                get colorSpaceConversion() { getters.push('colorSpaceConversion'); },
                get imageOrientation() { getters.push('imageOrientation'); },
                get premultiplyAlpha() { getters.push('premultiplyAlpha'); throw marker; },
                get resizeHeight() { getters.push('unexpected'); }
            }),
        ];
        for (const probe of probes) {
            const promise = probe();
            if (!(promise instanceof Promise)) throw new Error('missing Promise');
            promise.then(() => outcomes.push('unexpected success'), error => outcomes.push(error === marker ? 'marker' : error.name));
        }
        'scheduled'
        "#)?;
        assert!(!page_vm.vm().has_pending_bitmap_tasks());
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(outcomes)")?, r#"["TypeError","TypeError","TypeError","RangeError","InvalidStateError","TypeError","TypeError","TypeError","InvalidStateError","marker"]"#);
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(getters)")?, r#"["colorSpaceConversion","imageOrientation","premultiplyAlpha"]"#);
        Ok::<_, anyhow::Error>(())
    }).await.expect("bitmap Web IDL rejections");
}

async fn run_ready_bitmap_task(page_vm: &mut PageVm) -> anyhow::Result<()> {
    let loader = page_vm.request_client.clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !page_vm
        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::BitmapTask, &loader)
        .await?
    {
        anyhow::ensure!(
            Instant::now() < deadline,
            "bitmap decode did not enqueue its result"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn bitmap_blob_decodes_bytes_and_settles_in_a_later_task_in_the_intrinsic_realm() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let pixels = moli_image::RgbaImage::try_new(1, 2, vec![255, 0, 0, 255, 0, 255, 0, 255])?;
        let encoded = moli_image::encode_png(&pixels)?.bytes;
        page_vm.vm_mut().eval(&format!("globalThis.pngBytes = new Uint8Array({encoded:?}); 'ready'"))?;
        page_vm.vm_mut().eval(r#"
        globalThis.outcomes = [];
        const prototype = ImageBitmap.prototype;
        const valid = createImageBitmap(new Blob([pngBytes], {type: 'text/plain'}), {imageOrientation: 'flipY'});
        const invalid = createImageBitmap(new Blob(['not an image'], {type: 'image/png'}));
        const empty = createImageBitmap(new Blob());
        globalThis.ImageBitmap = function ReplacedImageBitmap() {};
        pngBytes.fill(0);
        valid.then(bitmap => {
            const canvas = new OffscreenCanvas(1, 2);
            const ctx = canvas.getContext('2d');
            ctx.drawImage(bitmap, 0, 0);
            outcomes.push([Object.getPrototypeOf(bitmap) === prototype, bitmap.width, bitmap.height, Array.from(ctx.getImageData(0, 0, 1, 2).data)]);
        }, error => outcomes.push(error.name));
        invalid.catch(error => outcomes.push('invalid:' + error.name));
        empty.catch(error => outcomes.push('empty:' + error.name));
        Promise.resolve().then(() => outcomes.push('microtask'));
        'scheduled'
        "#)?;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(outcomes)")?, r#"["microtask"]"#);
        for _ in 0..3 { run_ready_bitmap_task(&mut page_vm).await?; }
        assert!(!page_vm.vm().has_pending_bitmap_tasks());
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(outcomes.filter(Array.isArray))")?, "[[true,1,2,[0,255,0,255,255,0,0,255]]]");
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(outcomes.filter(value => typeof value === 'string').sort())")?, r#"["empty:InvalidStateError","invalid:InvalidStateError","microtask"]"#);
        Ok::<_, anyhow::Error>(())
    }).await.expect("bitmap Blob decode and Promise task timing");
}

#[tokio::test(flavor = "current_thread")]
async fn bitmap_snapshots_pixels_crops_resizes_and_rejects_closed_sources() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(r#"
        globalThis.results = [];
        const source = new OffscreenCanvas(2, 1);
        const ctx = source.getContext('2d');
        ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 2, 1);
        const pending = createImageBitmap(source, 2, 0, -2, 1, {resizeWidth: 4, resizeQuality: 'pixelated'});
        ctx.fillStyle = 'blue'; ctx.fillRect(0, 0, 2, 1);
        pending.then(async bitmap => {
            const dest = new OffscreenCanvas(4, 2);
            const destCtx = dest.getContext('2d');
            destCtx.drawImage(bitmap, 0, 0);
            results.push([bitmap.width, bitmap.height, Array.from(destCtx.getImageData(0, 0, 1, 1).data)]);
            const copyPromise = createImageBitmap(bitmap);
            bitmap.close(); bitmap.close();
            try { destCtx.drawImage(bitmap, 0, 0); } catch(error) { results.push(error.name); }
            await createImageBitmap(bitmap).catch(error => results.push(error.name));
            const copy = await copyPromise;
            results.push([copy.width, copy.height]);
        });
        const data = new ImageData(new Uint8ClampedArray([0, 255, 0, 255]), 1, 1);
        createImageBitmap(data, -1, 0, 2, 1, {resizeQuality: 'pixelated'}).then(bitmap => {
            const dest = new OffscreenCanvas(2, 1);
            const ctx = dest.getContext('2d');
            ctx.drawImage(bitmap, 0, 0);
            globalThis.cropPixels = Array.from(ctx.getImageData(0, 0, 2, 1).data);
        });
        data.data.fill(0);
        const htmlCanvas = document.createElement('canvas');
        htmlCanvas.width = 5; htmlCanvas.height = 3;
        Object.defineProperty(htmlCanvas, 'width', {get() { throw new Error('observable width getter'); }});
        createImageBitmap(htmlCanvas).then(bitmap => globalThis.htmlDimensions = [bitmap.width, bitmap.height]);
        'scheduled'
        "#)?;
        for _ in 0..4 { run_ready_bitmap_task(&mut page_vm).await?; }
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(results)")?, r#"[[4,2,[255,0,0,255]],"InvalidStateError","InvalidStateError",[4,2]]"#);
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(cropPixels)")?, "[0,0,0,0,0,255,0,255]");
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(htmlDimensions)")?, "[5,3]");
        Ok::<_, anyhow::Error>(())
    }).await.expect("bitmap snapshots and close");
}

#[tokio::test(flavor = "current_thread")]
async fn bitmap_task_survives_document_open_in_the_same_window() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(r#"
        globalThis.outcome = 'pending';
        createImageBitmap(new OffscreenCanvas(3, 2)).then(bitmap => outcome = [bitmap.width, bitmap.height]);
        document.open();
        'replaced'
        "#)?;
        assert!(page_vm.vm().has_pending_bitmap_tasks());
        run_ready_bitmap_task(&mut page_vm).await?;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(outcome)")?, "[3,2]");
        assert!(!page_vm.vm().has_pending_bitmap_tasks());
        Ok::<_, anyhow::Error>(())
    }).await.expect("bitmap Window lifetime");
}

#[tokio::test(flavor = "current_thread")]
async fn bitmap_task_uses_child_intrinsics_and_retires_when_the_child_window_is_removed() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let loader = page_vm.request_client.clone();
        page_vm.vm_mut().eval(r#"
        const frame = document.createElement('iframe');
        globalThis.bitmapFrame = frame;
        globalThis.bitmapChildOutcome = 'pending';
        (document.body || document.documentElement || document).appendChild(frame);
        void frame.contentWindow.Function;
        const script = frame.contentDocument.createElement('script');
        script.textContent = "createImageBitmap(new OffscreenCanvas(3, 4)).then(bitmap => { parent.bitmapChildOutcome = [bitmap.width, bitmap.height, Object.getPrototypeOf(bitmap) === ImageBitmap.prototype]; createImageBitmap(new OffscreenCanvas(1, 1)).then(() => parent.bitmapChildOutcome = 'stale'); });";
        frame.contentDocument.body.appendChild(script);
        'scheduled'
        "#)?;
        run_expected_child_realm_materialization_for_wait(&mut page_vm, "child bitmap realm").await;
        assert!(page_vm.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ChildDocumentScriptReady, &loader).await?);
        run_ready_bitmap_task(&mut page_vm).await?;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(bitmapChildOutcome)")?, "[3,4,true]");
        assert!(page_vm.vm().has_pending_bitmap_tasks());
        page_vm.vm_mut().eval("document.open(); 'retired'")?;
        assert!(!page_vm.vm().has_pending_bitmap_tasks());
        run_ready_bitmap_task(&mut page_vm).await?;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(bitmapChildOutcome)")?, "[3,4,true]");
        Ok::<_, anyhow::Error>(())
    }).await.expect("bitmap child realm settlement and retirement");
}
use crate::context_bootstrap::BitmapRejection;
use crate::page_task_queue::{
    MainDocumentMetaRefreshNavigationTask, PageInternalLoadingTargetEffect,
    PageOwnedInternalLoadingTask,
};

#[tokio::test(flavor = "current_thread")]
async fn create_image_bitmap_offscreen_canvas_and_close_contract() {
    run_page_vm_async_test(async move {
    let mut page_vm = test_page_vm();
    let loader = page_vm.request_client.clone();

    page_vm.vm_mut().exec(
        r#"
        (() => {
          const caughtName = callback => {
            try {
              callback();
              return "none";
            } catch (error) {
              return error.name;
            }
          };
          const descriptor = Object.getOwnPropertyDescriptor(window, "createImageBitmap");
          const blank = new OffscreenCanvas(16, 9);
          const drawn = new OffscreenCanvas(16, 9);
          drawn.getContext("2d").fillRect(0, 0, 1, 1);
          globalThis.__imageBitmapProbe = {
            functionShape: [
              typeof createImageBitmap,
              createImageBitmap.name,
              createImageBitmap.length,
              Object.prototype.hasOwnProperty.call(createImageBitmap, "prototype"),
              descriptor.enumerable,
              descriptor.configurable,
              descriptor.writable,
              caughtName(() => new createImageBitmap(drawn)),
              caughtName(() => createImageBitmap().catch(() => {})),
            ],
            constructorShape: [
              typeof ImageBitmap,
              ImageBitmap.name,
              ImageBitmap.length,
              Object.prototype.toString.call(ImageBitmap.prototype),
              Object.getPrototypeOf(ImageBitmap.prototype) === Object.prototype,
              caughtName(() => new ImageBitmap()),
            ],
            settled: false,
          };
          Promise.all([
            createImageBitmap(blank).then(
              () => "resolved",
              error => `rejected:${error.name}`,
            ),
            createImageBitmap(drawn).then(bitmap => {
              const before = [
                Object.prototype.toString.call(bitmap),
                bitmap instanceof ImageBitmap,
                Object.getPrototypeOf(bitmap) === ImageBitmap.prototype,
                Object.getOwnPropertyNames(bitmap).length,
                bitmap.width,
                bitmap.height,
                typeof bitmap.close,
              ];
              const closeResult = bitmap.close();
              return [before, typeof closeResult, bitmap.width, bitmap.height];
            }),
          ]).then(([blankOutcome, bitmapOutcome]) => {
            __imageBitmapProbe.blankOutcome = blankOutcome;
            __imageBitmapProbe.bitmapOutcome = bitmapOutcome;
            __imageBitmapProbe.settled = true;
          });
        })()
        "#,
        None,
    )
    .expect("createImageBitmap probe should execute");

    assert_eq!(page_vm.vm_mut().eval("String(__imageBitmapProbe.settled)")?, "false");
    for _ in 0..2 {
        assert!(page_vm.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::BitmapTask, &loader).await?);
    }
    let result = page_vm.vm_mut()
        .eval("JSON.stringify(globalThis.__imageBitmapProbe)")
        .expect("createImageBitmap probe should be readable");
    assert_eq!(
        result,
        r#"{"functionShape":["function","createImageBitmap",1,false,true,true,true,"TypeError","none"],"constructorShape":["function","ImageBitmap",0,"[object ImageBitmap]",true,"TypeError"],"settled":true,"blankOutcome":"resolved","bitmapOutcome":[["[object ImageBitmap]",true,true,0,16,9,"function"],"undefined",0,0]}"#
    );
    Ok::<_, anyhow::Error>(())
    }).await.expect("bitmap task surface");
}

#[test]
fn bitmap_task_rejects_a_real_page_vm_replacement_identity_collision() {
    run_page_vm_large_stack_async_test(
        "bitmap-real-page-vm-replacement-collision",
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
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let retired_producer = page_vm
                        .vm_mut()
                        .register_pending_bitmap_task_producer_for_executor_test()?;
                    let retired_owner = retired_producer.owner();
                    assert_eq!(
                        retired_owner.root_document(),
                        page_vm.document_lifecycle.identity().document
                    );
                    retired_producer
                        .send(Err(BitmapRejection::InvalidState))
                        .expect("retired Bitmap task should enter the stable Page source");

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

                    let current_producer = page_vm
                        .vm_mut()
                        .register_pending_bitmap_task_producer_for_executor_test()?;
                    let current_owner = current_producer.owner();
                    assert_eq!(
                        retired_owner.task(),
                        current_owner.task(),
                        "fresh PageVm counters should naturally reuse the first Bitmap task id and transport generation"
                    );
                    assert_eq!(
                        retired_owner.execution_context(),
                        current_owner.execution_context(),
                        "fresh PageVm counters should naturally reuse the top Window/realm identity"
                    );
                    assert_ne!(
                        retired_owner.root_document(),
                        current_owner.root_document(),
                        "the stable Page queue must namespace identical local owners by root Document"
                    );
                    assert_eq!(
                        current_owner.root_document(),
                        page_vm.document_lifecycle.identity().document
                    );
                    current_producer
                        .send(Err(BitmapRejection::InvalidState))
                        .expect("replacement Bitmap task should enter the same stable Page source");

                    let current_document_owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("replacement main Document owner");
                    page_vm
                        .vm()
                        .schedule_page_internal_loading_task(
                            PageOwnedInternalLoadingTask::MetaRefreshNavigation(
                            MainDocumentMetaRefreshNavigationTask::new(
                                current_document_owner,
                                0,
                                Url::parse("https://example.test/refresh").unwrap(),
                            ),
                            ),
                            Instant::now(),
                        )
                        .expect("internal-loading task should enter the stable Page source");
                    park_current_document_websocket_for_test(
                        &mut page_vm,
                        moli_websocket::Event::TextMessage {
                            socket_id: 41,
                            data: "blocked".to_owned(),
                        },
                    )
                    .await;
                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::BitmapTask, &loader)
                            .await?,
                        "retired Bitmap task should remain runnable beside independent internal-loading and backpressured WebSocket work"
                    );

                    assert_eq!(
                        page_vm
                            .vm()
                            .current_pending_bitmap_task_execution_context(current_owner.task()),
                        Some(current_owner.execution_context()),
                        "discarding the old completion must not remove the colliding replacement Promise"
                    );

                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::BitmapTask, &loader)
                            .await?,
                        "replacement Bitmap task should consume the following turn"
                    );

                    assert_eq!(
                        page_vm
                            .vm()
                            .current_pending_bitmap_task_execution_context(current_owner.task()),
                        None,
                        "the current completion must settle exactly the replacement Promise"
                    );
                    let internal_loading = page_vm
                        .run_internal_loading_body_for_test()
                        .expect("the independent internal-loading task should remain queued");
                    assert_eq!(
                        internal_loading.action.target_effect,
                        PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                            effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated,
                        },
                        "the synthetic refresh must still enforce its own post-load prerequisite"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("Bitmap replacement should run through the typed task executor");
            server
                .await
                .expect("Bitmap PageVm replacement server should finish");
        },
    );
}
