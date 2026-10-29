// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn positioned_layout_matches_chromium_auto_margin_and_relative_inset_rules() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/positioned-layout.html")?,
        );
        page_vm
            .vm_mut()
            .set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 1440,
                inner_height: 620,
                outer_width: 1440,
                outer_height: 620,
                device_pixel_ratio: 1.0,
                screen_width: 1440,
                screen_height: 620,
                screen_avail_width: 1440,
                screen_avail_height: 620,

..Default::default()
}))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;font-size:16px;background:white}
#centered{position:fixed;left:0;right:0;top:0;width:975px;height:20px;margin-left:auto;margin-right:auto;background:red}
#definite-parent{position:absolute;left:0;top:30px;width:100px;height:400px}
#definite-child{position:relative;top:calc(max(120px,100% - 12.6875rem));width:10px;height:10px;background:blue}
#indefinite-parent{position:absolute;left:0;top:500px;width:100px;min-height:100px}
#indefinite-child{position:relative;top:calc(10px + 10%);width:10px;height:100px;background:lime}
</style>`;
document.body.innerHTML = `<div id=centered></div><div id=definite-parent><div id=definite-child></div></div><div id=indefinite-parent><div id=indefinite-child></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['centered','definite-parent','definite-child','indefinite-parent','indefinite-child'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            // Chromium retains the half-pixel origin in DOM geometry.
            ("centered", [232.5, 0.0, 975.0, 20.0]),
            ("definite-parent", [0.0, 30.0, 100.0, 400.0]),
            ("definite-child", [0.0, 227.0, 10.0, 10.0]),
            ("indefinite-parent", [0.0, 500.0, 100.0, 100.0]),
            ("indefinite-child", [0.0, 500.0, 10.0, 100.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1440, 620, 1.0))?
            .expect("positioned fixture must retain a layout root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        // Blink retains the fractional layout origin for DOM geometry, but
        // box background painting snaps the fill rect to device pixels.
        // The 975px box therefore paints [233, 1208), with no half-covered
        // pixel at either edge at device scale 1.
        assert_eq!(pixel(232, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(233, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(1207, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(1208, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(5, 230), [0, 0, 255, 255]);
        assert_eq!(pixel(5, 505), [0, 255, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("positioned layout fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn containment_matches_chromium_containing_block_eligibility_and_paint_clip() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/containment-layout.html")?,
        );
        page_vm
            .vm_mut()
            .set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 800,
                inner_height: 600,
                outer_width: 800,
                outer_height: 600,
                device_pixel_ratio: 1.0,
                screen_width: 800,
                screen_height: 600,
                screen_avail_width: 800,
                screen_avail_height: 600,

..Default::default()
}))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
#stage{position:relative;width:800px;height:600px}
.cb{position:absolute;top:20px;box-sizing:border-box;width:160px;height:100px;border:4px solid black;padding:6px;background:rgb(230,230,230)}
#layout{left:20px;contain:layout}#paint{left:220px;contain:paint}#content{left:420px;contain:content}#strict{left:620px;contain:strict}
.abs{position:absolute;right:10px;top:8px;width:20px;height:16px;background:red}.fixed{position:fixed;left:12px;top:10px;width:18px;height:14px;background:blue}
.overflow{position:absolute;left:-20px;bottom:-20px;width:40px;height:40px;background:lime}
#shadow-host{position:absolute;left:100px;top:180px;width:300px;height:100px;background:rgb(240,240,240)}
#inline-outer{position:absolute;left:500px;top:180px;width:250px;height:100px;background:rgb(240,240,240)}
#inline-container{contain:paint}#inline-abs{position:absolute;right:5px;top:6px;width:20px;height:15px;background:purple}
#will-contain{position:absolute;left:20px;top:320px;width:140px;height:80px;will-change:contain;background:silver}
#will-position{position:absolute;left:180px;top:320px;width:140px;height:80px;will-change:position;background:silver}
#will-transform{position:absolute;left:340px;top:320px;width:140px;height:80px;will-change:transform;background:silver}
#content-auto{position:absolute;left:380px;top:500px;width:140px;height:80px;content-visibility:auto;background:silver}
.will-abs{position:absolute;right:7px;top:8px;width:20px;height:15px}.will-fixed{position:fixed;left:9px;top:10px;width:18px;height:14px}
#bfc-row{position:absolute;left:20px;top:430px;width:500px}.bfc{display:block;position:relative;width:100px;background:yellow}
.bfc.plain{left:0}.bfc.layout{left:120px;contain:layout}.bfc.paint{left:220px;contain:paint}.float{float:left;width:20px;height:30px;background:red}
#table{position:absolute;left:560px;top:320px;width:220px;height:100px}#table-row{contain:paint}#table-cell-contained{contain:paint}
.table-abs{position:absolute;right:3px;top:4px;width:10px;height:10px}
</style>`;
document.body.innerHTML = `<div id=stage>
<div id=layout class=cb><div id=layout-abs class=abs></div><div id=layout-fixed class=fixed></div><div class=overflow></div></div>
<div id=paint class=cb><div id=paint-abs class=abs></div><div id=paint-fixed class=fixed></div><div class=overflow></div></div>
<div id=content class=cb><div id=content-abs class=abs></div><div id=content-fixed class=fixed></div><div class=overflow></div></div>
<div id=strict class=cb><div id=strict-abs class=abs></div><div id=strict-fixed class=fixed></div><div class=overflow></div></div>
<div id=shadow-host></div><div id=inline-outer><span id=inline-container>inline<div id=inline-abs></div></span></div>
<div id=will-contain><div id=will-contain-abs class=will-abs></div><div id=will-contain-fixed class=will-fixed></div></div>
<div id=will-position><div id=will-position-abs class=will-abs></div><div id=will-position-fixed class=will-fixed></div></div>
<div id=will-transform><div id=will-transform-abs class=will-abs></div><div id=will-transform-fixed class=will-fixed></div></div>
<div id=content-auto><div id=content-auto-abs class=will-abs></div><div id=content-auto-fixed class=will-fixed></div></div>
<div id=bfc-row><div id=plain-bfc class='bfc plain'><div class=float></div></div><div id=layout-bfc class='bfc layout'><div class=float></div></div><div id=paint-bfc class='bfc paint'><div class=float></div></div></div>
<table id=table><tbody><tr id=table-row><td><div id=row-abs class=table-abs></div></td></tr><tr><td id=table-cell-contained><div id=cell-abs class=table-abs></div></td></tr></tbody></table>
</div>`;
const shadow=document.getElementById('shadow-host').attachShadow({mode:'open'});
shadow.innerHTML=`<style>.root{contain:content;margin-left:40px;width:200px;height:100px;background:orange}.action{position:absolute;right:8px;top:9px;width:30px;height:20px;background:black}</style><div id=root class=root><div id=shadow-action class=action></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let result = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const shadow=document.getElementById('shadow-host').shadowRoot;
const ids=['layout','layout-abs','layout-fixed','paint','paint-abs','paint-fixed','content','content-abs','content-fixed','strict','strict-abs','strict-fixed','inline-outer','inline-container','inline-abs','will-contain','will-contain-abs','will-contain-fixed','will-position','will-position-abs','will-position-fixed','will-transform','will-transform-abs','will-transform-fixed','content-auto','content-auto-abs','content-auto-fixed','plain-bfc','layout-bfc','paint-bfc'];
const rect=e=>{const r=e.getBoundingClientRect();return [r.x,r.y,r.width,r.height]};
const geometry=Object.fromEntries(ids.map(id=>[id,rect(document.getElementById(id))]));
geometry['shadow-host']=rect(document.getElementById('shadow-host'));geometry['shadow-root']=rect(shadow.getElementById('root'));geometry['shadow-action']=rect(shadow.getElementById('shadow-action'));
const offsets={};for(const id of ['layout-abs','layout-fixed','paint-abs','paint-fixed','content-abs','content-fixed','strict-abs','strict-fixed','inline-abs','will-contain-abs','will-contain-fixed','will-position-abs','will-position-fixed','will-transform-abs','will-transform-fixed','content-auto-abs','content-auto-fixed','row-abs','cell-abs']) offsets[id]=document.getElementById(id).offsetParent?.id??null;
offsets['shadow-action']=shadow.getElementById('shadow-action').offsetParent?.id??null;return {geometry,offsets}})())"#,
        )?;
        let result: serde_json::Value = serde_json::from_str(&result)?;
        let geometry = &result["geometry"];
        for (id, expected) in [
            ("layout", [20.0, 20.0, 160.0, 100.0]),
            ("layout-abs", [146.0, 32.0, 20.0, 16.0]),
            ("layout-fixed", [36.0, 34.0, 18.0, 14.0]),
            ("paint-abs", [346.0, 32.0, 20.0, 16.0]),
            ("paint-fixed", [236.0, 34.0, 18.0, 14.0]),
            ("content-abs", [546.0, 32.0, 20.0, 16.0]),
            ("content-fixed", [436.0, 34.0, 18.0, 14.0]),
            ("strict-abs", [746.0, 32.0, 20.0, 16.0]),
            ("strict-fixed", [636.0, 34.0, 18.0, 14.0]),
            ("inline-abs", [725.0, 186.0, 20.0, 15.0]),
            ("shadow-root", [140.0, 180.0, 200.0, 100.0]),
            ("shadow-action", [302.0, 189.0, 30.0, 20.0]),
            ("will-contain-abs", [133.0, 328.0, 20.0, 15.0]),
            ("will-contain-fixed", [29.0, 330.0, 18.0, 14.0]),
            ("will-position-abs", [293.0, 328.0, 20.0, 15.0]),
            ("will-position-fixed", [9.0, 10.0, 18.0, 14.0]),
            ("will-transform-abs", [453.0, 328.0, 20.0, 15.0]),
            ("will-transform-fixed", [349.0, 330.0, 18.0, 14.0]),
            ("content-auto", [380.0, 500.0, 140.0, 80.0]),
            ("content-auto-abs", [493.0, 508.0, 20.0, 15.0]),
            ("content-auto-fixed", [389.0, 510.0, 18.0, 14.0]),
            ("plain-bfc", [20.0, 430.0, 100.0, 0.0]),
            ("layout-bfc", [160.0, 430.0, 100.0, 30.0]),
            ("paint-bfc", [240.0, 460.0, 100.0, 30.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {result}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; result={result}"
                );
            }
        }
        for (id, expected) in [
            ("layout-abs", Some("layout")),
            ("layout-fixed", Some("layout")),
            ("paint-abs", Some("paint")),
            ("paint-fixed", Some("paint")),
            ("content-abs", Some("content")),
            ("content-fixed", Some("content")),
            ("strict-abs", Some("strict")),
            ("strict-fixed", Some("strict")),
            ("inline-abs", Some("inline-outer")),
            ("will-contain-abs", Some("will-contain")),
            ("will-contain-fixed", Some("will-contain")),
            ("will-position-abs", Some("will-position")),
            ("will-position-fixed", None),
            ("will-transform-abs", Some("will-transform")),
            ("will-transform-fixed", Some("will-transform")),
            ("content-auto-abs", Some("content-auto")),
            ("content-auto-fixed", Some("content-auto")),
            ("row-abs", Some("table")),
            ("cell-abs", Some("table-cell-contained")),
            ("shadow-action", Some("root")),
        ] {
            assert_eq!(
                result["offsets"][id].as_str(),
                expected,
                "unexpected offsetParent for {id}: {result}"
            );
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("containment fixture must retain a layout root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(pixel(10, 105), [0, 255, 0, 255]);
        assert_eq!(pixel(210, 105), [255, 255, 255, 255]);
        assert_eq!(pixel(230, 105), [0, 255, 0, 255]);
        assert_eq!(pixel(410, 105), [255, 255, 255, 255]);
        assert_eq!(pixel(430, 105), [0, 255, 0, 255]);
        assert_eq!(pixel(610, 105), [255, 255, 255, 255]);
        assert_eq!(pixel(630, 105), [0, 255, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("containment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn filters_establish_containing_blocks_except_on_the_document_element() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/filter-containing-blocks.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html{filter:opacity(99%);will-change:backdrop-filter}
html,body{margin:0;padding:0}
.case{margin-left:100px;width:160px;height:40px}
#filter{filter:opacity(80%)}
#backdrop{backdrop-filter:blur(0px)}
#will-filter{will-change:filter}
#will-backdrop{will-change:backdrop-filter}
.absolute{position:absolute;left:11px;top:12px;width:10px;height:10px}
.fixed{position:fixed;left:13px;top:14px;width:10px;height:10px}
#inline-owner{margin-left:100px;height:40px}
#inline-filter{filter:opacity(80%)}
#inline-absolute{position:absolute;left:17px;top:18px;width:10px;height:10px}
#root-fixed{position:fixed;left:7px;top:8px;width:10px;height:10px}
</style>`;
document.body.innerHTML = `
<div id=filter class=case><span id=filter-absolute class=absolute></span><span id=filter-fixed class=fixed></span></div>
<div id=backdrop class=case><span id=backdrop-absolute class=absolute></span><span id=backdrop-fixed class=fixed></span></div>
<div id=will-filter class=case><span id=will-filter-absolute class=absolute></span><span id=will-filter-fixed class=fixed></span></div>
<div id=will-backdrop class=case><span id=will-backdrop-absolute class=absolute></span><span id=will-backdrop-fixed class=fixed></span></div>
<div id=inline-owner>prefix <span id=inline-filter>inline<span id=inline-absolute></span></span></div>
<span id=root-fixed></span>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        // Geometry queries intentionally consume the last published layout.
        // Publish this mutation through the rendering lifecycle first.
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("filter fixture must retain a layout root");

        let result = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{
const ids=['filter','backdrop','will-filter','will-backdrop','inline-filter','root-fixed'];
const rect=id=>{const r=document.getElementById(id).getBoundingClientRect();return [r.x,r.y]};
const relative={};
for(const id of ['filter','backdrop','will-filter','will-backdrop']){
  const parent=rect(id);
  for(const suffix of ['absolute','fixed']){
    const child=rect(`${id}-${suffix}`);
    relative[`${id}-${suffix}`]=[child[0]-parent[0],child[1]-parent[1]];
  }
}
const inline=rect('inline-filter');const inlineChild=rect('inline-absolute');
relative['inline-absolute']=[inlineChild[0]-inline[0],inlineChild[1]-inline[1]];
const offsetParents={};
for(const id of Object.keys(relative)) offsetParents[id]=document.getElementById(id).offsetParent?.id??null;
offsetParents['root-fixed']=document.getElementById('root-fixed').offsetParent?.id??null;
return {relative,offsetParents,rootFixed:rect('root-fixed')};
})())"#,
        )?;
        let result: serde_json::Value = serde_json::from_str(&result)?;
        for id in ["filter", "backdrop", "will-filter", "will-backdrop"] {
            for (suffix, expected) in [("absolute", [11.0, 12.0]), ("fixed", [13.0, 14.0])] {
                let child = format!("{id}-{suffix}");
                let actual = result["relative"][&child]
                    .as_array()
                    .unwrap_or_else(|| panic!("missing relative geometry for {child}: {result}"));
                for (axis, expected) in expected.into_iter().enumerate() {
                    let actual = actual[axis].as_f64().expect("numeric geometry") as f32;
                    assert!(
                        (actual - expected).abs() <= 0.05,
                        "{child}[{axis}]: expected {expected}, got {actual}; result={result}"
                    );
                }
                assert_eq!(
                    result["offsetParents"][&child].as_str(),
                    Some(id),
                    "unexpected offsetParent for {child}: {result}"
                );
            }
        }
        for (id, expected) in [
            ("inline-absolute", [17.0, 18.0]),
            ("root-fixed", [7.0, 8.0]),
        ] {
            let source = if id == "root-fixed" {
                &result["rootFixed"]
            } else {
                &result["relative"][id]
            };
            let actual = source
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {result}"));
            for (axis, expected) in expected.into_iter().enumerate() {
                let actual = actual[axis].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{axis}]: expected {expected}, got {actual}; result={result}"
                );
            }
        }
        assert_eq!(
            result["offsetParents"]["inline-absolute"].as_str(),
            Some("inline-filter"),
            "filter must apply before Blink's IsBox gate: {result}"
        );
        assert_eq!(
            result["offsetParents"]["root-fixed"].as_str(),
            None,
            "the document element is excluded by Filter Effects: {result}"
        );

        // The same filtered element can also be the root of a one-shot
        // subtree source. It is still an ordinary element in its owner
        // document, so occupying LayoutWorld's root slot must not grant the
        // document-element exception.
        let subtree_root = page_vm
            .vm()
            .element_handle_by_id_for_test("filter")
            .expect("filtered subtree root");
        let subtree_children = [
            (
                page_vm
                    .vm()
                    .element_handle_by_id_for_test("filter-absolute")
                    .expect("absolute subtree child"),
                [11.0, 12.0],
            ),
            (
                page_vm
                    .vm()
                    .element_handle_by_id_for_test("filter-fixed")
                    .expect("fixed subtree child"),
                [13.0, 14.0],
            ),
        ];
        let subtree_pass = page_vm.vm().build_layout_pass_for_subtree_for_test(
            subtree_root,
            moli_layout::LayoutPassRequest::new(
                moli_layout::LayoutViewport::new(800, 600, 1.0),
                moli_layout::LayoutFlushReason::Test,
            ),
        )?;
        let subtree_answers = subtree_pass.answer_queries(&moli_layout::LayoutQueryBatch::new(
            subtree_children
                .iter()
                .map(|(source, _)| moli_layout::LayoutQuery::ElementMetrics { source: *source })
                .collect(),
        ));
        for ((source, expected), answer) in subtree_children
            .into_iter()
            .zip(subtree_answers.answers)
        {
            let moli_layout::LayoutQueryAnswer::ElementMetrics(Some(metrics)) = answer else {
                panic!("missing subtree metrics for {source:?}");
            };
            assert_eq!(
                metrics.offset_parent,
                Some(subtree_root),
                "a filtered subtree root must remain an ordinary containing block"
            );
            for (actual, expected) in [metrics.offset_position.x, metrics.offset_position.y]
                .into_iter()
                .zip(expected)
            {
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "subtree child {source:?}: expected offset {expected}, got {actual}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("filter containing-block fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn fixed_flex_auto_margin_consumes_free_space_once() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/fixed-flex-layout.html")?,
        );
        page_vm
            .vm_mut()
            .set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 1440,
                inner_height: 900,
                outer_width: 1440,
                outer_height: 900,
                device_pixel_ratio: 1.0,
                screen_width: 1440,
                screen_height: 900,
                screen_avail_width: 1440,
                screen_avail_height: 900,

..Default::default()
}))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#stage{position:relative;width:80%;height:900px;margin:auto}
#header{display:flex;position:fixed;left:0;box-sizing:border-box;justify-content:space-between;width:100%;min-width:768px;height:5vh;margin:10px 0;padding:0 24px}
#left{width:596px;height:45px;margin-right:auto;background:red}
#right{width:308px;height:45px;background:blue}
</style>`;
document.body.innerHTML = `<main id=stage><header id=header><div id=left></div><div id=right></div></header></main>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['stage','header','left','right'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("stage", [144.0, 0.0, 1152.0, 900.0]),
            ("header", [0.0, 10.0, 1440.0, 45.0]),
            ("left", [24.0, 10.0, 596.0, 45.0]),
            ("right", [1108.0, 10.0, 308.0, 45.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("fixed flex auto-margin fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn atomic_inline_auto_width_shrink_wraps_before_parent_text_alignment() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-fit-content.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.line{height:40px;text-align:right;font-size:0;background:rgb(1,2,3)}
#wide{width:1236px;margin-left:112px}
#narrow{width:200px;margin-left:20px}
#flex,#grid,#specified,#margined,#table{width:400px}
.card{display:inline-block;height:40px;text-align:left;background:rgb(4,5,6)}
.headline{height:40px}
.primary,.secondary{display:inline-block;height:40px}
.primary{width:162px;background:rgb(7,8,9)}
.secondary{width:84px;margin-left:8px;background:rgb(10,11,12)}
.pair-a,.pair-b{width:100px;height:40px;background:rgb(13,14,15)}
.pair-b{width:50px;background:rgb(16,17,18)}
#flex-card{display:inline-flex;column-gap:10px}
#grid-card{display:inline-grid;grid-template-columns:100px 50px;column-gap:10px}
#specified-card{width:300px}
#margined-card{margin-left:10px;margin-right:20px}
#table-card{display:inline-table;border-collapse:separate;border-spacing:0}
#table-card td{height:40px;padding:0}
#table-a{width:100px}#table-b{width:50px}
</style>`;
document.body.innerHTML = `
<div id=wide class=line><div id=wide-card class=card><div class=headline><span id=wide-primary class=primary></span> <span id=wide-secondary class=secondary></span></div></div></div>
<div id=narrow class=line><div id=narrow-card class=card><div class=headline><span id=narrow-primary class=primary></span> <span id=narrow-secondary class=secondary></span></div></div></div>
<div id=flex class=line><div id=flex-card><div class=pair-a></div><div class=pair-b></div></div></div>
<div id=grid class=line><div id=grid-card><div class=pair-a></div><div class=pair-b></div></div></div>
<div id=specified class=line><div id=specified-card class=card><div class=headline><span class=primary></span> <span class=secondary></span></div></div></div>
<div id=margined class=line><div id=margined-card class=card><div class=headline><span class=primary></span> <span class=secondary></span></div></div></div>
<div id=table class=line><table id=table-card><tr><td id=table-a></td><td id=table-b></td></tr></table></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['wide','wide-card','wide-primary','wide-secondary','narrow','narrow-card','narrow-primary','narrow-secondary','flex','flex-card','grid','grid-card','specified','specified-card','margined','margined-card','table','table-card','table-a','table-b'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("wide", [112.0, 0.0, 1236.0, 40.0]),
            ("wide-card", [1094.0, 0.0, 254.0, 40.0]),
            ("wide-primary", [1094.0, 0.0, 162.0, 40.0]),
            ("wide-secondary", [1264.0, 0.0, 84.0, 40.0]),
            ("narrow", [20.0, 40.0, 200.0, 40.0]),
            ("narrow-card", [20.0, 40.0, 200.0, 40.0]),
            ("narrow-primary", [20.0, 40.0, 162.0, 40.0]),
            ("narrow-secondary", [28.0, 80.0, 84.0, 40.0]),
            ("flex", [0.0, 80.0, 400.0, 40.0]),
            ("flex-card", [240.0, 80.0, 160.0, 40.0]),
            ("grid", [0.0, 120.0, 400.0, 40.0]),
            ("grid-card", [240.0, 120.0, 160.0, 40.0]),
            ("specified", [0.0, 160.0, 400.0, 40.0]),
            ("specified-card", [100.0, 160.0, 300.0, 40.0]),
            ("margined", [0.0, 200.0, 400.0, 40.0]),
            ("margined-card", [126.0, 200.0, 254.0, 40.0]),
            ("table", [0.0, 240.0, 400.0, 40.0]),
            ("table-card", [250.0, 240.0, 150.0, 40.0]),
            ("table-a", [250.0, 240.0, 100.0, 40.0]),
            ("table-b", [350.0, 240.0, 50.0, 40.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic inline fit-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn center_user_agent_alignment_centers_atomic_inline_children() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/center-user-agent-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#host{width:584px;margin-left:100px;font-size:0}
#first,#second{display:inline-block;height:20px}
#first{width:120px}
#second{width:100px}
</style>`;
document.body.innerHTML = `<center id=host><span id=first></span><span id=second></span></center>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['host','first','second'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("host", [100.0, 0.0, 584.0, 20.0]),
            ("first", [282.0, 0.0, 120.0, 20.0]),
            ("second", [402.0, 0.0, 100.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("center user-agent alignment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn absolute_auto_width_from_an_inline_formatting_context_shrinks_to_fit() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/absolute-fit-content.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#stage{position:relative;width:800px;height:600px}
.case{position:absolute;height:70px;font-size:0}
.prefix{display:inline-block;width:30px;height:10px}
.abs{position:absolute;top:5px}
.primary,.secondary{display:inline-block;height:20px}
.primary{width:160px}
.secondary{width:80px;margin-left:8px}
#left-max-case{left:20px;top:20px;width:300px}
#left-limit-case{left:380px;top:20px;width:200px}
#left-min-case{left:640px;top:20px;width:120px}
#right-max-case{left:20px;top:110px;width:300px}
#stretch-case{left:380px;top:110px;width:300px}
#margin-min-case{left:20px;top:200px;width:200px}
#max-clamp-case{left:260px;top:200px;width:300px}
#min-clamp-case{left:20px;top:290px;width:300px}
#specified-case{left:380px;top:290px;width:300px}
#static-ltr-case{left:20px;top:380px;width:300px}
#static-rtl-case{left:380px;top:380px;width:300px;direction:rtl;text-align:left}
#flex-case{display:flex;left:20px;top:470px;width:300px}
#left-max{left:20px}
#left-limit{left:20px}
#left-min{left:10px}
#right-max{right:20px}
#stretch{left:20px;right:30px}
#margin-min{left:20px;margin-left:10px;margin-right:15px}
#max-clamp{left:20px;max-width:200px}
#min-clamp{left:20px;min-width:260px}
#specified{left:20px;width:120px}
#flex-abs{left:20px}
</style>`;
document.body.innerHTML = `<div id=stage>
  <div id=left-max-case class=case><span class=prefix></span><div id=left-max class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=left-limit-case class=case><span class=prefix></span><div id=left-limit class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=left-min-case class=case><span class=prefix></span><div id=left-min class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=right-max-case class=case><span class=prefix></span><div id=right-max class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=stretch-case class=case><span class=prefix></span><div id=stretch class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=margin-min-case class=case><span class=prefix></span><div id=margin-min class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=max-clamp-case class=case><span class=prefix></span><div id=max-clamp class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=min-clamp-case class=case><span class=prefix></span><div id=min-clamp class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=specified-case class=case><span class=prefix></span><div id=specified class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
  <div id=static-ltr-case class=case><span class=prefix></span><span id=static-ltr class=abs><span class=primary></span> <span class=secondary></span></span></div>
  <div id=static-rtl-case class=case><span class=prefix></span><span id=static-rtl class=abs><span class=primary></span> <span class=secondary></span></span></div>
  <div id=flex-case class=case><span class=prefix></span><div id=flex-abs class=abs><div><span class=primary></span> <span class=secondary></span></div></div></div>
</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['left-max-case','left-max','left-limit-case','left-limit','left-min-case','left-min','right-max-case','right-max','stretch-case','stretch','margin-min-case','margin-min','max-clamp-case','max-clamp','min-clamp-case','min-clamp','specified-case','specified','static-ltr-case','static-ltr','static-rtl-case','static-rtl','flex-case','flex-abs'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("left-max-case", [20.0, 20.0, 300.0, 70.0]),
            ("left-max", [40.0, 25.0, 248.0, 20.0]),
            ("left-limit-case", [380.0, 20.0, 200.0, 70.0]),
            ("left-limit", [400.0, 25.0, 180.0, 40.0]),
            ("left-min-case", [640.0, 20.0, 120.0, 70.0]),
            ("left-min", [650.0, 25.0, 160.0, 40.0]),
            ("right-max", [52.0, 115.0, 248.0, 20.0]),
            ("stretch", [400.0, 115.0, 250.0, 20.0]),
            ("margin-min", [50.0, 205.0, 160.0, 40.0]),
            ("max-clamp", [280.0, 205.0, 200.0, 40.0]),
            ("min-clamp", [40.0, 295.0, 260.0, 20.0]),
            ("specified", [400.0, 295.0, 120.0, 40.0]),
            ("static-ltr", [50.0, 385.0, 248.0, 20.0]),
            ("flex-abs", [40.0, 475.0, 248.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        // Keep the RTL auto-inset branch in the sizing contract. Parley 0.10
        // does not bidi-reorder zero-width out-of-flow placeholders, so its
        // physical static-position x remains a separate inline-bidi gap; the
        // Chromium differential records the correct x without baking the
        // current approximation into this regression.
        let static_rtl = geometry["static-rtl"]
            .as_array()
            .unwrap_or_else(|| panic!("missing geometry for static-rtl: {geometry}"));
        for (index, expected) in [(2, 160.0), (3, 40.0)] {
            let actual = static_rtl[index].as_f64().expect("numeric geometry") as f32;
            assert!(
                (actual - expected).abs() <= 0.05,
                "static-rtl[{index}]: expected {expected}, got {actual}; geometry={geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("absolute fit-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn float_line_clearance_respects_pre_and_nowrap() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/float-nowrap.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.case{position:absolute;top:0;width:200px;height:120px;font:16px/20px monospace}
.float{float:left;width:60px;height:80px}
p{margin:0;width:200px}
#pre{left:0;white-space:pre}
#nowrap{left:240px;white-space:nowrap}
#wrap{left:480px;white-space:normal}
.atom{display:inline-block;width:240px;height:24px;vertical-align:top}
</style>`;
document.body.innerHTML = `<div class=case id=pre><div class=float></div><p>abcdefghijabcdefghij  <br>abcdefghijabcdefghij</p></div><div class=case id=nowrap><div class=float></div><p><span class=atom></span></p></div><div class=case id=wrap><div class=float></div><p><span class=atom></span></p></div>`;
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;
        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(['pre','nowrap','wrap'].map(id=>{
const c=document.getElementById(id), p=c.querySelector('p');
const r=c.getBoundingClientRect(), t=(p.querySelector('span')||p).getBoundingClientRect();
return [t.x-r.x,t.y-r.y,p.getBoundingClientRect().height];
}))"#,
        )?;
        let geometry: Vec<[f32; 3]> = serde_json::from_str(&geometry)?;
        // Chromium: pre/nowrap content stays beside the float and overflows;
        // a wrapping paragraph moves its oversized atom below the float.
        for (actual, expected) in geometry.iter().zip([
            [0.0, 0.0, 40.0],
            [60.0, 0.0, 24.0],
            [0.0, 80.0, 104.0],
        ]) {
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "expected {expected}, got {actual}; geometry={geometry:?}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("float nowrap fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn floated_auto_width_inline_formatting_contexts_shrink_to_fit() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/float-fit-content.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
#stage{position:relative;width:1200px;height:620px}
.case{position:absolute;height:70px;font-size:0}
.auto-float{float:left}
.right{float:right}
.primary,.secondary{display:inline-block;height:20px}
.primary{width:160px}
.secondary{width:80px;margin-left:8px}
#baidu-case{left:20px;top:20px;width:1076px}
#baidu-logo{float:left;margin-top:17px}
#baidu-logo img{display:inline;width:101px;height:33px}
#baidu-main{float:left;width:748px;height:45px;margin:15px 0 8px 18px}
#max-case{left:20px;top:120px;width:300px}
#limit-case{left:380px;top:120px;width:200px}
#min-case{left:640px;top:120px;width:120px}
#right-case{left:20px;top:220px;width:300px}
#margin-case{left:380px;top:220px;width:200px}
#max-clamp-case{left:640px;top:220px;width:300px}
#min-clamp-case{left:20px;top:320px;width:300px}
#specified-case{left:380px;top:320px;width:300px}
#edge-case{left:740px;top:320px;width:300px}
#block-control-case{left:20px;top:420px;width:300px}
#replaced-control-case{left:380px;top:420px;width:300px}
#stretch-control-case{left:740px;top:420px;width:300px}
#inline-margin-case{left:20px;top:520px;width:200px}
#negative-margin-case{left:380px;top:520px;width:200px}
#inline-negative-margin-case{left:740px;top:520px;width:200px}
#margin-float{margin-left:10px;margin-right:15px}
#inline-margin-float{margin-left:10px;margin-right:15px}
#negative-margin-float,#inline-negative-margin-float{margin-left:-10px;margin-right:-15px}
#max-clamp{max-width:200px}
#min-clamp{min-width:260px}
#specified{width:120px}
#edge{margin-left:10px;margin-right:15px;padding:0 10px;border:2px solid black}
#block-control{float:left}
#block-control>div{width:180px;height:20px}
#replaced-control{float:left;width:101px;height:33px}
</style>`;
document.body.innerHTML = `<div id=stage>
  <div id=baidu-case class=case><a id=baidu-logo><img id=baidu-logo-image width=101 height=33 alt=""></a><div id=baidu-main></div></div>
  <div id=max-case class=case><div id=max class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=limit-case class=case><div id=limit class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=min-case class=case><div id=min class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=right-case class=case><div id=right class="auto-float right"><span class=primary></span> <span class=secondary></span></div></div>
  <div id=margin-case class=case><div id=margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=max-clamp-case class=case><div id=max-clamp class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=min-clamp-case class=case><div id=min-clamp class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=specified-case class=case><div id=specified class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=edge-case class=case><div id=edge class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=block-control-case class=case><div id=block-control><div></div></div></div>
  <div id=replaced-control-case class=case><img id=replaced-control width=101 height=33 alt=""></div>
  <div id=stretch-control-case class=case><div id=stretch-control><span class=primary></span> <span class=secondary></span></div></div>
  <div id=inline-margin-case class=case><span></span><div id=inline-margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=negative-margin-case class=case><div id=negative-margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
  <div id=inline-negative-margin-case class=case><span></span><div id=inline-negative-margin-float class=auto-float><span class=primary></span> <span class=secondary></span></div></div>
</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['baidu-case','baidu-logo','baidu-logo-image','baidu-main','max-case','max','limit-case','limit','min-case','min','right-case','right','margin-case','margin-float','max-clamp-case','max-clamp','min-clamp-case','min-clamp','specified-case','specified','edge-case','edge','block-control-case','block-control','replaced-control-case','replaced-control','stretch-control-case','stretch-control','inline-margin-case','inline-margin-float','negative-margin-case','negative-margin-float','inline-negative-margin-case','inline-negative-margin-float'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("baidu-case", [20.0, 20.0, 1076.0, 70.0]),
            ("baidu-logo", [20.0, 37.0, 101.0, 33.0]),
            ("baidu-logo-image", [20.0, 37.0, 101.0, 33.0]),
            ("baidu-main", [139.0, 35.0, 748.0, 45.0]),
            ("max-case", [20.0, 120.0, 300.0, 70.0]),
            ("max", [20.0, 120.0, 248.0, 20.0]),
            ("limit-case", [380.0, 120.0, 200.0, 70.0]),
            ("limit", [380.0, 120.0, 200.0, 40.0]),
            ("min-case", [640.0, 120.0, 120.0, 70.0]),
            ("min", [640.0, 120.0, 160.0, 40.0]),
            ("right-case", [20.0, 220.0, 300.0, 70.0]),
            ("right", [72.0, 220.0, 248.0, 20.0]),
            ("margin-case", [380.0, 220.0, 200.0, 70.0]),
            ("margin-float", [390.0, 220.0, 175.0, 40.0]),
            ("max-clamp-case", [640.0, 220.0, 300.0, 70.0]),
            ("max-clamp", [640.0, 220.0, 200.0, 40.0]),
            ("min-clamp-case", [20.0, 320.0, 300.0, 70.0]),
            ("min-clamp", [20.0, 320.0, 260.0, 20.0]),
            ("specified-case", [380.0, 320.0, 300.0, 70.0]),
            ("specified", [380.0, 320.0, 120.0, 40.0]),
            ("edge-case", [740.0, 320.0, 300.0, 70.0]),
            ("edge", [750.0, 320.0, 272.0, 24.0]),
            ("block-control-case", [20.0, 420.0, 300.0, 70.0]),
            ("block-control", [20.0, 420.0, 180.0, 20.0]),
            ("replaced-control-case", [380.0, 420.0, 300.0, 70.0]),
            ("replaced-control", [380.0, 420.0, 101.0, 33.0]),
            ("stretch-control-case", [740.0, 420.0, 300.0, 70.0]),
            ("stretch-control", [740.0, 420.0, 300.0, 20.0]),
            ("inline-margin-case", [20.0, 520.0, 200.0, 70.0]),
            ("inline-margin-float", [30.0, 520.0, 175.0, 40.0]),
            ("negative-margin-case", [380.0, 520.0, 200.0, 70.0]),
            ("negative-margin-float", [370.0, 520.0, 225.0, 40.0]),
            ("inline-negative-margin-case", [740.0, 520.0, 200.0, 70.0]),
            (
                "inline-negative-margin-float",
                [730.0, 520.0, 225.0, 40.0],
            ),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("float fit-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn intrinsic_width_keywords_match_chromium_across_formatting_contexts() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-width.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}.case{position:absolute;width:300px;height:70px;font-size:0}.probe{height:20px}.a,.b{display:inline-block;height:20px}.a{width:160px}.b{width:80px;margin-left:8px}
#min-case{left:20px;top:20px}#max-case{left:380px;top:20px}#fit-case{left:740px;top:20px}#fit-narrow-case{left:20px;top:120px;width:200px}#fit-min-case{left:380px;top:120px;width:120px}#min-clamp-case{left:740px;top:120px}#max-clamp-case{left:20px;top:220px}#conflict-case{left:380px;top:220px}#content-box-case{left:740px;top:220px}#border-box-case{left:20px;top:320px}#flex-min-case{left:380px;top:320px;display:flex}#flex-max-case{left:740px;top:320px;display:flex}#flex-cross-case{left:20px;top:420px;display:flex;flex-direction:column}#grid-min-case{left:380px;top:420px;display:grid}#grid-max-case{left:740px;top:420px;display:grid}#absolute-min-case{left:20px;top:520px}#absolute-fit-case{left:380px;top:520px;width:200px}#float-min-case{left:740px;top:520px}#float-max-case{left:20px;top:620px}#replaced-case{left:380px;top:620px}#stretch-case{left:740px;top:620px}#webkit-fill-case{left:20px;top:720px}#aspect-block-case{left:380px;top:720px}#aspect-flex-case{left:740px;top:720px}#aspect-grid-case{left:20px;top:820px}#flex-grow-case{left:380px;top:820px;display:flex}#flex-shrink-case{left:740px;top:820px;width:120px;display:flex}#flex-basis-content-case{left:20px;top:920px;display:flex}#auto-grid-min-case{left:380px;top:920px}#auto-grid-max-case{left:740px;top:920px}#absolute-inset-fit-case{left:20px;top:1020px;width:200px}#float-fit-margin-case{left:380px;top:1020px;width:200px}#float-stretch-margin-case{left:740px;top:1020px;width:200px}#inline-min-case{left:20px;top:1120px}#inline-max-case{left:380px;top:1120px}#inline-fit-case{left:740px;top:1120px;width:200px}#min-fit-case{left:20px;top:1220px;width:200px}#max-fit-case{left:380px;top:1220px;width:200px}#min-stretch-case{left:740px;top:1220px}#max-stretch-case{left:20px;top:1320px}#min-webkit-fill-case{left:380px;top:1320px}#max-webkit-fill-case{left:740px;top:1320px}
.edge{padding:0 10px;border:2px solid black}.flex-item{flex:0 0 auto}.absolute{position:absolute;left:0;right:0}.float{float:left}.stretch{margin-left:10px;margin-right:15px}.aspect-probe{height:20px;aspect-ratio:20}.grow{flex:1 1 auto}.shrink{flex:0 1 auto;min-width:0}.basis-content{flex:0 0 content}.auto-grid{display:grid;width:max-content}
</style>`;
const content='<span class=a></span> <span class=b></span>';
document.body.innerHTML = `
<div id=min-case class=case><div id=min class=probe style="width:min-content">${content}</div></div>
<div id=max-case class=case><div id=max class=probe style="width:max-content">${content}</div></div>
<div id=fit-case class=case><div id=fit class=probe style="width:fit-content">${content}</div></div>
<div id=fit-narrow-case class=case><div id=fit-narrow class=probe style="width:fit-content">${content}</div></div>
<div id=fit-min-case class=case><div id=fit-min class=probe style="width:fit-content">${content}</div></div>
<div id=min-clamp-case class=case><div id=min-clamp class=probe style="width:100px;min-width:max-content">${content}</div></div>
<div id=max-clamp-case class=case><div id=max-clamp class=probe style="width:300px;max-width:min-content">${content}</div></div>
<div id=conflict-case class=case><div id=conflict class=probe style="width:200px;min-width:max-content;max-width:min-content">${content}</div></div>
<div id=content-box-case class=case><div id=content-box class="probe edge" style="box-sizing:content-box;width:min-content">${content}</div></div>
<div id=border-box-case class=case><div id=border-box class="probe edge" style="box-sizing:border-box;width:min-content">${content}</div></div>
<div id=flex-min-case class=case><div id=flex-min class="probe flex-item" style="width:min-content">${content}</div></div>
<div id=flex-max-case class=case><div id=flex-max class="probe flex-item" style="width:max-content">${content}</div></div>
<div id=flex-cross-case class=case><div id=flex-cross class=probe style="width:min-content">${content}</div></div>
<div id=grid-min-case class=case><div id=grid-min class=probe style="width:min-content">${content}</div></div>
<div id=grid-max-case class=case><div id=grid-max class=probe style="width:max-content">${content}</div></div>
<div id=absolute-min-case class=case><div id=absolute-min class="probe absolute" style="width:min-content">${content}</div></div>
<div id=absolute-fit-case class=case><div id=absolute-fit class="probe absolute" style="width:fit-content">${content}</div></div>
<div id=float-min-case class=case><div id=float-min class="probe float" style="width:min-content">${content}</div></div>
<div id=float-max-case class=case><div id=float-max class="probe float" style="width:max-content">${content}</div></div>
<div id=replaced-case class=case><svg id=replaced style="display:block;width:min-content" width=180 height=40 viewBox="0 0 180 40"></svg></div>
<div id=stretch-case class=case><div id=stretch class="probe stretch" style="width:stretch"></div></div>
<div id=webkit-fill-case class=case><div id=webkit-fill class="probe stretch" style="width:-webkit-fill-available"></div></div>
<div id=aspect-block-case class=case><div id=aspect-block class="probe aspect-probe" style="width:min-content">${content}</div></div>
<div id=aspect-flex-case class=case><div id=aspect-flex class="probe aspect-probe" style="display:flex;width:min-content"><span class=a></span><span class=b></span></div></div>
<div id=aspect-grid-case class=case><div id=aspect-grid class="probe aspect-probe" style="display:grid;width:min-content"><span class=a></span><span class=b></span></div></div>
<div id=flex-grow-case class=case><div id=flex-grow class="probe grow" style="width:min-content">${content}</div></div>
<div id=flex-shrink-case class=case><div id=flex-shrink class="probe shrink" style="width:max-content">${content}</div></div>
<div id=flex-basis-content-case class=case><div id=flex-basis-content class="probe basis-content" style="width:min-content">${content}</div></div>
<div id=auto-grid-min-case class=case><div id=auto-grid-min class=auto-grid><div id=auto-grid-min-item class=probe style="width:min-content">${content}</div></div></div>
<div id=auto-grid-max-case class=case><div id=auto-grid-max class=auto-grid><div id=auto-grid-max-item class=probe style="width:max-content">${content}</div></div></div>
<div id=absolute-inset-fit-case class=case><div id=absolute-inset-fit class="probe absolute" style="left:40px;right:auto;width:fit-content">${content}</div></div>
<div id=float-fit-margin-case class=case><div id=float-fit-margin class="probe float stretch" style="width:fit-content">${content}</div></div>
<div id=float-stretch-margin-case class=case><div id=float-stretch-margin class="probe float stretch" style="width:stretch">${content}</div></div>
<div id=inline-min-case class=case><span id=inline-min class=probe style="display:inline-block;width:min-content">${content}</span></div>
<div id=inline-max-case class=case><span id=inline-max class=probe style="display:inline-block;width:max-content">${content}</span></div>
<div id=inline-fit-case class=case><span id=inline-fit class=probe style="display:inline-block;width:fit-content">${content}</span></div>
<div id=min-fit-case class=case><div id=min-fit class=probe style="width:100px;min-width:fit-content">${content}</div></div>
<div id=max-fit-case class=case><div id=max-fit class=probe style="width:300px;max-width:fit-content">${content}</div></div>
<div id=min-stretch-case class=case><div id=min-stretch class="probe stretch" style="width:100px;min-width:stretch"></div></div>
<div id=max-stretch-case class=case><div id=max-stretch class="probe stretch" style="width:400px;max-width:stretch"></div></div>
<div id=min-webkit-fill-case class=case><div id=min-webkit-fill class="probe stretch" style="width:100px;min-width:-webkit-fill-available"></div></div>
<div id=max-webkit-fill-case class=case><div id=max-webkit-fill class="probe stretch" style="width:400px;max-width:-webkit-fill-available"></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let supports = page_vm.vm_mut().eval(
            r#"(()=>{const values=['min-content','max-content','fit-content','fit-content(120px)','fit-content(50%)','stretch','-webkit-fill-available'];const result=Object.fromEntries(values.map(value=>[value,CSS.supports('width',value)]));result['grid-fit-content(120px)']=CSS.supports('grid-template-columns','fit-content(120px)');for(const [key,property,value] of [['min-width:fit-content','min-width','fit-content'],['max-width:fit-content','max-width','fit-content'],['min-width:stretch','min-width','stretch'],['max-width:stretch','max-width','stretch'],['min-width:-webkit-fill-available','min-width','-webkit-fill-available'],['max-width:-webkit-fill-available','max-width','-webkit-fill-available']])result[key]=CSS.supports(property,value);return JSON.stringify(result)})()"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&supports)?,
            serde_json::json!({
                "min-content": true,
                "max-content": true,
                "fit-content": true,
                "fit-content(120px)": false,
                "fit-content(50%)": false,
                "stretch": true,
                "-webkit-fill-available": true,
                "grid-fit-content(120px)": true,
                "min-width:fit-content": true,
                "max-width:fit-content": true,
                "min-width:stretch": true,
                "max-width:stretch": true,
                "min-width:-webkit-fill-available": true,
                "max-width:-webkit-fill-available": true,
            })
        );

        let ids = [
            "min", "max", "fit", "fit-narrow", "fit-min", "min-clamp", "max-clamp",
            "conflict", "content-box", "border-box", "flex-min", "flex-max", "flex-cross",
            "grid-min", "grid-max", "absolute-min", "absolute-fit", "float-min", "float-max",
            "replaced", "stretch", "webkit-fill", "aspect-block", "aspect-flex",
            "aspect-grid", "flex-grow", "flex-shrink", "flex-basis-content",
            "auto-grid-min", "auto-grid-min-item", "auto-grid-max", "auto-grid-max-item",
            "absolute-inset-fit", "float-fit-margin", "float-stretch-margin",
            "inline-min", "inline-max", "inline-fit", "min-fit", "max-fit", "min-stretch",
            "max-stretch", "min-webkit-fill", "max-webkit-fill",
        ];
        let geometry = page_vm.vm_mut().eval(&format!(
            "JSON.stringify(Object.fromEntries({ids:?}.map(id=>{{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]}})))"
        ))?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("min", [20.0, 20.0, 160.0, 20.0]),
            ("max", [380.0, 20.0, 248.0, 20.0]),
            ("fit", [740.0, 20.0, 248.0, 20.0]),
            ("fit-narrow", [20.0, 120.0, 200.0, 20.0]),
            ("fit-min", [380.0, 120.0, 160.0, 20.0]),
            ("min-clamp", [740.0, 120.0, 248.0, 20.0]),
            ("max-clamp", [20.0, 220.0, 160.0, 20.0]),
            ("conflict", [380.0, 220.0, 248.0, 20.0]),
            ("content-box", [740.0, 220.0, 184.0, 24.0]),
            ("border-box", [20.0, 320.0, 184.0, 20.0]),
            ("flex-min", [380.0, 320.0, 160.0, 20.0]),
            ("flex-max", [740.0, 320.0, 248.0, 20.0]),
            ("flex-cross", [20.0, 420.0, 160.0, 20.0]),
            ("grid-min", [380.0, 420.0, 160.0, 20.0]),
            ("grid-max", [740.0, 420.0, 248.0, 20.0]),
            ("absolute-min", [20.0, 520.0, 160.0, 20.0]),
            ("absolute-fit", [380.0, 520.0, 200.0, 20.0]),
            ("float-min", [740.0, 520.0, 160.0, 20.0]),
            ("float-max", [20.0, 620.0, 248.0, 20.0]),
            ("replaced", [380.0, 620.0, 180.0, 40.0]),
            ("stretch", [750.0, 620.0, 275.0, 20.0]),
            ("webkit-fill", [30.0, 720.0, 275.0, 20.0]),
            ("aspect-block", [380.0, 720.0, 400.0, 20.0]),
            ("aspect-flex", [740.0, 720.0, 400.0, 20.0]),
            ("aspect-grid", [20.0, 820.0, 400.0, 20.0]),
            ("flex-grow", [380.0, 820.0, 300.0, 20.0]),
            ("flex-shrink", [740.0, 820.0, 120.0, 20.0]),
            ("flex-basis-content", [20.0, 920.0, 248.0, 20.0]),
            ("auto-grid-min", [380.0, 920.0, 160.0, 20.0]),
            ("auto-grid-min-item", [380.0, 920.0, 160.0, 20.0]),
            ("auto-grid-max", [740.0, 920.0, 248.0, 20.0]),
            ("auto-grid-max-item", [740.0, 920.0, 248.0, 20.0]),
            ("absolute-inset-fit", [60.0, 1020.0, 160.0, 20.0]),
            ("float-fit-margin", [390.0, 1020.0, 175.0, 20.0]),
            ("float-stretch-margin", [750.0, 1020.0, 175.0, 20.0]),
            ("inline-min", [20.0, 1120.0, 160.0, 20.0]),
            ("inline-max", [380.0, 1120.0, 248.0, 20.0]),
            ("inline-fit", [740.0, 1120.0, 200.0, 20.0]),
            ("min-fit", [20.0, 1220.0, 200.0, 20.0]),
            ("max-fit", [380.0, 1220.0, 200.0, 20.0]),
            ("min-stretch", [750.0, 1220.0, 275.0, 20.0]),
            ("max-stretch", [30.0, 1320.0, 275.0, 20.0]),
            ("min-webkit-fill", [390.0, 1320.0, 275.0, 20.0]),
            ("max-webkit-fill", [750.0, 1320.0, 275.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1200, 1420, 1.0))?
            .expect("intrinsic width fixture must retain a root");
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "intrinsic-sizing-keyword-deferred"
        }));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic width fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn nested_inline_intrinsic_keywords_survive_float_contribution_probes() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-nested-keywords.html")?,
        );
        let fixture = include_str!("../../../../../tests/fixtures/intrinsic-nested-keywords.html");
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(fixture)?
        ))?;
        page.vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("nested intrinsic fixture should retain a root");
        // Chromium and the frozen 0.13 browser agree on these dimensions.
        // A max-content child must remain 240px wide even when its float's
        // intrinsic contribution is measured with MinContent available space.
        for (keyword, width, height) in [
            ("min-content", 160, 40),
            ("max-content", 240, 20),
            ("fit-content", 160, 40),
        ] {
            for prefix in ["float", "inline"] {
                let id = format!("{prefix}-{keyword}");
                let actual = page.vm_mut().eval(&format!(
                    "(()=>{{const r=document.getElementById({}).getBoundingClientRect();return JSON.stringify([r.width,r.height])}})()",
                    serde_json::to_string(&id)?
                ))?;
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&actual)?,
                    serde_json::json!([width, height]),
                    "{id}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested intrinsic keyword fixture should run");
}
