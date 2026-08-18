use super::*;

#[tokio::test(flavor = "current_thread")]
async fn computed_style_resolves_margins_from_the_published_layout_tree() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/computed-used-margin.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.container{width:200px;height:100px}
.item{width:40px;height:20px}
#block{margin:auto}
#flex,#stale-container{display:flex}
#flex-item,#stale{margin:auto}
#grid{display:grid;grid-template-columns:1fr;grid-template-rows:1fr}
#grid-item{margin:auto}
#vertical{display:flex;writing-mode:vertical-rl}
#vertical-item{margin:auto}
#percent{margin:10% 5%}
#fixed{margin-left:.1px}
#inline{display:inline;margin:auto}
#hidden{display:none;margin:auto}
#zoom{display:flex;zoom:2}
#zoom-item{margin:auto}
#collapsed{display:block;margin-top:10px}
#collapsed-child{display:block;width:10px;height:10px;margin-top:50px}
</style>`;
document.body.innerHTML = `
  <div class=container><div id=block class=item></div></div>
  <div id=flex class=container><div id=flex-item class=item></div></div>
  <div id=grid class=container><div id=grid-item class=item></div></div>
  <div id=vertical class=container><div id=vertical-item class=item></div></div>
  <div class=container><div id=percent class=item></div></div>
  <div class=container><div id=fixed class=item></div></div>
  <span id=inline>inline</span>
  <div id=hidden></div>
  <div id=zoom class=container><div id=zoom-item class=item></div></div>
  <div id=stale-container class=container><div id=stale class=item></div></div>
  <div id=collapsed><div id=collapsed-child></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let viewport = moli_layout::PaintViewport::new(800, 1200, 1.0);
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("computed used-margin fixture must publish a layout tree");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
(() => {
  const margins = id => {
    const style = getComputedStyle(document.getElementById(id));
    return [style.marginTop, style.marginRight, style.marginBottom, style.marginLeft];
  };
  return JSON.stringify(Object.fromEntries([
    'block','flex-item','grid-item','vertical-item','percent','fixed',
    'inline','hidden','zoom-item','stale','collapsed'
  ].map(id => [id, margins(id)])));
})()
"#,
            )?,
            r#"{"block":["0px","80px","0px","80px"],"flex-item":["40px","80px","40px","80px"],"grid-item":["0px","0px","0px","0px"],"vertical-item":["40px","80px","40px","80px"],"percent":["20px","10px","20px","10px"],"fixed":["0px","0px","0px","0.1px"],"inline":["auto","auto","auto","auto"],"hidden":["auto","auto","auto","auto"],"zoom-item":["40px","80px","40px","80px"],"stale":["40px","80px","40px","80px"],"collapsed":["10px","0px","0px","0px"]}"#,
            "resolved margins must follow Chromium's LayoutBox used-value boundary across formatting contexts",
        );

        page_vm.vm_mut().eval(
            "document.getElementById('stale-container').style.cssText += ';width:300px;height:200px'",
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "(() => { const s=getComputedStyle(document.getElementById('stale')); return [s.marginTop,s.marginRight,s.marginBottom,s.marginLeft].join('|'); })()",
            )?,
            "40px|80px|40px|80px",
            "a style mutation must not splice fresh container geometry into the published layout epoch",
        );

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("the explicit screenshot must publish the mutated layout tree");
        assert_eq!(
            page_vm.vm_mut().eval(
                "(() => { const s=getComputedStyle(document.getElementById('stale')); return [s.marginTop,s.marginRight,s.marginBottom,s.marginLeft].join('|'); })()",
            )?,
            "90px|130px|90px|130px",
            "resolved margins must advance with the next published layout epoch",
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("computed used-margin fixture should run");
}
