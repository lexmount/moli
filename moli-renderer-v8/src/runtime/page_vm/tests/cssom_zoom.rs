use super::*;

fn numeric(value: &serde_json::Value, path: &str) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("missing numeric {path}: {value}"))
}

#[tokio::test(flavor = "current_thread")]
async fn cssom_box_metrics_remove_effective_zoom_without_unzooming_client_rects() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/cssom-absolute-zoom-box-metrics.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.square{box-sizing:content-box;width:64px;height:64px;border:4px solid}
#zoomed{zoom:4}
#nested-parent{zoom:4}
#nested{zoom:4}
.button-row{display:flex;width:400px;zoom:2}
.button-row button{flex:0 0 auto;min-width:200px;margin:0}
</style>`;
document.body.innerHTML = `<div id=plain class=square></div><div id=zoomed class=square></div><div id=nested-parent><div id=nested class=square></div></div><div class=button-row><button id=button>Run</button></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1600, 1200, 1.0))?
            .expect("CSS zoom box-metric screenshot layout");

        let metrics = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['plain','zoomed','nested','button'].map(id=>{
  const element=document.getElementById(id),rect=element.getBoundingClientRect();
  return [id,{
    offset:[element.offsetWidth,element.offsetHeight],
    client:[element.clientWidth,element.clientHeight,element.clientLeft,element.clientTop],
    rect:[rect.width,rect.height]
  }];
})))"#,
        )?;
        let metrics: serde_json::Value = serde_json::from_str(&metrics)?;
        for id in ["plain", "zoomed", "nested"] {
            assert_eq!(numeric(&metrics[id]["offset"][0], id), 72.0, "{metrics}");
            assert_eq!(numeric(&metrics[id]["offset"][1], id), 72.0, "{metrics}");
            assert_eq!(numeric(&metrics[id]["client"][0], id), 64.0, "{metrics}");
            assert_eq!(numeric(&metrics[id]["client"][1], id), 64.0, "{metrics}");
            assert_eq!(numeric(&metrics[id]["client"][2], id), 4.0, "{metrics}");
            assert_eq!(numeric(&metrics[id]["client"][3], id), 4.0, "{metrics}");
        }
        assert_eq!(numeric(&metrics["plain"]["rect"][0], "plain rect"), 72.0);
        assert_eq!(numeric(&metrics["zoomed"]["rect"][0], "zoomed rect"), 288.0);
        assert_eq!(numeric(&metrics["nested"]["rect"][0], "nested rect"), 1152.0);
        assert_eq!(numeric(&metrics["button"]["offset"][0], "button offset"), 200.0);
        assert_eq!(numeric(&metrics["button"]["rect"][0], "button rect"), 400.0);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSS zoom box-metric fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cssom_root_client_size_remains_in_layout_viewport_space_under_zoom() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/cssom-root-zoom-client-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.documentElement.style.zoom='2';
document.body.style.margin='0';
document.body.innerHTML='<div id=child style="width:100px;height:20px"></div>';
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("root zoom CSSOM screenshot layout");

        let metrics = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const root=document.documentElement,child=document.getElementById('child'),rootRect=root.getBoundingClientRect(),childRect=child.getBoundingClientRect();return {rootClient:[root.clientWidth,root.clientHeight],rootWidth:[root.offsetWidth,rootRect.width],childWidth:[child.clientWidth,child.offsetWidth,childRect.width]}})())"#,
        )?;
        let metrics: serde_json::Value = serde_json::from_str(&metrics)?;
        assert_eq!(metrics["rootClient"], serde_json::json!([800, 600]));
        assert_eq!(numeric(&metrics["rootWidth"][0], "root offset width"), 400.0);
        assert_eq!(numeric(&metrics["rootWidth"][1], "root rect width"), 800.0);
        for (index, expected) in [100.0, 100.0, 200.0].into_iter().enumerate() {
            assert_eq!(numeric(&metrics["childWidth"][index], "child width"), expected);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("root zoom CSSOM fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cssom_offsets_resolve_the_offset_parent_before_removing_target_zoom() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/cssom-absolute-zoom-offsets.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
.outer{width:100px;height:100px;border:1px solid;position:relative;margin:10px}
.square{width:10px;height:10px;margin:1px}
.one{position:relative;top:10px;left:10px}
.two{position:absolute;top:20px;left:20px;zoom:2}
.three{position:absolute;top:10px;left:50px;zoom:.5}
</style>`;
document.body.innerHTML = `
<div class=outer><div id=u1 class="square one"></div><div id=u2 class="square two"></div><div id=u3 class="square three"></div></div>
<div class=outer style="zoom:3"><div id=z1 class="square one"></div><div id=z2 class="square two"></div><div id=z3 class="square three"></div></div>
<div id=outer class=outer style="margin:30px"><div style="margin:10px;zoom:2"><div id=inner-in-zoom class=square></div></div></div>
<div class=outer style="margin:30px"><div id=zoom-boundary><div id=zoomed-inner class=square style="zoom:2;width:100px;height:100px;border:1px solid"></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1200, 1200, 1.0))?
            .expect("CSS zoom offset screenshot layout");

        let offsets = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['u1','u2','u3','z1','z2','z3','inner-in-zoom','zoomed-inner'].map(id=>{const e=document.getElementById(id);return [id,[e.offsetTop,e.offsetLeft,e.offsetWidth,e.offsetHeight,e.offsetParent&&e.offsetParent.id]]})))"#,
        )?;
        let offsets: serde_json::Value = serde_json::from_str(&offsets)?;
        for (id, expected) in [
            ("u1", [11.0, 11.0]),
            ("u2", [21.0, 21.0]),
            ("u3", [11.0, 51.0]),
            ("z1", [11.0, 11.0]),
            ("z2", [21.0, 21.0]),
            ("z3", [11.0, 51.0]),
            ("inner-in-zoom", [10.0, 11.0]),
            ("zoomed-inner", [0.0, 1.0]),
        ] {
            assert_eq!(numeric(&offsets[id][0], id), expected[0], "{offsets}");
            assert_eq!(numeric(&offsets[id][1], id), expected[1], "{offsets}");
        }
        assert_eq!(offsets["zoomed-inner"][4], "zoom-boundary");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSS zoom offset fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cssom_scroll_metrics_and_scroll_state_share_unzoomed_units() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/cssom-absolute-zoom-scroll.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.container{height:100px;width:100px;overflow:scroll}
.content{height:250px;width:250px}
#zoomed{zoom:4}
</style>`;
document.body.innerHTML = `
<div class=container id=plain><div class=content></div></div>
<div class=container id=zoomed><div class=content></div></div>
<div class=container id=zoomed-content><div class=content style="zoom:2"></div></div>
<div style="zoom:2"><div class=container id=nested><div class=content></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1200, 1400, 1.0))?
            .expect("CSS zoom initial scroll layout");

        let initial = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['plain','zoomed','zoomed-content','nested'].map(id=>{const e=document.getElementById(id);return [id,[e.clientWidth,e.clientHeight,e.scrollWidth,e.scrollHeight]]})))"#,
        )?;
        let initial: serde_json::Value = serde_json::from_str(&initial)?;
        for id in ["plain", "zoomed", "nested"] {
            assert_eq!(numeric(&initial[id][0], id), 100.0, "{initial}");
            assert_eq!(numeric(&initial[id][1], id), 100.0, "{initial}");
            assert_eq!(numeric(&initial[id][2], id), 250.0, "{initial}");
            assert_eq!(numeric(&initial[id][3], id), 250.0, "{initial}");
        }
        assert_eq!(numeric(&initial["zoomed-content"][2], "zoomed content"), 500.0);
        assert_eq!(numeric(&initial["zoomed-content"][3], "zoomed content"), 500.0);

        page_vm.vm_mut().eval(
            "document.getElementById('plain').scrollTo(125,125);document.getElementById('zoomed').scrollTo(125,125);'scrolled'",
        )?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1200, 1400, 1.0))?
            .expect("CSS zoom scrolled layout");
        let offsets = page_vm.vm_mut().eval(
            "JSON.stringify(['plain','zoomed'].map(id=>{const e=document.getElementById(id);return [e.scrollLeft,e.scrollTop]}))",
        )?;
        let offsets: serde_json::Value = serde_json::from_str(&offsets)?;
        assert_eq!(offsets, serde_json::json!([[125, 125], [125, 125]]));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSS zoom scroll fixture should run");
}
