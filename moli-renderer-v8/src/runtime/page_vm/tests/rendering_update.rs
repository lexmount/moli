use super::*;

use base64::Engine as _;

use super::super::main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task;

use crate::page_task_queue::{
    PageRenderingUpdateTargetEffect, RendererPageRenderingUpdateTaskKind,
};
use crate::script_vm::MainDocumentLifecycleBody;

async fn dispatch_main_document_domcontentloaded_for_rendering_test(
    page_vm: &mut PageVm,
) -> anyhow::Result<crate::frame_owner_model::FrameDocumentTaskOwner> {
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("main Document owner");
    let interactive = page_vm
        .vm_mut()
        .finish_current_main_document_parsing(owner)
        .expect("parser completion should prepare the interactive transition");
    execute_main_document_lifecycle_on_owner_local_task(
        page_vm,
        MainDocumentLifecycleBody::Interactive(interactive),
    )
    .await?;
    execute_main_document_lifecycle_on_owner_local_task(
        page_vm,
        MainDocumentLifecycleBody::DomContentLoaded { owner },
    )
    .await?;
    Ok(owner)
}

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
            moli_layout::LayoutFlushReason::SynchronousGeometry,
            viewport,
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
async fn geometry_batch_reuses_latest_tree_until_fresh_paint_replaces_it() {
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
        let first = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            moli_layout::LayoutFlushReason::SynchronousGeometry,
            moli_layout::LayoutViewport::new(320, 200, 1.0),
            &batch,
        )?;
        let after_first = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_first = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!after_first.0);
        assert_eq!(after_first.1, before.1 + 1);
        assert_eq!(cache_after_first.0, cache_before.0);
        assert_eq!(cache_after_first.1, cache_before.1 + 1);
        assert_eq!(cache_after_first.2, cache_before.2 + 1);
        assert_eq!(first.answers.len(), batch.queries.len());
        assert_eq!(
            first.metrics.reason,
            moli_layout::LayoutFlushReason::SynchronousGeometry
        );
        assert_eq!(first.metrics.paint_operation_count, 0);
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
        let second_viewport = moli_layout::LayoutViewport::new(480, 300, 2.0);
        let second = moli_layout::GeometryProvider::answer(
            page_vm.vm_mut(),
            moli_layout::LayoutFlushReason::SynchronousGeometry,
            second_viewport,
            &batch,
        )?;
        let after_second = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_second = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!after_second.0);
        assert_eq!(after_second.1, before.1 + 1);
        assert_eq!(cache_after_second.0, cache_before.0 + 1);
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
                if metrics.viewport == second_viewport
        ));

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::LayoutViewport::new(320, 200, 1.0))?
            .expect("current document screenshot layout");
        let after_screenshot = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_screenshot = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert!(!after_screenshot.0);
        assert_eq!(after_screenshot.1, before.1 + 2);
        assert_eq!(cache_after_screenshot.0, cache_before.0 + 1);
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
            moli_layout::LayoutFlushReason::SynchronousGeometry,
            moli_layout::LayoutViewport::new(320, 200, 1.0),
            &batch,
        )?;
        let after_third = page_vm.vm().layout_pass_observability_for_test();
        let cache_after_third = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test();
        assert_eq!(after_third.1, before.1 + 2);
        assert_eq!(cache_after_third.0, cache_before.0 + 2);
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
            moli_layout::LayoutFlushReason::SynchronousGeometry,
            moli_layout::LayoutViewport::new(320, 200, 1.0),
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
            moli_layout::LayoutFlushReason::SynchronousGeometry,
            moli_layout::LayoutViewport::new(320, 200, 1.0),
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
child.documentElement.style.cssText = 'margin:0;padding:0;background:rgb(0,255,255)';
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
nestedDocument.documentElement.style.cssText = 'margin:0;padding:0;background:rgb(255,0,255)';
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
async fn positioned_layout_matches_chromium_auto_margin_and_relative_inset_rules() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/positioned-layout.html")?,
        );
        page_vm
            .vm_mut()
            .set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 1440,
                inner_height: 620,
                outer_width: 1440,
                outer_height: 620,
                device_pixel_ratio: 1.0,
                screen_width: 1440,
                screen_height: 620,
                screen_avail_width: 1440,
                screen_avail_height: 620,
            }))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;font-size:16px;background:white}
#centered{position:fixed;left:0;right:0;top:0;width:975px;height:20px;margin-left:auto;margin-right:auto;background:red}
#definite-parent{position:absolute;left:0;top:30px;width:100px;height:400px}
#definite-child{position:relative;top:calc(max(120px,100% - 12.6875rem));width:10px;height:10px;background:blue}
#indefinite-parent{position:absolute;left:0;top:500px;width:100px;min-height:100px}
#indefinite-child{position:relative;top:calc(10px + 10%);width:10px;height:100px;background:lime}
</style>`;
document.body.innerHTML = `<div id=centered></div><div id=definite-parent><div id=definite-child></div></div><div id=indefinite-parent><div id=indefinite-child></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['centered','definite-parent','definite-child','indefinite-parent','indefinite-child'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            // Chromium retains the half-pixel origin in DOM geometry.
            ("centered", [232.5, 0.0, 975.0, 20.0]),
            ("definite-parent", [0.0, 30.0, 100.0, 400.0]),
            ("definite-child", [0.0, 227.0, 10.0, 10.0]),
            ("indefinite-parent", [0.0, 500.0, 100.0, 100.0]),
            ("indefinite-child", [0.0, 500.0, 10.0, 100.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1440, 620, 1.0))?
            .expect("positioned fixture must retain a layout root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        // Blink retains the fractional layout origin for DOM geometry, but
        // box background painting snaps the fill rect to device pixels.
        // The 975px box therefore paints [233, 1208), with no half-covered
        // pixel at either edge at device scale 1.
        assert_eq!(pixel(232, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(233, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(1207, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(1208, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(5, 230), [0, 0, 255, 255]);
        assert_eq!(pixel(5, 505), [0, 255, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("positioned layout fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn containment_matches_chromium_containing_block_eligibility_and_paint_clip() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/containment-layout.html")?,
        );
        page_vm
            .vm_mut()
            .set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 800,
                inner_height: 600,
                outer_width: 800,
                outer_height: 600,
                device_pixel_ratio: 1.0,
                screen_width: 800,
                screen_height: 600,
                screen_avail_width: 800,
                screen_avail_height: 600,
            }))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
#stage{position:relative;width:800px;height:600px}
.cb{position:absolute;top:20px;box-sizing:border-box;width:160px;height:100px;border:4px solid black;padding:6px;background:rgb(230,230,230)}
#layout{left:20px;contain:layout}#paint{left:220px;contain:paint}#content{left:420px;contain:content}#strict{left:620px;contain:strict}
.abs{position:absolute;right:10px;top:8px;width:20px;height:16px;background:red}.fixed{position:fixed;left:12px;top:10px;width:18px;height:14px;background:blue}
.overflow{position:absolute;left:-20px;bottom:-20px;width:40px;height:40px;background:lime}
#shadow-host{position:absolute;left:100px;top:180px;width:300px;height:100px;background:rgb(240,240,240)}
#inline-outer{position:absolute;left:500px;top:180px;width:250px;height:100px;background:rgb(240,240,240)}
#inline-container{contain:paint}#inline-abs{position:absolute;right:5px;top:6px;width:20px;height:15px;background:purple}
#will-contain{position:absolute;left:20px;top:320px;width:140px;height:80px;will-change:contain;background:silver}
#will-position{position:absolute;left:180px;top:320px;width:140px;height:80px;will-change:position;background:silver}
#will-transform{position:absolute;left:340px;top:320px;width:140px;height:80px;will-change:transform;background:silver}
#content-auto{position:absolute;left:380px;top:500px;width:140px;height:80px;content-visibility:auto;background:silver}
.will-abs{position:absolute;right:7px;top:8px;width:20px;height:15px}.will-fixed{position:fixed;left:9px;top:10px;width:18px;height:14px}
#bfc-row{position:absolute;left:20px;top:430px;width:500px}.bfc{display:block;position:relative;width:100px;background:yellow}
.bfc.plain{left:0}.bfc.layout{left:120px;contain:layout}.bfc.paint{left:220px;contain:paint}.float{float:left;width:20px;height:30px;background:red}
#table{position:absolute;left:560px;top:320px;width:220px;height:100px}#table-row{contain:paint}#table-cell-contained{contain:paint}
.table-abs{position:absolute;right:3px;top:4px;width:10px;height:10px}
</style>`;
document.body.innerHTML = `<div id=stage>
<div id=layout class=cb><div id=layout-abs class=abs></div><div id=layout-fixed class=fixed></div><div class=overflow></div></div>
<div id=paint class=cb><div id=paint-abs class=abs></div><div id=paint-fixed class=fixed></div><div class=overflow></div></div>
<div id=content class=cb><div id=content-abs class=abs></div><div id=content-fixed class=fixed></div><div class=overflow></div></div>
<div id=strict class=cb><div id=strict-abs class=abs></div><div id=strict-fixed class=fixed></div><div class=overflow></div></div>
<div id=shadow-host></div><div id=inline-outer><span id=inline-container>inline<div id=inline-abs></div></span></div>
<div id=will-contain><div id=will-contain-abs class=will-abs></div><div id=will-contain-fixed class=will-fixed></div></div>
<div id=will-position><div id=will-position-abs class=will-abs></div><div id=will-position-fixed class=will-fixed></div></div>
<div id=will-transform><div id=will-transform-abs class=will-abs></div><div id=will-transform-fixed class=will-fixed></div></div>
<div id=content-auto><div id=content-auto-abs class=will-abs></div><div id=content-auto-fixed class=will-fixed></div></div>
<div id=bfc-row><div id=plain-bfc class='bfc plain'><div class=float></div></div><div id=layout-bfc class='bfc layout'><div class=float></div></div><div id=paint-bfc class='bfc paint'><div class=float></div></div></div>
<table id=table><tbody><tr id=table-row><td><div id=row-abs class=table-abs></div></td></tr><tr><td id=table-cell-contained><div id=cell-abs class=table-abs></div></td></tr></tbody></table>
</div>`;
const shadow=document.getElementById('shadow-host').attachShadow({mode:'open'});
shadow.innerHTML=`<style>.root{contain:content;margin-left:40px;width:200px;height:100px;background:orange}.action{position:absolute;right:8px;top:9px;width:30px;height:20px;background:black}</style><div id=root class=root><div id=shadow-action class=action></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let result = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const shadow=document.getElementById('shadow-host').shadowRoot;
const ids=['layout','layout-abs','layout-fixed','paint','paint-abs','paint-fixed','content','content-abs','content-fixed','strict','strict-abs','strict-fixed','inline-outer','inline-container','inline-abs','will-contain','will-contain-abs','will-contain-fixed','will-position','will-position-abs','will-position-fixed','will-transform','will-transform-abs','will-transform-fixed','content-auto','content-auto-abs','content-auto-fixed','plain-bfc','layout-bfc','paint-bfc'];
const rect=e=>{const r=e.getBoundingClientRect();return [r.x,r.y,r.width,r.height]};
const geometry=Object.fromEntries(ids.map(id=>[id,rect(document.getElementById(id))]));
geometry['shadow-host']=rect(document.getElementById('shadow-host'));geometry['shadow-root']=rect(shadow.getElementById('root'));geometry['shadow-action']=rect(shadow.getElementById('shadow-action'));
const offsets={};for(const id of ['layout-abs','layout-fixed','paint-abs','paint-fixed','content-abs','content-fixed','strict-abs','strict-fixed','inline-abs','will-contain-abs','will-contain-fixed','will-position-abs','will-position-fixed','will-transform-abs','will-transform-fixed','content-auto-abs','content-auto-fixed','row-abs','cell-abs']) offsets[id]=document.getElementById(id).offsetParent?.id??null;
offsets['shadow-action']=shadow.getElementById('shadow-action').offsetParent?.id??null;return {geometry,offsets}})())"#,
        )?;
        let result: serde_json::Value = serde_json::from_str(&result)?;
        let geometry = &result["geometry"];
        for (id, expected) in [
            ("layout", [20.0, 20.0, 160.0, 100.0]),
            ("layout-abs", [146.0, 32.0, 20.0, 16.0]),
            ("layout-fixed", [36.0, 34.0, 18.0, 14.0]),
            ("paint-abs", [346.0, 32.0, 20.0, 16.0]),
            ("paint-fixed", [236.0, 34.0, 18.0, 14.0]),
            ("content-abs", [546.0, 32.0, 20.0, 16.0]),
            ("content-fixed", [436.0, 34.0, 18.0, 14.0]),
            ("strict-abs", [746.0, 32.0, 20.0, 16.0]),
            ("strict-fixed", [636.0, 34.0, 18.0, 14.0]),
            ("inline-abs", [725.0, 186.0, 20.0, 15.0]),
            ("shadow-root", [140.0, 180.0, 200.0, 100.0]),
            ("shadow-action", [302.0, 189.0, 30.0, 20.0]),
            ("will-contain-abs", [133.0, 328.0, 20.0, 15.0]),
            ("will-contain-fixed", [29.0, 330.0, 18.0, 14.0]),
            ("will-position-abs", [293.0, 328.0, 20.0, 15.0]),
            ("will-position-fixed", [9.0, 10.0, 18.0, 14.0]),
            ("will-transform-abs", [453.0, 328.0, 20.0, 15.0]),
            ("will-transform-fixed", [349.0, 330.0, 18.0, 14.0]),
            ("content-auto", [380.0, 500.0, 140.0, 80.0]),
            ("content-auto-abs", [493.0, 508.0, 20.0, 15.0]),
            ("content-auto-fixed", [389.0, 510.0, 18.0, 14.0]),
            ("plain-bfc", [20.0, 430.0, 100.0, 0.0]),
            ("layout-bfc", [160.0, 430.0, 100.0, 30.0]),
            ("paint-bfc", [240.0, 460.0, 100.0, 30.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {result}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; result={result}"
                );
            }
        }
        for (id, expected) in [
            ("layout-abs", Some("layout")),
            ("layout-fixed", Some("layout")),
            ("paint-abs", Some("paint")),
            ("paint-fixed", Some("paint")),
            ("content-abs", Some("content")),
            ("content-fixed", Some("content")),
            ("strict-abs", Some("strict")),
            ("strict-fixed", Some("strict")),
            ("inline-abs", Some("inline-outer")),
            ("will-contain-abs", Some("will-contain")),
            ("will-contain-fixed", Some("will-contain")),
            ("will-position-abs", Some("will-position")),
            ("will-position-fixed", None),
            ("will-transform-abs", Some("will-transform")),
            ("will-transform-fixed", Some("will-transform")),
            ("content-auto-abs", Some("content-auto")),
            ("content-auto-fixed", Some("content-auto")),
            ("row-abs", Some("table")),
            ("cell-abs", Some("table-cell-contained")),
            ("shadow-action", Some("root")),
        ] {
            assert_eq!(
                result["offsets"][id].as_str(),
                expected,
                "unexpected offsetParent for {id}: {result}"
            );
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("containment fixture must retain a layout root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(pixel(10, 105), [0, 255, 0, 255]);
        assert_eq!(pixel(210, 105), [255, 255, 255, 255]);
        assert_eq!(pixel(230, 105), [0, 255, 0, 255]);
        assert_eq!(pixel(410, 105), [255, 255, 255, 255]);
        assert_eq!(pixel(430, 105), [0, 255, 0, 255]);
        assert_eq!(pixel(610, 105), [255, 255, 255, 255]);
        assert_eq!(pixel(630, 105), [0, 255, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("containment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn fixed_flex_auto_margin_consumes_free_space_once() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/fixed-flex-layout.html")?,
        );
        page_vm
            .vm_mut()
            .set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 1440,
                inner_height: 900,
                outer_width: 1440,
                outer_height: 900,
                device_pixel_ratio: 1.0,
                screen_width: 1440,
                screen_height: 900,
                screen_avail_width: 1440,
                screen_avail_height: 900,
            }))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#stage{position:relative;width:80%;height:900px;margin:auto}
#header{display:flex;position:fixed;left:0;box-sizing:border-box;justify-content:space-between;width:100%;min-width:768px;height:5vh;margin:10px 0;padding:0 24px}
#left{width:596px;height:45px;margin-right:auto;background:red}
#right{width:308px;height:45px;background:blue}
</style>`;
document.body.innerHTML = `<main id=stage><header id=header><div id=left></div><div id=right></div></header></main>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['stage','header','left','right'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("stage", [144.0, 0.0, 1152.0, 900.0]),
            ("header", [0.0, 10.0, 1440.0, 45.0]),
            ("left", [24.0, 10.0, 596.0, 45.0]),
            ("right", [1108.0, 10.0, 308.0, 45.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("fixed flex auto-margin fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn atomic_inline_auto_width_shrink_wraps_before_parent_text_alignment() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-fit-content.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.line{height:40px;text-align:right;font-size:0;background:rgb(1,2,3)}
#wide{width:1236px;margin-left:112px}
#narrow{width:200px;margin-left:20px}
#flex,#grid,#specified,#margined,#table{width:400px}
.card{display:inline-block;height:40px;text-align:left;background:rgb(4,5,6)}
.headline{height:40px}
.primary,.secondary{display:inline-block;height:40px}
.primary{width:162px;background:rgb(7,8,9)}
.secondary{width:84px;margin-left:8px;background:rgb(10,11,12)}
.pair-a,.pair-b{width:100px;height:40px;background:rgb(13,14,15)}
.pair-b{width:50px;background:rgb(16,17,18)}
#flex-card{display:inline-flex;column-gap:10px}
#grid-card{display:inline-grid;grid-template-columns:100px 50px;column-gap:10px}
#specified-card{width:300px}
#margined-card{margin-left:10px;margin-right:20px}
#table-card{display:inline-table;border-collapse:separate;border-spacing:0}
#table-card td{height:40px;padding:0}
#table-a{width:100px}#table-b{width:50px}
</style>`;
document.body.innerHTML = `
<div id=wide class=line><div id=wide-card class=card><div class=headline><span id=wide-primary class=primary></span> <span id=wide-secondary class=secondary></span></div></div></div>
<div id=narrow class=line><div id=narrow-card class=card><div class=headline><span id=narrow-primary class=primary></span> <span id=narrow-secondary class=secondary></span></div></div></div>
<div id=flex class=line><div id=flex-card><div class=pair-a></div><div class=pair-b></div></div></div>
<div id=grid class=line><div id=grid-card><div class=pair-a></div><div class=pair-b></div></div></div>
<div id=specified class=line><div id=specified-card class=card><div class=headline><span class=primary></span> <span class=secondary></span></div></div></div>
<div id=margined class=line><div id=margined-card class=card><div class=headline><span class=primary></span> <span class=secondary></span></div></div></div>
<div id=table class=line><table id=table-card><tr><td id=table-a></td><td id=table-b></td></tr></table></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['wide','wide-card','wide-primary','wide-secondary','narrow','narrow-card','narrow-primary','narrow-secondary','flex','flex-card','grid','grid-card','specified','specified-card','margined','margined-card','table','table-card','table-a','table-b'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("wide", [112.0, 0.0, 1236.0, 40.0]),
            ("wide-card", [1094.0, 0.0, 254.0, 40.0]),
            ("wide-primary", [1094.0, 0.0, 162.0, 40.0]),
            ("wide-secondary", [1264.0, 0.0, 84.0, 40.0]),
            ("narrow", [20.0, 40.0, 200.0, 40.0]),
            ("narrow-card", [20.0, 40.0, 200.0, 40.0]),
            ("narrow-primary", [20.0, 40.0, 162.0, 40.0]),
            ("narrow-secondary", [28.0, 80.0, 84.0, 40.0]),
            ("flex", [0.0, 80.0, 400.0, 40.0]),
            ("flex-card", [240.0, 80.0, 160.0, 40.0]),
            ("grid", [0.0, 120.0, 400.0, 40.0]),
            ("grid-card", [240.0, 120.0, 160.0, 40.0]),
            ("specified", [0.0, 160.0, 400.0, 40.0]),
            ("specified-card", [100.0, 160.0, 300.0, 40.0]),
            ("margined", [0.0, 200.0, 400.0, 40.0]),
            ("margined-card", [126.0, 200.0, 254.0, 40.0]),
            ("table", [0.0, 240.0, 400.0, 40.0]),
            ("table-card", [250.0, 240.0, 150.0, 40.0]),
            ("table-a", [250.0, 240.0, 100.0, 40.0]),
            ("table-b", [350.0, 240.0, 50.0, 40.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic inline fit-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn center_user_agent_alignment_centers_atomic_inline_children() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/center-user-agent-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#host{width:584px;margin-left:100px;font-size:0}
#first,#second{display:inline-block;height:20px}
#first{width:120px}
#second{width:100px}
</style>`;
document.body.innerHTML = `<center id=host><span id=first></span><span id=second></span></center>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['host','first','second'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("host", [100.0, 0.0, 584.0, 20.0]),
            ("first", [282.0, 0.0, 120.0, 20.0]),
            ("second", [402.0, 0.0, 100.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("center user-agent alignment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn absolute_auto_width_from_an_inline_formatting_context_shrinks_to_fit() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/absolute-fit-content.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#stage{position:relative;width:800px;height:600px}
.case{position:absolute;height:70px;font-size:0}
.prefix{display:inline-block;width:30px;height:10px}
.abs{position:absolute;top:5px}
.primary,.secondary{display:inline-block;height:20px}
.primary{width:160px}
.secondary{width:80px;margin-left:8px}
#left-max-case{left:20px;top:20px;width:300px}
#left-limit-case{left:380px;top:20px;width:200px}
#left-min-case{left:640px;top:20px;width:120px}
#right-max-case{left:20px;top:110px;width:300px}
#stretch-case{left:380px;top:110px;width:300px}
#margin-min-case{left:20px;top:200px;width:200px}
#max-clamp-case{left:260px;top:200px;width:300px}
#min-clamp-case{left:20px;top:290px;width:300px}
#specified-case{left:380px;top:290px;width:300px}
#static-ltr-case{left:20px;top:380px;width:300px}
#static-rtl-case{left:380px;top:380px;width:300px;direction:rtl;text-align:left}
#flex-case{display:flex;left:20px;top:470px;width:300px}
#left-max{left:20px}
#left-limit{left:20px}
#left-min{left:10px}
#right-max{right:20px}
#stretch{left:20px;right:30px}
#margin-min{left:20px;margin-left:10px;margin-right:15px}
#max-clamp{left:20px;max-width:200px}
#min-clamp{left:20px;min-width:260px}
#specified{left:20px;width:120px}
#flex-abs{left:20px}
</style>`;
document.body.innerHTML = `<div id=stage>
  <div id=left-max-case class=case><span class=prefix></span><div id=left-max class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=left-limit-case class=case><span class=prefix></span><div id=left-limit class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=left-min-case class=case><span class=prefix></span><div id=left-min class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=right-max-case class=case><span class=prefix></span><div id=right-max class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=stretch-case class=case><span class=prefix></span><div id=stretch class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=margin-min-case class=case><span class=prefix></span><div id=margin-min class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=max-clamp-case class=case><span class=prefix></span><div id=max-clamp class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=min-clamp-case class=case><span class=prefix></span><div id=min-clamp class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=specified-case class=case><span class=prefix></span><div id=specified class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=static-ltr-case class=case><span class=prefix></span><span id=static-ltr class=abs><span class=primary></span> <span class=secondary></span></span></div>
  <div id=static-rtl-case class=case><span class=prefix></span><span id=static-rtl class=abs><span class=primary></span> <span class=secondary></span></span></div>
  <div id=flex-case class=case><span class=prefix></span><div id=flex-abs class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['left-max-case','left-max','left-limit-case','left-limit','left-min-case','left-min','right-max-case','right-max','stretch-case','stretch','margin-min-case','margin-min','max-clamp-case','max-clamp','min-clamp-case','min-clamp','specified-case','specified','static-ltr-case','static-ltr','static-rtl-case','static-rtl','flex-case','flex-abs'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("left-max-case", [20.0, 20.0, 300.0, 70.0]),
            ("left-max", [40.0, 25.0, 248.0, 20.0]),
            ("left-limit-case", [380.0, 20.0, 200.0, 70.0]),
            ("left-limit", [400.0, 25.0, 180.0, 40.0]),
            ("left-min-case", [640.0, 20.0, 120.0, 70.0]),
            ("left-min", [650.0, 25.0, 160.0, 40.0]),
            ("right-max", [52.0, 115.0, 248.0, 20.0]),
            ("stretch", [400.0, 115.0, 250.0, 20.0]),
            ("margin-min", [50.0, 205.0, 160.0, 40.0]),
            ("max-clamp", [280.0, 205.0, 200.0, 40.0]),
            ("min-clamp", [40.0, 295.0, 260.0, 20.0]),
            ("specified", [400.0, 295.0, 120.0, 40.0]),
            ("static-ltr", [50.0, 385.0, 248.0, 20.0]),
            ("flex-abs", [40.0, 475.0, 248.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        // Keep the RTL auto-inset branch in the sizing contract. Parley 0.10
        // does not bidi-reorder zero-width out-of-flow placeholders, so its
        // physical static-position x remains a separate inline-bidi gap; the
        // Chromium differential records the correct x without baking the
        // current approximation into this regression.
        let static_rtl = geometry["static-rtl"]
            .as_array()
            .unwrap_or_else(|| panic!("missing geometry for static-rtl: {geometry}"));
        for (index, expected) in [(2, 160.0), (3, 40.0)] {
            let actual = static_rtl[index].as_f64().expect("numeric geometry") as f32;
            assert!(
                (actual - expected).abs() <= 0.05,
                "static-rtl[{index}]: expected {expected}, got {actual}; geometry={geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("absolute fit-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn floated_auto_width_inline_formatting_contexts_shrink_to_fit() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/float-fit-content.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#stage{position:relative;width:1200px;height:620px}
.case{position:absolute;height:70px;font-size:0}
.auto-float{float:left}
.right{float:right}
.primary,.secondary{display:inline-block;height:20px}
.primary{width:160px}
.secondary{width:80px;margin-left:8px}
#baidu-case{left:20px;top:20px;width:1076px}
#baidu-logo{float:left;margin-top:17px}
#baidu-logo img{display:inline;width:101px;height:33px}
#baidu-main{float:left;width:748px;height:45px;margin:15px 0 8px 18px}
#max-case{left:20px;top:120px;width:300px}
#limit-case{left:380px;top:120px;width:200px}
#min-case{left:640px;top:120px;width:120px}
#right-case{left:20px;top:220px;width:300px}
#margin-case{left:380px;top:220px;width:200px}
#max-clamp-case{left:640px;top:220px;width:300px}
#min-clamp-case{left:20px;top:320px;width:300px}
#specified-case{left:380px;top:320px;width:300px}
#edge-case{left:740px;top:320px;width:300px}
#block-control-case{left:20px;top:420px;width:300px}
#replaced-control-case{left:380px;top:420px;width:300px}
#stretch-control-case{left:740px;top:420px;width:300px}
#inline-margin-case{left:20px;top:520px;width:200px}
#negative-margin-case{left:380px;top:520px;width:200px}
#inline-negative-margin-case{left:740px;top:520px;width:200px}
#margin-float{margin-left:10px;margin-right:15px}
#inline-margin-float{margin-left:10px;margin-right:15px}
#negative-margin-float,#inline-negative-margin-float{margin-left:-10px;margin-right:-15px}
#max-clamp{max-width:200px}
#min-clamp{min-width:260px}
#specified{width:120px}
#edge{margin-left:10px;margin-right:15px;padding:0 10px;border:2px solid black}
#block-control{float:left}
#block-control>div{width:180px;height:20px}
#replaced-control{float:left;width:101px;height:33px}
</style>`;
document.body.innerHTML = `<div id=stage>
  <div id=baidu-case class=case><a id=baidu-logo><img id=baidu-logo-image width=101 height=33 alt=""></a><div id=baidu-main></div></div>
  <div id=max-case class=case><div id=max class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=limit-case class=case><div id=limit class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=min-case class=case><div id=min class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=right-case class=case><div id=right class="auto-float right"><span class=primary></span> <span class=secondary></span></div></div>
  <div id=margin-case class=case><div id=margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=max-clamp-case class=case><div id=max-clamp class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=min-clamp-case class=case><div id=min-clamp class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=specified-case class=case><div id=specified class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=edge-case class=case><div id=edge class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=block-control-case class=case><div id=block-control><div></div></div></div>
  <div id=replaced-control-case class=case><img id=replaced-control width=101 height=33 alt=""></div>
  <div id=stretch-control-case class=case><div id=stretch-control><span class=primary></span> <span class=secondary></span></div></div>
  <div id=inline-margin-case class=case><span></span><div id=inline-margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=negative-margin-case class=case><div id=negative-margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=inline-negative-margin-case class=case><span></span><div id=inline-negative-margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['baidu-case','baidu-logo','baidu-logo-image','baidu-main','max-case','max','limit-case','limit','min-case','min','right-case','right','margin-case','margin-float','max-clamp-case','max-clamp','min-clamp-case','min-clamp','specified-case','specified','edge-case','edge','block-control-case','block-control','replaced-control-case','replaced-control','stretch-control-case','stretch-control','inline-margin-case','inline-margin-float','negative-margin-case','negative-margin-float','inline-negative-margin-case','inline-negative-margin-float'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("baidu-case", [20.0, 20.0, 1076.0, 70.0]),
            ("baidu-logo", [20.0, 37.0, 101.0, 33.0]),
            ("baidu-logo-image", [20.0, 37.0, 101.0, 33.0]),
            ("baidu-main", [139.0, 35.0, 748.0, 45.0]),
            ("max-case", [20.0, 120.0, 300.0, 70.0]),
            ("max", [20.0, 120.0, 248.0, 20.0]),
            ("limit-case", [380.0, 120.0, 200.0, 70.0]),
            ("limit", [380.0, 120.0, 200.0, 40.0]),
            ("min-case", [640.0, 120.0, 120.0, 70.0]),
            ("min", [640.0, 120.0, 160.0, 40.0]),
            ("right-case", [20.0, 220.0, 300.0, 70.0]),
            ("right", [72.0, 220.0, 248.0, 20.0]),
            ("margin-case", [380.0, 220.0, 200.0, 70.0]),
            ("margin-float", [390.0, 220.0, 175.0, 40.0]),
            ("max-clamp-case", [640.0, 220.0, 300.0, 70.0]),
            ("max-clamp", [640.0, 220.0, 200.0, 40.0]),
            ("min-clamp-case", [20.0, 320.0, 300.0, 70.0]),
            ("min-clamp", [20.0, 320.0, 260.0, 20.0]),
            ("specified-case", [380.0, 320.0, 300.0, 70.0]),
            ("specified", [380.0, 320.0, 120.0, 40.0]),
            ("edge-case", [740.0, 320.0, 300.0, 70.0]),
            ("edge", [750.0, 320.0, 272.0, 24.0]),
            ("block-control-case", [20.0, 420.0, 300.0, 70.0]),
            ("block-control", [20.0, 420.0, 180.0, 20.0]),
            ("replaced-control-case", [380.0, 420.0, 300.0, 70.0]),
            ("replaced-control", [380.0, 420.0, 101.0, 33.0]),
            ("stretch-control-case", [740.0, 420.0, 300.0, 70.0]),
            ("stretch-control", [740.0, 420.0, 300.0, 20.0]),
            ("inline-margin-case", [20.0, 520.0, 200.0, 70.0]),
            ("inline-margin-float", [30.0, 520.0, 175.0, 40.0]),
            ("negative-margin-case", [380.0, 520.0, 200.0, 70.0]),
            ("negative-margin-float", [370.0, 520.0, 225.0, 40.0]),
            ("inline-negative-margin-case", [740.0, 520.0, 200.0, 70.0]),
            (
                "inline-negative-margin-float",
                [730.0, 520.0, 225.0, 40.0],
            ),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("float fit-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn intrinsic_width_keywords_match_chromium_across_formatting_contexts() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-width.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}.case{position:absolute;width:300px;height:70px;font-size:0}.probe{height:20px}.a,.b{display:inline-block;height:20px}.a{width:160px}.b{width:80px;margin-left:8px}
#min-case{left:20px;top:20px}#max-case{left:380px;top:20px}#fit-case{left:740px;top:20px}#fit-narrow-case{left:20px;top:120px;width:200px}#fit-min-case{left:380px;top:120px;width:120px}#min-clamp-case{left:740px;top:120px}#max-clamp-case{left:20px;top:220px}#conflict-case{left:380px;top:220px}#content-box-case{left:740px;top:220px}#border-box-case{left:20px;top:320px}#flex-min-case{left:380px;top:320px;display:flex}#flex-max-case{left:740px;top:320px;display:flex}#flex-cross-case{left:20px;top:420px;display:flex;flex-direction:column}#grid-min-case{left:380px;top:420px;display:grid}#grid-max-case{left:740px;top:420px;display:grid}#absolute-min-case{left:20px;top:520px}#absolute-fit-case{left:380px;top:520px;width:200px}#float-min-case{left:740px;top:520px}#float-max-case{left:20px;top:620px}#replaced-case{left:380px;top:620px}#stretch-case{left:740px;top:620px}#webkit-fill-case{left:20px;top:720px}#aspect-block-case{left:380px;top:720px}#aspect-flex-case{left:740px;top:720px}#aspect-grid-case{left:20px;top:820px}#flex-grow-case{left:380px;top:820px;display:flex}#flex-shrink-case{left:740px;top:820px;width:120px;display:flex}#flex-basis-content-case{left:20px;top:920px;display:flex}#auto-grid-min-case{left:380px;top:920px}#auto-grid-max-case{left:740px;top:920px}#absolute-inset-fit-case{left:20px;top:1020px;width:200px}#float-fit-margin-case{left:380px;top:1020px;width:200px}#float-stretch-margin-case{left:740px;top:1020px;width:200px}#inline-min-case{left:20px;top:1120px}#inline-max-case{left:380px;top:1120px}#inline-fit-case{left:740px;top:1120px;width:200px}#min-fit-case{left:20px;top:1220px;width:200px}#max-fit-case{left:380px;top:1220px;width:200px}#min-stretch-case{left:740px;top:1220px}#max-stretch-case{left:20px;top:1320px}#min-webkit-fill-case{left:380px;top:1320px}#max-webkit-fill-case{left:740px;top:1320px}
.edge{padding:0 10px;border:2px solid black}.flex-item{flex:0 0 auto}.absolute{position:absolute;left:0;right:0}.float{float:left}.stretch{margin-left:10px;margin-right:15px}.aspect-probe{height:20px;aspect-ratio:20}.grow{flex:1 1 auto}.shrink{flex:0 1 auto;min-width:0}.basis-content{flex:0 0 content}.auto-grid{display:grid;width:max-content}
</style>`;
const content='<span class=a></span> <span class=b></span>';
document.body.innerHTML = `
<div id=min-case class=case><div id=min class=probe style="width:min-content">${content}</div></div>
<div id=max-case class=case><div id=max class=probe style="width:max-content">${content}</div></div>
<div id=fit-case class=case><div id=fit class=probe style="width:fit-content">${content}</div></div>
<div id=fit-narrow-case class=case><div id=fit-narrow class=probe style="width:fit-content">${content}</div></div>
<div id=fit-min-case class=case><div id=fit-min class=probe style="width:fit-content">${content}</div></div>
<div id=min-clamp-case class=case><div id=min-clamp class=probe style="width:100px;min-width:max-content">${content}</div></div>
<div id=max-clamp-case class=case><div id=max-clamp class=probe style="width:300px;max-width:min-content">${content}</div></div>
<div id=conflict-case class=case><div id=conflict class=probe style="width:200px;min-width:max-content;max-width:min-content">${content}</div></div>
<div id=content-box-case class=case><div id=content-box class="probe edge" style="box-sizing:content-box;width:min-content">${content}</div></div>
<div id=border-box-case class=case><div id=border-box class="probe edge" style="box-sizing:border-box;width:min-content">${content}</div></div>
<div id=flex-min-case class=case><div id=flex-min class="probe flex-item" style="width:min-content">${content}</div></div>
<div id=flex-max-case class=case><div id=flex-max class="probe flex-item" style="width:max-content">${content}</div></div>
<div id=flex-cross-case class=case><div id=flex-cross class=probe style="width:min-content">${content}</div></div>
<div id=grid-min-case class=case><div id=grid-min class=probe style="width:min-content">${content}</div></div>
<div id=grid-max-case class=case><div id=grid-max class=probe style="width:max-content">${content}</div></div>
<div id=absolute-min-case class=case><div id=absolute-min class="probe absolute" style="width:min-content">${content}</div></div>
<div id=absolute-fit-case class=case><div id=absolute-fit class="probe absolute" style="width:fit-content">${content}</div></div>
<div id=float-min-case class=case><div id=float-min class="probe float" style="width:min-content">${content}</div></div>
<div id=float-max-case class=case><div id=float-max class="probe float" style="width:max-content">${content}</div></div>
<div id=replaced-case class=case><svg id=replaced style="display:block;width:min-content" width=180 height=40 viewBox="0 0 180 40"></svg></div>
<div id=stretch-case class=case><div id=stretch class="probe stretch" style="width:stretch"></div></div>
<div id=webkit-fill-case class=case><div id=webkit-fill class="probe stretch" style="width:-webkit-fill-available"></div></div>
<div id=aspect-block-case class=case><div id=aspect-block class="probe aspect-probe" style="width:min-content">${content}</div></div>
<div id=aspect-flex-case class=case><div id=aspect-flex class="probe aspect-probe" style="display:flex;width:min-content"><span class=a></span><span class=b></span></div></div>
<div id=aspect-grid-case class=case><div id=aspect-grid class="probe aspect-probe" style="display:grid;width:min-content"><span class=a></span><span class=b></span></div></div>
<div id=flex-grow-case class=case><div id=flex-grow class="probe grow" style="width:min-content">${content}</div></div>
<div id=flex-shrink-case class=case><div id=flex-shrink class="probe shrink" style="width:max-content">${content}</div></div>
<div id=flex-basis-content-case class=case><div id=flex-basis-content class="probe basis-content" style="width:min-content">${content}</div></div>
<div id=auto-grid-min-case class=case><div id=auto-grid-min class=auto-grid><div id=auto-grid-min-item class=probe style="width:min-content">${content}</div></div></div>
<div id=auto-grid-max-case class=case><div id=auto-grid-max class=auto-grid><div id=auto-grid-max-item class=probe style="width:max-content">${content}</div></div></div>
<div id=absolute-inset-fit-case class=case><div id=absolute-inset-fit class="probe absolute" style="left:40px;right:auto;width:fit-content">${content}</div></div>
<div id=float-fit-margin-case class=case><div id=float-fit-margin class="probe float stretch" style="width:fit-content">${content}</div></div>
<div id=float-stretch-margin-case class=case><div id=float-stretch-margin class="probe float stretch" style="width:stretch">${content}</div></div>
<div id=inline-min-case class=case><span id=inline-min class=probe style="display:inline-block;width:min-content">${content}</span></div>
<div id=inline-max-case class=case><span id=inline-max class=probe style="display:inline-block;width:max-content">${content}</span></div>
<div id=inline-fit-case class=case><span id=inline-fit class=probe style="display:inline-block;width:fit-content">${content}</span></div>
<div id=min-fit-case class=case><div id=min-fit class=probe style="width:100px;min-width:fit-content">${content}</div></div>
<div id=max-fit-case class=case><div id=max-fit class=probe style="width:300px;max-width:fit-content">${content}</div></div>
<div id=min-stretch-case class=case><div id=min-stretch class="probe stretch" style="width:100px;min-width:stretch"></div></div>
<div id=max-stretch-case class=case><div id=max-stretch class="probe stretch" style="width:400px;max-width:stretch"></div></div>
<div id=min-webkit-fill-case class=case><div id=min-webkit-fill class="probe stretch" style="width:100px;min-width:-webkit-fill-available"></div></div>
<div id=max-webkit-fill-case class=case><div id=max-webkit-fill class="probe stretch" style="width:400px;max-width:-webkit-fill-available"></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let supports = page_vm.vm_mut().eval(
            r#"(()=>{const values=['min-content','max-content','fit-content','fit-content(120px)','fit-content(50%)','stretch','-webkit-fill-available'];const result=Object.fromEntries(values.map(value=>[value,CSS.supports('width',value)]));result['grid-fit-content(120px)']=CSS.supports('grid-template-columns','fit-content(120px)');for(const [key,property,value] of [['min-width:fit-content','min-width','fit-content'],['max-width:fit-content','max-width','fit-content'],['min-width:stretch','min-width','stretch'],['max-width:stretch','max-width','stretch'],['min-width:-webkit-fill-available','min-width','-webkit-fill-available'],['max-width:-webkit-fill-available','max-width','-webkit-fill-available']])result[key]=CSS.supports(property,value);return JSON.stringify(result)})()"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&supports)?,
            serde_json::json!({
                "min-content": true,
                "max-content": true,
                "fit-content": true,
                "fit-content(120px)": false,
                "fit-content(50%)": false,
                "stretch": true,
                "-webkit-fill-available": true,
                "grid-fit-content(120px)": true,
                "min-width:fit-content": true,
                "max-width:fit-content": true,
                "min-width:stretch": true,
                "max-width:stretch": true,
                "min-width:-webkit-fill-available": true,
                "max-width:-webkit-fill-available": true,
            })
        );

        let ids = [
            "min", "max", "fit", "fit-narrow", "fit-min", "min-clamp", "max-clamp",
            "conflict", "content-box", "border-box", "flex-min", "flex-max", "flex-cross",
            "grid-min", "grid-max", "absolute-min", "absolute-fit", "float-min", "float-max",
            "replaced", "stretch", "webkit-fill", "aspect-block", "aspect-flex",
            "aspect-grid", "flex-grow", "flex-shrink", "flex-basis-content",
            "auto-grid-min", "auto-grid-min-item", "auto-grid-max", "auto-grid-max-item",
            "absolute-inset-fit", "float-fit-margin", "float-stretch-margin",
            "inline-min", "inline-max", "inline-fit", "min-fit", "max-fit", "min-stretch",
            "max-stretch", "min-webkit-fill", "max-webkit-fill",
        ];
        let geometry = page_vm.vm_mut().eval(&format!(
            "JSON.stringify(Object.fromEntries({ids:?}.map(id=>{{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]}})))"
        ))?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("min", [20.0, 20.0, 160.0, 20.0]),
            ("max", [380.0, 20.0, 248.0, 20.0]),
            ("fit", [740.0, 20.0, 248.0, 20.0]),
            ("fit-narrow", [20.0, 120.0, 200.0, 20.0]),
            ("fit-min", [380.0, 120.0, 160.0, 20.0]),
            ("min-clamp", [740.0, 120.0, 248.0, 20.0]),
            ("max-clamp", [20.0, 220.0, 160.0, 20.0]),
            ("conflict", [380.0, 220.0, 248.0, 20.0]),
            ("content-box", [740.0, 220.0, 184.0, 24.0]),
            ("border-box", [20.0, 320.0, 184.0, 20.0]),
            ("flex-min", [380.0, 320.0, 160.0, 20.0]),
            ("flex-max", [740.0, 320.0, 248.0, 20.0]),
            ("flex-cross", [20.0, 420.0, 160.0, 20.0]),
            ("grid-min", [380.0, 420.0, 160.0, 20.0]),
            ("grid-max", [740.0, 420.0, 248.0, 20.0]),
            ("absolute-min", [20.0, 520.0, 160.0, 20.0]),
            ("absolute-fit", [380.0, 520.0, 200.0, 20.0]),
            ("float-min", [740.0, 520.0, 160.0, 20.0]),
            ("float-max", [20.0, 620.0, 248.0, 20.0]),
            ("replaced", [380.0, 620.0, 180.0, 40.0]),
            ("stretch", [750.0, 620.0, 275.0, 20.0]),
            ("webkit-fill", [30.0, 720.0, 275.0, 20.0]),
            ("aspect-block", [380.0, 720.0, 400.0, 20.0]),
            ("aspect-flex", [740.0, 720.0, 400.0, 20.0]),
            ("aspect-grid", [20.0, 820.0, 400.0, 20.0]),
            ("flex-grow", [380.0, 820.0, 300.0, 20.0]),
            ("flex-shrink", [740.0, 820.0, 120.0, 20.0]),
            ("flex-basis-content", [20.0, 920.0, 248.0, 20.0]),
            ("auto-grid-min", [380.0, 920.0, 160.0, 20.0]),
            ("auto-grid-min-item", [380.0, 920.0, 160.0, 20.0]),
            ("auto-grid-max", [740.0, 920.0, 248.0, 20.0]),
            ("auto-grid-max-item", [740.0, 920.0, 248.0, 20.0]),
            ("absolute-inset-fit", [60.0, 1020.0, 160.0, 20.0]),
            ("float-fit-margin", [390.0, 1020.0, 175.0, 20.0]),
            ("float-stretch-margin", [750.0, 1020.0, 175.0, 20.0]),
            ("inline-min", [20.0, 1120.0, 160.0, 20.0]),
            ("inline-max", [380.0, 1120.0, 248.0, 20.0]),
            ("inline-fit", [740.0, 1120.0, 200.0, 20.0]),
            ("min-fit", [20.0, 1220.0, 200.0, 20.0]),
            ("max-fit", [380.0, 1220.0, 200.0, 20.0]),
            ("min-stretch", [750.0, 1220.0, 275.0, 20.0]),
            ("max-stretch", [30.0, 1320.0, 275.0, 20.0]),
            ("min-webkit-fill", [390.0, 1320.0, 275.0, 20.0]),
            ("max-webkit-fill", [750.0, 1320.0, 275.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1200, 1420, 1.0))?
            .expect("intrinsic width fixture must retain a root");
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "intrinsic-sizing-keyword-deferred"
        }));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic width fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_collapsed_table_borders_with_chromium_geometry_and_pixels() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/collapsed-table-borders.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}table{position:absolute;border-collapse:collapse;table-layout:fixed;padding:0}td{box-sizing:border-box;padding:0}
#precedence{left:20px;top:20px;width:100px;border:4px solid rgb(90,0,90)}
#precedence colgroup{border-right:4px solid rgb(0,130,0)}
#precedence col{border-bottom:4px solid rgb(0,0,180)}
#precedence tbody{border-left:4px solid rgb(230,120,0)}
#precedence tr{border-top:4px solid rgb(0,160,160)}
#precedence td{width:50px;height:30px;background:rgb(240,240,240)}
#rules{left:20px;top:90px;width:120px;border:2px solid black}
#rules td{width:60px;height:30px;background:rgb(245,245,210)}
#wide-left{border-right:4px solid rgb(0,0,255)}#wide-right{border-left:8px solid rgb(255,0,0)}
#style-left{border-right:6px dashed rgb(0,150,0)}#style-right{border-left:6px solid rgb(0,0,255)}
#hidden-left{border-right:20px double rgb(120,0,120)}#hidden-right{border-left:1px hidden red}
#span{left:20px;top:220px;width:100px;border:0}
#span td{width:50px;height:30px;padding:0}
#spanning{border:6px solid rgb(0,140,0);background:rgb(210,255,210)}
#upper,#lower{border:2px solid rgb(220,0,0);background:rgb(255,220,220)}
#colspan{left:20px;top:300px;width:100px;border:0}
#colspan td{width:50px;height:30px;padding:0}
#across{border:6px solid rgb(0,140,0);background:rgb(210,255,210)}
#col-left,#col-right{border:2px solid rgb(220,0,0);background:rgb(255,220,220)}
</style>`;
document.body.innerHTML = `<table id=precedence><colgroup><col><col></colgroup><tbody><tr><td id=p0></td><td id=p1></td></tr></tbody></table>
<table id=rules><tbody><tr><td id=wide-left></td><td id=wide-right></td></tr><tr><td id=style-left></td><td id=style-right></td></tr><tr><td id=hidden-left></td><td id=hidden-right></td></tr></tbody></table>
<table id=span><tbody><tr><td id=spanning rowspan=2></td><td id=upper></td></tr><tr><td id=lower></td></tr></tbody></table>
<table id=colspan><tbody><tr><td id=across colspan=2></td></tr><tr><td id=col-left></td><td id=col-right></td></tr></tbody></table>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['precedence','p0','p1','rules','wide-left','wide-right','style-left','style-right','hidden-left','hidden-right','span','spanning','upper','lower','colspan','across','col-left','col-right'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("precedence", [20.0, 20.0, 104.0, 34.0]),
            ("p0", [22.0, 22.0, 50.0, 30.0]),
            ("p1", [72.0, 22.0, 50.0, 30.0]),
            ("rules", [20.0, 90.0, 122.0, 92.0]),
            ("wide-left", [21.0, 91.0, 60.0, 30.0]),
            ("wide-right", [81.0, 91.0, 60.0, 30.0]),
            ("style-left", [21.0, 121.0, 60.0, 30.0]),
            ("style-right", [81.0, 121.0, 60.0, 30.0]),
            ("hidden-left", [21.0, 151.0, 60.0, 30.0]),
            ("hidden-right", [81.0, 151.0, 60.0, 30.0]),
            ("span", [20.0, 220.0, 104.0, 66.0]),
            ("spanning", [23.0, 223.0, 50.0, 60.0]),
            ("upper", [73.0, 223.0, 50.0, 30.0]),
            ("lower", [73.0, 253.0, 50.0, 30.0]),
            ("colspan", [20.0, 300.0, 100.0, 64.0]),
            ("across", [23.0, 303.0, 94.0, 30.0]),
            ("col-left", [23.0, 333.0, 47.0, 30.0]),
            ("col-right", [70.0, 333.0, 47.0, 30.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}"
                );
            }
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 380, 1.0))?
            .expect("collapsed table fixture must retain a layout root");
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "collapsed-table-border-fallback"
        }));
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        for (label, point, expected) in [
            ("row beats lower sources", (70, 21), [0, 160, 160, 255]),
            ("row-group beats columns", (21, 35), [230, 120, 0, 255]),
            ("column beats table", (70, 51), [0, 0, 180, 255]),
            ("column-group beats table", (122, 35), [0, 130, 0, 255]),
            ("wider edge wins", (80, 105), [255, 0, 0, 255]),
            ("solid beats equal dashed", (80, 135), [0, 0, 255, 255]),
            (
                "hidden suppresses wider double",
                (80, 165),
                [245, 245, 210, 255],
            ),
            (
                "rowspan suppresses its internal edge",
                (45, 253),
                [210, 255, 210, 255],
            ),
            (
                "neighbor keeps its horizontal edge",
                (95, 253),
                [220, 0, 0, 255],
            ),
            (
                "colspan suppresses its internal edge",
                (70, 318),
                [210, 255, 210, 255],
            ),
            (
                "next row keeps its vertical edge",
                (70, 348),
                [220, 0, 0, 255],
            ),
        ] {
            assert_eq!(pixel(point.0, point.1), expected, "{label}");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("collapsed table border fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paint_executes_clip_filter_and_gradient_mask_layers() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-effects.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
.case{position:absolute;left:0;width:40px;height:20px;background:rgb(255,0,0)}
#clipped{top:0;clip-path:inset(0 20px 0 0)}
#filtered{top:30px;filter:brightness(0)}
#masked{top:60px}
#grouped{left:10px;top:90px;width:20px;box-shadow:8px 0 0 0 blue;outline:2px solid lime;opacity:.5}
#clip-shadow{left:10px;top:130px;width:20px;box-shadow:8px 0 0 0 blue;clip-path:inset(0)}
#closest{top:160px;width:40px;height:40px;clip-path:circle(closest-corner at 10px 10px)}
#farthest{left:50px;top:160px;width:40px;height:40px;clip-path:circle(farthest-corner at 10px 10px)}
</style>`;
document.body.innerHTML = '<div id=clipped class=case></div><div id=filtered class=case></div><div id=masked class=case></div><div id=grouped class=case></div><div id=clip-shadow class=case></div><div id=closest class=case></div><div id=farthest class=case></div>';
document.getElementById('masked').style.setProperty('mask-image','linear-gradient(to right,transparent 0%,transparent 49%,black 51%,black 100%)');
document.getElementById('masked').style.setProperty('mask-repeat','no-repeat');
'installed'
"#,
        )?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 220, 1.0))?
            .expect("effect fixture must retain a layout root");

        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::PushLayer {
                filter: Some(moli_layout::PaintFilter::Brightness(amount)),
                ..
            } if *amount == 0.0
        )));
        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::PushLayer {
                composite: moli_layout::PaintCompositeMode::DestIn,
                ..
            }
        )));
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "mask-image-resource-deferred"
                && diagnostic.code != "filter-url-reference-unsupported"
                && diagnostic.code != "clip-path-url-reference-unsupported"
                && diagnostic.code != "clip-path-corner-radius-unsupported"
        }));

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            &image.rgba[index..index + 4]
        };
        assert_eq!(pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(30, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(10, 40), [0, 0, 0, 255]);
        assert_eq!(pixel(10, 70), [255, 255, 255, 255]);
        assert_eq!(pixel(30, 70), [255, 0, 0, 255]);
        let assert_channel_near = |actual: u8, expected: u8| {
            assert!(
                actual.abs_diff(expected) <= 2,
                "expected channel near {expected}, got {actual}"
            );
        };
        for (actual, expected) in pixel(15, 100).iter().zip([255, 128, 128, 255]) {
            assert_channel_near(*actual, expected);
        }
        for (actual, expected) in pixel(9, 100).iter().zip([128, 255, 128, 255]) {
            assert_channel_near(*actual, expected);
        }
        for (actual, expected) in pixel(36, 100).iter().zip([128, 128, 255, 255]) {
            assert_channel_near(*actual, expected);
        }
        assert_eq!(pixel(15, 140), [255, 0, 0, 255]);
        assert_eq!(pixel(36, 140), [255, 255, 255, 255]);
        assert_eq!(pixel(10, 170), [255, 0, 0, 255]);
        assert_eq!(pixel(30, 190), [255, 255, 255, 255]);
        assert_eq!(pixel(89, 199), [255, 0, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("clip/filter/mask screenshot fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_css_gradient_domains_hints_and_interpolation_like_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/css-gradient-domain.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}.case{position:absolute;width:100px;height:100px}
#negative{left:0;top:0;background:linear-gradient(to bottom,rgba(0,0,0,.5) -20%,transparent 30%),white}
#overflow{left:110px;top:0;background:linear-gradient(to bottom,rgb(255,0,0) 80%,rgb(0,0,255) 120%)}
#repeat{left:220px;top:0;background:repeating-linear-gradient(to bottom,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#radial-overflow{left:330px;top:0;background:radial-gradient(circle 50px at 50px 50px,rgb(255,0,0) 80%,rgb(0,0,255) 120%)}
#radial{left:0;top:110px;background:radial-gradient(circle 50px at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#conic{left:110px;top:110px;background:conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#hint{left:220px;top:110px;background:linear-gradient(to right,rgb(255,0,0) 0%,25%,rgb(0,0,255) 100%)}
#conic-overflow{left:330px;top:110px;background:conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) 80%,rgb(0,0,255) 120%)}
#degenerate{left:0;top:220px;background:linear-gradient(to right,rgb(255,0,0) -20%,rgb(0,0,255) -20%)}
#repeat-degenerate{left:110px;top:220px;background:repeating-linear-gradient(to right,rgb(255,0,0) 20%,rgb(0,0,255) 20%)}
#oklab{left:220px;top:220px;background:linear-gradient(to right in oklab,rgb(255,0,0),rgb(0,0,255))}
#repeat-radial{left:330px;top:220px;background:repeating-radial-gradient(circle 50px at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#repeat-conic{left:0;top:330px;background:repeating-conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#p3-linear{left:110px;top:330px;background:linear-gradient(to right in display-p3-linear,rgb(255,0,0),rgb(0,0,255))}
#normal-conic{left:220px;top:330px;background:conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) 0%,rgb(0,0,255) 100%)}
</style>`;
document.body.innerHTML = '<div id=negative class=case></div><div id=overflow class=case></div><div id=repeat class=case></div><div id=radial-overflow class=case></div><div id=radial class=case></div><div id=conic class=case></div><div id=hint class=case></div><div id=conic-overflow class=case></div><div id=degenerate class=case></div><div id=repeat-degenerate class=case></div><div id=oklab class=case></div><div id=repeat-radial class=case></div><div id=repeat-conic class=case></div><div id=p3-linear class=case></div><div id=normal-conic class=case></div>';
'installed'
"#,
        )?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(430, 430, 1.0))?
            .expect("gradient fixture must retain a layout root");

        let brushes = snapshot.fragments.iter().filter_map(|fragment| match fragment {
            moli_layout::PaintFragment::Fill { brush, .. } => Some(brush),
            _ => None,
        });
        let gradients = brushes
            .filter(|brush| !matches!(brush, moli_layout::PaintBrush::Solid(_)))
            .collect::<Vec<_>>();
        assert_eq!(gradients.len(), 15);
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::LinearGradient(gradient)
                if (gradient.start.y + 20.0).abs() <= 0.01
                    && (gradient.end.y - 30.0).abs() <= 0.01
                    && gradient.extend == moli_layout::PaintGradientExtend::Pad
                    && gradient.stops.first().is_some_and(|stop| stop.offset == 0.0)
                    && gradient.stops.last().is_some_and(|stop| stop.offset == 1.0)
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::RadialGradient(gradient)
                if gradient.start_radius == 0.0
                    && (gradient.end_radius - 0.3).abs() <= 0.01
                    && (gradient.transform.coefficients[4] - 50.0).abs() <= 0.01
                    && (gradient.transform.coefficients[5] - 50.0).abs() <= 0.01
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::ConicGradient(gradient)
                if gradient.center == moli_layout::PaintPoint::ZERO
                    && gradient.start_angle_radians == 0.0
                    && (gradient.end_angle_radians - std::f32::consts::TAU).abs() <= 0.01
                    && (gradient.transform.coefficients[4] - 50.0).abs() <= 0.01
                    && (gradient.transform.coefficients[5] - 50.0).abs() <= 0.01
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::LinearGradient(gradient)
                if gradient.stops.len() == 11
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::LinearGradient(gradient)
                if gradient.interpolation.color_space
                    == moli_layout::PaintGradientColorSpace::Oklab
        )));

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        let assert_pixel_near = |label: &str, point: (u32, u32), expected: [u8; 4]| {
            let actual = pixel(point.0, point.1);
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 2),
                "{label}: expected {expected:?}, got {actual:?}"
            );
        };
        for (label, point, expected) in [
            ("negative start", (50, 0), [179, 179, 179, 255]),
            ("negative middle", (50, 10), [205, 205, 205, 255]),
            ("negative end", (50, 29), [254, 254, 254, 255]),
            ("negative padded", (50, 50), [255, 255, 255, 255]),
            ("overflow padded", (160, 50), [255, 0, 0, 255]),
            ("overflow start", (160, 80), [251, 0, 3, 255]),
            ("overflow middle", (160, 99), [131, 0, 124, 255]),
            ("repeat first", (270, 0), [150, 0, 104, 255]),
            ("repeat second", (270, 30), [252, 0, 2, 255]),
            ("radial center", (50, 160), [145, 0, 109, 255]),
            ("radial transition", (60, 160), [45, 0, 209, 255]),
            ("radial pad", (70, 160), [0, 0, 255, 255]),
            ("conic top", (160, 120), [151, 0, 103, 255]),
            ("conic right", (200, 160), [24, 0, 230, 255]),
            ("conic bottom", (160, 200), [0, 0, 255, 255]),
            ("hint quarter", (245, 160), [127, 0, 129, 255]),
            ("hint middle", (270, 160), [74, 0, 181, 255]),
            ("hint third quarter", (295, 160), [36, 0, 220, 255]),
            ("degenerate nonrepeat", (50, 270), [0, 0, 255, 255]),
            ("degenerate repeat", (160, 270), [0, 0, 255, 255]),
            ("oklab quarter", (245, 270), [197, 74, 111, 255]),
            ("oklab middle", (270, 270), [139, 83, 163, 255]),
            ("oklab third quarter", (295, 270), [80, 71, 211, 255]),
            ("radial overflow pad", (419, 50), [255, 0, 0, 255]),
            ("radial overflow start", (420, 50), [248, 0, 6, 255]),
            ("radial overflow middle", (429, 50), [134, 0, 121, 255]),
            ("conic overflow pad", (380, 120), [255, 0, 0, 255]),
            ("conic overflow middle", (350, 130), [207, 0, 48, 255]),
            ("repeat radial center", (380, 270), [145, 0, 109, 255]),
            ("repeat radial next period", (400, 270), [198, 0, 56, 255]),
            ("repeat conic top", (50, 340), [152, 0, 103, 255]),
            ("repeat conic right", (90, 380), [24, 0, 230, 255]),
            ("repeat conic bottom", (50, 420), [154, 0, 101, 255]),
            ("display p3 linear quarter", (135, 380), [224, 0, 138, 255]),
            ("display p3 linear middle", (160, 380), [186, 0, 188, 255]),
            ("display p3 linear third quarter", (185, 380), [136, 0, 226, 255]),
            ("normal conic top", (270, 340), [254, 0, 0, 255]),
            ("normal conic right", (310, 380), [190, 0, 64, 255]),
            ("normal conic bottom", (270, 420), [128, 0, 127, 255]),
            ("normal conic left", (230, 380), [64, 0, 190, 255]),
        ] {
            assert_pixel_near(label, point, expected);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSS gradient domain fixture should match Chromium");
}

#[tokio::test]
async fn screenshot_paint_consumes_ready_webp_and_svg_css_url_layers() {
    run_page_vm_async_test(async move {
        // A lossless 2x2 red WebP. Keep the HTTP fixture encoded so this
        // product path exercises metadata probing and bounded WebP decode.
        let raster_url = "data:image/webp;base64,UklGRhwAAABXRUJQVlA4TA8AAAAvAUAAAAcQ/Y/+ByKi/wEA"
            .to_owned();
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/css-url-image-layers.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        let local_executor = page_vm.local_executor.clone();

        local_executor.run(async move {
            let vector_url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="blue"/></svg>"#,
            )
        );
            let mask_url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect x="20" width="20" height="20" fill="white"/></svg>"#,
            )
        );
            let css = format!(
                r#"
html,body{{margin:0;padding:0;background:white}}
.case{{position:absolute;left:0;width:40px;height:20px}}
#raster{{top:0;background-image:url("{raster_url}");background-size:10px 10px;background-repeat:repeat-x}}
#vector{{top:30px;background-image:url("{vector_url}");background-size:20px auto;background-repeat:repeat-x}}
#masked{{top:60px;background:red;mask-image:url("{mask_url}");mask-size:40px 20px;mask-repeat:no-repeat}}
#inline-line{{position:absolute;left:0;top:90px;font:20px/20px sans-serif}}
#inline-vector{{padding:0 10px;color:transparent;background-image:url("{vector_url}");background-size:20px 20px;background-repeat:repeat-x}}
"#
            );
            page_vm.vm_mut().eval(&format!(
                "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML='<div id=raster class=case></div><div id=vector class=case></div><div id=masked class=case></div><div id=inline-line><span id=inline-vector>X</span></div>';'installed'",
                serde_json::to_string(&css)?,
            ))?;
            page_vm.vm_mut().sync_live_document_style_sources();

            // The first paint demand discovers computed CSSOM URLs and queues
            // the bounded decoders. CSS images have no DOM load-event task; a
            // later screencast/screenshot samples immutable ready resources.
            page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(
                    80, 120, 1.0,
                ))?
                .expect("CSS image fixture must retain a layout root");
            let urls = [&raster_url, &vector_url, &mask_url];
            let completion_notify = page_vm.vm().css_image_completion_notify_for_test();
            let all_ready = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    // Register before checking state so a completion between
                    // the check and await cannot be lost.
                    let notified = completion_notify.notified();
                    if urls.iter().all(|url| {
                        page_vm
                            .vm()
                            .css_image_resource_is_ready_for_test(url)
                    }) {
                        break;
                    }
                    notified.await;
                }
            })
            .await
            .is_ok();
            assert!(
                all_ready,
                "bounded local CSS image decodes must complete: {:?}",
                page_vm.vm().css_image_resource_observability_for_test()
            );

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(
                    80, 120, 1.0,
                ))?
                .expect("ready CSS image fixture must retain a layout root");
            assert_eq!(snapshot.images.len(), 1);
            assert_eq!(snapshot.svg_images.len(), 3);
            assert!(snapshot.fragments.iter().any(|fragment| {
                matches!(fragment, moli_layout::PaintFragment::Image(_))
            }));
            assert!(snapshot.fragments.iter().any(|fragment| {
                matches!(fragment, moli_layout::PaintFragment::SvgImage(_))
            }));
            assert!(snapshot.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "background-image-resource-unavailable"
                    && diagnostic.code != "mask-image-resource-unavailable"
                    && diagnostic.code != "background-image-type-unsupported"
                    && diagnostic.code != "mask-image-type-unsupported"
            }));

            let image = moli_paint::raster_snapshot(&snapshot)?;
            let pixel = |x: u32, y: u32| {
                let index = ((y * image.width + x) * 4) as usize;
                &image.rgba[index..index + 4]
            };
            assert_eq!(pixel(5, 5), [255, 0, 0, 255]);
            assert_eq!(pixel(35, 5), [255, 0, 0, 255]);
            assert_eq!(pixel(5, 15), [255, 255, 255, 255]);
            assert_eq!(pixel(5, 35), [0, 0, 255, 255]);
            assert_eq!(pixel(25, 35), [0, 0, 255, 255]);
            assert_eq!(pixel(5, 65), [255, 255, 255, 255]);
            assert_eq!(pixel(30, 65), [255, 0, 0, 255]);
            assert_eq!(pixel(5, 95), [0, 0, 255, 255]);
            Ok::<_, anyhow::Error>(())
        })
        .await
    })
    .await
    .expect("WebP and SVG CSS URL image-layer fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paints_fresh_inline_svg_resources_with_computed_current_color() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-replaced.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
#icon{position:absolute;left:0;top:0;color:red}
#icon.blue{color:blue}
#ratio{position:absolute;left:0;top:30px;color:green}
#feishu-time{position:absolute;left:60px;top:0;color:#646a73;font-size:16px}
#feishu-time.css-width{width:24px}
</style>`;
document.body.innerHTML = `
<svg id="icon" width="40" height="20" viewBox="0 0 4 2">
  <rect id="shape" width="2" height="2" fill="currentColor"></rect>
</svg>
<svg id="feishu-time" width="1em" height="1em" viewBox="0 0 24 24" data-icon="TimeOutlined">
  <rect width="24" height="24" fill="currentColor"></rect>
</svg>
<svg id="ratio" viewBox="0 0 1 1">
  <rect width="1" height="1" fill="currentColor"></rect>
</svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        assert_eq!(
            page_vm.vm_mut().eval(
                "[getComputedStyle(document.getElementById('feishu-time')).width,getComputedStyle(document.getElementById('feishu-time')).height].join('|')",
            )?,
            "16px|16px",
            "SVG presentation attributes must resolve 1em through the element's computed font size like Chromium",
        );

        let first = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 190, 1.0))?
            .expect("inline SVG fixture must retain a layout root");
        assert_eq!(first.svg_images.len(), 3);
        assert_eq!(
            first
                .fragments
                .iter()
                .filter(|fragment| matches!(fragment, moli_layout::PaintFragment::SvgImage(_)))
                .count(),
            3
        );
        first
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::SvgImage(image) => Some(image.destination),
                _ => None,
            })
            .find(|destination| {
                (destination.width - 16.0).abs() <= 0.01
                    && (destination.height - 16.0).abs() <= 0.01
            })
            .expect("the Feishu-style 1em SVG must paint into a 16x16 destination");
        assert!(first.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "replaced-content-placeholder"
                && diagnostic.code != "svg-resource-unsupported"
        }));

        let first_raster = moli_paint::raster_snapshot(&first)?;
        let first_pixel = |x: u32, y: u32| {
            let index = ((y * first_raster.width + x) * 4) as usize;
            &first_raster.rgba[index..index + 4]
        };
        // The authored subtree has no serialized stylesheet. Red and green
        // therefore prove that the root's resolved Stylo `color` reached
        // usvg's inherited `currentColor` rather than falling back to black.
        assert_eq!(first_pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(first_pixel(30, 5), [255, 255, 255, 255]);
        assert_eq!(first_pixel(65, 5), [100, 106, 115, 255]);
        assert_eq!(first_pixel(80, 5), [255, 255, 255, 255]);
        assert_eq!(first_pixel(140, 40), [0, 128, 0, 255]);
        // A viewBox-only square uses its 1:1 ratio inside the CSS 300x150
        // default object size, yielding a 150x150 replaced box rather than
        // losing the ratio and stretching to 300x150.
        assert_eq!(first_pixel(170, 40), [255, 255, 255, 255]);

        page_vm.vm_mut().eval(
            "document.getElementById('icon').classList.add('blue');document.getElementById('shape').setAttribute('x','2');document.getElementById('feishu-time').setAttribute('width','2em');'mutated'",
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "const icon=document.getElementById('feishu-time');const fromAttribute=getComputedStyle(icon).width;icon.classList.add('css-width');const fromCss=getComputedStyle(icon).width;icon.classList.remove('css-width');[fromAttribute,fromCss,getComputedStyle(icon).width].join('|')",
            )?,
            "32px|24px|32px",
            "mutated presentation attributes must recascade and author CSS must override them",
        );
        let second = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 190, 1.0))?
            .expect("mutated inline SVG fixture must retain a layout root");
        assert_eq!(second.svg_images.len(), 3);
        assert!(second.fragments.iter().any(|fragment| {
            matches!(
                fragment,
                moli_layout::PaintFragment::SvgImage(image)
                    if (image.destination.width - 32.0).abs() <= 0.01
                        && (image.destination.height - 16.0).abs() <= 0.01
            )
        }));
        assert!(first.svg_images.iter().all(|old| {
            second
                .svg_images
                .iter()
                .all(|fresh| !std::sync::Arc::ptr_eq(&old.image, &fresh.image))
        }));

        let second_raster = moli_paint::raster_snapshot(&second)?;
        let second_pixel = |x: u32, y: u32| {
            let index = ((y * second_raster.width + x) * 4) as usize;
            &second_raster.rgba[index..index + 4]
        };
        assert_eq!(second_pixel(5, 5), [255, 255, 255, 255]);
        assert_eq!(second_pixel(30, 5), [0, 0, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("fresh inline SVG replaced-resource fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paints_background_clip_text_with_transparent_webkit_fill() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/background-clip-text.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
#gradient{display:inline-block;padding:12px;font:32px/40px sans-serif;color:black;background-image:linear-gradient(90deg,rgb(255,0,0),rgb(0,0,255));background-repeat:no-repeat;background-clip:text;-webkit-text-fill-color:transparent}
#inline-line{font:32px/40px sans-serif;color:black}
#inline-gradient{background-image:linear-gradient(90deg,rgb(255,0,0),rgb(0,0,255));background-repeat:no-repeat;background-clip:text;-webkit-text-fill-color:transparent}
#inline-sibling{-webkit-text-fill-color:transparent}
</style>`;
document.body.innerHTML = '<div id=gradient><span id=child>MMMM</span></div><div id=inline-line><span id=inline-gradient>MMMM</span><span id=inline-sibling>WWWW</span></div>';
'installed'
"#,
        )?;

        let computed = page_vm.vm_mut().eval(
            r#"[
getComputedStyle(document.getElementById('gradient')).getPropertyValue('-webkit-text-fill-color'),
getComputedStyle(document.getElementById('child')).getPropertyValue('-webkit-text-fill-color'),
getComputedStyle(document.getElementById('inline-gradient')).getPropertyValue('-webkit-text-fill-color')
].join('|')"#,
        )?;
        assert_eq!(
            computed,
            "rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)",
            "the Stylo longhand must cascade and inherit before paint"
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 120, 1.0))?
            .expect("background-clip:text fixture must retain a layout root");
        assert!(
            snapshot
                .fragments
                .iter()
                .filter(|fragment| matches!(
                    fragment,
                    moli_layout::PaintFragment::PushLayer {
                        composite: moli_layout::PaintCompositeMode::DestIn,
                        ..
                    }
                ))
                .count()
                >= 2,
            "both atomic and flattened inline backgrounds must receive text masks"
        );
        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::GlyphRun(run) if run.color.alpha == 0.0
        )));
        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::GlyphRun(run)
                if run.color == moli_layout::PaintColor::BLACK
        )));
        let mut mask_depth = None::<usize>;
        let mut mask_glyph_count = 0usize;
        let mut mask_glyph_counts = Vec::new();
        for fragment in &snapshot.fragments {
            match fragment {
                moli_layout::PaintFragment::PushLayer {
                    composite: moli_layout::PaintCompositeMode::DestIn,
                    ..
                } if mask_depth.is_none() => {
                    mask_depth = Some(1);
                    mask_glyph_count = 0;
                }
                moli_layout::PaintFragment::PushLayer { .. }
                | moli_layout::PaintFragment::PushClip { .. } => {
                    if let Some(depth) = mask_depth.as_mut() {
                        *depth += 1;
                    }
                }
                moli_layout::PaintFragment::PopLayer => {
                    let closes_mask = mask_depth.as_mut().is_some_and(|depth| {
                        *depth -= 1;
                        *depth == 0
                    });
                    if closes_mask {
                        mask_glyph_counts.push(mask_glyph_count);
                        mask_depth = None;
                    }
                }
                moli_layout::PaintFragment::GlyphRun(run) if mask_depth.is_some() => {
                    mask_glyph_count += run.glyphs.len();
                }
                _ => {}
            }
        }
        assert_eq!(
            mask_glyph_counts,
            [4, 4],
            "the flattened inline mask must exclude the adjacent transparent sibling's glyphs"
        );
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "background-clip-text-fallback"
        }));

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(
            pixel(4, 4),
            [255, 255, 255, 255],
            "padding must stay transparent instead of exposing the gradient rectangle"
        );
        assert_eq!(
            pixel(13, 13),
            [255, 255, 255, 255],
            "blank line-box space must stay transparent instead of using the old content-box fallback"
        );
        assert_eq!(
            pixel(20, 50),
            [255, 255, 255, 255],
            "line-height leading must not expose the gradient outside glyph ink"
        );
        let colored_ink = image
            .rgba
            .chunks_exact(4)
            .filter(|pixel| {
                let [red, green, blue, alpha] = <[u8; 4]>::try_from(*pixel).unwrap();
                alpha == 255
                    && (red.abs_diff(blue) > 20
                        || red.max(blue).saturating_sub(green) > 20)
            })
            .count();
        assert!(
            colored_ink > 50,
            "the text mask must retain visible gradient glyph ink; colored_ink={colored_ink}"
        );
        let colored_inline_ink = image
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(index, pixel)| {
                let [red, green, blue, alpha] = <[u8; 4]>::try_from(*pixel).unwrap();
                let y = *index as u32 / image.width;
                y >= 64
                    && alpha == 255
                    && (red.abs_diff(blue) > 20
                        || red.max(blue).saturating_sub(green) > 20)
            })
            .count();
        assert!(
            colored_inline_ink > 50,
            "a flattened inline box must paint gradient glyph ink through its owner IFC mask; colored_inline_ink={colored_inline_ink}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("background-clip:text screenshot fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn baseline_atomic_inline_keeps_parent_strut_descent_in_flex_header() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/baseline-atomic-strut.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#bar{display:flex;align-items:center;box-sizing:border-box;width:200px;height:60px;padding:6px}
#header{display:flex;align-items:center}
/* A zero-size font makes the 3px half-leading below the baseline deterministic. */
#wrapper{display:block;font-size:0;line-height:6px}
#atomic{display:inline-block;width:48px;height:48px;background:blue}
</style>`;
document.body.innerHTML = `<div id=bar><header id=header><div id=wrapper><span id=atomic></span></div></header></div>`;
'installed'
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['bar','header','wrapper','atomic'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("bar", [0.0, 0.0, 200.0, 60.0]),
            ("header", [6.0, 4.5, 48.0, 51.0]),
            ("wrapper", [6.0, 4.5, 48.0, 51.0]),
            ("atomic", [6.0, 4.5, 48.0, 48.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("baseline atomic strut fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn atomic_inline_location_uses_the_global_layout_unit_grid() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-layout-unit.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#line{margin:0;white-space:nowrap;font:16px/75px sans-serif}
#atomic{display:inline-block;position:relative;left:.1px}
</style>`;
document.body.innerHTML = `<p id=line>All the <span id=atomic>words</span> after</p>`;
'installed'
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const line=document.getElementById('line');const atomic=document.getElementById('atomic');const rect=node=>{const range=document.createRange();range.selectNodeContents(node);const value=range.getBoundingClientRect();return [value.x,value.right]};const box=atomic.getBoundingClientRect();return {preceding:rect(line.firstChild),atomic:[box.x,box.right],text:rect(atomic)}})())"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        let number = |group: &str, index: usize| {
            geometry[group][index]
                .as_f64()
                .unwrap_or_else(|| panic!("missing {group}[{index}] in {geometry}"))
                as f32
        };
        let preceding_right = number("preceding", 1);
        let atomic_x = number("atomic", 0);
        let text_x = number("text", 0);
        let geometry_epsilon = 0.0001;
        let unrounded_atomic_x = preceding_right + 0.1;
        let expected_atomic_x = (unrounded_atomic_x * 64.0).round() / 64.0;

        assert!(
            (unrounded_atomic_x * 64.0 - (unrounded_atomic_x * 64.0).round()).abs() > 0.05,
            "fixture must place the unrounded atomic origin away from the 1/64 grid: {geometry}"
        );
        assert!(
            (atomic_x - expected_atomic_x).abs() <= geometry_epsilon,
            "the atomic outer placement must use the global 1/64 layout grid; expected {expected_atomic_x}: {geometry}"
        );
        assert!(
            (text_x - atomic_x).abs() <= geometry_epsilon,
            "the atomic IFC must paint its first glyph at the rounded box origin: {geometry}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic inline layout-unit fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rounded_flex_max_content_width_does_not_rewrap_its_text() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-intrinsic-rounding.html")?,
        );
        let encoded_font = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(encoded_font);
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{ font-family:MoliAhem; src:url(data:font/woff2;base64,{encoded}) format('woff2') }}
* {{ box-sizing:border-box }}
html,body {{ margin:0 }}
.row {{ display:flex; justify-content:center; gap:8px; padding-top:20px }}
.item {{ display:inline-flex; align-items:center; gap:8px; padding:10px 14px; border:1px solid; border-radius:24px }}
.icon {{ width:20px; height:20px; flex:0 0 auto }}
.text,#constrained {{ font-family:MoliAhem; font-size:9px }}
#constrained {{ width:80px }}
</style>`;
document.body.innerHTML = `<div class=row><div class=item id=ask><i class=icon></i><span class=text id=ask-text>Ask about files</span></div></div><div id=constrained>Ask about files</div>`;
'installed'
"#
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['ask','ask-text','constrained'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        // Chromium's Ahem `normal` line-height follows the font metrics: 9px
        // for one line and 18px after wrapping. The icon and padding, rather
        // than an inflated synthetic line-height, establish the 42px pill.
        for (id, expected) in [
            ("ask", [139.015625, 42.0]),
            ("ask-text", [81.015625, 9.0]),
            ("constrained", [80.0, 18.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry");
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rounded flex max-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stylesheet_lifecycle_registers_only_the_current_documents_data_web_fonts() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-web-font.html")?,
        );
        let encoded_font = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(encoded_font);
        let install_font = |family: &str| {
            format!(
                r#"
(() => {{
const style = document.createElement('style');
style.id = 'layout-web-font';
style.textContent = `
  @font-face {{
    font-family: {family};
    src: url(data:font/woff2;base64,{encoded}) format('woff2');
  }}
  body {{ font: 20px {family}; }}
`;
document.head.append(style);
document.body.textContent = 'AAAA';
return 'installed';
}})()
"#
            )
        };

        page_vm.vm_mut().eval(&install_font("FirstDocumentFace"))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1),
            "the stylesheet lifecycle should discover, decode, and register the current @font-face before layout"
        );
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("current Document should have a layout root");
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1),
            "layout should reuse the font registered by the stylesheet lifecycle"
        );
        let expected_ttf = wuff::decompress_woff2(encoded_font).expect("valid WOFF2 fixture");
        assert!(
            snapshot
                .fonts
                .iter()
                .any(|resource| resource.font.data.as_ref() == expected_ttf),
            "the snapshot must shape text with the decoded deterministic web font"
        );
        let first_document_cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(
            first_document_cache.3.is_some(),
            "a successful screenshot layout must publish geometry"
        );

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write('<!doctype html><html><head></head><body></body></html>');
document.close();
'replaced'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (0, 0, 0),
            "document.open() must replace the document-owned font sidecar"
        );
        let replacement_cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(
            replacement_cache.3.is_none(),
            "the main Document owner transition must clear its old layout snapshot"
        );
        assert_eq!(replacement_cache.2, first_document_cache.2);

        page_vm
            .vm_mut()
            .eval(&install_font("ReplacementDocumentFace"))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1),
            "the replacement document should register only its own @font-face before layout"
        );
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("replacement Document should have a layout root");
        let replacement_cache_after_layout =
            page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(replacement_cache_after_layout.3.is_some());
        assert_eq!(replacement_cache_after_layout.2, first_document_cache.2 + 1);
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1)
        );
        page_vm
            .vm_mut()
            .eval("document.querySelector('#layout-web-font').remove(); 'removed'")?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("replacement Document should remain layoutable");
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (0, 0, 0),
            "the next one-shot demand must revoke a removed @font-face without a generation fence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document-owned data web-font layout test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn same_family_unicode_range_faces_shape_mixed_text_with_both_subsets() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/segmented-web-font.html")?,
        );
        let latin = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        ));
        let cjk = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-cjk.ttf"
        ));
        let latin = base64::engine::general_purpose::STANDARD.encode(latin);
        let cjk = base64::engine::general_purpose::STANDARD.encode(cjk);
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{
  font-family: MoliSegmented;
  src: url(data:font/ttf;base64,{latin}) format('truetype');
  font-weight: 400;
  unicode-range: U+0000-00FF;
}}
@font-face {{
  font-family: MoliSegmented;
  src: url(data:font/ttf;base64,{cjk}) format('truetype');
  font-weight: 400;
  unicode-range: U+4E00-9FFF;
}}
body {{ margin: 0; font: 32px/40px MoliSegmented, sans-serif; }}
</style>`;
document.body.textContent = 'R中';
'installed'
"#
        ))?;

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 80, 1.0))?
            .expect("segmented web-font fixture should have a layout root");
        let used_fonts = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) => snapshot.font(run.font),
                _ => None,
            })
            .map(|font| font.font.data.as_ref())
            .collect::<Vec<_>>();
        assert!(
            used_fonts.iter().any(|font| *font
                == include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-ahem.ttf"
                ))),
            "the Latin character must use the Latin subset"
        );
        assert!(
            used_fonts.iter().any(|font| *font
                == include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-cjk.ttf"
                ))),
            "the CJK character must use the CJK subset"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("segmented web-font layout test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cjk_regular_face_uses_chromium_synthetic_bold_threshold() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/cjk-synthetic-bold.html")?,
        );
        let cjk = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-cjk.ttf"
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(cjk);
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{ font-family: MoliCJKRegular; src: url(data:font/ttf;base64,{encoded}) format('truetype'); font-weight: 400; }}
html, body {{ margin: 0; padding: 0; background: white; }}
.case {{ font-family: MoliCJKRegular; font-size: 32px; line-height: 40px; height: 40px; color: black; }}
#normal {{ font-weight: 400; }}
#medium {{ font-weight: 500; }}
#semibold {{ font-weight: 600; }}
#disabled {{ font-weight: 600; font-synthesis-weight: none; }}
</style>`;
document.body.innerHTML = '<div id=normal class=case>中</div><div id=medium class=case>中</div><div id=semibold class=case>中</div><div id=disabled class=case>中</div>';
'installed'
"#
        ))?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 160, 1.0))?
            .expect("CJK fixture should have a layout root");
        let runs = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) => Some(run),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 4, "the fixture should shape one run per line");
        assert_eq!(runs[0].glyph_embolden, moli_layout::PaintPoint::ZERO);
        assert_eq!(
            runs[1].glyph_embolden,
            moli_layout::PaintPoint::ZERO,
            "CSS weight 500 must remain below Chromium's synthetic-bold threshold"
        );
        assert!(
            runs[2].glyph_embolden.x > 0.0 && runs[2].glyph_embolden.y > 0.0,
            "the unavailable CJK 600 face should preserve Parley's faux-bold request"
        );
        assert_eq!(
            runs[3].glyph_embolden,
            moli_layout::PaintPoint::ZERO,
            "font-synthesis-weight:none must suppress faux bold"
        );
        for run in &runs {
            assert_eq!(snapshot.font(run.font).expect("run font").font.data.as_ref(), cjk);
        }

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let ink = |top: u32| {
            (top..top + 40)
                .flat_map(|y| (0..40).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let offset = ((y * image.width + x) * 4) as usize;
                    let pixel = &image.rgba[offset..offset + 4];
                    u64::from(255 - pixel[0])
                        + u64::from(255 - pixel[1])
                        + u64::from(255 - pixel[2])
                })
                .sum::<u64>()
        };
        let normal_ink = ink(0);
        let medium_ink = ink(40);
        let semibold_ink = ink(80);
        let disabled_ink = ink(120);
        assert_eq!(
            medium_ink, normal_ink,
            "CSS weight 500 must raster exactly the regular CJK face"
        );
        assert!(
            semibold_ink > normal_ink,
            "Vello CPU must raster faux bold with more ink: normal={normal_ink}, semibold={semibold_ink}"
        );
        assert_eq!(
            disabled_ink, normal_ink,
            "disabling synthesis must raster exactly the regular CJK face"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CJK synthetic-bold rendering test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn form_control_ua_styles_match_the_chromium_headless_contract() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/form-control-ua.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `
  <input id=text-input>
  <input id=checkbox type=checkbox>
  <input id=radio type=radio>
  <input id=range type=range>
  <input id=color type=color>
  <input id=disabled-text disabled>
  <textarea id=textarea></textarea>
  <select id=select><option>A</option></select>
  <select id=disabled-select disabled><option>A</option></select>
  <output id=output></output>
  <meter id=meter></meter>
  <progress id=progress></progress>
`;
'installed'
"#,
        )?;

        let computed = page_vm.vm_mut().eval(
            r#"
(() => {
  const read = id => {
    const style = getComputedStyle(document.getElementById(id));
    return {
      appearance: style.appearance,
      backgroundColor: style.backgroundColor,
      borderTopColor: style.borderTopColor,
      borderTopStyle: style.borderTopStyle,
      borderTopWidth: style.borderTopWidth,
      boxSizing: style.boxSizing,
      color: style.color,
      cursor: style.cursor,
      display: style.display,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      height: style.height,
      margin: style.margin,
      opacity: style.opacity,
      overflow: style.overflow,
      overflowClipMargin: style.getPropertyValue('overflow-clip-margin'),
      overflowWrap: style.getPropertyValue('overflow-wrap'),
      padding: style.padding,
      textAlign: style.textAlign,
      whiteSpace: style.whiteSpace,
      width: style.width
    };
  };
  return JSON.stringify({
    textInput: read('text-input'),
    checkbox: read('checkbox'),
    radio: read('radio'),
    range: read('range'),
    color: read('color'),
    disabledText: read('disabled-text'),
    textarea: read('textarea'),
    select: read('select'),
    disabledSelect: read('disabled-select'),
    output: read('output'),
    meter: read('meter'),
    progress: read('progress')
  });
})()
"#,
        )?;
        let computed: serde_json::Value = serde_json::from_str(&computed)?;

        for control in [
            "textInput",
            "checkbox",
            "radio",
            "range",
            "color",
            "disabledText",
            "textarea",
            "select",
            "disabledSelect",
        ] {
            assert_eq!(computed[control]["display"], "inline-block", "{control}");
            assert_eq!(computed[control]["appearance"], "auto", "{control}");
        }
        assert_eq!(computed["textInput"]["fontFamily"], "Arial, sans-serif");
        let control_font_size = computed["textInput"]["fontSize"]
            .as_str()
            .expect("computed control font-size")
            .trim_end_matches("px")
            .parse::<f32>()?;
        assert!((control_font_size - 13.3333).abs() <= 0.01);
        assert_eq!(computed["textInput"]["boxSizing"], "border-box");
        assert_eq!(computed["textInput"]["padding"], "1px 2px");
        assert_eq!(computed["textInput"]["borderTopWidth"], "2px");
        assert_eq!(computed["textInput"]["borderTopStyle"], "inset");
        assert_eq!(
            computed["textInput"]["borderTopColor"],
            "rgb(118, 118, 118)"
        );
        assert_eq!(
            computed["textInput"]["backgroundColor"],
            "rgb(255, 255, 255)"
        );
        assert_eq!(computed["textInput"]["color"], "rgb(0, 0, 0)");
        assert_eq!(computed["textInput"]["cursor"], "text");
        assert_eq!(computed["textInput"]["overflow"], "clip");
        assert_eq!(computed["textInput"]["overflowClipMargin"], "0px");
        assert_eq!(computed["textInput"]["textAlign"], "start");

        assert_eq!(computed["checkbox"]["boxSizing"], "border-box");
        assert_eq!(computed["checkbox"]["margin"], "3px 3px 3px 4px");
        assert_eq!(computed["checkbox"]["padding"], "0px");
        assert_eq!(computed["checkbox"]["borderTopWidth"], "0px");
        assert_eq!(computed["checkbox"]["backgroundColor"], "rgba(0, 0, 0, 0)");
        assert_eq!(computed["checkbox"]["cursor"], "default");
        assert_eq!(computed["radio"]["boxSizing"], "border-box");
        assert_eq!(computed["radio"]["margin"], "3px 3px 0px 5px");

        assert_eq!(computed["range"]["margin"], "2px");
        assert_eq!(computed["range"]["padding"], "0px");
        assert_eq!(computed["range"]["borderTopWidth"], "0px");
        assert_eq!(computed["range"]["cursor"], "default");
        assert_eq!(computed["range"]["overflow"], "visible");

        assert_eq!(computed["color"]["boxSizing"], "border-box");
        assert_eq!(computed["color"]["width"], "50px");
        assert_eq!(computed["color"]["height"], "27px");
        assert_eq!(computed["color"]["padding"], "1px 2px");
        assert_eq!(computed["color"]["borderTopWidth"], "1px");

        assert_eq!(computed["disabledText"]["cursor"], "default");
        assert_eq!(
            computed["disabledText"]["backgroundColor"],
            "rgba(239, 239, 239, 0.3)"
        );
        assert_eq!(
            computed["disabledText"]["borderTopColor"],
            "rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(computed["disabledText"]["color"], "rgb(84, 84, 84)");

        assert_eq!(computed["textarea"]["fontFamily"], "monospace");
        assert_eq!(computed["textarea"]["padding"], "2px");
        assert_eq!(computed["textarea"]["borderTopWidth"], "1px");
        assert_eq!(computed["textarea"]["borderTopStyle"], "solid");
        assert_eq!(computed["textarea"]["whiteSpace"], "pre-wrap");
        assert_eq!(computed["textarea"]["overflow"], "auto");
        assert_eq!(computed["textarea"]["overflowWrap"], "break-word");

        assert_eq!(computed["select"]["boxSizing"], "border-box");
        assert_eq!(computed["select"]["borderTopWidth"], "1px");
        assert_eq!(computed["select"]["borderTopStyle"], "solid");
        assert_eq!(computed["select"]["whiteSpace"], "pre");
        assert_eq!(computed["select"]["cursor"], "default");
        assert_eq!(computed["disabledSelect"]["opacity"], "0.7");
        assert_eq!(
            computed["disabledSelect"]["borderTopColor"],
            "rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(computed["disabledSelect"]["color"], "rgb(109, 109, 109)");

        assert_eq!(computed["output"]["display"], "inline");
        assert_eq!(computed["meter"]["display"], "inline-block");
        assert_eq!(computed["meter"]["boxSizing"], "border-box");
        assert_eq!(computed["meter"]["width"], "80px");
        assert_eq!(computed["meter"]["height"], "16px");
        assert_eq!(computed["progress"]["display"], "inline-block");
        assert_eq!(computed["progress"]["boxSizing"], "border-box");
        assert_eq!(computed["progress"]["width"], "160px");
        assert_eq!(computed["progress"]["height"], "16px");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("form-control UA stylesheet fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn button_ua_defaults_and_flow_content_alignment_match_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/button-ua-layout.html")?,
        );
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        )));
        let css = format!(
            r#"
@font-face{{font-family:MoliAhem;src:url(data:font/ttf;base64,{font}) format('truetype')}}
html,body{{margin:0;padding:0}}
.control{{display:block;width:108px;height:44px;padding:0;border:0;font-family:MoliAhem;font-size:20px;line-height:20px;font-weight:400}}
#button{{background:rgb(1,2,3);color:rgb(11,12,13)}}
#input{{background:rgb(4,5,6);color:rgb(21,22,23)}}
#defaults,#disabled{{visibility:hidden}}
"#
        );
        page_vm.vm_mut().eval(&format!(
            "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
            serde_json::to_string(&css)?,
            serde_json::to_string(
                "<button id=button class=control>BBBB</button><input id=input class=control type=submit value=BBBB><button id=defaults>Default</button><button id=disabled disabled>Disabled</button>"
            )?,
        ))?;

        let computed = page_vm.vm_mut().eval(
            r#"
(() => {
  const read = id => {
    const style = getComputedStyle(document.getElementById(id));
    return {
      display: style.display,
      boxSizing: style.boxSizing,
      textAlign: style.textAlign,
      appearance: style.appearance,
      margin: style.margin,
      padding: style.padding,
      borderTopWidth: style.borderTopWidth,
      borderTopStyle: style.borderTopStyle,
      backgroundColor: style.backgroundColor,
      borderTopColor: style.borderTopColor,
      color: style.color,
      cursor: style.cursor,
      overflow: style.overflow,
      whiteSpace: style.whiteSpace,
      userSelect: style.userSelect,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      fontWeight: style.fontWeight,
      lineHeight: style.lineHeight
    };
  };
  return JSON.stringify({
    button: read('button'),
    input: read('input'),
    defaults: read('defaults'),
    disabled: read('disabled')
  });
})()
"#,
        )?;
        let computed: serde_json::Value = serde_json::from_str(&computed)?;
        assert_eq!(computed["button"]["display"], "block");
        assert_eq!(computed["button"]["boxSizing"], "border-box");
        assert_eq!(computed["button"]["textAlign"], "center");
        assert_eq!(computed["button"]["overflow"], "visible");
        assert_eq!(computed["button"]["whiteSpace"], "normal");
        assert_eq!(computed["input"]["display"], "block");
        assert_eq!(computed["input"]["boxSizing"], "border-box");
        assert_eq!(computed["input"]["textAlign"], "center");
        assert_eq!(computed["input"]["overflow"], "clip");
        assert_eq!(computed["input"]["whiteSpace"], "pre");
        assert_eq!(computed["input"]["userSelect"], "none");
        assert_eq!(computed["defaults"]["display"], "inline-block");
        assert_eq!(computed["defaults"]["boxSizing"], "border-box");
        assert_eq!(computed["defaults"]["textAlign"], "center");
        assert_eq!(computed["defaults"]["appearance"], "auto");
        assert_eq!(computed["defaults"]["margin"], "0px");
        assert_eq!(computed["defaults"]["padding"], "1px 6px");
        assert_eq!(computed["defaults"]["borderTopWidth"], "2px");
        assert_eq!(computed["defaults"]["borderTopStyle"], "outset");
        assert_eq!(computed["defaults"]["cursor"], "default");
        assert_eq!(computed["defaults"]["fontFamily"], "Arial, sans-serif");
        let default_font_size = computed["defaults"]["fontSize"]
            .as_str()
            .expect("computed font-size string")
            .trim_end_matches("px")
            .parse::<f32>()?;
        assert!((default_font_size - 13.3333).abs() <= 0.01);
        assert_eq!(computed["defaults"]["fontWeight"], "400");
        assert_eq!(computed["defaults"]["lineHeight"], "normal");
        assert_eq!(
            computed["disabled"]["backgroundColor"],
            "rgba(239, 239, 239, 0.3)"
        );
        assert_eq!(
            computed["disabled"]["borderTopColor"],
            "rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(computed["disabled"]["color"], "rgba(16, 16, 16, 0.3)");

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 120, 1.0))?
            .ok_or_else(|| anyhow::anyhow!("button fixture lost its layout root"))?;
        let rgb = |red: u8, green: u8, blue: u8| {
            moli_layout::PaintColor::new(
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                1.0,
            )
        };
        let glyphs = |color| {
            snapshot
                .fragments
                .iter()
                .filter_map(|fragment| match fragment {
                    moli_layout::PaintFragment::GlyphRun(run) if run.color == color => {
                        Some(run.glyphs_in_surface())
                    }
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>()
        };
        let assert_label = |label: &str,
                            actual: &[moli_layout::PaintGlyph],
                            expected_y: f32| {
            assert_eq!(actual.len(), 4, "{label}: {actual:?}");
            for (index, glyph) in actual.iter().enumerate() {
                let expected_x = 30.0 + index as f32 * 12.0;
                assert!(
                    (glyph.x - expected_x).abs() <= 0.05
                        && (glyph.y - expected_y).abs() <= 0.05,
                    "{label}[{index}]: actual={glyph:?}, expected=({expected_x}, {expected_y})"
                );
            }
        };
        assert_label("button label", &glyphs(rgb(11, 12, 13)), 28.0);
        assert_label("input label", &glyphs(rgb(21, 22, 23)), 72.0);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("button UA/layout fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn color_emoji_web_font_rasterizes_cbdt_png() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/color-emoji.html")?,
        );
        let color_emoji = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/noto-color-emoji-cbdt-subset.ttf.b64"
        ))
        .split_ascii_whitespace()
        .collect::<String>();
        let css = format!(
            r#"
@font-face{{font-family:MoliColorEmoji;src:url(data:font/ttf;base64,{color_emoji}) format('truetype')}}
html,body{{margin:0;padding:0;background:white}}
#emoji{{font-family:MoliColorEmoji;font-size:64px;line-height:80px;color:rgb(1,2,3);font-synthesis:none}}
"#
        );
        page_vm.vm_mut().eval(&format!(
            "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
            serde_json::to_string(&css)?,
            serde_json::to_string("<div id=emoji>®️⁉️8️⃣</div>")?,
        ))?;

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 100, 1.0))?
            .ok_or_else(|| anyhow::anyhow!("color emoji fixture lost its layout root"))?;
        let decoded_font = base64::engine::general_purpose::STANDARD.decode(&color_emoji)?;
        assert!(
            snapshot
                .fonts
                .iter()
                .any(|font| font.font.data.as_ref() == decoded_font),
            "the emoji run must retain the downloaded CBDT font in the owned snapshot"
        );

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let saturated_pixels = image
            .rgba
            .chunks_exact(4)
            .filter(|pixel| {
                let [red, green, blue, alpha] = <[u8; 4]>::try_from(*pixel).unwrap();
                let max = red.max(green).max(blue);
                let min = red.min(green).min(blue);
                alpha > 0 && max > 100 && max.saturating_sub(min) > 40
            })
            .count();
        assert!(
            saturated_pixels > 50,
            "CBDT glyphs must retain their embedded colors instead of using the near-black CSS text color; saturated_pixels={saturated_pixels}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("color emoji fixture should shape and rasterize");
}

#[tokio::test(flavor = "current_thread")]
async fn layout_demand_matches_the_fixed_font_inline_corpus() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-inline-corpus.html")?,
        );
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let latin = encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        )));
        let hebrew_emoji = encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-hebrew-emoji.ttf"
        )));
        let cjk = encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-cjk.ttf"
        )));
        let fixed_css = format!(
            r#"
@font-face{{font-family:MoliAhem;src:url(data:font/ttf;base64,{latin}) format('truetype')}}
@font-face{{font-family:MoliHebrewEmoji;src:url(data:font/ttf;base64,{hebrew_emoji}) format('truetype')}}
@font-face{{font-family:MoliCJK;src:url(data:font/ttf;base64,{cjk}) format('truetype')}}
html,body{{margin:0;padding:0}}
.fixed{{font-family:MoliAhem,MoliHebrewEmoji,MoliCJK;font-size:20px;line-height:20px}}
"#
        );
        let render = |page_vm: &mut PageVm,
                      case_css: &str,
                      body: &str|
         -> anyhow::Result<moli_layout::PaintSnapshot> {
            page_vm.vm_mut().eval(&format!(
                "document.documentElement.lang='en';document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
                serde_json::to_string(&(fixed_css.clone() + case_css))?,
                serde_json::to_string(body)?,
            ))?;
            page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 200, 1.0))?
                .ok_or_else(|| anyhow::anyhow!("inline fixture lost its layout root"))
        };
        let shared = render(
            &mut page_vm,
            r#"
#stream{width:80px}
#stream::before{content:'X';color:rgb(1,2,3)}
#a{color:rgb(11,12,13)}
#nested{color:rgb(21,22,23);margin:0 2px;padding:0 3px;border-left:1px solid;border-right:1px solid;background:rgb(4,5,6)}
#c{color:rgb(31,32,33)}#d{color:rgb(41,42,43)}#e{color:rgb(51,52,53)}
#trailing{color:rgb(61,62,63)}#after-trailing{width:10px;height:5px;background:rgb(71,72,73)}
"#,
            r#"<div id=stream class=fixed><span id=a>A</span><span id=nested>B</span><span id=c>C</span><br><span id=d>D</span><span id=e>E</span></div><div id=trailing class=fixed>A<br></div><div id=after-trailing></div>"#,
        )?;
        let whitespace = render(
            &mut page_vm,
            r#"
#collapse{width:60px}#upper{text-transform:uppercase;color:rgb(11,12,13)}
#ca{color:rgb(21,22,23)}#cb{color:rgb(31,32,33)}
#cjk{width:41px;font-family:MoliCJK,MoliHebrewEmoji;color:rgb(41,42,43)}
#preserve{white-space-collapse:preserve-breaks;width:60px;color:rgb(51,52,53)}
#breakspaces{white-space-collapse:break-spaces;width:36px;color:rgb(61,62,63)}
#nowrap{white-space:nowrap;width:24px;color:rgb(71,72,73)}
"#,
            "<div id=collapse class=fixed><span id=ca>A</span>  <span id=upper>ab</span>   <span id=cb>B</span></div><div id=cjk class=fixed>中\n文😀</div><div id=preserve class=fixed>A   B\nC</div><div id=breakspaces class=fixed>A   B</div><div id=nowrap class=fixed>ABC</div>",
        )?;
        let bidi = render(
            &mut page_vm,
            r#"
#bidi{direction:rtl;width:120px}#hebrew{font-family:MoliHebrewEmoji;color:rgb(11,12,13)}
#latin{direction:ltr;unicode-bidi:isolate;color:rgb(21,22,23)}
#emoji{font-family:MoliHebrewEmoji;color:rgb(31,32,33)}
#spacing{width:120px;letter-spacing:2px;word-spacing:4px;text-indent:12px;font-weight:625;font-stretch:87.5%;font-style:italic;color:rgb(41,42,43)}
"#,
            "<div id=bidi class=fixed><span id=hebrew>אב</span><span id=latin>AB</span><span id=emoji>😀</span></div><div id=spacing class=fixed>A A</div>",
        )?;
        let vertical = render(
            &mut page_vm,
            r#"
#align{width:140px}.atomic{display:inline-block}
#top{width:10px;height:30px;vertical-align:top;background:rgb(41,42,43)}
#bottom{width:10px;height:10px;vertical-align:bottom;background:rgb(51,52,53)}
#middle{width:10px;height:8px;vertical-align:middle;background:rgb(61,62,63)}
#raised{vertical-align:10px;color:rgb(71,72,73);background:rgb(1,2,3)}
#after{width:10px;height:5px;background:rgb(81,82,83)}
"#,
            "<div id=align class=fixed><span id=strut>A</span><span id=top class=atomic></span><span id=bottom class=atomic></span><span id=middle class=atomic></span><span id=raised>R</span></div><div id=after></div>",
        )?;
        let continuation = render(
            &mut page_vm,
            r#"#wrap-root{width:40px;word-break:break-all}#wrap{padding:0 1px;border-left:1px solid;border-right:1px solid;background:rgb(91,92,93);color:rgb(101,102,103)}"#,
            "<div id=wrap-root class=fixed><span id=wrap>ABCDE</span></div>",
        )?;
        let preserved_break_baseline = render(
            &mut page_vm,
            r#"
#baseline-wrapper{width:200px;font-size:0;line-height:0;background:rgb(111,112,113)}
#baseline-text,#baseline-break{display:inline-block;width:100px;height:200px}
#baseline-text{background:rgb(121,122,123)}
#baseline-break{white-space:pre;background:rgb(131,132,133)}
"#,
            "<div id=baseline-wrapper><div id=baseline-text>text</div><div id=baseline-break>\n</div></div>",
        )?;
        let rgb = |red: u8, green: u8, blue: u8| {
            moli_layout::PaintColor::new(
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                1.0,
            )
        };
        let glyph_runs =
            |snapshot: &moli_layout::PaintSnapshot,
             color: moli_layout::PaintColor| {
                snapshot
                    .fragments
                    .iter()
                    .filter_map(|fragment| match fragment {
                        moli_layout::PaintFragment::GlyphRun(run)
                            if run.color == color =>
                        {
                            Some(run.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
        let glyphs =
            |snapshot: &moli_layout::PaintSnapshot,
             color: moli_layout::PaintColor| {
                glyph_runs(snapshot, color)
                    .into_iter()
                    .flat_map(|run| run.glyphs_in_surface())
                    .collect::<Vec<_>>()
            };
        let solid_rects =
            |snapshot: &moli_layout::PaintSnapshot,
             color: moli_layout::PaintColor| {
                snapshot
                    .fragments
                    .iter()
                    .filter_map(|fragment| {
                        fragment
                            .solid_fill_in_surface()
                            .filter(|(_, actual)| *actual == color)
                            .map(|(rect, _)| rect)
                    })
                    .collect::<Vec<_>>()
            };
        let assert_points = |label: &str,
                             actual: &[moli_layout::PaintGlyph],
                             expected: &[(f32, f32)]| {
            assert_eq!(actual.len(), expected.len(), "{label}: {actual:?}");
            for (index, (glyph, (x, y))) in actual.iter().zip(expected).enumerate() {
                assert!(
                    (glyph.x - x).abs() <= 0.05 && (glyph.y - y).abs() <= 0.05,
                    "{label}[{index}]: actual={glyph:?}, expected=({x}, {y})"
                );
            }
        };
        let assert_rects = |label: &str,
                            actual: &[moli_layout::PaintRect],
                            expected: &[(f32, f32, f32, f32)]| {
            assert_eq!(actual.len(), expected.len(), "{label}: {actual:?}");
            for (index, (rect, (x, y, width, height))) in
                actual.iter().zip(expected).enumerate()
            {
                assert!(
                    (rect.x - x).abs() <= 0.05
                        && (rect.y - y).abs() <= 0.05
                        && (rect.width - width).abs() <= 0.05
                        && (rect.height - height).abs() <= 0.05,
                    "{label}[{index}]: actual={rect:?}, expected=({x}, {y}, {width}, {height})"
                );
            }
        };
        assert_rects(
            "shared nested inline fragment",
            &solid_rects(&shared, rgb(4, 5, 6)),
            &[(26.0, 0.0, 20.0, 20.0)],
        );
        for (label, color, expected) in [
            ("pseudo", rgb(1, 2, 3), vec![(0.0, 16.0)]),
            ("first span", rgb(11, 12, 13), vec![(12.0, 16.0)]),
            ("nested span", rgb(21, 22, 23), vec![(30.0, 16.0)]),
            ("third span", rgb(31, 32, 33), vec![(48.0, 16.0)]),
            ("post-br first", rgb(41, 42, 43), vec![(0.0, 36.0)]),
            ("post-br second", rgb(51, 52, 53), vec![(12.0, 36.0)]),
        ] {
            assert_points(label, &glyphs(&shared, color), &expected);
        }
        assert_points(
            "trailing br keeps exactly one line box",
            &glyphs(&shared, rgb(61, 62, 63)),
            &[(0.0, 56.0)],
        );
        assert_rects(
            "block following trailing br",
            &solid_rects(&shared, rgb(71, 72, 73)),
            &[(0.0, 60.0, 10.0, 5.0)],
        );

        assert_points(
            "collapsed and transformed text",
            &[
                glyphs(&whitespace, rgb(21, 22, 23))[0],
                glyphs(&whitespace, rgb(11, 12, 13))[0],
                glyphs(&whitespace, rgb(11, 12, 13))[1],
                glyphs(&whitespace, rgb(31, 32, 33))[0],
            ],
            &[(0.0, 16.0), (24.0, 16.0), (36.0, 16.0), (0.0, 36.0)],
        );
        assert_points(
            "cjk segment break and emoji fallback",
            &glyphs(&whitespace, rgb(41, 42, 43)),
            &[(0.0, 58.0), (20.0, 58.0), (0.0, 78.0), (20.0, 78.0)],
        );
        assert_points(
            "preserve-breaks",
            &glyphs(&whitespace, rgb(51, 52, 53)),
            &[(0.0, 96.0), (12.0, 96.0), (24.0, 96.0), (0.0, 116.0)],
        );
        assert_points(
            "break-spaces",
            &glyphs(&whitespace, rgb(61, 62, 63)),
            &[
                (0.0, 136.0),
                (12.0, 136.0),
                (24.0, 136.0),
                (0.0, 156.0),
                (12.0, 156.0),
            ],
        );
        assert_points(
            "nowrap",
            &glyphs(&whitespace, rgb(71, 72, 73)),
            &[(0.0, 176.0), (12.0, 176.0), (24.0, 176.0)],
        );
        let cjk_runs = glyph_runs(&whitespace, rgb(41, 42, 43));
        assert_eq!(cjk_runs.len(), 3, "CJK and emoji must select separate faces");
        assert_eq!(
            whitespace.fonts[cjk_runs[0].font.index()]
                .font
                .data
                .as_ref(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-cjk.ttf"
            ))
        );
        assert_eq!(
            whitespace.fonts[cjk_runs[2].font.index()]
                .font
                .data
                .as_ref(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-hebrew-emoji.ttf"
            ))
        );

        assert_points(
            "rtl emoji",
            &glyphs(&bidi, rgb(31, 32, 33)),
            &[(50.218_75, 17.0)],
        );
        assert_points(
            "isolated latin run",
            &glyphs(&bidi, rgb(21, 22, 23)),
            &[(71.068_36, 17.0), (83.068_36, 17.0)],
        );
        assert_points(
            "visual hebrew run",
            &glyphs(&bidi, rgb(11, 12, 13)),
            &[(95.068_36, 17.0), (106.630_86, 17.0)],
        );
        let spacing_runs = glyph_runs(&bidi, rgb(41, 42, 43));
        assert_eq!(spacing_runs.len(), 1, "spacing fixture must remain one line");
        assert_points(
            "indent letter and word spacing",
            &spacing_runs[0].glyphs_in_surface(),
            &[(12.0, 37.0), (26.0, 37.0), (44.0, 37.0)],
        );
        assert!(
            spacing_runs[0]
                .glyph_skew_radians
                .is_some_and(|skew| (skew - 0.244_346_1).abs() <= 0.000_1),
            "font-style synthesis must survive snapshot projection: {spacing_runs:?}"
        );

        for (label, color, expected) in [
            ("raised inline background", rgb(1, 2, 3), (42.0, 0.0, 12.0, 20.0)),
            ("top atomic", rgb(41, 42, 43), (12.0, 0.0, 10.0, 30.0)),
            ("bottom atomic", rgb(51, 52, 53), (22.0, 20.0, 10.0, 10.0)),
            ("middle atomic", rgb(61, 62, 63), (32.0, 14.0, 10.0, 8.0)),
            ("following block", rgb(81, 82, 83), (0.0, 30.0, 10.0, 5.0)),
        ] {
            assert_rects(label, &solid_rects(&vertical, color), &[expected]);
        }
        assert_points(
            "baseline strut",
            &glyphs(&vertical, moli_layout::PaintColor::BLACK),
            &[(0.0, 26.0)],
        );
        assert_points(
            "raised baseline shift",
            &glyphs(&vertical, rgb(71, 72, 73)),
            &[(42.0, 16.0)],
        );

        assert_rects(
            "inline continuation backgrounds",
            &solid_rects(&continuation, rgb(91, 92, 93)),
            &[(0.0, 0.0, 38.0, 20.0), (0.0, 20.0, 26.0, 20.0)],
        );
        let continuation_borders = continuation
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::Border { rect, widths, .. } => {
                    Some((*rect, *widths))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(continuation_borders.len(), 2);
        assert_eq!(continuation_borders[0].1.left, 1.0);
        assert_eq!(continuation_borders[0].1.right, 0.0);
        assert_eq!(continuation_borders[1].1.left, 0.0);
        assert_eq!(continuation_borders[1].1.right, 1.0);
        assert_points(
            "inline continuation glyphs",
            &glyphs(&continuation, rgb(101, 102, 103)),
            &[
                (2.0, 16.0),
                (14.0, 16.0),
                (26.0, 16.0),
                (0.0, 36.0),
                (12.0, 36.0),
            ],
        );
        assert_rects(
            "zero-sized text inline-block baseline",
            &solid_rects(&preserved_break_baseline, rgb(121, 122, 123)),
            &[(0.0, 0.0, 100.0, 200.0)],
        );
        assert_rects(
            "preserved-break inline-block baseline",
            &solid_rects(&preserved_break_baseline, rgb(131, 132, 133)),
            &[(100.0, 0.0, 100.0, 200.0)],
        );
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (3, 3, 3),
            "repeated one-shot mutation/layout demands must retain exactly the current faces"
        );

        assert!(
            shared.fonts.iter().any(|resource| {
                resource.font.data.as_ref()
                    == include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../moli-layout/tests/fixtures/moli-ahem.ttf"
                    ))
                    .as_slice()
            }),
            "snapshot must own the selected fixed Latin face"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("fixed-font inline corpus should run");
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_autofocus_runs_as_a_rendering_update_after_domcontentloaded() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/post-parse-autofocus").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `<input id="defaultFocus" autofocus>`;
globalThis.__autofocusOrder = [];
document.addEventListener("DOMContentLoaded", () => {
  __autofocusOrder.push("dcl");
  Promise.resolve().then(() => __autofocusOrder.push("dcl-microtask"));
});
defaultFocus.addEventListener("focus", () => {
  __autofocusOrder.push("focus");
  Promise.resolve().then(() => __autofocusOrder.push("focus-microtask"));
});
"installed"
"#,
        )?;
        let owner =
            dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;

        assert_eq!(
            page_vm.vm_mut().eval("__autofocusOrder.join('|')")?,
            "dcl|dcl-microtask",
            "DOMContentLoaded and its checkpoint must finish before autofocus"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === defaultFocus")?,
            "false",
            "the lifecycle body must only publish autofocus rendering work"
        );

        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("post-parse autofocus should publish one exact rendering task");
        let (selected_owner, selected_kind) = claimed
            .rendering_update_owner_and_kind()
            .expect("rendering selector must preserve the exact task identity");
        assert_eq!(
            selected_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(owner)
        );
        assert_eq!(
            selected_kind,
            RendererPageRenderingUpdateTaskKind::PostParseAutofocus
        );
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__autofocusOrder.join('|')")?,
            "dcl|dcl-microtask|focus|focus-microtask",
            "the selected rendering task must own autofocus callback completion"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === defaultFocus")?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse autofocus rendering-update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn domcontentloaded_microtask_focus_prevents_autofocus_task_admission() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/manual-focus-before-autofocus").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `
  <input id="defaultFocus" autofocus>
  <input id="manualFocus">
`;
document.addEventListener("DOMContentLoaded", () => {
  Promise.resolve().then(() => manualFocus.focus());
});
"installed"
"#,
        )?;
        dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === manualFocus")?,
            "true",
            "DOMContentLoaded's checkpoint must settle manual focus before admission"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                )
                .is_none(),
            "a Document that acquired focus must not publish redundant autofocus work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("manual focus admission test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_autofocus_rendering_task_does_not_retarget_document_open_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/stale-post-parse-autofocus").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `<input id="retiredFocus" autofocus>`;
globalThis.__retiredAutofocusEvents = 0;
retiredFocus.addEventListener("focus", () => __retiredAutofocusEvents++);
"installed"
"#,
        )?;
        let retired_owner =
            dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired Document should publish one exact autofocus task");
        let (claimed_owner, claimed_kind) = claimed
            .rendering_update_owner_and_kind()
            .expect("rendering claim must retain its exact owner and kind");
        assert_eq!(
            claimed_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(retired_owner)
        );
        assert_eq!(
            claimed_kind,
            RendererPageRenderingUpdateTaskKind::PostParseAutofocus
        );

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write('<!doctype html><body><input id="replacementFocus" autofocus></body>');
document.close();
"replaced"
"#,
        )?;
        let replacement_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement Document owner");
        assert_ne!(retired_owner, replacement_owner);

        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(__retiredAutofocusEvents)")?,
            "0",
            "a claimed old-Document task must not dispatch into the replacement"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === replacementFocus")?,
            "false",
            "stale settlement must not reuse the payload against a colliding replacement"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale autofocus rendering-update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_update_body_leaves_reactions_and_runtime_scripts_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/rendering-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__renderingTaskBoundary = [];
document.addEventListener("scroll", () => {
  __renderingTaskBoundary.push("callback");
  Promise.resolve().then(() => {
    __renderingTaskBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__renderingTaskBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
});
scrollTo(0, 10);
"queued"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let task = page_vm
            .take_rendering_update_body_task_for_test()
            .expect("one exact rendering-update task should be ready");
        let body = page_vm.apply_selected_page_rendering_update_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageRenderingUpdateTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval("__renderingTaskBoundary.join('|')")?,
            "callback",
            "the rendering-update body must leave listener reactions pending"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the rendering-update body must not consume unrelated runtime residence"
        );

        page_vm
            .finish_selected_page_task_completion(body.action.into_page_task_completion(), &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__renderingTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected completion must own the checkpoint and runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rendering-update body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_update_without_a_live_event_target_only_checkpoints() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/rendering-missing-event-target").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.head.appendChild(document.createElement("style")).textContent = `
  @keyframes removed { from { left: 0px; } to { left: 10px; } }
  #removed-animation { position: relative; animation: removed 1s linear; }
`;
const removed = document.createElement("div");
removed.id = "removed-animation";
removed.addEventListener("animationstart", () => {
  throw new Error("a removed animation target must not receive its queued scan");
});
document.body.appendChild(removed);
removed.remove();
"queued-then-removed"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let task = page_vm
            .take_rendering_update_body_task_for_test()
            .expect("the removed target's exact animation scan should remain queued");
        assert_eq!(
            task.kind(),
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
        let body = page_vm.apply_selected_page_rendering_update_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageRenderingUpdateTargetEffect::CurrentOwnerHadNoEventTarget
        );
        page_vm
            .finish_selected_page_task_completion(body.action.into_page_task_completion(), &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a current task with no callback only owns the agent checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rendering-update checkpoint-only test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn scroll_rendering_update_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/scroll-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
globalThis.__retiredScrollEvents = 0;
document.addEventListener("scroll", () => __retiredScrollEvents++);
document.addEventListener("scrollend", () => __retiredScrollEvents++);
scrollTo(0, 10);
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"replaced"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id,
            after_document.local_window_id
        );

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired Document rendering work should remain an exact source entry");
        let (stale_owner, stale_kind) = stale
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain the retired task identity");
        assert_eq!(
            stale_kind,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
        );
        assert_eq!(
            stale_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(before_document),
            "the claimed task must retain the retired exact Document"
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(page_vm.vm_mut().eval("String(__retiredScrollEvents)")?, "0");

        page_vm.vm_mut().eval(
            r#"
globalThis.__currentScrollEvents = [];
document.addEventListener("scroll", () => __currentScrollEvents.push("scroll"));
document.addEventListener("scrollend", () => __currentScrollEvents.push("scrollend"));
scrollTo(0, 20);
"queued-current"
"#,
        )?;
        let current = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("replacement Document rendering work should remain runnable");
        let (current_owner, selected_kind) = current
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain its exact owner and kind");
        assert_ne!(stale_owner, current_owner);
        assert_eq!(
            current_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(after_document)
        );
        assert_eq!(
            selected_kind,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
        );
        page_vm
            .run_claimed_selected_page_task_for_test(current, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__currentScrollEvents.join('|')")?,
            "scroll|scrollend"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact rendering update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn animation_rendering_update_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/animation-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
document.head.appendChild(document.createElement("style")).textContent = `
  @keyframes retired { from { left: 0px; } to { left: 10px; } }
  #retired-animation { position: relative; animation: retired 1s linear; }
`;
document.body.innerHTML = `<div id="retired-animation"></div>`;
globalThis.__retiredAnimationEvents = 0;
document.getElementById("retired-animation").addEventListener(
  "animationstart",
  () => __retiredAnimationEvents++
);
document.open();
document.write("<!doctype html><title>replacement</title><body></body>");
document.close();
"replaced"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired Document animation work should remain an exact source entry");
        let (stale_owner, stale_kind) = stale
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain the retired task identity");
        assert_eq!(
            stale_kind,
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
        assert_eq!(
            stale_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(before_document),
            "the claimed task must retain the retired exact Document"
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(__retiredAnimationEvents)")?,
            "0"
        );

        page_vm.vm_mut().eval(
            r#"
document.head.appendChild(document.createElement("style")).textContent = `
  @keyframes current { from { left: 0px; } to { left: 10px; } }
  #current-animation { position: relative; animation: current 1s linear; }
`;
document.body.innerHTML = `<div id="current-animation"></div>`;
globalThis.__currentAnimationEvents = 0;
document.getElementById("current-animation").addEventListener(
  "animationstart",
  () => __currentAnimationEvents++
);
"queued-current"
"#,
        )?;
        assert!(!page_vm.vm().has_ready_timeout());
        let current = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("replacement Document animation work should remain runnable");
        let (current_owner, selected_kind) = current
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain its exact owner and kind");
        assert_eq!(
            selected_kind,
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
        assert_ne!(stale_owner, current_owner);
        assert_eq!(
            current_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(after_document)
        );
        page_vm
            .run_claimed_selected_page_task_for_test(current, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(__currentAnimationEvents)")?,
            "1"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact animation rendering update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn scroll_rendering_update_discards_a_retired_child_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/scroll-stale-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "scroll-stale-child";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "scroll-stale-child")?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__retiredChildScrollEvents = 0;
const staleFrame = document.getElementById("scroll-stale-child");
staleFrame.contentDocument.addEventListener(
  "scroll",
  () => parent.__retiredChildScrollEvents++
);
staleFrame.contentDocument.addEventListener(
  "scrollend",
  () => parent.__retiredChildScrollEvents++
);
staleFrame.contentWindow.scrollTo(0, 12);
staleFrame.remove();
"retired"
"#,
        )?;

        assert!(!page_vm.vm().has_ready_timeout());
        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired child rendering work should remain an exact source entry");
        let (stale_owner, stale_kind) = stale
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain the retired child task identity");
        assert_eq!(
            stale_kind,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
        );
        assert_eq!(
            stale_owner.root_document(),
            page_vm.document_lifecycle.identity().document
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__retiredChildScrollEvents)")?,
            "0"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                )
                .is_none(),
            "stale settlement must retire the child Host-local payload"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child-Document-exact rendering update test should run");
}

#[test]
fn rendering_update_rejects_a_real_page_vm_replacement_id_collision() {
    run_page_vm_large_stack_async_test(
        "rendering-update-page-vm-replacement-id-collision",
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
                    page_vm.vm_mut().eval("scrollTo(0, 10); 'queued-retired'")?;
                    let retired_root = page_vm.document_lifecycle.identity().document;

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'navigating'"))?;
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

                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__replacementScrollEvents = [];
document.addEventListener("scroll", () => __replacementScrollEvents.push("scroll"));
document.addEventListener("scrollend", () => __replacementScrollEvents.push("scrollend"));
scrollTo(0, 20);
"queued-current"
"#,
                    )?;

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::RenderingUpdate,
                        )
                        .expect("retired PageVm rendering task should consume one stale turn");
                    let (stale_owner, stale_kind) = stale
                        .rendering_update_owner_and_kind()
                        .expect("rendering selector must retain the retired task identity");
                    assert_eq!(
                        stale_owner.root_document(),
                        retired_root,
                        "the first selected task must remain bound to the retired PageVm"
                    );
                    assert_eq!(
                        stale_kind,
                        RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;

                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::RenderingUpdate,
                        )
                        .expect("replacement rendering task must survive stale-head settlement");
                    let (selected_owner, _) = current
                        .rendering_update_owner_and_kind()
                        .expect("rendering selector must retain its exact owner");
                    assert_eq!(selected_owner.root_document(), current_root);
                    assert_ne!(stale_owner, selected_owner);
                    assert_eq!(
                        stale_owner.target(),
                        selected_owner.target(),
                        "fresh PageVm counters should naturally reuse the local Document target"
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementScrollEvents.join('|')")?,
                        "scroll|scrollend"
                    );
                    assert!(!page_vm.vm().has_ready_timeout());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("rendering update replacement should use exact root arbitration");
            server
                .await
                .expect("rendering update replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_update_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/rendering-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__renderingChildOrder = [];
document.addEventListener("scroll", () => {
  __renderingChildOrder.push("callback");
  Promise.resolve().then(() => {
    __renderingChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "rendering-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
scrollTo(0, 15);
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate, &loader)
                .await?,
            "the exact rendering update should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__renderingChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must precede callback child-record synchronization"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during callback completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rendering-update post-checkpoint child synchronization test should run");
}
