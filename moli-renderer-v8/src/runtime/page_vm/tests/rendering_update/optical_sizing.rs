use super::*;

#[tokio::test(flavor = "current_thread")]
async fn font_optical_sizing_shapes_real_variable_font_advances() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/font-optical-sizing.html")?,
        );
        let variable = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-opsz.ttf"
        ));
        let static_font = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        ));
        let encoded_variable = base64::engine::general_purpose::STANDARD.encode(variable);
        let encoded_static = base64::engine::general_purpose::STANDARD.encode(static_font);
        // The 1000-UPEM fixture has opsz 8/16/64 masters with 400/600/1200-unit
        // advances. At opsz 32 its advance is 800 units. Check actual shaping,
        // not just the variation settings passed to Parley.
        let cases = [
            ("default", 16.0, "", 19.2),
            ("small", 8.0, "", 6.4),
            ("auto", 32.0, "font-optical-sizing:auto", 51.2),
            ("none", 32.0, "font-optical-sizing:none", 38.4),
            ("explicit", 32.0, "font-variation-settings:'opsz' 16", 38.4),
            (
                "explicit_none",
                32.0,
                "font-optical-sizing:none;font-variation-settings:'opsz' 64",
                76.8,
            ),
            ("other_axis", 32.0, "font-variation-settings:'wght' 700", 51.2),
            ("clamp_small", 4.0, "", 3.2),
            ("clamp_large", 96.0, "", 230.4),
            ("explicit_clamp", 32.0, "font-variation-settings:'opsz' 128", 76.8),
            ("static_auto", 32.0, "font-family:MoliStatic", 38.4),
            (
                "static_explicit",
                32.0,
                "font-family:MoliStatic;font-variation-settings:'opsz' 64",
                38.4,
            ),
        ];
        let content = cases
            .iter()
            .map(|(id, size, style, _)| {
                format!("<div><span id={id} style=\"font-size:{size}px;{style}\">AA</span></div>")
            })
            .collect::<String>();
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{ font-family:MoliOptical; src:url(data:font/ttf;base64,{encoded_variable}) format('truetype') }}
@font-face {{ font-family:MoliStatic; src:url(data:font/ttf;base64,{encoded_static}) format('truetype') }}
html,body {{ margin:0; padding:0 }}
span {{ display:inline-block; font-family:MoliOptical; line-height:1; white-space:pre }}
</style>`;
document.body.innerHTML = `{content}`;
'installed'
"#
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 640, 1.0))?
            .expect("optical sizing fixture should have a layout root");
        let runs = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) => Some(run),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), cases.len());
        let widths: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval(
            "JSON.stringify(Object.fromEntries(Array.from(document.querySelectorAll('span'), element => [element.id, element.getBoundingClientRect().width])))",
        )?)?;
        for ((id, size, _, expected), run) in cases.iter().zip(runs) {
            let font = snapshot.font(run.font).expect("shaped font");
            let expected_font: &[u8] = if id.starts_with("static_") {
                static_font
            } else {
                variable
            };
            assert_eq!(font.font.data.as_ref(), expected_font, "{id}: actual face");
            assert_eq!(
                run.font_size, *size,
                "{id}: optical size must not rescale CSS font-size"
            );
            assert_eq!(run.glyphs.len(), 2, "{id}: two shaped As");
            let advance = run.glyphs[1].x - run.glyphs[0].x;
            assert!(
                (advance * 2.0 - expected).abs() <= 0.01,
                "{id}: expected width {expected}, shaped advance {advance}"
            );
            let width = widths[id].as_f64().expect("measured width");
            assert!(
                (width - f64::from(*expected)).abs() <= 0.05,
                "{id}: expected width {expected}, got {width}"
            );
        }
        // Reusing the same node and document caches must honor style changes.
        page_vm.vm_mut().eval(
            "document.getElementById('auto').style.fontOpticalSizing = 'none'",
        )?;
        page_vm.vm_mut().publish_layout_for_test()?;
        let width = page_vm.vm_mut()
            .eval("document.getElementById('auto').getBoundingClientRect().width")?
            .parse::<f32>()?;
        assert!((width - 38.4).abs() <= 0.05, "none after auto: {width}");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("real-font optical sizing test should run");
}
