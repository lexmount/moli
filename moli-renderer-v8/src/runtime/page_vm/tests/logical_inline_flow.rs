use super::*;
use base64::Engine as _;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_measures_inline_content_without_border_box_double_counting() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/inline-content-measurement.html"),
        360,
        120,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_positions_final_fieldset_fragments_without_losing_parent_alignment() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/logical-fieldset-positioning.html"),
        180,
        90,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_separates_fieldset_legend_content_scrolling_and_baselines() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/logical-fieldset-flow.html"),
        544,
        64,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_shares_quantized_baselines_and_table_writing_contexts() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/logical-inline-baselines.html"),
        16,
        4,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_publishes_logical_inline_geometry_text_ranges_and_carets() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/logical-inline-flow.html"),
        760,
        140,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_positions_logical_inline_objects_and_float_exclusions() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/logical-inline-objects.html"),
        190,
        190,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paints_sliced_inline_edges_in_physical_content_space() {
    assert_logical_inline_fixture(
        include_str!("../../../../tests/fixtures/logical-inline-paint.html"),
        280,
        40,
    )
    .await;
}

async fn assert_logical_inline_fixture(
    fixture: &'static str,
    visible_count: usize,
    hidden_count: usize,
) {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/logical-inline-flow.html")?,
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
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        )));
        let fixture = fixture.replace(
            "{{MOLI_AHEM_DATA_URL}}",
            &format!("data:font/ttf;base64,{font}"),
        );
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(&fixture)?,
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        for phase in ["initial", "end", "start", "hidden"] {
            page.vm_mut()
                .eval(&format!("setOverflowPhase('{phase}')"))?;
            // Capture owns layout. Geometry and source queries below must use
            // that published snapshot, including after DOM/style mutations.
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
            let checks = checks.as_array().expect("logical inline checks");
            assert_eq!(
                checks.len(),
                if phase == "hidden" {
                    hidden_count
                } else {
                    visible_count
                }
            );
            for check in checks {
                assert_eq!(check["actual"], check["expected"], "{phase}: {check}");
                if let Some(pixel) = check.get("pixel") {
                    let x = pixel[0].as_f64().expect("pixel x") as usize;
                    let y = pixel[1].as_f64().expect("pixel y") as usize;
                    let offset = (y * image.width as usize + x) * 4;
                    let color: [u8; 4] = std::array::from_fn(|index| {
                        check["color"][index].as_u64().expect("color channel") as u8
                    });
                    assert_eq!(
                        &image.rgba[offset..offset + 4],
                        &color,
                        "{phase}: {} paint",
                        check["id"]
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("logical inline fixture should run");
}
