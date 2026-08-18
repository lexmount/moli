use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_scales_replaced_natural_sizes_into_effective_zoom_space() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/replaced-natural-size-zoom.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
canvas{display:block}
.container{height:100px;width:100px}
#x4-container{zoom:4}
</style>`;
document.body.innerHTML = `
<div class=container>
  <canvas id=plain width=1 height=1></canvas>
  <canvas id=x2 width=1 height=1 style="zoom:2"></canvas>
</div>
<div class=container id=x4-container>
  <canvas id=x4 width=1 height=1></canvas>
  <canvas id=x8 width=1 height=1 style="zoom:2"></canvas>
  <div id=after></div>
</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(480, 320, 1.0))?
            .expect("replaced natural-size zoom fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['plain','x2','x4','x8','after'].map(id=>{
  const rect=document.getElementById(id).getBoundingClientRect();
  return [id,[rect.left,rect.top,rect.width,rect.height]];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("plain", [8, 8, 1, 1]),
            ("x2", [8, 9, 2, 2]),
            ("x4", [8, 108, 4, 4]),
            ("x8", [8, 112, 8, 8]),
            ("after", [8, 120, 400, 0]),
        ] {
            assert_eq!(geometry[id], serde_json::json!(expected), "{id}");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("replaced natural-size zoom fixture should run");
}
