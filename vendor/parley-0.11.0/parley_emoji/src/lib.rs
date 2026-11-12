// Copyright 2026 Christian Hansen and the Parley Authors
// SPDX-License-Identifier: MIT

// Adapted from <https://github.com/chansen/c-emoji>

// After you edit the crate's doc comment, run this command, then check README.md for any missing links
// cargo rdme --workspace-project=parley_emoji

//! Emoji presentation resolution for text layout, translated from Christian Hansen's [c-emoji].
//!
//! Some Unicode characters and sequences can be displayed either as ordinary
//! text glyphs or as emoji. This crate determines the preferred presentation
//! of a grapheme cluster from its Unicode emoji properties, variation selectors,
//! and sequence structure. The implementation recognizes the emoji
//! sequences defined by [Unicode Technical Standard #51][UTS51].
//!
//! WARNING: This crate is currently designed only for use within Parley;
//! if you have a use case for it, please
//! [reach out](https://xi.zulipchat.com/#narrow/channel/205635-parley).
//! This crate exists entirely because the code it adapts doesn't match
//! Parley's existing license, but is otherwise currently treated as an
//! internal implementation detail of Parley.
//!
//! In Parley, this impacts font selection for these clusters.
//!
//! # Features
//!
//! The following crate [feature flags](https://doc.rust-lang.org/cargo/reference/features.html#dependency-features) are available:
//!
//! - `std` (enabled by default): This is currently unused and is provided for forward compatibility.
//!
//! Note that Parley Emoji currently requires that an allocator is available (i.e. it depends on [`alloc`][_alloc]).
//! Currently, Parley Emoji does not actually allocate, but the dependency is kept so starting to allocate
//! in the future is not a breaking change.
//!
//! [c-emoji]: <https://github.com/chansen/c-emoji>
//! [UTS51]: <https://www.unicode.org/reports/tr51/>

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

// Avoid adding alloc in the future being a breaking change.
extern crate alloc as _alloc;
// Ensure that we don't compile if you're using the std feature on a platform without `std`
#[cfg(feature = "std")]
extern crate std as _;

mod dfa;
mod types;

pub use dfa::EmojiDFA;
pub use types::{EmojiPresentationStyle, EmojiSegmentationCategory};
