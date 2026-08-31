use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_omits_whitespace_only_flex_and_grid_items_even_when_preserved() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-whitespace-box-generation.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.probe{white-space:pre;vertical-align:top}
.grid{display:inline-grid;grid-auto-flow:column;grid-auto-columns:20px;grid-auto-rows:20px}
.flex{display:inline-flex}
.item{width:20px;height:20px}
</style>`;
document.body.innerHTML = `
<div class="probe grid" id=grid-whitespace> \t\n <div class=item></div>\r\f </div>
<div class="probe grid" id=grid-nbsp>&nbsp;<div class=item></div></div>
<div class="probe grid" id=grid-text>text<div class=item></div></div>
<div class="probe flex" id=flex-whitespace> \t\n <div class=item></div>\r\f </div>
<div class="probe flex" id=flex-text>text<div class=item></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 200, 1.0))?
            .expect("Grid whitespace box-generation screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['grid-whitespace','grid-nbsp','grid-text','flex-whitespace','flex-text'].map(id=>{const host=document.getElementById(id),item=host.querySelector('.item'),hostRect=host.getBoundingClientRect(),itemRect=item.getBoundingClientRect(),itemX=itemRect.x-hostRect.x;if(id.startsWith('grid-'))return [id,{width:hostRect.width,itemX,columns:getComputedStyle(host).gridTemplateColumns}];if(id==='flex-whitespace')return [id,{width:hostRect.width,itemX}];return [id,{anonymousItem:itemX>0&&hostRect.width>itemRect.width}]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "grid-whitespace": {"width": 20, "itemX": 0, "columns": "20px"},
                "grid-nbsp": {"width": 40, "itemX": 20, "columns": "20px 20px"},
                "grid-text": {"width": 40, "itemX": 20, "columns": "20px 20px"},
                "flex-whitespace": {"width": 20, "itemX": 0},
                "flex-text": {"anonymousItem": true},
            }),
            "only CSS whitespace sequences must disappear before Grid item generation; non-breaking spaces and ordinary text must retain their anonymous item",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid whitespace box-generation fixture should run");
}
