use super::*;

async fn ready_background_snapshot(
    pixels: moli_image::RgbaImage,
    css: &'static str,
    markup: &'static str,
) -> anyhow::Result<(moli_layout::PaintSnapshot, serde_json::Value)> {
    run_page_vm_async_test(async move {
        let encoded = moli_image::encode_png(&pixels)?;
        let image_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(encoded.bytes),
        );
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        );
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/raster-background-repeat.html")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let executor = page.local_executor.clone();
        executor
            .run(async move {
                let css = css.replace("__IMAGE_URL__", &image_url);
                page.vm_mut().eval(&format!(
                    "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
                    serde_json::to_string(&css)?,
                    serde_json::to_string(markup)?,
                ))?;
                page.vm_mut().sync_live_document_style_sources();
                let viewport = moli_layout::PaintViewport::new(96, 80, 1.0);
                // Establish readiness independently of the projection assertion.
                // This is an ordinary OnDemand screen paint, with no Print job.
                page.vm_mut()
                    .screenshot_layout_snapshot(viewport)?
                    .expect("image fixture has a layout root");
                let completion = page.vm().css_image_completion_notify_for_test();
                tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    loop {
                        let notified = completion.notified();
                        if page.vm().css_image_resource_is_ready_for_test(&image_url) {
                            break;
                        }
                        notified.await;
                    }
                })
                .await?;
                let snapshot = page
                    .vm_mut()
                    .screenshot_layout_snapshot(viewport)?
                    .expect("ready image fixture has a layout root");
                let geometry = page.vm_mut().eval(
                    "JSON.stringify((()=>{const r=document.getElementById('asset').getBoundingClientRect();return [r.x,r.y,r.width,r.height]})())",
                )?;
                Ok((snapshot, serde_json::from_str(&geometry)?))
            })
            .await
    })
    .await
}

fn image_pixel(image: &moli_paint::RasterImage, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * image.width + x) * 4) as usize;
    image.rgba[offset..offset + 4].try_into().unwrap()
}

fn assert_one_repeat_image(snapshot: &moli_layout::PaintSnapshot) {
    assert_eq!(
        snapshot
            .fragments
            .iter()
            .filter(|fragment| matches!(fragment, moli_layout::PaintFragment::Image(_)))
            .count(),
        1,
        "continuous raster repetition must have bounded paint work",
    );
    assert!(snapshot.diagnostics.iter().all(|diagnostic| {
        diagnostic.code != "background-repeat-budget-exceeded"
            && diagnostic.code != "background-image-resource-unavailable"
    }));
}

#[tokio::test]
async fn raster_url_background_repeats_beyond_the_explicit_tile_budget() {
    let (snapshot, geometry) = ready_background_snapshot(
        moli_image::RgbaImage::try_new(1, 1, vec![17, 85, 221, 255]).unwrap(),
        r#"
html,body{margin:0;padding:0;background:white;scrollbar-width:none}
#asset{position:absolute;left:8px;top:8px;width:40px;height:20px;
background:magenta url("__IMAGE_URL__")}
"#,
        "<div id=asset></div>",
    )
    .await
    .expect("ready 1x1 background fixture");
    assert_eq!(geometry, serde_json::json!([8, 8, 40, 20]));
    let image = moli_paint::raster_snapshot(&snapshot).expect("background raster");
    for y in 8..28 {
        for x in 8..48 {
            assert_eq!(image_pixel(&image, x, y), [17, 85, 221, 255], "({x},{y})");
        }
    }
    assert_eq!(image_pixel(&image, 7, 12), [255; 4]);
    assert_eq!(image_pixel(&image, 48, 12), [255; 4]);
    assert_eq!(image_pixel(&image, 12, 28), [255; 4]);
    assert_one_repeat_image(&snapshot);
}

#[tokio::test]
async fn raster_url_background_repeat_preserves_pattern_position_transform_clip_and_alpha() {
    let (snapshot, geometry) = ready_background_snapshot(
        moli_image::RgbaImage::try_new(
            2,
            2,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 0, 0, 0, 0],
        )
        .unwrap(),
        r#"
html,body{margin:0;padding:0;background:white;scrollbar-width:none}
#clip{position:absolute;left:8px;top:8px;width:64px;height:40px;overflow:hidden}
#asset{position:absolute;left:0;top:0;width:96px;height:64px;
transform:translate(3px,5px);background-image:url("__IMAGE_URL__");
background-position:1px 1px;image-rendering:pixelated}
"#,
        "<div id=clip><div id=asset></div></div>",
    )
    .await
    .expect("ready 2x2 background fixture");
    assert_eq!(geometry, serde_json::json!([11, 13, 96, 64]));
    let image = moli_paint::raster_snapshot(&snapshot).expect("periodic background raster");
    // Tile origin is (12,14): element offset + property transform + position.
    // Sampling both near and far pixels rules out stretching one tile to fit.
    for (x, y) in [(12, 14), (26, 22), (68, 44)] {
        assert_eq!(image_pixel(&image, x, y), [255, 0, 0, 255]);
        assert_eq!(image_pixel(&image, x + 1, y), [0, 255, 0, 255]);
        let alpha = image_pixel(&image, x, y + 1);
        assert!((126..=129).contains(&alpha[0]), "{alpha:?}");
        assert!((126..=129).contains(&alpha[1]), "{alpha:?}");
        assert_eq!(&alpha[2..], &[255, 255]);
        assert_eq!(image_pixel(&image, x + 1, y + 1), [255; 4]);
    }
    for (x, y) in [(10, 20), (20, 12), (72, 20), (20, 48)] {
        assert_eq!(image_pixel(&image, x, y), [255; 4], "clip at ({x},{y})");
    }
    assert_one_repeat_image(&snapshot);
}
