use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_composites_solid_border_contours_once_before_transforms() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/solid-border-coverage.html")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let fixture = include_str!("../../../../tests/fixtures/solid-border-coverage.html");
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(fixture)?,
        ))?;
        page.vm_mut().sync_live_document_style_sources();
        let snapshot = page
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1280, 1800, 1.0))?
            .expect("border fixture layout");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let checks: serde_json::Value = serde_json::from_str(
            &page
                .vm_mut()
                .eval("JSON.stringify(collectBorderChecks())")?,
        )?;
        let checks = checks.as_array().expect("border checks");
        assert_eq!(checks.len(), 38);
        for check in checks {
            assert_eq!(check["actual"], check["expected"], "{check}");
            for sample in check["samples"].as_array().expect("border pixel samples") {
                let x = sample["point"][0].as_u64().expect("pixel x") as usize;
                let y = sample["point"][1].as_u64().expect("pixel y") as usize;
                let expected: [u8; 4] = std::array::from_fn(|index| {
                    sample["color"][index].as_u64().expect("color channel") as u8
                });
                let offset = (y * raster.width as usize + x) * 4;
                assert_eq!(
                    &raster.rgba[offset..offset + 4],
                    &expected,
                    "{} at ({x}, {y})",
                    check["id"],
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("solid border coverage fixture should run");
}
