use super::*;

fn fallback_snapshot(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
) -> moli_layout::PaintSnapshot {
    page.sync_live_document_style_sources();
    page.screenshot_layout_snapshot(moli_layout::LayoutViewport::new(200, 1600, 1.0))
        .expect("image fallback layout")
        .expect("real layout")
}

async fn assert_image_fallback_fixture(setup: &str, check: &str, cases: u32) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut page =
        new_storage_page_task_executor_test_vm_with_loader("https://image-fallback.test/", &loader);
    page.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    page.eval(&format!(
        "globalThis.fallbackSettled = false; ({setup}).then(() => {{ fallbackSettled = true; }})"
    ))
    .expect("fallback fixture");
    while page.eval("fallbackSettled").expect("fixture completion") != "true" {
        run_next_image_event_task(&mut page, &loader, "fallback image terminal").await;
    }
    let _ = fallback_snapshot(&mut page);
    let result = page
        .eval(&format!("JSON.stringify({check})"))
        .expect("fallback geometry");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).expect("fallback geometry JSON"),
        serde_json::json!({"cases":cases,"failures":[]}),
        "shared Chromium fixture must measure real fallback content"
    );
}

#[tokio::test]
async fn broken_image_fallback_lays_out_icon_alt_text_and_css_constraints() {
    assert_image_fallback_fixture(
        include_str!("../../../../../tests/fixtures/image-fallback-setup.js"),
        include_str!("../../../../../tests/fixtures/image-fallback-check.js"),
        44,
    )
    .await;
}

#[tokio::test]
async fn image_dimension_ratio_hint_survives_css_sizing_and_ignores_relative_attributes() {
    assert_image_fallback_fixture(
        include_str!("../../../../../tests/fixtures/image-fallback-ratio-setup.js"),
        include_str!("../../../../../tests/fixtures/image-fallback-ratio-check.js"),
        12,
    )
    .await;
}

#[tokio::test]
async fn image_fallback_tracks_resource_state_and_alt_mutations_at_render_checkpoints() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut page =
        new_storage_page_task_executor_test_vm_with_loader("https://image-fallback.test/", &loader);
    page.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    page.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::Image),
    );
    page.eval(
        r#"
        document.body.style.cssText = 'font:16px monospace;line-height:20px';
        globalThis.image = new Image();
        image.src = 'missing.png';
        document.body.append(image);
    "#,
    )
    .expect("pending image");
    let pending = page.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert!(
        fallback_snapshot(&mut page).svg_images.is_empty(),
        "pending image is not broken"
    );
    assert_eq!(
        page.eval("[image.width,image.height,image.complete].join('|')")
            .unwrap(),
        "0|0|false"
    );

    page.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        404,
        vec![],
        crate::runtime::RendererSyntheticResponseBody::from_bytes(vec![]),
    )
    .expect("missing image response");
    run_next_image_event_task(&mut page, &loader, "missing image error").await;
    assert_eq!(
        page.eval("image.width").unwrap(),
        "0",
        "resource completion must not replace published geometry synchronously"
    );
    let broken = fallback_snapshot(&mut page);
    assert_eq!(
        broken.svg_images.len(),
        1,
        "fallback has real icon paint content"
    );
    assert!(broken.fragments.iter().any(|fragment| matches!(fragment,
        moli_layout::PaintFragment::SvgImage(image) if image.destination.width == 16.0 && image.destination.height == 16.0)));
    assert_eq!(page.eval("[image.width,image.height,image.naturalWidth,image.naturalHeight,image.complete].join('|')").unwrap(), "16|16|0|0|true");

    page.eval("image.alt = 'abc'").expect("alt mutation");
    assert_eq!(
        page.eval("image.width").unwrap(),
        "16",
        "alt mutation must retain the last layout until rendering"
    );
    let with_alt = fallback_snapshot(&mut page);
    assert!(
        with_alt
            .fragments
            .iter()
            .any(|fragment| matches!(fragment, moli_layout::PaintFragment::GlyphRun(_))),
        "alt text must be shaped and painted"
    );
    assert_eq!(page.eval("image.width > 16 && image.height === 20 && image.childNodes.length === 0 && image.textContent === ''").unwrap(), "true");
    page.eval("image.alt = ''").expect("empty alt");
    assert!(fallback_snapshot(&mut page).svg_images.is_empty());
    assert_eq!(
        page.eval("[image.width,image.height].join('|')").unwrap(),
        "0|0"
    );
    page.eval("image.removeAttribute('alt'); image.title = 'abc'")
        .expect("title fallback");
    let _ = fallback_snapshot(&mut page);
    assert_eq!(
        page.eval("image.width > 16 && image.height === 20")
            .unwrap(),
        "true"
    );

    page.eval("image.src = 'ready.png'")
        .expect("replacement source");
    let pending = page.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert!(
        fallback_snapshot(&mut page).svg_images.is_empty(),
        "a replacement in flight must not keep the old broken icon"
    );
    let pixels =
        moli_image::RgbaImage::try_new(2, 3, [0, 255, 0, 255].repeat(6)).expect("ready pixels");
    let png = moli_image::encode_png(&pixels).expect("ready PNG");
    page.fulfill_pending_subresource_fetch(
        pending[0].internal_id,
        200,
        vec![("Content-Type".into(), "image/png".into())],
        crate::runtime::RendererSyntheticResponseBody::from_bytes(png.bytes),
    )
    .expect("ready image response");
    run_next_image_event_task(&mut page, &loader, "replacement image load").await;
    let ready = fallback_snapshot(&mut page);
    assert!(ready.svg_images.is_empty());
    assert_eq!(
        ready.images.len(),
        1,
        "ready pixels replace generated fallback content"
    );
    assert_eq!(page.eval("[image.width,image.height,image.naturalWidth,image.naturalHeight,document.images.length].join('|')").unwrap(), "2|3|2|3|1");
    assert_eq!(
        broken.svg_images.len(),
        1,
        "old paint snapshots stay immutable"
    );
}

#[tokio::test]
async fn image_size_hints_follow_cascade_and_mutation_without_overriding_css_auto() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut page =
        new_storage_page_task_executor_test_vm_with_loader("https://image-fallback.test/", &loader);
    page.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    page.eval(r#"
        document.body.innerHTML = '<style>.auto {width:auto;height:auto}</style><img id="image" width="40" height="30">';
        globalThis.image = document.getElementById('image');
    "#).expect("dimension hints");
    let _ = fallback_snapshot(&mut page);
    assert_eq!(
        page.eval("[image.width,image.height].join('|')").unwrap(),
        "40|30"
    );
    page.eval("image.width = 50").expect("width mutation");
    let _ = fallback_snapshot(&mut page);
    assert_eq!(
        page.eval("[image.width,image.height].join('|')").unwrap(),
        "50|30"
    );
    assert_eq!(
        page.eval("getComputedStyle(image).aspectRatio").unwrap(),
        "auto 50 / 30",
        "dimension mutations must update the ratio hint, not only used width"
    );
    page.eval("image.className = 'auto'").expect("CSS auto");
    let _ = fallback_snapshot(&mut page);
    assert_eq!(
        page.eval("[image.width,image.height].join('|')").unwrap(),
        "0|0",
        "author auto must override the dimension attributes"
    );
    page.eval("image.className = ''").expect("restore hints");
    let _ = fallback_snapshot(&mut page);
    assert_eq!(
        page.eval("[image.width,image.height].join('|')").unwrap(),
        "50|30"
    );
}

#[test]
fn image_fallback_keeps_the_document_quirks_dimension_rules() {
    for (doctype, compat_mode, expected) in [
        ("<!doctype html>", "CSS1Compat", "0|0"),
        ("", "BackCompat", "40|40"),
    ] {
        let mut page = new_parsed_test_vm(
            "https://image-fallback.test/",
            &format!("{doctype}<html><body><img id='image' width='40'></body></html>"),
        );
        page.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        assert_eq!(page.eval("document.compatMode").unwrap(), compat_mode);
        page.sync_live_document_style_sources();
        page.screenshot_layout_snapshot(moli_layout::LayoutViewport::new(200, 100, 1.0))
            .expect("quirks image layout")
            .expect("real layout");
        assert_eq!(
            page.eval(
                "const image=document.getElementById('image');[image.width,image.height].join('|')"
            )
            .unwrap(),
            expected
        );
    }
}
