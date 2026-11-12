// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Layout types.

mod alignment;
mod cluster;
mod line;
mod line_break;
mod run;
mod spacing;
pub(crate) mod style_metrics;
mod whitespace;
mod text_advance;

// TODO - Add to allowed lint set?
#[expect(
    clippy::module_inception,
    reason = "Private inner module for code organisation"
)]
mod layout;

pub use parley_engine::{FontMetrics, Glyph};

pub(crate) mod data;

pub use alignment::{Alignment, AlignmentOptions};
pub use cluster::{Affinity, Cluster, ClusterPath, ClusterSide};
pub use data::BreakReason;
pub use layout::Layout;
pub use line::{GlyphRun, Line, LineMetrics, PositionedInlineBox, PositionedLayoutItem};
pub use line_break::{
    BoxBreakData, BreakLines, BreakerState, LineBreakData, MaxHeightBreakData, YieldData,
};
pub use run::Run;

pub(crate) use data::{LayoutData, LayoutItem, LayoutItemKind, LineData, LineItemData};
pub(crate) use line::LineItem;
pub(crate) use spacing::Spacing;

// TODO - Deprecation not yet active to ease internal code migration.
#[deprecated(since = "TBD", note = "Access from the `editing` module instead.")]
pub use crate::editing::{Cursor, Selection};

// TODO - Move the following to `style` module and submodules.

use crate::style::Brush;
use crate::{LineHeight, OverflowWrap, TextWrapMode, VerticalAlign, WhiteSpaceCollapse};

/// Style properties.
#[derive(Clone, Debug, PartialEq)]
pub struct Style<B: Brush> {
    /// Index of the style of the enclosing span in [`Layout::styles`]. The root style (index 0)
    /// refers to itself.
    pub(crate) parent: u16,
    /// Brush for drawing glyphs.
    pub brush: B,
    /// Underline decoration.
    pub underline: Option<Decoration<B>>,
    /// Strikethrough decoration.
    pub strikethrough: Option<Decoration<B>>,
    /// Partially resolved line height, either in in layout units or dependent on metrics
    pub(crate) line_height: LineHeight,
    /// Vertical alignment of this style's span within the line
    pub(crate) vertical_align: VerticalAlign,
    /// Per-cluster overflow-wrap setting
    pub(crate) overflow_wrap: OverflowWrap,
    /// Per-cluster text-wrap-mode setting
    pub(crate) text_wrap_mode: TextWrapMode,
    /// Per-cluster whitespace collapsing and hanging behavior.
    pub(crate) white_space_collapse: WhiteSpaceCollapse,
    /// The locale of the text, if any.
    ///
    /// This is useful for consumers exposing the text to assistive technologies,
    /// which use it to select an appropriate pronunciation.
    pub locale: Option<fontique::Language>,
}

/// Underline or strikethrough decoration.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration<B: Brush> {
    /// Brush used to draw the decoration.
    pub brush: B,
    /// Offset of the decoration from the baseline. If `None`, use the metrics
    /// of the containing run.
    pub offset: Option<f32>,
    /// Thickness of the decoration. If `None`, use the metrics of the
    /// containing run.
    pub size: Option<f32>,
}

/// Lower and upper bounds on layout width based on its contents.
#[derive(Copy, Clone, Debug)]
pub struct ContentWidths {
    /// The minimum content width. This is the width of the layout if _all_ soft line-breaking
    /// opportunities are taken.
    pub min: f32,
    /// The maximum content width. This is the width of the layout if _no_ soft line-breaking
    /// opportunities are taken.
    pub max: f32,
}

/// Options controlling text-indent behavior, corresponding to CSS `text-indent` keywords.
#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub struct IndentOptions {
    /// If `true`, indent also applies after every hard line break, not just to the first line.
    /// Corresponds to the CSS `each-line` keyword. Defaults to `false`.
    pub each_line: bool,
    /// If `true`, inverts which lines are indented: continuation lines are indented
    /// instead of the first line(s). Corresponds to the CSS `hanging` keyword. Defaults to `false`.
    pub hanging: bool,
}

/// The inherent metrics of a span.
///
/// These metrics are based only on the [first available font](fontique::Query::first_available_font), and don't
/// take into account any fallback fonts (in contrast to, say, [`LineMetrics`]).
///
/// This is currently only available for the root span,
/// as [`Layout::root_span_metrics`](crate::Layout::root_span_metrics).
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SpanMetrics {
    /// Typographic ascent of the span's first available font.
    pub ascent: f32,
    /// Typographic descent of the span's first available font.
    pub descent: f32,
    /// Typographic x-height of the span's first available font.
    pub x_height: f32,
}
