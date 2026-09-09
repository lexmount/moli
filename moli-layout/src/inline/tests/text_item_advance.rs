use super::*;
use crate::{DocumentLayoutServices, SystemFontPolicy, WebFontFace, WebFontRegistration};
use parley::{Alignment, AlignmentOptions};

mod shared_glyphs;

fn shaped(text: &str) -> Layout<TextBrush> {
    shaped_with_boxes(text, &[], 0.0)
}

fn shaped_with_boxes(text: &str, boxes: &[InlineBox], letter_spacing: f32) -> Layout<TextBrush> {
    shaped_with_font(
        text,
        boxes,
        letter_spacing,
        "\"Advance Fixture\", \"Advance CJK\"",
    )
}

fn shaped_with_font(
    text: &str,
    boxes: &[InlineBox],
    letter_spacing: f32,
    font_family: &'static str,
) -> Layout<TextBrush> {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    for (name, data) in [
        (
            "Advance Fixture",
            include_bytes!("../../../tests/fixtures/moli-ahem.ttf").as_slice(),
        ),
        (
            "Advance CJK",
            include_bytes!("../../../tests/fixtures/moli-cjk.ttf").as_slice(),
        ),
        (
            "Shared Glyph Fixture",
            include_bytes!("../../../tests/fixtures/moli-ligatures.ttf").as_slice(),
        ),
    ] {
        services
            .register_web_font(WebFontRegistration::new(
                name,
                WebFontFace::new(name),
                data.to_vec(),
            ))
            .unwrap();
    }
    let mut style = TextStyle {
        font_family: parley::FontFamily::Source(font_family.into()),
        font_size: 13.0,
        letter_spacing,
        ..TextStyle::default()
    };
    let parley = services.parley_mut();
    parley.resolve_font_families(&mut style, None);
    let mut builder =
        parley
            .layout_context
            .style_run_builder(&mut parley.font_context, text, 1.0, false);
    let style_index = builder.push_style(style);
    builder.push_style_run(style_index, ..);
    for inline_box in boxes {
        builder.push_inline_box(inline_box.clone());
    }
    builder.build(text)
}

fn glyphs(layout: &Layout<TextBrush>) -> Vec<parley::Glyph> {
    layout
        .lines()
        .flat_map(|line| line.items())
        .flat_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => run.positioned_glyphs().collect::<Vec<_>>(),
            PositionedLayoutItem::InlineBox(_) => Vec::new(),
        })
        .collect()
}

#[test]
fn text_item_quantization_preserves_raw_glyphs_and_rounds_each_node_once() {
    let text = "iiii";
    let mut raw = shaped(text);
    raw.break_all_lines(None);
    let raw_glyphs = glyphs(&raw);
    assert_eq!(raw_glyphs.len(), 4);

    let mut single = shaped(text);
    single.set_text_item_quantization(1.0 / 64.0, &[4]);
    assert_eq!(single.calculate_content_widths().max, 31.203125);
    single.break_all_lines(None);
    assert_eq!(single.width(), 31.203125);
    assert_eq!(
        glyphs(&single),
        raw_glyphs,
        "allocation must not round glyph advances"
    );

    let mut split = shaped(text);
    split.set_text_item_quantization(1.0 / 64.0, &[1, 2, 4]);
    assert_eq!(split.calculate_content_widths().max, 31.234375);
    split.break_all_lines(None);
    assert_eq!(split.width(), 31.234375);
    let allocated_glyphs = glyphs(&split);
    assert_eq!(allocated_glyphs[0].x, 0.0);
    assert_eq!(allocated_glyphs[1].x, 7.8125);
    assert_eq!(allocated_glyphs[2].x, 15.625);
    assert_eq!(allocated_glyphs[3].x, 15.625 + raw_glyphs[0].advance);
    assert_eq!(
        allocated_glyphs
            .iter()
            .map(|g| g.advance)
            .collect::<Vec<_>>(),
        raw_glyphs.iter().map(|g| g.advance).collect::<Vec<_>>()
    );
}

#[test]
fn text_item_quantization_drives_line_breaks_and_survives_repeated_probes() {
    let mut single = shaped("ii ii");
    single.set_text_item_quantization(1.0 / 64.0, &[5]);
    single.break_all_lines(Some(39.0));
    assert_eq!(single.len(), 1);

    let mut split = shaped("ii ii");
    split.set_text_item_quantization(1.0 / 64.0, &[1, 3, 5]);
    assert_eq!(split.calculate_content_widths().max, 39.03125);
    assert_eq!(split.calculate_content_widths().min, 15.625);
    let source = split.clone();
    for (width, lines) in [(39.0, 2), (39.046875, 1), (39.0, 2)] {
        split.break_all_lines(Some(width));
        let mut fresh = source.clone();
        fresh.break_all_lines(Some(width));
        assert_eq!(split.len(), lines);
        assert_eq!(split.width(), fresh.width());
        assert_eq!(glyphs(&split), glyphs(&fresh));
    }
}

#[test]
fn text_item_quantization_does_not_split_allocation_at_font_fallback() {
    let text = "i中i";
    let mut raw = shaped(text);
    raw.break_all_lines(None);
    assert!(raw.lines().next().unwrap().runs().count() >= 3);
    let mut quantized = shaped(text);
    quantized.set_text_item_quantization(1.0 / 64.0, &[text.len()]);
    let width = (raw.width() * 64.0).ceil() / 64.0;
    assert_eq!(quantized.calculate_content_widths().max, width);
    quantized.break_all_lines(None);
    assert_eq!(quantized.width(), width);
    assert_eq!(glyphs(&quantized), glyphs(&raw));
}

#[test]
fn text_fragment_bounds_encompass_raw_offsets_without_per_character_advance() {
    let advance = 4.4453125;
    assert_eq!(
        encompass_text_inline_bounds(advance, advance),
        (4.4375, 4.453125)
    );
    assert_eq!(
        encompass_text_inline_bounds(0.0, 4.0 * advance),
        (0.0, 17.78125)
    );
    assert_eq!(
        encompass_text_inline_bounds(-advance, advance),
        (-4.453125, 4.453125)
    );
}

#[test]
fn zero_advance_text_fragment_remains_zero_width_at_fractional_offset() {
    assert_eq!(
        encompass_text_inline_bounds(3.6118164, 0.0),
        (3.609375, 0.0)
    );
}

#[test]
fn text_item_quantization_allocates_inline_boxes_between_shaped_items() {
    let mut layout = shaped_with_boxes(
        "iiii",
        &[InlineBox {
            id: 1,
            index: 1,
            width: 5.25,
            height: 10.0,
            kind: InlineBoxKind::InFlow,
        }],
        0.0,
    );
    layout.set_text_item_quantization(1.0 / 64.0, &[4]);
    assert_eq!(layout.calculate_content_widths().max, 36.46875);
    layout.break_all_lines(None);
    assert_eq!(layout.width(), 36.46875);
    let positions = glyphs(&layout);
    assert_eq!(positions[1].x, 13.0625);
    let inline_box = layout
        .lines()
        .flat_map(|line| line.items())
        .find_map(|item| {
            if let PositionedLayoutItem::InlineBox(inline_box) = item {
                Some(inline_box)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(inline_box.x, 7.8125);
}

#[test]
fn text_item_quantization_keeps_glyph_and_cluster_positions_aligned_after_justification() {
    let mut layout = shaped("ii ii ii");
    layout.set_text_item_quantization(1.0 / 64.0, &[1, 3, 4, 6, 8]);
    layout.break_all_lines(Some(41.0));
    let unaligned = glyphs(&layout);
    layout.align(Alignment::Justify, AlignmentOptions::default());
    for line in layout.lines() {
        let cluster_offsets = line
            .runs()
            .flat_map(|run| {
                run.visual_clusters()
                    .map(|cluster| cluster.visual_offset().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let glyph_offsets = line
            .items()
            .flat_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run
                    .positioned_glyphs()
                    .map(|glyph| glyph.x)
                    .collect::<Vec<_>>(),
                PositionedLayoutItem::InlineBox(_) => Vec::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(cluster_offsets.len(), glyph_offsets.len());
        for (cluster, glyph) in cluster_offsets.iter().zip(&glyph_offsets) {
            assert!(
                (cluster - glyph).abs() < 0.000_01,
                "cluster={cluster}, glyph={glyph}"
            );
        }
    }
    layout.align(Alignment::Start, AlignmentOptions::default());
    let restored = glyphs(&layout);
    for (before, after) in unaligned.iter().zip(&restored) {
        assert!((before.x - after.x).abs() < 0.000_01);
        assert!((before.advance - after.advance).abs() < 0.000_01);
    }
}

#[test]
fn text_item_quantization_does_not_allocate_negative_item_widths() {
    let mut layout = shaped_with_boxes("iiii", &[], -10.0);
    layout.set_text_item_quantization(1.0 / 64.0, &[1, 2, 3, 4]);
    assert_eq!(layout.calculate_content_widths().max, 0.0);
    layout.break_all_lines(None);
    assert_eq!(layout.full_width(), 0.0);
    assert!(
        layout
            .lines()
            .flat_map(|line| line.runs())
            .all(|run| run.advance() == 0.0)
    );
    assert!(glyphs(&layout).iter().all(|glyph| glyph.x == 0.0));
}
