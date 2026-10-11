// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! # Benchmarks
//!
//! This module provides benchmarks for text layout and rendering.

use crate::{ColorBrush, FONT_FAMILY_LIST, Sample, get_samples, with_contexts};
use parley::{
    Alignment, AlignmentOptions, BaseDirection, FontFamily, FontStyle, FontWeight, InlineBox,
    InlineBoxBidi, InlineBoxKind, Layout, PositionedLayoutItem, StyleProperty, VerticalAlign,
};
use std::hint::black_box;
use std::ops::Range;
use tango_bench::{Benchmark, benchmark_fn};

/// Benchmark obtaining every cluster's position on increasingly long, unwrapped lines.
pub fn cluster_positions() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for (label, sample) in [("LTR", "A"), ("Mixed RTL", "hello العربية ")] {
        for count in [128_usize, 512, 2048] {
            let text = sample
                .repeat(count.div_ceil(sample.chars().count()))
                .chars()
                .take(count)
                .collect::<String>();
            for positioned in [false, true] {
                let text = text.clone();
                let method = if positioned {
                    "iterator"
                } else {
                    "visual_offset"
                };
                benchmarks.push(benchmark_fn(
                    format!("Cluster Positions - {label}, {count} clusters, {method}"),
                    move |b| {
                        let mut layout = build_layout(&text, []);
                        layout.break_all_lines(None);
                        b.iter(move || {
                            for line in layout.lines() {
                                if positioned {
                                    for (cluster, offset) in line.positioned_visual_clusters() {
                                        black_box((cluster.path(), offset));
                                    }
                                } else {
                                    for run in line.runs() {
                                        for cluster in run.visual_clusters() {
                                            black_box((
                                                cluster.path(),
                                                cluster.visual_offset().unwrap(),
                                            ));
                                        }
                                    }
                                }
                            }
                        })
                    },
                ));
            }
        }
    }
    benchmarks
}

/// Benchmark building LTR and mixed-direction text with zero, one or many inline objects.
///
/// The many-object cases also measure line breaking and alignment without rebuilding.
pub fn inline_boxes() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for (label, sample) in [("LTR", "hello world "), ("Mixed RTL", "hello אב 123 ")] {
        let text = sample.repeat(100);
        let boundaries = text
            .char_indices()
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>();
        for count in [0, 1, 100] {
            let indices = (1..=count)
                .map(|i| boundaries[i * boundaries.len() / (count + 1)])
                .collect::<Vec<_>>();
            let build_text = text.clone();
            let build_indices = indices.clone();
            benchmarks.push(benchmark_fn(
                format!("Inline Boxes - {label}, {count} objects"),
                move |b| {
                    let text = build_text.clone();
                    let indices = build_indices.clone();
                    b.iter(move || black_box(build_inline_boxes(&text, &indices)))
                },
            ));
            if count == 100 {
                let break_text = text.clone();
                let break_indices = indices.clone();
                benchmarks.push(benchmark_fn(
                    format!("Inline Boxes Break - {label}, {count} objects"),
                    move |b| {
                        let mut layout = build_inline_boxes(&break_text, &break_indices);
                        b.iter(move || {
                            layout.break_all_lines(Some(200.0));
                            black_box((layout.len(), layout.width(), layout.height()));
                        })
                    },
                ));

                let align_text = text.clone();
                benchmarks.push(benchmark_fn(
                    format!("Inline Boxes Align - {label}, {count} objects"),
                    move |b| {
                        let mut layout = build_inline_boxes(&align_text, &indices);
                        layout.break_all_lines(Some(200.0));
                        b.iter(move || {
                            layout.align(Alignment::Justify, AlignmentOptions::default());
                            black_box(&layout);
                        })
                    },
                ));
            }
        }
    }
    benchmarks
}

fn build_inline_boxes(text: &str, indices: &[usize]) -> Layout<ColorBrush> {
    with_contexts(|font_cx, layout_cx| {
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, false);
        builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
        builder.set_base_direction(BaseDirection::Ltr);
        for (ordinal, &index) in indices.iter().enumerate() {
            builder.push_inline_box(InlineBox {
                id: ordinal as u64,
                index,
                kind: InlineBoxKind::InFlow,
                width: 10.0,
                height: 10.0,
                baseline: None,
                vertical_align: VerticalAlign::BASELINE,
            });
        }
        builder.build(text)
    })
}

/// Benchmark 100 transparent marker pairs, with zero or 100 neutral objects.
///
/// Every span uses the same style. Each pair contributes an `InheritNext` marker
/// at its start and an `InheritPrevious` marker at its end.
pub fn inline_host() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for (label, sample, direction) in [
        ("LTR", "hello world ", BaseDirection::Ltr),
        ("Mixed, LTR base", "hello אב 123 ", BaseDirection::Ltr),
        ("Mixed, RTL base", "hello אב 123 ", BaseDirection::Rtl),
    ] {
        let text = sample.repeat(100);
        let boundaries = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let chars_per_span = sample.chars().count();
        for object_count in [0, 100] {
            let text = text.clone();
            let boundaries = boundaries.clone();
            benchmarks.push(benchmark_fn(
                format!("Inline Host - {label}, 100 spans, {object_count} objects"),
                move |b| {
                    let text = text.clone();
                    let boundaries = boundaries.clone();
                    b.iter(move || {
                        with_contexts(|fonts, context| {
                            let mut builder = context.ranged_builder(fonts, &text, 1.0, false);
                            builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
                            builder.set_base_direction(direction);
                            for span in 0..100 {
                                let start = boundaries[span * chars_per_span];
                                let middle = boundaries[span * chars_per_span + chars_per_span / 2];
                                let end = boundaries[(span + 1) * chars_per_span];
                                for (slot, index, width, participation) in [
                                    (0, start, 2.0, InlineBoxBidi::InheritNext),
                                    (1, middle, 10.0, InlineBoxBidi::Neutral),
                                    (2, end, 2.0, InlineBoxBidi::InheritPrevious),
                                ] {
                                    if slot == 1 && object_count == 0 {
                                        continue;
                                    }
                                    builder.push_inline_box_with_bidi(
                                        InlineBox {
                                            id: (span * 3 + slot) as u64,
                                            index,
                                            kind: InlineBoxKind::InFlow,
                                            width,
                                            height: if slot == 1 { 10.0 } else { 0.0 },
                                            baseline: None,
                                            vertical_align: VerticalAlign::BASELINE,
                                        },
                                        participation,
                                    );
                                }
                            }
                            black_box(builder.build(&text))
                        })
                    })
                },
            ));
        }
    }
    benchmarks
}

/// Benchmark for default style.
pub fn defaults() -> Vec<Benchmark> {
    const DISPLAY_SCALE: f32 = 1.0;
    const QUANTIZE: bool = true;
    const MAX_ADVANCE: f32 = 200.0 * DISPLAY_SCALE;

    let samples = get_samples();

    samples
        .iter()
        .map(|sample| {
            benchmark_fn(
                format!("Default Style - {} {}", sample.name, sample.modification),
                |b| {
                    b.iter(|| {
                        let text = &sample.text;
                        with_contexts(|font_cx, layout_cx| {
                            let mut builder =
                                layout_cx.ranged_builder(font_cx, text, DISPLAY_SCALE, QUANTIZE);
                            builder.push_default(FontFamily::from(FONT_FAMILY_LIST));

                            let mut layout: Layout<ColorBrush> = builder.build(text);
                            layout.break_all_lines(Some(MAX_ADVANCE));
                            layout.align(Alignment::Start, AlignmentOptions::default());

                            black_box(layout);
                        });
                    })
                },
            )
        })
        .collect()
}

/// Benchmark for nonzero word and letter spacing.
pub fn spacing() -> Vec<Benchmark> {
    const DISPLAY_SCALE: f32 = 1.0;
    const QUANTIZE: bool = true;
    const MAX_ADVANCE: f32 = 200.0 * DISPLAY_SCALE;
    const WORD_SPACING: f32 = 2.0;
    const LETTER_SPACING: f32 = 1.0;

    let samples = get_samples();

    samples
        .iter()
        .map(|sample| {
            benchmark_fn(
                format!(
                    "Word + Letter Spacing - {} {}",
                    sample.name, sample.modification
                ),
                |b| {
                    b.iter(|| {
                        let text = &sample.text;
                        with_contexts(|font_cx, layout_cx| {
                            let mut builder =
                                layout_cx.ranged_builder(font_cx, text, DISPLAY_SCALE, QUANTIZE);
                            builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
                            builder.push_default(StyleProperty::WordSpacing(WORD_SPACING));
                            builder.push_default(StyleProperty::LetterSpacing(LETTER_SPACING));

                            let mut layout: Layout<ColorBrush> = builder.build(text);
                            layout.break_all_lines(Some(MAX_ADVANCE));
                            layout.align(Alignment::Start, AlignmentOptions::default());

                            black_box(layout);
                        });
                    })
                },
            )
        })
        .collect()
}

/// Benchmark repeatedly justifying an already line-broken layout.
pub fn repeated_justification() -> [Benchmark; 1] {
    const DISPLAY_SCALE: f32 = 1.0;
    const QUANTIZE: bool = true;
    const MAX_ADVANCE: f32 = 200.0 * DISPLAY_SCALE;

    let sample = get_samples()
        .iter()
        .find(|sample| sample.name == "latin" && sample.modification == "8000 characters")
        .expect("the Latin 8000-character benchmark sample should exist");

    [benchmark_fn(
        format!(
            "Repeated Justification - {} {}",
            sample.name, sample.modification
        ),
        move |b| {
            let text = &sample.text;
            let mut layout = with_contexts(|font_cx, layout_cx| {
                let mut builder = layout_cx.ranged_builder(font_cx, text, DISPLAY_SCALE, QUANTIZE);
                builder.push_default(FontFamily::from(FONT_FAMILY_LIST));

                let mut layout: Layout<ColorBrush> = builder.build(text);
                layout.break_all_lines(Some(MAX_ADVANCE));
                layout
            });

            b.iter(move || {
                layout.align(Alignment::Justify, AlignmentOptions::default());

                // Pass to black box so the optimizer cannot optimize alignment away.
                black_box(
                    layout
                        .lines()
                        .flat_map(|line| line.runs())
                        .map(|run| run.advance())
                        .sum::<f32>(),
                );
                layout.align(Alignment::Start, AlignmentOptions::default());

                // Pass to black box so the optimizer cannot optimize alignment away.
                black_box(
                    layout
                        .lines()
                        .flat_map(|line| line.runs())
                        .map(|run| run.advance())
                        .sum::<f32>(),
                )
            })
        },
    )]
}

/// Benchmark breaking a layout into lines.
///
/// This is part of the work that'd be performed as, e.g., a window is being resized.
pub fn line_breaking() -> Vec<Benchmark> {
    const NARROW_MAX_ADVANCE: f32 = 50.;
    const MAX_ADVANCE: f32 = 200.;
    const WORD_SPACING: f32 = 2.;
    const LETTER_SPACING: f32 = 1.;

    // The maximum advances to break at, with the name each is given in the benchmark name.
    const MAX_ADVANCES: [(&str, Option<f32>); 3] = [
        // Narrow lines break often and will have some emergency-breaks within words that don't fit.
        ("narrow", Some(NARROW_MAX_ADVANCE)),
        ("wider", Some(MAX_ADVANCE)),
        // Unwrapped lines only break at mandatory line breaks.
        ("unwrapped", None),
    ];

    fn break_lines(layout: &mut Layout<ColorBrush>, max_advance: Option<f32>) {
        layout.break_all_lines(max_advance);
        black_box((layout.len(), layout.width(), layout.height()));
    }

    // The samples are long enough to break into a meaningful number of lines.
    let samples = || {
        get_samples()
            .iter()
            .filter(|sample| sample.modification == "8000 characters")
    };

    let mut benchmarks = Vec::new();

    for sample in samples() {
        for (max_advance_name, max_advance) in MAX_ADVANCES {
            benchmarks.push(benchmark_fn(
                format!(
                    "Line Breaking - {} {}, {max_advance_name}",
                    sample.name, sample.modification
                ),
                move |b| {
                    let mut layout = build_layout(&sample.text, []);
                    b.iter(move || break_lines(&mut layout, max_advance))
                },
            ));
        }
    }

    // Also bench with word and letter spacing applied, as there are some zero-spacing fast paths.
    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!(
                "Line Breaking - {} {}, wrapped + spacing",
                sample.name, sample.modification
            ),
            |b| {
                let text_range = 0..sample.text.len();
                let mut layout = build_layout(
                    &sample.text,
                    [
                        (StyleProperty::WordSpacing(WORD_SPACING), text_range.clone()),
                        (StyleProperty::LetterSpacing(LETTER_SPACING), text_range),
                    ],
                );
                b.iter(move || break_lines(&mut layout, Some(MAX_ADVANCE)))
            },
        ));
    }

    benchmarks
}

/// Get the byte ranges of each consecutive chunk of `char_len` characters.
///
/// The last chunk holds the characters remaining.
fn chunks(text: &str, char_len: usize) -> impl Iterator<Item = Range<usize>> {
    let mut starts = text
        .char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .step_by(char_len)
        .peekable();
    std::iter::from_fn(move || {
        let start = starts.next()?;
        let end = starts.peek().copied().unwrap_or(text.len());
        Some(start..end)
    })
}

/// Create style spans changing style every few characters.
fn styled_spans(
    text: &str,
) -> impl Iterator<Item = (StyleProperty<'static, ColorBrush>, Range<usize>)> {
    let style_interval = (text.len() / 5).min(10);
    chunks(text, style_interval)
        .enumerate()
        .filter_map(|(style_idx, range)| {
            let style = match style_idx % 5 {
                0 => StyleProperty::FontStyle(FontStyle::Italic),
                1 => StyleProperty::FontWeight(FontWeight::BOLD),
                2 => StyleProperty::Underline(true),
                3 => StyleProperty::Strikethrough(true),
                4 => return None, // Default style
                _ => unreachable!(),
            };
            Some((style, range))
        })
}

/// Apply `style` to every other chunk of `chunk_char_len` characters of `text`.
fn alternating_spans<'s>(
    text: &str,
    chunk_char_len: usize,
    style: &StyleProperty<'s, ColorBrush>,
) -> impl Iterator<Item = (StyleProperty<'s, ColorBrush>, Range<usize>)> {
    chunks(text, chunk_char_len)
        .step_by(2)
        .map(move |range| (style.clone(), range))
}

/// Build a styled layout for `text`, changing style every few characters.
fn build_styled_layout(text: &str) -> Layout<ColorBrush> {
    const DISPLAY_SCALE: f32 = 1.0;
    const MAX_ADVANCE: f32 = 200.0 * DISPLAY_SCALE;

    let mut layout = build_layout(text, styled_spans(text));
    layout.break_all_lines(Some(MAX_ADVANCE));
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// Benchmark for styled text.
pub fn styled() -> Vec<Benchmark> {
    let samples = get_samples();

    samples
        .iter()
        .map(|sample| {
            benchmark_fn(
                format!("Styled - {} {}", sample.name, sample.modification),
                |b| {
                    b.iter(|| {
                        black_box(build_styled_layout(&sample.text));
                    })
                },
            )
        })
        .collect()
}

/// Benchmark for iterating the positioned glyph runs and glyphs of a styled layout, as a renderer
/// would.
///
/// The cases fall into three groups:
///
/// - 8000 characters of Latin with a single style, without line wrapping, giving one shaped run per
///   paragraph.
/// - 8000 characters of Latin with a style alternating every few characters, without line wrapping.
///   Glyph runs break wherever the style changes; some properties, such as bold, also split the
///   shaped run. This benches both cases.
/// - Mixed styling of each script with line wrapping. This covers script, bidi and font fallback
///   handling, and, being wrapped, the line-scoped runs a renderer normally walks.
pub fn iterate_glyph_runs() -> Vec<Benchmark> {
    let latin = get_samples()
        .iter()
        .find(|sample| sample.name == "latin" && sample.modification == "8000 characters")
        .expect("the Latin 8000-character benchmark sample should exist");

    let mut benchmarks = vec![benchmark_fn(
        format!(
            "Glyph Runs - {} {}, uniform",
            latin.name, latin.modification
        ),
        |b| {
            let layout = build_unwrapped_layout(&latin.text, []);
            b.iter(move || black_box(walk_items(&layout)))
        },
    )];

    // Underline splits the glyph runs only; bold also splits the shaped runs.
    for (label, chunk_len, style) in [
        ("non-splitting", 1, StyleProperty::Underline(true)),
        ("non-splitting", 16, StyleProperty::Underline(true)),
        ("splitting", 1, StyleProperty::FontWeight(FontWeight::BOLD)),
        ("splitting", 16, StyleProperty::FontWeight(FontWeight::BOLD)),
    ] {
        benchmarks.push(benchmark_fn(
            format!(
                "Glyph Runs - {} {}, {label} every {chunk_len} chars",
                latin.name, latin.modification
            ),
            move |b| {
                let layout = build_unwrapped_layout(
                    &latin.text,
                    alternating_spans(&latin.text, chunk_len, &style),
                );
                b.iter(move || black_box(walk_items(&layout)))
            },
        ));
    }

    for sample in get_samples()
        .iter()
        .filter(|sample| sample.modification == "8000 characters")
    {
        benchmarks.push(benchmark_fn(
            format!(
                "Glyph Runs - {} {}, mixed",
                sample.name, sample.modification
            ),
            |b| {
                let layout = build_styled_layout(&sample.text);
                b.iter(move || black_box(walk_items(&layout)))
            },
        ));
    }

    benchmarks
}

/// Iterate the positioned glyph runs and glyphs of `layout`, as a renderer would.
fn walk_items(layout: &Layout<ColorBrush>) -> (usize, f32) {
    let mut glyph_count = 0_usize;
    let mut advance = 0.0_f32;
    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    for glyph in glyph_run.positioned_glyphs() {
                        glyph_count += 1;
                        advance += glyph.advance;
                    }
                }
                PositionedLayoutItem::InlineBox(inline_box) => {
                    advance += inline_box.width;
                }
            }
        }
    }
    (glyph_count, advance)
}

/// Benchmark for computing content widths.
///
/// Note this benches the calculation on layouts before line breaking is performed; that's the
/// sequencing that'll likely be used by clients (i.e., measure possible sizes, and later on line
/// break once the target size is known).
pub fn content_widths() -> Vec<Benchmark> {
    const WORD_SPACING: f32 = 2.;
    const LETTER_SPACING: f32 = 1.;

    let samples = get_samples();

    let mut benchmarks = Vec::new();

    for sample in samples.iter() {
        benchmarks.push(benchmark_fn(
            format!(
                "Content Widths - {} {}, uniform",
                sample.name, sample.modification
            ),
            |b| {
                let layout = build_layout(&sample.text, []);
                b.iter(move || black_box(layout.calculate_content_widths()))
            },
        ));
    }

    for sample in samples.iter() {
        benchmarks.push(benchmark_fn(
            format!(
                "Content Widths - {} {}, mixed",
                sample.name, sample.modification
            ),
            |b| {
                let layout = build_layout(&sample.text, styled_spans(&sample.text));
                b.iter(move || black_box(layout.calculate_content_widths()))
            },
        ));
    }

    for sample in samples
        .iter()
        .filter(|sample| sample.modification == "8000 characters")
    {
        benchmarks.push(benchmark_fn(
            format!(
                "Content Widths - {} {}, spacing",
                sample.name, sample.modification
            ),
            |b| {
                let text_range = 0..sample.text.len();
                let layout = build_layout(
                    &sample.text,
                    [
                        (StyleProperty::WordSpacing(WORD_SPACING), text_range.clone()),
                        (StyleProperty::LetterSpacing(LETTER_SPACING), text_range),
                    ],
                );
                b.iter(move || black_box(layout.calculate_content_widths()))
            },
        ));
    }

    benchmarks
}

/// Build `text` into a layout. Each style of `styles` is applied to its given byte range.
///
/// This doesn't break the layout into lines.
pub(crate) fn build_layout<'a>(
    text: &str,
    styles: impl IntoIterator<Item = (StyleProperty<'a, ColorBrush>, Range<usize>)>,
) -> Layout<ColorBrush> {
    const DISPLAY_SCALE: f32 = 1.0;
    const QUANTIZE: bool = true;

    with_contexts(|font_cx, layout_cx| {
        let mut builder = layout_cx.ranged_builder(font_cx, text, DISPLAY_SCALE, QUANTIZE);
        builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
        for (style, range) in styles {
            builder.push(style, range);
        }

        builder.build(text)
    })
}

/// Build `text` and line break without a maximum advance, i.e., lines are not wrapped. Each style
/// of `styles` is applied to its given byte range.
///
/// The layout is a single long line. Depending on the styles used, the proportion of glyphs per
/// shaped run or style span varies. E.g., switching between bold and non-bold, the font changes, so
/// the styles split the text into separately shaped items. Switching between underline and
/// non-underline, there's no impact on shaping, and only the style spans change.
fn build_unwrapped_layout<'a>(
    text: &str,
    styles: impl IntoIterator<Item = (StyleProperty<'a, ColorBrush>, Range<usize>)>,
) -> Layout<ColorBrush> {
    let mut layout = build_layout(text, styles);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// Benchmark for a single very long line (no wrapping) with and without justification.
///
/// This exercises per-line work that scales with line length.
pub fn long_line() -> Vec<Benchmark> {
    const DISPLAY_SCALE: f32 = 1.0;
    const QUANTIZE: bool = true;

    fn layout_long_line(text: &str, max_advance: Option<f32>, alignment: Alignment) {
        with_contexts(|font_cx, layout_cx| {
            let mut builder = layout_cx.ranged_builder(font_cx, text, DISPLAY_SCALE, QUANTIZE);
            builder.push_default(FontFamily::from(FONT_FAMILY_LIST));

            let mut layout: Layout<ColorBrush> = builder.build(text);
            layout.break_all_lines(max_advance);
            layout.align(alignment, AlignmentOptions::default());

            black_box(layout);
        });
    }

    let samples = get_samples();

    samples
        .iter()
        .filter(|sample| sample.modification == "8000 characters")
        .flat_map(|sample| {
            let text: &'static str = Box::leak(sample.text.replace('\n', " ").into_boxed_str());
            [
                benchmark_fn(format!("Long Line - {}", sample.name), move |b| {
                    b.iter(move || layout_long_line(text, None, Alignment::Start))
                }),
                benchmark_fn(format!("Long Line Justify - {}", sample.name), move |b| {
                    // Justification requires a finite `max_advance`; use one wide enough that the
                    // text still lays out as a single line.
                    b.iter(move || layout_long_line(text, Some(1.0e7), Alignment::Justify))
                }),
            ]
        })
        .collect()
}

/// Benchmark the type of work a document viewer performs.
///
/// Each paragraph in the source text becomes a [`Layout`]. This benches the following:
///
/// - a full pass from building the layouts to iterating each layouts' glyphs;
/// - each of the various stages separately (e.g., line-breaking all the layouts one after the
///   other); and
/// - resizing layouts one after the other, cycling through some different max advances.
pub fn page() -> Vec<Benchmark> {
    // Around 90 characters of 16px Roboto per line.
    const MAX_ADVANCE: f32 = 720.;
    // The widths a resize cycles through.
    const RESIZE_MAX_ADVANCES: [f32; 6] = [720., 700., 660., 600., 520., 400.];

    /// One layout per paragraph of `page`.
    fn build_page(text: &[&str]) -> Vec<Layout<ColorBrush>> {
        text.iter()
            .map(|paragraph| build_layout(paragraph, []))
            .collect()
    }

    /// Line-break every layout of a page, returning the total line count.
    fn break_page(layouts: &mut [Layout<ColorBrush>], max_advance: f32) -> usize {
        layouts
            .iter_mut()
            .map(|layout| {
                layout.break_all_lines(Some(max_advance));
                layout.align(Alignment::Start, AlignmentOptions::default());
                layout.len()
            })
            .sum()
    }

    /// Measures every layout's content widths, and returns the total min and max widths.
    fn measure_page(layouts: &[Layout<ColorBrush>]) -> (f32, f32) {
        layouts.iter().fold((0.0, 0.0), |(min, max), layout| {
            let widths = layout.calculate_content_widths();
            (f32::max(min, widths.min), f32::max(max, widths.max))
        })
    }

    /// Walk the glyph runs of every layout of a page.
    fn paint_page(layouts: &[Layout<ColorBrush>]) -> (usize, f32) {
        layouts
            .iter()
            .map(walk_items)
            .fold((0, 0.0), |(count, advance), (c, a)| {
                (count + c, advance + a)
            })
    }

    /// Split a [`Sample`] into paragraphs.
    fn paragraphs_of(sample: &Sample) -> Vec<&str> {
        sample.text.split('\n').collect()
    }

    let samples = || {
        get_samples()
            .iter()
            .filter(|sample| sample.modification == "8000 characters")
    };

    let mut benchmarks = Vec::new();

    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!("Page Full - {} {}", sample.name, sample.modification),
            move |b| {
                let page = paragraphs_of(sample);
                b.iter(move || {
                    let mut layouts = build_page(&page);
                    let widths = measure_page(&layouts);
                    let lines = break_page(&mut layouts, MAX_ADVANCE);
                    let painted = paint_page(&layouts);
                    black_box((widths, lines, painted))
                })
            },
        ));
    }

    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!("Page Build - {} {}", sample.name, sample.modification),
            move |b| {
                let page = paragraphs_of(sample);
                b.iter(move || black_box(build_page(&page)))
            },
        ));
    }

    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!(
                "Page Content Widths - {} {}",
                sample.name, sample.modification
            ),
            move |b| {
                let layouts = build_page(&paragraphs_of(sample));
                b.iter(move || black_box(measure_page(&layouts)))
            },
        ));
    }

    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!("Page Break - {} {}", sample.name, sample.modification),
            move |b| {
                let mut layouts = build_page(&paragraphs_of(sample));
                b.iter(move || black_box(break_page(&mut layouts, MAX_ADVANCE)))
            },
        ));
    }

    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!("Page Resize - {} {}", sample.name, sample.modification),
            move |b| {
                let mut layouts = build_page(&paragraphs_of(sample));
                let mut max_advances = RESIZE_MAX_ADVANCES.iter().copied().cycle();
                b.iter(move || {
                    let max_advance = max_advances.next().unwrap();
                    black_box(break_page(&mut layouts, max_advance))
                })
            },
        ));
    }

    for sample in samples() {
        benchmarks.push(benchmark_fn(
            format!("Page Glyph Runs - {} {}", sample.name, sample.modification),
            move |b| {
                let mut layouts = build_page(&paragraphs_of(sample));
                break_page(&mut layouts, MAX_ADVANCE);
                b.iter(move || black_box(paint_page(&layouts)))
            },
        ));
    }

    benchmarks
}
