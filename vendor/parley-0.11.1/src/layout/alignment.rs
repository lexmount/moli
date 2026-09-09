// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    BreakReason,
    data::{ClusterData, LineItemData},
};
use crate::data::LayoutData;
use crate::style::Brush;

/// Alignment of a layout.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Alignment {
    /// This is [`Alignment::Left`] for LTR text and [`Alignment::Right`] for RTL text.
    #[default]
    Start,
    /// This is [`Alignment::Right`] for LTR text and [`Alignment::Left`] for RTL text.
    End,
    /// Align content to the left edge.
    ///
    /// For alignment that should be aware of text direction, use [`Alignment::Start`] or
    /// [`Alignment::End`] instead.
    Left,
    /// Align each line centered within the container.
    Center,
    /// Align content to the right edge.
    ///
    /// For alignment that should be aware of text direction, use [`Alignment::Start`] or
    /// [`Alignment::End`] instead.
    Right,
    /// Justify each line by spacing out content, except for the last line.
    Justify,
}

/// Additional options to fine tune alignment
#[derive(Debug, Clone, Copy)]
pub struct AlignmentOptions {
    /// If set to `true`, "end" and "center" alignment will apply even if the line contents are
    /// wider than the alignment width. If it is set to `false`, all overflowing lines will be
    /// [`Alignment::Start`] aligned.
    pub align_when_overflowing: bool,
}

#[expect(
    clippy::derivable_impls,
    reason = "Make default values explicit rather than relying on the implicit default value of bool"
)]
impl Default for AlignmentOptions {
    fn default() -> Self {
        Self {
            align_when_overflowing: false,
        }
    }
}

/// Align the layout.
///
/// If [`Alignment::Justify`] is requested, clusters' [`ClusterData::advance`] will be adjusted.
/// Prior to re-line-breaking or re-aligning, [`unjustify`] has to be called.
pub(crate) fn align<B: Brush>(
    layout: &mut LayoutData<B>,
    alignment: Alignment,
    options: AlignmentOptions,
) {
    #[cfg(feature = "accesskit")]
    {
        layout.alignment = Some(alignment);
    }
    layout.is_aligned_justified = alignment == Alignment::Justify;

    align_impl::<_, false>(layout, alignment, options);
}

/// Removes previous justification applied to clusters.
///
/// This is part of resetting state in preparation for re-line-breaking or re-aligning the same
/// layout.
pub(crate) fn unjustify<B: Brush>(layout: &mut LayoutData<B>) {
    if layout.is_aligned_justified {
        align_impl::<_, true>(layout, Alignment::Justify, AlignmentOptions::default());
        layout.is_aligned_justified = false;
    }
}

/// The actual alignment implementation.
///
/// This is const-generic over `UNDO_JUSTIFICATION`: justified alignment adjusts clusters'
/// [`ClusterData::advance`], and this mutation has to be undone for re-line-breaking or
/// re-aligning. `UNDO_JUSTIFICATION` indicates whether the adjustment has to be applied, or
/// undone.
///
/// Writing a separate function for undoing justification would be faster, but not by much, and
/// doing it this way we are sure the calculations performed are equivalent.
fn align_impl<B: Brush, const UNDO_JUSTIFICATION: bool>(
    layout: &mut LayoutData<B>,
    alignment: Alignment,
    options: AlignmentOptions,
) {
    // Whether the text base direction is right-to-left.
    let is_rtl = layout.base_level & 1 == 1;

    // Apply alignment to line items
    for line in &mut layout.lines {
        let indent = line.indent;

        if is_rtl {
            // In RTL text, trailing whitespace is on the left. As we hang that whitespace, offset
            // the line to the left. Note: indent is not subtracted here because `free_space` below
            // already accounts for it.
            line.metrics.offset = -line.metrics.trailing_whitespace;
        } else {
            line.metrics.offset = indent;
        }

        // Compute free space.
        let line_width = line.metrics.inline_max_coord - line.metrics.inline_min_coord;
        let free_space =
            line_width - indent - line.metrics.advance + line.metrics.trailing_whitespace;

        if !options.align_when_overflowing && free_space <= 0.0 {
            if is_rtl {
                // In RTL text, right-align on overflow.
                line.metrics.offset += free_space;
            }
            continue;
        }

        match (alignment, is_rtl) {
            (Alignment::Left, _) | (Alignment::Start, false) | (Alignment::End, true) => {
                // Do nothing
            }
            (Alignment::Right, _) | (Alignment::Start, true) | (Alignment::End, false) => {
                line.metrics.offset += free_space;
            }
            (Alignment::Center, _) => {
                line.metrics.offset += free_space * 0.5;
            }
            (Alignment::Justify, _) => {
                // Justified alignment doesn't have any effect if free_space is negative or zero
                if free_space <= 0.0 {
                    continue;
                }

                // Justified alignment doesn't apply to the last line of a paragraph
                // (`BreakReason::None`), (`BreakReason::Explicit`) or if there are no whitespace
                // gaps to adjust. In that case, start-align, i.e., left-align for LTR text and
                // right-align for RTL text.
                if matches!(line.break_reason, BreakReason::None | BreakReason::Explicit)
                    || line.num_spaces == 0
                {
                    if is_rtl {
                        line.metrics.offset += free_space;
                    }
                    continue;
                }

                let adjustment =
                    free_space / line.num_spaces as f32 * if UNDO_JUSTIFICATION { -1. } else { 1. };
                let mut applied = 0;
                // Iterate over text runs in the line and clusters in the text run
                //   - Iterate forwards for even bidi levels (which represent LTR runs)
                //   - Iterate backwards for odd bidi levels (which represent RTL runs)
                let line_items: &mut dyn Iterator<Item = &LineItemData> = if is_rtl {
                    &mut layout.line_items[line.item_range.clone()].iter().rev()
                } else {
                    &mut layout.line_items[line.item_range.clone()].iter()
                };
                line_items
                    .filter(|item| item.is_text_run())
                    .for_each(|line_item| {
                        let clusters = &mut layout.clusters[line_item.cluster_range.clone()];
                        let line_item_is_rtl = line_item.bidi_level & 1 != 0;
                        let clusters: &mut dyn Iterator<Item = &mut ClusterData> =
                            if line_item_is_rtl {
                                &mut clusters.iter_mut().rev()
                            } else {
                                &mut clusters.iter_mut()
                            };
                        clusters.for_each(|cluster| {
                            if applied == line.num_spaces {
                                return;
                            }
                            if cluster.info.whitespace().is_space_or_nbsp() {
                                cluster.advance += adjustment;
                                applied += 1;
                            }
                        });
                    });
            }
        }
    }
}
