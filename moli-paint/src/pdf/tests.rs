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
