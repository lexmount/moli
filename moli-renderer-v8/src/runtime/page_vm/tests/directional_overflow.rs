use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_directional_scroll_ranges_and_normal_flow_bounds() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/directional-overflow.html")?,
        );
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 1280,
            inner_height: 3600,
            outer_width: 1280,
            outer_height: 3600,
            device_pixel_ratio: 1.0,
            screen_width: 1280,
            screen_height: 3600,
            screen_avail_width: 1280,
            screen_avail_height: 3600,
        }))?;
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(include_str!(
                "../../../../tests/fixtures/directional-scroll-overflow.html"
            ))?,
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        // Every observation follows an explicit capture. Synchronous geometry
        // remains intentionally stale between layout demands, including after
        // the final display:none mutation removes the overflow contribution.
        for phase in ["initial", "end", "start", "hidden"] {
            page.vm_mut()
                .eval(&format!("setOverflowPhase('{phase}')"))?;
            let crate::runtime::RendererCaptureScreenshotReply::Captured(screenshot) = page
                .capture_screenshot(
                    crate::runtime::RendererCaptureScreenshotRequest::viewport_png(),
                )?
            else {
                panic!("explicit screenshot demand must produce pixels");
            };
            let image = moli_image::decode_png(&screenshot.bytes)?;
            let checks: serde_json::Value = serde_json::from_str(
                &page
                    .vm_mut()
                    .eval("JSON.stringify(collectOverflowChecks())")?,
            )?;
            let checks = checks.as_array().expect("overflow checks");
            assert_eq!(checks.len(), if phase == "hidden" { 282 } else { 462 });
            for check in checks {
                assert_eq!(check["actual"], check["expected"], "{phase}: {check}");
                if let Some(pixel) = check.get("pixel") {
                    let x = pixel[0].as_f64().expect("pixel x") as usize;
                    let y = pixel[1].as_f64().expect("pixel y") as usize;
                    let offset = (y * image.width as usize + x) * 4;
                    assert_eq!(
                        &image.rgba[offset..offset + 4],
                        &[31, 127, 63, 255],
                        "{phase}: {} paint",
                        check["id"]
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("directional overflow fixture should run");
}
