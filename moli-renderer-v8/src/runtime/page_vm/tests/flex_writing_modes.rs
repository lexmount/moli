use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_sideways_writing_modes_across_stylo_and_layout() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-sideways-writing-modes.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
*{box-sizing:border-box}
html,body{margin:0;padding:0}
.fixture{
  display:flex;
  width:120px;
  height:100px;
  justify-content:space-between;
  align-items:flex-start;
  align-content:space-between;
  row-gap:7px;
  column-gap:11px;
}
.item{flex:0 0 30px;width:20px;height:18px}
.srl{writing-mode:sideways-rl}
.slr{writing-mode:sideways-lr}
.rtl{direction:rtl}
.column{flex-direction:column}
.wrap{flex-wrap:wrap}
</style>`;
const items = count => '<i class=item></i>'.repeat(count);
document.body.innerHTML = `
<div id=srl-row class="fixture srl">${items(2)}</div>
<div id=srl-row-rtl class="fixture srl rtl">${items(2)}</div>
<div id=slr-row class="fixture slr">${items(2)}</div>
<div id=slr-row-rtl class="fixture slr rtl">${items(2)}</div>
<div id=srl-column class="fixture srl column">${items(2)}</div>
<div id=slr-column class="fixture slr column">${items(2)}</div>
<div id=srl-wrap class="fixture srl wrap">${items(4)}</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"JSON.stringify([
CSS.supports('writing-mode','sideways-rl'),
CSS.supports('writing-mode','sideways-lr'),
getComputedStyle(document.getElementById('srl-row')).writingMode,
getComputedStyle(document.getElementById('slr-row')).writingMode
])"#,
            )?,
            r#"[true,true,"sideways-rl","sideways-lr"]"#,
            "standalone Stylo must parse, cascade, and serialize both sideways modes",
        );

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 800, 1.0))?
            .expect("sideways flex fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('.fixture')].map(host=>{
  const parent=host.getBoundingClientRect();
  return [host.id,[...host.children].map(item=>{
    const rect=item.getBoundingClientRect();
    return [rect.x-parent.x,rect.y-parent.y,rect.width,rect.height];
  })];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "srl-row": [[100, 0, 20, 30], [100, 70, 20, 30]],
                "srl-row-rtl": [[100, 70, 20, 30], [100, 0, 20, 30]],
                "slr-row": [[0, 70, 20, 30], [0, 0, 20, 30]],
                "slr-row-rtl": [[0, 0, 20, 30], [0, 70, 20, 30]],
                "srl-column": [[90, 0, 30, 18], [0, 0, 30, 18]],
                "slr-column": [[0, 82, 30, 18], [90, 82, 30, 18]],
                "srl-wrap": [
                    [100, 0, 20, 30],
                    [100, 70, 20, 30],
                    [0, 0, 20, 30],
                    [0, 70, 20, 30]
                ],
            }),
            "sideways modes must retain their vertical inline axes, physical block progression, and direction-dependent inline start",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("sideways flex writing-mode fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_distributed_flex_start_fallback_on_reversed_overflow() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-reversed-overflow.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
*{box-sizing:border-box}
html,body{margin:0;padding:0}
.fixture{
  display:flex;
  width:120px;
  height:100px;
  align-items:flex-start;
  row-gap:7px;
  column-gap:11px;
}
.item{flex:0 0 30px;width:20px;height:18px}
.main{flex-direction:row-reverse}
.cross{width:100px;height:60px;flex-wrap:wrap-reverse}
.cross .item{flex-basis:60px;width:40px;height:40px}
.vertical{writing-mode:vertical-rl;flex-direction:row-reverse}
</style>`;
const items = count => '<i class=item></i>'.repeat(count);
document.body.innerHTML = `
<div id=main-space class="fixture main" style="justify-content:space-between">${items(4)}</div>
<div id=main-stretch class="fixture main" style="justify-content:stretch">${items(4)}</div>
<div id=cross-space class="fixture cross" style="align-content:space-between">${items(2)}</div>
<div id=cross-stretch class="fixture cross" style="align-content:stretch">${items(2)}</div>
<div id=vertical-space class="fixture vertical" style="justify-content:space-between">${items(4)}</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 600, 1.0))?
            .expect("overflowing flex fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('.fixture')].map(host=>{
  const parent=host.getBoundingClientRect();
  return [host.id,[...host.children].map(item=>{
    const rect=item.getBoundingClientRect();
    return [rect.x-parent.x,rect.y-parent.y,rect.width,rect.height];
  })];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        let reversed_main = serde_json::json!([
            [90, 0, 30, 18],
            [49, 0, 30, 18],
            [8, 0, 30, 18],
            [-33, 0, 30, 18]
        ]);
        let reversed_cross = serde_json::json!([[0, 20, 60, 40], [0, -27, 60, 40]]);
        assert_eq!(geometry["main-space"], reversed_main);
        assert_eq!(geometry["main-stretch"], reversed_main);
        assert_eq!(geometry["cross-space"], reversed_cross);
        assert_eq!(geometry["cross-stretch"], reversed_cross);
        assert_eq!(
            geometry["vertical-space"],
            serde_json::json!([
                [100, 70, 20, 30],
                [100, 29, 20, 30],
                [100, -12, 20, 30],
                [100, -53, 20, 30]
            ])
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("reversed flex overflow fixture should run");
}
