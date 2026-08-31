use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_grid_normal_and_explicit_stretch_for_replaced_items() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-normal-self-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:grid;width:350px;height:250px}
.grid>svg{display:block}
.parent-normal{place-items:normal}
.parent-center{place-items:center}
.align-stretch{align-self:stretch}
.justify-stretch{justify-self:stretch}
.both-stretch{place-self:stretch}
.normal-override{place-self:normal}
</style>`;
document.body.innerHTML = `
<div class=grid id=default-host><svg id=default-item></svg></div>
<div class="grid parent-normal" id=parent-normal-host><svg id=parent-normal-item></svg></div>
<div class=grid id=align-host><svg class=align-stretch id=align-item></svg></div>
<div class=grid id=justify-host><svg class=justify-stretch id=justify-item></svg></div>
<div class=grid id=both-host><svg class=both-stretch id=both-item></svg></div>
<div class="grid parent-center" id=center-host><svg id=center-item></svg></div>
<div class="grid parent-center" id=override-host><svg class=normal-override id=override-item></svg></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 1_800, 1.0))?
            .expect("Grid self-alignment screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([
  ['default','default-host','default-item'],
  ['parentNormal','parent-normal-host','parent-normal-item'],
  ['alignStretch','align-host','align-item'],
  ['justifyStretch','justify-host','justify-item'],
  ['bothStretch','both-host','both-item'],
  ['centerInherited','center-host','center-item'],
  ['normalOverride','override-host','override-item']
].map(([name,hostId,itemId])=>{
  const host=document.getElementById(hostId).getBoundingClientRect();
  const item=document.getElementById(itemId).getBoundingClientRect();
  return [name,[item.left-host.left,item.top-host.top,item.width,item.height]];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "default": [0, 0, 300, 150],
                "parentNormal": [0, 0, 300, 150],
                "alignStretch": [0, 0, 300, 250],
                "justifyStretch": [0, 0, 350, 150],
                "bothStretch": [0, 0, 350, 250],
                "centerInherited": [25, 50, 300, 150],
                "normalOverride": [0, 0, 300, 150],
            }),
            "Grid normal must remain distinct from explicit stretch through the Stylo/Taffy boundary",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid self-alignment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_grid_alignment_auto_sizing_for_absolute_items() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/absolute-grid-self-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:grid;position:relative;width:100px;height:80px}
.item{position:absolute;inset:0;height:20px;aspect-ratio:2/1}
.normal{place-self:normal}
.stretch{place-self:stretch}
.start{place-self:start}
.without-ratio{aspect-ratio:auto}
</style>`;
document.body.innerHTML = `
<div class=grid id=normal-host><div class="item normal" id=normal-item></div></div>
<div class=grid id=stretch-host><div class="item stretch" id=stretch-item></div></div>
<div class=grid id=start-host><div class="item start without-ratio" id=start-item></div></div>
<div class=grid id=implicit-host><div class="item normal without-ratio" id=implicit-item></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 400, 1.0))?
            .expect("absolute Grid self-alignment screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([
  ['normal','normal-host','normal-item'],
  ['stretch','stretch-host','stretch-item'],
  ['start','start-host','start-item'],
  ['implicit','implicit-host','implicit-item']
].map(([name,hostId,itemId])=>{
  const host=document.getElementById(hostId).getBoundingClientRect();
  const item=document.getElementById(itemId).getBoundingClientRect();
  return [name,[item.left-host.left,item.top-host.top,item.width,item.height]];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "normal": [0, 0, 40, 20],
                "stretch": [0, 0, 100, 20],
                "start": [0, 0, 0, 20],
                "implicit": [0, 0, 100, 20],
            }),
            "absolute Grid sizing must distinguish fit-content, implicit stretch, and explicit stretch",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("absolute Grid self-alignment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_svg_default_object_size_out_of_grid_natural_ratio() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/grid-svg-natural-sizing.html").expect("document URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:grid;width:350px;height:250px}
.grid>img{display:block}
.align-stretch{align-self:stretch}
.justify-stretch{justify-self:stretch}
.both-stretch{place-self:stretch}
</style>`;
const source = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMCIvPg==";
document.body.innerHTML = `
<div class=grid id=normal-host><img src="${source}" id=normal-item></div>
<div class=grid id=align-host><img src="${source}" class=align-stretch id=align-item></div>
<div class=grid id=justify-host><img src="${source}" class=justify-stretch id=justify-item></div>
<div class=grid id=both-host><img src="${source}" class=both-stretch id=both-item></div>`;
'installed'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "Array.from(document.images, image => [image.complete, image.naturalWidth, image.naturalHeight]).join('|')",
            )?,
            "true,20,150|true,20,150|true,20,150|true,20,150",
            "local SVG resources should expose their concrete natural dimensions before layout",
        );

        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 1_000, 1.0))?
            .expect("Grid SVG natural-sizing screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([
  ['normal','normal-host','normal-item'],
  ['alignStretch','align-host','align-item'],
  ['justifyStretch','justify-host','justify-item'],
  ['bothStretch','both-host','both-item']
].map(([name,hostId,itemId])=>{
  const host=document.getElementById(hostId).getBoundingClientRect();
  const item=document.getElementById(itemId).getBoundingClientRect();
  return [name,[item.left-host.left,item.top-host.top,item.width,item.height]];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "normal": [0, 0, 20, 150],
                "alignStretch": [0, 0, 20, 250],
                "justifyStretch": [0, 0, 350, 150],
                "bothStretch": [0, 0, 350, 250],
            }),
            "the 300x150 concrete fallback must not manufacture a natural aspect ratio",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid SVG natural-sizing fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_renders_zero_axis_svg_in_the_stretched_grid_content_viewport() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_image_fetch_enabled(true);
        let document_url =
            Url::parse("https://example.com/grid-zero-axis-svg.html").expect("document URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:grid;width:350px;height:250px;background:gray}
.grid>img{display:block}
.align-stretch{align-self:stretch}
.justify-stretch{justify-self:stretch}
.both-stretch{place-self:stretch}
</style>`;
const source = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIwIiBoZWlnaHQ9IjIwIj48Y2lyY2xlIGN4PSI1MCUiIGN5PSI1MCUiIHI9IjUwJSIgZmlsbD0iYmx1ZSIvPjwvc3ZnPg==";
document.body.innerHTML = `
<div class=grid id=both-host><img src="${source}" class=both-stretch id=both-item></div>
<div class=grid id=align-host><img src="${source}" class=align-stretch id=align-item></div>
<div class=grid id=justify-host><img src="${source}" class=justify-stretch id=justify-item></div>
<div class=grid id=normal-host><img src="${source}" id=normal-item></div>`;
'installed'
"#,
        )?;
        for _ in 0..4 {
            let task = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if let Some(task) = page_vm.take_dom_manipulation_body_task_for_test(
                        PageDomManipulationTestFamily::ImageLoadEvent,
                    ) {
                        break task;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("each local SVG decode should publish an image-load task");
            let crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(
                image_task,
            ) = task
            else {
                unreachable!("exact image-load selection preserves its task variant")
            };
            assert_eq!(
                image_task.kind(),
                crate::page_task_queue::RendererPageImageLoadEventKind::Load,
                "zero-axis SVG decoding must finish as an available image",
            );
            page_vm
                .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                    crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(
                        image_task,
                    ),
                    &loader,
                )
                .await?;
        }
        assert_eq!(
            page_vm.vm_mut().eval(
                "Array.from(document.images, image => [image.complete, image.naturalWidth, image.naturalHeight]).join('|')",
            )?,
            "true,0,20|true,0,20|true,0,20|true,0,20",
        );

        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 1_000, 1.0))?
            .expect("zero-axis SVG Grid screenshot layout");
        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([
  ['bothStretch','both-host','both-item'],
  ['alignStretch','align-host','align-item'],
  ['justifyStretch','justify-host','justify-item'],
  ['normal','normal-host','normal-item']
].map(([name,hostId,itemId])=>{
  const host=document.getElementById(hostId).getBoundingClientRect();
  const item=document.getElementById(itemId).getBoundingClientRect();
  return [name,[item.left-host.left,item.top-host.top,item.width,item.height]];
})))"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&geometry)?,
            serde_json::json!({
                "bothStretch": [0, 0, 350, 250],
                "alignStretch": [0, 0, 0, 250],
                "justifyStretch": [0, 0, 350, 20],
                "normal": [0, 0, 0, 20],
            }),
        );

        let svg_destinations = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::SvgImage(image) => Some(image.destination),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            svg_destinations,
            [
                moli_layout::LayoutRect::new(0.0, 0.0, 350.0, 250.0),
                moli_layout::LayoutRect::new(0.0, 500.0, 350.0, 20.0),
            ],
            "only nonempty stretched SVG boxes should enter the paint snapshot",
        );

        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let offset = ((y * raster.width + x) * 4) as usize;
            raster.rgba[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(pixel(175, 125), [0, 0, 255, 255]);
        assert_eq!(pixel(175, 375), [128, 128, 128, 255]);
        assert_eq!(pixel(175, 510), [0, 0, 255, 255]);
        assert_eq!(pixel(175, 760), [128, 128, 128, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("zero-axis SVG Grid fixture should run");
}
