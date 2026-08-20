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

#[tokio::test(flavor = "current_thread")]
async fn screenshot_transfers_stretched_cross_size_to_grid_items_automatic_minimum() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-ratio-automatic-minimum.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#grid{display:grid;width:200px;height:200px}
#item{aspect-ratio:2;align-self:stretch;justify-self:stretch}
</style>`;
document.body.innerHTML = `<div id=grid><div id=item></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 240, 1.0))?
            .expect("ratio Grid automatic-minimum screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(["grid","item"].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [rect.width,rect.height]}))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!([[200, 200], [400, 200]]),
            "the definite 200px cross-axis stretch must transfer through the 2:1 ratio before the auto column consumes the item's automatic minimum",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid ratio automatic-minimum fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_normal_grid_cross_stretch_for_ratio_intrinsic_contribution() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-ratio-normal-cross-stretch.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#grid{display:grid;width:200px;height:200px}
#item{aspect-ratio:2}
</style>`;
document.body.innerHTML = `<div id=grid><div id=item></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 240, 1.0))?
            .expect("normal Grid ratio screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(["grid","item"].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [rect.width,rect.height]}))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!([[200, 200], [400, 200]]),
            "normal Grid alignment must use its definite weak cross stretch when measuring the ratio item's inline contribution",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("normal Grid ratio cross-stretch fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_does_not_transfer_inline_maximum_into_explicit_grid_block_stretch() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-ratio-explicit-stretch-maximum.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#grid{display:grid;width:200px;height:200px}
#item{aspect-ratio:2;max-width:200px;align-self:stretch;justify-self:stretch}
</style>`;
document.body.innerHTML = `<div id=grid><div id=item></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 240, 1.0))?
            .expect("explicit Grid ratio stretch screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(["grid","item"].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [rect.width,rect.height]}))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!([[200, 200], [200, 200]]),
            "the inline maximum must not become a ratio-transferred block maximum when block auto sizing is an explicit stretch",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("explicit Grid ratio stretch fixture should run");
}
