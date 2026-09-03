use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_applies_the_absolute_ratio_dependent_automatic_minimum() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/absolute-ratio-automatic-minimum.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>html,body{margin:0}</style>`;
document.body.innerHTML = `<div id=target style="height:100px;aspect-ratio:1/2;position:absolute"><div style="width:100px"></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?
            .expect("absolute ratio automatic-minimum fixture must retain a layout root");

        assert_eq!(
            page_vm.vm_mut().eval(
                "const rect=document.querySelector('#target').getBoundingClientRect();JSON.stringify([rect.width,rect.height])",
            )?,
            "[100,100]",
            "the min-content contribution must floor a ratio-derived absolute inline size",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("absolute ratio automatic-minimum fixture should run");
}

/// Regression for WPT css/css-sizing/aspect-ratio/flex-aspect-ratio-026.html.
#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_item_contributions_for_column_flex_intrinsic_width() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/column-flex-intrinsic-width.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.column{display:flex;flex-direction:column;float:left;height:1px}
.item{box-sizing:border-box;min-width:25px;padding-left:15px;padding-top:10px}
.item>div{height:190px}
</style>`;
document.body.innerHTML = `
<div class=column><div id=border-box class=item style="aspect-ratio:1/1"><div></div></div></div>
<div class=column><div id=auto-ratio class=item style="aspect-ratio:auto 1/1"><div></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?
            .expect("column flex intrinsic-width fixture must retain a layout root");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"JSON.stringify([...document.querySelectorAll('.column,.item')].map(element=>{const rect=element.getBoundingClientRect();return [rect.width,rect.height]}))"#,
            )?,
            "[[25,1],[25,200],[25,1],[25,200]]",
            "a content-derived main size must not become the column container's intrinsic inline contribution",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("column flex intrinsic-width fixture should run");
}

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
async fn screenshot_normalizes_border_box_insets_before_ratio_sizing() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/border-box-aspect-ratio-floor.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.item{box-sizing:border-box;border:20px solid blue;display:block}
.horizontal{aspect-ratio:2/1}
.vertical{aspect-ratio:1/2}
.spacer{margin-bottom:10px}
</style>`;
const source='data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMCIgaGVpZ2h0PSI1MCI+PC9zdmc+';
document.body.innerHTML = `
<img class="item horizontal" id="image-height" style="width:auto;height:20px" src="${source}">
<img class="item horizontal" id="image-max-height" style="width:auto;max-height:20px" src="${source}">
<img class="item vertical" id="image-width" style="height:auto;width:20px" src="${source}">
<img class="item vertical" id="image-max-width" style="height:auto;max-width:20px" src="${source}">
<div class="item horizontal spacer" id="block-height" style="width:auto;height:20px"></div>
<div class="item horizontal spacer" id="block-max-height" style="width:auto;max-height:20px"></div>
<div class="item vertical spacer" id="block-width" style="height:auto;width:20px"></div>
<div class="item vertical" id="block-max-width" style="height:auto;max-width:20px"></div>`;
'installed'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "[...document.images].every(image=>image.complete&&image.naturalWidth===20&&image.naturalHeight===50)",
            )?,
            "true",
            "the replaced fixtures must expose their natural sizes before layout",
        );
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 640, 1.0))?
            .expect("border-box aspect-ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for id in ["image-height", "image-max-height", "block-height", "block-max-height"] {
            assert_eq!(
                geometry[id],
                serde_json::json!([80, 40]),
                "horizontal ratio must use the inset-floored block size: {geometry}",
            );
        }
        for id in ["image-width", "image-max-width", "block-width", "block-max-width"] {
            assert_eq!(
                geometry[id],
                serde_json::json!([40, 80]),
                "vertical ratio must use the inset-floored inline size: {geometry}",
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("border-box aspect-ratio floor fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_normalizes_sparse_natural_sizes_with_the_preferred_ratio_box() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/sparse-natural-size-aspect-ratio.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
img{display:block;aspect-ratio:1/1}
#border-box{box-sizing:border-box;padding-left:50px;height:min-content}
#content-box{box-sizing:content-box;padding-left:50px;height:min-content}
#horizontal-flow,#vertical-flow{box-sizing:border-box;border:20px solid;aspect-ratio:2/1}
#vertical-flow{writing-mode:vertical-rl}
</style>`;
const source = `<svg xmlns="http://www.w3.org/2000/svg" width="50px"></svg>`;
const fixedSource = `<svg xmlns="http://www.w3.org/2000/svg" width="20px" height="50px"></svg>`;
document.body.innerHTML = `
<img id=border-box src="data:image/svg+xml,${encodeURIComponent(source)}">
<img id=content-box src="data:image/svg+xml,${encodeURIComponent(source)}">
<img id=horizontal-flow src="data:image/svg+xml,${encodeURIComponent(fixedSource)}">
<img id=vertical-flow src="data:image/svg+xml,${encodeURIComponent(fixedSource)}">`;
'installed'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "[...document.images].map(image=>`${image.complete}:${image.naturalWidth}`).join(',')",
            )?,
            "true:50,true:50,true:20,true:20",
            "the SVG fixtures must expose their natural widths before layout",
        );
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 320, 1.0))?
            .expect("sparse natural-size fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.images].map(image=>{const rect=image.getBoundingClientRect();return [image.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "border-box": [100, 100],
                "content-box": [100, 50],
                "horizontal-flow": [80, 40],
                "vertical-flow": [180, 90],
            }),
            "the preferred ratio must normalize natural sizes only after its sizing box, insets, and logical axes are known",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("sparse natural-size aspect-ratio fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_recomputes_flex_ratio_cross_size_from_the_final_main_size() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-final-main-ratio-cross-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.row{display:flex;width:100px}
.column{display:flex;flex-direction:column;align-items:flex-start;height:100px}
.grow{flex:1;aspect-ratio:1/1}
#ordinary{width:50px;min-width:0}
#replaced{width:50px;min-height:0}
#column{height:50px;min-height:0}
#explicit-cross{width:50px;height:30px}
#content-box{box-sizing:content-box;width:50px;min-width:0;padding:10px}
#content-box-host{width:120px}
#limited-cross{width:50px;min-width:0;max-height:80px}
</style>`;
const source = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMCIgaGVpZ2h0PSI1MCI+PC9zdmc+";
document.body.innerHTML = `
<div class=row><div class=grow id=ordinary></div></div>
<div class=row><img class=grow id=replaced src="${source}"></div>
<div class=column><div class=grow id=column></div></div>
<div class=row><div class=grow id=explicit-cross></div></div>
<div class=row id=content-box-host><div class=grow id=content-box></div></div>
<div class=row><div class=grow id=limited-cross></div></div>`;
'installed'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "const image=document.getElementById('replaced');[image.complete,image.naturalWidth,image.naturalHeight].join(',')",
            )?,
            "true,20,50",
            "the replaced fixture must expose its natural size before layout",
        );
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 640, 1.0))?
            .expect("flex final-main ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]:not(#content-box-host)')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "ordinary": [100, 100],
                "replaced": [100, 100],
                "column": [100, 100],
                "explicit-cross": [100, 30],
                "content-box": [120, 120],
                "limited-cross": [100, 80],
            }),
            "only a direct cross size may outrank the ratio transferred from the final flexed main size",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex final-main ratio fixture should run");
}

/// Regression for WPT css/css-flexbox/flex-aspect-ratio-img-column-017.html.
#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_svg_metadata_before_decode_in_a_flex_automatic_minimum() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_image_fetch_enabled(true);
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-default-svg-automatic-minimum.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>html,body{margin:0}</style>`;
document.body.innerHTML = `<div style="display:flex;flex-direction:column;height:0;width:150px">
  <img id=target src='data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" />' style="height:200px;background:green">
</div>`;
'installed'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "const image=document.getElementById('target');[image.complete,image.naturalWidth,image.naturalHeight].join(',')",
            )?,
            "true,300,150",
            "probed SVG dimensions must be observable before paint content is committed",
        );
        let image = page_vm
            .vm()
            .document_runtime
            .get_element_by_id("target")
            .expect("metadata-ready SVG image");
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("page context host");
        assert!(
            context_host
                .borrow()
                .ready_image_for_layout(image)
                .is_none(),
            "metadata availability must not masquerade as decoded paint content",
        );
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?
            .expect("default SVG flex fixture must retain a layout root");

        assert_eq!(
            page_vm.vm_mut().eval(
                "const rect=document.getElementById('target').getBoundingClientRect();JSON.stringify([rect.width,rect.height])",
            )?,
            "[150,150]",
            "the replaced content-size suggestion must preserve the 150px flex automatic minimum",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("default SVG flex automatic-minimum fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_parent_resolved_grid_sizes_across_ratio_constraints() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-ratio-known-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:grid;width:100px;aspect-ratio:2;max-width:50px;align-content:center}
.item{width:100px;height:100px}
#authored-max{max-height:20px}
#explicit-min{min-height:0}
</style>`;
document.body.innerHTML = `
<div class=grid id=automatic><div class=item></div></div>
<div class=grid id=authored-max><div class=item></div></div>
<div class=grid id=explicit-min><div class=item></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 480, 1.0))?
            .expect("grid ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('.grid')].map(grid=>{const item=grid.firstElementChild,g=grid.getBoundingClientRect(),i=item.getBoundingClientRect();return [grid.id,[g.width,g.height,i.x-g.x,i.y-g.y,i.width,i.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("automatic", serde_json::json!([50, 100, 0, 0, 100, 100])),
            ("authored-max", serde_json::json!([50, 20, 0, -40, 100, 100])),
            ("explicit-min", serde_json::json!([50, 25, 0, -37.5, 100, 100])),
        ] {
            assert_eq!(
                geometry[id],
                expected,
                "Chromium-calibrated grid ratio geometry mismatch for {id}: {geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("grid ratio known-size fixture should run");
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
#intrinsic-transferred-maximum{width:max-content;max-height:100px;aspect-ratio:1/1}
#intrinsic-transferred-maximum>div{width:200px}
#content-box{box-sizing:content-box;width:min-content;height:100px;padding:10px;aspect-ratio:1/1}
</style>`;
document.body.innerHTML = `
<div class="item ratio-preferred" id="block"></div>
<div class="item" id="minimum"><div></div></div>
<div class="item" id="maximum"><div></div></div>
<div class="item" id="clamped-opposite"></div>
<div class="item" id="intrinsic-transferred-maximum"><div></div></div>
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
            ("intrinsic-transferred-maximum", [100, 100]),
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

#[tokio::test(flavor = "current_thread")]
async fn screenshot_exposes_definite_block_geometry_to_intrinsic_width_measurements() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-percentage-constraint-space.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#target{width:min-content;height:200px}
#percentage-parent,#ratio-child{height:100%}
#ratio-child{aspect-ratio:1/1}
</style>`;
document.body.innerHTML = `<div id="target"><div id="percentage-parent"><div id="ratio-child"></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let read_geometry = |page_vm: &mut PageVm| -> anyhow::Result<serde_json::Value> {
            let geometry = page_vm.vm_mut().eval(
                r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
            )?;
            Ok(serde_json::from_str(&geometry)?)
        };

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 480, 1.0))?
            .expect("initial intrinsic percentage fixture must retain a layout root");
        let initial = read_geometry(&mut page_vm)?;
        for id in ["target", "percentage-parent", "ratio-child"] {
            assert_eq!(initial[id], serde_json::json!([200, 200]), "initial geometry mismatch for {id}: {initial}");
        }

        page_vm.vm_mut().eval("document.querySelector('#target').style.height='100px';'updated'")?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 480, 1.0))?
            .expect("updated intrinsic percentage fixture must retain a layout root");
        let updated = read_geometry(&mut page_vm)?;
        for id in ["target", "percentage-parent", "ratio-child"] {
            assert_eq!(updated[id], serde_json::json!([100, 100]), "updated geometry mismatch for {id}: {updated}");
        }

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic percentage constraint-space fixture should run");
}
