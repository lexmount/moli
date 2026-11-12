// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::text_advance::{TextAdvance, TextItemQuantization};

use crate::inline_box::{InlineBox, LayoutInlineBox};
use crate::layout::spacing::{Justification, Spacing};
use crate::layout::style_metrics::{BoxMetrics, StyleMetrics};
use crate::layout::whitespace::whitespace_hangs;
use crate::layout::{ContentWidths, LineMetrics, Style};
use crate::resolve::ResolvedStyle;
use crate::style::Brush;
use crate::{
    IndentOptions, InlineBoxKind, LineHeight, OverflowWrap, TextWrapMode, WhiteSpaceCollapse,
};
use core::ops::Range;

use alloc::vec::Vec;
use parlance::BidiLevel;
use parley_engine::shape::Whitespace;
use parley_engine::{ShapedRun, ShapedSlice, ShapedText};

/// `HarfRust`-based run data
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunData {
    /// Font attributes, needed for accessibility.
    pub(crate) font_attrs: fontique::Attributes,
    /// Synthesis for rendering (contains variation settings)
    pub(crate) synthesis: fontique::Synthesis,
    /// The line height
    pub line_height: f32,
    /// Additional spacing inserted between this run's atoms.
    ///
    /// TODO: Letter spacing in the form of gaps should not be applied between cursive scripts, see
    /// [CSS Text 4 § 8.2.1][css-spacing-cursive]. Currently we erroneously *do* apply it.
    ///
    /// [css-spacing-cursive]: https://www.w3.org/TR/css-text-4/#cursive-tracking
    pub(crate) spacing: Spacing,
    /// The box metrics this run adds to the line box, if this run has uniform box metrics.
    ///
    /// If the metrics change within the run, this is `None`. The run's boxes should then be
    /// resolved per style.
    pub(crate) run_box: Option<BoxMetrics>,
    /// Whether this run has atoms with styles changing mid-atom.
    ///
    /// This can happen when ligatures span across styles, or (awkwardly) if a style changes within
    /// a grapheme.
    pub(crate) has_mixed_style_atoms: bool,
}

/// The metrics of a run's box. This differs from the style-derived box by taking into account any
/// fallback fonts (rather than just using the first available font).
///
/// Per [CSS Inline 3 § 4.1], glyphs from fonts other than the first available font only
/// contribute to the line when the `line-height` is `normal` ([`LineHeight::MetricsRelative`]);
/// otherwise the style's span box alone sizes the line and this returns `None`.
///
/// [CSS Inline 3 § 4.1]: https://drafts.csswg.org/css-inline-3/#inline-height
#[inline]
pub(crate) fn run_box_metrics<B: Brush>(
    style: &Style<B>,
    shaped_run: &ShapedRun,
    quantize: bool,
) -> Option<BoxMetrics> {
    match style.line_height {
        LineHeight::MetricsRelative(_) => {
            let (size, metrics) = (shaped_run.font_size, &shaped_run.font_metrics);
            let line_height = style.line_height.resolve(size, metrics, quantize);
            Some(BoxMetrics::from_font(metrics, line_height, quantize))
        }
        LineHeight::FontSizeRelative(_) | LineHeight::Absolute(_) => None,
    }
}

#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub enum BreakReason {
    #[default]
    None,
    Regular,
    Explicit,
    Emergency,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct LineData {
    /// Range of the source text.
    pub(crate) text_range: Range<usize>,
    /// Range of line items.
    pub(crate) item_range: Range<usize>,
    /// Metrics for the line.
    pub(crate) metrics: LineMetrics,
    /// The cause of the line break.
    pub(crate) break_reason: BreakReason,
    /// Maximum advance for the line.
    pub(crate) max_advance: f32,
    /// The number of justification opportunities on the line.
    pub(crate) num_justification_opportunities: u32,
    pub(crate) justification: Justification,
    /// Text indent applied to this line.
    pub(crate) indent: f32,
    /// This line's entries in [`LayoutData::aligned_subtree_offsets`].
    ///
    /// Empty for lines with only baseline-relative content because the root aligned subtree
    /// for each line trivially has an offset of 0
    pub(crate) aligned_subtree_offsets: Range<u32>,
}

/// Position of an [independent aligned subtree] rooted at a span with
/// `vertical-align: top | bottom` on a line. Computed for each such subtree on the line in
/// `BreakLines::finish_line`.
///
/// [independent aligned subtree]: crate::layout::style_metrics#aligned-subtrees
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AlignedSubtreeOffset {
    /// Style index of the subtree root (a span with `vertical-align: top | bottom`)
    pub(crate) root: u16,
    /// Offset from the line's baseline to the subtree's baseline (positive upwards).
    pub(crate) baseline_offset: f32,
}

impl LineData {
    /// Offset from the line's baseline to the baseline of the aligned subtree rooted at style
    /// `root` (positive upwards). `offsets` is [`LayoutData::aligned_subtree_offsets`].
    pub(crate) fn aligned_subtree_offset(
        &self,
        offsets: &[AlignedSubtreeOffset],
        root: u16,
    ) -> f32 {
        if root == 0 {
            return 0.;
        }
        let range =
            self.aligned_subtree_offsets.start as usize..self.aligned_subtree_offsets.end as usize;
        offsets[range]
            .iter()
            .find(|subtree| subtree.root == root)
            .map_or(0., |subtree| subtree.baseline_offset)
    }

    /// Absolute block-axis coordinate (offset from the top of the layout) of the baseline of the given style's span box.
    // `offsets` is [`LayoutData::aligned_subtree_offsets`].
    pub(crate) fn style_baseline(
        &self,
        offsets: &[AlignedSubtreeOffset],
        metrics: &StyleMetrics,
    ) -> f32 {
        self.metrics.baseline
            - self.aligned_subtree_offset(offsets, metrics.aligned_subtree_root)
            - metrics.baseline_offset
    }

    pub(crate) fn size(&self) -> f32 {
        self.metrics.line_height
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LineItemData {
    /// Whether the item is a run or an inline box
    pub(crate) kind: LayoutItemKind,
    /// The index of the run or inline box in the runs or `inline_boxes` vec
    pub(crate) index: usize,
    /// Bidi level for the item (used for reordering)
    pub(crate) bidi_level: BidiLevel,

    // Fields that only apply to text runs (Ignored for boxes)
    // TODO: factor this out?
    /// Range of the source text.
    pub(crate) text_range: Range<usize>,
    /// This run's shaped clusters on this line, as a range into [`ShapedText::shaped_clusters`].
    ///
    /// The bounds are atom-aligned.
    pub(crate) shaped_cluster_range: Range<u32>,
    pub(crate) allocated_advance: Option<f32>,
}

impl LineItemData {
    #[inline(always)]
    pub(crate) fn is_rtl(&self) -> bool {
        self.bidi_level.is_rtl()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutItemKind {
    TextRun,
    InlineBox,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LayoutItem {
    /// Whether the item is a run or an inline box
    pub(crate) kind: LayoutItemKind,
    /// The index of the run or inline box in the runs or `inline_boxes` vec
    pub(crate) index: usize,
    /// Bidi level for the item (used for reordering)
    pub(crate) bidi_level: BidiLevel,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutData<B: Brush> {
    // General settings (directly from the "builder")
    /// The display scale factor
    pub(crate) scale: f32,
    /// Whether metrics should be quantized to pixel boundaries
    pub(crate) quantize: bool,
    /// The `BiDi` base level
    pub(crate) base_level: BidiLevel,
    /// The length of the text in the layout
    pub(crate) text_len: usize,

    // Output of style resolution (input to line breaking)
    pub(crate) styles: Vec<Style<B>>,
    pub(crate) style_metrics: Vec<StyleMetrics>,
    pub(crate) inline_boxes: Vec<LayoutInlineBox>,

    // Output of shaping (input to line breaking)
    pub(crate) shaped_text: ShapedText,
    pub(crate) text_item_quantization: Option<TextItemQuantization>,
    pub(crate) runs: Vec<RunData>,
    pub(crate) items: Vec<LayoutItem>,

    // Output of line breaking
    /// The lines in the
    pub(crate) lines: Vec<LineData>,
    /// Items within each line
    pub(crate) line_items: Vec<LineItemData>,
    /// Position of each aligned subtree rooted at a `vertical-align: top | bottom` style on each line.
    /// The root aligned subtree of each line doesn't have an entry as its offset is trivially zero.
    ///
    /// Each line owns a contiguous slice ([`LineData::aligned_subtree_offsets`]).
    pub(crate) aligned_subtree_offsets: Vec<AlignedSubtreeOffset>,
    /// The width constraint that was used to line break the layout
    pub(crate) layout_max_advance: f32,
    /// The computed width of the layout excluding hanging whitespace
    pub(crate) width: f32,
    /// The computed width of the layout including hanging whitespace
    pub(crate) full_width: f32,
    /// The computed height of the layout
    pub(crate) height: f32,

    // Output of alignment
    /// The alignment that was applied to the layout, if any.
    pub(crate) alignment: Option<super::Alignment>,
    /// The text-indent amount in layout units.
    pub(crate) indent_amount: f32,
    /// Options controlling text-indent behavior (each-line, hanging).
    pub(crate) indent_options: IndentOptions,
}

impl<B: Brush> Default for LayoutData<B> {
    fn default() -> Self {
        Self {
            scale: 1.,
            quantize: true,
            base_level: BidiLevel::new(0),
            text_len: 0,
            width: 0.,
            full_width: 0.,
            height: 0.,
            styles: Vec::new(),
            style_metrics: Vec::new(),
            inline_boxes: Vec::new(),
            shaped_text: ShapedText::new(),
            text_item_quantization: None,
            runs: Vec::new(),
            items: Vec::new(),
            lines: Vec::new(),
            line_items: Vec::new(),
            aligned_subtree_offsets: Vec::new(),
            alignment: None,
            layout_max_advance: 0.0,
            indent_amount: 0.0,
            indent_options: IndentOptions::default(),
        }
    }
}

impl<B: Brush> LayoutData<B> {
    pub(crate) fn clear(&mut self) {
        self.scale = 1.;
        self.quantize = true;
        self.base_level = BidiLevel::new(0);
        self.text_len = 0;
        self.width = 0.;
        self.full_width = 0.;
        self.height = 0.;
        self.layout_max_advance = 0.0;
        self.indent_amount = 0.0;
        self.indent_options = IndentOptions::default();
        self.styles.clear();
        self.style_metrics.clear();
        self.inline_boxes.clear();
        self.shaped_text.clear();
        self.text_item_quantization = None;
        self.runs.clear();
        self.items.clear();
        self.lines.clear();
        self.line_items.clear();
        self.aligned_subtree_offsets.clear();
        self.alignment = None;
    }

    /// Push an inline box to the list of items
    pub(crate) fn push_inline_box(&mut self, index: usize, bidi_level: BidiLevel) {
        self.items.push(LayoutItem {
            kind: LayoutItemKind::InlineBox,
            index,
            bidi_level,
        });
    }
    /// Processes a shaped run into [`RunData`] and a [`LayoutItem`].
    ///
    /// `char_styles` are the style indices of the text's characters.
    #[expect(clippy::cast_possible_truncation, reason = "deferred")]
    pub(crate) fn process_shaped_run(
        &mut self,
        shaped_run_idx: usize,
        run_style: &ResolvedStyle<B>,
        spacing: Spacing,
        char_styles: &[u16],
    ) {
        let shaped_run = &self.shaped_text.runs()[shaped_run_idx];
        debug_assert!(
            !shaped_run.shaped_clusters_range.is_empty(),
            "Shaped runs returned by `parley_engine` must be non-empty"
        );
        let style_index =
            self.shaped_text.characters()[shaped_run.characters_range.start as usize].style_index;

        let line_height = self.styles[style_index as usize].line_height.resolve(
            shaped_run.font_size,
            &shaped_run.font_metrics,
            self.quantize,
        );

        // The run's box metrics, if it's uniform across all the styles within the run. A run's box
        // can change mid-run if the line height changes.
        let run_box = {
            let chars = shaped_run.range.char_range.clone();
            let first_style = char_styles[chars.start];
            let line_height = self.styles[usize::from(first_style)].line_height;

            let uniform_line_height = char_styles[chars].iter().all(|&s| {
                s == first_style || self.styles[usize::from(s)].line_height == line_height
            });

            if uniform_line_height {
                run_box_metrics(
                    &self.styles[usize::from(first_style)],
                    shaped_run,
                    self.quantize,
                )
            } else {
                None
            }
        };

        // An atom has mixed styles iff a style changes at one of its characters other than its
        // first.
        let has_mixed_style_atoms = {
            let chars = shaped_run.range.char_range.clone();
            let slice = self.shaped_text.run_slice(shaped_run_idx as u32);
            (chars.start + 1..chars.end).any(|char_idx| {
                char_styles[char_idx - 1] != char_styles[char_idx]
                    && slice
                        .atom_at_char(char_idx as u32)
                        .is_some_and(|atom| atom.char_range().start != char_idx as u32)
            })
        };

        let font = &self.shaped_text.fonts()[shaped_run.font_index];
        let run = RunData {
            font_attrs: fontique::Attributes {
                width: run_style.font_width,
                weight: run_style.font_weight,
                style: run_style.font_style,
            },
            synthesis: font.synthesis,
            line_height,
            spacing,
            run_box,
            has_mixed_style_atoms,
        };

        self.runs.push(run);
        self.items.push(LayoutItem {
            kind: LayoutItemKind::TextRun,
            index: self.runs.len() - 1,
            bidi_level: shaped_run.bidi_level,
        });
    }

    /// Calculates the min- and max-content widths of the layout.
    ///
    /// The max-content width is the widest line when breaking only at forced breaks. The
    /// min-content width is the widest line when breaking at every opportunity.
    ///
    /// This should mirror the line breaker in terms of layout decisions like hanging whitespace (if
    /// it doesn't, one of the calculations is buggy).
    ///
    /// Hanging whitespace is not considered when measuring the line's content for fit, but some
    /// whitespace hangs *conditionally*. See the module-level docs of [`crate::layout::whitespace`]
    /// for an explanation of how conditionally and unconditionally hanging whitespace are
    /// determined. For the content widths this means:
    ///
    /// - max-content width: if there's a conditionally-hanging suffix of whitespace, it fits, so
    ///   nothing hangs; without one, the unconditionally hanging whitespace hangs in full.
    /// - min-content width: any conditionally-hanging suffix hangs in full, so all hanging
    ///   whitespace hangs in full.
    pub(crate) fn calculate_content_widths(&self) -> ContentWidths {
        ContentWidthsMeasurer::new().measure(self)
    }
}

struct ContentWidthsMeasurer {
    min_width: f32,
    max_width: f32,

    running_min_width: TextAdvance,
    running_max_width: TextAdvance,

    /// The running advance of whitespace that would hang if a line ended here. Can exceed
    /// `running_min_width` when the hanging whitespace started before the last break
    /// opportunity, in which case the line consists entirely of hanging whitespace.
    running_hanging_whitespace: f32,

    /// Whether that running whitespace hangs conditionally before a forced break (following CSS
    /// Text 4 § 4.3.2, that's the case for `WhiteSpaceCollapse::Preserve`). Conditionally hanging
    /// whitespace only hangs if it doesn't fit, which here means it counts towards the max-content
    /// width, but not the min-content width.
    hangs_conditionally: bool,

    text_wrap_mode: TextWrapMode,
}

impl ContentWidthsMeasurer {
    #[inline(always)]
    fn new() -> Self {
        Self {
            min_width: 0.,
            max_width: 0.,
            running_min_width: TextAdvance::default(),
            running_max_width: TextAdvance::default(),
            running_hanging_whitespace: 0.,
            hangs_conditionally: false,
            text_wrap_mode: TextWrapMode::Wrap,
        }
    }

    /// Ends the current min-content fragment at a soft wrap opportunity.
    ///
    /// Under a min-content constraint, every soft wrap opportunity is taken.
    #[inline(always)]
    fn soft_break(&mut self) {
        self.min_width = self.min_width.max(
            self.running_min_width
                .without_trailing_space(self.running_hanging_whitespace),
        );
        self.running_min_width = TextAdvance::default();
    }

    /// Ends the current line at a forced break or the end of the layout.
    #[inline(always)]
    fn hard_break(&mut self) {
        self.min_width = self.min_width.max(
            self.running_min_width
                .without_trailing_space(self.running_hanging_whitespace),
        );
        if !self.hangs_conditionally {
            self.running_max_width = TextAdvance::from_width(
                self.running_max_width
                    .without_trailing_space(self.running_hanging_whitespace),
            );
        }
        self.max_width = self.max_width.max(self.running_max_width.value);
        self.running_min_width = TextAdvance::default();
        self.running_max_width = TextAdvance::default();
        self.running_hanging_whitespace = 0.0;
    }

    #[inline(always)]
    fn measure<B: Brush>(mut self, layout_data: &LayoutData<B>) -> ContentWidths {
        for item in &layout_data.items {
            match item.kind {
                LayoutItemKind::TextRun => {
                    let slice = layout_data.shaped_text.run_slice(item.index as u32);
                    let run_spacing = layout_data.runs[item.index].spacing;
                    // Trailing whitespace can only hang if it ends up at the line's end edge after
                    // bidi reordering. We don't currently apply UAX #9 L1 (resetting trailing
                    // whitespace to paragraph level), so only logically-last items that match the
                    // paragraph level are guaranteed to be at that edge.
                    let can_hang = item.bidi_level == layout_data.base_level;
                    let is_rtl = item.bidi_level.is_rtl();

                    if run_spacing.is_zero() {
                        self.measure_text_run::<B, false>(
                            &layout_data.styles,
                            slice,
                            run_spacing,
                            can_hang,
                            is_rtl,
                            layout_data.text_item_quantization.as_ref(),
                        );
                    } else {
                        self.measure_text_run::<B, true>(
                            &layout_data.styles,
                            slice,
                            run_spacing,
                            can_hang,
                            is_rtl,
                            layout_data.text_item_quantization.as_ref(),
                        );
                    }
                }
                LayoutItemKind::InlineBox => {
                    self.measure_inline_box(&layout_data.inline_boxes[item.index].inline_box);
                }
            }
        }

        // The end of a layout is considered to be a forced line break as per CSS Text 4 § 5, so
        // whitespace can hang conditionally.
        self.hard_break();

        // Negative-width inline boxes (e.g. negative margins) can make the max-content width
        // smaller than the min-content width. CSS Sizing 3 § 2.1 requires the max-content size to
        // be floored by the min-content size.
        self.max_width = self.max_width.max(self.min_width);

        ContentWidths {
            min: self.min_width,
            max: self.max_width,
        }
    }

    /// Measures a text run.
    ///
    /// The run is scanned one shaped cluster at a time. break opportunities are only considered at
    /// grapheme starts, and the hanging-whitespace accumulator is updated per cluster (a cluster
    /// that doesn't hang resets it, one that does extends it), which is equivalent to the per-atom
    /// "hangs entirely / hangs partially from the logical end" rule of
    /// [`atom_hanging_advance`](crate::layout::whitespace::atom_hanging_advance).
    ///
    /// With `SPACED`, each atom's spacing gap is added to the cluster on its visual end, i.e., its
    /// logically last cluster for LTR and its logically first cluster for RTL. The gap then hangs
    /// with that cluster, as in `atom_hanging_advance`. Without `SPACED`, `spacing` is ignored.
    #[inline(always)]
    fn measure_text_run<B: Brush, const SPACED: bool>(
        &mut self,
        styles: &[Style<B>],
        slice: ShapedSlice<'_>,
        spacing: Spacing,
        can_hang: bool,
        is_rtl: bool,
        quantization: Option<&TextItemQuantization>,
    ) {
        let clusters = slice.shaped_clusters();

        // The spacing gap of the current atom.
        let mut gap = 0.;
        let mut skip_atom = false;
        for (i, cluster) in clusters.iter().enumerate() {
            let whitespace = cluster.whitespace();
            let style = &styles[cluster.style_index as usize];
            let is_atom_start = i == 0 || cluster.is_grapheme_start();
            if is_atom_start {
                skip_atom = false;
                let prev_text_wrap_mode = self.text_wrap_mode;
                self.text_wrap_mode = style.text_wrap_mode;
                if prev_text_wrap_mode == TextWrapMode::Wrap
                    && (cluster.is_soft_wrap_opportunity_before()
                        || style.overflow_wrap == OverflowWrap::Anywhere)
                {
                    self.soft_break();
                }
                // `Whitespace::Newline` are forced breaks. Note newlines have no advance.
                if whitespace == Whitespace::Newline {
                    // Newlines hang, so whitespace before them keeps hanging.
                    self.hard_break();
                    skip_atom = true;
                    continue;
                }
                if SPACED {
                    gap = spacing.gaps(whitespace).after;
                }
            } else if skip_atom {
                continue;
            }

            let mut advance = cluster.advance;
            if SPACED {
                let owns_gap = if is_rtl {
                    is_atom_start
                } else {
                    clusters
                        .get(i + 1)
                        .is_none_or(|next| next.is_grapheme_start())
                };
                if owns_gap {
                    advance += gap;
                }
            }
            let allocation = quantization
                .map(|policy| policy.cluster(slice.shaped_clusters_range().start + i as u32));
            self.running_min_width = self.running_min_width.with_text(advance, allocation);
            self.running_max_width = self.running_max_width.with_text(advance, allocation);

            // A cluster hangs if all of its characters hang. Its first character is checked via the
            // cached flags so the common case never touches `characters`.
            let hangs = can_hang
                && whitespace_hangs(whitespace, style)
                && (cluster.char_len() == 1
                    || slice
                        .characters_in(cluster.chars_range())
                        .iter()
                        .all(|character| {
                            whitespace_hangs(
                                character.whitespace,
                                &styles[character.style_index as usize],
                            )
                        }));
            if hangs {
                self.running_hanging_whitespace += advance;
                self.hangs_conditionally =
                    style.white_space_collapse == WhiteSpaceCollapse::Preserve;
            } else {
                self.running_hanging_whitespace = 0.0;
            }
        }
    }

    fn measure_inline_box(&mut self, inline_box: &InlineBox) {
        if inline_box.kind == InlineBoxKind::InFlow {
            // Inline boxes have soft wrap opportunities on both sides.
            let wraps = self.text_wrap_mode == TextWrapMode::Wrap;
            if wraps {
                self.soft_break();
            }
            // Inline boxes don't hang.
            self.running_hanging_whitespace = 0.0;
            self.running_min_width =
                TextAdvance::from_width(self.running_min_width.value + inline_box.width);
            self.running_max_width =
                TextAdvance::from_width(self.running_max_width.value + inline_box.width);
            if wraps {
                self.soft_break();
            }
        }
    }
}
