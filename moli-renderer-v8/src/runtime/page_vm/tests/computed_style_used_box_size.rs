use super::*;

#[tokio::test(flavor = "current_thread")]
async fn computed_style_resolves_box_sizes_from_the_published_layout_tree() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/computed-used-box-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.probe{position:absolute;margin:0}
#precedence{left:0;top:0;min-width:260px;max-width:200px}
#content{left:0;top:40px;box-sizing:content-box;width:100px;height:40px;padding:10px;border:5px solid;transform:scale(2);transform-origin:0 0}
#border{left:300px;top:40px;box-sizing:border-box;width:100px;height:40px;padding:10px;border:5px solid}
#atomic{left:0;top:200px;display:inline-block;width:auto;height:auto;min-width:120px;min-height:30px}
#inline{left:0;top:250px;position:static;display:inline;width:80%;height:30px}
#hidden{display:none;width:77%;height:88%}
#contents{display:contents;width:66%;height:55%}
#stale{left:0;top:300px;box-sizing:content-box;width:90px;height:45px;padding:10px;border:5px solid}
#zoom{left:300px;top:300px;box-sizing:content-box;width:100px;height:20px;padding:10px;zoom:2}
#shrink-to-fit{left:500px;top:300px;height:136px}
#shrink-to-fit-child{height:68px;min-width:260px;max-width:200px}
</style>`;
document.body.innerHTML = `
  <div id=precedence class=probe></div>
  <div id=content class=probe></div>
  <div id=border class=probe></div>
  <span id=atomic></span>
  <span id=inline></span>
  <div id=hidden></div>
  <div id=contents></div>
  <div id=stale class=probe></div>
  <div id=zoom class=probe></div>
  <div id=shrink-to-fit class=probe><div id=shrink-to-fit-child></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let viewport = moli_layout::PaintViewport::new(800, 600, 1.0);
        assert_eq!(
            page_vm.vm_mut().eval(
                "(() => { const style=getComputedStyle(document.getElementById('precedence')); return `${style.width}|${style.height}`; })()",
            )?,
            "260px|0px",
            "the first resolved-value read must be allowed to publish the cold layout epoch",
        );
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("computed used-size fixture must publish a layout tree");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"JSON.stringify({computed:Object.fromEntries(['precedence','content','border','atomic','inline','hidden','contents','stale','zoom','shrink-to-fit','shrink-to-fit-child'].map(id=>{const style=getComputedStyle(document.getElementById(id));return [id,[style.width,style.height]]})),rects:Object.fromEntries(['precedence','content','border','atomic','stale','zoom','shrink-to-fit','shrink-to-fit-child'].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [id,[rect.width,rect.height]]}))})"#,
            )?,
            r#"{"computed":{"precedence":["260px","0px"],"content":["100px","40px"],"border":["100px","40px"],"atomic":["120px","30px"],"inline":["80%","30px"],"hidden":["77%","88%"],"contents":["66%","55%"],"stale":["90px","45px"],"zoom":["100px","20px"],"shrink-to-fit":["260px","136px"],"shrink-to-fit-child":["260px","68px"]},"rects":{"precedence":[260,0],"content":[260,140],"border":[100,40],"atomic":[120,30],"stale":[120,75],"zoom":[240,80],"shrink-to-fit":[260,136],"shrink-to-fit-child":[260,68]}}"#,
            "resolved width/height must use the untransformed content or border box selected by the published layout-time box-sizing",
        );

        page_vm.vm_mut().eval(
            "document.getElementById('stale').style.cssText += ';box-sizing:border-box;width:180px;height:70px'",
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "(() => { const style=getComputedStyle(document.getElementById('stale')); return `${style.width}|${style.height}`; })()",
            )?,
            "90px|45px",
            "a style mutation must not splice fresh box-sizing into the previously published layout result",
        );

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("the explicit screenshot must publish the mutated layout tree");
        assert_eq!(
            page_vm.vm_mut().eval(
                "(() => { const style=getComputedStyle(document.getElementById('stale')); return `${style.width}|${style.height}`; })()",
            )?,
            "180px|70px",
            "resolved width/height must advance after the screenshot publishes fresh layout",
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("computed used-size fixture should run");
}
