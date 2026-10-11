// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    test_name,
    util::{ColorBrush, TestEnv},
};
use parley::VerticalAlign;
use parley::{Alignment, AlignmentOptions, InlineBox, InlineBoxKind, Layout, PositionedLayoutItem};

#[test]
fn out_of_flow_box_has_no_effect_on_layout() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "Hello world! This is a test of out-of-flow boxes.";

    // Reference layout without any boxes
    let builder_ref = env.ranged_builder(text);
    let layout_ref = builder_ref.build(text);
    let widths_ref = layout_ref.calculate_content_widths();

    // Treatment layout with a large OutOfFlow box
    let mut builder_oof = env.ranged_builder(text);
    builder_oof.push_inline_box(InlineBox {
        id: 42,
        kind: InlineBoxKind::OutOfFlow,
        index: 6,
        width: 9999.0,
        height: 9999.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });
    let layout_oof = builder_oof.build(text);
    let widths_oof = layout_oof.calculate_content_widths();

    // Content widths should be unaffected
    assert_eq!(widths_ref.min, widths_oof.min);
    assert_eq!(widths_ref.max, widths_oof.max);

    // Perform line breaking and alignment
    let mut layout_ref = layout_ref;
    layout_ref.break_all_lines(Some(100.0));
    layout_ref.align(Alignment::Start, AlignmentOptions::default());

    let mut layout_oof = layout_oof;
    layout_oof.break_all_lines(Some(100.0));
    layout_oof.align(Alignment::Start, AlignmentOptions::default());

    // Layout dimensions and line count should be unaffected
    assert_eq!(layout_ref.width(), layout_oof.width());
    assert_eq!(layout_ref.height(), layout_oof.height());
    assert_eq!(layout_ref.len(), layout_oof.len());

    // Glyph positions should be identical
    let glyph_xs = |layout: &Layout<ColorBrush>| -> Vec<f32> {
        layout
            .lines()
            .flat_map(|line| {
                line.items().filter_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        Some(run.positioned_glyphs().map(|g| g.x).collect::<Vec<_>>())
                    }
                    _ => None,
                })
            })
            .flatten()
            .collect()
    };
    assert_eq!(glyph_xs(&layout_ref), glyph_xs(&layout_oof),);

    // The OutOfFlow box should still appear in the positioned items
    let found_oof = layout_oof.lines().flat_map(|l| l.items()).any(|item| {
        matches!(
            item,
            PositionedLayoutItem::InlineBox(ref ibox)
                if ibox.id == 42 && ibox.kind == InlineBoxKind::OutOfFlow
        )
    });
    assert!(found_oof);
}

/// An out-of-flow box is not in-flow content, so it must not introduce a line break
/// opportunity: a word containing one still overflows rather than being split.
#[test]
fn out_of_flow_box_is_not_a_line_break_opportunity() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "unbreakable";

    let line_ranges = |layout: &Layout<ColorBrush>| -> Vec<_> {
        layout.lines().map(|line| line.text_range()).collect()
    };

    let mut layout_ref = env.ranged_builder(text).build(text);
    layout_ref.break_all_lines(Some(20.0));
    assert_eq!(layout_ref.len(), 1);

    // A box anywhere in the word, including at either edge, leaves the line breaking unchanged.
    for index in [0, 5, text.len()] {
        let mut builder = env.ranged_builder(text);
        builder.push_inline_box(InlineBox {
            id: 42,
            kind: InlineBoxKind::OutOfFlow,
            index,
            width: 9999.0,
            height: 9999.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(20.0));

        assert_eq!(layout.width(), layout_ref.width(), "index {index}");
        assert_eq!(layout.height(), layout_ref.height(), "index {index}");
        assert_eq!(
            line_ranges(&layout),
            line_ranges(&layout_ref),
            "index {index}"
        );
    }
}
