//! Shared tokenization primitives for structured HTTP header field values.
//!
//! The observable behavior follows Blink's `HeaderFieldTokenizer` normal and
//! relaxed modes. Higher-level crates remain responsible for deciding which
//! tokens, separators, and parameters are valid for a particular header.
//!
//! One compatibility detail is intentionally non-standard: Blink currently
//! accepts DEL (`0x7f`) as a token character even though MIME token grammar
//! classifies it as a control. This crate preserves that behavior. Relaxed mode
//! also admits MIME `tspecials` other than space, semicolon, and quote, matching
//! Blink rather than defining another standards-compliance mode.

mod parameters;
mod tokenizer;

pub use parameters::{split_outside_quoted_strings, unquote_parameter_value};
pub use tokenizer::{HeaderFieldTokenMode, HeaderFieldTokenizer};

#[cfg(test)]
mod tests;
