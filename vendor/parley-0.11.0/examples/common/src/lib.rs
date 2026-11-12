// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared layout config, building, and output path for vello_cpu_render and vello_hybrid_render.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use parley::VerticalAlign;
use parley::fontique::Blob;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, GenericFamily, InlineBox,
    InlineBoxKind, Layout, LayoutContext, LineHeight, StyleProperty,
};
use peniko::Color;

/// Latin-only text for the simple layout — exercises basic glyph caching without emoji or bidi.
pub const SIMPLE_TEXT: &str = "Some text here. Let's make it a bit longer so that line wrapping kicks in easily. This demonstrates basic glyph caching with plain Latin text and common punctuation???";

/// Rich text mixing bitmap emoji, COLR emoji, Arabic (bidi), underline, and strikethrough.
pub const RICH_TEXT: &str = "Some text here. Let's make it a bit longer so that line wrapping kicks in. Bitmap emoji 😊 and COLR emoji 🎉.\nAnd also some اللغة العربية arabic text.\nThis is underlining pq and strikethrough text.";

/// Minimal brush type carrying only a solid color, used as the `Brush` generic
/// parameter for Parley layouts in these examples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorBrush {
    pub color: Color,
}

impl Default for ColorBrush {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
        }
    }
}

/// Knobs shared across both example renderers.
#[derive(Clone, Debug)]
pub struct ExampleConfig {
    pub text: String,
    pub display_scale: f32,
    /// Whether glyph positions are quantized (snapped) to integer pixels.
    pub quantize: bool,
    /// Maximum line advance before wrapping; `None` disables wrapping.
    pub max_advance: Option<f32>,
    pub foreground_color: Color,
    pub background_color: Color,
    /// Pixel padding added on each side of the rendered frame.
    pub padding: u32,
    /// Whether to apply hinting when rasterizing glyphs.
    pub hint: bool,
    /// Whether to use the atlas-based glyph cache path.
    pub use_atlas_cache: bool,
}

impl Default for ExampleConfig {
    fn default() -> Self {
        Self {
            text: "".to_owned(),
            display_scale: 1.0,
            quantize: true,
            max_advance: Some(200.0),
            foreground_color: Color::BLACK,
            background_color: Color::WHITE,
            padding: 20,
            hint: true,
            // Vello Hybrid doesn't support uploading bitmap pixmaps directly yet
            // (see vello#1459), so disabling the atlas cache will panic on bitmap emojis.
            use_atlas_cache: true,
        }
    }
}

/// A prepared layout with its dimensions and config.
pub type PreparedLayout = (Layout<ColorBrush>, u16, u16, ExampleConfig);

/// Build both layouts from a **shared** `FontContext` so that the same font
/// gets the same `font_id` regardless of which layout it appears in.
/// Returns `(simple, rich)` prepared layouts.
pub fn prepare_layouts() -> (PreparedLayout, PreparedLayout) {
    let simple_config = ExampleConfig {
        text: SIMPLE_TEXT.to_owned(),
        ..ExampleConfig::default()
    };
    let rich_config = ExampleConfig {
        text: RICH_TEXT.to_owned(),
        ..ExampleConfig::default()
    };

    let mut font_cx = FontContext::new();
    let mut layout_cx = LayoutContext::new();

    let (simple_layout, sw, sh) = build_simple_layout(&mut font_cx, &mut layout_cx, &simple_config);

    let (rich_layout, rw, rh) = build_rich_layout(
        &mut font_cx,
        &mut layout_cx,
        &rich_config,
        &default_colr_font_path(),
    );

    (
        (simple_layout, sw, sh, simple_config),
        (rich_layout, rw, rh, rich_config),
    )
}

/// Default path to the COLR emoji font (relative to workspace).
pub fn default_colr_font_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../parley_dev/assets/fonts/noto_color_emoji/NotoColorEmoji-Subset.ttf"
    ))
}

/// Build a simple layout with plain Latin text (no emoji, no decorations, no inline boxes).
pub fn build_simple_layout(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ColorBrush>,
    config: &ExampleConfig,
) -> (Layout<ColorBrush>, u16, u16) {
    let mut builder =
        layout_cx.ranged_builder(font_cx, &config.text, config.display_scale, config.quantize);

    let foreground_brush = ColorBrush {
        color: config.foreground_color,
    };
    builder.push_default(StyleProperty::Brush(foreground_brush));
    builder.push_default(GenericFamily::SystemUi);
    builder.push_default(LineHeight::FontSizeRelative(1.3));
    builder.push_default(StyleProperty::FontSize(16.0));

    let bold = FontWeight::new(600.0);
    builder.push(StyleProperty::FontWeight(bold), 0..4);

    let purple_brush = ColorBrush {
        color: Color::from_rgb8(128, 0, 128),
    };
    let here_range = {
        let (start, matched) = config.text.match_indices("here").next().unwrap();
        start..start + matched.len()
    };
    builder.push(StyleProperty::Brush(purple_brush), here_range);

    let mut layout = builder.build(&config.text);
    layout.break_all_lines(config.max_advance);
    layout.align(Alignment::Start, AlignmentOptions::default());

    let width = layout.width().ceil() as u16;
    let height = layout.height().ceil() as u16;
    let padded_width = width + (config.padding * 2) as u16;
    let padded_height = height + (config.padding * 2) as u16;

    (layout, padded_width, padded_height)
}

/// Load COLR emoji font from `colr_font_path` into `font_cx`, then build the
/// rich layout with emoji, Arabic text, underline/strikethrough, and an inline box.
pub fn build_rich_layout(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ColorBrush>,
    config: &ExampleConfig,
    colr_font_path: &Path,
) -> (Layout<ColorBrush>, u16, u16) {
    let colr_font_data = std::fs::read(colr_font_path).expect("Failed to load COLR font");
    font_cx
        .collection
        .register_fonts(Blob::new(Arc::new(colr_font_data)), None);

    let (underline_range, strikethrough_range, party_emoji_range) = style_ranges(&config.text);

    let mut builder =
        layout_cx.ranged_builder(font_cx, &config.text, config.display_scale, config.quantize);

    let foreground_brush = ColorBrush {
        color: config.foreground_color,
    };
    builder.push_default(StyleProperty::Brush(foreground_brush));
    builder.push_default(GenericFamily::SystemUi);
    builder.push_default(LineHeight::FontSizeRelative(1.3));
    builder.push_default(StyleProperty::FontSize(16.0));

    let bold = FontWeight::new(600.0);
    builder.push(StyleProperty::FontWeight(bold), 0..4);

    let purple_brush = ColorBrush {
        color: Color::from_rgb8(128, 0, 128),
    };
    let here_range = {
        let (start, matched) = config.text.match_indices("here").next().unwrap();
        start..start + matched.len()
    };
    builder.push(StyleProperty::Brush(purple_brush), here_range);

    builder.push(StyleProperty::Underline(true), underline_range);
    builder.push(StyleProperty::Strikethrough(true), strikethrough_range);
    builder.push(FontFamily::named("Noto Color Emoji"), party_emoji_range);
    builder.push_inline_box(InlineBox {
        id: 0,
        kind: InlineBoxKind::InFlow,
        index: 40,
        width: 50.0,
        height: 50.0,
        baseline: None,
        vertical_align: VerticalAlign::BASELINE,
    });

    let mut layout = builder.build(&config.text);
    layout.break_all_lines(config.max_advance);
    layout.align(Alignment::Start, AlignmentOptions::default());

    let width = layout.width().ceil() as u16;
    let height = layout.height().ceil() as u16;
    let padded_width = width + (config.padding * 2) as u16;
    let padded_height = height + (config.padding * 2) as u16;

    (layout, padded_width, padded_height)
}

/// Locate byte ranges for underline, strikethrough, and party-emoji substrings.
pub fn style_ranges(text: &str) -> (Range<usize>, Range<usize>, Range<usize>) {
    let underline_range = {
        let (start, matched) = text.match_indices("underlining pq").next().unwrap();
        start..start + matched.len()
    };
    let strikethrough_range = {
        let (start, matched) = text.match_indices("strikethrough").next().unwrap();
        start..start + matched.len()
    };
    let party_emoji_range = {
        let (start, matched) = text.match_indices("🎉").next().unwrap();
        start..start + matched.len()
    };
    (underline_range, strikethrough_range, party_emoji_range)
}

/// Selects which pre-built layout a frame should render.
#[derive(Clone, Copy, Debug)]
pub enum FrameKind {
    /// Latin-only, no decorations, no emoji.
    Simple,
    /// Mixed scripts, emoji, underline, strikethrough, inline box.
    Rich,
}

/// A single frame in the example render sequence.
#[derive(Clone, Debug)]
pub struct FrameSpec {
    /// Human-readable description printed before the frame runs.
    pub label: &'static str,
    pub kind: FrameKind,
}

/// The default frame sequence used by both examples. Designed to exercise
/// cold-start, warm-cache, partial-overlap, eviction, and repopulation paths.
pub fn frame_sequence() -> Vec<FrameSpec> {
    vec![
        FrameSpec {
            label: "Phase 1: Cold start — simple layout (populate cache)",
            kind: FrameKind::Simple,
        },
        FrameSpec {
            label: "Phase 2: Warm cache — simple layout (all hits)",
            kind: FrameKind::Simple,
        },
        FrameSpec {
            label: "Phase 3a: Rich layout (partial overlap, round 1)",
            kind: FrameKind::Rich,
        },
        FrameSpec {
            label: "Phase 3b: Rich layout (partial overlap, round 2 — evicts simple-only)",
            kind: FrameKind::Rich,
        },
        FrameSpec {
            label: "Phase 4: Simple layout after eviction (shared hit, simple-only repopulate)",
            kind: FrameKind::Simple,
        },
        FrameSpec {
            label: "Phase 5: Rich layout (final render)",
            kind: FrameKind::Rich,
        },
    ]
}

/// Resolve the `examples/_output` directory from a crate's `CARGO_MANIFEST_DIR`.
///
/// Callers should pass `env!("CARGO_MANIFEST_DIR")`.
pub fn output_dir(manifest_dir: &str) -> PathBuf {
    let mut path = PathBuf::from(manifest_dir);
    path.pop();
    path.push("_output");
    let _ = std::fs::create_dir(&path);
    path
}

/// Accumulates per-label timing data across multiple frames.
///
/// Supports both single-shot timings and accumulation mode (summing many
/// start/end intervals into one entry per frame).
pub struct FrameStats {
    /// Per-label timing data (label -> vec of durations)
    timings: HashMap<String, Vec<Duration>>,
    /// Active timers (label -> start instant)
    active_timers: HashMap<String, Instant>,
    /// Accumulators for labels in accumulation mode (label -> accumulated duration)
    accumulators: HashMap<String, Duration>,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameStats {
    /// Create an empty stats collector with no timings recorded.
    pub fn new() -> Self {
        Self {
            timings: HashMap::new(),
            active_timers: HashMap::new(),
            accumulators: HashMap::new(),
        }
    }

    /// Start timing for a labeled section. Call `end()` with the same label to record the duration.
    pub fn start(&mut self, label: impl Into<String>) {
        let label = label.into();
        self.active_timers.insert(label, Instant::now());
    }

    /// End timing for a labeled section and record the duration.
    /// If the label is in accumulation mode, adds to the accumulator instead.
    pub fn end(&mut self, label: impl Into<String>) {
        let label = label.into();
        if let Some(start) = self.active_timers.remove(&label) {
            let duration = start.elapsed();

            if let Some(accumulator) = self.accumulators.get_mut(&label) {
                *accumulator += duration;
            } else {
                self.timings.entry(label).or_default().push(duration);
            }
        }
    }

    /// Enable accumulation mode for a label. All subsequent start()/end() calls
    /// with this label will sum up until finish_accumulating() is called.
    pub fn start_accumulating(&mut self, label: impl Into<String>) {
        self.accumulators.insert(label.into(), Duration::ZERO);
    }

    /// Disable accumulation mode and record the total accumulated duration.
    pub fn finish_accumulating(&mut self, label: impl Into<String>) {
        let label = label.into();
        if let Some(total) = self.accumulators.remove(&label) {
            self.timings.entry(label).or_default().push(total);
        }
    }

    /// Print all collected timings, split into one-time and per-frame groups.
    pub fn print_summary(&self) {
        let mut one_time = Vec::new();
        let mut per_frame = Vec::new();

        for (label, times) in &self.timings {
            if times.len() == 1 {
                one_time.push(label.as_str());
            } else {
                per_frame.push(label.as_str());
            }
        }

        one_time.sort();
        per_frame.sort();

        if !one_time.is_empty() {
            println!("\nOne-time operations:");
            for label in one_time {
                if let Some(times) = self.timings.get(label) {
                    self.print_timing_entry(label, times);
                }
            }
        }

        if !per_frame.is_empty() {
            println!("\nPer-frame statistics:");
            for label in per_frame {
                if let Some(times) = self.timings.get(label) {
                    self.print_timing_entry(label, times);
                }
            }
        }
    }

    /// Print a single timing entry: just the value if n=1, otherwise min/max/median/avg/sum.
    fn print_timing_entry(&self, name: &str, times: &[Duration]) {
        if times.is_empty() {
            return;
        }

        let mut sorted = times.to_vec();
        sorted.sort();

        let min = sorted[0];
        let max = sorted[sorted.len() - 1];
        let median = sorted[sorted.len() / 2];
        let sum: Duration = times.iter().sum();
        let avg = sum / times.len() as u32;

        let count = times.len();
        if count == 1 {
            println!("  {}: {:?}", name, sum);
        } else {
            println!("  {} (n={}):", name, count);
            println!("    median: {:?}", median);
            println!("    avg:    {:?}", avg);
            println!("    min:    {:?}", min);
            println!("    max:    {:?}", max);
            println!("    sum:    {:?}", sum);
        }
    }
}
