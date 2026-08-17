use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_projects_external_svg_root_paint_like_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-computed-paint.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
#run{position:absolute;left:0;top:0;width:80px;height:40px;margin:0;padding:0;border:0;background:rgb(174,67,28);color:white}
.icon{position:absolute;left:30px;top:10px;display:block;width:20px;height:20px;fill:currentcolor}
#stroke{position:absolute;left:100px;top:10px;display:block;width:20px;height:20px;color:rgb(0,128,0);fill:none;stroke:currentcolor;stroke-width:4px;stroke-linecap:butt}
</style>`;
document.body.innerHTML = `
<button id=run><svg id=run-icon class=icon viewBox="0 0 20 20"><path d="M2 2v16l16-8z"></path></svg></button>
<svg id=stroke viewBox="0 0 20 20"><path d="M2 10h16"></path></svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        assert_eq!(
            page_vm.vm_mut().eval(
                "[getComputedStyle(document.getElementById('run-icon')).fill,getComputedStyle(document.getElementById('stroke')).stroke].join('|')",
            )?,
            "rgb(255, 255, 255)|rgb(0, 128, 0)",
            "Stylo must resolve the external SVG paint rules before the paint bridge snapshots them",
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(140, 50, 1.0))?
            .expect("inline SVG computed-paint fixture must retain a layout root");
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };

        assert_eq!(pixel(40, 20), [255, 255, 255, 255]);
        assert_eq!(pixel(15, 20), [174, 67, 28, 255]);
        assert_eq!(pixel(110, 20), [0, 128, 0, 255]);
        assert_eq!(pixel(90, 20), [255, 255, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG computed-paint fixture should run");
}
