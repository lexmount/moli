use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_sizes_viewbox_only_svg_from_available_inline_space() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-auto-size.html")?,
        );
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/inline-svg-auto-size.html"
        ));
        page_vm.vm_mut().eval(&format!(
            "document.documentElement.innerHTML = {}; 'installed'",
            serde_json::to_string(fixture)?,
        ))?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("inline SVG sizing fixture must retain a layout root");
        let geometry = page_vm.vm_mut().eval(
            "JSON.stringify(Object.fromEntries([...document.querySelectorAll('svg')].map(element => { const rect = element.getBoundingClientRect(); return [element.id, [rect.width, rect.height]]; })))",
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        // Measured in Chromium 145. A viewBox supplies a ratio, not a fixed
        // 150px natural size; definite author sizes may still overflow.
        for (id, expected) in [
            ("plus", [24, 24]),
            ("padded-parent", [24, 24]),
            ("wide", [24, 12]),
            ("no-ratio", [300, 150]),
            ("width", [80, 80]),
            ("css-height", [32, 32]),
            ("margin", [20, 13]),
            ("css-ratio", [24, 12]),
            ("percent", [24, 24]),
            ("min", [40, 40]),
            ("max", [20, 20]),
            ("absolute", [64, 64]),
            ("absolute-left", [54, 54]),
            ("absolute-right", [54, 54]),
            ("absolute-insets", [44, 44]),
            ("absolute-margin", [48, 48]),
            ("shrink", [0, 0]),
            ("shrink-percent", [300, 300]),
            ("block-margin", [80, 80]),
            ("block-padding-margin", [80, 43]),
            ("block-percent-margin", [80, 80]),
            ("block-negative-margin", [120, 120]),
            ("block-auto-margin", [100, 100]),
            ("block-visible-margin", [80, 80]),
            ("float-margin", [80, 80]),
            ("block-float-margin", [80, 80]),
            ("inline-float-margin", [80, 80]),
            ("flex-margin", [80, 80]),
            ("grid-margin", [80, 80]),
        ] {
            assert_eq!(
                geometry[id],
                serde_json::json!(expected),
                "Chromium-calibrated SVG geometry mismatch for {id}: {geometry}",
            );
        }
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(pixel(24, 24), [34, 34, 34, 255]);
        assert_eq!(pixel(14, 24), [238, 238, 238, 255]);
        assert_eq!(pixel(30, 30), [238, 238, 238, 255]);
        assert_eq!(pixel(24, 55), [255, 255, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG automatic sizing fixture should run");
}

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
#presentation{position:absolute;left:140px;top:10px;display:block;color:rgb(51,51,51)}
</style>`;
document.body.innerHTML = `
<button id=run><svg id=run-icon class=icon fill="none" viewBox="0 0 20 20"><path d="M2 2v16l16-8z"></path></svg></button>
<svg id=stroke stroke="red" viewBox="0 0 20 20"><path d="M2 10h16"></path></svg>
<svg id=presentation width="20" height="20" fill="none" viewBox="0 0 20 20">
  <rect id=presentation-shape x="2" y="2" width="16" height="16" stroke="currentColor" stroke-width="2"></rect>
</svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        assert_eq!(
            page_vm.vm_mut().eval(
                "[getComputedStyle(document.getElementById('run-icon')).fill,getComputedStyle(document.getElementById('stroke')).stroke,getComputedStyle(document.getElementById('presentation')).fill,getComputedStyle(document.getElementById('presentation-shape')).stroke,getComputedStyle(document.getElementById('presentation-shape')).strokeWidth].join('|')",
            )?,
            "rgb(255, 255, 255)|rgb(0, 128, 0)|none|rgb(51, 51, 51)|2px",
            "presentation attributes must enter Stylo below author CSS, retain SVG unitless lengths, and inherit currentColor normally",
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(170, 50, 1.0))?
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
        assert_eq!(pixel(142, 20), [51, 51, 51, 255]);
        assert_eq!(
            pixel(150, 20),
            [255, 255, 255, 255],
            "fill=none must not be replaced by SVG's initial black fill"
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('presentation').setAttribute('fill','blue')")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("getComputedStyle(document.getElementById('presentation')).fill")?,
            "rgb(0, 0, 255)",
            "changing a presentation attribute must invalidate its computed style",
        );
        let mutated = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(170, 50, 1.0))?
            .expect("mutated SVG presentation fixture must retain a layout root");
        let mutated_raster = moli_paint::raster_snapshot(&mutated)?;
        let center = ((20 * mutated_raster.width + 150) * 4) as usize;
        assert_eq!(
            &mutated_raster.rgba[center..center + 4],
            [0, 0, 255, 255],
            "the invalidated computed fill must reach the inline SVG paint bridge"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG computed-paint fixture should run");
}

// Inline SVG text is painted from a `usvg::Tree` reparsed out of the live
// subtree, so text only survives when the root carries the font that applies
// to it. Two otherwise identical badges draw the same string: one inherits its
// `font-family`/`font-weight` from document CSS, the other authors the very
// same values inside the SVG. Both must produce the same glyph pixels.
//
// This isolates the consumer-visible behavior instead of raw pixel presence:
// once `moli-image` resolves an installed default family, a badge that *lost*
// its inherited font still paints glyphs, so a "some bright pixels exist"
// check would pass while the inherited font was silently replaced. Comparing
// the two badges fails as soon as the serialized root stops carrying the
// computed font, and it also fails if a quoted family breaks the serialized
// `style` attribute (the escaped text is what the parse actually consumes).
// A host that has no installed font at all leaves both badges blank, which
// the glyph-presence assertion reports.
#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_inherited_inline_svg_font_equal_to_an_authored_one() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-inherited-font.html")?,
        );
        // `"DejaVu Sans Mono", monospace` is quoted, so the computed value
        // serializes with double quotes and must survive the `style` attribute.
        page_vm.vm_mut().eval(
            r##"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
svg{position:absolute;top:0;width:90px;height:40px}
#inherited{left:0;font-family:"DejaVu Sans Mono",monospace;font-size:18px;font-weight:bold}
#authored{left:110px;font-size:18px}
</style>`;
document.body.innerHTML = `
<svg id=inherited width=90 height=40 viewBox="0 0 90 40"><rect width=90 height=40 rx=8 fill="#087a65"/><text x=8 y=26 fill="white">SVG OK</text></svg>
<svg id=authored width=90 height=40 viewBox="0 0 90 40"><rect width=90 height=40 rx=8 fill="#087a65"/><text x=8 y=26 fill="white" font-family='"DejaVu Sans Mono", monospace' font-weight=bold font-size=18>SVG OK</text></svg>`;
'installed'
"##,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        // Precondition: the inherited side authors no font at all, so the only
        // way it can match the second badge is by inheriting the CSS font.
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.getElementById('inherited').hasAttribute('font-family'))",
            )?,
            "false",
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(210, 50, 1.0))?
            .expect("inline SVG font fixture must retain a layout root");
        assert_eq!(
            snapshot.svg_images.len(),
            2,
            "both inline SVGs must reach paint as vector resources",
        );
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * raster.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.rgba[index..index + 4]).expect("RGBA pixel")
        };
        // Sample only the interior of each teal rectangle: the rounded corners
        // and the page background stay outside the ROI, so every lit pixel is
        // glyph coverage rather than backdrop.
        let glyph_mask = |left: u32| -> Vec<bool> {
            (8..32)
                .flat_map(|y| (18..72).map(move |x| (x + left, y)))
                .map(|(x, y)| {
                    let [red, green, blue, alpha] = pixel(x, y);
                    alpha == 255 && red > 160 && green > 160 && blue > 160
                })
                .collect()
        };
        for left in [0, 110] {
            // (45, 37) sits below the 26px baseline and inside the rounded
            // rectangle, so it must hold the untouched badge fill: every lit
            // pixel in the ROI above is therefore glyph coverage.
            assert_eq!(pixel(left + 45, 37), [8, 122, 101, 255], "badge fill");
        }
        let inherited = glyph_mask(0);
        let authored = glyph_mask(110);
        let lit = |mask: &[bool]| mask.iter().filter(|painted| **painted).count();
        let (inherited_lit, authored_lit) = (lit(&inherited), lit(&authored));
        assert!(
            inherited_lit > 20 && authored_lit > 20,
            "both badges must paint glyphs; inherited={inherited_lit}, authored={authored_lit}",
        );
        let coverage_gap = inherited_lit.abs_diff(authored_lit);
        assert!(
            coverage_gap <= authored_lit / 50 + 2,
            "an inherited font must paint the same glyph coverage as the same \
             font authored inside the SVG; inherited={inherited_lit}, \
             authored={authored_lit} means the serialized root lost the \
             computed font or the quoted family broke its style attribute",
        );
        // Glyph outlines, not just their count, must match: the same font at the
        // same size can only land on the same pixels up to antialiasing
        // threshold flips.
        let differing = inherited
            .iter()
            .zip(&authored)
            .filter(|(left, right)| left != right)
            .count();
        assert!(
            differing <= authored_lit / 20 + 2,
            "inherited and authored glyphs must occupy the same pixels, {differing} differ",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline SVG inherited-font fixture should run");
}
