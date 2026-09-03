use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_flex_content_basis_block_indefiniteness() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-content-basis-definiteness.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.column{display:flex;flex-direction:column}
.item{min-height:0}
.percentage{width:50px;height:100%}
</style>`;
document.body.innerHTML = `
<div class=column><div id=t1 class=item style="flex:1 1 content;height:100px"><div class=percentage></div></div></div>
<div class=column><div id=t2 class=item style="flex:1 1 auto;height:100px"><div class=percentage></div></div></div>
<div style="display:flex"><div id=t3 class=item style="flex:1 1 content;height:100px"><div class=percentage></div></div></div>
<div class=column><div id=t4 class=item style="flex:1 1 content;height:200px"><div style="width:50px;height:50px"></div><div style="width:50px;height:50%"></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 600, 1.0))?
            .expect("flex content-basis percentage screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['t1','t2','t3','t4'].map(id=>[id,document.getElementById(id).lastElementChild.offsetHeight])))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({"t1": 0, "t2": 100, "t3": 100, "t4": 0}),
            "only an initially definite flex-item block size may resolve descendant percentages",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex content-basis percentage fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_orthogonal_percentages_against_ratio_derived_block_size() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/ratio-derived-percentage-basis.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>html,body{margin:0}</style>`;
document.body.innerHTML = `<div id=container style="width:100px;aspect-ratio:1/1"><div id=child style="width:100%;height:100%;writing-mode:vertical-lr"></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("ratio-derived percentage-basis screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['container','child'].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "container": [100, 100],
                "child": [100, 100],
            }),
            "a block size made definite through aspect-ratio must be the percentage basis of an orthogonal child",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ratio-derived percentage-basis fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_absolute_calc_terms_in_intrinsic_flex_margins() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-calc-margin.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.probe{display:flex;height:20px}
#mixed,#percentage,#length{width:min-content}
#mixed>i{margin-left:calc(10% + 100px)}
#percentage>i{margin-left:10%}
#length>i{margin-left:100px}
#definite{width:200px}
#definite>i{width:1px;height:1px;margin-left:calc(10% + 100px)}
</style>`;
document.body.innerHTML = `
  <div id=mixed class=probe><i></i></div>
  <div id=percentage class=probe><i></i></div>
  <div id=length class=probe><i></i></div>
  <div id=definite class=probe><i id=definite-child></i></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 100, 1.0))?
            .expect("intrinsic percentage-resolution screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const rect=id=>document.getElementById(id).getBoundingClientRect();const definite=rect('definite'),child=rect('definite-child');return {mixed:rect('mixed').width,percentage:rect('percentage').width,length:rect('length').width,definite:definite.width,definiteChildOffset:child.x-definite.x}})())"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (name, expected) in [
            ("mixed", 100.0),
            ("percentage", 0.0),
            ("length", 100.0),
            ("definite", 200.0),
            ("definiteChildOffset", 120.0),
        ] {
            let actual = geometry[name]
                .as_f64()
                .unwrap_or_else(|| panic!("missing numeric {name}: {geometry}"))
                as f32;
            assert!(
                (actual - expected).abs() <= 0.05,
                "{name}: expected {expected}, got {actual}; geometry={geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic calc-margin fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_percentage_max_height_indefinite_during_grid_intrinsic_measurement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-intrinsic-percentage-max-height.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.case{display:grid;width:180px;height:200px;margin-bottom:20px}
.sidebar{display:flex;flex:1;flex-direction:row;height:100%}
.controlled{display:flex;overflow:auto}
.wrapper{display:grid;grid-template-columns:100px;grid-template-rows:50px 1fr;overflow:hidden}
.contents{grid-row:2/3;overflow:auto;max-height:100%}
.contents>div{height:400px}
#without-max .contents{max-height:none}
#definite-wrapper .wrapper{height:200px}
#auto-app{height:auto}
</style>`;
document.body.innerHTML = `
<div class=case id=original><div class=sidebar><div class=controlled><div class=wrapper><div></div><div class=contents><div></div></div></div></div></div></div>
<div class=case id=without-max><div class=sidebar><div class=controlled><div class=wrapper><div></div><div class=contents><div></div></div></div></div></div></div>
<div class=case id=definite-wrapper><div class=sidebar><div class=controlled><div class=wrapper><div></div><div class=contents><div></div></div></div></div></div></div>
<div class=case id=auto-app><div class=sidebar><div class=controlled><div class=wrapper><div></div><div class=contents><div></div></div></div></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 1_200, 1.0))?
            .expect("intrinsic percentage max-height screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['original','without-max','definite-wrapper','auto-app'].map(id=>{const c=document.getElementById(id),height=e=>e.getBoundingClientRect().height;return [id,{case:height(c),sidebar:height(c.querySelector('.sidebar')),controlled:height(c.querySelector('.controlled')),wrapper:height(c.querySelector('.wrapper')),contents:height(c.querySelector('.contents'))}]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "original": {"case": 200, "sidebar": 450, "controlled": 450, "wrapper": 450, "contents": 400},
                "without-max": {"case": 200, "sidebar": 450, "controlled": 450, "wrapper": 450, "contents": 400},
                "definite-wrapper": {"case": 200, "sidebar": 200, "controlled": 200, "wrapper": 200, "contents": 150},
                "auto-app": {"case": 450, "sidebar": 450, "controlled": 450, "wrapper": 450, "contents": 400},
            }),
            "an unresolved percentage max-height must not cap an intrinsic contribution, but must resolve once its containing block is definite",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic percentage max-height fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_reflows_an_orthogonal_grid_item_against_the_final_block_size() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-orthogonal-dynamic.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>html,body{margin:0}</style>`;
document.body.innerHTML = `<div id=target style="display:inline-grid;background:green"><div id=item style="writing-mode:vertical-lr;line-height:0"><span id=first style="display:inline-block;height:100px;width:50px"></span><span id=second style="display:inline-block;height:50px;width:50px"></span></div></div>`;
document.body.offsetTop;
document.getElementById('target').style.height = '100px';
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("orthogonal grid screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify({...Object.fromEntries(['target','item','first','second'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,{x:r.x,y:r.y,width:r.width,height:r.height}]})),columns:getComputedStyle(target).gridTemplateColumns,rows:getComputedStyle(target).gridTemplateRows})"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "target": {"x": 0, "y": 0, "width": 100, "height": 100},
                "item": {"x": 0, "y": 0, "width": 100, "height": 100},
                "first": {"x": 0, "y": 0, "width": 50, "height": 100},
                "second": {"x": 50, "y": 0, "width": 50, "height": 50},
                "columns": "100px",
                "rows": "100px",
            }),
            "an orthogonal grid item must recompute its block contribution from the final inline grid area",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("orthogonal grid dynamic fixture should run");
}
