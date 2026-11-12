// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for font shaping style properties.

use std::borrow::Cow;

use crate::test_name;
use crate::util::{TestEnv, samples};
use parley::AlignmentOptions;
use parley::layout::Alignment;
use parley::setting::Tag;
use parley::style::{FontFeature, FontFeatures, FontVariation, FontVariations, StyleProperty};

// ============================================================================
// FontVariations Tests
// ============================================================================

#[test]
fn style_variations_weight_axis() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test weight axis (wght) with different values
    for weight in [100.0, 400.0, 700.0, 900.0] {
        let variations = FontVariations::List(Cow::Borrowed(&[FontVariation {
            tag: Tag::new(b"wght"),
            value: weight,
        }]));

        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Arimo")));
        builder.push_default(StyleProperty::FontVariations(variations));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name(&format!("wght_{weight}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_variations_width_axis() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test width axis (wdth) with different values
    for width in [75.0, 100.0, 125.0] {
        let variations = FontVariations::List(Cow::Borrowed(&[FontVariation {
            tag: Tag::new(b"wdth"),
            value: width,
        }]));

        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Roboto Flex")));
        builder.push_default(StyleProperty::FontVariations(variations));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        env.with_name(&format!("wdth_{width}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_variations_multiple_axes() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test multiple axes at once
    let variations = FontVariations::List(Cow::Borrowed(&[
        FontVariation {
            tag: Tag::new(b"wght"),
            value: 700.0,
        },
        FontVariation {
            tag: Tag::new(b"wdth"),
            value: 75.0,
        },
    ]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named("Roboto Flex")));
    builder.push_default(StyleProperty::FontVariations(variations));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

// ============================================================================
// FontFeatures Tests
// ============================================================================

#[test]
fn style_features_ligatures_on() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LIGATURES;

    // Enable ligatures explicitly
    let features = FontFeatures::List(Cow::Borrowed(&[FontFeature {
        tag: Tag::new(b"liga"),
        value: 1,
    }]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(features));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_ligatures_off() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LIGATURES;

    // Disable ligatures
    let features = FontFeatures::List(Cow::Borrowed(&[FontFeature {
        tag: Tag::new(b"liga"),
        value: 0,
    }]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(features));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_small_caps() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    // Enable small caps
    let features = FontFeatures::List(Cow::Borrowed(&[FontFeature {
        tag: Tag::new(b"smcp"),
        value: 1,
    }]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(features));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_ligatures_ltr_cluster_details() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "abfi";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(100.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    let line = layout.lines().next().unwrap();
    let item = line.items().next().unwrap();
    let glyph_run = match item {
        parley::PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
        parley::PositionedLayoutItem::InlineBox(_) => unreachable!(),
    };
    let mut last_advance = f32::MAX;
    glyph_run.run().clusters().enumerate().for_each(|(i, c)| {
        match i % 4 {
            // 'a' and 'b' are not ligatures.
            0 | 1 => assert!(!c.is_ligature_start() && !c.is_ligature_continuation()),
            // "f" is the ligature start whose cluster contains the "fi" glyph.
            2 => {
                assert!(c.is_ligature_start());
                assert_eq!(c.glyphs().count(), 1);
                assert_eq!(c.text_range().len(), 1);
                assert_eq!(c.glyphs().next().unwrap().id, 444);
                // The glyph for this ligature lives in the start cluster and should
                // contain the whole ligature's advance.
                assert_eq!(c.glyphs().next().unwrap().advance, c.advance() * 2.0);
            }
            // "i" is the ligature continuation whose cluster shares the advance with
            // the ligature start.
            3 => {
                assert!(c.is_ligature_continuation());
                // A continuation shares its advance with the previous cluster.
                assert_eq!(c.advance(), last_advance);
                assert_eq!(c.text_range().len(), 1);
                assert_eq!(c.glyphs().count(), 0);
            }
            _ => unreachable!(),
        }
        last_advance = c.advance();
    });
    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_ligatures_rtl_cluster_details() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "نصٍّ";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(100.0));
    layout.align(Alignment::Start, AlignmentOptions::default());

    let line = layout.lines().next().unwrap();
    let item = line.items().next().unwrap();
    let glyph_run = match item {
        parley::PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
        parley::PositionedLayoutItem::InlineBox(_) => unreachable!(),
    };
    let mut last_advance = f32::MAX;
    glyph_run.run().clusters().enumerate().for_each(|(i, c)| {
        match i % 4 {
            // "ن" and "ص" are not ligatures.
            0 | 1 => assert!(!c.is_ligature_start() && !c.is_ligature_continuation()),
            // "ّ" is the ligature continuation whose cluster shares the advance with
            // the ligature start.
            2 => {
                assert!(c.is_ligature_continuation());
                assert_eq!(c.text_range().len(), 2);
                assert_eq!(c.glyphs().count(), 0);
            }
            // The first visual character (i.e. the last logical character) is the ligature start.
            3 => {
                assert!(c.is_ligature_start());
                assert_eq!(c.glyphs().count(), 1);
                assert_eq!(c.text_range().len(), 2);
                // The advance should be shared with the previous cluster of the ligature.
                assert_eq!(c.advance(), last_advance);
                // This cluster should contain the one glyph of the ligature whose advance
                // is the sum of the advances of the component clusters.
                assert_eq!(c.glyphs().next().unwrap().advance, c.advance() * 2.0);
            }
            _ => unreachable!(),
        }
        last_advance = c.advance();
    });
    env.check_layout_snapshot(&layout);
}

// ============================================================================
// Locale Tests
// ============================================================================

#[test]
fn style_locale_arabic() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::ARABIC;

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Locale(Some("ar".parse().unwrap())));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_locale_mixed_bidi() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::MIXED_BIDI;

    // Test with Arabic locale
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Locale(Some("ar".parse().unwrap())));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    env.with_name("ar").check_layout_snapshot(&layout);

    // Test with English locale
    let mut builder_en = env.ranged_builder(text);
    builder_en.push_default(StyleProperty::Locale(Some("en".parse().unwrap())));
    let mut layout_en = builder_en.build(text);
    layout_en.break_all_lines(None);
    layout_en.align(Alignment::Start, AlignmentOptions::default());

    env.with_name("en").check_layout_snapshot(&layout_en);
}
