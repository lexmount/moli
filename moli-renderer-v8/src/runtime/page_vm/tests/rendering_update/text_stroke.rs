use super::*;

#[tokio::test(flavor = "current_thread")]
async fn background_clip_text_stroke_preserves_transparent_stroke_ink_and_geometry() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/background-clip-text-stroke.html")?,
        );
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        )));
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face{{font-family:MoliStroke;src:url(data:font/ttf;base64,{font})}}
html,body{{margin:0;background:white}}
#title{{position:absolute;left:0;top:0;padding:20px;width:100px;font:50px/1 MoliStroke;
background:red;background-clip:text;-webkit-text-fill-color:transparent;
-webkit-text-stroke:0 transparent}}
</style>`;
document.body.innerHTML = '<div id=title>A</div>';
"#
        ))?;
        let viewport = moli_layout::PaintViewport::new(160, 120, 1.0);
        let plain = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .unwrap();
        let geometry = page_vm.vm_mut().eval(
            "JSON.stringify(document.getElementById('title').getBoundingClientRect().toJSON())",
        )?;
        let pixel = |image: &moli_paint::RasterImage, x: usize, y: usize| -> [u8; 4] {
            let i = (y * image.width as usize + x) * 4;
            image.rgba[i..i + 4].try_into().unwrap()
        };
        let plain_image = moli_paint::raster_snapshot(&plain)?;
        assert_eq!(pixel(&plain_image, 17, 40), [255; 4]);
        assert_eq!(pixel(&plain_image, 25, 40), [255, 0, 0, 255]);

        page_vm
            .vm_mut()
            .eval("document.getElementById('title').style.webkitTextStrokeWidth = '8px'")?;
        let stroked = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .unwrap();
        let image = moli_paint::raster_snapshot(&stroked)?;
        assert_eq!(pixel(&image, 17, 40), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 25, 40), [255, 0, 0, 255]);
        assert!(stroked.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::GlyphRun(run)
                if run.color == moli_layout::PaintColor::BLACK
                    && run.stroke.is_some_and(|stroke| stroke.width == 8.0
                        && stroke.color == moli_layout::PaintColor::BLACK)
        )));
        assert_eq!(
            page_vm.vm_mut().eval(
                "JSON.stringify(document.getElementById('title').getBoundingClientRect().toJSON())",
            )?,
            geometry,
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("background clip text-stroke regression should run");
}
