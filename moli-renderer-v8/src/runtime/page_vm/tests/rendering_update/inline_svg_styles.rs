use super::*;

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_paints_descendant_document_styles_like_presentation_attributes() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-text-styles.html")?,
        );
        page.vm_mut().eval(include_str!(
            "../../../../../tests/fixtures/svg-text-style-setup.js"
        ))?;
        page.vm_mut().sync_live_document_style_sources();
        let viewport = moli_layout::LayoutViewport::new(300, 100, 1.0);
        let authored = page
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("authored SVG");
        assert_eq!(authored.svg_images.len(), 1);
        let text_bounds = authored.svg_images[0]
            .image
            .tree()
            .node_by_id("label")
            .expect("actual SVG text must be shaped, not omitted")
            .bounding_box();
        assert!(text_bounds.width() > 0.0 && text_bounds.height() > 0.0);
        let expected = moli_paint::raster_snapshot(&authored)?;

        page.vm_mut().eval(r#"
            enableSvgDocumentStyles();
            globalThis.beforeSvgCapture = document.body.innerHTML;
            globalThis.svgObserver = new MutationObserver(() => {});
            svgObserver.observe(document.body, {
                subtree: true, attributes: true, childList: true
            });
        "#)?;
        let projected = page
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("stylesheet SVG");
        assert_eq!(projected.svg_images.len(), 1);
        let projected_bounds = projected.svg_images[0]
            .image
            .tree()
            .node_by_id("label")
            .expect("stylesheet text must be shaped")
            .bounding_box();
        assert_eq!(
            projected_bounds, text_bounds,
            "CSS fonts, sizes, spacing and text-anchor must reach the same text layout as attributes"
        );
        let actual = moli_paint::raster_snapshot(&projected)?;
        assert_eq!((actual.width, actual.height), (expected.width, expected.height));
        let first_difference = actual
            .rgba
            .iter()
            .zip(&expected.rgba)
            .position(|(actual, expected)| actual != expected);
        assert!(
            first_difference.is_none(),
            "CSS text/shape fill and stroke must reach actual paint; first differing byte={first_difference:?}"
        );
        assert_eq!(
            page.vm_mut().eval(
                "document.body.innerHTML===beforeSvgCapture && svgObserver.takeRecords().length===0"
            )?,
            "true",
            "derived style projection must not mutate web-visible markup"
        );
        assert!(!std::sync::Arc::ptr_eq(
            &authored.svg_images[0].image,
            &projected.svg_images[0].image,
        ));
        assert!(
            moli_paint::raster_snapshot(&authored)?.rgba == expected.rgba,
            "an older paint snapshot remains immutable"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG document styles fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_svg_descendant_styles_use_the_capture_viewport() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-text-viewport.html")?,
        );
        page.vm_mut().eval(r#"
          document.head.innerHTML='<style>text{font:12px monospace}@media(min-width:300px){text{font-size:24px}}</style>';
          document.body.innerHTML='<svg width="160" height="80"><text id="label" x="2" y="40">iiWW</text></svg>';
        "#)?;
        page.vm_mut().sync_live_document_style_sources();
        let small = page
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::LayoutViewport::new(200, 100, 1.0))?
            .expect("small SVG");
        let large = page
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::LayoutViewport::new(400, 100, 1.0))?
            .expect("large SVG");
        let small = small.svg_images[0]
            .image
            .tree()
            .node_by_id("label")
            .expect("small text")
            .bounding_box();
        let large = large.svg_images[0]
            .image
            .tree()
            .node_by_id("label")
            .expect("large text")
            .bounding_box();
        assert!(small.width() > 0.0 && small.height() > 0.0);
        assert!(
            (large.width() - small.width() * 2.0).abs() < 0.001,
            "SVG text must use the current capture media environment: small={small:?}, large={large:?}"
        );
        assert!((large.height() - small.height() * 2.0).abs() < 0.001);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG viewport style fixture");
}
