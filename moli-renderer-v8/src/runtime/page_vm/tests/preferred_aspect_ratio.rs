use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_preferred_ratios_at_layout_boundaries() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/preferred-aspect-ratio.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.ordinary{display:block;width:100px;border:20px solid}
#ratio-content{aspect-ratio:2/1}
#ratio-border{box-sizing:border-box;aspect-ratio:2/1}
#auto-content{aspect-ratio:auto 2/1}
#auto-border{box-sizing:border-box;aspect-ratio:auto 2/1}
svg{display:block;width:120px;height:auto}
.asymmetric-insets{
  box-sizing:border-box;
  padding:3px 10px;
  border:solid;
  border-width:2px 5px;
}
#natural-auto{aspect-ratio:auto}
#natural-auto-fallback{aspect-ratio:auto 3/2}
#fallback-no-natural{aspect-ratio:auto 3/2}
#specified-border{aspect-ratio:3/2}
#natural-border{aspect-ratio:auto}
#height-only{width:auto;height:50px;aspect-ratio:2/1}
#both-definite{width:130px;height:70px;aspect-ratio:1/1}
#degenerate{aspect-ratio:0/1}
#auto-degenerate{aspect-ratio:auto 0/1}
.automatic-minimum{display:block}
#block-automatic-minimum,#row-flex-automatic-minimum{
  height:100px;
  aspect-ratio:1/2;
}
#block-transferred-maximum{
  height:200px;
  max-height:100px;
  aspect-ratio:1/2;
}
#column-flex-automatic-minimum{
  width:100px;
  aspect-ratio:2/1;
}
#row-flex-automatic-minimum,#column-flex-automatic-minimum{flex-basis:0}
#flex-transferred-maximum{max-height:50px;aspect-ratio:2/1}
.row-flex{display:flex}
.column-flex{display:flex;flex-direction:column}
.wide-content{width:100px}
.wider-content{width:200px}
.tall-content{height:100px}
</style>`;
document.body.innerHTML = `
<div class=ordinary id=ratio-content></div>
<div class=ordinary id=ratio-border></div>
<div class=ordinary id=auto-content></div>
<div class=ordinary id=auto-border></div>
<svg id=natural-auto viewBox="0 0 200 100"></svg>
<svg id=natural-auto-fallback viewBox="0 0 200 100"></svg>
<svg class=asymmetric-insets id=fallback-no-natural></svg>
<svg class=asymmetric-insets id=specified-border viewBox="0 0 200 100"></svg>
<svg class=asymmetric-insets id=natural-border viewBox="0 0 200 100"></svg>
<svg id=height-only viewBox="0 0 200 100"></svg>
<svg id=both-definite viewBox="0 0 200 100"></svg>
<svg id=degenerate viewBox="0 0 200 100"></svg>
<svg id=auto-degenerate viewBox="0 0 200 100"></svg>
<div class=automatic-minimum id=block-automatic-minimum><div class=wide-content></div></div>
<div class=automatic-minimum id=block-transferred-maximum><div class=wide-content></div></div>
<div class=row-flex><div class=automatic-minimum id=row-flex-automatic-minimum><div class=wide-content></div></div></div>
<div class=column-flex><div class=automatic-minimum id=column-flex-automatic-minimum><div class=tall-content></div></div></div>
<div class=row-flex><div class=automatic-minimum id=flex-transferred-maximum><div class=wider-content></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 1_400, 1.0))?
            .expect("preferred aspect-ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("ratio-content", [140, 90]),
            ("ratio-border", [100, 50]),
            ("auto-content", [140, 90]),
            ("auto-border", [100, 70]),
            ("natural-auto", [120, 60]),
            ("natural-auto-fallback", [120, 60]),
            ("fallback-no-natural", [120, 70]),
            ("specified-border", [120, 80]),
            ("natural-border", [120, 55]),
            ("height-only", [100, 50]),
            ("both-definite", [130, 70]),
            ("degenerate", [120, 60]),
            ("auto-degenerate", [120, 60]),
            ("block-automatic-minimum", [100, 100]),
            ("block-transferred-maximum", [100, 100]),
            ("row-flex-automatic-minimum", [100, 100]),
            ("column-flex-automatic-minimum", [100, 100]),
            ("flex-transferred-maximum", [100, 50]),
        ] {
            assert_eq!(
                geometry[id],
                serde_json::json!(expected),
                "Chromium-calibrated geometry mismatch for {id}: {geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("preferred aspect-ratio fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_intrinsic_keywords_from_ratio_dependent_content_contributions() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-ratio-contributions.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.item{display:block;flex:none;align-self:flex-start;justify-self:start}
.ratio-preferred{width:min-content;height:100px;aspect-ratio:1/1}
.host{width:300px;height:150px}
.flex{display:flex}
.grid{display:grid}
.absolute-host{position:relative}
.absolute-host>.item{position:absolute;left:0;top:0}
#minimum{width:auto;min-width:min-content;height:25px;aspect-ratio:4/1}
#minimum>div{width:150px}
#maximum{width:200px;max-width:max-content;height:25px;aspect-ratio:4/1}
#maximum>div{width:150px}
#clamped-opposite{width:min-content;height:300px;max-height:25px;aspect-ratio:4/1}
#content-box{box-sizing:content-box;width:min-content;height:100px;padding:10px;aspect-ratio:1/1}
</style>`;
document.body.innerHTML = `
<div class="item ratio-preferred" id="block"></div>
<div class="item" id="minimum"><div></div></div>
<div class="item" id="maximum"><div></div></div>
<div class="item" id="clamped-opposite"></div>
<div class="item" id="content-box"></div>
<div class="host flex"><div class="item ratio-preferred" id="flex"></div></div>
<div class="host grid"><div class="item ratio-preferred" id="grid"></div></div>
<div class="host absolute-host"><div class="item ratio-preferred" id="absolute"></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 1_000, 1.0))?
            .expect("intrinsic ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("block", [100, 100]),
            ("minimum", [100, 25]),
            ("maximum", [100, 25]),
            ("clamped-opposite", [100, 25]),
            ("content-box", [120, 120]),
            ("flex", [100, 100]),
            ("grid", [100, 100]),
            ("absolute", [100, 100]),
        ] {
            assert_eq!(
                geometry[id],
                serde_json::json!(expected),
                "Chromium-calibrated intrinsic ratio geometry mismatch for {id}: {geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic ratio contribution fixture should run");
}
