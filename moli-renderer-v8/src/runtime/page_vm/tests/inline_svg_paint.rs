use super::*;
use moli_page_types::LayoutPolicy;

const SVG_PUBLICATION_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/inline-svg-layout-publication.js"
));

#[tokio::test(flavor = "current_thread")]
async fn realtime_inline_svg_outer_boxes_defer_scene_decoding() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-outer-boxes.html")?,
        );
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 500,
            inner_height: 160,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))?;
        page_vm.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page_vm.vm_mut().eval(&format!(
            "{SVG_PUBLICATION_FIXTURE}\ninstallSvgLayoutPublication()"
        ))?;
        page_vm.vm_mut().eval(
            r#"
            document.getElementById('html').textContent = 'Body text';
            const title = document.createElementNS('http://www.w3.org/2000/svg', 'title');
            title.textContent = 'Icon label';
            document.getElementById('scene').prepend(title);
        "#,
        )?;
        let parsed = crate::layout_renderer::svg_parse_count();
        let boxes = r#"
            JSON.stringify([
                document.getElementById('html').getBoundingClientRect().width,
                document.getElementById('scene').getBoundingClientRect().width,
                document.getElementById('scene').getBoundingClientRect().height,
                document.getElementById('shape').getBBox().width,
            ])
        "#;
        assert_eq!(page_vm.vm_mut().eval(boxes)?, "[80,200,100,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed);
        assert_eq!(
            page_vm.vm_mut().eval("document.body.innerText")?,
            "Body text"
        );
        assert_eq!(
            crate::layout_renderer::svg_parse_count(),
            parsed,
            "text nodes do not need SVG object quads"
        );
        let passes = page_vm.vm().layout_pass_observability_for_test().1;

        let child_geometry = r#"
            JSON.stringify((r => [r.x,r.y,r.width,r.height])(
                document.getElementById('shape').getBoundingClientRect()
            ))
        "#;
        assert_eq!(page_vm.vm_mut().eval(child_geometry)?, "[110,50,10,10]");
        assert_eq!(page_vm.vm_mut().eval(child_geometry)?, "[110,50,10,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, passes);

        page_vm.vm_mut().eval("mutateSvgLayoutPublication()")?;
        assert_eq!(page_vm.vm_mut().eval(boxes)?, "[300,200,100,100]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        assert_eq!(page_vm.vm_mut().eval(child_geometry)?, "[330,50,100,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 160, 1.0))?
            .expect("screenshot requests paint");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = (55 * raster.width as usize + 335) * 4;
        assert_eq!(&raster.rgba[pixel..pixel + 4], [0, 255, 0, 255]);
        assert_eq!(page_vm.vm_mut().eval(child_geometry)?, "[330,50,100,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 3);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("only SVG descendant queries and paint need a decoded scene");
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_inline_svg_geometry_decodes_only_the_queried_scene_in_a_frame() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-embedded-geometry.html")?,
        );
        page_vm.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page_vm.vm_mut().eval(r#"
            document.body.innerHTML = '<iframe id="frame" style="width:300px;height:200px"></iframe>';
            const child = document.getElementById('frame').contentDocument;
            child.body.style.margin = '0';
            child.body.innerHTML =
                '<svg style="display:contents">' +
                '<svg id="scene" style="display:block;width:120px;height:80px;zoom:2;border:1px solid;padding:3px">' +
                '<svg x="10" y="20" width="40" height="30">' +
                '<rect x="2" y="3" width="25" height="15"/></svg></svg></svg>' +
                '<svg width="5" height="5"><rect width="5" height="5"/></svg>'.repeat(499);
        "#)?;
        let parsed = crate::layout_renderer::svg_parse_count();
        assert_eq!(
            page_vm.vm_mut().eval("child.getElementById('scene').getBoundingClientRect().width")?,
            "256"
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed);
        let passes = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page_vm.vm_mut().eval(r#"
                const range = child.createRange();
                range.selectNode(child.querySelector('rect'));
                JSON.stringify((r => [r.x, r.y, r.width, r.height])(range.getBoundingClientRect()))
            "#)?,
            "[32,54,50,30]"
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, passes);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("an SVG range prepares only its scene and reuses the frame layout");
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_inline_svg_range_geometry_follows_transform_and_scroll() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-range-projection.html")?,
        );
        page_vm.set_scrollbars_hidden(true)?;
        page_vm.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page_vm.vm_mut().eval(
            r#"
            document.head.innerHTML =
                '<style>html,body{margin:0}body{height:2000px}svg{display:block}</style>';
            const scene = '<svg width="120" height="60">' +
                '<rect x="10" y="20" width="10" height="10"/>' +
                '<rect x="30" y="20" width="10" height="10"/></svg>';
            document.body.innerHTML =
                '<div id="wrapper" style="transform:translate(0px,0px)">' +
                scene.repeat(2) + '</div>';
            const wrapper = document.getElementById('wrapper');
            const range = document.createRange();
            range.selectNodeContents(wrapper);
            const readRects = () => JSON.stringify(Array.from(range.getClientRects())
                .filter(r => r.width === 10 && r.height === 10)
                .map(r => [r.x, r.y, r.width, r.height]));
        "#,
        )?;
        let parsed = crate::layout_renderer::svg_parse_count();
        const INITIAL: &str = "[[10,20,10,10],[30,20,10,10],[10,80,10,10],[30,80,10,10]]";
        assert_eq!(page_vm.vm_mut().eval("readRects()")?, INITIAL);
        let passes = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        assert_eq!(page_vm.vm_mut().eval("readRects()")?, INITIAL);
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("wrapper.style.transform='translate(17px,23px)';readRects()")?,
            "[[27,43,10,10],[47,43,10,10],[27,103,10,10],[47,103,10,10]]"
        );
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 4);

        assert_eq!(
            page_vm.vm_mut().eval("scrollTo(0,30);readRects()")?,
            "[[27,13,10,10],[47,13,10,10],[27,73,10,10],[47,73,10,10]]"
        );
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 4);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("batched SVG geometry tracks transform and scroll in one layout publication");
}

#[tokio::test(flavor = "current_thread")]
async fn ondemand_inline_svg_decodes_the_frozen_scene_after_a_live_move() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/frozen-svg-input.html")?,
        );
        page_vm.vm_mut().set_layout_policy(LayoutPolicy::OnDemand);
        let vm = page_vm.vm_mut();
        vm.eval(
            r#"
            document.body.style.margin = '0';
            document.body.innerHTML =
                '<svg style="display:block" width="100" height="40">' +
                '<rect id="shape" x="10" y="5" width="12" height="10"/></svg>' +
                '<svg id="other" style="display:block" width="100" height="40">' +
                '<rect id="neighbor" x="5" y="7" width="10" height="10"/></svg>';
            const shape = document.getElementById('shape');
            const other = document.getElementById('other');
            const neighbor = document.getElementById('neighbor');
            const rect = node => JSON.stringify((r => [r.x, r.y, r.width, r.height])(
                node.getBoundingClientRect()
            ));
        "#,
        )?;
        let parsed = crate::layout_renderer::svg_parse_count();
        assert_eq!(vm.eval("rect(other)")?, "[0,40,100,40]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed);
        let passes = vm.layout_pass_observability_for_test().1;
        vm.eval("shape.setAttribute('width','48'); other.append(shape)")?;

        assert_eq!(vm.eval("rect(shape)")?, "[10,5,12,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        assert_eq!(vm.eval("rect(shape)")?, "[10,5,12,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        assert_eq!(vm.eval("rect(neighbor)")?, "[5,47,10,10]");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("OnDemand geometry decodes the input and source identities from its snapshot");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_capture_decodes_only_the_effective_paint_region() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            let mut page_vm = test_page_vm_with_loader_and_document_url(
                &loader,
                Vec::new(),
                Url::parse("https://example.com/inline-svg-capture-region.html")?,
            );
            let viewport = moli_layout::LayoutViewport::new(160, 100, 1.0);
            page_vm.set_scrollbars_hidden(true)?;
            page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: viewport.css_width,
                inner_height: viewport.css_height,
                device_pixel_ratio: 1.0,
                ..Default::default()
            }))?;
            page_vm.vm_mut().set_layout_policy(policy);
            let vm = page_vm.vm_mut();
            vm.eval(
                r#"
                const icon = (top, fill) => '<svg width="20" height="20" ' +
                    'style="position:absolute;top:' + top + 'px;left:0">' +
                    '<rect width="20" height="20" fill="' + fill + '"/></svg>';
                document.body.style.margin = '0';
                const frame = (id, top) => '<iframe id="' + id + '" style="position:absolute;' +
                    'left:50px;top:' + top + 'px;width:80px;height:80px;border:0"></iframe>';
                document.body.innerHTML = icon(10, 'red') + frame('near', 10) + frame('far', 250) +
                    '<div style="position:absolute;top:0;left:30px;width:20px;height:10px;' +
                    'overflow:hidden">' + icon(30, 'lime') + '</div>' +
                    '<svg width="20" height="20" style="position:absolute;top:110px;left:135px;' +
                    'filter:drop-shadow(0 -25px 0 magenta)">' +
                    '<rect width="20" height="20" fill="blue"/></svg>' +
                    Array.from({length:64}, (_, i) => icon(400 + 25*i, 'blue')).join('');
                const install = (id, contents) => {
                    const child = document.getElementById(id).contentDocument;
                    child.body.style.margin = '0';
                    child.body.innerHTML = contents;
                };
                install('near', icon(5, 'lime') + icon(40, 'blue') + icon(100, 'red'));
                install('far', icon(5, 'lime'));
            "#,
            )?;
            let parsed = crate::layout_renderer::svg_parse_count();
            let shot = vm
                .screenshot_layout_snapshot(viewport)?
                .expect("viewport screenshot");
            assert_eq!(
                crate::layout_renderer::svg_parse_count(),
                parsed + 4,
                "{policy:?}"
            );
            let screen = moli_paint::raster_snapshot(&shot)?;
            let pixel = (20 * screen.width as usize + 55) * 4;
            assert_eq!(&screen.rgba[pixel..pixel + 4], [0, 255, 0, 255]);
            let shadow = (90 * screen.width as usize + 140) * 4;
            assert_eq!(&screen.rgba[shadow..shadow + 4], [255, 0, 255, 255]);

            let clip = moli_layout::LayoutRect::new(50.0, 10.0, 80.0, 20.0);
            let shot = vm
                .paint_layout_snapshot_with_capture(
                    viewport,
                    moli_layout::LayoutFlushReason::Screenshot,
                    moli_layout::PaintCaptureRequest::page_clip(clip, 1.0),
                    crate::native_bridge::LayoutPublicationRequest::Transient,
                    None,
                )?
                .expect("clipped screenshot");
            assert_eq!(
                crate::layout_renderer::svg_parse_count(),
                parsed + 5,
                "{policy:?}"
            );
            let crop = moli_paint::raster_snapshot(&shot)?;
            for (y, row) in crop.rgba.as_chunks::<{ 80 * 4 }>().0.iter().enumerate() {
                let start = ((10 + y) * screen.width as usize + 50) * 4;
                assert_eq!(row, &screen.rgba[start..start + row.len()]);
            }

            let shot = vm
                .paint_layout_snapshot_with_capture(
                    viewport,
                    moli_layout::LayoutFlushReason::Screenshot,
                    moli_layout::PaintCaptureRequest::full_document(),
                    crate::native_bridge::LayoutPublicationRequest::Transient,
                    None,
                )?
                .expect("full-page screenshot");
            assert_eq!(
                crate::layout_renderer::svg_parse_count(),
                parsed + 5 + 69,
                "{policy:?}"
            );
            let full = moli_paint::raster_snapshot(&shot)?;
            let pixel = (260 * full.width as usize + 55) * 4;
            assert_eq!(&full.rgba[pixel..pixel + 4], [0, 255, 0, 255]);
            for (y, row) in screen.rgba.as_chunks::<{ 160 * 4 }>().0.iter().enumerate() {
                let start = y * full.width as usize * 4;
                assert_eq!(row, &full.rgba[start..start + row.len()]);
            }

            #[cfg(feature = "ace-protocol")]
            {
                vm.eval("document.querySelector('svg rect').id = 'cropTarget'")?;
                let source = vm.document_runtime.get_element_by_id("cropTarget").unwrap();
                let parsed = crate::layout_renderer::svg_parse_count();
                let bounds = vm.node_screenshot_clip(source, viewport)?;
                assert_eq!(
                    [bounds.x, bounds.y, bounds.width, bounds.height],
                    [0.0, 10.0, 20.0, 20.0]
                );
                assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("viewport, clipped, and full-page captures select their own SVG scenes");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_and_iframe_borders_paint_without_preparing_contents() {
    use moli_layout::{LayoutFlushReason, LayoutRect, LayoutViewport, PaintCaptureRequest};

    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            let mut page_vm = test_page_vm_with_loader_and_document_url(
                &loader, Vec::new(), Url::parse("https://example.com/replaced-border.html")?,
            );
            let viewport = LayoutViewport::new(180, 100, 1.0);
            page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: viewport.css_width,
                inner_height: viewport.css_height,
                device_pixel_ratio: 1.0,
                ..Default::default()
            }))?;
            page_vm.vm_mut().set_layout_policy(policy);
            let vm = page_vm.vm_mut();
            vm.eval(r#"
                document.body.style.margin = '0';
                document.body.innerHTML = '<style>svg,iframe{position:absolute;top:0;width:40px;height:40px;' +
                    'border:20px solid blue;overflow:hidden} svg{left:0}iframe{left:90px}</style>' +
                    '<svg><rect width="40" height="40" fill="red"/></svg><iframe id="frame"></iframe>';
                const child = document.getElementById('frame').contentDocument;
                child.body.style.margin = '0';
                child.body.innerHTML = '<svg width="40" height="40"><rect width="40" height="40" fill="lime"/></svg>';
            "#)?;
            let count = crate::layout_renderer::svg_parse_count();
            let clip = LayoutRect::new(0.0, 0.0, 180.0, 8.0);
            let border = vm.paint_layout_snapshot_with_capture(
                viewport, LayoutFlushReason::Screenshot, PaintCaptureRequest::page_clip(clip, 1.0),
                crate::native_bridge::LayoutPublicationRequest::Screen, None,
            )?.expect("border capture");
            assert_eq!(crate::layout_renderer::svg_parse_count(), count, "{policy:?}");
            let layouts = vm.layout_pass_observability_for_test().1;
            assert_eq!(vm.eval("child.querySelector('svg').getBoundingClientRect().width")?, "40");
            assert_eq!(vm.layout_pass_observability_for_test().1, layouts);
            assert_eq!(crate::layout_renderer::svg_parse_count(), count);
            let border = moli_paint::raster_snapshot(&border)?;
            let screen = vm.screenshot_layout_snapshot(viewport)?.expect("screen capture");
            assert_eq!(crate::layout_renderer::svg_parse_count(), count + 2, "{policy:?}");
            let screen = moli_paint::raster_snapshot(&screen)?;
            assert_eq!(border.rgba, screen.rgba[..border.rgba.len()]);
            for (x, rgba) in [(35, [255, 0, 0, 255]), (120, [0, 255, 0, 255])] {
                let pixel = (35 * screen.width as usize + x) * 4;
                assert_eq!(&screen.rgba[pixel..pixel+4], rgba);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSS border paint has no SVG or frame content dependency");
}

#[cfg(feature = "devtools")]
#[tokio::test(flavor = "current_thread")]
async fn realtime_screencast_decodes_new_svg_content_as_it_scrolls_into_view() {
    run_page_vm_async_test(async move {
        use moli_layout::{LayoutFlushReason, LayoutViewport, PaintCaptureRequest};

        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/retained-svg-screen-paint.html")?,
        );
        let viewport = LayoutViewport::new(160, 100, 1.0);
        page_vm.set_scrollbars_hidden(true)?;
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: viewport.css_width,
            inner_height: viewport.css_height,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))?;
        page_vm.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        let vm = page_vm.vm_mut();
        vm.eval(r#"
            document.body.style.cssText = 'margin:0;height:1000px';
            document.body.innerHTML =
                '<svg style="display:block" width="20" height="20"><rect id="shape" width="20" height="20" fill="red"/></svg>' +
                '<iframe id="frame" style="position:absolute;top:300px;left:0;width:60px;height:60px;border:0"></iframe>';
            const child = document.getElementById('frame').contentDocument;
            child.body.style.margin = '0';
            child.body.innerHTML = '<svg style="display:block" width="20" height="20"><rect width="20" height="20" fill="blue"/></svg>';
        "#)?;
        let parsed = crate::layout_renderer::svg_parse_count();
        let session = crate::runtime::RendererScreencastPaintSession::default();
        let capture = |vm: &mut crate::script_vm::ScriptVm| -> anyhow::Result<_> {
            let frame = vm.paint_current_screen_layout(
                Some(&session),
                viewport,
                PaintCaptureRequest::viewport(),
                LayoutFlushReason::Screencast,
            )?.expect("retained screen paint");
            Ok(moli_paint::raster_snapshot(&frame)?)
        };
        let first = capture(vm)?;
        assert_eq!(&first.rgba[..4], [255, 0, 0, 255]);
        assert_eq!(vm.eval("document.getElementById('shape').getBoundingClientRect().width")?, "20");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        let passes = vm.layout_pass_observability_for_test().1;
        assert_eq!(capture(vm)?.rgba, first.rgba);
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);

        vm.eval("scrollTo(0, 300)")?;
        let scrolled = capture(vm)?;
        assert_eq!(&scrolled.rgba[..4], [0, 0, 255, 255]);
        assert_eq!(vm.eval("child.querySelector('rect').getBoundingClientRect().width")?, "20");
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        vm.eval("scrollTo(0, 0)")?;
        assert_eq!(capture(vm)?.rgba, first.rgba);
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);

        vm.eval("document.getElementById('shape').setAttribute('fill','lime')")?;
        assert_eq!(&capture(vm)?.rgba[..4], [0, 255, 0, 255]);
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 3);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("screen paint reuses decoded scenes until the layout inputs change");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_negative_intrinsic_lengths_respect_css_viewport_sizes() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-viewport-sizes.html")?,
        );
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/inline-svg-viewport-sizes.js"
        ));
        let metadata: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval(
            &format!("{fixture}\nJSON.stringify(installSvgViewportSizes())"),
        )?)?;
        let viewport = moli_layout::PaintViewport::new(
            metadata["width"].as_u64().unwrap() as u32,
            metadata["height"].as_u64().unwrap() as u32,
            1.0,
        );
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .unwrap();
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        for case in metadata["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let observed: serde_json::Value =
                serde_json::from_str(&page_vm.vm_mut().eval(&format!(
                    "JSON.stringify(readSvgViewportSize({}))",
                    serde_json::to_string(name)?
                ))?)?;
            assert_eq!(
                observed,
                json!([0, case["top"], case["size"][0], case["size"][1]]),
                "{name}"
            );
            let y = case["top"].as_u64().unwrap() as usize + 10;
            let offset = (y * raster.width as usize + 10) * 4;
            let visible =
                case["size"][0].as_u64().unwrap() > 0 && case["size"][1].as_u64().unwrap() > 0;
            assert_eq!(
                &raster.rgba[offset..offset + 4],
                if visible {
                    &[255, 0, 0, 255]
                } else {
                    &[255, 255, 255, 255]
                },
                "{name} paint"
            );
        }
        for (width, expected) in [("", 0), ("80px", 80), ("", 0)] {
            page_vm.vm_mut().eval(&format!(
                "document.getElementById('svg-viewport-css-width').style.width={};'changed'",
                serde_json::to_string(width)?,
            ))?;
            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(viewport)?
                .unwrap();
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("readSvgViewportSize('css-width')[2]")?,
                expected.to_string(),
                "changing CSS must reveal or override the negative intrinsic width",
            );
            let raster = moli_paint::raster_snapshot(&snapshot)?;
            let top = metadata["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["name"] == "css-width")
                .unwrap()["top"]
                .as_u64()
                .unwrap() as usize;
            let offset = ((top + 10) * raster.width as usize + 10) * 4;
            assert_eq!(
                &raster.rgba[offset..offset + 4],
                if expected > 0 {
                    &[255, 0, 0, 255]
                } else {
                    &[255, 255, 255, 255]
                }
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG negative sizes must match Chromium geometry and paint");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_zoom_preserves_font_and_stroke_user_units() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-scene-units.html")?,
        );
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/inline-svg-scene-inputs.js"
        ));
        let metadata = page_vm.vm_mut().eval(&format!(
            "{fixture}\nJSON.stringify(installSvgSceneInputs('units'))"
        ))?;
        let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(
                metadata["width"].as_u64().unwrap() as u32,
                metadata["height"].as_u64().unwrap() as u32,
                1.0,
            ))?
            .unwrap();
        let parsed = crate::layout_renderer::svg_parse_count();
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        for case in metadata["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let observed = page_vm.vm_mut().eval(&format!(
                "JSON.stringify(svgSceneInputRects({}))",
                serde_json::to_string(name)?
            ))?;
            let observed: serde_json::Value = serde_json::from_str(&observed)?;
            assert!(
                observed["target"][2].as_f64().unwrap() > 0.0,
                "{name}: {observed}"
            );
            for (target, reference) in observed["target"]
                .as_array()
                .unwrap()
                .iter()
                .zip(observed["reference"].as_array().unwrap())
            {
                assert!(
                    (target.as_f64().unwrap() - reference.as_f64().unwrap()).abs() < 0.01,
                    "{name}: {observed}"
                );
            }
            let top = case["top"].as_u64().unwrap() as usize;
            let mut ink = 0_u64;
            for y in top..top + 140 {
                for x in 0..200 {
                    let target = (y * raster.width as usize + x) * 4;
                    let reference = (y * raster.width as usize + x + 240) * 4;
                    assert_eq!(
                        &raster.rgba[target..target + 4],
                        &raster.rgba[reference..reference + 4],
                        "{name} pixel ({x},{y})"
                    );
                    ink += u64::from(255 - raster.rgba[reference]);
                }
            }
            assert!(
                ink > 1000,
                "{name}: reference must contain visible glyphs or strokes"
            );
        }
        assert_eq!(
            crate::layout_renderer::svg_parse_count(),
            parsed,
            "clean SVG reads must reuse the scene publication"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG zoom must match equivalent outer transforms in geometry and pixels");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_final_styles_do_not_become_author_selector_attributes() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-original-attributes.html")?,
        );
        page_vm.vm_mut().eval(r##"
            document.head.innerHTML = '<style>html,body{margin:0;background:white}svg{display:block}</style>';
            document.body.innerHTML = `<svg width="100" height="50">
              <style>rect[style]{transform:matrix(1,0,0,1,40,0);fill:blue!important}rect[id]{transform:matrix(1,0,0,1,60,0)}</style>
              <rect width="20" height="20" fill="red"/>
            </svg>`;
            'installed'
        "##)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 50, 1.0))?.unwrap();
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify((r=>[r.x,r.y,r.width,r.height])(document.querySelector('rect').getBoundingClientRect()))")?, "[0,0,20,20]");
        assert_eq!(page_vm.vm_mut().eval("document.querySelector('rect').getAttributeNames().join(',')")?, "width,height,fill");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: usize| &raster.rgba[(10 * raster.width as usize + x) * 4..][..4];
        assert_eq!(pixel(10), [255, 0, 0, 255]);
        assert_eq!(pixel(50), [255, 255, 255, 255]);
        assert_eq!(pixel(70), [255, 255, 255, 255]);
        Ok::<_, anyhow::Error>(())
    }).await.expect("final SVG styles must not change selector matching");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_dense_source_keeps_its_original_input_budget() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-dense.html")?,
        );
        page_vm.vm_mut().eval(r#"
            document.head.innerHTML = '<style>html,body{margin:0;background:white}svg{display:block}</style>';
            document.body.innerHTML = '<svg width="100" height="100">' + '<circle r="1"></circle>'.repeat(30000) + '</svg>';
            'installed'
        "#)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 100, 1.0))?.unwrap();
        assert_eq!(snapshot.svg_images.len(), 1, "690 KB of original XML must remain a vector image");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        // The sampled quarter-circle edge is antialiased; repeated source-over
        // composition can round to 1 rather than exactly 0.
        assert!(raster.rgba[..3].iter().all(|channel| *channel <= 2));
        assert_eq!(raster.rgba[3], 255);
        Ok::<_, anyhow::Error>(())
    }).await.expect("derived styles must not consume the encoded SVG input budget");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_batch_geometry_uses_one_frozen_scene() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-frozen-geometry.html")?,
        );
        page_vm.vm_mut().eval(&format!(
            "{SVG_PUBLICATION_FIXTURE}\ninstallSvgLayoutPublication()"
        ))?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        let parsed = crate::layout_renderer::svg_parse_count();
        let initial = page_vm
            .vm_mut()
            .eval("JSON.stringify(readSvgLayoutPublication())")?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&initial)?,
            json!({
                "htmlBefore":80,"svg":[100,40,200,100],"object":[110,50,10,10],"shapeCount":1,
                "range":[110,50,10,10],"rangeCount":1000,"htmlAfter":80,
            })
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 1,
            "the first geometry demand must publish once, including all SVG sources"
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        let publications = page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test()
            .2;

        // An unrelated HTML change must not turn an SVG read into a page refresh.
        page_vm
            .vm_mut()
            .eval("document.getElementById('html').style.width='300px';'changed'")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(readSvgLayoutPublication())")?,
            initial
        );
        page_vm.vm_mut().eval("mutateSvgLayoutPublication()")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(readSvgLayoutPublication())")?,
            initial,
            "single-source and Range reads must use the same old publication"
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 1
        );
        assert_eq!(
            page_vm
                .vm()
                .layout_snapshot_cache_observability_for_test()
                .2,
            publications
        );
        assert_eq!(
            crate::layout_renderer::svg_parse_count(),
            parsed + 1,
            "1000 dirty SVG sources must not be reparsed by geometry reads"
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 160, 1.0))?
            .expect("fresh screenshot");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        for (x, y, color) in [(250, 10, [255, 0, 0, 255]), (335, 55, [0, 255, 0, 255])] {
            let offset = (y * raster.width as usize + x) * 4;
            assert_eq!(&raster.rgba[offset..offset + 4], color);
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &page_vm
                    .vm_mut()
                    .eval("JSON.stringify(readSvgLayoutPublication())")?
            )?,
            json!({
                "htmlBefore":300,"svg":[300,40,200,100],"object":[330,50,100,10],"shapeCount":1,
                "range":[310,50,120,10],"rangeCount":1000,"htmlAfter":300,
            })
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 2
        );
        assert_eq!(
            page_vm
                .vm()
                .layout_snapshot_cache_observability_for_test()
                .2,
            publications + 1
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG single and batch queries must only consume the frozen publication");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_reveal_waits_for_visual_publication() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-hidden-chart.html")?,
        );
        page_vm.vm_mut().eval(r#"
            document.head.innerHTML = '<style>html,body{margin:0}#container{width:250px}.hidden{display:none}svg{font:12px "DejaVu Sans"}</style>';
            document.body.innerHTML = '<div id="container"><svg class="hidden"><g><text>Code review</text></g></svg></div>';
            'installed'
        "#)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        assert_eq!(page_vm.vm_mut().eval("document.getElementById('container').clientWidth")?, "250");
        let passes = page_vm.vm().layout_pass_observability_for_test().1;
        let hidden = page_vm.vm_mut().eval(r#"(() => {
            const svg=document.querySelector('svg'), text=svg.querySelector('text');
            svg.classList.remove('hidden');
            text.setAttribute('x','40'); text.setAttribute('y','50');
            const r=text.getBoundingClientRect();
            return JSON.stringify([r.x,r.y,r.width,r.height]);
        })()"#)?;
        assert_eq!(hidden,"[0,0,0,0]","revealing SVG must not refresh old geometry");
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1,passes);
        page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(400,150,1.0))?;
        let measured = page_vm.vm_mut().eval(r#"JSON.stringify((r=>[r.x,r.y,r.width,r.height])(document.querySelector('text').getBoundingClientRect()))"#)?;
        let measured: Vec<f64> = serde_json::from_str(&measured)?;
        assert!(measured[0] > 39.0 && measured[1] > 30.0,"{measured:?}");
        assert!(measured[2] > 50.0 && measured[3] > 8.0,"{measured:?}");
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1,passes+1);
        Ok::<_, anyhow::Error>(())
    }).await.expect("revealed SVG must become measurable only after a new publication");
}

#[cfg(feature = "devtools")]
#[tokio::test(flavor = "current_thread")]
async fn inline_svg_screencast_publishes_changed_geometry_and_skips_clean_polls() {
    use crate::runtime::{
        RendererCaptureScreencastFrameReply as Reply, RendererCaptureScreencastFrameRequest,
        RendererScreenshotFormat,
    };
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-screencast-publication.html")?,
        );
        page_vm.vm_mut().eval(&format!(
            "{SVG_PUBLICATION_FIXTURE}\ninstallSvgLayoutPublication()"
        ))?;
        let request = |known_visual_state| RendererCaptureScreencastFrameRequest {
            base_background_color: [255; 4],
            format: RendererScreenshotFormat::Png,
            quality: 100,
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
            known_visual_state,
            paint_session: Default::default(),
        };
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        let parsed = crate::layout_renderer::svg_parse_count();
        let Reply::Captured(first) = page_vm.capture_screencast_frame(request(None))? else {
            panic!("first screencast must capture");
        };
        let initial = page_vm
            .vm_mut()
            .eval("JSON.stringify(readSvgLayoutPublication())")?;
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 1
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        assert_eq!(
            page_vm.capture_screencast_frame(request(Some(first.visual_state.clone())))?,
            Reply::Unchanged
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 1
        );

        page_vm.vm_mut().eval("mutateSvgLayoutPublication()")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(readSvgLayoutPublication())")?,
            initial
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 1
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 1);
        let Reply::Captured(second) =
            page_vm.capture_screencast_frame(request(Some(first.visual_state)))?
        else {
            panic!("changed SVG and HTML must capture one new frame");
        };
        let measured: serde_json::Value = serde_json::from_str(
            &page_vm
                .vm_mut()
                .eval("JSON.stringify(readSvgLayoutPublication())")?,
        )?;
        assert_eq!(measured["htmlBefore"], 300);
        assert_eq!(measured["htmlAfter"], 300);
        assert_eq!(measured["object"], json!([330, 50, 100, 10]));
        assert_eq!(measured["range"], json!([310, 50, 120, 10]));
        let raster = moli_image::decode_png(&second.image.bytes)?;
        let offset = (55 * raster.width as usize + 335) * 4;
        assert_eq!(&raster.rgba[offset..offset + 4], [0, 255, 0, 255]);
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 2
        );
        assert_eq!(crate::layout_renderer::svg_parse_count(), parsed + 2);
        assert_eq!(
            page_vm.capture_screencast_frame(request(Some(second.visual_state)))?,
            Reply::Unchanged
        );
        assert_eq!(
            page_vm.vm().layout_pass_observability_for_test().1,
            before + 2
        );
        assert_eq!(
            crate::layout_renderer::svg_parse_count(),
            parsed + 2,
            "clean screencast polls must not rebuild or reparse the scene"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("screencast must own SVG refreshes just as it owns HTML refreshes");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_geometry_keeps_clipped_and_flattened_source_elements() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-source-geometry.html")?,
        );
        page_vm.vm_mut().eval(r##"
            document.head.innerHTML = '<style>html,body{margin:0}svg{display:block}</style>';
            document.body.innerHTML = `<svg width="300" height="150">
              <defs><clipPath id="empty"/></defs>
              <g id="group"><rect id="clipped" x="10" y="20" width="30" height="40" clip-path="url(#empty)"/></g>
              <svg id="nested" x="10" y="20" width="100" height="80" overflow="visible">
                <rect id="same" x="2" y="3" width="30" height="40"/>
                <rect id="same" x="40" y="3" width="10" height="40"/>
              </svg>
            </svg>`;
            'installed'
        "##)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(300, 150, 1.0))?;
        assert_eq!(page_vm.vm_mut().eval(r#"JSON.stringify(['#clipped','#group','#nested'].flatMap(s=>[...document.querySelectorAll(s)]).concat([...document.querySelectorAll('#same')]).map(e=>{const r=e.getBoundingClientRect();return [r.x,r.y,r.width,r.height]}))"#)?,
            "[[10,20,30,40],[10,20,30,40],[12,23,48,40],[12,23,30,40],[50,23,10,40]]");
        Ok::<_, anyhow::Error>(())
    }).await.expect("object geometry must not depend on retained paint nodes or unique author IDs");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_client_rects_include_graphics_text_and_viewport_transforms() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-client-rects.html")?,
        );
        page_vm.vm_mut().eval(&format!(
            "document.documentElement.innerHTML = {}; 'installed'",
            serde_json::to_string(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/inline-svg-client-rects.html")))?,
        ))?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?;
        let result = page_vm.vm_mut().eval(r#"JSON.stringify(Object.fromEntries(
            ['scene','shape','axis','hidden','transparent','nested','nested-shape','clipped','label','group'].map(id => {
                const e=document.getElementById(id), r=e.getBoundingClientRect();
                return [id, {rect:[r.x,r.y,r.width,r.height],count:e.getClientRects().length}];
            })))"#)?;
        let values: serde_json::Value = serde_json::from_str(&result)?;
        // Chromium 145.0.7632.116, with the same fixture and installed font.
        for (id, expected) in [
            ("scene", [40.0, 30.0, 300.0, 200.0]),
            ("shape", [70.0, 84.0, 60.0, 30.0]),
            ("axis", [70.0, 144.0, 160.0, 0.0]),
            ("transparent", [240.0, 50.0, 40.0, 20.0]),
            ("nested", [188.0, 122.0, 40.0, 20.0]),
            ("nested-shape", [188.0, 122.0, 40.0, 20.0]),
            ("clipped", [60.0, 40.0, 40.0, 20.0]),
        ] {
            for (actual, expected) in values[id]["rect"].as_array().unwrap().iter().zip(expected) {
                assert!((actual.as_f64().unwrap() - expected).abs() < 0.01, "{id}: {values}");
            }
            assert_eq!(values[id]["count"], 1);
        }
        assert_eq!(values["hidden"]["count"], 0, "{values}");
        assert_eq!(values["hidden"]["rect"], json!([0,0,0,0]));
        assert!(values["label"]["rect"][2].as_f64().unwrap() > 100.0, "text must be shaped: {values}");
        assert!(values["label"]["rect"][3].as_f64().unwrap() > 30.0, "text must be shaped: {values}");
        let range: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval(r#"
            const text = document.getElementById('label').firstChild;
            const range = document.createRange();
            range.setStart(text, 0);
            range.setEnd(text, text.length);
            const {x, y, width, height} = range.getBoundingClientRect();
            JSON.stringify([x, y, width, height]);
        "#)?)?;
        assert_eq!(range, values["label"]["rect"], "whole SVG text selection");
        assert!(values["group"]["rect"][3].as_f64().unwrap() > 120.0, "group must include text: {values}");

        assert_eq!(page_vm.vm_mut().eval("document.getElementById('shape').getAttribute('fill')")?, "none", "geometry projection must not change authored paint");
        page_vm.vm_mut().eval("document.getElementById('scene').style.cssText += ';transform:translate(11px,13px) scale(2);transform-origin:0 0'; 'changed'")?;
        page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify((r=>[r.x,r.y,r.width,r.height])(document.getElementById('shape').getBoundingClientRect()))")?, "[111,151,120,60]");
        page_vm.vm_mut().eval("document.getElementById('scene').style.transform = 'matrix(2,0,0,2,11,13)'; 'changed'")?;
        page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify((r=>[r.x,r.y,r.width,r.height])(document.getElementById('shape').getBoundingClientRect()))")?, "[111,151,120,60]", "outer CSS transforms must be applied once");
        Ok::<_, anyhow::Error>(())
    }).await.expect("SVG client rects must include object geometry");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_chart_can_size_itself_from_group_and_text_client_rects() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-measured-chart.html")?,
        );
        page_vm.vm_mut().eval(r##"
            document.head.innerHTML = `<style>
              html,body {margin:0;background:white}
              svg {display:block;font:12px "DejaVu Sans"}
              .axis {stroke:rgb(17,99,41);stroke-width:2px}
              .label {fill:rgb(89,99,110);font-size:12px}
              .hidden {display:none}
            </style>`;
            document.body.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg"><g>
              <line class="axis" x1="10" y1="125" x2="240" y2="125"/>
              <line class="axis" x1="125" y1="10" x2="125" y2="240"/>
              <path fill="#40c463" d="M125 125 L125 154 L14 125 Z"/>
              <ellipse class="hidden" cx="500" cy="500" rx="5" ry="5"/>
              <text class="label" x="125" y="0" text-anchor="middle">Code review</text>
              <text class="label" x="250" y="125">Issues</text>
              <text class="label" x="125" y="255" text-anchor="middle">Pull requests</text>
              <text class="label" x="0" y="125" text-anchor="end">Commits</text>
            </g></svg>`;
            'installed'
        "##)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let result = page_vm.vm_mut().eval(r#"(() => {
            const svg=document.querySelector('svg'), g=svg.querySelector('g');
            const rect=g.getBoundingClientRect(), outer=svg.getBoundingClientRect();
            g.setAttribute('transform', `translate(${-rect.left+outer.left} ${-rect.top+outer.top})`);
            svg.setAttribute('width', Math.ceil(rect.width));
            svg.setAttribute('height', Math.ceil(rect.height));
            return JSON.stringify([rect.width,rect.height]);
        })()"#)?;
        let size: Vec<f64> = serde_json::from_str(&result)?;
        assert!(size[0] > 280.0 && size[0] < 400.0, "chart width: {result}");
        assert!(size[1] > 250.0 && size[1] < 300.0, "chart height: {result}");
        let snapshot = page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?.unwrap();
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        // Text bounds give the group a fractional translation. Measure the
        // antialiased color coverage over white, without requiring axes and
        // glyphs to land on whole pixels in every font rasterizer.
        let color_coverage = |color: [u8; 3]| {
            raster.rgba.as_chunks::<4>().0.iter().map(|pixel| {
                let alpha = f64::from(255 - pixel[0]) / f64::from(255 - color[0]);
                if (1..3).all(|channel| {
                    (f64::from(pixel[channel]) - (255.0 - alpha * f64::from(255 - color[channel]))).abs() < 2.0
                }) { alpha } else { 0.0 }
            }).sum::<f64>()
        };
        assert!(color_coverage([64,196,99]) > 500.0, "measured SVG must paint the area");
        assert!(color_coverage([17,99,41]) > 400.0, "external CSS must paint both axes: {}", color_coverage([17,99,41]));
        assert!(color_coverage([89,99,110]) > 50.0, "external CSS must paint the labels");
        assert_eq!(page_vm.vm_mut().eval("document.querySelector('svg g').hasAttribute('id')")?, "false", "query identifiers must not leak into the live DOM");

        // Referenced shapes outside <defs> must still inherit from <use>,
        // rather than freezing the source element's original inherited fill.
        page_vm.vm_mut().eval(r##"document.body.innerHTML = '<svg width="80" height="40" style="fill:blue"><rect id="shared" width="20" height="20"/><use href="#shared" x="30" style="fill:red"/></svg>'; 'installed'"##)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(80,40,1.0))?.unwrap();
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: usize, y: usize| &raster.rgba[(y * raster.width as usize + x) * 4..][..4];
        assert_eq!(pixel(10,10), [0,0,255,255]);
        assert_eq!(pixel(40,10), [255,0,0,255]);
        Ok::<_, anyhow::Error>(())
    }).await.expect("SVG chart must measure and paint its group");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_sizes_viewbox_only_svg_from_available_inline_space() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-auto-size.html")?,
        );
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/inline-svg-auto-size.html"
        ));
        page_vm.vm_mut().eval(&format!(
            "document.documentElement.innerHTML = {}; 'installed'",
            serde_json::to_string(fixture)?,
        ))?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("inline SVG sizing fixture must retain a layout root");
        let geometry = page_vm.vm_mut().eval(
            "JSON.stringify(Object.fromEntries([...document.querySelectorAll('svg')].map(element => { const rect = element.getBoundingClientRect(); return [element.id, [rect.width, rect.height]]; })))",
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        // Measured in Chromium 145. A viewBox supplies a ratio, not a fixed
        // 150px natural size; definite author sizes may still overflow.
        for (id, expected) in [
            ("plus", [24, 24]),
            ("padded-parent", [24, 24]),
            ("wide", [24, 12]),
            ("no-ratio", [300, 150]),
            ("width", [80, 80]),
            ("css-height", [32, 32]),
            ("margin", [20, 13]),
            ("css-ratio", [24, 12]),
            ("percent", [24, 24]),
            ("min", [40, 40]),
            ("max", [20, 20]),
            ("absolute", [64, 64]),
            ("absolute-left", [54, 54]),
            ("absolute-right", [54, 54]),
            ("absolute-insets", [44, 44]),
            ("absolute-margin", [48, 48]),
            ("shrink", [0, 0]),
            ("shrink-percent", [300, 300]),
            ("block-margin", [80, 80]),
            ("block-padding-margin", [80, 43]),
            ("block-percent-margin", [80, 80]),
            ("block-negative-margin", [120, 120]),
            ("block-auto-margin", [100, 100]),
            ("block-visible-margin", [80, 80]),
            ("float-margin", [80, 80]),
            ("block-float-margin", [80, 80]),
            ("inline-float-margin", [80, 80]),
            ("flex-margin", [80, 80]),
            ("grid-margin", [80, 80]),
        ] {
            assert_eq!(
                geometry[id],
                serde_json::json!(expected),
                "Chromium-calibrated SVG geometry mismatch for {id}: {geometry}",
            );
        }
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(pixel(24, 24), [34, 34, 34, 255]);
        assert_eq!(pixel(14, 24), [238, 238, 238, 255]);
        assert_eq!(pixel(30, 30), [238, 238, 238, 255]);
        assert_eq!(pixel(24, 55), [255, 255, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG automatic sizing fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_projects_external_svg_root_paint_like_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-computed-paint.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
#run{position:absolute;left:0;top:0;width:80px;height:40px;margin:0;padding:0;border:0;background:rgb(174,67,28);color:white}
.icon{position:absolute;left:30px;top:10px;display:block;width:20px;height:20px;fill:currentcolor}
#stroke{position:absolute;left:100px;top:10px;display:block;width:20px;height:20px;color:rgb(0,128,0);fill:none;stroke:currentcolor;stroke-width:4px;stroke-linecap:butt}
#presentation{position:absolute;left:140px;top:10px;display:block;color:rgb(51,51,51)}
</style>`;
document.body.innerHTML = `
<button id=run><svg id=run-icon class=icon fill="none" viewBox="0 0 20 20"><path d="M2 2v16l16-8z"></path></svg></button>
<svg id=stroke stroke="red" viewBox="0 0 20 20"><path d="M2 10h16"></path></svg>
<svg id=presentation width="20" height="20" fill="none" viewBox="0 0 20 20">
  <rect id=presentation-shape x="2" y="2" width="16" height="16" stroke="currentColor" stroke-width="2"></rect>
</svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        assert_eq!(
            page_vm.vm_mut().eval(
                "[getComputedStyle(document.getElementById('run-icon')).fill,getComputedStyle(document.getElementById('stroke')).stroke,getComputedStyle(document.getElementById('presentation')).fill,getComputedStyle(document.getElementById('presentation-shape')).stroke,getComputedStyle(document.getElementById('presentation-shape')).strokeWidth].join('|')",
            )?,
            "rgb(255, 255, 255)|rgb(0, 128, 0)|none|rgb(51, 51, 51)|2px",
            "presentation attributes must enter Stylo below author CSS, retain SVG unitless lengths, and inherit currentColor normally",
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(170, 50, 1.0))?
            .expect("inline SVG computed-paint fixture must retain a layout root");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };

        assert_eq!(pixel(40, 20), [255, 255, 255, 255]);
        assert_eq!(pixel(15, 20), [174, 67, 28, 255]);
        assert_eq!(pixel(110, 20), [0, 128, 0, 255]);
        assert_eq!(pixel(90, 20), [255, 255, 255, 255]);
        assert_eq!(pixel(142, 20), [51, 51, 51, 255]);
        assert_eq!(
            pixel(150, 20),
            [255, 255, 255, 255],
            "fill=none must not be replaced by SVG's initial black fill"
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('presentation').setAttribute('fill','blue')")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("getComputedStyle(document.getElementById('presentation')).fill")?,
            "rgb(0, 0, 255)",
            "changing a presentation attribute must invalidate its computed style",
        );
        let mutated = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(170, 50, 1.0))?
            .expect("mutated SVG presentation fixture must retain a layout root");
        let mutated_raster = moli_paint::raster_snapshot(&mutated)?;
        let center = ((20 * mutated_raster.width + 150) * 4) as usize;
        assert_eq!(
            &mutated_raster.rgba[center..center + 4],
            [0, 0, 255, 255],
            "the invalidated computed fill must reach the inline SVG paint bridge"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG computed-paint fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_clips_inline_svg_to_its_viewport_unless_overflow_is_visible() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-overflow.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
svg{position:absolute;top:10px;display:block}
</style>`;
document.body.innerHTML = `
<svg id=clipped width="20" height="20" style="left:10px"><rect width="40" height="40" fill="red"></rect></svg>
<svg id=visible width="20" height="20" style="left:60px;overflow:visible"><rect width="40" height="40" fill="blue"></rect></svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(110, 60, 1.0))?
            .expect("inline SVG overflow fixture must retain a layout root");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };

        assert_eq!(pixel(20, 20), [255, 0, 0, 255]);
        assert_eq!(
            pixel(40, 40),
            [255, 255, 255, 255],
            "the UA sheet must clip an embedded svg to its viewport"
        );
        assert_eq!(pixel(70, 20), [0, 0, 255, 255]);
        assert_eq!(
            pixel(90, 40),
            [0, 0, 255, 255],
            "author overflow:visible must still paint outside the viewport"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG overflow fixture should run");
}

// Inline SVG text is painted from a `usvg::Tree` reparsed out of the live
// subtree, so text only survives when the root carries the font that applies
// to it. Two otherwise identical badges draw the same string: one inherits its
// `font-family`/`font-weight` from document CSS, the other authors the very
// same values inside the SVG. Both must produce the same glyph pixels.
//
// This isolates the consumer-visible behavior instead of raw pixel presence:
// once `moli-image` resolves an installed default family, a badge that *lost*
// its inherited font still paints glyphs, so a "some bright pixels exist"
// check would pass while the inherited font was silently replaced. Comparing
// the two badges fails as soon as the serialized root stops carrying the
// computed font, and it also fails if a quoted family breaks the serialized
// `style` attribute (the escaped text is what the parse actually consumes).
// A host that has no installed font at all leaves both badges blank, which
// the glyph-presence assertion reports.
#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_inherited_inline_svg_font_equal_to_an_authored_one() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-inherited-font.html")?,
        );
        // `"DejaVu Sans Mono", monospace` is quoted, so the computed value
        // serializes with double quotes and must survive the `style` attribute.
        page_vm.vm_mut().eval(
            r##"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
svg{position:absolute;top:0;width:90px;height:40px}
#inherited{left:0;font-family:"DejaVu Sans Mono",monospace;font-size:18px;font-weight:bold}
#authored{left:110px;font-size:18px}
</style>`;
document.body.innerHTML = `
<svg id=inherited width=90 height=40 viewBox="0 0 90 40"><rect width=90 height=40 rx=8 fill="#087a65"/><text x=8 y=26 fill="white">SVG OK</text></svg>
<svg id=authored width=90 height=40 viewBox="0 0 90 40"><rect width=90 height=40 rx=8 fill="#087a65"/><text x=8 y=26 fill="white" font-family='"DejaVu Sans Mono", monospace' font-weight=bold font-size=18>SVG OK</text></svg>`;
'installed'
"##,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        // Precondition: the inherited side authors no font at all, so the only
        // way it can match the second badge is by inheriting the CSS font.
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.getElementById('inherited').hasAttribute('font-family'))",
            )?,
            "false",
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(210, 50, 1.0))?
            .expect("inline SVG font fixture must retain a layout root");
        assert_eq!(
            snapshot.svg_images.len(),
            2,
            "both inline SVGs must reach paint as vector resources",
        );
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };
        // Sample only the interior of each teal rectangle: the rounded corners
        // and the page background stay outside the ROI, so every lit pixel is
        // glyph coverage rather than backdrop.
        let glyph_mask = |left: u32| -> Vec<bool> {
            (8..32)
                .flat_map(|y| (18..72).map(move |x| (x + left, y)))
                .map(|(x, y)| {
                    let [red, green, blue, alpha] = pixel(x, y);
                    alpha == 255 && red > 160 && green > 160 && blue > 160
                })
                .collect()
        };
        for left in [0, 110] {
            // (45, 37) sits below the 26px baseline and inside the rounded
            // rectangle, so it must hold the untouched badge fill: every lit
            // pixel in the ROI above is therefore glyph coverage.
            assert_eq!(pixel(left + 45, 37), [8, 122, 101, 255], "badge fill");
        }
        let inherited = glyph_mask(0);
        let authored = glyph_mask(110);
        let lit = |mask: &[bool]| mask.iter().filter(|painted| **painted).count();
        let (inherited_lit, authored_lit) = (lit(&inherited), lit(&authored));
        assert!(
            inherited_lit > 20 && authored_lit > 20,
            "both badges must paint glyphs; inherited={inherited_lit}, authored={authored_lit}",
        );
        let coverage_gap = inherited_lit.abs_diff(authored_lit);
        assert!(
            coverage_gap <= authored_lit / 50 + 2,
            "an inherited font must paint the same glyph coverage as the same \
             font authored inside the SVG; inherited={inherited_lit}, \
             authored={authored_lit} means the serialized root lost the \
             computed font or the quoted family broke its style attribute",
        );
        // Glyph outlines, not just their count, must match: the same font at the
        // same size can only land on the same pixels up to antialiasing
        // threshold flips.
        let differing = inherited
            .iter()
            .zip(&authored)
            .filter(|(left, right)| left != right)
            .count();
        assert!(
            differing <= authored_lit / 20 + 2,
            "inherited and authored glyphs must occupy the same pixels, {differing} differ",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG inherited-font fixture should run");
}

// The SVG decoder caches system fonts process-wide. Give this Linux regression
// a child-only Fontconfig containing checked-in, distinguishable faces;
// neither host fonts nor other parallel tests can change the selected outlines.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
async fn screenshot_inline_svg_variant_faces_isolate_system_fonts() -> anyhow::Result<()> {
    const CHILD: &str = "MOLI_SVG_VARIANT_RASTER_CHILD";
    const COMPLETED: &str = "svg-variant-raster-child: completed";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let directory = tempfile::tempdir()?;
        let fonts = directory.path().join("fonts");
        std::fs::create_dir(&fonts)?;
        let fixtures: [(&str, &[u8]); 6] = [
            (
                "regular.ttf",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-svg-regular.ttf"
                )),
            ),
            (
                "italic.ttf",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-svg-italic.ttf"
                )),
            ),
            (
                "condensed.ttf",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-svg-condensed.ttf"
                )),
            ),
            (
                "semibold.ttf",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-svg-semibold.ttf"
                )),
            ),
            (
                "bold.ttf",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-svg-bold.ttf"
                )),
            ),
            (
                "fallback.ttf",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-ahem.ttf"
                )),
            ),
        ];
        for (name, bytes) in fixtures {
            std::fs::write(fonts.join(name), bytes)?;
        }
        let xml_path = |path: &std::path::Path| {
            path.to_string_lossy()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let aliases = ["serif", "sans-serif", "monospace"]
            .map(|generic| {
                format!(
                    "<alias><family>{generic}</family><prefer>\
                <family>Moli Ahem</family></prefer></alias>"
                )
            })
            .join("");
        let config = directory.path().join("fonts.conf");
        // A distinct fallback family makes a missing root font-family
        // projection visible even when every requested SVG face is installed.
        std::fs::write(
            &config,
            format!(
                "<fontconfig><dir>{}</dir><cachedir>{}</cachedir>{aliases}</fontconfig>",
                xml_path(&fonts),
                xml_path(directory.path()),
            ),
        )?;
        let module = module_path!().split_once("::").expect("test module").1;
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                &format!("{module}::screenshot_inline_svg_variant_faces_isolate_system_fonts"),
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("FONTCONFIG_FILE", &config)
            .env_remove("FONTCONFIG_PATH")
            .env_remove("FONTCONFIG_SYSROOT")
            .env_remove("XDG_CONFIG_HOME")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "variant raster child failed:\n{stdout}\n{stderr}"
        );
        // An incorrect --exact filter can exit successfully without a test.
        assert!(
            stdout.contains(COMPLETED),
            "variant assertions did not run:\n{stdout}"
        );
        for line in stdout
            .lines()
            .filter(|line| line.starts_with("svg-variant "))
        {
            println!("{line}");
        }
        return Ok(());
    }

    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-font-variants.html")?,
        );
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/inline-svg-font-variants.html",
        ));
        page_vm.vm_mut().eval(&format!(
            "document.documentElement.innerHTML = {}; 'installed'",
            serde_json::to_string(fixture)?,
        ))?;
        page_vm.vm_mut().sync_live_document_style_sources();
        assert_eq!(
            page_vm.vm_mut().eval(
                "['lighter-inherited','bolder-inherited','bold-inherited'].map(id => { const svg = document.getElementById(id); return [getComputedStyle(svg).fontWeight, getComputedStyle(svg.querySelector('text')).fontWeight].join('/'); }).join('|')",
            )?,
            "700/400|500/700|700/700",
            "the isolated SVG parse must agree with the live document's computed weights",
        );
        let snapshot = page_vm.vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(300, 350, 1.0))?
            .expect("variant layout root");
        assert_eq!(snapshot.svg_images.len(), 15, "all font variants must reach vector paint");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };
        let ink = |x, y| {
            let [r, g, b, a] = pixel(x, y);
            a == 255 && r > 200 && g > 200 && b > 200
        };
        let mask = |left: u32, top: u32| -> Vec<bool> {
            (2..58).flat_map(|y| (2..88).map(move |x| (left + x, top + y)))
                .map(|(x, y)| ink(x, y)).collect()
        };
        let lit = |mask: &[bool]| mask.iter().filter(|painted| **painted).count();
        const BADGE: [u8; 4] = [15, 118, 110, 255];
        for (variant, top, empty, painted) in [
            ("italic", 0, (12, 18), (30, 30)),
            ("condensed", 70, (30, 30), (12, 18)),
        ] {
            let inherited = mask(0, top);
            let authored = mask(100, top);
            let normal = mask(200, top);
            let normal_ink = lit(&normal);
            assert!((900..1_300).contains(&normal_ink), "{variant}: regular outline missing");
            assert!(ink(200 + empty.0, top + empty.1)
                && ink(200 + painted.0, top + painted.1), "{variant}: normal control");
            for left in [0, 100, 200] {
                assert_eq!(pixel(left + 45, top + 57), BADGE, "{variant}: badge background");
            }
            // A triangle and a half-width rectangle prove real face selection:
            // normal fallback (including synthetic normal-face scaling) cannot
            // satisfy both the outline probes and the 35%..65% coverage bound.
            for (label, left, glyph) in [
                ("inherited", 0, &inherited), ("authored", 100, &authored),
            ] {
                assert_eq!(pixel(left + empty.0, top + empty.1), BADGE,
                    "{variant}-{label}: wrong variant face; normal paints this pixel");
                assert!(ink(left + painted.0, top + painted.1),
                    "{variant}-{label}: requested outline missing");
                let coverage = lit(glyph);
                assert!(coverage * 20 >= normal_ink * 7 && coverage * 20 <= normal_ink * 13,
                    "{variant}-{label}: variant {coverage} must differ from normal {normal_ink}");
            }
            let differing = inherited.iter().zip(&authored).filter(|(a, b)| a != b).count();
            assert!(differing <= normal_ink / 50 + 2,
                "{variant}: inherited/authored glyphs differ in {differing} pixels");
            println!("svg-variant {variant}: normal={normal_ink}, inherited={}, authored={}, mismatch={differing}",
                lit(&inherited), lit(&authored));
        }
        for (variant, top, expected_weight) in [
            ("lighter", 140, 400),
            ("bolder", 210, 700),
            ("bold", 280, 700),
        ] {
            let inherited = mask(0, top);
            let authored = mask(100, top);
            let control = mask(200, top);
            let authored_ink = lit(&authored);
            assert!(authored_ink > 200 && lit(&control) > 200,
                "{variant}: reference and control must paint glyphs");
            let control_difference = authored.iter().zip(&control).filter(|(a, b)| a != b).count();
            assert!(control_difference > 100,
                "{variant}: distinct weights must select distinct outlines, {control_difference} pixels differ");
            for left in [0, 100, 200] {
                assert_eq!(pixel(left + 45, top + 57), BADGE, "{variant}: badge background");
            }
            for left in [0, 100] {
                assert!(ink(left + 12, top + 18), "{variant}: requested outline missing");
                assert_eq!(ink(left + 30, top + 18), expected_weight == 400,
                    "{variant}: expected the {expected_weight} outline");
            }
            let differing = inherited.iter().zip(&authored).filter(|(a, b)| a != b).count();
            assert!(differing <= authored_ink / 50 + 2,
                "{variant}: inherited/authored glyphs differ in {differing} pixels");
            println!("svg-variant {variant}: inherited={}, authored={authored_ink}, control={}, mismatch={differing}",
                lit(&inherited), lit(&control));
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("isolated SVG variant raster");
    println!("{COMPLETED}");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_template_contents_preserve_neighboring_graphics_identity() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-template-mapping.html")?,
        );
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/inline-svg-scene-inputs.js"
        ));
        let metadata = page_vm.vm_mut().eval(&format!(
            "{fixture}\nJSON.stringify(installSvgSceneInputs('template'))"
        ))?;
        let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(
                metadata["width"].as_u64().unwrap() as u32,
                metadata["height"].as_u64().unwrap() as u32,
                1.0,
            ))?
            .unwrap();
        assert_eq!(
            snapshot.svg_images.len(),
            2,
            "inert HTML contents must not discard the SVG scene"
        );
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        for case in metadata["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let top = case["top"].as_u64().unwrap();
            let observed = page_vm.vm_mut().eval(&format!(
                "JSON.stringify(svgSceneInputRects({}))",
                serde_json::to_string(name)?
            ))?;
            let observed: serde_json::Value = serde_json::from_str(&observed)?;
            assert_eq!(observed["children"], 0);
            assert!(observed["content"].as_u64().unwrap() > 0);
            assert_eq!(
                observed["rects"],
                json!([[0, top, 20, 20], [40, top, 20, 20]]),
                "{name}"
            );
            for (x, color) in [(10, [255, 0, 0, 255]), (50, [0, 0, 255, 255])] {
                let offset = (((top as usize + 10) * raster.width as usize) + x) * 4;
                assert_eq!(
                    &raster.rgba[offset..offset + 4],
                    color,
                    "{name}: neighboring SVG styles must keep their identities"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG element mapping must follow serializer template traversal");
}
