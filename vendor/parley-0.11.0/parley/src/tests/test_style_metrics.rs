// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for per-style span box metrics.

use super::utils::ColorBrush;
use super::utils::fonts::{FONT_FAMILY_LIST, create_font_context};
use crate::layout::style_metrics::StyleMetrics;
use crate::{
    AlignmentBaseline, BaselineShift, FontFamily, InlineBox, InlineBoxKind, Layout, LayoutContext,
    LineHeight, StyleProperty, TextStyle, VerticalAlign,
};

fn root_style() -> TextStyle<'static, 'static, ColorBrush> {
    TextStyle {
        font_family: FontFamily::from(FONT_FAMILY_LIST),
        font_size: 20.,
        line_height: LineHeight::Absolute(30.),
        ..TextStyle::default()
    }
}

/// Root style with three spans: `A` (40px) containing `B` (10px, `vertical-align: super`) and a
/// sibling `top` aligned span `C` containing `D`.
fn build() -> Layout<ColorBrush> {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root);
    builder.push_text("root ");
    builder.push_style_span(TextStyle {
        font_size: 40.,
        line_height: LineHeight::Absolute(50.),
        ..root_style()
    });
    builder.push_style_span(TextStyle {
        font_size: 10.,
        line_height: LineHeight::Absolute(20.),
        vertical_align: VerticalAlign::SUPER,
        ..root_style()
    });
    builder.push_text("B");
    builder.pop_style_span();
    builder.pop_style_span();
    builder.push_style_modification_span(&[StyleProperty::VerticalAlign(VerticalAlign::TOP)]);
    builder
        .push_style_modification_span(&[StyleProperty::VerticalAlign(VerticalAlign::length(3.))]);
    builder.push_text("D");
    builder.pop_style_span();
    builder.pop_style_span();
    let (layout, _) = builder.build();
    layout
}

fn metrics(layout: &Layout<ColorBrush>) -> &[StyleMetrics] {
    &layout.data.style_metrics
}

#[test]
fn one_metrics_entry_per_style() {
    let layout = build();
    assert_eq!(layout.data.styles.len(), 5);
    assert_eq!(metrics(&layout).len(), 5);
}

#[test]
fn line_height_is_distributed_as_half_leading() {
    let layout = build();
    let root = metrics(&layout)[0];
    assert!((root.over + root.under - 30.).abs() < 1e-4);
    assert!(root.over > root.ascent && root.under > root.descent);
    // Roboto at 20px.
    assert!(root.ascent > 15. && root.ascent < 20.);
    assert!(root.x_height > 9. && root.x_height < 12.);
    assert_eq!(root.baseline_offset, 0.);
    assert_eq!(root.aligned_subtree_root, 0);
}

#[test]
fn ancestors_without_text_have_metrics() {
    let layout = build();
    // Span A has no direct text but still has a style entry with its own 40px metrics.
    let a = metrics(&layout)[1];
    assert!((a.over + a.under - 50.).abs() < 1e-4);
    assert!(a.ascent > 30.);
    assert_eq!(a.baseline_offset, 0.);
    assert_eq!(a.aligned_subtree_root, 0);
}

#[test]
fn super_shifts_relative_to_parent() {
    let layout = build();
    let b = metrics(&layout)[2];
    assert_eq!(layout.data.styles[2].parent, 1);
    // WebKit/Blink constant: a third of the *parent's* font size (40px).
    assert!((b.baseline_offset - 40. / 3.).abs() < 1e-4);
    assert_eq!(b.aligned_subtree_root, 0);
}

#[test]
fn top_starts_an_independent_aligned_subtree() {
    let layout = build();
    let c = metrics(&layout)[3];
    let d = metrics(&layout)[4];
    assert_eq!(c.baseline_offset, 0.);
    assert_eq!(c.aligned_subtree_root, 3);
    // Children of a `top` box are relative to it, not to the root.
    assert_eq!(d.baseline_offset, 3.);
    assert_eq!(d.aligned_subtree_root, 3);
}

#[test]
fn text_top_middle_and_text_bottom() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root);
    for align in [
        VerticalAlign::TEXT_TOP,
        VerticalAlign::MIDDLE,
        VerticalAlign::TEXT_BOTTOM,
    ] {
        builder.push_style_span(TextStyle {
            font_size: 10.,
            line_height: LineHeight::Absolute(12.),
            vertical_align: align,
            ..root_style()
        });
        builder.push_text("x");
        builder.pop_style_span();
    }
    let (layout, _) = builder.build();
    let m = metrics(&layout);
    let root = m[0];
    let (text_top, middle, text_bottom) = (m[1], m[2], m[3]);
    // The box top sits at the parent's content ascent.
    assert!((text_top.baseline_offset + text_top.over - root.ascent).abs() < 1e-4);
    // The box midpoint sits at half the parent's x-height.
    let mid = middle.baseline_offset + (middle.over - middle.under) / 2.;
    assert!((mid - root.x_height / 2.).abs() < 1e-4);
    // The box bottom sits at the parent's content descent.
    assert!((text_bottom.baseline_offset - text_bottom.under + root.descent).abs() < 1e-4);
}

#[test]
fn alignment_baseline_and_baseline_shift_compose() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root);
    for align in [
        VerticalAlign::TEXT_TOP,
        VerticalAlign::new(AlignmentBaseline::TextTop, BaselineShift::Length(-4.)),
        VerticalAlign::new(AlignmentBaseline::TextTop, BaselineShift::Super),
    ] {
        builder.push_style_span(TextStyle {
            font_size: 10.,
            vertical_align: align,
            ..root_style()
        });
        builder.push_text("x");
        builder.pop_style_span();
    }
    let (layout, _) = builder.build();
    let m = metrics(&layout);
    // `vertical-align: text-top -4px` is `text-top` lowered by 4px ...
    assert!((m[2].baseline_offset - (m[1].baseline_offset - 4.)).abs() < 1e-4);
    // ... and `text-top super` is `text-top` raised by a third of the parent font size.
    assert!((m[3].baseline_offset - (m[1].baseline_offset + m[0].font_size / 3.)).abs() < 1e-4);
}

#[test]
fn trailing_line_after_newline_keeps_newline_style() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = TextStyle {
        line_height: LineHeight::Absolute(10.),
        ..root_style()
    };
    let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root);
    builder.push_style_span(TextStyle {
        font_size: 40.,
        line_height: LineHeight::Absolute(40.),
        ..root_style()
    });
    builder.push_text("x\n");
    builder.pop_style_span();
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    let mut lines = layout.lines();
    let first = lines.next().unwrap();
    let last = lines.next().unwrap();
    assert!(lines.next().is_none());
    assert_eq!(first.metrics().line_height, 40.);
    assert_eq!(last.metrics().line_height, 40.);
    assert_eq!(last.metrics().block_min_coord, 40.);
    assert_eq!(layout.height(), 80.);
}

#[test]
fn quantized_shifted_baselines_are_whole_pixels() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., true, &root);
    builder.push_text("a");
    for align in [
        VerticalAlign::length(0.5),
        VerticalAlign::MIDDLE,
        VerticalAlign::SUPER,
    ] {
        builder.push_style_span(TextStyle {
            font_size: 13.,
            line_height: LineHeight::Absolute(15.5),
            vertical_align: align,
            ..root_style()
        });
        builder.push_text("b");
        builder.pop_style_span();
    }
    builder.push_style_span(TextStyle {
        vertical_align: VerticalAlign::BOTTOM,
        ..root_style()
    });
    builder.push_style_span(TextStyle {
        font_size: 13.,
        vertical_align: VerticalAlign::length(0.5),
        ..root_style()
    });
    builder.push_text("c");
    builder.pop_style_span();
    builder.pop_style_span();
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    let line = layout.lines().next().unwrap();
    let mut runs = 0;
    for item in line.items() {
        if let crate::PositionedLayoutItem::GlyphRun(run) = item {
            assert_eq!(run.baseline(), run.baseline().round());
            runs += 1;
        }
    }
    assert_eq!(runs, 5);
}

#[test]
fn quantization_does_not_compound_along_ancestors() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., true, &root);
    builder.push_text("a");
    for _ in 0..3 {
        builder.push_style_span(TextStyle {
            vertical_align: VerticalAlign::length(0.4),
            ..root_style()
        });
    }
    builder.push_text("b");
    for _ in 0..3 {
        builder.pop_style_span();
    }
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    let line = layout.lines().next().unwrap();
    let mut baselines = line.items().filter_map(|item| match item {
        crate::PositionedLayoutItem::GlyphRun(run) => Some(run.baseline()),
        _ => None,
    });
    let a = baselines.next().unwrap();
    let b = baselines.next().unwrap();
    // 3 × 0.4 = 1.2 rounds to a 1px shift; rounding each level would give none.
    assert_eq!(a - b, 1.);
}

#[test]
fn quantized_metrics_are_whole_pixels() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = TextStyle {
        line_height: LineHeight::MetricsRelative(1.),
        ..root_style()
    };
    let mut builder = lcx.tree_builder(&mut fcx, 1., true, &root);
    builder.push_text("x");
    let (layout, _) = builder.build();
    let root = metrics(&layout)[0];
    assert_eq!(root.ascent.fract(), 0.);
    assert_eq!(root.descent.fract(), 0.);
    assert_eq!(root.over.fract(), 0.);
    assert_eq!(root.under.fract(), 0.);
}

#[test]
fn line_with_only_empty_inline_boxes_is_invisible() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut line_height = |height: f32, kind: InlineBoxKind| {
        let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root);
        builder.push_inline_box(InlineBox {
            id: 0,
            kind,
            index: 0,
            width: 0.,
            height,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        layout.lines().next().unwrap().metrics().line_height
    };
    // Empty and out-of-flow boxes alone do not make the strut apply.
    assert_eq!(line_height(0., InlineBoxKind::InFlow), 0.);
    assert_eq!(line_height(10., InlineBoxKind::OutOfFlow), 0.);
    // A non-empty in-flow box does.
    assert_eq!(line_height(10., InlineBoxKind::InFlow), 30.);
}

/// With quantization, an inline box's fractional height must not be rounded up twice (once for
/// the ascent and once for the descent): the line reserves exactly the box's height.
#[test]
fn quantized_inline_box_extents_are_not_rounded_separately() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., true, &root);
    // Baseline splits the box into 40.5 above and 40.5 below the baseline; rounding each
    // separately would reserve 82px for an 81px box.
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 0,
        width: 10.,
        height: 81.,
        baseline: Some(40.5),
        vertical_align: VerticalAlign::BASELINE,
    });
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    let line = layout.lines().next().unwrap();
    assert_eq!(line.metrics().line_height, 81.);
}

/// A box whose reserved space is negative (e.g. an atomic inline with a large negative
/// `margin-top`) is still content: the strut applies and the line keeps its height.
#[test]
fn line_with_negative_height_inline_box_is_not_invisible() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let root = root_style();
    let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root);
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 0,
        width: 10.,
        height: -300.,
        baseline: Some(-305.),
        vertical_align: VerticalAlign::BASELINE,
    });
    let (mut layout, _) = builder.build();
    layout.break_all_lines(None);
    let line = layout.lines().next().unwrap();
    assert_eq!(line.metrics().line_height, 30.);
    let inline_box = line
        .items()
        .find_map(|item| match item {
            crate::PositionedLayoutItem::InlineBox(inline_box) => Some(inline_box),
            _ => None,
        })
        .unwrap();
    // The box's own baseline coincides with the line's baseline.
    assert_eq!(
        inline_box.y + inline_box.baseline.unwrap(),
        line.metrics().baseline
    );
}

#[test]
fn fallback_font_glyphs_only_size_the_line_for_normal_line_height() {
    // CSS Inline 3 § 4.1: with a non-`normal` `line-height`, a span box's layout bounds derive
    // solely from its first available font, ignoring glyphs from fallback fonts.
    let mut fcx = create_font_context();
    let kufi = fcx.collection.family_id("Noto Kufi Arabic").unwrap();
    fcx.collection
        .set_fallbacks(fontique::Script::from_bytes(*b"Arab"), [kufi].into_iter());
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let roboto = [parlance::FontFamilyName::Named("Roboto".into())];
    let mut line_height = |text: &str, line_height: LineHeight| {
        let mut builder = lcx.tree_builder(
            &mut fcx,
            1.,
            false,
            &TextStyle {
                font_family: FontFamily::from(&roboto[..]),
                font_size: 20.,
                line_height,
                ..TextStyle::default()
            },
        );
        builder.push_text(text);
        let (mut layout, _) = builder.build();
        layout.break_all_lines(None);
        layout.lines().next().unwrap().metrics().line_height
    };

    // Arabic falls back to Noto Kufi Arabic, whose ascent/descent differ from Roboto's.
    assert_eq!(line_height("x", LineHeight::Absolute(30.)), 30.);
    assert_eq!(line_height("\u{0628}", LineHeight::Absolute(30.)), 30.);
    assert_eq!(line_height("x", LineHeight::FontSizeRelative(1.5)), 30.);
    assert_eq!(
        line_height("\u{0628}", LineHeight::FontSizeRelative(1.5)),
        30.
    );

    let normal = line_height("x", LineHeight::NORMAL);
    let normal_fallback = line_height("\u{0628}", LineHeight::NORMAL);
    assert!(
        normal_fallback > normal,
        "fallback glyphs should grow a `normal` line: {normal_fallback} vs {normal}"
    );
}

#[test]
fn breaking_after_reverting_to_a_default_state_does_not_panic() {
    // `BreakerState::default()` is public, so the line metrics it carries must be a valid
    // (empty) line rather than one that indexing the root aligned subtree would panic on.
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
    let mut builder = lcx.tree_builder(&mut fcx, 1., false, &root_style());
    builder.push_text("one two three four");
    let (mut layout, _) = builder.build();

    let mut breaker = layout.break_lines();
    breaker.break_next_with_length(4).unwrap();
    breaker.revert_to(crate::layout::BreakerState::default());
    while breaker.break_next_with_length(4).is_some() {}
    breaker.finish();

    assert!(layout.len() >= 4);
    for line in layout.lines() {
        assert!(line.metrics().line_height > 0.);
    }
}
