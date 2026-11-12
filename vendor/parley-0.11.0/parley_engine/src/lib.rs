// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parley Engine provides low level APIs for shaping paragraphs of text.
//!
//! ## Usage
//!
//! Use [`Analyzer`], [`Analysis`], [`Shaper`] and [`ShapedText`] to shape a paragraph of text into
//! glyphs. Correct reshaping of lines is in progress; in the meantime you can break text at
//! [`Atom`] or [`ShapedCluster`][crate::shape::ShapedCluster] boundaries.
//!
//! Text analysis is performed before shaping, and the same source string must be passed to each
//! stage.
//!
//! Higher-level users may prefer using [`parley`][parley], which uses this crate and implements
//! layout and styling.
//!
//! ```rust,no_run
//! # // We only compile this doctest because we don't have a font available.
//! # use parley_engine::{Analysis, AnalysisOptions, Analyzer, FontInstanceRef, FontSelector, ShapedText, ShapeOptions, Shaper};
//! # use parley_engine::shape::CharCluster;
//! # use parley_engine::itemize::{Item, Segment};
//! #
//! # struct NoFont;
//! # impl FontSelector for NoFont {
//! #     fn select_font(
//! #         &mut self,
//! #         _segment: &Segment,
//! #         _options: &ShapeOptions<'_>,
//! #         _cluster: &mut CharCluster,
//! #     ) -> Option<FontInstanceRef<'_>> {
//! #         unimplemented!()
//! #     }
//! # }
//! #
//! # let select_font = NoFont;
//! let mut analysis = Analysis::default();
//! let mut analyzer = Analyzer::default();
//! let mut shaped_text = ShapedText::default();
//! let mut shaper = Shaper::default();
//!
//! let text = "The quick brown ثعلب jumps over the lazy dog.";
//! let char_count = text.chars().count();
//! let char_style_indices = vec![0; char_count];
//!
//! analyzer.analyze(text, &AnalysisOptions::default(), &mut analysis);
//! shaper.shape_text(
//!     text,
//!     &analysis,
//!     &char_style_indices,
//!     [Item {
//!         char_end: char_count.try_into().unwrap(),
//!         options: ShapeOptions {
//!             font_size: 16.0,
//!             language: None,
//!             features: &[],
//!             variations: &[],
//!         },
//!      }],
//!      select_font, // Selects fonts covering each cluster.
//!      &mut shaped_text,
//! );
//!
//! for (run_idx, run) in shaped_text.runs().iter().enumerate() {
//!     let slice = shaped_text.run_slice(run_idx as u32);
//!     // You can, for example, measure grapheme advances for hit-testing or
//!     // placing carets.
//!     for atom in slice.atoms_start() {
//!         for grapheme in atom.graphemes_start() {
//!             std::dbg!(grapheme);
//!         }
//!     }
//!
//!     // Or get glyphs for rendering (for simplicity, this iterates clusters in
//!     // logical order, but for rendering you'd want to reorder runs and clusters
//!     // according to their `run.bidi_level`).
//!     for cluster in slice.shaped_clusters_range() {
//!         for glyph in slice.shaped_cluster_glyphs(cluster) {
//!             std::dbg!(glyph);
//!         }
//!     }
//! }
//! ```
//!
//! ## Features
//!
//! - `std` (enabled by default): This is currently unused and is provided for forward compatibility.
//! - `bytemuck`: Forwarded to [`parlance`], which then implements traits from `bytemuck` on its
//!   types.
//!
//! [parley]: https://docs.rs/parley

// LINEBENDER LINT SET - lib.rs - v3
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
#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

mod analysis;
mod analyzer;
pub mod bidi;
pub mod break_overrides;
mod glyph;
pub mod itemize;
mod lru_cache;
pub mod shape;
#[cfg(test)]
mod test_inline_bidi;

pub use linebender_resource_handle::FontData;
pub use parlance::{BaseDirection, NormalizedCoord};

pub use analysis::{Analysis, AnalysisDataSources, CharInfo};
pub use analyzer::{AnalysisOptions, Analyzer, BidiObject, InlineBoxBidi, LineBreakConfig};
pub use glyph::Glyph;
pub use shape::atom::{Atom, Atoms, Grapheme, Graphemes, ShapedClusterGlyphs, ShapedSlice};
pub use shape::shaped_text::{FontMetrics, ShapedRun, ShapedText};
pub use shape::shaper::{FontInstance, FontInstanceRef, FontSelector, ShapeOptions, Shaper};
