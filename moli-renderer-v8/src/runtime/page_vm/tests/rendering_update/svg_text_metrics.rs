use super::*;

const SETUP: &str = include_str!("../../../../../tests/fixtures/svg-text-metrics-setup.js");
const CHECK: &str = include_str!("../../../../../tests/fixtures/svg-text-metrics-check.js");

#[tokio::test(flavor = "current_thread")]
async fn svg_text_queries_use_shaped_glyphs_and_native_source_provenance() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/svg-text-metrics.html")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(SETUP)?;
        page.vm_mut().sync_live_document_style_sources();
        let snapshot = page
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::LayoutViewport::new(800, 600, 1.0))?
            .expect("real SVG layout");
        assert!(!snapshot.svg_images.is_empty());
        let actual = page.vm_mut().eval(&format!("JSON.stringify({CHECK})"))?;
        let actual: serde_json::Value = serde_json::from_str(&actual)?;
        assert_eq!(
            actual["failures"],
            serde_json::json!([]),
            "SVG queries: {actual}"
        );
        assert!(actual["cases"].as_u64().unwrap() >= 40);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG text metrics fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn svg_text_queries_cold_start_then_keep_frozen_metrics_until_capture() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/svg-text-freeze.html")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval("document.body.innerHTML='<svg><text id=label style=\"font:20px serif\">WWW</text></svg>'")?;
        page.vm_mut().sync_live_document_style_sources();
        assert_eq!(page.vm_mut().eval("globalThis.label=document.getElementById('label'); globalThis.oldLength=label.getComputedTextLength(); oldLength>0 && label.getNumberOfChars()===3")?, "true");
        page.vm_mut().eval("label.textContent='WWWWWW'; label.style.fontSize='40px'")?;
        assert_eq!(page.vm_mut().eval("label.getComputedTextLength()===oldLength && label.getNumberOfChars()===3")?, "true");
        page.vm_mut().screenshot_layout_snapshot(moli_layout::LayoutViewport::new(800,600,1.0))?
            .expect("refresh SVG text");
        assert_eq!(page.vm_mut().eval("label.getNumberOfChars()===6 && Math.abs(label.getComputedTextLength()-4*oldLength)<0.01")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.expect("frozen SVG text fixture");
}
