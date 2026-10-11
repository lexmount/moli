// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Whitespace-related utilities, for implementing e.g. whitespace collapse and whitespace hanging.
//!
//! # Conditional vs. unconditional whitespace hanging
//!
//! Under CSS Text 4 § 4.3.2, some whitespace hangs, and some may hang *conditionally*.
//! Conditionally hanging whitespace only hangs if it overflows the line. A sequence of
//! `white-space-collapse: preserve; text-wrap-mode: wrap;` whitespace hangs conditionally if it's
//! directly followed by a forced line break; otherwise, it hangs unconditionally. The end of the
//! layout counts as a forced line break (CSS Text 4 § 5). In particular, if the `preserve` sequence
//! is followed by a sequence of `collapse` whitespace (and not a break), then it hangs
//! unconditionally. Effectively, the two sequences can just be treated as one that's
//! unconditionally hanging.
//!
//! Under CSS Text 4 § 9.2, unconditionally hanging whitespace followed by conditionally hanging
//! whitespace, hangs in full, but only if the conditionally-hanging whitespace hangs in full. See
//! <https://github.com/linebender/parley/pull/831> for an HTML sample.
//!
//! So, in effect, the only behaviorally observable case of conditional vs. unconditional hanging
//! can be represented as a suffix of conditionally hanging whitespace, preceded by unconditionally
//! hanging whitespace.
//!
//! Further note that normally, under `white-space-collapse: collapse`, most but not all whitespace
//! is collapsed; e.g., the Ideographic Space (U+3000) does not collapse. It generally takes
//! deliberate effort to write a sample that exercises these edge cases.

use parley_engine::shape::Whitespace;
use parley_engine::{Atom, ShapedSlice};

use crate::layout::Style;
use crate::layout::spacing::EffectiveSpacing;
use crate::style::Brush;
use crate::{TextWrapMode, WhiteSpaceCollapse};

impl WhiteSpaceCollapse {
    /// Whether `c` is whitespace that this mode collapses.
    pub(crate) fn is_collapsible(self, c: char) -> bool {
        match self {
            Self::Collapse => c.is_ascii_whitespace(),
            Self::Preserve | Self::BreakSpaces => false,
            Self::PreserveBreaks => matches!(c, ' ' | '\t'),
        }
    }
}

/// Whether whitespace with the given style hangs past the line's end edge.
///
/// Following [CSS Text 4 § 4.3.2][css-hanging], non-breaking spaces don't hang, preserved
/// whitespace only hangs when wrapping is enabled, and whitespace in a collapsing mode always
/// hangs.
///
/// [css-hanging]: https://www.w3.org/TR/css-text-4/#white-space-phase-2
///
// Note: CSS Text 4 effectively includes tab, newline, plus all of the "space separators" here
// (Unicode category `Zs`), except for non-breaking space. However, we only include space and
// ideographic space from the "space separators". See
// https://github.com/linebender/parley/pull/762#discussion_r3923722770.
#[inline(always)]
pub(crate) fn whitespace_hangs<B: Brush>(whitespace: Whitespace, style: &Style<B>) -> bool {
    match whitespace {
        Whitespace::Newline => true,
        Whitespace::Space | Whitespace::IdeographicSpace | Whitespace::Tab => {
            match style.white_space_collapse {
                WhiteSpaceCollapse::BreakSpaces => false,
                WhiteSpaceCollapse::Preserve => style.text_wrap_mode == TextWrapMode::Wrap,
                _ => true,
            }
        }
        _ => false,
    }
}

/// The advance of the logically trailing clusters of `atom` that hang past the line's end, and
/// whether that is the atom in its entirety.
///
/// An atom can hang partially: e.g., a prepend character followed by a space is a single atom (as
/// it's a grapheme), but may consist of multiple shaped clusters, of which the spaces can hang.
/// The atom's spacing at its logical end hangs along with the clusters, and if the atom hangs in
/// its entirety does its spacing at its logical start hang as well.
#[inline(always)]
pub(crate) fn atom_hanging_advance<B: Brush>(
    slice: ShapedSlice<'_>,
    atom: &Atom<'_>,
    styles: &[Style<B>],
    spacing: EffectiveSpacing,
    is_rtl: bool,
) -> (f32, bool) {
    let gaps = spacing.gaps(atom);
    let (gap_start, gap_end) = if is_rtl {
        (gaps.after, gaps.before)
    } else {
        (gaps.before, gaps.after)
    };
    let mut last_cluster = true;
    let mut all_hang = true;
    let mut hanging = 0.;
    for cluster in atom.shaped_clusters().iter().rev() {
        let cluster_hangs = slice
            .characters_in(cluster.chars_range())
            .iter()
            .all(|character| {
                whitespace_hangs(
                    character.whitespace,
                    &styles[character.style_index as usize],
                )
            });
        if !cluster_hangs {
            all_hang = false;
            break;
        }
        if last_cluster {
            hanging += gap_end;
            last_cluster = false;
        }
        hanging += cluster.advance;
    }

    let hanging = if all_hang {
        hanging + gap_start
    } else {
        hanging
    };

    (hanging, all_hang)
}
