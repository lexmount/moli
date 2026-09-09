use super::*;

#[tokio::test(flavor = "current_thread")]
async fn image_dimensions_use_rendered_content_size_without_forcing_a_refresh() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/image-dimensions.html")?,
        );
        page_vm.vm_mut().eval(include_str!(
            "../../../../../tests/fixtures/image-layout-dimensions-setup.js"
        ))?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let viewport = moli_layout::LayoutViewport::new(200, 600, 1.0);
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("image dimensions layout");
        let source = include_str!("../../../../../tests/fixtures/image-layout-dimensions.js");
        let result = page_vm
            .vm_mut()
            .eval(&format!("JSON.stringify({source})"))?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result)?,
            serde_json::json!({
                "css": [40,30,0,0], "override": [40,30,0,0], "edges": [40,30,0,0],
                "borderbox": [30,20,0,0], "transformed": [40,30,0,0],
                "zoomed": [40,30,0,0], "vertical": [40,30,0,0],
                "fractional": [41,31,0,0], "hidden": [33,22,0,0]
            }),
            "shared fixture must match Chromium's content-box image dimensions"
        );
        let passes = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
const image = document.getElementById('css');
image.style.width = '70px'; image.style.height = '50px';
image.width = 120; image.height = 90;
[image.width,image.height].join('|')
"#
            )?,
            "40|30",
            "synchronous getters must retain the published snapshot after mutation"
        );
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, passes);
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("refreshed image dimensions layout");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("[image.width,image.height].join('|')")?,
            "70|50",
            "a rendering checkpoint publishes the new CSS size, not the HTML attributes"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("image dimension geometry fixture should run");
}
