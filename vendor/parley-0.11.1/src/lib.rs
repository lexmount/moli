// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parley is a library for rich text layout.
//!
//! Some key types are:
//! - [`FontContext`] and [`LayoutContext`] are resources which should be shared globally (or at coarse-grained boundaries).
//!   - [`FontContext`] is database of fonts.
//!   - [`LayoutContext`] is scratch space that allows for reuse of allocations between layouts.
//! - Builders for creating a [`Layout`]:
//!   - [`RangedBuilder`]: styles specified as a flat `Vec` of property spans
//!   - [`TreeBuilder`]: styles specified as a tree of spans
//!   - [`StyleRunBuilder`]: styles specified as a shared style table plus non-overlapping indexed runs
//!
//!   They are constructed using [`LayoutContext::ranged_builder`], [`LayoutContext::tree_builder`],
//!   and [`LayoutContext::style_run_builder`].
//! - [`Layout`] which represents styled paragraph(s) of text and can perform shaping, line-breaking, bidi-reordering, and alignment of that text.
//!
//!   `Layout` supports re-linebreaking and re-aligning many times (in case the width at which wrapping should occur changes). But if the text content or
//!   the styles applied to that content change then a new `Layout` must be created using a new
//!   `RangedBuilder`, `TreeBuilder`, or `StyleRunBuilder`.
//!
//! ## Usage Example
//!
//! See the [examples](https://github.com/linebender/parley/tree/main/examples) directory for more complete usage examples that include rendering.
//!
//! ```rust
//! use parley::{
//!    Alignment, AlignmentOptions, FontContext, FontWeight, InlineBox, InlineBoxKind, Layout,
//!    LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
//! };
//!
//! // Create a FontContext (font database) and LayoutContext (scratch space).
//! // These are both intended to be constructed rarely (perhaps even once per app):
//! let mut font_cx = FontContext::new();
//! let mut layout_cx = LayoutContext::new();
//!
//! // Create a `RangedBuilder` or a `TreeBuilder`, which are used to construct a `Layout`.
//! const DISPLAY_SCALE : f32 = 1.0;
//! const TEXT : &str = "Lorem Ipsum...";
//! let mut builder = layout_cx.ranged_builder(&mut font_cx, &TEXT, DISPLAY_SCALE, true);
//!
//! // Set default styles that apply to the entire layout
//! builder.push_default(StyleProperty::FontSize(16.0));
//!
//! // Set a style that applies to the first 4 characters
//! builder.push(StyleProperty::FontWeight(FontWeight::new(600.0)), 0..4);
//!
//! // Add a box to be laid out inline with the text
//! builder.push_inline_box(InlineBox { id: 0, kind: InlineBoxKind::InFlow, index: 5, width: 50.0, height: 50.0 });
//!
//! // Build the builder into a Layout
//! let mut layout: Layout<()> = builder.build(&TEXT);
//!
//! // Run line-breaking and alignment on the Layout
//! const MAX_WIDTH : Option<f32> = Some(100.0);
//! layout.break_all_lines(MAX_WIDTH);
//! layout.align(Alignment::Start, AlignmentOptions::default());
//!
//! // Inspect computed layout (see examples for more details)
//! let width = layout.width();
//! let height = layout.height();
//! for line in layout.lines() {
//!     for item in line.items() {
//!         match item {
//!             PositionedLayoutItem::GlyphRun(glyph_run) => {
//!                 // Render the glyph run
//!             }
//!             PositionedLayoutItem::InlineBox(inline_box) => {
//!                 // Render the inline box
//!             }
//!         };
//!     }
//! }
//! ```

// LINEBENDER LINT SET - lib.rs - v4
// See https://linebender.org/wiki/canonical-lints/
// These lints shouldn't apply to examples or tests.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
// These lints shouldn't apply to examples.
#![warn(clippy::print_stdout, clippy::print_stderr)]
// Targeting e.g. 32-bit means structs containing usize can give false positives for 64-bit.
#![cfg_attr(target_pointer_width = "64", warn(clippy::trivially_copy_pass_by_ref))]
// END LINEBENDER LINT SET
#![cfg_attr(docsrs, feature(doc_cfg))]
#![no_std]
#![allow(missing_docs, reason = "We have many as-yet undocumented items.")]
#![expect(
    missing_debug_implementations,
    clippy::allow_attributes_without_reason,
    clippy::cast_possible_truncation,
    clippy::missing_assert_message,
    reason = "Deferred"
)]
#![expect(
    single_use_lifetimes,
    reason = "False positive: https://github.com/rust-lang/rust/issues/129255"
)]

#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("parley requires either the `std` or `libm` feature to be enabled");

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub use fontique;

mod analysis;
mod bidi;
mod break_overrides;
mod builder;
mod context;
mod convert;
mod font;
mod inline_box;
mod lru_cache;
mod resolve;
mod shape;
mod util;

pub mod editing;
pub mod layout;
pub mod setting;
pub mod style;

#[cfg(test)]
mod tests;

pub use linebender_resource_handle::FontData;
pub use util::BoundingBox;

pub use break_overrides::{
    AsciiLineBreakTable, AsciiLineBreakTableBuilder, CHROMIUM_LINE_BREAK_OVERRIDE,
    LineBreakContext, LineBreakOverrideFn,
};
pub use builder::{RangedBuilder, StyleRunBuilder, TreeBuilder};
pub use context::LayoutContext;
pub use font::FontContext;
pub use inline_box::{InlineBox, InlineBoxKind};
#[doc(inline)]
pub use layout::Layout;

pub use editing::*;
pub use layout::*;
pub use style::*;

#[deprecated(
    note = "Old name for this type, use `parley::FontData` instead.",
    since = "0.6.0"
)]
pub type Font = FontData;
