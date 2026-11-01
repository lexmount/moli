// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn script_enabled_noscript_keeps_computed_display_but_generates_no_layout_box() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/noscript-layout.html")?,
        );
        let computed = page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0;padding:0}noscript{display:block}#visible{width:20px;height:10px;background:red}</style>';
document.body.innerHTML = '<noscript id=fallback><meta content="0;url=/redirect" http-equiv=refresh><div>raw fallback</div></noscript><div id=visible></div>';
getComputedStyle(document.getElementById('fallback')).display
"#,
        )?;
        assert_eq!(
            computed, "block",
            "layout suppression must not rewrite the observable computed display"
        );
        page_vm.vm_mut().sync_live_document_style_sources();

        let fallback = page_vm
            .vm()
            .element_handle_by_id_for_test("fallback")
            .expect("noscript handle");
        let visible = page_vm
            .vm()
            .element_handle_by_id_for_test("visible")
            .expect("visible handle");
        let viewport = moli_layout::LayoutViewport::new(100, 50, 1.0);
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("current document screenshot layout");
        let batch = moli_layout::LayoutQueryBatch::new(vec![
            moli_layout::LayoutQuery::BoxModel { source: fallback },
            moli_layout::LayoutQuery::BoxModel { source: visible },
        ]);
        let output = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            &batch,
        )?;
        assert!(matches!(
            output.answers[0],
            moli_layout::LayoutQueryAnswer::BoxModel(None)
        ));
        let visible_rect = match &output.answers[1] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect()
            }
            answer => panic!("unexpected visible box-model answer: {answer:?}"),
        };
        assert_eq!(
            visible_rect,
            moli_layout::LayoutRect::new(0.0, 0.0, 20.0, 10.0)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("script-enabled noscript layout fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn geometry_batch_requires_explicit_output_and_reuses_it_until_the_next_output() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-batch.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0;padding:0}#target{width:120px;height:40px;background:red}#pass-through,#hidden{position:absolute;left:0;top:0;width:120px;height:40px;z-index:10}#pass-through{pointer-events:none}#hidden{visibility:hidden}#scroller{position:absolute;left:200px;top:60px;width:80px;height:60px;overflow:hidden}#wide{width:200px;height:120px}#transformed{position:absolute;left:0;top:100px;width:40px;height:20px;transform:translate(15px,5px)}</style>';
document.body.innerHTML = '<div id="target"></div><div id="pass-through"></div><div id="hidden"></div><div id="scroller"><div id="wide"></div></div><div id="transformed"></div>';
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let target = page_vm
            .vm()
            .element_handle_by_id_for_test("target")
            .expect("target handle");
        let wide = page_vm
            .vm()
            .element_handle_by_id_for_test("wide")
            .expect("wide child handle");
        let transformed = page_vm
            .vm()
            .element_handle_by_id_for_test("transformed")
            .expect("transformed handle");
        let before = page_vm.vm().layout_pass_observability_for_test();
        let cache_before = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!before.0, "no pass may remain active between demands");
        assert!(cache_before.3.is_none());

        let batch = moli_layout::LayoutQueryBatch::new(vec![
            moli_layout::LayoutQuery::DocumentMetrics,
            moli_layout::LayoutQuery::BoxModel { source: target },
            moli_layout::LayoutQuery::ClientRects { source: target },
            moli_layout::LayoutQuery::HitTest {
                point: moli_layout::LayoutPoint::new(10.0, 10.0),
                ignore_pointer_events_none: false,
            },
            moli_layout::LayoutQuery::BoxModel { source: wide },
            moli_layout::LayoutQuery::HitTest {
                point: moli_layout::LayoutPoint::new(210.0, 70.0),
                ignore_pointer_events_none: false,
            },
            moli_layout::LayoutQuery::BoxModel {
                source: transformed,
            },
        ]);
        let viewport = moli_layout::LayoutViewport::new(320, 200, 1.0);
        let error = moli_layout::GeometryProvider::answer(page_vm.vm_mut(), &batch).unwrap_err();
        assert_eq!(error, moli_layout::LayoutError::NoLayoutSnapshot);
        for command in ["Page.captureScreenshot", "Page.startScreencast"] {
            assert!(error.to_string().contains(command));
        }
        assert!(!error.to_string().contains("Page.printToPDF"));
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before.1);
        page_vm.vm_mut().screenshot_layout_snapshot(viewport)?.expect("initial screenshot layout");
        let first = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            &batch,
        )?;
        let after_first = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_first = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!after_first.0);
        assert_eq!(after_first.1, before.1 + 1);
        assert_eq!(cache_after_first.0, cache_before.0 + 1);
        assert_eq!(cache_after_first.1, cache_before.1 + 1);
        assert_eq!(cache_after_first.2, cache_before.2 + 1);
        assert_eq!(first.answers.len(), batch.queries.len());
        assert_eq!(
            first.metrics.reason,
            moli_layout::LayoutFlushReason::Screenshot
        );
        assert!(first.metrics.paint_operation_count > 0);
        let first_width = match &first.answers[1] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect().width
            }
            answer => panic!("unexpected box-model answer: {answer:?}"),
        };
        assert!((first_width - 120.0).abs() <= 0.05, "{first_width}");
        assert!(matches!(
            first.answers[3],
            moli_layout::LayoutQueryAnswer::HitTest(Some(hit)) if hit.source == target
        ));
        let wide_rect = match &first.answers[4] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect()
            }
            answer => panic!("unexpected scrolled box-model answer: {answer:?}"),
        };
        assert!((wide_rect.x - 200.0).abs() <= 0.05, "{wide_rect:?}");
        assert!((wide_rect.y - 60.0).abs() <= 0.05, "{wide_rect:?}");
        assert!(matches!(
            first.answers[5],
            moli_layout::LayoutQueryAnswer::HitTest(Some(hit)) if hit.source == wide
        ));
        let transformed_rect = match &first.answers[6] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect()
            }
            answer => panic!("unexpected transformed box-model answer: {answer:?}"),
        };
        assert!(
            (transformed_rect.x - 15.0).abs() <= 0.05
                && (transformed_rect.y - 105.0).abs() <= 0.05,
            "{transformed_rect:?}"
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').style.width='180px'; 'mutated'")?;
        let second = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            &batch,
        )?;
        let after_second = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_second = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!after_second.0);
        assert_eq!(after_second.1, before.1 + 1);
        assert_eq!(cache_after_second.0, cache_before.0 + 2);
        assert_eq!(cache_after_second.1, cache_before.1 + 1);
        assert_eq!(cache_after_second.2, cache_before.2 + 1);
        let second_width = match &second.answers[1] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect().width
            }
            answer => panic!("unexpected box-model answer: {answer:?}"),
        };
        assert!((second_width - 120.0).abs() <= 0.05, "{second_width}");
        assert_eq!(after_second.2, after_first.2);
        assert_eq!(after_second.3, after_first.3);
        assert_eq!(second.metrics, first.metrics);
        assert!(matches!(
            second.answers[0],
            moli_layout::LayoutQueryAnswer::DocumentMetrics(metrics)
                if metrics.viewport == viewport
        ));

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("current document screenshot layout");
        let after_screenshot = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_screenshot = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!after_screenshot.0);
        assert_eq!(after_screenshot.1, before.1 + 2);
        assert_eq!(cache_after_screenshot.0, cache_before.0 + 2);
        assert_eq!(cache_after_screenshot.1, cache_before.1 + 1);
        assert_eq!(cache_after_screenshot.2, cache_before.2 + 2);
        let (_, retention) = cache_after_screenshot
            .3
            .expect("fresh paint layout should publish its frozen tree");
        assert!(retention.box_count > 0);
        assert!(retention.fragment_count > 0);
        assert!(retention.estimated_geometry_bytes > 0);
        let screenshot_metrics = after_screenshot.3.expect("screenshot layout metrics");
        assert_eq!(
            screenshot_metrics.reason,
            moli_layout::LayoutFlushReason::Screenshot
        );
        assert_eq!(
            screenshot_metrics.paint_operation_count,
            snapshot.fragments.len()
        );
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "transform-paint-deferred"
                && diagnostic.code != "scroll-paint-deferred"
        }));

        let third = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            &batch,
        )?;
        let after_third = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_third = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert_eq!(after_third.1, before.1 + 2);
        assert_eq!(cache_after_third.0, cache_before.0 + 3);
        assert_eq!(cache_after_third.1, cache_before.1 + 1);
        assert_eq!(cache_after_third.2, cache_before.2 + 2);
        let third_width = match &third.answers[1] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect().width
            }
            answer => panic!("unexpected refreshed box-model answer: {answer:?}"),
        };
        assert!((third_width - 180.0).abs() <= 0.05, "{third_width}");
        assert_eq!(third.metrics.reason, moli_layout::LayoutFlushReason::Screenshot);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("one-shot geometry batch test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn every_screencast_frame_refreshes_and_publishes_geometry() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/screencast-layout-cache.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0}#target{width:40px;height:20px;background:red}</style>';
document.body.innerHTML = '<div id=target></div>';
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let target = page_vm
            .vm()
            .element_handle_by_id_for_test("target")
            .expect("target handle");
        let batch = moli_layout::LayoutQueryBatch::new(vec![
            moli_layout::LayoutQuery::BoxModel { source: target },
        ]);
        let passes_before = page_vm.vm().layout_pass_observability_for_test().1;
        let cache_before = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();

        page_vm
            .vm_mut()
            .paint_layout_snapshot(
                moli_layout::PaintViewport::new(320, 200, 1.0),
                moli_layout::LayoutFlushReason::Screencast,
            )?
            .expect("first screencast frame layout");
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_before + 1
        );
        let cache_after_first = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert_eq!(cache_after_first.2, cache_before.2 + 1);
        assert!(cache_after_first.3.is_some());

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').style.width='80px'; 'mutated'")?;
        let stale = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            &batch,
        )?;
        let stale_width = match &stale.answers[0] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect().width
            }
            answer => panic!("unexpected stale box-model answer: {answer:?}"),
        };
        assert!((stale_width - 40.0).abs() <= 0.05, "{stale_width}");
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_before + 1
        );

        page_vm
            .vm_mut()
            .paint_layout_snapshot(
                moli_layout::PaintViewport::new(320, 200, 1.0),
                moli_layout::LayoutFlushReason::Screencast,
            )?
            .expect("second screencast frame layout");
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_before + 2
        );
        let cache_after_second = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert_eq!(cache_after_second.0, cache_before.0 + 1);
        assert_eq!(cache_after_second.1, cache_before.1);
        assert_eq!(cache_after_second.2, cache_before.2 + 2);
        assert!(cache_after_second.3.is_some());

        let refreshed = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            &batch,
        )?;
        let refreshed_width = match &refreshed.answers[0] {
            moli_layout::LayoutQueryAnswer::BoxModel(Some(model)) => {
                model.border.bounding_rect().width
            }
            answer => panic!("unexpected refreshed box-model answer: {answer:?}"),
        };
        assert!((refreshed_width - 80.0).abs() <= 0.05, "{refreshed_width}");
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_before + 2
        );
        let cache_after_query = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert_eq!(cache_after_query.0, cache_before.0 + 2);
        assert_eq!(cache_after_query.1, cache_before.1);
        assert_eq!(cache_after_query.2, cache_before.2 + 2);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("screencast layout cache test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screencast_visual_token_tracks_child_document_styles_without_retaining_paint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/child-screencast-style-generation.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.documentElement.style.cssText = 'margin:0;background:white';
document.body.style.cssText = 'margin:0';
const frame = document.createElement('iframe');
frame.style.cssText = 'display:block;width:40px;height:20px;border:0';
document.body.appendChild(frame);
const child = frame.contentDocument;
child.documentElement.style.cssText = 'margin:0';
child.body.style.cssText = 'margin:0;width:40px;height:20px';
globalThis.__childScreencastSheet = new frame.contentWindow.CSSStyleSheet();
globalThis.__childScreencastSheet.replaceSync('body { background: red; }');
child.adoptedStyleSheets = [globalThis.__childScreencastSheet];
'installed'
"#,
        )?;

        let passes_before = page_vm.vm().layout_pass_observability_for_test().1;
        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(first) =
            page_vm.capture_screencast_frame(viewport_screencast_request(None))?
        else {
            panic!("first child-style screencast demand must capture a frame");
        };
        let first_raster = moli_image::decode_png(&first.image.bytes)?;
        assert_eq!(&first_raster.rgba[0..4], [255, 0, 0, 255]);
        let first_visual_state = first.visual_state;
        let passes_after_first = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(passes_after_first, passes_before + 1);

        assert_eq!(
            page_vm.capture_screencast_frame(viewport_screencast_request(Some(
                first_visual_state.clone(),
            )))?,
            crate::runtime::RendererCaptureScreencastFrameReply::Unchanged,
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_after_first,
            "an unchanged token must avoid layout and paint entirely",
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__childScreencastSheet.replaceSync('body { background: blue; }')")?;
        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(refreshed) = page_vm
            .capture_screencast_frame(viewport_screencast_request(Some(
                first_visual_state.clone(),
            )))?
        else {
            panic!("a child stylesheet mutation must capture a fresh frame");
        };
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_after_first + 1,
            "a child-only stylesheet generation must invalidate the session token",
        );
        assert_ne!(refreshed.visual_state, first_visual_state);
        let refreshed_raster = moli_image::decode_png(&refreshed.image.bytes)?;
        assert_eq!(&refreshed_raster.rgba[0..4], [0, 0, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child screencast style-generation fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screencast_visual_token_tracks_canvas_and_scroll_while_screenshot_stays_fresh() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/screencast-visual-generations.html")?,
        );
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 40,
            inner_height: 30,
            outer_width: 40,
            outer_height: 30,
            device_pixel_ratio: 1.0,
            screen_width: 40,
            screen_height: 30,
            screen_avail_width: 40,
            screen_avail_height: 30,

..Default::default()
}))?;
        page_vm.vm_mut().eval(
            r#"
document.documentElement.style.cssText = 'margin:0;background:white';
document.body.style.cssText = 'margin:0;height:100px';
const canvas = document.createElement('canvas');
canvas.id = 'surface';
canvas.width = 40;
canvas.height = 100;
canvas.style.cssText = 'display:block;width:40px;height:100px';
document.body.appendChild(canvas);
const context = canvas.getContext('2d');
context.fillStyle = 'red';
context.fillRect(0, 0, 40, 100);
'installed'
"#,
        )?;

        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(first) =
            page_vm.capture_screencast_frame(viewport_screencast_request(None))?
        else {
            panic!("initial canvas screencast demand must capture a frame");
        };
        let first_state = first.visual_state;
        assert_eq!(
            &moli_image::decode_png(&first.image.bytes)?.rgba[0..4],
            [255, 0, 0, 255]
        );

        assert_eq!(
            page_vm
                .capture_screencast_frame(viewport_screencast_request(Some(first_state.clone())))?,
            crate::runtime::RendererCaptureScreencastFrameReply::Unchanged,
        );

        // Canvas pixel writes do not mutate the DOM or stylesheet worlds. The
        // resource generation must independently invalidate the token.
        page_vm.vm_mut().eval(
            "const c=document.getElementById('surface').getContext('2d');c.fillStyle='#0000ff';c.fillRect(0,0,40,100);'painted'",
        )?;
        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(canvas_frame) = page_vm
            .capture_screencast_frame(viewport_screencast_request(Some(first_state.clone())))?
        else {
            panic!("canvas mutation must capture a fresh screencast frame");
        };
        let canvas_state = canvas_frame.visual_state;
        assert_ne!(canvas_state, first_state);
        assert_eq!(
            &moli_image::decode_png(&canvas_frame.image.bytes)?.rgba[0..4],
            [0, 0, 255, 255]
        );

        page_vm.vm_mut().eval("scrollTo(0, 30); scrollY")?;
        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(scrolled) = page_vm
            .capture_screencast_frame(viewport_screencast_request(Some(canvas_state.clone())))?
        else {
            panic!("scroll mutation must capture a fresh screencast frame");
        };
        let scrolled_state = scrolled.visual_state;
        assert_ne!(scrolled_state, canvas_state);

        let passes_before_screenshot = page_vm.vm().layout_pass_observability_for_test().1;
        let crate::runtime::RendererCaptureScreenshotReply::Captured(screenshot) =
            page_vm.capture_screenshot(crate::runtime::RendererCaptureScreenshotRequest::viewport_png())?
        else {
            panic!("ordinary screenshots must ignore screencast token suppression");
        };
        assert_eq!(screenshot.mime_type, "image/png");
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            passes_before_screenshot + 1,
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("screencast visual generations fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn async_resource_publication_during_screencast_capture_forces_a_followup_frame() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/screencast-resource-race.html")?,
        );
        page_vm
            .vm_mut()
            .eval("document.documentElement.style.cssText='margin:0;background:red';'ready'")?;

        let generation = page_vm.vm().visual_resource_generation_handle_for_test();
        let rendezvous = std::sync::Arc::new(std::sync::Barrier::new(2));
        let worker_rendezvous = rendezvous.clone();
        let completion = std::thread::spawn(move || {
            worker_rendezvous.wait();
            generation.bump();
            worker_rendezvous.wait();
        });

        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(raced) = page_vm
            .capture_screencast_frame_with_before_layout_hook(
                viewport_screencast_request(None),
                || {
                    rendezvous.wait();
                    rendezvous.wait();
                },
            )?
        else {
            panic!("the initial screencast poll must capture a frame");
        };
        completion
            .join()
            .expect("the asynchronous resource publisher must finish");

        let crate::runtime::RendererCaptureScreencastFrameReply::Captured(followup) = page_vm
            .capture_screencast_frame(viewport_screencast_request(Some(raced.visual_state)))?
        else {
            panic!("a resource publication racing the prior paint must force one followup frame");
        };
        assert_eq!(
            page_vm.capture_screencast_frame(viewport_screencast_request(Some(
                followup.visual_state,
            )))?,
            crate::runtime::RendererCaptureScreencastFrameReply::Unchanged,
            "the followup frame must close the resource-generation race",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("screencast resource-race fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_composes_iframe_documents_into_exact_used_content_viewports() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/iframe-paint-composition.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.documentElement.style.cssText = 'margin:0;padding:0;background:white';
document.body.style.cssText = 'margin:0;padding:0';

const frame = document.createElement('iframe');
frame.id = 'paint-frame';
frame.style.cssText = 'position:absolute;left:20px;top:10px;display:block;box-sizing:border-box;width:120px;height:80px;margin:0;border:4px solid black;padding:6px;background:rgb(255,255,0);transform:translate(5px,3px)';
document.body.appendChild(frame);

const child = frame.contentDocument;
child.documentElement.style.cssText = 'margin:0;padding:0;background:rgb(0,255,255);scrollbar-width:none';
child.body.style.cssText = 'position:relative;margin:0;padding:0;width:200px;height:120px';

const viewportSized = child.createElement('div');
viewportSized.id = 'viewport-sized';
viewportSized.style.cssText = 'position:absolute;left:0;top:0;width:50vw;height:50vh;background:rgb(255,0,0)';
child.body.appendChild(viewportSized);

const clipped = child.createElement('div');
clipped.style.cssText = 'position:absolute;left:90px;top:0;width:30px;height:100px;background:rgb(0,128,0)';
child.body.appendChild(clipped);

const label = child.createElement('span');
label.textContent = 'frame';
label.style.cssText = 'position:absolute;left:55px;top:35px;font:10px/10px sans-serif;color:black';
child.body.appendChild(label);
const icon = child.createElementNS('http://www.w3.org/2000/svg', 'svg');
icon.setAttribute('width', '8');
icon.setAttribute('height', '8');
icon.style.cssText = 'position:absolute;left:75px;top:45px';
const iconRect = child.createElementNS('http://www.w3.org/2000/svg', 'rect');
iconRect.setAttribute('width', '8');
iconRect.setAttribute('height', '8');
iconRect.setAttribute('fill', 'rgb(128,0,128)');
icon.appendChild(iconRect);
child.body.appendChild(icon);

const nested = child.createElement('iframe');
nested.style.cssText = 'position:absolute;left:10px;top:35px;display:block;box-sizing:border-box;width:40px;height:20px;margin:0;border:2px solid rgb(0,0,255);padding:2px;background:rgb(255,255,0)';
child.body.appendChild(nested);
const nestedDocument = nested.contentDocument;
nestedDocument.documentElement.style.cssText = 'margin:0;padding:0;background:rgb(255,0,255);scrollbar-width:none';
nestedDocument.body.style.cssText = 'position:relative;margin:0;padding:0;width:80px;height:40px';
const nestedViewportSized = nestedDocument.createElement('div');
nestedViewportSized.style.cssText = 'width:50vw;height:100vh;background:rgb(0,0,0)';
nestedDocument.body.appendChild(nestedViewportSized);
'installed'
"#,
        )?;

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 120, 1.0))?
            .expect("iframe fixture must retain a layout root");
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "replaced-content-placeholder"
        }));
        assert!(
            !snapshot.fonts.is_empty()
                && snapshot.fragments.iter().any(|fragment| matches!(
                    fragment,
                    moli_layout::PaintFragment::GlyphRun(_)
                )),
            "child glyph resources must be remapped into the parent snapshot"
        );
        assert!(
            !snapshot.svg_images.is_empty()
                && snapshot.fragments.iter().any(|fragment| matches!(
                    fragment,
                    moli_layout::PaintFragment::SvgImage(_)
                )),
            "child SVG resources must be remapped into the parent snapshot"
        );
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            &image.rgba[index..index + 4]
        };

        // The transformed 120x80 border box starts at (25,13). Its exact used
        // content viewport is 100x60 after the 4px border and 6px padding.
        assert_eq!(pixel(26, 14), [0, 0, 0, 255]);
        assert_eq!(pixel(31, 20), [255, 255, 0, 255]);
        assert_eq!(pixel(36, 24), [255, 0, 0, 255]);
        assert_eq!(pixel(84, 24), [255, 0, 0, 255]);
        assert_eq!(pixel(85, 24), [0, 255, 255, 255]);
        assert_eq!(pixel(36, 52), [255, 0, 0, 255]);
        assert_eq!(pixel(36, 53), [0, 255, 255, 255]);

        // Child overflow is clipped at the iframe content edge rather than
        // painting through its parent padding and border.
        assert_eq!(pixel(130, 30), [0, 128, 0, 255]);
        assert_eq!(pixel(136, 30), [255, 255, 0, 255]);
        assert_eq!(pixel(146, 30), [255, 255, 255, 255]);

        // Nested browsing contexts recurse through the same composition seam.
        // Its 40x20 border box yields a 32x12 content viewport, so 50vw is 16px.
        assert_eq!(pixel(46, 59), [0, 0, 255, 255]);
        assert_eq!(pixel(48, 61), [255, 255, 0, 255]);
        assert_eq!(pixel(50, 63), [0, 0, 0, 255]);
        assert_eq!(pixel(64, 63), [0, 0, 0, 255]);
        assert_eq!(pixel(65, 63), [255, 0, 255, 255]);

        page_vm
            .vm_mut()
            .eval("document.getElementById('paint-frame').contentWindow.scrollTo(20,10);'scrolled'")?;
        let scrolled = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 120, 1.0))?
            .expect("scrolled iframe fixture must retain a layout root");
        let scrolled = moli_paint::raster_snapshot(&scrolled)?;
        let scrolled_pixel = |x: u32, y: u32| {
            let index = ((y * scrolled.width + x) * 4) as usize;
            &scrolled.rgba[index..index + 4]
        };
        assert_eq!(scrolled_pixel(60, 24), [255, 0, 0, 255]);
        assert_eq!(scrolled_pixel(70, 24), [0, 255, 255, 255]);
        assert_eq!(scrolled_pixel(110, 24), [0, 128, 0, 255]);
        assert_eq!(scrolled_pixel(136, 24), [255, 255, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("iframe snapshot composition fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_composites_transparent_iframe_canvas_over_its_owner_background() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/transparent-iframe-canvas.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.documentElement.style.cssText = 'margin:0;padding:0;background:white';
document.body.style.cssText = 'margin:0;padding:0';
document.body.innerHTML = `
<iframe id=transparent style="position:absolute;left:0;top:0;display:block;width:100px;aspect-ratio:1/1;border:0;background:green"></iframe>
<iframe id=colored style="position:absolute;left:110px;top:0;display:block;width:100px;aspect-ratio:1/1;border:0;background:green"></iframe>`;
const colored = document.getElementById('colored').contentDocument;
colored.documentElement.style.cssText = 'margin:0;padding:0;background:blue';
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(210, 100, 1.0))?
            .expect("iframe canvas fixture must retain a root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            &image.rgba[index..index + 4]
        };

        assert_eq!(
            pixel(50, 50),
            [0, 128, 0, 255],
            "a transparent child canvas must expose the iframe owner's background"
        );
        assert_eq!(
            pixel(160, 50),
            [0, 0, 255, 255],
            "a child document's own canvas background must still cover the owner"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("transparent iframe canvas fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_and_screencast_paint_fresh_canvas_2d_backing_stores() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/canvas-paint-composition.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.documentElement.style.cssText = 'margin:0;padding:0;background:white';
document.body.style.cssText = 'margin:0;padding:0';

const canvas = document.createElement('canvas');
canvas.id = 'main-canvas';
canvas.width = 4;
canvas.height = 2;
canvas.style.cssText = 'position:absolute;left:0;top:0;width:40px;height:20px;image-rendering:pixelated';
document.body.appendChild(canvas);
const context = canvas.getContext('2d');
context.fillStyle = '#ff0000';
context.fillRect(0,0,2,2);
context.fillStyle = '#0000ff';
context.fillRect(2,0,2,2);

const frame = document.createElement('iframe');
frame.id = 'canvas-frame';
frame.style.cssText = 'position:absolute;left:0;top:30px;display:block;width:20px;height:10px;margin:0;border:0;padding:0';
document.body.appendChild(frame);
const child = frame.contentDocument;
child.documentElement.style.cssText = 'margin:0;padding:0;background:white';
child.body.style.cssText = 'margin:0;padding:0';
const childCanvas = child.createElement('canvas');
childCanvas.id = 'child-canvas';
childCanvas.width = 1;
childCanvas.height = 1;
childCanvas.style.cssText = 'display:block;width:20px;height:10px;image-rendering:pixelated';
child.body.appendChild(childCanvas);
const childContext = childCanvas.getContext('2d');
childContext.fillStyle = '#ff00ff';
childContext.fillRect(0,0,1,1);

const detachedCanvas = document.createElement('canvas');
detachedCanvas.width = 1;
detachedCanvas.height = 1;
detachedCanvas.style.cssText = 'position:absolute;left:30px;top:30px;width:20px;height:10px;image-rendering:pixelated';
const detachedContext = detachedCanvas.getContext('2d');
detachedContext.fillStyle = '#ff0000';
detachedContext.fillRect(0,0,1,1);
detachedCanvas.setAttribute('width','1');
document.body.appendChild(detachedCanvas);
'installed'
"#,
        )?;

        let first = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(80, 60, 1.0))?
            .expect("canvas fixture must retain a layout root");
        assert_eq!(first.images.len(), 3);
        assert!(first.fragments.iter().any(|fragment| {
            matches!(fragment, moli_layout::PaintFragment::Image(_))
        }));
        assert!(
            first
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "canvas-content-unavailable")
        );
        let first_raster = moli_paint::raster_snapshot(&first)?;
        let pixel = |image: &moli_image::RgbaImage, x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(pixel(&first_raster, 5, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(&first_raster, 35, 5), [0, 0, 255, 255]);
        assert_eq!(pixel(&first_raster, 10, 35), [255, 0, 255, 255]);
        // Attribute assignment also resets a backing store created while the
        // canvas is detached; appending it must not resurrect the red pixels.
        assert_eq!(pixel(&first_raster, 35, 35), [255, 255, 255, 255]);

        // Chromium resets Canvas2D even when a width/height assignment keeps
        // the same bitmap dimensions. Exercise both the content-attribute and
        // IDL setter paths before taking another paint snapshot.
        page_vm.vm_mut().eval(
            r#"
document.getElementById('main-canvas').setAttribute('width','4');
document.getElementById('canvas-frame').contentDocument.getElementById('child-canvas').height = 1;
'reset'
"#,
        )?;
        let reset = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(80, 60, 1.0))?
            .expect("reset canvas fixture must retain a layout root");
        assert!(
            reset
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "canvas-content-unavailable")
        );
        let reset_raster = moli_paint::raster_snapshot(&reset)?;
        assert_eq!(pixel(&reset_raster, 5, 5), [255, 255, 255, 255]);
        assert_eq!(pixel(&reset_raster, 10, 35), [255, 255, 255, 255]);

        page_vm.vm_mut().eval(
            r#"
const mainContext = document.getElementById('main-canvas').getContext('2d');
mainContext.fillStyle = '#008000';
mainContext.fillRect(0,0,4,2);
const nextChildContext = document.getElementById('canvas-frame').contentDocument.getElementById('child-canvas').getContext('2d');
nextChildContext.fillStyle = '#ffff00';
nextChildContext.fillRect(0,0,1,1);
'mutated'
"#,
        )?;
        let screencast = page_vm
            .vm_mut()
            .paint_layout_snapshot(
                moli_layout::PaintViewport::new(80, 60, 1.0),
                moli_layout::LayoutFlushReason::Screencast,
            )?
            .expect("screencast canvas fixture must retain a layout root");
        let screencast_raster = moli_paint::raster_snapshot(&screencast)?;
        assert_eq!(pixel(&screencast_raster, 5, 5), [0, 128, 0, 255]);
        assert_eq!(pixel(&screencast_raster, 10, 35), [255, 255, 0, 255]);

        // A later Canvas mutation replaces the host Arc; it must not mutate a
        // previously returned owned paint snapshot.
        let first_raster_after_mutation = moli_paint::raster_snapshot(&first)?;
        assert_eq!(
            pixel(&first_raster_after_mutation, 5, 5),
            [255, 0, 0, 255]
        );
        assert_eq!(
            pixel(&first_raster_after_mutation, 10, 35),
            [255, 0, 255, 255]
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("canvas screenshot/screencast fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn webkit_text_stroke_and_paint_order_reach_glyph_runs() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/text-stroke.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0}div{font:20px/1 sans-serif}#a{-webkit-text-stroke:3px rgb(255,0,0);paint-order:stroke fill}#b{-webkit-text-stroke:2px;color:rgb(0,0,255)}#c{}</style>';
document.body.innerHTML = '<div id=a>A</div><div id=b>B</div><div id=c>C</div>';
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 80, 1.0))?
            .expect("text stroke fixture must retain a layout root");
        let strokes: Vec<_> = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) => Some(run.stroke),
                _ => None,
            })
            .collect();
        assert_eq!(
            strokes,
            vec![
                Some(moli_layout::PaintTextStroke {
                    width: 3.0,
                    color: moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                    paint_first: true,
                }),
                // -webkit-text-stroke-color defaults to currentcolor.
                Some(moli_layout::PaintTextStroke {
                    width: 2.0,
                    color: moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0),
                    paint_first: false,
                }),
                None,
            ]
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("text stroke fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn backdrop_filter_projects_a_fragment_inside_the_elements_own_layers() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/backdrop-filter.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0}div{position:absolute;width:20px;height:20px}#glass{left:0;opacity:.5;filter:invert(1);backdrop-filter:blur(3px) drop-shadow(1px 1px red) saturate(2)}#plain{left:30px;backdrop-filter:grayscale(1)}</style>';
document.body.innerHTML = '<div id=glass></div><div id=plain></div>';
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(60, 30, 1.0))?
            .expect("backdrop fixture must retain a layout root");
        let codes: Vec<_> = snapshot
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect();
        assert!(codes.contains(&"backdrop-filter-drop-shadow-unsupported"), "{codes:?}");
        assert!(!codes.contains(&"backdrop-filter-unsupported"), "{codes:?}");

        let backdrops: Vec<_> = snapshot
            .fragments
            .iter()
            .enumerate()
            .filter_map(|(index, fragment)| match fragment {
                moli_layout::PaintFragment::BackdropFilter {
                    filters,
                    own_scopes,
                    ..
                } => Some((index, filters.clone(), *own_scopes)),
                _ => None,
            })
            .collect();
        assert_eq!(backdrops.len(), 2);
        let (glass, filters, own_scopes) = &backdrops[0];
        assert_eq!(
            filters,
            &vec![
                moli_layout::PaintFilter::Blur(3.0),
                moli_layout::PaintFilter::Saturate(2.0),
            ]
        );
        // Its own opacity layer wraps the backdrop ...
        assert_eq!(*own_scopes, 1);
        assert!(matches!(
            snapshot.fragments[glass - 1],
            moli_layout::PaintFragment::PushLayer { opacity, filter: None, .. } if opacity == 0.5
        ));
        // ... then it becomes a Backdrop Root for its descendants, and only
        // then does its own filter layer start (known limitation).
        assert!(matches!(
            snapshot.fragments[glass + 1],
            moli_layout::PaintFragment::PushLayer { filter: None, .. }
        ));
        assert!(matches!(
            snapshot.fragments[glass + 2],
            moli_layout::PaintFragment::BackdropRoot
        ));
        assert!(matches!(
            snapshot.fragments[glass + 3],
            moli_layout::PaintFragment::PushLayer {
                filter: Some(moli_layout::PaintFilter::Invert(_)),
                ..
            }
        ));
        assert_eq!(backdrops[1].1, vec![moli_layout::PaintFilter::Grayscale(1.0)]);
        assert_eq!(backdrops[1].2, 0);
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("backdrop filter fixture should run");
}

/// Renders `body` over a red canvas at 120x60 and returns the RGBA at `(x, y)`
/// samples plus the projected snapshot.
async fn backdrop_fixture_pixels(
    css: &str,
    body: &str,
    samples: &[(u32, u32)],
) -> anyhow::Result<(Vec<[u8; 4]>, moli_layout::PaintSnapshot)> {
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut page_vm = test_page_vm_with_loader_and_document_url(
        &loader,
        Vec::new(),
        Url::parse("https://example.com/backdrop-roots.html")?,
    );
    page_vm.vm_mut().eval(&format!(
        r#"
document.head.innerHTML = `<style>html,body{{margin:0;background:rgb(255,0,0)}}div{{position:absolute}}{css}</style>`;
document.body.innerHTML = `{body}`;
'installed'
"#
    ))?;
    page_vm.vm_mut().sync_live_document_style_sources();
    let snapshot = page_vm
        .vm_mut()
        .screenshot_layout_snapshot(moli_layout::PaintViewport::new(120, 60, 1.0))?
        .expect("backdrop fixture must retain a layout root");
    let image = moli_paint::raster_snapshot(&snapshot)?;
    let pixels = samples
        .iter()
        .map(|&(x, y)| {
            let offset = ((y * image.width + x) * 4) as usize;
            [
                image.rgba[offset],
                image.rgba[offset + 1],
                image.rgba[offset + 2],
                image.rgba[offset + 3],
            ]
        })
        .collect();
    Ok((pixels, snapshot))
}

fn backdrop_fragment_count(snapshot: &moli_layout::PaintSnapshot) -> usize {
    snapshot
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment, moli_layout::PaintFragment::BackdropFilter { .. }))
        .count()
}

#[tokio::test(flavor = "current_thread")]
async fn backdrop_filter_is_not_painted_for_visibility_hidden_elements() {
    run_page_vm_async_test(async move {
        // invert(1) turns the red canvas cyan wherever a backdrop is painted.
        let (pixels, snapshot) = backdrop_fixture_pixels(
            "#glass{left:10px;top:10px;width:40px;height:40px;backdrop-filter:invert(1)}",
            // A visible descendant keeps the hidden element's stacking context
            // in the paint plan, which is where its backdrop used to leak.
            "<div id=glass style='visibility:hidden'><div style='visibility:visible;left:0;top:0;width:4px;height:4px;background:rgb(0,0,255)'></div></div>",
            &[(30, 30)],
        )
        .await?;
        assert_eq!(backdrop_fragment_count(&snapshot), 0);
        assert_eq!(pixels[0], [255, 0, 0, 255]);
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("hidden backdrop fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn backdrop_filter_is_clipped_by_ancestor_overflow() {
    run_page_vm_async_test(async move {
        let (pixels, snapshot) = backdrop_fixture_pixels(
            "#box{left:10px;top:10px;width:40px;height:40px;overflow:hidden}#glass{left:0;top:0;width:100px;height:40px;backdrop-filter:invert(1)}",
            "<div id=box><div id=glass></div></div>",
            &[(30, 30), (80, 30)],
        )
        .await?;
        // Inverted inside the overflow box, untouched outside it.
        assert_eq!(pixels[0], [0, 255, 255, 255]);
        assert_eq!(pixels[1], [255, 0, 0, 255]);
        // The clip wraps the backdrop and counts among the scopes it skips.
        let own_scopes = snapshot
            .fragments
            .iter()
            .find_map(|fragment| match fragment {
                moli_layout::PaintFragment::BackdropFilter { own_scopes, .. } => Some(*own_scopes),
                _ => None,
            })
            .expect("one backdrop fragment");
        assert!(own_scopes >= 1, "{own_scopes}");
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("overflow backdrop fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn backdrop_filter_ancestor_is_a_backdrop_root_for_descendant_backdrops() {
    run_page_vm_async_test(async move {
        // The parent's backdrop-filter is a Backdrop Root, so the larger child
        // reads only the parent's content: nothing is inverted outside it.
        let (pixels, snapshot) = backdrop_fixture_pixels(
            "#parent{left:10px;top:10px;width:40px;height:40px;backdrop-filter:brightness(1)}#child{left:0;top:0;width:100px;height:40px;backdrop-filter:invert(1)}",
            "<div id=parent><div id=child></div></div>",
            &[(30, 30), (80, 30)],
        )
        .await?;
        assert_eq!(backdrop_fragment_count(&snapshot), 2);
        assert_eq!(pixels[1], [255, 0, 0, 255]);
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("nested backdrop fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn clip_path_ancestor_is_a_backdrop_root_for_descendant_backdrops() {
    run_page_vm_async_test(async move {
        // Inside the clip-path element the child's backdrop is that element's
        // own (empty) content, so the red canvas behind it is not inverted.
        let (pixels, _) = backdrop_fixture_pixels(
            "#parent{left:10px;top:10px;width:40px;height:40px;clip-path:inset(0)}#child{left:0;top:0;width:40px;height:40px;backdrop-filter:invert(1)}",
            "<div id=parent><div id=child></div></div>",
            &[(30, 30)],
        )
        .await?;
        assert_eq!(pixels[0], [255, 0, 0, 255]);
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("clip-path backdrop fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn documents_without_backdrop_filter_emit_no_backdrop_root_markers() {
    run_page_vm_async_test(async move {
        let (_, snapshot) = backdrop_fixture_pixels(
            "#parent{left:10px;top:10px;width:40px;height:40px;clip-path:inset(0)}",
            "<div id=parent></div>",
            &[(30, 30)],
        )
        .await?;
        assert!(
            !snapshot
                .fragments
                .iter()
                .any(|fragment| matches!(fragment, moli_layout::PaintFragment::BackdropRoot))
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("marker-free fixture should run");
}
