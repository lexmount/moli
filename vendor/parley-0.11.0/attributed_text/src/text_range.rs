// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::ops::Range;

use crate::{Endpoint, Error, TextStorage};

/// A validated byte range into a UTF-8 text buffer.
///
/// This is a convenience wrapper around `Range<usize>` that carries invariants useful for
/// attributed text APIs:
///
/// - `start <= end`
/// - `start` and `end` are within the text bounds
/// - `start` and `end` lie on UTF-8 codepoint boundaries
///
/// **Why `TextRange`?**
///
/// Many text APIs accept `Range<usize>` byte offsets. That is flexible, but it means every call
/// must re-check bounds and UTF-8 boundary alignment, and every caller has to decide how to handle
/// failures.
///
/// `TextRange` lets you validate once and then pass the range to APIs that can be infallible with
/// respect to range correctness.
///
/// ## Important
///
/// `TextRange` does not currently encode which specific text buffer it was validated against. It
/// is the caller's responsibility to only reuse a `TextRange` with the same underlying text
/// content it was validated for.
///
/// ## Example
///
/// ```
/// use attributed_text::{AttributedText, TextRange};
///
/// let mut text = AttributedText::new("Hello!");
/// let range = TextRange::new(text.text(), 0..5).unwrap();
/// text.apply_attribute(range, ());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextRange {
    start: usize,
    end: usize,
}

impl TextRange {
    /// Returns a validated `TextRange` for the provided text.
    #[inline]
    pub fn new<T: TextStorage + ?Sized>(text: &T, range: Range<usize>) -> Result<Self, Error> {
        validate_range(text, &range)?;
        Ok(Self {
            start: range.start,
            end: range.end,
        })
    }

    /// Creates a `TextRange` without validation.
    ///
    /// This is intended for internal callers that already maintain range invariants.
    #[must_use]
    #[inline]
    pub const fn new_unchecked(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The start byte offset.
    #[must_use]
    #[inline]
    pub const fn start(self) -> usize {
        self.start
    }

    /// The end byte offset (exclusive).
    #[must_use]
    #[inline]
    pub const fn end(self) -> usize {
        self.end
    }

    /// Returns the length of this range in bytes.
    #[must_use]
    #[inline]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    /// Returns `true` if this range contains no bytes.
    #[must_use]
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns `true` if `index` is contained in this range.
    #[must_use]
    #[inline]
    pub const fn contains(self, index: usize) -> bool {
        self.start <= index && index < self.end
    }

    /// Returns `true` if this range and `other` overlap.
    #[must_use]
    #[inline]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && self.end > other.start
    }

    /// Returns this range as a `Range<usize>`.
    #[must_use]
    #[inline]
    pub fn as_range(self) -> Range<usize> {
        self.start..self.end
    }
}

impl From<TextRange> for Range<usize> {
    #[inline]
    fn from(value: TextRange) -> Self {
        value.as_range()
    }
}

#[inline]
pub(crate) fn validate_range<T: TextStorage + ?Sized>(
    text: &T,
    range: &Range<usize>,
) -> Result<(), Error> {
    let text_len = text.len();
    if range.start > range.end {
        return Err(Error::invalid_range(range.start, range.end, text_len));
    }
    if range.start > text_len || range.end > text_len {
        return Err(Error::invalid_bounds(range.start, range.end, text_len));
    }
    if !text.is_char_boundary(range.start) {
        return Err(Error::not_on_char_boundary(
            text,
            range.start,
            range.end,
            text_len,
            Endpoint::Start,
            range.start,
        ));
    }
    if !text.is_char_boundary(range.end) {
        return Err(Error::not_on_char_boundary(
            text,
            range.start,
            range.end,
            text_len,
            Endpoint::End,
            range.end,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TextRange, validate_range};
    use crate::{Endpoint, ErrorKind};

    #[test]
    fn validates_ok_ranges() {
        let t = "Hello!";
        assert!(validate_range(&t, &(0..0)).is_ok());
        assert!(validate_range(&t, &(0..6)).is_ok());
        assert!(TextRange::new(&t, 1..3).is_ok());
    }

    #[test]
    #[expect(
        clippy::reversed_empty_ranges,
        reason = "We want an invalid range for testing."
    )]
    fn rejects_start_greater_than_end() {
        let t = "Hello!";
        let err = TextRange::new(&t, 4..3).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidRange);
        assert_eq!(err.start(), 4);
        assert_eq!(err.end(), 3);
        assert_eq!(err.len(), 6);
    }

    #[test]
    fn rejects_out_of_bounds() {
        let t = "Hello!";
        let err = TextRange::new(&t, 0..7).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidBounds);
        assert_eq!(err.start(), 0);
        assert_eq!(err.end(), 7);
        assert_eq!(err.len(), 6);
    }

    #[test]
    fn rejects_not_on_char_boundary_start() {
        // "é" is 2 bytes in UTF-8; index 1 is not a boundary.
        let t = "éclair";
        let err = TextRange::new(&t, 1..2).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotOnCharBoundary);
        let b = err.boundary().expect("boundary info");
        assert_eq!(b.which, Endpoint::Start);
        assert_eq!(b.index, 1);
        assert_eq!(b.char_start, 0);
        assert_eq!(b.char_end, 2);
    }

    #[test]
    fn rejects_not_on_char_boundary_end() {
        // "é" is 2 bytes in UTF-8; index 1 is not a boundary.
        let t = "éclair";
        let err = TextRange::new(&t, 0..1).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotOnCharBoundary);
        let b = err.boundary().expect("boundary info");
        assert_eq!(b.which, Endpoint::End);
        assert_eq!(b.index, 1);
        assert_eq!(b.char_start, 0);
        assert_eq!(b.char_end, 2);
    }
}
