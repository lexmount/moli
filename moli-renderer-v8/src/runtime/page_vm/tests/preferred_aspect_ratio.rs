use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_the_content_box_for_auto_preferred_aspect_ratios() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/preferred-aspect-ratio-box.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.item{display:block;border:20px solid;width:100px}
.ratio{aspect-ratio:2/1}
.auto-ratio{aspect-ratio:auto 2/1}
.border-box{box-sizing:border-box}
</style>`;
document.body.innerHTML = `
  <div id=ratio-content class="item ratio"></div>
  <div id=ratio-border class="item ratio border-box"></div>
  <div id=auto-content class="item auto-ratio"></div>
  <div id=auto-border class="item auto-ratio border-box"></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 400, 1.0))?
            .expect("preferred aspect-ratio screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['ratio-content','ratio-border','auto-content','auto-border'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("ratio-content", [140.0, 90.0]),
            ("ratio-border", [100.0, 50.0]),
            ("auto-content", [140.0, 90.0]),
            ("auto-border", [100.0, 70.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (axis, expected) in expected.into_iter().enumerate() {
                let actual = actual[axis].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{axis}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("preferred aspect-ratio fixture should run");
}
