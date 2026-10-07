// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn baseline_atomic_inline_keeps_parent_strut_descent_in_flex_header() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/baseline-atomic-strut.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#bar{display:flex;align-items:center;box-sizing:border-box;width:200px;height:60px;padding:6px}
#header{display:flex;align-items:center}
/* A zero-size font makes the 3px half-leading below the baseline deterministic. */
#wrapper{display:block;font-size:0;line-height:6px}
#atomic{display:inline-block;width:48px;height:48px;background:blue}
</style>`;
document.body.innerHTML = `<div id=bar><header id=header><div id=wrapper><span id=atomic></span></div></header></div>`;
'installed'
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        page_vm.vm_mut().publish_layout_for_test()?;
        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['bar','header','wrapper','atomic'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("bar", [0.0, 0.0, 200.0, 60.0]),
            ("header", [6.0, 4.5, 48.0, 51.0]),
            ("wrapper", [6.0, 4.5, 48.0, 51.0]),
            ("atomic", [6.0, 4.5, 48.0, 48.0]),
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
    .expect("baseline atomic strut fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn atomic_inline_location_uses_the_global_layout_unit_grid() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-layout-unit.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#line{margin:0;white-space:nowrap;font:16px/75px sans-serif}
#atomic{display:inline-block;position:relative;left:.1px}
</style>`;
document.body.innerHTML = `<p id=line>All the <span id=atomic>words</span> after</p>`;
'installed'
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        page_vm.vm_mut().publish_layout_for_test()?;
        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const line=document.getElementById('line');const atomic=document.getElementById('atomic');const rect=node=>{const range=document.createRange();range.selectNodeContents(node);const value=range.getBoundingClientRect();return [value.x,value.right]};const box=atomic.getBoundingClientRect();return {preceding:rect(line.firstChild),atomic:[box.x,box.right],text:rect(atomic)}})())"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        let number = |group: &str, index: usize| {
            geometry[group][index]
                .as_f64()
                .unwrap_or_else(|| panic!("missing {group}[{index}] in {geometry}"))
                as f32
        };
        let preceding_right = number("preceding", 1);
        let atomic_x = number("atomic", 0);
        let text_x = number("text", 0);
        let geometry_epsilon = 0.0001;
        let unrounded_atomic_x = preceding_right + 0.1;
        let expected_atomic_x = (unrounded_atomic_x * 64.0).round() / 64.0;

        assert!(
            (unrounded_atomic_x * 64.0 - (unrounded_atomic_x * 64.0).round()).abs() > 0.05,
            "fixture must place the unrounded atomic origin away from the 1/64 grid: {geometry}"
        );
        assert!(
            (atomic_x - expected_atomic_x).abs() <= geometry_epsilon,
            "the atomic outer placement must use the global 1/64 layout grid; expected {expected_atomic_x}: {geometry}"
        );
        assert!(
            (text_x - atomic_x).abs() <= geometry_epsilon,
            "the atomic IFC must paint its first glyph at the rounded box origin: {geometry}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic inline layout-unit fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rounded_flex_max_content_width_does_not_rewrap_its_text() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-intrinsic-rounding.html")?,
        );
        let encoded_font = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(encoded_font);
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{ font-family:MoliAhem; src:url(data:font/woff2;base64,{encoded}) format('woff2') }}
* {{ box-sizing:border-box }}
html,body {{ margin:0 }}
.row {{ display:flex; justify-content:center; gap:8px; padding-top:20px }}
.item {{ display:inline-flex; align-items:center; gap:8px; padding:10px 14px; border:1px solid; border-radius:24px }}
.icon {{ width:20px; height:20px; flex:0 0 auto }}
.text,#constrained {{ font-family:MoliAhem; font-size:9px }}
#constrained {{ width:80px }}
</style>`;
document.body.innerHTML = `<div class=row><div class=item id=ask><i class=icon></i><span class=text id=ask-text>Ask about files</span></div></div><div id=constrained>Ask about files</div>`;
'installed'
"#
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        page_vm.vm_mut().publish_layout_for_test()?;
        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['ask','ask-text','constrained'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        // Chromium's Ahem `normal` line-height follows the font metrics: 9px
        // for one line and 18px after wrapping. The icon and padding, rather
        // than an inflated synthetic line-height, establish the 42px pill.
        for (id, expected) in [
            ("ask", [139.015625, 42.0]),
            ("ask-text", [81.015625, 9.0]),
            ("constrained", [80.0, 18.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry");
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rounded flex max-content fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn block_in_inline_cssom_uses_structural_ancestors() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/block-in-inline-ancestry.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;font-size:0}
#parent{position:relative}
#block{height:20px}
#absolute{position:absolute;height:10px}
</style>`;
document.body.innerHTML = `<span id=parent><div id=block><div id=absolute></div></div></span>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 100, 1.0))?
            .expect("block-in-inline fixture must retain a layout root");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"[
document.getElementById('block').offsetParent === document.getElementById('parent'),
document.getElementById('absolute').offsetParent === document.getElementById('parent')
].join('|')"#,
            )?,
            "true|true",
            "formatting promotion must preserve LayoutObject ancestry for CSSOM",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("block-in-inline CSSOM ancestry fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn elements_from_point_filters_html_text_hits_from_the_penetrating_list() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/elements-from-point-negative-margin.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0}</style>';
document.body.innerHTML = `<div id=outer style="background:yellow">
  <div id=inner style="width:100px;height:100px;margin-bottom:-100px;background:lime"></div>
  Hello
</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 120, 1.0))?
            .expect("negative-margin hit-test fixture must retain a layout root");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"const rect = document.getElementById('outer').getBoundingClientRect();
const x = rect.left + 1;
const y = rect.top + 1;
[
  document.elementFromPoint(x, y)?.id,
  document.elementsFromPoint(x, y).map(element => element.id || element.localName).join(',')
].join('|')"#,
            )?,
            "outer|inner,outer,body,html",
            "single and penetrating hit tests must apply their distinct text-node policies",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("negative-margin elementsFromPoint fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_offset_metrics_keep_empty_trailing_space_anchor_after_bordered_inline() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-offset-fragments.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body { margin:0 }
.container { position:relative; width:80px; height:70px; padding:10px; font:10px/10px sans-serif }
.reference { border:1px solid transparent; padding:0 6px }
</style>`;
document.body.innerHTML = `
<div class=container><br><span class=reference></span><span class=target> </span></div>`;
'installed'
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 90, 1.0))?
            .expect("inline offset fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"(()=>{const target=document.querySelector('.target');const reference=target.previousSibling;return JSON.stringify({reference:[reference.offsetLeft,reference.offsetTop,reference.offsetWidth,reference.offsetHeight],target:[target.offsetLeft,target.offsetTop,target.offsetWidth,target.offsetHeight]})})()"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        let reference = geometry["reference"]
            .as_array()
            .unwrap_or_else(|| panic!("missing reference metrics: {geometry}"));
        let target = geometry["target"]
            .as_array()
            .unwrap_or_else(|| panic!("missing target metrics: {geometry}"));
        let number = |values: &[serde_json::Value], axis: usize| {
            values[axis].as_f64().expect("numeric offset geometry")
        };
        assert_eq!(number(reference, 2), 14.0, "reference width: {geometry}");
        assert_eq!(
            number(target, 0),
            number(reference, 0) + number(reference, 2),
            "the empty inline starts after the reference fragment: {geometry}"
        );
        assert_eq!(
            number(target, 2),
            0.0,
            "the empty inline remains zero-width: {geometry}"
        );
        assert!(
            number(target, 3) > 0.0,
            "a zero-width inline still contributes its non-zero height: {geometry}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline offset fragment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stylesheet_lifecycle_registers_only_the_current_documents_data_web_fonts() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-web-font.html")?,
        );
        let encoded_font = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(encoded_font);
        let install_font = |family: &str| {
            format!(
                r#"
(() => {{
const style = document.createElement('style');
style.id = 'layout-web-font';
style.textContent = `
  @font-face {{
    font-family: {family};
    src: url(data:font/woff2;base64,{encoded}) format('woff2');
  }}
  body {{ font: 20px {family}; }}
`;
document.head.append(style);
document.body.textContent = 'AAAA';
return 'installed';
}})()
"#
            )
        };

        page_vm.vm_mut().eval(&install_font("FirstDocumentFace"))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1),
            "the stylesheet lifecycle should discover, decode, and register the current @font-face before layout"
        );
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("current Document should have a layout root");
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1),
            "layout should reuse the font registered by the stylesheet lifecycle"
        );
        let expected_ttf = wuff::decompress_woff2(encoded_font).expect("valid WOFF2 fixture");
        assert!(
            snapshot
                .fonts
                .iter()
                .any(|resource| resource.font.data.as_ref() == expected_ttf),
            "the snapshot must shape text with the decoded deterministic web font"
        );
        let first_document_cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(
            first_document_cache.3.is_some(),
            "a successful screenshot layout must publish geometry"
        );

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write('<!doctype html><html><head></head><body></body></html>');
document.close();
'replaced'
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (0, 0, 0),
            "document.open() must replace the document-owned font sidecar"
        );
        let replacement_cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(
            replacement_cache.3.is_none(),
            "the main Document owner transition must clear its old layout snapshot"
        );
        assert_eq!(replacement_cache.2, first_document_cache.2);

        page_vm
            .vm_mut()
            .eval(&install_font("ReplacementDocumentFace"))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1),
            "the replacement document should register only its own @font-face before layout"
        );
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("replacement Document should have a layout root");
        let replacement_cache_after_layout =
            page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(replacement_cache_after_layout.3.is_some());
        assert_eq!(replacement_cache_after_layout.2, first_document_cache.2 + 1);
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (1, 1, 1)
        );
        page_vm
            .vm_mut()
            .eval("document.querySelector('#layout-web-font').remove(); 'removed'")?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("replacement Document should remain layoutable");
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (0, 0, 0),
            "the next one-shot demand must revoke a removed @font-face without a generation fence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document-owned data web-font layout test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn same_family_unicode_range_faces_shape_mixed_text_with_both_subsets() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/segmented-web-font.html")?,
        );
        let latin = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        ));
        let cjk = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-cjk.ttf"
        ));
        let latin = base64::engine::general_purpose::STANDARD.encode(latin);
        let cjk = base64::engine::general_purpose::STANDARD.encode(cjk);
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{
  font-family: MoliSegmented;
  src: url(data:font/ttf;base64,{latin}) format('truetype');
  font-weight: 400;
  unicode-range: U+0000-00FF;
}}
@font-face {{
  font-family: MoliSegmented;
  src: url(data:font/ttf;base64,{cjk}) format('truetype');
  font-weight: 400;
  unicode-range: U+4E00-9FFF;
}}
body {{ margin: 0; font: 32px/40px MoliSegmented, sans-serif; }}
</style>`;
document.body.textContent = 'R中';
'installed'
"#
        ))?;

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 80, 1.0))?
            .expect("segmented web-font fixture should have a layout root");
        let used_fonts = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) => snapshot.font(run.font),
                _ => None,
            })
            .map(|font| font.font.data.as_ref())
            .collect::<Vec<_>>();
        assert!(
            used_fonts.iter().any(|font| *font
                == include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-ahem.ttf"
                ))),
            "the Latin character must use the Latin subset"
        );
        assert!(
            used_fonts.iter().any(|font| *font
                == include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-cjk.ttf"
                ))),
            "the CJK character must use the CJK subset"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("segmented web-font layout test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cjk_regular_face_uses_chromium_synthetic_bold_threshold() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/cjk-synthetic-bold.html")?,
        );
        let cjk = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-cjk.ttf"
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(cjk);
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{ font-family: MoliCJKRegular; src: url(data:font/ttf;base64,{encoded}) format('truetype'); font-weight: 400; }}
html, body {{ margin: 0; padding: 0; background: white; }}
.case {{ font-family: MoliCJKRegular; font-size: 32px; line-height: 40px; height: 40px; color: black; }}
#normal {{ font-weight: 400; }}
#medium {{ font-weight: 500; }}
#semibold {{ font-weight: 600; }}
#disabled {{ font-weight: 600; font-synthesis-weight: none; }}
</style>`;
document.body.innerHTML = '<div id=normal class=case>中</div><div id=medium class=case>中</div><div id=semibold class=case>中</div><div id=disabled class=case>中</div>';
'installed'
"#
        ))?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 160, 1.0))?
            .expect("CJK fixture should have a layout root");
        let runs = snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) => Some(run),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 4, "the fixture should shape one run per line");
        assert_eq!(runs[0].glyph_embolden, moli_layout::PaintPoint::ZERO);
        assert_eq!(
            runs[1].glyph_embolden,
            moli_layout::PaintPoint::ZERO,
            "CSS weight 500 must remain below Chromium's synthetic-bold threshold"
        );
        assert!(
            runs[2].glyph_embolden.x > 0.0 && runs[2].glyph_embolden.y > 0.0,
            "the unavailable CJK 600 face should preserve Parley's faux-bold request"
        );
        assert_eq!(
            runs[3].glyph_embolden,
            moli_layout::PaintPoint::ZERO,
            "font-synthesis-weight:none must suppress faux bold"
        );
        for run in &runs {
            assert_eq!(snapshot.font(run.font).expect("run font").font.data.as_ref(), cjk);
        }

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let ink = |top: u32| {
            (top..top + 40)
                .flat_map(|y| (0..40).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let offset = ((y * image.width + x) * 4) as usize;
                    let pixel = &image.rgba[offset..offset + 4];
                    u64::from(255 - pixel[0])
                        + u64::from(255 - pixel[1])
                        + u64::from(255 - pixel[2])
                })
                .sum::<u64>()
        };
        let normal_ink = ink(0);
        let medium_ink = ink(40);
        let semibold_ink = ink(80);
        let disabled_ink = ink(120);
        assert_eq!(
            medium_ink, normal_ink,
            "CSS weight 500 must raster exactly the regular CJK face"
        );
        assert!(
            semibold_ink > normal_ink,
            "Vello CPU must raster faux bold with more ink: normal={normal_ink}, semibold={semibold_ink}"
        );
        assert_eq!(
            disabled_ink, normal_ink,
            "disabling synthesis must raster exactly the regular CJK face"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CJK synthetic-bold rendering test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn form_control_ua_styles_match_the_chromium_headless_contract() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/form-control-ua.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `
  <input id=text-input>
  <input id=checkbox type=checkbox>
  <input id=radio type=radio>
  <input id=range type=range>
  <input id=color type=color>
  <input id=disabled-text disabled>
  <textarea id=textarea></textarea>
  <select id=select><option>A</option></select>
  <select id=disabled-select disabled><option>A</option></select>
  <output id=output></output>
  <meter id=meter></meter>
  <progress id=progress></progress>
`;
'installed'
"#,
        )?;

        let computed = page_vm.vm_mut().eval(
            r#"
(() => {
  const read = id => {
    const style = getComputedStyle(document.getElementById(id));
    return {
      appearance: style.appearance,
      backgroundColor: style.backgroundColor,
      borderTopColor: style.borderTopColor,
      borderTopStyle: style.borderTopStyle,
      borderTopWidth: style.borderTopWidth,
      boxSizing: style.boxSizing,
      color: style.color,
      cursor: style.cursor,
      display: style.display,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      height: style.height,
      margin: style.margin,
      opacity: style.opacity,
      overflow: style.overflow,
      overflowClipMargin: style.getPropertyValue('overflow-clip-margin'),
      overflowWrap: style.getPropertyValue('overflow-wrap'),
      padding: style.padding,
      textAlign: style.textAlign,
      whiteSpace: style.whiteSpace,
      width: style.width
    };
  };
  return JSON.stringify({
    textInput: read('text-input'),
    checkbox: read('checkbox'),
    radio: read('radio'),
    range: read('range'),
    color: read('color'),
    disabledText: read('disabled-text'),
    textarea: read('textarea'),
    select: read('select'),
    disabledSelect: read('disabled-select'),
    output: read('output'),
    meter: read('meter'),
    progress: read('progress')
  });
})()
"#,
        )?;
        let computed: serde_json::Value = serde_json::from_str(&computed)?;

        for control in [
            "textInput",
            "checkbox",
            "radio",
            "range",
            "color",
            "disabledText",
            "textarea",
            "select",
            "disabledSelect",
        ] {
            assert_eq!(computed[control]["display"], "inline-block", "{control}");
            assert_eq!(computed[control]["appearance"], "auto", "{control}");
        }
        assert_eq!(computed["textInput"]["fontFamily"], "Arial, sans-serif");
        let control_font_size = computed["textInput"]["fontSize"]
            .as_str()
            .expect("computed control font-size")
            .trim_end_matches("px")
            .parse::<f32>()?;
        assert!((control_font_size - 13.3333).abs() <= 0.01);
        assert_eq!(computed["textInput"]["boxSizing"], "border-box");
        assert_eq!(computed["textInput"]["padding"], "1px 2px");
        assert_eq!(computed["textInput"]["borderTopWidth"], "2px");
        assert_eq!(computed["textInput"]["borderTopStyle"], "inset");
        assert_eq!(
            computed["textInput"]["borderTopColor"],
            "rgb(118, 118, 118)"
        );
        assert_eq!(
            computed["textInput"]["backgroundColor"],
            "rgb(255, 255, 255)"
        );
        assert_eq!(computed["textInput"]["color"], "rgb(0, 0, 0)");
        assert_eq!(computed["textInput"]["cursor"], "text");
        assert_eq!(computed["textInput"]["overflow"], "clip");
        assert_eq!(computed["textInput"]["overflowClipMargin"], "0px");
        assert_eq!(computed["textInput"]["textAlign"], "start");

        assert_eq!(computed["checkbox"]["boxSizing"], "border-box");
        assert_eq!(computed["checkbox"]["margin"], "3px 3px 3px 4px");
        assert_eq!(computed["checkbox"]["padding"], "0px");
        assert_eq!(computed["checkbox"]["borderTopWidth"], "0px");
        assert_eq!(computed["checkbox"]["backgroundColor"], "rgba(0, 0, 0, 0)");
        assert_eq!(computed["checkbox"]["cursor"], "default");
        assert_eq!(computed["radio"]["boxSizing"], "border-box");
        assert_eq!(computed["radio"]["margin"], "3px 3px 0px 5px");

        assert_eq!(computed["range"]["margin"], "2px");
        assert_eq!(computed["range"]["padding"], "0px");
        assert_eq!(computed["range"]["borderTopWidth"], "0px");
        assert_eq!(computed["range"]["cursor"], "default");
        assert_eq!(computed["range"]["overflow"], "visible");

        assert_eq!(computed["color"]["boxSizing"], "border-box");
        assert_eq!(computed["color"]["width"], "50px");
        assert_eq!(computed["color"]["height"], "27px");
        assert_eq!(computed["color"]["padding"], "1px 2px");
        assert_eq!(computed["color"]["borderTopWidth"], "1px");

        assert_eq!(computed["disabledText"]["cursor"], "default");
        assert_eq!(
            computed["disabledText"]["backgroundColor"],
            "rgba(239, 239, 239, 0.3)"
        );
        assert_eq!(
            computed["disabledText"]["borderTopColor"],
            "rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(computed["disabledText"]["color"], "rgb(84, 84, 84)");

        assert_eq!(computed["textarea"]["fontFamily"], "monospace");
        assert_eq!(computed["textarea"]["padding"], "2px");
        assert_eq!(computed["textarea"]["borderTopWidth"], "1px");
        assert_eq!(computed["textarea"]["borderTopStyle"], "solid");
        assert_eq!(computed["textarea"]["whiteSpace"], "pre-wrap");
        assert_eq!(computed["textarea"]["overflow"], "auto");
        assert_eq!(computed["textarea"]["overflowWrap"], "break-word");

        assert_eq!(computed["select"]["boxSizing"], "border-box");
        assert_eq!(computed["select"]["borderTopWidth"], "1px");
        assert_eq!(computed["select"]["borderTopStyle"], "solid");
        assert_eq!(computed["select"]["whiteSpace"], "pre");
        assert_eq!(computed["select"]["cursor"], "default");
        assert_eq!(computed["disabledSelect"]["opacity"], "0.7");
        assert_eq!(
            computed["disabledSelect"]["borderTopColor"],
            "rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(computed["disabledSelect"]["color"], "rgb(109, 109, 109)");

        assert_eq!(computed["output"]["display"], "inline");
        assert_eq!(computed["meter"]["display"], "inline-block");
        assert_eq!(computed["meter"]["boxSizing"], "border-box");
        assert_eq!(computed["meter"]["width"], "80px");
        assert_eq!(computed["meter"]["height"], "16px");
        assert_eq!(computed["progress"]["display"], "inline-block");
        assert_eq!(computed["progress"]["boxSizing"], "border-box");
        assert_eq!(computed["progress"]["width"], "160px");
        assert_eq!(computed["progress"]["height"], "16px");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("form-control UA stylesheet fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn button_ua_defaults_and_flow_content_alignment_match_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/button-ua-layout.html")?,
        );
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        )));
        let css = format!(
            r#"
@font-face{{font-family:MoliAhem;src:url(data:font/ttf;base64,{font}) format('truetype')}}
html,body{{margin:0;padding:0}}
.control{{display:block;width:108px;height:44px;padding:0;border:0;font-family:MoliAhem;font-size:20px;line-height:20px;font-weight:400}}
#button{{background:rgb(1,2,3);color:rgb(11,12,13)}}
#input{{background:rgb(4,5,6);color:rgb(21,22,23)}}
#defaults,#disabled{{visibility:hidden}}
"#
        );
        page_vm.vm_mut().eval(&format!(
            "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
            serde_json::to_string(&css)?,
            serde_json::to_string(
                "<button id=button class=control>BBBB</button><input id=input class=control type=submit value=BBBB><button id=defaults>Default</button><button id=disabled disabled>Disabled</button>"
            )?,
        ))?;

        let computed = page_vm.vm_mut().eval(
            r#"
(() => {
  const read = id => {
    const style = getComputedStyle(document.getElementById(id));
    return {
      display: style.display,
      boxSizing: style.boxSizing,
      textAlign: style.textAlign,
      appearance: style.appearance,
      margin: style.margin,
      padding: style.padding,
      borderTopWidth: style.borderTopWidth,
      borderTopStyle: style.borderTopStyle,
      backgroundColor: style.backgroundColor,
      borderTopColor: style.borderTopColor,
      color: style.color,
      cursor: style.cursor,
      overflow: style.overflow,
      whiteSpace: style.whiteSpace,
      userSelect: style.userSelect,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      fontWeight: style.fontWeight,
      lineHeight: style.lineHeight
    };
  };
  return JSON.stringify({
    button: read('button'),
    input: read('input'),
    defaults: read('defaults'),
    disabled: read('disabled')
  });
})()
"#,
        )?;
        let computed: serde_json::Value = serde_json::from_str(&computed)?;
        assert_eq!(computed["button"]["display"], "block");
        assert_eq!(computed["button"]["boxSizing"], "border-box");
        assert_eq!(computed["button"]["textAlign"], "center");
        assert_eq!(computed["button"]["overflow"], "visible");
        assert_eq!(computed["button"]["whiteSpace"], "normal");
        assert_eq!(computed["input"]["display"], "block");
        assert_eq!(computed["input"]["boxSizing"], "border-box");
        assert_eq!(computed["input"]["textAlign"], "center");
        assert_eq!(computed["input"]["overflow"], "clip");
        assert_eq!(computed["input"]["whiteSpace"], "pre");
        assert_eq!(computed["input"]["userSelect"], "none");
        assert_eq!(computed["defaults"]["display"], "inline-block");
        assert_eq!(computed["defaults"]["boxSizing"], "border-box");
        assert_eq!(computed["defaults"]["textAlign"], "center");
        assert_eq!(computed["defaults"]["appearance"], "auto");
        assert_eq!(computed["defaults"]["margin"], "0px");
        assert_eq!(computed["defaults"]["padding"], "1px 6px");
        assert_eq!(computed["defaults"]["borderTopWidth"], "2px");
        assert_eq!(computed["defaults"]["borderTopStyle"], "outset");
        assert_eq!(computed["defaults"]["cursor"], "default");
        assert_eq!(computed["defaults"]["fontFamily"], "Arial, sans-serif");
        let default_font_size = computed["defaults"]["fontSize"]
            .as_str()
            .expect("computed font-size string")
            .trim_end_matches("px")
            .parse::<f32>()?;
        assert!((default_font_size - 13.3333).abs() <= 0.01);
        assert_eq!(computed["defaults"]["fontWeight"], "400");
        assert_eq!(computed["defaults"]["lineHeight"], "normal");
        assert_eq!(
            computed["disabled"]["backgroundColor"],
            "rgba(239, 239, 239, 0.3)"
        );
        assert_eq!(
            computed["disabled"]["borderTopColor"],
            "rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(computed["disabled"]["color"], "rgba(16, 16, 16, 0.3)");

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 120, 1.0))?
            .ok_or_else(|| anyhow::anyhow!("button fixture lost its layout root"))?;
        let rgb = |red: u8, green: u8, blue: u8| {
            moli_layout::PaintColor::new(
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                1.0,
            )
        };
        let glyphs = |color| {
            snapshot
                .fragments
                .iter()
                .filter_map(|fragment| match fragment {
                    moli_layout::PaintFragment::GlyphRun(run) if run.color == color => {
                        Some(run.glyphs_in_surface())
                    }
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>()
        };
        let assert_label = |label: &str,
                            actual: &[moli_layout::PaintGlyph],
                            expected_y: f32| {
            assert_eq!(actual.len(), 4, "{label}: {actual:?}");
            for (index, glyph) in actual.iter().enumerate() {
                let expected_x = 30.0 + index as f32 * 12.0;
                assert!(
                    (glyph.x - expected_x).abs() <= 0.05
                        && (glyph.y - expected_y).abs() <= 0.05,
                    "{label}[{index}]: actual={glyph:?}, expected=({expected_x}, {expected_y})"
                );
            }
        };
        assert_label("button label", &glyphs(rgb(11, 12, 13)), 28.0);
        assert_label("input label", &glyphs(rgb(21, 22, 23)), 72.0);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("button UA/layout fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn color_emoji_web_font_rasterizes_cbdt_png() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/color-emoji.html")?,
        );
        let color_emoji = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/noto-color-emoji-cbdt-subset.ttf.b64"
        ))
        .split_ascii_whitespace()
        .collect::<String>();
        let css = format!(
            r#"
@font-face{{font-family:MoliColorEmoji;src:url(data:font/ttf;base64,{color_emoji}) format('truetype')}}
html,body{{margin:0;padding:0;background:white}}
#emoji{{font-family:MoliColorEmoji;font-size:64px;line-height:80px;color:rgb(1,2,3);font-synthesis:none}}
"#
        );
        page_vm.vm_mut().eval(&format!(
            "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
            serde_json::to_string(&css)?,
            serde_json::to_string("<div id=emoji>®️⁉️8️⃣</div>")?,
        ))?;

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 100, 1.0))?
            .ok_or_else(|| anyhow::anyhow!("color emoji fixture lost its layout root"))?;
        let decoded_font = base64::engine::general_purpose::STANDARD.decode(&color_emoji)?;
        assert!(
            snapshot
                .fonts
                .iter()
                .any(|font| font.font.data.as_ref() == decoded_font),
            "the emoji run must retain the downloaded CBDT font in the owned snapshot"
        );

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let saturated_pixels = image
            .rgba
            .as_chunks::<4>().0.iter()
            .filter(|pixel| {
                let [red, green, blue, alpha] = **pixel;
                let max = red.max(green).max(blue);
                let min = red.min(green).min(blue);
                alpha > 0 && max > 100 && max.saturating_sub(min) > 40
            })
            .count();
        assert!(
            saturated_pixels > 50,
            "CBDT glyphs must retain their embedded colors instead of using the near-black CSS text color; saturated_pixels={saturated_pixels}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("color emoji fixture should shape and rasterize");
}

#[tokio::test(flavor = "current_thread")]
async fn layout_demand_matches_the_fixed_font_inline_corpus() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-inline-corpus.html")?,
        );
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let latin = encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.ttf"
        )));
        let hebrew_emoji = encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-hebrew-emoji.ttf"
        )));
        let cjk = encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-cjk.ttf"
        )));
        let fixed_css = format!(
            r#"
@font-face{{font-family:MoliAhem;src:url(data:font/ttf;base64,{latin}) format('truetype')}}
@font-face{{font-family:MoliHebrewEmoji;src:url(data:font/ttf;base64,{hebrew_emoji}) format('truetype')}}
@font-face{{font-family:MoliCJK;src:url(data:font/ttf;base64,{cjk}) format('truetype')}}
html,body{{margin:0;padding:0}}
.fixed{{font-family:MoliAhem,MoliHebrewEmoji,MoliCJK;font-size:20px;line-height:20px}}
"#
        );
        let render = |page_vm: &mut PageVm,
                      case_css: &str,
                      body: &str|
         -> anyhow::Result<moli_layout::PaintSnapshot> {
            page_vm.vm_mut().eval(&format!(
                "document.documentElement.lang='en';document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML={};'installed'",
                serde_json::to_string(&(fixed_css.clone() + case_css))?,
                serde_json::to_string(body)?,
            ))?;
            page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 200, 1.0))?
                .ok_or_else(|| anyhow::anyhow!("inline fixture lost its layout root"))
        };
        let shared = render(
            &mut page_vm,
            r#"
#stream{width:80px}
#stream::before{content:'X';color:rgb(1,2,3)}
#a{color:rgb(11,12,13)}
#nested{color:rgb(21,22,23);margin:0 2px;padding:0 3px;border-left:1px solid;border-right:1px solid;background:rgb(4,5,6)}
#c{color:rgb(31,32,33)}#d{color:rgb(41,42,43)}#e{color:rgb(51,52,53)}
#trailing{color:rgb(61,62,63)}#after-trailing{width:10px;height:5px;background:rgb(71,72,73)}
"#,
            r#"<div id=stream class=fixed><span id=a>A</span><span id=nested>B</span><span id=c>C</span><br><span id=d>D</span><span id=e>E</span></div><div id=trailing class=fixed>A<br></div><div id=after-trailing></div>"#,
        )?;
        let whitespace = render(
            &mut page_vm,
            r#"
#collapse{width:60px}#upper{text-transform:uppercase;color:rgb(11,12,13)}
#ca{color:rgb(21,22,23)}#cb{color:rgb(31,32,33)}
#cjk{width:41px;font-family:MoliCJK,MoliHebrewEmoji;color:rgb(41,42,43)}
#preserve{white-space-collapse:preserve-breaks;width:60px;color:rgb(51,52,53)}
#breakspaces{white-space-collapse:break-spaces;width:36px;color:rgb(61,62,63)}
#nowrap{white-space:nowrap;width:24px;color:rgb(71,72,73)}
"#,
            "<div id=collapse class=fixed><span id=ca>A</span>  <span id=upper>ab</span>   <span id=cb>B</span></div><div id=cjk class=fixed>中\n文😀</div><div id=preserve class=fixed>A   B\nC</div><div id=breakspaces class=fixed>A   B</div><div id=nowrap class=fixed>ABC</div>",
        )?;
        let bidi = render(
            &mut page_vm,
            r#"
#bidi{direction:rtl;width:120px}#hebrew{font-family:MoliHebrewEmoji;color:rgb(11,12,13)}
#latin{direction:ltr;unicode-bidi:isolate;color:rgb(21,22,23)}
#emoji{font-family:MoliHebrewEmoji;color:rgb(31,32,33)}
#spacing{width:120px;letter-spacing:2px;word-spacing:4px;text-indent:12px;font-weight:625;font-stretch:87.5%;font-style:italic;color:rgb(41,42,43)}
"#,
            "<div id=bidi class=fixed><span id=hebrew>אב</span><span id=latin>AB</span><span id=emoji>😀</span></div><div id=spacing class=fixed>A A</div>",
        )?;
        let vertical = render(
            &mut page_vm,
            r#"
#align{width:140px}.atomic{display:inline-block}
#top{width:10px;height:30px;vertical-align:top;background:rgb(41,42,43)}
#bottom{width:10px;height:10px;vertical-align:bottom;background:rgb(51,52,53)}
#middle{width:10px;height:8px;vertical-align:middle;background:rgb(61,62,63)}
#raised{vertical-align:10px;color:rgb(71,72,73);background:rgb(1,2,3)}
#after{width:10px;height:5px;background:rgb(81,82,83)}
"#,
            "<div id=align class=fixed><span id=strut>A</span><span id=top class=atomic></span><span id=bottom class=atomic></span><span id=middle class=atomic></span><span id=raised>R</span></div><div id=after></div>",
        )?;
        let continuation = render(
            &mut page_vm,
            r#"#wrap-root{width:40px;word-break:break-all}#wrap{padding:0 1px;border-left:1px solid;border-right:1px solid;background:rgb(91,92,93);color:rgb(101,102,103)}"#,
            "<div id=wrap-root class=fixed><span id=wrap>ABCDE</span></div>",
        )?;
        let preserved_break_baseline = render(
            &mut page_vm,
            r#"
#baseline-wrapper{width:200px;font-size:0;line-height:0;background:rgb(111,112,113)}
#baseline-text,#baseline-break{display:inline-block;width:100px;height:200px}
#baseline-text{background:rgb(121,122,123)}
#baseline-break{white-space:pre;background:rgb(131,132,133)}
"#,
            "<div id=baseline-wrapper><div id=baseline-text>text</div><div id=baseline-break>\n</div></div>",
        )?;
        let rgb = |red: u8, green: u8, blue: u8| {
            moli_layout::PaintColor::new(
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                1.0,
            )
        };
        let glyph_runs =
            |snapshot: &moli_layout::PaintSnapshot,
             color: moli_layout::PaintColor| {
                snapshot
                    .fragments
                    .iter()
                    .filter_map(|fragment| match fragment {
                        moli_layout::PaintFragment::GlyphRun(run)
                            if run.color == color =>
                        {
                            Some(run.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
        let glyphs =
            |snapshot: &moli_layout::PaintSnapshot,
             color: moli_layout::PaintColor| {
                glyph_runs(snapshot, color)
                    .into_iter()
                    .flat_map(|run| run.glyphs_in_surface())
                    .collect::<Vec<_>>()
            };
        let solid_rects =
            |snapshot: &moli_layout::PaintSnapshot,
             color: moli_layout::PaintColor| {
                snapshot
                    .fragments
                    .iter()
                    .filter_map(|fragment| {
                        fragment
                            .solid_fill_in_surface()
                            .filter(|(_, actual)| *actual == color)
                            .map(|(rect, _)| rect)
                    })
                    .collect::<Vec<_>>()
            };
        let assert_points = |label: &str,
                             actual: &[moli_layout::PaintGlyph],
                             expected: &[(f32, f32)]| {
            assert_eq!(actual.len(), expected.len(), "{label}: {actual:?}");
            for (index, (glyph, (x, y))) in actual.iter().zip(expected).enumerate() {
                assert!(
                    (glyph.x - x).abs() <= 0.05 && (glyph.y - y).abs() <= 0.05,
                    "{label}[{index}]: actual={glyph:?}, expected=({x}, {y})"
                );
            }
        };
        let assert_rects = |label: &str,
                            actual: &[moli_layout::PaintRect],
                            expected: &[(f32, f32, f32, f32)]| {
            assert_eq!(actual.len(), expected.len(), "{label}: {actual:?}");
            for (index, (rect, (x, y, width, height))) in
                actual.iter().zip(expected).enumerate()
            {
                assert!(
                    (rect.x - x).abs() <= 0.05
                        && (rect.y - y).abs() <= 0.05
                        && (rect.width - width).abs() <= 0.05
                        && (rect.height - height).abs() <= 0.05,
                    "{label}[{index}]: actual={rect:?}, expected=({x}, {y}, {width}, {height})"
                );
            }
        };
        assert_rects(
            "shared nested inline fragment",
            &solid_rects(&shared, rgb(4, 5, 6)),
            &[(26.0, 0.0, 20.0, 20.0)],
        );
        for (label, color, expected) in [
            ("pseudo", rgb(1, 2, 3), vec![(0.0, 16.0)]),
            ("first span", rgb(11, 12, 13), vec![(12.0, 16.0)]),
            ("nested span", rgb(21, 22, 23), vec![(30.0, 16.0)]),
            ("third span", rgb(31, 32, 33), vec![(48.0, 16.0)]),
            ("post-br first", rgb(41, 42, 43), vec![(0.0, 36.0)]),
            ("post-br second", rgb(51, 52, 53), vec![(12.0, 36.0)]),
        ] {
            assert_points(label, &glyphs(&shared, color), &expected);
        }
        assert_points(
            "trailing br keeps exactly one line box",
            &glyphs(&shared, rgb(61, 62, 63)),
            &[(0.0, 56.0)],
        );
        assert_rects(
            "block following trailing br",
            &solid_rects(&shared, rgb(71, 72, 73)),
            &[(0.0, 60.0, 10.0, 5.0)],
        );

        assert_points(
            "collapsed and transformed text",
            &[
                glyphs(&whitespace, rgb(21, 22, 23))[0],
                glyphs(&whitespace, rgb(11, 12, 13))[0],
                glyphs(&whitespace, rgb(11, 12, 13))[1],
                glyphs(&whitespace, rgb(31, 32, 33))[0],
            ],
            &[(0.0, 16.0), (24.0, 16.0), (36.0, 16.0), (0.0, 36.0)],
        );
        assert_points(
            "cjk segment break and emoji fallback",
            &glyphs(&whitespace, rgb(41, 42, 43)),
            &[(0.0, 58.0), (20.0, 58.0), (0.0, 78.0), (20.0, 78.0)],
        );
        assert_points(
            "preserve-breaks",
            &glyphs(&whitespace, rgb(51, 52, 53)),
            &[(0.0, 96.0), (12.0, 96.0), (24.0, 96.0), (0.0, 116.0)],
        );
        assert_points(
            "break-spaces",
            &glyphs(&whitespace, rgb(61, 62, 63)),
            &[
                (0.0, 136.0),
                (12.0, 136.0),
                (24.0, 136.0),
                (0.0, 156.0),
                (12.0, 156.0),
            ],
        );
        assert_points(
            "nowrap",
            &glyphs(&whitespace, rgb(71, 72, 73)),
            &[(0.0, 176.0), (12.0, 176.0), (24.0, 176.0)],
        );
        let cjk_runs = glyph_runs(&whitespace, rgb(41, 42, 43));
        assert_eq!(cjk_runs.len(), 3, "CJK and emoji must select separate faces");
        assert_eq!(
            whitespace.fonts[cjk_runs[0].font.index()]
                .font
                .data
                .as_ref(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-cjk.ttf"
            ))
        );
        assert_eq!(
            whitespace.fonts[cjk_runs[2].font.index()]
                .font
                .data
                .as_ref(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-hebrew-emoji.ttf"
            ))
        );

        assert_points(
            "rtl emoji",
            &glyphs(&bidi, rgb(31, 32, 33)),
            &[(50.218_75, 17.0)],
        );
        assert_points(
            "isolated latin run",
            &glyphs(&bidi, rgb(21, 22, 23)),
            &[(71.068_36, 17.0), (83.068_36, 17.0)],
        );
        assert_points(
            "visual hebrew run",
            &glyphs(&bidi, rgb(11, 12, 13)),
            &[(95.068_36, 17.0), (106.630_86, 17.0)],
        );
        let spacing_runs = glyph_runs(&bidi, rgb(41, 42, 43));
        assert_eq!(spacing_runs.len(), 1, "spacing fixture must remain one line");
        assert_points(
            "indent letter and word spacing",
            &spacing_runs[0].glyphs_in_surface(),
            &[(12.0, 37.0), (26.0, 37.0), (44.0, 37.0)],
        );
        assert!(
            spacing_runs[0]
                .glyph_skew_radians
                .is_some_and(|skew| (skew - 0.244_346_1).abs() <= 0.000_1),
            "font-style synthesis must survive snapshot projection: {spacing_runs:?}"
        );

        for (label, color, expected) in [
            ("raised inline background", rgb(1, 2, 3), (42.0, 0.0, 12.0, 20.0)),
            ("top atomic", rgb(41, 42, 43), (12.0, 0.0, 10.0, 30.0)),
            ("bottom atomic", rgb(51, 52, 53), (22.0, 20.0, 10.0, 10.0)),
            ("middle atomic", rgb(61, 62, 63), (32.0, 14.0, 10.0, 8.0)),
            ("following block", rgb(81, 82, 83), (0.0, 30.0, 10.0, 5.0)),
        ] {
            assert_rects(label, &solid_rects(&vertical, color), &[expected]);
        }
        assert_points(
            "baseline strut",
            &glyphs(&vertical, moli_layout::PaintColor::BLACK),
            &[(0.0, 26.0)],
        );
        assert_points(
            "raised baseline shift",
            &glyphs(&vertical, rgb(71, 72, 73)),
            &[(42.0, 16.0)],
        );

        assert_rects(
            "inline continuation backgrounds",
            &solid_rects(&continuation, rgb(91, 92, 93)),
            &[(0.0, 0.0, 38.0, 20.0), (0.0, 20.0, 26.0, 20.0)],
        );
        let continuation_borders = continuation
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::Border { rect, widths, .. } => {
                    Some((*rect, *widths))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(continuation_borders.len(), 2);
        assert_eq!(continuation_borders[0].1.left, 1.0);
        assert_eq!(continuation_borders[0].1.right, 0.0);
        assert_eq!(continuation_borders[1].1.left, 0.0);
        assert_eq!(continuation_borders[1].1.right, 1.0);
        assert_points(
            "inline continuation glyphs",
            &glyphs(&continuation, rgb(101, 102, 103)),
            &[
                (2.0, 16.0),
                (14.0, 16.0),
                (26.0, 16.0),
                (0.0, 36.0),
                (12.0, 36.0),
            ],
        );
        assert_rects(
            "zero-sized text inline-block baseline",
            &solid_rects(&preserved_break_baseline, rgb(121, 122, 123)),
            &[(0.0, 0.0, 100.0, 200.0)],
        );
        assert_rects(
            "preserved-break inline-block baseline",
            &solid_rects(&preserved_break_baseline, rgb(131, 132, 133)),
            &[(100.0, 0.0, 100.0, 200.0)],
        );
        assert_eq!(
            page_vm.vm_mut().document_web_font_counts_for_test(),
            (3, 3, 3),
            "repeated one-shot mutation/layout demands must retain exactly the current faces"
        );

        assert!(
            shared.fonts.iter().any(|resource| {
                resource.font.data.as_ref()
                    == include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../moli-layout/tests/fixtures/moli-ahem.ttf"
                    ))
                    .as_slice()
            }),
            "snapshot must own the selected fixed Latin face"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("fixed-font inline corpus should run");
}
