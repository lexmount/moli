use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_svg_default_object_size_out_of_its_natural_ratio() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/svg-natural-sizing.html").expect("document URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
img{display:block}
.block-stretch{height:250px}
.inline-stretch{width:350px}
.both-stretch{width:350px;height:250px}
</style>`;
const source = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMCIvPg==";
document.body.innerHTML = `
<img src="${source}" id=natural>
<img src="${source}" class=block-stretch id=block-stretch>
<img src="${source}" class=inline-stretch id=inline-stretch>
<img src="${source}" class=both-stretch id=both-stretch>`;
'installed'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "Array.from(document.images, image => [image.complete, image.naturalWidth, image.naturalHeight]).join('|')",
            )?,
            "true,20,150|true,20,150|true,20,150|true,20,150",
            "local SVG resources should expose their concrete natural dimensions before layout",
        );

        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 1_000, 1.0))?
            .expect("SVG natural-sizing screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(
  ['natural','block-stretch','inline-stretch','both-stretch'].map(id => {
    const rect=document.getElementById(id).getBoundingClientRect();
    return [id,[rect.width,rect.height]];
  })
))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "natural": [20, 150],
                "block-stretch": [20, 250],
                "inline-stretch": [350, 150],
                "both-stretch": [350, 250],
            }),
            "the 300x150 concrete fallback must not manufacture a natural aspect ratio",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SVG natural-sizing fixture should run");
}
