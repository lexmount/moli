use super::*;

#[tokio::test(flavor = "current_thread")]
async fn empty_text_control_caret_keeps_control_opacity_transform_and_clip() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/control-caret-effects.html")?,
        );
        page.vm_mut().eval(
            r#"
document.head.innerHTML=`<style>
html,body{margin:0;background:white}
input{display:block;box-sizing:border-box;width:220px;height:100px;border:0;
  padding:8px;outline:none;background:white;caret-color:red;font:20px sans-serif}
::placeholder{font-size:48px;opacity:0}
</style>`;
document.body.innerHTML='<input id=field placeholder=Example>';
document.getElementById('field').focus();
'installed'
"#,
        )?;
        page.vm_mut().sync_live_document_style_sources();
        let mut reference: Option<moli_layout::PaintRect> = None;
        for (state, styles) in [
            ("normal", ""),
            ("translated", "transform:translate(17px,11px)"),
            ("translucent", "opacity:0.5"),
            ("transparent", "opacity:0"),
            ("clipped", "clip-path:inset(0 0 0 50%)"),
        ] {
            page.vm_mut().eval(&format!(
                "document.getElementById('field').style.cssText={};'updated'",
                serde_json::to_string(styles)?,
            ))?;
            let snapshot = page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(260, 140, 1.0))?
                .expect("control caret effects layout");
            if matches!(state, "normal" | "translated") {
                let caret = snapshot
                    .fragments
                    .iter()
                    .find_map(|fragment| match fragment {
                        moli_layout::PaintFragment::Fill {
                            shape: moli_layout::PaintShape::Rect(rect),
                            brush: moli_layout::PaintBrush::Solid(color),
                            transform,
                            ..
                        } if *color == moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0)
                            && rect.width == 1.5 =>
                        {
                            Some(transform.map_rect(*rect).bounding_rect())
                        }
                        _ => None,
                    })
                    .expect("editor caret paint");
                if let Some(reference) = reference {
                    assert_eq!(
                        caret,
                        moli_layout::PaintRect::new(
                            reference.x + 17.0,
                            reference.y + 11.0,
                            reference.width,
                            reference.height
                        )
                    );
                } else {
                    reference = Some(caret);
                }
            }
            let image = moli_paint::raster_snapshot(&snapshot)?;
            let pixels = image.rgba.chunks_exact(4);
            match state {
                "transparent" | "clipped" => assert!(
                    !pixels
                        .clone()
                        .any(|p| i16::from(p[0]) - i16::from(p[1]) > 25),
                    "{state}: the control's effects also apply to the caret",
                ),
                "translucent" => assert!(
                    pixels
                        .clone()
                        .any(|p| p[0] > 240 && (100..=160).contains(&p[1]) && p[1] == p[2]),
                    "the caret must be composited with the control's opacity",
                ),
                _ => assert!(pixels.clone().any(|p| p[0] > 200 && p[1] < 50 && p[2] < 50)),
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("control caret effects fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn placeholder_opacity_does_not_hide_empty_text_control_carets() {
    assert_placeholder_style_keeps_empty_editor_caret(&["opacity:1", "opacity:0"]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn placeholder_font_metrics_do_not_change_empty_text_control_carets() {
    assert_placeholder_style_keeps_empty_editor_caret(&[
        "font-size:20px;line-height:normal",
        "font-size:48px;line-height:96px",
    ])
    .await;
}

async fn assert_placeholder_style_keeps_empty_editor_caret(
    placeholder_styles: &'static [&'static str],
) {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/placeholder-caret.html")?,
        );
        page.vm_mut().eval(r#"
document.head.innerHTML=`<style>
html,body{margin:0;background:white}
input,textarea{display:block;box-sizing:border-box;width:220px;height:100px;
  border:0;padding:8px;outline:none;background:white;color:black;
  caret-color:red;font:20px sans-serif}
</style><style id=placeholder-style></style>`;
'installed'
"#)?;
        for tag in ["input", "textarea"] {
            page.vm_mut().eval(&format!(
                "document.body.innerHTML='<{tag} id=field></{tag}>';document.getElementById('field').focus();'focused'"
            ))?;
            page.vm_mut().sync_live_document_style_sources();
            let mut reference = None;
            // Blink keeps the empty inner editor separate from the placeholder
            // sibling (TextControlInnerEditorElement / UpdatePlaceholderText).
            for placeholder_style in std::iter::once(None)
                .chain(placeholder_styles.iter().copied().map(Some))
            {
                let script = if let Some(style) = placeholder_style {
                    format!(
                        "document.getElementById('field').placeholder='Example';document.getElementById('placeholder-style').textContent={};'updated'",
                        serde_json::to_string(&format!("::placeholder{{color:black;{style}}}"))?,
                    )
                } else {
                    "document.getElementById('placeholder-style').textContent='';'reference'".to_owned()
                };
                page.vm_mut().eval(&script)?;
                let snapshot = page.vm_mut().screenshot_layout_snapshot(
                    moli_layout::PaintViewport::new(260, 140, 1.0),
                )?.expect("placeholder caret layout");
                let image = moli_paint::raster_snapshot(&snapshot)?;
                let red_pixels = image.rgba.chunks_exact(4).enumerate().filter_map(|(index, pixel)| {
                    (pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50 && pixel[3] > 200)
                        .then_some((index as u32 % image.width, index as u32 / image.width))
                }).collect::<Vec<_>>();
                assert!(!red_pixels.is_empty(), "{tag}/{placeholder_style:?}: the caret must remain visible after compositing");
                if let Some(reference) = &reference {
                    assert_eq!(&red_pixels, reference, "{tag}/{placeholder_style:?}: placeholder styling must not change caret position or size");
                } else {
                    reference = Some(red_pixels);
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("placeholder caret independence fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn focused_text_controls_paint_utf16_selection_and_empty_carets() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/text-control-selection.html")?,
        );
        page.vm_mut().eval(r#"
document.head.innerHTML='<style>html,body{margin:0}input,textarea{display:block;width:220px;height:40px;border:0;padding:8px;color:red;caret-color:lime;font:20px/24px sans-serif}::placeholder{color:gray}</style>';
document.body.innerHTML='<input id=plain><input id=placeholder placeholder=Example><textarea id=area></textarea>';
'installed'
"#)?;
        page.vm_mut().sync_live_document_style_sources();
        for id in ["plain", "placeholder", "area"] {
            let mut caret_start = 0.0;
            let mut selected_right = 0.0;
            for (state, script, caret, selected) in [
                ("empty", "el.focus();el.value='';el.setSelectionRange(0,0)", true, false),
                ("before emoji", "el.value='A😀B';el.setSelectionRange(1,1)", true, false),
                ("selected emoji", "el.setSelectionRange(1,3)", false, true),
                ("after emoji", "el.setSelectionRange(3,3)", true, false),
                ("blurred", "el.blur()", false, false),
            ] {
                page.vm_mut().eval(&format!("{{const el=document.getElementById('{id}');{script};}}'updated'"))?;
                let snapshot = page.vm_mut().screenshot_layout_snapshot(
                    moli_layout::PaintViewport::new(260, 180, 1.0),
                )?.expect("text control layout");
                let fills = snapshot.fragments.iter().filter_map(|fragment| match fragment {
                    moli_layout::PaintFragment::Fill { shape:moli_layout::PaintShape::Rect(rect),brush:moli_layout::PaintBrush::Solid(color),transform, .. } => Some((transform.map_rect(*rect).bounding_rect(),*color)),
                    _ => None,
                }).collect::<Vec<_>>();
                let carets = fills.iter().filter(|(rect,color)| rect.width == 1.5 && *color == moli_layout::PaintColor::new(0.0,1.0,0.0,1.0)).collect::<Vec<_>>();
                let selections = fills.iter().filter(|(_,color)| *color == moli_layout::PaintColor::new(180.0/255.0,213.0/255.0,1.0,1.0)).collect::<Vec<_>>();
                assert_eq!(carets.len(),usize::from(caret),"{id}/{state}: active caret");
                assert_eq!(selections.len(),usize::from(selected),"{id}/{state}: active selection");
                if let Some((rect,_))=carets.first() {
                    assert!(rect.height>0.0,"{id}/{state}: visible caret");
                    if state=="empty" {assert_eq!(rect.x,8.0,"caret starts at the CSS padding edge");}
                    if state=="before emoji" {caret_start=rect.x;}
                    if state=="after emoji" {assert!((rect.x-selected_right).abs()<0.1,"UTF-16 selection ends at the next caret");}
                }
                if let Some((rect,_))=selections.first() {
                    assert!((rect.x-caret_start).abs()<0.1,"selection starts at the previous caret");
                    assert!(rect.width>0.0);
                    selected_right=rect.right();
                }
            }
        }
        Ok::<_,anyhow::Error>(())
    }).await.expect("text control selection fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn contenteditable_atomic_inlines_paint_carets_without_glyphs() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-caret.html")?,
        );
        page.vm_mut().eval(include_str!("atomic_inline_caret.js"))?;
        page.vm_mut().sync_live_document_style_sources();
        for (label, text, offset, after_atomic) in [
            ("atomic and forced break", "\n", 0, true),
            ("glyphs on another line", "\nX", 0, true),
            ("empty middle line", "X\n\nY", 2, false),
            ("drawable glyph carrier", "X", 0, true),
        ] {
            page.vm_mut().eval(&format!(
                "setAtomicCaret({}, {offset});'updated'",
                serde_json::to_string(text)?,
            ))?;
            let snapshot = page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 140, 1.0))?
                .expect("atomic caret layout");
            let carets = snapshot
                .fragments
                .iter()
                .filter_map(|fragment| match fragment {
                    moli_layout::PaintFragment::Fill {
                        shape: moli_layout::PaintShape::Rect(rect),
                        brush: moli_layout::PaintBrush::Solid(color),
                        transform,
                        ..
                    } if *color == moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0) => {
                        Some(transform.map_rect(*rect).bounding_rect())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                carets.len(),
                1,
                "{label}: one caret without a glyph carrier"
            );
            let caret = carets[0];
            assert_eq!(caret.width, 1.5, "{label}: caret width");
            assert!(caret.height > 0.0, "{label}: caret height");
            assert!(
                (caret.x - if after_atomic { 24.0 } else { 0.0 }).abs() < 0.1,
                "{label}: caret position {caret:?}"
            );
            if !after_atomic {
                assert!(
                    caret.y >= 32.0,
                    "caret belongs to the empty middle line: {caret:?}"
                );
            }
            if text == "\n" {
                assert!(
                    !snapshot.fragments.iter().any(|fragment| matches!(
                        fragment,
                        moli_layout::PaintFragment::GlyphRun(_)
                    )),
                    "the regression fixture must have no drawable glyphs"
                );
            }
            let image = moli_paint::raster_snapshot(&snapshot)?;
            assert!(
                image
                    .rgba
                    .chunks_exact(4)
                    .any(|p| p[0] > 200 && p[1] < 50 && p[2] < 50),
                "{label}: visible red caret pixels"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("glyphless atomic caret fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn contenteditable_atomic_inline_caret_keeps_effects_and_viewport_culling() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-caret-effects.html")?,
        );
        page.vm_mut().eval(include_str!("atomic_inline_caret.js"))?;
        page.vm_mut().sync_live_document_style_sources();
        for (label, styles, visible) in [
            ("translated", "transform:translate(17px,11px)", true),
            ("transparent", "opacity:0", false),
            ("clip path", "clip-path:inset(0 0 0 50%)", false),
            // The box border is above the capture, but its overflowing caret
            // still intersects the viewport.
            (
                "caret-only capture",
                "height:1px;transform:translateY(-12px)",
                true,
            ),
        ] {
            page.vm_mut().eval(&format!(
                "setAtomicCaret('\\n', 0, {});'updated'",
                serde_json::to_string(styles)?,
            ))?;
            let snapshot = page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 140, 1.0))?
                .expect("atomic caret effects layout");
            let image = moli_paint::raster_snapshot(&snapshot)?;
            let red = image.rgba.chunks_exact(4).any(|p| {
                i16::from(p[0]) - i16::from(p[1]) > 20 && i16::from(p[0]) - i16::from(p[2]) > 20
            });
            assert_eq!(red, visible, "{label}: caret compositing");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic caret effects fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn contenteditable_atomic_inlines_paint_glyphless_selection_ranges() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-selection.html")?,
        );
        page.vm_mut().eval(include_str!("atomic_inline_caret.js"))?;
        page.vm_mut().sync_live_document_style_sources();
        for (label, text, start, end, expected_lines) in [
            ("selected newline", "\nX", 0, 1, vec![0]),
            ("selected empty line", "X\n\nY", 2, 3, vec![1]),
            ("text and newlines", "X\n\nY", 0, 4, vec![0, 1, 2]),
            ("glyph without newline", "X\nY", 0, 1, vec![0]),
            ("reversed empty line", "X\n\nY", 3, 2, vec![1]),
            ("UTF-16 newline offset", "😀\nX", 2, 3, vec![0]),
        ] {
            page.vm_mut().eval(&format!(
                "setAtomicRange({}, {start}, {end});'updated'",
                serde_json::to_string(text)?,
            ))?;
            let snapshot = page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 140, 1.0))?
                .expect("atomic range layout");
            let rects = range_selection_rects(&snapshot);
            assert!(
                !rects.is_empty(),
                "{label}: selected text needs highlight commands"
            );
            let mut lines = rects
                .iter()
                .map(|rect| (rect.y / 32.0).round() as usize)
                .collect::<Vec<_>>();
            lines.sort_unstable();
            lines.dedup();
            assert_eq!(
                lines, expected_lines,
                "{label}: every selected line is painted"
            );
            for (index, rect) in rects.iter().enumerate() {
                assert!(
                    rect.width > 0.0 && rect.height > 0.0,
                    "{label}: positive highlight geometry"
                );
                for other in &rects[index + 1..] {
                    let width = rect.right().min(other.right()) - rect.x.max(other.x);
                    let height = rect.bottom().min(other.bottom()) - rect.y.max(other.y);
                    assert!(
                        width <= 0.01 || height <= 0.01,
                        "{label}: selection is not painted twice: {rect:?}, {other:?}"
                    );
                }
            }
            let image = moli_paint::raster_snapshot(&snapshot)?;
            assert!(
                has_selection_pixels(&image),
                "{label}: visible highlight pixels"
            );
            if label == "text and newlines" {
                assert!(
                    dark_pixels(&image, 24, 0, 56, 32) > 0,
                    "selected first-line glyphs stay above the highlight"
                );
                assert!(
                    dark_pixels(&image, 0, 64, 80, 32) > 0,
                    "selected last-line glyphs stay above the highlight"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("glyphless atomic selection fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn contenteditable_atomic_inline_selection_keeps_order_effects_and_culling() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-selection-effects.html")?,
        );
        page.vm_mut().eval(include_str!("atomic_inline_caret.js"))?;
        page.vm_mut().sync_live_document_style_sources();
        for (label, styles, overlap, visible) in [
            ("translated", "transform:translate(17px,11px)", "", true),
            ("transparent", "opacity:0", "", false),
            ("clip path", "clip-path:inset(0 0 0 50%)", "", false),
            ("highlight-only capture", "", "", true),
            ("above earlier atomic", "", "earlier", true),
            ("below later atomic", "", "later", false),
        ] {
            page.vm_mut().eval(&format!(
                "setAtomicRange('\\nX', 0, 1, {}, {});'updated'",
                serde_json::to_string(styles)?,
                serde_json::to_string(overlap)?,
            ))?;
            let capture = if label == "highlight-only capture" {
                moli_layout::PaintCaptureRequest::page_clip(
                    moli_layout::PaintRect::new(26.0, 0.0, 2.0, 32.0),
                    1.0,
                )
            } else {
                moli_layout::PaintCaptureRequest::viewport()
            };
            let snapshot = page
                .vm_mut()
                .paint_layout_snapshot_with_capture(
                    moli_layout::PaintViewport::new(160, 140, 1.0),
                    moli_layout::LayoutFlushReason::Screenshot,
                    capture,
                )?
                .expect("atomic range effects layout");
            let image = moli_paint::raster_snapshot(&snapshot)?;
            assert_eq!(
                has_selection_pixels(&image),
                visible,
                "{label}: selection compositing"
            );
            if overlap == "later" {
                assert!(
                    !range_selection_rects(&snapshot).is_empty(),
                    "the highlight is painted before being covered"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic selection order and effects fixture");
}

fn range_selection_rects(snapshot: &moli_layout::PaintSnapshot) -> Vec<moli_layout::PaintRect> {
    let selection_color = moli_layout::PaintColor::new(180.0 / 255.0, 213.0 / 255.0, 1.0, 1.0);
    snapshot
        .fragments
        .iter()
        .filter_map(|fragment| match fragment {
            moli_layout::PaintFragment::Fill {
                shape: moli_layout::PaintShape::Rect(rect),
                brush: moli_layout::PaintBrush::Solid(color),
                transform,
                ..
            } if *color == selection_color => Some(transform.map_rect(*rect).bounding_rect()),
            _ => None,
        })
        .collect()
}

fn has_selection_pixels(image: &moli_paint::RasterImage) -> bool {
    image
        .rgba
        .chunks_exact(4)
        .any(|p| p[0].abs_diff(180) <= 1 && p[1].abs_diff(213) <= 1 && p[2] >= 254)
}

fn dark_pixels(image: &moli_paint::RasterImage, x: u32, y: u32, width: u32, height: u32) -> usize {
    (y..y + height)
        .flat_map(|y| (x..x + width).map(move |x| ((y * image.width + x) * 4) as usize))
        .filter(|&index| {
            image.rgba[index..index + 3]
                .iter()
                .all(|&channel| channel < 128)
        })
        .count()
}

#[tokio::test(flavor = "current_thread")]
async fn placeholder_pseudo_style_tracks_empty_and_filled_form_controls() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/placeholder-paint.html")?,
        );
        // Like Blink's TextFieldInputType::UpdatePlaceholderText, placeholder
        // content has its own pseudo style, independently of the input value.
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
input,textarea{display:block;width:200px;height:40px;padding:0;border:0;
  color:red;font:20px/28px sans-serif}
::placeholder{color:rgb(31,63,127);font-size:24px;opacity:0.5}
</style>`;
document.body.innerHTML=`<input placeholder=Example><input type=email placeholder=Example>
<textarea placeholder=Example></textarea>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        for (value, expected_color, expected_size) in [
            (
                "",
                moli_layout::PaintColor::new(31.0 / 255.0, 63.0 / 255.0, 127.0 / 255.0, 1.0),
                24.0,
            ),
            (
                "Example",
                moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                20.0,
            ),
            (
                "",
                moli_layout::PaintColor::new(31.0 / 255.0, 63.0 / 255.0, 127.0 / 255.0, 1.0),
                24.0,
            ),
        ] {
            page_vm.vm_mut().eval(&format!(
                "document.querySelectorAll('input,textarea').forEach(el=>el.value={});'updated'",
                serde_json::to_string(value)?,
            ))?;
            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 180, 1.0))?
                .expect("placeholder layout");
            let runs = snapshot
                .fragments
                .iter()
                .filter_map(|fragment| match fragment {
                    moli_layout::PaintFragment::GlyphRun(run) => Some(run),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(runs.len(), 3, "each control paints one label");
            for run in runs {
                assert_eq!(run.color, expected_color, "value={value:?}");
                assert_eq!(run.font_size, expected_size);
            }
            let placeholder_layers = snapshot.fragments.iter().filter(|fragment| matches!(
                fragment, moli_layout::PaintFragment::PushLayer { opacity, .. } if *opacity == 0.5
            )).count();
            assert_eq!(
                placeholder_layers,
                if value.is_empty() { 3 } else { 0 },
                "placeholder opacity must not affect typed text"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("placeholder paint fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_select_listboxes_paint_every_option_and_clip_their_contents() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/select-listbox.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
select{position:absolute;top:0;width:140px;height:90px;box-sizing:border-box;
       margin:0;padding:0;border:1px solid black;background:white;color:black;font:16px/20px sans-serif}
#multi{left:0}#selected{left:160px}#sized{left:320px}#single{left:480px;height:22px}
#clipped{left:640px;height:36px}
</style>`;
const options = `<option value=a>Alpha</option><option value=b label=Beta>not the label</option><option value=c>\n Gamma \t </option>`;
document.body.innerHTML = `<select id=multi multiple>${options}</select>
<select id=selected multiple>${options}</select><select id=sized size=3>${options}</select>
<select id=single>${options}</select>
<select id=clipped multiple><option>AlphaBetaGammaAlphaBetaGamma</option><option>Beta</option><option>Gamma</option></select>`;
document.getElementById('selected').options[1].selected = true;
document.getElementById('single').value = 'b';
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        assert_eq!(
            page_vm.vm_mut().eval(
                "[...document.getElementById('multi').options].map(o=>o.label).join('|') + ';' + document.getElementById('multi').selectedOptions.length",
            )?,
            "Alpha|Beta|Gamma;0",
            "the unselected listbox must still have all three option labels",
        );
        let viewport = moli_layout::PaintViewport::new(800, 120, 1.0);
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("select fixture must retain a layout root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        for (name, x) in [("multiple", 0), ("selected multiple", 160), ("size=3", 320)] {
            for (row, label) in ["Alpha", "Beta", "Gamma"].into_iter().enumerate() {
                assert!(
                    dark_pixels(&image, x + 2, 1 + row as u32 * 25, 130, 24) > 10,
                    "{name} must paint {label} on row {row}, independently of selectedness",
                );
            }
            assert_eq!(
                dark_pixels(&image, x + 52, 26, 80, 24),
                0,
                "{name} must use the option's label attribute rather than its longer text content",
            );
        }
        assert!(dark_pixels(&image, 482, 2, 130, 18) > 10);
        assert_eq!(dark_pixels(&image, 482, 24, 130, 60), 0, "a dropdown paints only its selected label");
        assert!(dark_pixels(&image, 642, 2, 130, 24) > 10);
        assert_eq!(dark_pixels(&image, 642, 37, 136, 70), 0, "listbox rows must be clipped vertically");
        assert_eq!(dark_pixels(&image, 781, 2, 19, 32), 0, "long option labels must be clipped horizontally");

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
const select = document.getElementById('multi');
select.options[0].label = 'Delta';
select.options[1].remove();
select.options[0].selected = true;
select.options[1].selected = true;
return select.value + ';' + [...select.selectedOptions].map(o=>o.label).join('|');
})()"#,
            )?,
            "a;Delta|Gamma",
        );
        let updated = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("mutated select fixture must retain a layout root");
        let updated = moli_paint::raster_snapshot(&updated)?;
        assert!(dark_pixels(&updated, 2, 1, 130, 24) > 10);
        assert!(dark_pixels(&updated, 2, 26, 130, 24) > 10);
        assert_eq!(dark_pixels(&updated, 2, 51, 130, 24), 0, "removing an option must remove its painted row");
        let first_row_end = (25 * image.width * 4) as usize;
        assert_ne!(
            image.rgba[..first_row_end], updated.rgba[..first_row_end],
            "changing an option label must change the painted text",
        );

        page_vm.vm_mut().eval(
            "document.getElementById('multi').add(new Option('', 'blank'), 1); 'inserted'",
        )?;
        let with_blank = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("select with an empty option must retain a layout root");
        let with_blank = moli_paint::raster_snapshot(&with_blank)?;
        assert!(dark_pixels(&with_blank, 2, 1, 130, 24) > 10);
        assert_eq!(dark_pixels(&with_blank, 2, 26, 130, 24), 0, "an empty option must keep its row");
        assert!(dark_pixels(&with_blank, 2, 51, 130, 24) > 10);

        assert_eq!(page_vm.vm_mut().eval(
            r#"(() => {
const select = document.getElementById('multi');
select.options[0].label = '';
select.size = 1;
select.value = 'c';
select.style.height = '22px';
return select.value + ';' + select.selectedOptions[0].label;
})()"#,
        )?, "c;Gamma");
        let dropdown = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("single-row multiple select must retain a layout root");
        let dropdown = moli_paint::raster_snapshot(&dropdown)?;
        assert!(dark_pixels(&dropdown, 2, 2, 130, 18) > 10, "size=1 must paint the selected label even when multiple is set");
        assert_eq!(dark_pixels(&dropdown, 2, 24, 130, 60), 0);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("select listbox rendering fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn focused_form_text_does_not_duplicate_control_outlines_and_shadows() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/form-control-focus-paint.html")?,
        );
        // Blink's TextControlInnerEditorElement creates an inherited style;
        // non-inherited control decoration belongs only to the outer box.
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
input{display:block;box-sizing:border-box;width:300px;height:54px;border:0;
      padding:10px 16px;border-radius:20px;background:#eee;color:red;font:20px/24px sans-serif;outline:none}
input:focus{outline:2px solid blue;box-shadow:0 0 0 4px lime}
</style>`;
document.body.innerHTML = `<input id=field placeholder="Example">`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        for (state, script, focused) in [
            ("empty", "'ready'", false),
            ("focused", "document.getElementById('field').focus();'focused'", true),
            ("typed", "document.getElementById('field').value='moli probe';'typed'", true),
            ("cleared", "document.getElementById('field').value='';'cleared'", true),
            ("blurred", "document.getElementById('field').blur();'blurred'", false),
        ] {
            page_vm.vm_mut().eval(script)?;
            let snapshot = page_vm.vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 90, 1.0))?
                .expect("form control layout");
            let outlines = snapshot.fragments.iter().filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::Border { rect, colors, .. }
                    if colors.top == moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0) => Some(rect),
                _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(outlines.len(), usize::from(focused), "{state}: one outline on the control");
            if let Some(rect) = outlines.first() {
                assert_eq!(rect.width, 304.0, "{state}: outline follows the outer border box");
                assert_eq!(rect.height, 58.0);
            }
            let shadows = snapshot.fragments.iter().filter(|fragment| matches!(
                fragment, moli_layout::PaintFragment::BoxShadow(_)
            )).count();
            assert_eq!(shadows, usize::from(focused), "{state}: one control shadow");
            assert!(snapshot.fragments.iter().any(|fragment| matches!(
                fragment, moli_layout::PaintFragment::GlyphRun(run) if run.font_size == 20.0
            )), "{state}: text keeps the inherited font");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("form focus paint fixture should run");
}
