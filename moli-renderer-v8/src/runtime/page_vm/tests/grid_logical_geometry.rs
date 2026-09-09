use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_grid_tracks_and_fragments_in_all_logical_flows() {
    check_logical_geometry_fixture(
        include_str!("../../../../tests/fixtures/grid-logical-flows.html"),
        520,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_table_parts_consume_logical_grid_columns() {
    check_logical_geometry_fixture(
        include_str!("../../../../tests/fixtures/table-logical-columns.html"),
        106,
    )
    .await;
}

async fn check_logical_geometry_fixture(source: &'static str, expected_count: usize) {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/logical-geometry.html")?,
        );
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 1280,
            inner_height: 7200,
            outer_width: 1280,
            outer_height: 7200,
            device_pixel_ratio: 1.0,
            screen_width: 1280,
            screen_height: 7200,
            screen_avail_width: 1280,
            screen_avail_height: 7200,
        }))?;
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(source)?,
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        // On-demand geometry reads intentionally remain stale until capture.
        let crate::runtime::RendererCaptureScreenshotReply::Captured(screenshot) = page
            .capture_screenshot(crate::runtime::RendererCaptureScreenshotRequest::viewport_png())?
        else {
            panic!("logical geometry fixture must produce a screenshot");
        };
        let image = moli_image::decode_png(&screenshot.bytes)?;
        let checks: serde_json::Value = serde_json::from_str(
            &page
                .vm_mut()
                .eval("JSON.stringify(collectGeometryChecks())")?,
        )?;
        let checks = checks.as_array().expect("logical geometry checks");
        assert_eq!(checks.len(), expected_count);
        for check in checks {
            assert_eq!(check["actual"], check["expected"], "{check}");
            if let Some(pixel) = check.get("pixel") {
                let x = pixel[0].as_f64().expect("pixel x") as usize;
                let y = pixel[1].as_f64().expect("pixel y") as usize;
                let offset = (y * image.width as usize + x) * 4;
                let color: [u8; 4] = serde_json::from_value(check["color"].clone())?;
                assert_eq!(
                    &image.rgba[offset..offset + 4],
                    &color,
                    "{} paint",
                    check["id"]
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("logical geometry fixture should run");
}
