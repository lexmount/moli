// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test that the various builders produce the same results.

use std::{vec, vec::Vec};

use fontique::{FontStyle, FontWeight, FontWidth};
use peniko::color::palette;

use super::utils::{
    ColorBrush,
    asserts::assert_eq_layout_data,
    fonts::{FONT_FAMILY_LIST, create_font_context},
};
use crate::{
    BaseDirection, BreakReason, FontContext, FontFamily, FontFeatures, FontVariations, Layout,
    LayoutContext, LineBreak, LineHeight, OverflowWrap, RangedBuilder, StyleProperty,
    StyleRunBuilder, TextStyle, TextWrapMode, TreeBuilder, VerticalAlign, WhiteSpaceCollapse,
    WordBreak,
};

/// Set of options for [`build_layout_with_ranged`].
struct RangedOptions<'a> {
    scale: f32,
    quantize: bool,
    max_advance: Option<f32>,
    text: &'a str,
}

/// Set of options for [`build_layout_with_tree`].
struct TreeOptions<'a, 'b> {
    scale: f32,
    quantize: bool,
    max_advance: Option<f32>,
    root_style: &'a TextStyle<'b, 'b, ColorBrush>,
}

/// Generates a `Layout` with a ranged builder.
fn build_layout_with_ranged(
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<ColorBrush>,
    opts: &RangedOptions<'_>,
    with_builder: impl Fn(&mut RangedBuilder<'_, ColorBrush>),
) -> Layout<ColorBrush> {
    let mut rb = lcx.ranged_builder(fcx, opts.text, opts.scale, opts.quantize);
    with_builder(&mut rb);
    let mut layout = rb.build(opts.text);
    layout.break_all_lines(opts.max_advance);
    layout
}

/// Generates a `Layout` with a tree builder.
fn build_layout_with_tree(
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<ColorBrush>,
    opts: &TreeOptions<'_, '_>,
    with_builder: impl Fn(&mut TreeBuilder<'_, ColorBrush>),
) -> Layout<ColorBrush> {
    let mut tb = lcx.tree_builder(fcx, opts.scale, opts.quantize, opts.root_style);
    with_builder(&mut tb);
    let (mut layout, _) = tb.build();
    layout.break_all_lines(opts.max_advance);
    layout
}

/// Generates a `Layout` with a style run builder.
fn build_layout_with_style_runs(
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<ColorBrush>,
    opts: &RangedOptions<'_>,
    with_builder: impl Fn(&mut StyleRunBuilder<'_, ColorBrush>),
) -> Layout<ColorBrush> {
    let mut rb = lcx.style_run_builder(fcx, opts.text, opts.scale, opts.quantize);
    with_builder(&mut rb);
    let mut layout = rb.build(opts.text);
    layout.break_all_lines(opts.max_advance);
    layout
}

#[test]
fn builders_apply_base_direction() {
    let text = "123 / 456";
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();

    let mut ranged = lcx.ranged_builder(&mut fcx, text, 1.0, true);
    ranged.push_default(FontFamily::from(FONT_FAMILY_LIST));
    ranged.set_base_direction(BaseDirection::Rtl);
    let mut ranged_layout = ranged.build(text);
    ranged_layout.break_all_lines(None);
    assert!(ranged_layout.is_rtl());
    assert_eq!(
        ranged_layout
            .lines()
            .flat_map(|line| line.runs())
            .map(|run| run.text_range())
            .collect::<Vec<_>>(),
        [6..9, 3..6, 0..3]
    );

    let root_style = TextStyle {
        font_family: FontFamily::from(FONT_FAMILY_LIST),
        ..TextStyle::default()
    };
    let mut tree = lcx.tree_builder(&mut fcx, 1.0, true, &root_style);
    tree.set_base_direction(BaseDirection::Rtl);
    tree.push_text(text);
    let (mut tree_layout, _) = tree.build();
    tree_layout.break_all_lines(None);
    assert!(tree_layout.is_rtl());
    assert_eq_layout_data(
        &ranged_layout.data,
        &tree_layout.data,
        "tree base direction",
    );

    let mut style_runs = lcx.style_run_builder(&mut fcx, text, 1.0, true);
    style_runs.set_base_direction(BaseDirection::Rtl);
    let style = style_runs.push_style(root_style);
    style_runs.push_style_run(style, ..);
    let mut style_run_layout = style_runs.build(text);
    style_run_layout.break_all_lines(None);
    assert!(style_run_layout.is_rtl());
    assert_eq_layout_data(
        &ranged_layout.data,
        &style_run_layout.data,
        "style-run base direction",
    );
}

/// Computes layout in various ways to ensure they all produce the same result.
///
/// ```text
/// LayoutContext A - Ranged
/// LayoutContext A - Ranged for idempotency
///
/// LayoutContext B - Tree
/// LayoutContext B - Tree for idempotency
///
/// LayoutContext C - Ranged for dirt
/// LayoutContext C - Tree from dirty
///
/// LayoutContext D - Tree for dirt
/// LayoutContext D - Ranged from dirty
/// ```
fn assert_builders_produce_same_result<'b>(
    text: &str,
    scale: f32,
    quantize: bool,
    max_advance: Option<f32>,
    root_style: &TextStyle<'b, 'b, ColorBrush>,
    with_ranged_builder: impl Fn(&mut RangedBuilder<'_, ColorBrush>),
    with_tree_builder: impl Fn(&mut TreeBuilder<'_, ColorBrush>),
) {
    let mut fcx = create_font_context();

    let mut lcx_a: LayoutContext<ColorBrush> = LayoutContext::new();
    let mut lcx_b: LayoutContext<ColorBrush> = LayoutContext::new();
    let mut lcx_c: LayoutContext<ColorBrush> = LayoutContext::new();
    let mut lcx_d: LayoutContext<ColorBrush> = LayoutContext::new();

    let ropts = RangedOptions {
        scale,
        quantize,
        max_advance,
        text,
    };
    let topts = TreeOptions {
        scale,
        quantize,
        max_advance,
        root_style,
    };

    // Source of truth - ranged builder from a clean layout context
    let layout_truth = build_layout_with_ranged(&mut fcx, &mut lcx_a, &ropts, &with_ranged_builder);

    // Testing idempotence of ranged builder creation
    let layout = build_layout_with_ranged(&mut fcx, &mut lcx_a, &ropts, &with_ranged_builder);
    assert_eq_layout_data(&layout_truth.data, &layout.data, "lcx_a_rb_two");

    // Basic builder compatibility - tree builder from a clean layout context
    let layout = build_layout_with_tree(&mut fcx, &mut lcx_b, &topts, &with_tree_builder);
    assert_eq_layout_data(&layout_truth.data, &layout.data, "lcx_b_tb_one");

    // Testing idempotence of tree builder creation
    let layout = build_layout_with_tree(&mut fcx, &mut lcx_b, &topts, &with_tree_builder);
    assert_eq_layout_data(&layout_truth.data, &layout.data, "lcx_b_tb_two");

    // Priming a fresh layout context with ranged builder creation
    let _ = build_layout_with_ranged(&mut fcx, &mut lcx_c, &ropts, &with_ranged_builder);

    // Testing tree builder creation with a dirty layout context
    let layout = build_layout_with_tree(&mut fcx, &mut lcx_c, &topts, &with_tree_builder);
    assert_eq_layout_data(&layout_truth.data, &layout.data, "lcx_c_tb_one");

    // Priming a fresh layout context with tree builder creation
    let _ = build_layout_with_tree(&mut fcx, &mut lcx_d, &topts, &with_tree_builder);

    // Testing ranged builder creation with a dirty layout context
    let layout = build_layout_with_ranged(&mut fcx, &mut lcx_d, &ropts, &with_ranged_builder);
    assert_eq_layout_data(&layout_truth.data, &layout.data, "lcx_d_rb_one");
}

/// Returns a root style that uses non-default values.
///
/// The [`TreeBuilder`] version of [`set_root_style`].
fn create_root_style() -> TextStyle<'static, 'static, ColorBrush> {
    TextStyle {
        font_family: FontFamily::from(FONT_FAMILY_LIST),
        font_size: 20.,
        font_width: FontWidth::CONDENSED,
        font_style: FontStyle::Italic,
        font_weight: FontWeight::BOLD,
        font_variations: FontVariations::empty(), // TODO: Set a non-default value
        font_features: FontFeatures::empty(),     // TODO: Set a non-default value
        locale: Some("en-US".parse().unwrap()),
        brush: ColorBrush::new(palette::css::GREEN),
        has_underline: true,
        underline_offset: Some(2.),
        underline_size: Some(3.5),
        underline_brush: Some(ColorBrush::new(palette::css::CYAN)),
        has_strikethrough: true,
        strikethrough_offset: Some(1.3),
        strikethrough_size: Some(1.7),
        strikethrough_brush: Some(ColorBrush::new(palette::css::BEIGE)),
        line_height: LineHeight::Absolute(30.),
        // Parent-relative, so it compounds through nested tree spans but not through ranged
        // styles; keep it at the default so both builders agree.
        vertical_align: VerticalAlign::BASELINE,
        word_spacing: 2.,
        letter_spacing: 1.5,
        word_break: WordBreak::BreakAll,
        line_break: LineBreak::Loose,
        overflow_wrap: OverflowWrap::Anywhere,
        text_wrap_mode: TextWrapMode::Wrap,
        white_space_collapse: WhiteSpaceCollapse::PreserveBreaks,
    }
}

/// Sets a root style with non-default values.
///
/// The [`RangedBuilder`] version of [`create_root_style`].
fn set_root_style(rb: &mut RangedBuilder<'_, ColorBrush>) {
    rb.push_default(FontFamily::from(FONT_FAMILY_LIST));
    rb.push_default(StyleProperty::FontSize(20.));
    rb.push_default(StyleProperty::FontWidth(FontWidth::CONDENSED));
    rb.push_default(StyleProperty::FontStyle(FontStyle::Italic));
    rb.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
    rb.push_default(FontVariations::empty());
    rb.push_default(FontFeatures::empty());
    rb.push_default(StyleProperty::Locale(Some("en-US".parse().unwrap())));
    rb.push_default(StyleProperty::Brush(ColorBrush::new(palette::css::GREEN)));
    rb.push_default(StyleProperty::Underline(true));
    rb.push_default(StyleProperty::UnderlineOffset(Some(2.)));
    rb.push_default(StyleProperty::UnderlineSize(Some(3.5)));
    rb.push_default(StyleProperty::UnderlineBrush(Some(ColorBrush::new(
        palette::css::CYAN,
    ))));
    rb.push_default(StyleProperty::Strikethrough(true));
    rb.push_default(StyleProperty::StrikethroughOffset(Some(1.3)));
    rb.push_default(StyleProperty::StrikethroughSize(Some(1.7)));
    rb.push_default(StyleProperty::StrikethroughBrush(Some(ColorBrush::new(
        palette::css::BEIGE,
    ))));
    rb.push_default(LineHeight::Absolute(30.));
    rb.push_default(VerticalAlign::BASELINE);
    rb.push_default(StyleProperty::WordSpacing(2.));
    rb.push_default(StyleProperty::LetterSpacing(1.5));
    rb.push_default(StyleProperty::WordBreak(WordBreak::BreakAll));
    rb.push_default(StyleProperty::LineBreak(LineBreak::Loose));
    rb.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
    rb.push_default(StyleProperty::WhiteSpaceCollapse(
        WhiteSpaceCollapse::PreserveBreaks,
    ));
}

/// Test that all the builders have the same default behavior.
#[test]
fn builders_default() {
    let text = "Builders often wear hard hats for safety while working on construction sites.";
    let scale = 2.;
    let quantize = false;
    let max_advance = Some(50.);
    let root_style = TextStyle {
        font_family: FontFamily::from(FONT_FAMILY_LIST),
        ..TextStyle::default()
    };

    let with_ranged_builder = |rb: &mut RangedBuilder<'_, ColorBrush>| {
        rb.push_default(FontFamily::from(FONT_FAMILY_LIST));
    };
    let with_tree_builder = |tb: &mut TreeBuilder<'_, ColorBrush>| {
        tb.push_text(text);
    };

    assert_builders_produce_same_result(
        text,
        scale,
        quantize,
        max_advance,
        &root_style,
        with_ranged_builder,
        with_tree_builder,
    );
}

/// Test that `StyleRunBuilder` produces the same result as `RangedBuilder` when given equivalent
/// styles.
#[test]
fn builders_style_runs_match_ranged() {
    let text = "Builders often wear hard hats.";
    let scale = 2.;
    let quantize = false;
    let max_advance = Some(120.);

    let root_style: TextStyle<'static, 'static, ColorBrush> = TextStyle {
        font_family: FontFamily::from(FONT_FAMILY_LIST),
        ..TextStyle::default()
    };

    let split = text.len() / 2;
    let mut modified_style = root_style.clone();
    modified_style.font_size = 40.;
    modified_style.letter_spacing = 1.25;

    let mut fcx = create_font_context();
    let mut lcx_a: LayoutContext<ColorBrush> = LayoutContext::new();
    let mut lcx_b: LayoutContext<ColorBrush> = LayoutContext::new();

    let ropts = RangedOptions {
        scale,
        quantize,
        max_advance,
        text,
    };

    let ranged = build_layout_with_ranged(&mut fcx, &mut lcx_a, &ropts, |rb| {
        rb.push_default(FontFamily::from(FONT_FAMILY_LIST));
        rb.push(
            StyleProperty::FontSize(modified_style.font_size),
            split..text.len(),
        );
        rb.push(
            StyleProperty::LetterSpacing(modified_style.letter_spacing),
            split..text.len(),
        );
    });

    let runs = build_layout_with_style_runs(&mut fcx, &mut lcx_b, &ropts, |rb| {
        let family: FontFamily<'static> = root_style.font_family.clone().into_owned();
        let root_run: TextStyle<'static, 'static, ColorBrush> = TextStyle {
            font_family: family.clone(),
            ..root_style.clone()
        };

        let modified_run: TextStyle<'static, 'static, ColorBrush> = TextStyle {
            font_family: family,
            ..modified_style.clone()
        };

        let root_index = rb.push_style(root_run);
        let modified_index = rb.push_style(modified_run);
        rb.push_style_run(root_index, 0..split);
        rb.push_style_run(modified_index, split..text.len());
    });

    assert_eq_layout_data(&ranged.data, &runs.data, "style_runs_match_ranged");
}

/// Test that `StyleRunBuilder` handles a first run whose style table index is not zero.
#[test]
fn style_runs_first_run_can_use_nonzero_style_index() {
    let text = "Builders often wear hard hats.";
    let scale = 2.;
    let quantize = false;
    let max_advance = Some(50.);
    let root_style = create_root_style();
    let mut modified_style = root_style.clone();
    modified_style.font_size = 40.;

    let mut fcx = create_font_context();
    let mut lcx_a: LayoutContext<ColorBrush> = LayoutContext::new();
    let mut lcx_b: LayoutContext<ColorBrush> = LayoutContext::new();

    let ropts = RangedOptions {
        scale,
        quantize,
        max_advance,
        text,
    };

    let ranged = build_layout_with_ranged(&mut fcx, &mut lcx_a, &ropts, |rb| {
        set_root_style(rb);
        rb.push(
            StyleProperty::FontSize(modified_style.font_size),
            0..text.len(),
        );
    });

    let runs = build_layout_with_style_runs(&mut fcx, &mut lcx_b, &ropts, |rb| {
        let family: FontFamily<'static> = root_style.font_family.clone().into_owned();
        let root_run: TextStyle<'static, 'static, ColorBrush> = TextStyle {
            font_family: family.clone(),
            ..root_style.clone()
        };

        let modified_run: TextStyle<'static, 'static, ColorBrush> = TextStyle {
            font_family: family,
            ..modified_style.clone()
        };

        let _root_index = rb.push_style(root_run);
        let modified_index = rb.push_style(modified_run);
        rb.push_style_run(modified_index, 0..text.len());
    });

    assert_eq_layout_data(
        &ranged.data,
        &runs.data,
        "style_runs_first_run_can_use_nonzero_style_index",
    );
}

/// Test that all the builders behave the same when given the same root style.
#[test]
fn builders_root_only() {
    let text = "Builders often wear hard hats for safety while working on construction sites.";
    let scale = 2.;
    let quantize = false;
    let max_advance = Some(50.);
    let root_style = create_root_style();

    let with_ranged_builder = |rb: &mut RangedBuilder<'_, ColorBrush>| {
        set_root_style(rb);
    };
    let with_tree_builder = |tb: &mut TreeBuilder<'_, ColorBrush>| {
        tb.push_text(text);
    };

    assert_builders_produce_same_result(
        text,
        scale,
        quantize,
        max_advance,
        &root_style,
        with_ranged_builder,
        with_tree_builder,
    );
}

/// Test that an empty layout doesn't crash
#[test]
fn builders_empty() {
    let text = "";
    let scale = 1.;
    let quantize = false;
    let max_advance = Some(50.);
    let root_style = create_root_style();

    let with_ranged_builder = |rb: &mut RangedBuilder<'_, ColorBrush>| set_root_style(rb);
    let with_tree_builder = |_tb: &mut TreeBuilder<'_, ColorBrush>| {};

    assert_builders_produce_same_result(
        text,
        scale,
        quantize,
        max_advance,
        &root_style,
        with_ranged_builder,
        with_tree_builder,
    );
}

/// Test that all the builders behave the same with mixed styles.
#[test]
fn builders_mixed_styles() {
    let text = "Builders often wear hard hats for safety while working on construction sites.";
    let scale = 2.;
    let quantize = false;
    let max_advance = Some(50.);
    let root_style = create_root_style();

    let with_ranged_builder = |rb: &mut RangedBuilder<'_, ColorBrush>| {
        set_root_style(rb);

        // Make the first word bigger
        rb.push(StyleProperty::FontSize(68.), 0..8);
        // Push two modified styles for the same range
        rb.push(StyleProperty::LetterSpacing(4.), 12..17);
        rb.push(StyleProperty::WordSpacing(3.), 12..17);
        // Plus, change the line height for the last letter
        rb.push(StyleProperty::LineHeight(LineHeight::Absolute(40.)), 16..17);
    };
    let with_tree_builder = |tb: &mut TreeBuilder<'_, ColorBrush>| {
        // Make the first word bigger
        tb.push_style_modification_span(&[StyleProperty::FontSize(68.)]);
        tb.push_text(&text[..8]);
        tb.pop_style_span();

        tb.push_text(&text[8..12]);

        // Push two modified styles in batch
        tb.push_style_modification_span(&[
            StyleProperty::LetterSpacing(4.),
            StyleProperty::WordSpacing(3.),
        ]);
        tb.push_text(&text[12..16]);
        // Plus, change the line height for the last letter
        tb.push_style_modification_span(&[StyleProperty::LineHeight(LineHeight::Absolute(40.))]);
        tb.push_text(&text[16..17]);
        tb.pop_style_span();
        tb.pop_style_span();

        tb.push_text(&text[17..]);
    };

    assert_builders_produce_same_result(
        text,
        scale,
        quantize,
        max_advance,
        &root_style,
        with_ranged_builder,
        with_tree_builder,
    );
}

/// Check that reusing a [`Layout`] works, by overwriting a layout with
/// [`RangedBuilder::build_into`].
#[test]
fn ranged_builder_reuse_layout() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();

    static FIRST_TEXT: &str = "Some first text.";
    static SECOND_TEXT: &str = "Some second text.";
    const MAX_ADVANCE: f32 = 50.;

    let mut layout = Layout::new();

    let mut builder = lcx.ranged_builder(&mut fcx, FIRST_TEXT, 1., false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.build_into(&mut layout, FIRST_TEXT);
    layout.break_all_lines(Some(MAX_ADVANCE));

    let mut builder = lcx.ranged_builder(&mut fcx, SECOND_TEXT, 1., false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    builder.build_into(&mut layout, SECOND_TEXT);
    layout.break_all_lines(Some(MAX_ADVANCE));

    let mut builder = lcx.ranged_builder(&mut fcx, SECOND_TEXT, 1., false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    let mut expected = builder.build(SECOND_TEXT);
    expected.break_all_lines(Some(MAX_ADVANCE));

    assert_eq_layout_data(
        &expected.data,
        &layout.data,
        "Expected reused layout to be identical to fresh layout",
    );
}

#[test]
fn builders_crlf_counts_as_single_line_break() {
    let mut fcx = create_font_context();
    let mut line_count = |text: &str| -> usize {
        let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
        let ropts = RangedOptions {
            scale: 1.0,
            quantize: false,
            max_advance: None,
            text,
        };
        let layout = build_layout_with_ranged(&mut fcx, &mut lcx, &ropts, |rb| {
            set_root_style(rb);
        });
        layout.lines().len()
    };

    let crlf = line_count("a\r\nb");
    let lf = line_count("a\nb");
    let cr = line_count("a\rb");

    assert_eq!(crlf, 2, "CRLF should produce exactly two lines");
    assert_eq!(lf, 2, "LF should produce exactly two lines");
    assert_eq!(crlf, lf, "CRLF should match LF line count");
    assert_eq!(crlf, cr, "CRLF, LF, and CR should match line count");

    let double_crlf = line_count("a\r\n\r\nb");
    let double_lf = line_count("a\n\nb");

    assert_eq!(
        double_crlf, 3,
        "two CRLF breaks should produce one blank line, not extra blanks"
    );
    assert_eq!(
        double_crlf, double_lf,
        "two CRLF breaks should match two LF breaks"
    );

    let trailing_cr = line_count("a\r");
    let trailing_lf = line_count("a\n");

    assert_eq!(
        trailing_cr, trailing_lf,
        "trailing CR should match trailing LF line count"
    );
}

/// ICU4X's line segmenter emits a soft break opportunity at the end of a
/// complex-script (Thai, Khmer, Lao, ...) run even when the next character is a
/// mandatory break, which must not shadow the hard break.
/// See <https://github.com/linebender/parley/issues/768>.
#[test]
fn builders_newline_inside_complex_script_run_is_hard_break() {
    let mut fcx = create_font_context();
    let mut break_reasons = |text: &str| -> Vec<BreakReason> {
        let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();
        let ropts = RangedOptions {
            scale: 1.0,
            quantize: false,
            max_advance: None,
            text,
        };
        let layout = build_layout_with_ranged(&mut fcx, &mut lcx, &ropts, |rb| {
            set_root_style(rb);
            rb.push_default(StyleProperty::WordBreak(WordBreak::Normal));
        });
        layout.lines().map(|line| line.break_reason()).collect()
    };

    for text in [
        "กก\nกก",
        "ក្ម\nក្ម",
        "ກກ\nກກ",
        "กก\r\nกก",
        "กก\u{2028}กก",
        "aa\nbb",
    ] {
        assert_eq!(
            break_reasons(text),
            vec![BreakReason::Explicit, BreakReason::None],
            "{text:?} should produce exactly two lines with an explicit break",
        );
    }
}

#[test]
fn builders_empty_text_after_styled_layout_reuses_context() {
    let mut fcx = FontContext::default();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();

    let root = TextStyle::default();
    let mut builder = lcx.tree_builder(&mut fcx, 1.0, true, &root);
    builder.push_style_span(TextStyle {
        font_size: 20.,
        ..Default::default()
    });
    builder.push_text("a");
    builder.pop_style_span();
    let (layout, _) = builder.build();
    assert_eq!(layout.styles().len(), 2);

    // The empty layout is shaped with a substitute space, which must use the root style rather
    // than a stale style index left over from the previous layout.
    let builder = lcx.ranged_builder(&mut fcx, "", 1.0, true);
    let mut layout = builder.build("");
    layout.break_all_lines(None);
    assert_eq!(layout.lines().count(), 1);
}
