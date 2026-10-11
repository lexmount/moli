// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fundamental text property types.
//!
//! This crate is intended as a lightweight, `no_std`-friendly vocabulary layer that can be shared
//! across style systems, text layout engines, and font tooling. It focuses on small, typed
//! representations of common “leaf” concepts (weights, widths, OpenType tags, language tags, etc).
//!
//! ## Features
//!
//! - `std` (enabled by default): This is currently unused and is provided for forward compatibility.
//! - `bytemuck`: Implement traits from `bytemuck` on [`GenericFamily`], [`BidiLevel`] and
//!   [`NormalizedCoord`].
//!
//! ## Example
//!
//! ```
//! use parlance::{Language, Tag};
//!
//! let tag = Tag::parse("wght").unwrap();
//! assert_eq!(tag.to_bytes(), *b"wght");
//!
//! let lang = Language::parse("zh-Hans-CN").unwrap();
//! assert_eq!(lang.as_str(), "zh-Hans-CN");
//! assert_eq!(lang.language(), "zh");
//! assert_eq!(lang.script(), Some("Hans"));
//! assert_eq!(lang.region(), Some("CN"));
//! ```
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

mod bidi;
mod font;
mod font_family;
mod generic_family;
#[cfg(feature = "bytemuck")]
mod impl_bytemuck;
mod language;
mod normalized_coord;
mod script;
mod tag;
mod text;

pub use bidi::{BidiControl, BidiDirection, BidiLevel, BidiOverride};
pub use font::{FontStyle, FontWeight, FontWidth};
pub use font_family::{FontFamily, FontFamilyName, ParseFontFamilyError, ParseFontFamilyErrorKind};
pub use generic_family::GenericFamily;
pub use language::{Language, ParseLanguageError};
pub use normalized_coord::NormalizedCoord;
pub use script::{ParseScriptError, Script};
pub use tag::{FontFeature, FontVariation, ParseSettingsError, ParseSettingsErrorKind, Tag};
pub use text::{BaseDirection, LineBreak, OverflowWrap, TextWrapMode, WordBreak};
