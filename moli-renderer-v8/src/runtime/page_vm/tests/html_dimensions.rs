use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_html_dimension_hints_through_the_cascade() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/html-dimensions.html")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let fixture = include_str!("../../../../tests/fixtures/html-dimension-hints.html");
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(fixture)?,
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        for phase in ["initial", "mutated", "resized", "reset"] {
            page.vm_mut()
                .eval(&format!("setDimensionPhase('{phase}')"))?;
            // Attribute mutation updates style inputs, but synchronous geometry
            // intentionally observes the last snapshot until screenshot demand.
            page.vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1280, 4000, 1.0))?
                .expect("dimension fixture must retain a layout root");
            let checks: serde_json::Value = serde_json::from_str(
                &page
                    .vm_mut()
                    .eval("JSON.stringify(collectDimensionChecks())")?,
            )?;
            let checks = checks.as_array().expect("dimension checks");
            assert_eq!(checks.len(), 70, "{phase}");
            for check in checks {
                assert_eq!(check["actual"], check["expected"], "{phase}: {check}");
                assert_eq!(
                    check["style"][2], check["expectedRatio"],
                    "{phase}: {check}"
                );
                if !check["expectedStyle"].is_null() {
                    assert_eq!(check["style"], check["expectedStyle"], "{phase}: {check}");
                }
                assert_eq!(check["bitmap"], check["expectedBitmap"], "{phase}: {check}");
                assert_eq!(
                    check["natural"], check["expectedNatural"],
                    "{phase}: {check}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("HTML dimension cascade fixture should run");
}
