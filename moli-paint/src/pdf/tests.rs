use std::sync::Arc;

use moli_layout::{
    PaintColor, PaintFragment, PaintGlyph, PaintGlyphRun, PaintPoint, PaintRect, PaintShape,
    PaintSnapshot, PaintTextSemantics, PaintTransform2D, PaintViewport,
};
use skrifa::MetadataProvider;

use super::*;

fn snapshot() -> PaintSnapshot {
    PaintSnapshot::new(PaintViewport::new(300, 600, 1.0), PaintColor::WHITE)
}
fn options() -> PdfOptions {
    PdfOptions {
        width: 300.0,
        height: 300.0,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        ..Default::default()
    }
}
fn text(snapshot: &mut PaintSnapshot, text: &str, baseline: f32, cjk: bool) -> PaintGlyphRun {
    let bytes: &[u8] = if cjk {
        include_bytes!("../../../moli-layout/tests/fixtures/moli-cjk.ttf")
    } else {
        include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.ttf")
    };
    let font = peniko::FontData::new(peniko::Blob::new(Arc::new(bytes.to_vec())), 0);
    let face = skrifa::FontRef::from_index(bytes, 0).unwrap();
    let mut glyphs = Vec::new();
    let mut clusters = Vec::new();
    for (i, (offset, ch)) in text.char_indices().enumerate() {
        glyphs.push(PaintGlyph {
            id: face.charmap().map(ch).unwrap().to_u32(),
            x: 10.0 + i as f32 * 12.0,
            y: baseline,
        });
        clusters.push(offset..offset + ch.len_utf8());
    }
    PaintGlyphRun {
        font: snapshot.intern_font(&font),
        font_size: 20.0,
        normalized_coords: Vec::new(),
        color: PaintColor::BLACK,
        glyph_skew_radians: None,
        glyph_embolden: PaintPoint::ZERO,
        glyphs,
        text: Some(PaintTextSemantics {
            text: Arc::from(text),
            clusters,
        }),
        transform: PaintTransform2D::IDENTITY,
        stroke: None,
    }
}
fn document(snapshot: &PaintSnapshot, options: &PdfOptions) -> lopdf::Document {
    let pdf = pdf_snapshot(snapshot, options).unwrap();
    lopdf::Document::load_mem(&pdf).unwrap()
}
fn extracted(document: &lopdf::Document, pages: &[u32]) -> String {
    document
        .extract_text(pages)
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
#[test]
fn embeds_fonts_and_unicode_instead_of_a_page_image() {
    let mut snapshot = snapshot();
    let run = text(&mut snapshot, "Font Text 123", 35.0, false);
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    let chinese = text(&mut snapshot, "中文", 65.0, true);
    snapshot.push_fragment(PaintFragment::GlyphRun(chinese));
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1, 2]), "Font Text 123 中文");
    assert!(
        doc.objects
            .values()
            .any(|o| o.as_dict().is_ok_and(|d| d.has(b"FontFile2")))
    );
    assert!(
        !doc.objects.values().any(|o| o.as_stream().is_ok_and(|s| s
            .dict
            .get(b"Subtype")
            .and_then(|o| o.as_name())
            .ok()
            == Some(b"Image")))
    );
}
#[test]
fn pagination_and_ranges_do_not_duplicate_or_leak_text() {
    let mut snapshot = snapshot();
    for (label, y) in [("First", 40.0), ("Boundary", 302.0), ("Second", 350.0)] {
        let run = text(&mut snapshot, label, y, false);
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
    }
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1]), "First Boundary");
    assert_eq!(extracted(&doc, &[2]), "Second");
    let selected = document(
        &snapshot,
        &PdfOptions {
            page_ranges: vec![1..=1],
            ..options()
        },
    );
    assert_eq!(selected.get_pages().len(), 1);
    assert_eq!(extracted(&selected, &[1]), "Second");
    assert!(matches!(
        pdf_snapshot(
            &snapshot,
            &PdfOptions {
                page_ranges: vec![100..=usize::MAX],
                ..options()
            }
        ),
        Err(PdfError::PageRangeExceeded)
    ));
}
#[test]
fn shadows_masks_and_filtered_text_have_one_semantic_copy() {
    use moli_layout::{PaintBlendMode, PaintCompositeMode, PaintFilter, PaintTextShadow};
    let mut snapshot = snapshot();
    let run = text(&mut snapshot, "Shadow", 35.0, false);
    snapshot.push_fragment(PaintFragment::TextShadow(PaintTextShadow {
        run: run.clone(),
        color: PaintColor::BLACK,
        offset: PaintPoint::new(2.0, 2.0),
        blur_radius: 2.0,
    }));
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    let run = text(&mut snapshot, "Filtered", 90.0, false);
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(0.0, 60.0, 200.0, 50.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Blur(1.0)),
    });
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let mut run = text(&mut snapshot, "Mask", 140.0, false);
    run.text = None;
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1, 2]), "Shadow Filtered");
}
#[test]
fn clips_exclude_off_page_text_and_dpr_does_not_change_pagination() {
    let mut snapshot = snapshot();
    snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
        0.0, 0.0, 300.0, 60.0,
    )));
    for (label, y) in [("Visible", 35.0), ("Hidden", 100.0)] {
        let run = text(&mut snapshot, label, y, false);
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
    }
    snapshot.push_fragment(PaintFragment::PopLayer);
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1, 2]), "Visible");
    snapshot.surface.device_scale = 3.0;
    let high_dpi = document(&snapshot, &options());
    assert_eq!(high_dpi.get_pages().len(), doc.get_pages().len());
    assert_eq!(extracted(&high_dpi, &[1, 2]), "Visible");
}
#[test]
fn rejects_malformed_semantics_and_layer_streams() {
    let mut snapshot = snapshot();
    let mut run = text(&mut snapshot, "Bad", 35.0, false);
    run.text.as_mut().unwrap().clusters[0] = 0..999;
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    assert!(matches!(
        pdf_snapshot(&snapshot, &options()),
        Err(PdfError::Encoding(_))
    ));
    snapshot.fragments = vec![PaintFragment::PopLayer];
    assert!(matches!(
        pdf_snapshot(&snapshot, &options()),
        Err(PdfError::Paint(_))
    ));
}

#[test]
fn variable_fonts_invert_avar_and_keep_distinct_instances() {
    let bytes = include_bytes!("../../tests/fixtures/moli-pdf.ttf");
    let coords = super::text::variation_coordinates(bytes, 0, &[12288]).unwrap();
    assert_eq!(coords[0].0.get(), b"wght");
    assert_eq!(coords[0].1, 650.0);
    let mut snapshot = snapshot();
    let font = peniko::FontData::new(peniko::Blob::new(Arc::new(bytes.to_vec())), 0);
    let id = snapshot.intern_font(&font);
    for (coords, y) in [(vec![0], 30.0), (vec![12288], 80.0)] {
        let mut run = text(&mut snapshot, "Weight", y, false);
        run.font = id;
        run.normalized_coords = coords;
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
    }
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1]), "Weight Weight");
    let fonts = doc.get_page_fonts(doc.get_pages()[&1]).unwrap();
    assert_eq!(
        fonts.len(),
        2,
        "different variable instances need distinct PDF font resources"
    );
}

#[test]
fn transparency_group_bounds_include_the_last_glyph_advance() {
    let mut snapshot = snapshot();
    let run = text(&mut snapshot, "Opacity", 50.0, false);
    let right = run.glyphs.last().unwrap().x + 12.0;
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 0.5,
        blend_mode: moli_layout::PaintBlendMode::Normal,
        composite: moli_layout::PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 300.0, 300.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: None,
    });
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let document = document(&snapshot, &options());
    let bounds: Vec<_> = document
        .objects
        .values()
        .filter_map(|object| {
            let stream = object.as_stream().ok()?;
            if stream.dict.get(b"Subtype").ok()?.as_name().ok()? != b"Form" {
                return None;
            }
            Some(
                stream.dict.get(b"BBox").ok()?.as_array().ok()?[2]
                    .as_float()
                    .unwrap(),
            )
        })
        .collect();
    assert!(!bounds.is_empty());
    assert!(
        bounds.iter().any(|bound| *bound >= right - 0.01),
        "opacity XObjects must not clip their final glyph: {bounds:?}"
    );
}

#[test]
fn hidden_masks_and_zero_opacity_do_not_export_source_text() {
    use moli_layout::{PaintBlendMode, PaintCompositeMode};
    let layer = |composite, opacity| PaintFragment::PushLayer {
        opacity,
        blend_mode: PaintBlendMode::Normal,
        composite,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 300.0, 300.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: None,
    };
    let mut snapshot = snapshot();
    let hidden = text(&mut snapshot, "Hidden", 40.0, false);
    snapshot.fragments = vec![
        layer(PaintCompositeMode::SrcOver, 1.0),
        PaintFragment::GlyphRun(hidden.clone()),
        layer(PaintCompositeMode::DestIn, 1.0),
        PaintFragment::PopLayer, // An empty mask removes all destination ink.
        PaintFragment::PopLayer,
        layer(PaintCompositeMode::SrcOver, 0.0),
        PaintFragment::GlyphRun(hidden),
        PaintFragment::PopLayer,
    ];
    assert_eq!(extracted(&document(&snapshot, &options()), &[1, 2]), "");
    snapshot.fragments.insert(
        3,
        PaintFragment::Fill {
            fill_rule: moli_layout::PaintFillRule::NonZero,
            shape: PaintShape::Rect(PaintRect::new(0.0, 0.0, 30.0, 100.0)),
            brush: moli_layout::PaintBrush::Solid(PaintColor::BLACK),
            transform: PaintTransform2D::IDENTITY,
        },
    );
    assert!(
        matches!(pdf_snapshot(&snapshot, &options()), Err(PdfError::Encoding(message))
        if message.contains("destination masks"))
    );
}

#[test]
fn path_clips_test_ink_instead_of_only_the_clip_bounding_box() {
    use moli_layout::{PaintPath, PaintPathElement as El};
    let mut snapshot = snapshot();
    snapshot.push_fragment(PaintFragment::PushClip {
        shape: PaintShape::Path(PaintPath {
            elements: vec![
                El::MoveTo(PaintPoint::ZERO),
                El::LineTo(PaintPoint::new(100.0, 0.0)),
                El::LineTo(PaintPoint::new(0.0, 100.0)),
                El::Close,
            ],
            bounds: PaintRect::new(0.0, 0.0, 100.0, 100.0),
        }),
        transform: PaintTransform2D::IDENTITY,
    });
    let visible = text(&mut snapshot, "Hi", 30.0, false);
    let mut hidden = text(&mut snapshot, "No", 90.0, false);
    for glyph in &mut hidden.glyphs {
        glyph.x += 70.0;
    }
    snapshot.push_fragment(PaintFragment::GlyphRun(visible));
    snapshot.push_fragment(PaintFragment::GlyphRun(hidden));
    snapshot.push_fragment(PaintFragment::PopLayer);
    assert_eq!(extracted(&document(&snapshot, &options()), &[1, 2]), "Hi");
}

#[test]
fn embedded_snapshot_rebases_fonts_and_preserves_owned_text() {
    let mut parent = snapshot();
    let parent_run = text(&mut parent, "中文", 30.0, true);
    parent.push_fragment(PaintFragment::GlyphRun(parent_run));
    let mut child = snapshot();
    let child_run = text(&mut child, "Child", 30.0, false);
    child.push_fragment(PaintFragment::GlyphRun(child_run));
    parent.append_embedded_snapshot(
        child,
        PaintRect::new(0.0, 0.0, 100.0, 60.0),
        PaintTransform2D::translation(40.0, 80.0),
    );
    assert_eq!(
        extracted(&document(&parent, &options()), &[1, 2]),
        "中文 Child"
    );
}

#[test]
fn boundary_cluster_ownership_follows_the_remaining_clipped_ink() {
    let mut snapshot = snapshot();
    let run = text(&mut snapshot, "Boundary", 302.0, false);
    snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
        0.0, 300.0, 300.0, 20.0,
    )));
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1]), "");
    assert_eq!(extracted(&doc, &[2]), "Boundary");
}

#[test]
fn native_path_fill_rules_reach_pdf_vector_operators() {
    use moli_layout::{PaintBrush, PaintFillRule, PaintPath, PaintPathElement as E};
    for (rule, operator) in [
        (PaintFillRule::NonZero, "f"),
        (PaintFillRule::EvenOdd, "f*"),
    ] {
        let mut snapshot = snapshot();
        snapshot.push_fragment(PaintFragment::Fill {
            shape: PaintShape::Path(PaintPath {
                bounds: PaintRect::new(10., 10., 20., 20.),
                elements: vec![
                    E::MoveTo(PaintPoint::new(10., 10.)),
                    E::LineTo(PaintPoint::new(30., 10.)),
                    E::LineTo(PaintPoint::new(30., 30.)),
                    E::LineTo(PaintPoint::new(10., 30.)),
                    E::Close,
                ],
            }),
            brush: PaintBrush::Solid(PaintColor::BLACK),
            transform: PaintTransform2D::IDENTITY,
            fill_rule: rule,
        });
        let doc = document(&snapshot, &options());
        let operations: Vec<_> = doc
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .map(|stream| {
                stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone())
            })
            .filter_map(|bytes| lopdf::content::Content::decode(&bytes).ok())
            .flat_map(|content| content.operations)
            .collect();
        assert!(
            operations
                .iter()
                .any(|operation| operation.operator == operator),
            "missing {operator} for {rule:?}"
        );
        if rule == PaintFillRule::NonZero {
            assert!(
                !operations
                    .iter()
                    .any(|operation| operation.operator == "f*")
            );
        }
    }
}

#[test]
fn repeated_images_fill_the_clipped_pdf_region() {
    let mut snapshot = snapshot();
    let image = snapshot.add_image(Arc::new(
        moli_image::RgbaImage::try_new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap(),
    ));
    snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
        12.0, 10.0, 32.0, 16.0,
    )));
    snapshot.push_fragment(PaintFragment::Image(moli_layout::PaintImage {
        image,
        destination: PaintRect::new(8.0, 8.0, 2.0, 1.0),
        repeat_bounds: Some(PaintRect::new(8.0, 8.0, 40.0, 20.0)),
        sampling: moli_layout::PaintImageSampling::Nearest,
        transform: PaintTransform2D::IDENTITY,
    }));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let doc = document(&snapshot, &options());
    let repeated_images: Vec<_> = doc
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter(|stream| {
            stream.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image")
                && stream.dict.get(b"Width").and_then(|v| v.as_i64()).ok() == Some(64)
                && stream.dict.get(b"Height").and_then(|v| v.as_i64()).ok() == Some(32)
        })
        .map(|stream| stream.decompressed_content().expect("PDF image bytes"))
        .filter(|bytes| bytes.len() == 64 * 32 * 3)
        .collect();
    assert_eq!(repeated_images.len(), 1, "one bounded RGB image in the PDF");
    for (index, pixel) in repeated_images[0].chunks_exact(3).enumerate() {
        let expected = if ((index % 64) / 2).is_multiple_of(2) {
            [255, 0, 0]
        } else {
            [0, 255, 0]
        };
        assert_eq!(pixel, expected, "PDF pattern at pixel {index}");
    }
}

#[test]
fn pdf_rejects_nonfinite_image_repeat_bounds() {
    let mut snapshot = snapshot();
    let image = snapshot.add_image(Arc::new(
        moli_image::RgbaImage::try_new(1, 1, vec![255; 4]).unwrap(),
    ));
    snapshot.push_fragment(PaintFragment::Image(moli_layout::PaintImage {
        image,
        destination: PaintRect::new(0.0, 0.0, 1.0, 1.0),
        repeat_bounds: Some(PaintRect::new(0.0, 0.0, f32::NAN, 20.0)),
        sampling: moli_layout::PaintImageSampling::Nearest,
        transform: PaintTransform2D::IDENTITY,
    }));
    assert!(matches!(
        pdf_snapshot(&snapshot, &options()),
        Err(PdfError::Encoding(_))
    ));
}

fn content_operations(doc: &lopdf::Document) -> Vec<lopdf::content::Operation> {
    doc.objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .map(|stream| {
            stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone())
        })
        .filter_map(|bytes| lopdf::content::Content::decode(&bytes).ok())
        .flat_map(|content| content.operations)
        .collect()
}

#[test]
fn text_stroke_is_exported_as_a_pdf_stroke_with_one_semantic_copy() {
    for paint_first in [false, true] {
        let mut snapshot = snapshot();
        let mut run = text(&mut snapshot, "Stroke", 35.0, false);
        run.stroke = Some(moli_layout::PaintTextStroke {
            width: 2.0,
            color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
            paint_first,
        });
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
        let doc = document(&snapshot, &options());
        assert_eq!(extracted(&doc, &[1]), "Stroke", "paint_first={paint_first}");
        let operations = content_operations(&doc);
        let stroked_text = operations.iter().any(|op| {
            op.operator == "Tr" && op.operands.first().and_then(|o| o.as_i64().ok()) == Some(1)
        });
        let stroked_path = operations.iter().any(|op| op.operator == "S");
        assert!(
            stroked_text || stroked_path,
            "missing stroke operator, paint_first={paint_first}"
        );
        assert!(operations.iter().any(|op| {
            op.operator == "w" && op.operands.first().and_then(|o| o.as_float().ok()) == Some(2.0)
        }));
    }
}

#[test]
fn pdf_text_stroke_sets_the_miter_limit_and_keeps_miter_only_ink() {
    let mut snapshot = snapshot();
    let mut run = text(&mut snapshot, "A", 130.0, false);
    let font = peniko::FontData::new(
        peniko::Blob::new(Arc::new(
            include_bytes!("../../tests/fixtures/moli-text-stroke.ttf").to_vec(),
        )),
        0,
    );
    run.font = snapshot.intern_font(&font);
    run.font_size = 100.0;
    run.glyphs[0].id = 1;
    run.glyphs[0].x = 50.0;
    run.color = PaintColor::TRANSPARENT;
    run.stroke = Some(moli_layout::PaintTextStroke {
        width: 10.0,
        color: PaintColor::BLACK,
        paint_first: false,
    });
    // Only the miter tip (y=36..45) enters this clip; the unexpanded glyph
    // starts at y=50, and a round join reaches no further than y=45.
    snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
        75.0, 36.0, 10.0, 7.0,
    )));
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    snapshot.push_fragment(PaintFragment::PopLayer);
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1]), "A");
    let operations = content_operations(&doc);
    assert!(
        operations.iter().any(|op| {
            op.operator == "M" && op.operands.first().and_then(|o| o.as_float().ok()) == Some(4.0)
        }),
        "text strokes must explicitly set miter limit 4"
    );
    assert!(
        !operations.iter().any(|op| {
            op.operator == "j" && op.operands.first().and_then(|o| o.as_i64().ok()) == Some(1)
        }),
        "text strokes must not use round joins"
    );
}

#[test]
fn pdf_synthetic_bold_uses_css_pixels_with_and_without_text_stroke() {
    for cjk in [false, true] {
        for font_size in [16.0, 32.0, 64.0] {
            for stroke in [false, true] {
                let mut snapshot = snapshot();
                let label = if cjk { "中" } else { "B" };
                let mut run = text(&mut snapshot, label, 90.0, cjk);
                run.font_size = font_size;
                run.glyph_embolden = PaintPoint::new(0.3, 0.3);
                run.stroke = stroke.then_some(moli_layout::PaintTextStroke {
                    width: 2.0,
                    color: PaintColor::BLACK,
                    paint_first: false,
                });
                snapshot.push_fragment(PaintFragment::GlyphRun(run));
                let doc = document(&snapshot, &options());
                assert_eq!(extracted(&doc, &[1]), label);
                let widths: Vec<_> = content_operations(&doc)
                    .iter()
                    .filter(|op| op.operator == "w")
                    .filter_map(|op| op.operands.first()?.as_float().ok())
                    .collect();
                assert!(
                    widths.iter().any(|width| (width - 0.6).abs() < 0.001),
                    "cjk={cjk}, size={font_size}, stroke={stroke}: {widths:?}"
                );
                if stroke {
                    assert!(widths.iter().any(|width| (width - 2.6).abs() < 0.001));
                }
            }
        }
    }
}

#[test]
fn pdf_text_stroke_only_ink_survives_clipping_and_transforms() {
    for scale in [1.0_f32, 2.0] {
        let mut snapshot = snapshot();
        let mut run = text(&mut snapshot, "A", 50.0, false);
        run.glyphs[0].x = 70.0;
        run.stroke = Some(moli_layout::PaintTextStroke {
            width: 8.0,
            color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
            paint_first: false,
        });
        run.transform.coefficients = [f64::from(scale), 0.0, 0.0, f64::from(scale), 0.0, 0.0];
        snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
            0.0,
            0.0,
            67.0 * scale,
            100.0 * scale,
        )));
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
        snapshot.push_fragment(PaintFragment::PopLayer);
        let doc = document(&snapshot, &options());
        assert_eq!(extracted(&doc, &[1]), "A", "scale={scale}");
        assert!(content_operations(&doc).iter().any(|op| {
            op.operator == "S"
                || (op.operator == "Tr"
                    && op.operands.first().and_then(|o| o.as_i64().ok()) == Some(1))
        }));
    }
}

#[test]
fn pdf_text_stroke_crosses_page_boundaries_without_duplicating_text() {
    let mut snapshot = snapshot();
    let mut run = text(&mut snapshot, "A", 299.0, false);
    run.stroke = Some(moli_layout::PaintTextStroke {
        width: 8.0,
        color: PaintColor::BLACK,
        paint_first: false,
    });
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    let doc = document(&snapshot, &options());
    assert_eq!(extracted(&doc, &[1, 2]), "A");
    let page = doc.get_pages()[&2];
    let operations = lopdf::content::Content::decode(&doc.get_page_content(page))
        .unwrap()
        .operations;
    assert!(
        operations.iter().any(|op| op.operator == "S"),
        "the second page must retain the outline crossing the page edge"
    );
}

#[test]
fn filtered_wide_text_stroke_retains_all_rasterized_ink() {
    use moli_layout::{PaintBlendMode, PaintCompositeMode, PaintFilter};
    for (scale, width, skew, label) in [
        (1.0_f64, 8.0, None, "A"),
        (1.0, 100.0, None, "A"),
        (1.5, 100.0, None, "A"),
        (1.5, 8.0, Some(0.7), "A"),
        (1.5, 100.0, Some(0.7), "A"),
        (1.0, 8.0, None, "A A"),
        (1.0, 8.0, None, "中"),
        (1.5, 8.0, None, "B"),
        (1.0, 2.0, None, "C"),
    ] {
        let mut snapshot =
            PaintSnapshot::new(PaintViewport::new(300, 300, 2.0), PaintColor::TRANSPARENT);
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite: PaintCompositeMode::SrcOver,
            clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 300.0, 300.0)),
            transform: PaintTransform2D::IDENTITY,
            filter: Some(PaintFilter::Opacity(0.5)),
        });
        let mut run = text(&mut snapshot, label, 150.0, label == "中");
        for glyph in &mut run.glyphs {
            glyph.x += 140.0;
        }
        run.glyph_skew_radians = skew;
        if label == "B" {
            let font = peniko::FontData::new(
                peniko::Blob::new(Arc::new(
                    include_bytes!("../../tests/fixtures/moli-pdf.ttf").to_vec(),
                )),
                0,
            );
            run.font = snapshot.intern_font(&font);
            run.normalized_coords = vec![12288];
        }
        if label == "C" {
            let font = peniko::FontData::new(
                peniko::Blob::new(Arc::new(
                    include_bytes!("../../tests/fixtures/moli-text-stroke.ttf").to_vec(),
                )),
                0,
            );
            run.font = snapshot.intern_font(&font);
            run.glyphs[0].id = 3;
        }
        run.color = PaintColor::TRANSPARENT;
        run.stroke = Some(moli_layout::PaintTextStroke {
            width,
            color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
            paint_first: false,
        });
        run.transform.coefficients = [
            scale,
            0.0,
            0.0,
            scale,
            150.0 * (1.0 - scale),
            150.0 * (1.0 - scale),
        ];
        if label == "A A" {
            let mut empty = run.clone();
            empty.glyphs = vec![run.glyphs[1]];
            empty.text = None;
            snapshot.push_fragment(PaintFragment::GlyphRun(empty));
        }
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
        snapshot.push_fragment(PaintFragment::PopLayer);
        let reference = crate::raster_snapshot(&snapshot).unwrap();
        let expected_ink = reference
            .rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 0)
            .count();
        assert!(expected_ink > 0);
        let doc = document(&snapshot, &options());
        assert_eq!(extracted(&doc, &[1]), label);
        let images: Vec<_> = doc
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| stream.dict.has(b"SMask"))
            .collect();
        assert_eq!(images.len(), 1, "one filtered text image");
        let image_width = images[0].dict.get(b"Width").unwrap().as_i64().unwrap();
        let image_height = images[0].dict.get(b"Height").unwrap().as_i64().unwrap();
        if width == 8.0 && scale == 1.0 && matches!(label, "A" | "A A") {
            // A 20px glyph plus an 8px outline should not allocate the old
            // 224x224 image around its origin. Measure ink and keep the
            // conservative miter margin rather than a whole-em guard.
            assert!(
                image_width * image_height < 20_000,
                "oversized filter raster: {image_width}x{image_height}"
            );
        }
        let masks: Vec<_> = doc
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter_map(|stream| stream.dict.get(b"SMask").ok()?.as_reference().ok())
            .map(|id| {
                doc.get_object(id)
                    .unwrap()
                    .as_stream()
                    .unwrap()
                    .decompressed_content()
                    .unwrap()
            })
            .collect();
        assert_eq!(masks.len(), 1, "one filtered text image");
        let actual_ink = masks[0].iter().filter(|&&alpha| alpha > 0).count();
        assert_eq!(
            actual_ink, expected_ink,
            "filter allocation must preserve the complete stroke, scale={scale}, stroke={width}, skew={skew:?}, text={label:?}"
        );
    }
}

#[test]
fn filtered_hollow_text_keeps_one_searchable_semantic_copy() {
    use moli_layout::{PaintBlendMode, PaintCompositeMode, PaintFilter};
    for paint_first in [false, true] {
        let mut snapshot = snapshot();
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite: PaintCompositeMode::SrcOver,
            clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 200.0, 100.0)),
            transform: PaintTransform2D::IDENTITY,
            filter: Some(PaintFilter::Blur(1.0)),
        });
        let mut run = text(&mut snapshot, "TEST", 50.0, false);
        run.color = PaintColor::TRANSPARENT;
        run.stroke = Some(moli_layout::PaintTextStroke {
            width: 4.0,
            color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
            paint_first,
        });
        snapshot.push_fragment(PaintFragment::GlyphRun(run));
        snapshot.push_fragment(PaintFragment::PopLayer);
        let doc = document(&snapshot, &options());
        assert_eq!(
            extracted(&doc, &[1, 2]),
            "TEST",
            "paint_first={paint_first}"
        );
        assert!(doc.objects.values().any(|object| {
            object.as_stream().is_ok_and(|stream| {
                stream
                    .dict
                    .get(b"Subtype")
                    .and_then(|value| value.as_name())
                    .ok()
                    == Some(b"Image")
            })
        }));
    }
}

fn filtered_fixture_glyph(bytes: &[u8], ch: char, stroke: Option<f32>) -> PaintSnapshot {
    use moli_layout::{PaintBlendMode, PaintCompositeMode, PaintFilter, PaintTextStroke};
    let mut snapshot =
        PaintSnapshot::new(PaintViewport::new(300, 300, 2.0), PaintColor::TRANSPARENT);
    let mut run = text(&mut snapshot, "A", 150.0, false);
    let font = peniko::FontData::new(peniko::Blob::new(Arc::new(bytes.to_vec())), 0);
    let face = skrifa::FontRef::from_index(bytes, 0).unwrap();
    run.font = snapshot.intern_font(&font);
    run.font_size = 100.0;
    run.glyphs[0].id = face.charmap().map(ch).unwrap().to_u32();
    run.glyphs[0].x = 100.0;
    run.color = if stroke.is_some() {
        PaintColor::TRANSPARENT
    } else {
        PaintColor::BLACK
    };
    run.stroke = stroke.map(|width| PaintTextStroke {
        width,
        color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
        paint_first: false,
    });
    let semantics = run.text.as_mut().unwrap();
    semantics.text = Arc::from(ch.to_string());
    semantics.clusters[0] = 0..ch.len_utf8();
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 300.0, 300.0)),
        transform: PaintTransform2D::IDENTITY,
        filter: Some(PaintFilter::Opacity(0.5)),
    });
    snapshot.push_fragment(PaintFragment::GlyphRun(run));
    snapshot.push_fragment(PaintFragment::PopLayer);
    snapshot
}

// Count actual painted pixels and each fixture color, including alpha. This
// catches losing a color layer even when the base glyph remains visible.
fn glyph_ink_counts(pixels: &[u8]) -> [usize; 3] {
    let mut counts = [0; 3];
    for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0) {
        counts[0] += 1;
        counts[1] += usize::from(pixel[2] > 200 && pixel[0] < 60 && pixel[1] < 120);
        counts[2] += usize::from(pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60);
    }
    counts
}

fn assert_filtered_fixture_ink(snapshot: &PaintSnapshot, label: &str) -> [usize; 3] {
    let reference = crate::raster_snapshot(snapshot).unwrap();
    let expected = glyph_ink_counts(&reference.rgba);
    let doc = document(snapshot, &options());
    let images: Vec<_> = doc
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter(|stream| stream.dict.has(b"SMask"))
        .collect();
    if expected[0] == 0 {
        assert!(
            images.is_empty(),
            "empty glyphs must not allocate a PDF image"
        );
        return expected;
    }
    // lopdf's general extractor prefers Type3 Differences names to ToUnicode.
    // Decode the emitted text through its CMap, as PDF readers do. These
    // one-glyph fixtures have exactly one font in their page resources.
    let page = doc.get_pages()[&1];
    let fonts = doc.get_page_fonts(page).unwrap();
    assert_eq!(fonts.len(), 1);
    let mut font = (*fonts.values().next().unwrap()).clone();
    if font.get(b"Subtype").unwrap().as_name().unwrap() == b"Type3" {
        font.remove(b"Encoding");
    }
    let encoding = font.get_font_encoding(&doc).unwrap();
    let content = lopdf::content::Content::decode(&doc.get_page_content(page)).unwrap();
    let mut text = String::new();
    for operation in content
        .operations
        .iter()
        .filter(|op| matches!(op.operator.as_str(), "Tj" | "TJ"))
    {
        for operand in &operation.operands {
            let items = match operand {
                lopdf::Object::Array(items) => items.as_slice(),
                value => std::slice::from_ref(value),
            };
            for item in items {
                if let lopdf::Object::String(bytes, _) = item {
                    text.push_str(&encoding.bytes_to_string(bytes).unwrap());
                }
            }
        }
    }
    assert_eq!(text, label);
    assert_eq!(images.len(), 1);
    let rgb = images[0].decompressed_content().unwrap();
    let mask = images[0]
        .dict
        .get(b"SMask")
        .unwrap()
        .as_reference()
        .unwrap();
    let alpha = doc
        .get_object(mask)
        .unwrap()
        .as_stream()
        .unwrap()
        .decompressed_content()
        .unwrap();
    assert_eq!(rgb.len(), alpha.len() * 3);
    let pixels: Vec<_> = rgb
        .chunks_exact(3)
        .zip(alpha)
        .flat_map(|(rgb, alpha)| [rgb[0], rgb[1], rgb[2], alpha])
        .collect();
    assert_eq!(
        glyph_ink_counts(&pixels),
        expected,
        "all glyph layers and stroke ink must survive"
    );
    expected
}

#[test]
fn filtered_colr_layers_outside_the_base_outline_survive() {
    let snapshot = filtered_fixture_glyph(
        include_bytes!("../../tests/fixtures/moli-pdf.ttf"),
        '★',
        None,
    );
    let counts = assert_filtered_fixture_ink(&snapshot, "★");
    assert!(counts[1] > 0, "the blue layer lies above the base outline");
}

#[test]
fn filtered_colr_glyph_with_an_empty_base_keeps_ink_and_text() {
    let snapshot = filtered_fixture_glyph(
        include_bytes!("../../tests/fixtures/moli-colr.ttf"),
        '☆',
        None,
    );
    let counts = assert_filtered_fixture_ink(&snapshot, "☆");
    assert!(
        counts[1] > 0 && counts[2] > 0,
        "both color layers are visible"
    );
}
