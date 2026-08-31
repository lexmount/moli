use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paints_flex_and_grid_items_as_atomic_inline_level_boxes() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-grid-atomic-paint.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
.case{width:100px;height:100px;margin-bottom:10px}
.grid,.inline-grid,.ordered,.stacked,.positioned-descendant{grid-template:100px/100px}
.grid,.ordered,.stacked,.positioned-descendant{display:grid}
.inline-grid{display:inline-grid}
.grid>*,.inline-grid>*,.ordered>*,.stacked>*,.positioned-descendant>*{grid-area:1/1;width:100px;height:100px}
.flex{display:flex}
.flex>*{flex:none;width:100px;height:100px}
.flex>.cover{margin-left:-100px}
.cover{background:green}
.ordered>.cover{order:1}
.ordered>.content{order:0}
.stacked>.content{z-index:1}
.positioned-descendant .elevated{position:relative;width:100px;height:100px;background:red}
</style>`;
const content = className => `<svg class="${className}" width="100" height="100" viewBox="0 0 100 100"><rect width="100" height="100" fill="red"/></svg>`;
document.body.innerHTML = `
<div class="case grid">${content('content')}<div class=cover></div></div>
<div class="case inline-grid">${content('content')}<div class=cover></div></div>
<div class="case flex">${content('content')}<div class=cover></div></div>
<div class="case ordered"><div class=cover></div>${content('content')}</div>
<div class="case stacked">${content('content')}<div class=cover></div></div>
<div class="case positioned-descendant"><div><div class=elevated></div></div><div class=cover></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(120, 660, 1.0))?
            .expect("flex/grid atomic-paint fixture should retain a layout root");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };

        for (label, y) in [
            ("grid item", 50),
            ("inline-grid item", 160),
            ("flex item", 270),
            ("order-modified grid item", 380),
        ] {
            assert_eq!(
                pixel(50, y),
                [0, 128, 0, 255],
                "{label} descendants must not escape the item's atomic paint boundary",
            );
        }
        assert_eq!(
            pixel(50, 490),
            [255, 0, 0, 255],
            "a non-auto z-index on a static grid item must still establish a stacking context",
        );
        assert_eq!(
            pixel(50, 600),
            [255, 0, 0, 255],
            "a positioned descendant of an atomic grid item must participate in the parent stacking context",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex/grid atomic-paint fixture should run");
}
