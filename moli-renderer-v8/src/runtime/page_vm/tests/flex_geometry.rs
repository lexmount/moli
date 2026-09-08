use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_flex_baselines_across_logical_flows() {
    assert_flex_geometry_fixture(
        include_str!("../../../../tests/fixtures/flex-baseline-flows.html"),
        "collectFlexBaselineChecks()",
        324,
        1980,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_flex_static_margin_boxes_across_containing_blocks() {
    assert_flex_geometry_fixture(
        include_str!("../../../../tests/fixtures/flex-static-position-margins.html"),
        "collectFlexStaticMarginChecks()",
        117,
        6000,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_physical_absolute_insets_in_reversed_flex_scrollers() {
    assert_flex_geometry_fixture(
        include_str!("../../../../tests/fixtures/flex-physical-insets.html"),
        "collectFlexPhysicalInsetChecks()",
        96,
        1260,
    )
    .await;
}

async fn assert_flex_geometry_fixture(
    fixture: &'static str,
    collect_checks: &'static str,
    expected_count: usize,
    viewport_height: u32,
) {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-geometry.html")?,
        );
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 1280,
            inner_height: viewport_height,
            outer_width: 1280,
            outer_height: viewport_height,
            device_pixel_ratio: 1.0,
            screen_width: 1280,
            screen_height: viewport_height,
            screen_avail_width: 1280,
            screen_avail_height: viewport_height,
        }))?;
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
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
                .eval(&format!("JSON.stringify({collect_checks})"))?,
        )?;
        let checks = checks.as_array().expect("flex geometry checks");
        assert_eq!(checks.len(), expected_count);
        for check in checks {
            assert_eq!(check["actual"], check["expected"], "{check}");
            let x = check["pixel"][0].as_f64().expect("pixel x") as usize;
            let y = check["pixel"][1].as_f64().expect("pixel y") as usize;
            let offset = (y * image.width as usize + x) * 4;
            let color: [u8; 4] = check.get("color").map_or([31, 127, 63, 255], |color| {
                std::array::from_fn(|index| color[index].as_u64().expect("color channel") as u8)
            });
            assert_eq!(
                &image.rgba[offset..offset + 4],
                &color,
                "{} paint",
                check["id"]
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex geometry fixture should run");
}
