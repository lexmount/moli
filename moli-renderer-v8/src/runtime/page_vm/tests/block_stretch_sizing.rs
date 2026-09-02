use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_block_stretch_from_the_containing_constraint_space() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/block-stretch-sizing.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.parent{display:block;box-sizing:content-box;width:300px;height:100px}
.child{width:20px}
.content-20{height:20px}
.content-120{height:120px}
.preferred{height:stretch}
.minimum{min-height:stretch}
.maximum{height:120px;max-height:stretch}
.as-flex{display:flex}
.as-grid{display:grid}
.indefinite{height:auto}
.minimum-parent{height:auto;min-height:100px}
.maximum-parent{height:auto;max-height:100px}
.margined{height:stretch;margin:10px 0}
.border-end{border-bottom:5px solid}
.padding-start{padding-top:5px}
.new-bfc{display:flow-root}
.scroll-container{overflow:auto}
.floated{float:left}
.percentage-descendant{height:50%}
.ratio{width:auto;height:stretch;aspect-ratio:2/1}
.vertical-flow{writing-mode:vertical-rl}
.vertical{writing-mode:vertical-rl;width:50px;height:50px;border-right:5px solid}
.vertical>.child{width:stretch;height:20px;margin-left:7px;margin-right:3px}
</style>`;
document.body.innerHTML = `
<div class=parent><div id=preferred class="child preferred"><div class=content-20></div></div></div>
<div class=parent><div id=minimum class="child minimum"><div class=content-20></div></div></div>
<div class=parent><div id=maximum class="child maximum"><div class=content-20></div></div></div>
<div class=parent><div id=flex class="child preferred as-flex"><div class=content-20></div></div></div>
<div class=parent><div id=grid class="child preferred as-grid"><div class=content-20></div></div></div>
<div id=indefinite-parent class="parent indefinite"><div id=indefinite class="child preferred"><div class=content-20></div></div></div>
<div id=minimum-parent class="parent minimum-parent"><div id=minimum-parent-child class="child preferred"><div class=content-20></div></div></div>
<div id=maximum-parent class="parent maximum-parent"><div id=maximum-parent-child class="child preferred"><div class=content-120></div></div></div>
<div class=parent><div id=collapsed class="child margined"></div></div>
<div class="parent border-end"><div id=border-end class="child margined"></div></div>
<div class="parent padding-start"><div id=padding-start class="child margined"></div></div>
<div class="parent new-bfc"><div id=new-bfc class="child margined"></div></div>
<div class="parent scroll-container"><div id=scroll-container class="child margined"></div></div>
<div class=parent><div id=floated class="child margined floated"></div></div>
<div class=parent><div id=percentage class="child preferred"><div id=percentage-descendant class=percentage-descendant></div></div></div>
<div class=parent><div id=ratio class="child ratio"></div></div>
<div class=vertical-flow><div class="parent vertical"><div id=vertical class=child></div></div></div>
<div class="parent vertical"><div id=vertical-orthogonal class=child></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 2_400, 1.0))?
            .expect("block stretch fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;

        for (id, expected) in [
            ("preferred", [20, 100]),
            ("minimum", [20, 100]),
            ("maximum", [20, 100]),
            ("flex", [20, 100]),
            ("grid", [20, 100]),
            ("indefinite-parent", [300, 20]),
            ("indefinite", [20, 20]),
            ("minimum-parent", [300, 100]),
            ("minimum-parent-child", [20, 20]),
            ("maximum-parent", [300, 100]),
            ("maximum-parent-child", [20, 120]),
            ("collapsed", [20, 100]),
            ("border-end", [20, 90]),
            ("padding-start", [20, 90]),
            ("new-bfc", [20, 80]),
            ("scroll-container", [20, 80]),
            ("floated", [20, 80]),
            ("percentage", [20, 100]),
            ("percentage-descendant", [20, 50]),
            ("ratio", [200, 100]),
            ("vertical", [47, 20]),
            ("vertical-orthogonal", [40, 20]),
        ] {
            assert_eq!(
                geometry[id],
                serde_json::json!(expected),
                "Chromium-calibrated block stretch geometry mismatch for {id}: {geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("block stretch fixture should run");
}
