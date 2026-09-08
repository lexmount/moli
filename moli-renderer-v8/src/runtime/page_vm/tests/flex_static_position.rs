use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_flex_static_margin_boxes_across_containing_blocks() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-static-position-margins.html")?,
        );
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 1280,
            inner_height: 6000,
            outer_width: 1280,
            outer_height: 6000,
            device_pixel_ratio: 1.0,
            screen_width: 1280,
            screen_height: 6000,
            screen_avail_width: 1280,
            screen_avail_height: 6000,
        }))?;
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let fixture = include_str!("../../../../tests/fixtures/flex-static-position-margins.html");
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(fixture)?
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        // Geometry reads intentionally use the last snapshot. Demand the same
        // complete paint that the release Moli/Chromium comparison captures.
        let crate::runtime::RendererCaptureScreenshotReply::Captured(screenshot) = page
            .capture_screenshot(crate::runtime::RendererCaptureScreenshotRequest::viewport_png())?
        else {
            panic!("explicit screenshot demand must produce pixels");
        };
        let image = moli_image::decode_png(&screenshot.bytes)?;
        let checks: serde_json::Value = serde_json::from_str(
            &page
                .vm_mut()
                .eval("JSON.stringify(collectFlexStaticMarginChecks())")?,
        )?;
        let checks = checks.as_array().expect("flex static margin checks");
        assert_eq!(checks.len(), 117);
        for check in checks {
            assert_eq!(check["actual"], check["expected"], "{check}");
            let x = check["pixel"][0].as_f64().expect("pixel x") as usize;
            let y = check["pixel"][1].as_f64().expect("pixel y") as usize;
            let offset = (y * image.width as usize + x) * 4;
            assert_eq!(
                &image.rgba[offset..offset + 4],
                &[31, 127, 63, 255],
                "{} paint",
                check["id"]
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex static-position margin fixture should run");
}
