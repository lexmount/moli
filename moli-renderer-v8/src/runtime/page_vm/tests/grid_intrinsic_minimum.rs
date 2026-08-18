use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_includes_grid_item_outer_minimum_in_an_intrinsic_track() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-item-minimum-width.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#grid{
  display:inline-grid;
  border:5px solid;
  grid:10px 10px/minmax(auto,0px);
}
#contributor{
  width:60px;
  margin-left:5px;
  margin-right:10px;
  padding-left:6px;
  padding-right:3px;
  border-left:2px solid;
  border-right:4px solid;
}
</style>`;
document.body.innerHTML = `<div id=grid><div id=contributor></div><div id=stretched></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 80, 1.0))?
            .expect("intrinsic Grid track screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(["grid","contributor","stretched"].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [rect.width,rect.height]}))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!([[100, 30], [75, 10], [90, 10]]),
            "the 60px content box plus padding, border and margins must establish a 90px automatic track minimum",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid item minimum-width fixture should run");
}
