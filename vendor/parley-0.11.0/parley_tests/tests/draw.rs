// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for glyph and decoration drawing via `vello_cpu`.
//!
//! These tests focus on the interaction between transforms, hinting, and
//! decoration rendering (especially ink-skipping underlines).

use crate::test_name;
use crate::util::TestEnv;
use parley::style::FontFamily;
use parley::{Alignment, AlignmentOptions, Layout, StyleProperty};
use peniko::kurbo::Affine;

use super::util::ColorBrush;

/// Configuration for draw test variations.
struct DrawConfig {
    /// Global render scale (1.0 = 1x, 2.0 = 2x, etc.).
    scale: f64,
    /// Whether hinting is enabled.
    hint: bool,
    /// Optional per-glyph skew (for fake italics).
    skew: Option<f64>,
}

impl DrawConfig {
    fn suffix(&self) -> String {
        let scale_str = if self.scale == 1.0 {
            String::new()
        } else {
            format!("{}x_", self.scale as i32)
        };
        let hint_str = if self.hint { "hint" } else { "nohint" };
        let skew_str = if self.skew.is_some() { "_skew" } else { "" };
        format!("{scale_str}{hint_str}{skew_str}")
    }

    fn apply(&self, env: &mut TestEnv) {
        env.rendering_config().scale = self.scale;
        env.rendering_config().hint = self.hint;
        env.rendering_config().glyph_transform = self.skew.map(|s| Affine::skew(s, 0.0));
    }
}

const TEST_CONFIGS: &[DrawConfig] = &[
    DrawConfig {
        scale: 1.0,
        hint: false,
        skew: None,
    },
    DrawConfig {
        scale: 1.0,
        hint: true,
        skew: None,
    },
    DrawConfig {
        scale: 1.0,
        hint: false,
        skew: Some(0.2),
    },
    DrawConfig {
        scale: 1.0,
        hint: true,
        skew: Some(0.2),
    },
    DrawConfig {
        scale: 2.0,
        hint: false,
        skew: None,
    },
    DrawConfig {
        scale: 2.0,
        hint: true,
        skew: None,
    },
    DrawConfig {
        scale: 2.0,
        hint: false,
        skew: Some(0.2),
    },
    DrawConfig {
        scale: 2.0,
        hint: true,
        skew: Some(0.2),
    },
];

/// Run a test across all hint/skew configurations.
///
/// The `build_layout` closure receives a mutable `TestEnv` and should return the layout to test.
fn test_with_configs<F>(env: &mut TestEnv, mut build_layout: F)
where
    F: FnMut(&mut TestEnv) -> Layout<ColorBrush>,
{
    for config in TEST_CONFIGS {
        config.apply(env);
        let layout = build_layout(env);
        env.with_name(&config.suffix())
            .check_layout_snapshot(&layout);
    }
}

/// Test underline ink-skipping across different hinting, per-glyph transform, and scale configurations.
#[test]
fn draw_underline_descenders() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "happy puppy\njumping quickly";

    test_with_configs(&mut env, |env| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::Underline(true));
        builder.push_default(StyleProperty::FontSize(24.0));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    });
}

/// Test bitmap (CBTF) emoji rendering across different hinting, per-glyph transform, and scale configurations.
///
/// Disabled: snapshots were generated on macOS (ARM/NEON) but fail on
/// x86-64 (Ubuntu).
///
/// Proven:
/// - PNG roundtrip (`into_png` → `from_png`) is lossless — 0 differing
///   pixels when round-tripping the current render.
///
/// Hypothesis (not yet proven at the SIMD level):
/// - The vello CPU renderer's bicubic image filter (`FilteredImagePainter<2>`)
///   produces different edge-pixel values on ARM/NEON vs x86-64 (AVX2/SSE4.2)
///   due to floating-point differences in the SIMD paths.
///
/// Snapshots need to be regenerated per platform.
#[test]
#[ignore]
fn draw_bitmap_emoji() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "\u{2705}\u{1f440}\u{1f389}\u{1f920}";

    test_with_configs(&mut env, |env| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(
            "Noto Color Emoji CBTF",
        )));
        builder.push_default(StyleProperty::FontSize(24.0));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    });
}

/// Test COLR emoji rendering across different hinting, per-glyph transform, and scale configurations.
#[test]
fn draw_colr_emoji() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "\u{2705}\u{1f440}\u{1f389}\u{1f920}";

    test_with_configs(&mut env, |env| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(
            "Noto Color Emoji",
        )));
        builder.push_default(StyleProperty::FontSize(24.0));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    });
}

/// Test COLR emoji rendering across different hinting, per-glyph transform, and scale configurations.
///
/// The COLR emoji with presentation style(VS16).
///
/// The default color emoji is different for each system, so only macOS was added for testing.
#[cfg(all(target_os = "macos", feature = "system"))]
#[test]
fn draw_colr_emoji_with_presentation_style() {
    let mut env = TestEnv::new(test_name!(), None);

    let collection = &mut env.font_context().collection;
    collection.load_system_fonts();

    let text = "\u{270c}\u{fe0f}\u{2705}\u{270c}\u{fe0f}";

    test_with_configs(&mut env, |env| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontSize(24.0));
        // Following <fontique/src/backend/coretext.rs#L33>
        builder.push_default(StyleProperty::FontFamily(
            parley::GenericFamily::Emoji.into(),
        ));
        builder.push(
            StyleProperty::FontFamily(FontFamily::named("Noto Color Emoji")),
            0..9,
        );

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    });
}

/// Test COLR emoji rendering across different hinting, per-glyph transform, and scale configurations.
///
/// The COLR emoji with presentation style without setting the default font,
/// and should fallback to the system default color emoji font.
///
/// The default color emoji is different for each system, so only macOS was added for testing.
#[cfg(all(target_os = "macos", feature = "system"))]
#[test]
fn draw_colr_emoji_with_presentation_style_without_setting_default_font() {
    let mut env = TestEnv::new(test_name!(), None);

    let collection = &mut env.font_context().collection;
    collection.load_system_fonts();

    let text = "\u{270c}\u{fe0f}\u{2705}\u{270c}\u{fe0f}";

    test_with_configs(&mut env, |env| {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontSize(24.0));
        builder.push(
            StyleProperty::FontFamily(FontFamily::named("Noto Color Emoji")),
            0..9,
        );

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    });
}
