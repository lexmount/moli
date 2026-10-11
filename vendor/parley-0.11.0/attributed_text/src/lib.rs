// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Attributed Text is a Rust crate which ...
//!
//! ## Features
//!
//! - `std` (enabled by default): This is currently unused and is provided for forward compatibility.
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

extern crate alloc;

mod attribute_segments;
mod attributed_text;
mod error;
mod text_range;
mod text_storage;

pub use crate::attribute_segments::{
    ActiveSpans, ActiveSpansIter, AttributeSegment, AttributeSegments, AttributeSegmentsWorkspace,
};
pub use crate::attributed_text::AttributedText;
pub use crate::error::{BoundaryInfo, Endpoint, Error, ErrorKind};
pub use crate::text_range::TextRange;
pub use crate::text_storage::{TextChunk, TextStorage};
