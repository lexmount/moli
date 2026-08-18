use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_contained_grid_size_to_expand_auto_fit_tracks() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-contained-auto-fit.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#grid{
  display:grid;
  contain:size;
  contain-intrinsic-size:70px 80px;
  width:max-content;
  border:3px solid;
  gap:5px;
  grid-template:1fr 2fr/repeat(auto-fit,15px);
}
.item{height:100%}
</style>`;
document.body.innerHTML = `<div id=grid>${'<div class=item></div>'.repeat(6)}</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 140, 1.0))?
            .expect("contained grid auto-fit screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const grid=document.getElementById('grid').getBoundingClientRect(),items=[...document.querySelectorAll('.item')].map(item=>{const rect=item.getBoundingClientRect();return [rect.x-grid.x,rect.y-grid.y,rect.width,rect.height]});return {grid:[grid.width,grid.height],items}})())"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "grid": [76, 86],
                "items": [
                    [3, 3, 15, 25],
                    [23, 3, 15, 25],
                    [43, 3, 15, 25],
                    [3, 33, 15, 50],
                    [23, 33, 15, 50],
                    [43, 33, 15, 50],
                ],
            }),
            "the contained content box must establish Grid's auto-fit constraint before track expansion",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("contained grid auto-fit fixture should run");
}
