// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::TextStorage;

/// Rich error type for attributed text operations.
///
/// Carries a non-exhaustive [`ErrorKind`] plus contextual information about the
/// attempted range and, when relevant, the enclosing UTF-8 character span at
/// the offending index.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    /// The non-exhaustive category describing this error.
    kind: ErrorKind,

    /// The start byte index of the caller-provided range.
    start: usize,

    /// The end byte index (exclusive) of the caller-provided range.
    end: usize,

    /// The length in bytes of the underlying text at the time of failure.
    len: usize,

    /// Extra detail for boundary-related errors, when available.
    boundary: Option<BoundaryInfo>,
}

#[expect(
    clippy::len_without_is_empty,
    reason = "`Error::len` reports source text length context; an `is_empty` method would be misleading and unused."
)]
impl Error {
    /// The machine-readable category for this error.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The start byte index of the range provided by the caller.
    pub fn start(&self) -> usize {
        self.start
    }

    /// The end byte index of the range provided by the caller.
    pub fn end(&self) -> usize {
        self.end
    }

    /// The length in bytes of the underlying text at the time of the error.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Extra details for boundary-related errors, if available.
    pub fn boundary(&self) -> Option<BoundaryInfo> {
        self.boundary
    }

    pub(crate) fn invalid_bounds(start: usize, end: usize, len: usize) -> Self {
        Self {
            kind: ErrorKind::InvalidBounds,
            start,
            end,
            len,
            boundary: None,
        }
    }

    pub(crate) fn invalid_range(start: usize, end: usize, len: usize) -> Self {
        Self {
            kind: ErrorKind::InvalidRange,
            start,
            end,
            len,
            boundary: None,
        }
    }

    pub(crate) fn not_on_char_boundary<T: TextStorage + ?Sized>(
        text: &T,
        start: usize,
        end: usize,
        len: usize,
        which: Endpoint,
        index: usize,
    ) -> Self {
        let (cs, ce) = enclosing_char_span(text, index).unwrap_or((index, index));
        Self {
            kind: ErrorKind::NotOnCharBoundary,
            start,
            end,
            len,
            boundary: Some(BoundaryInfo {
                which,
                index,
                char_start: cs,
                char_end: ce,
            }),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.kind {
            ErrorKind::InvalidBounds => write!(
                f,
                "range {}..{} out of bounds for len {}",
                self.start, self.end, self.len
            ),
            ErrorKind::InvalidRange => {
                write!(f, "invalid range {}..{}: start > end", self.start, self.end)
            }
            ErrorKind::NotOnCharBoundary => {
                if let Some(b) = self.boundary {
                    let which = match b.which {
                        Endpoint::Start => "start",
                        Endpoint::End => "end",
                    };
                    write!(
                        f,
                        "range {}..{}: {} index {} not on UTF-8 boundary (char {}..{})",
                        self.start, self.end, which, b.index, b.char_start, b.char_end
                    )
                } else {
                    write!(
                        f,
                        "range {}..{} not on UTF-8 boundary",
                        self.start, self.end
                    )
                }
            }
        }
    }
}

impl core::error::Error for Error {}

/// The non-exhaustive category of an error.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Provided range indices were out of bounds relative to the text length.
    InvalidBounds,

    /// The provided range had `start > end`.
    InvalidRange,

    /// Either `start` or `end` was not aligned to a UTF-8 character boundary.
    NotOnCharBoundary,
}

/// Identifies which endpoint of a range failed boundary validation.
///
/// This type is surfaced via [`BoundaryInfo`], which is attached to [`Error`]
/// for boundary-related failures.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// The `start` endpoint of the range.
    Start,

    /// The `end` endpoint of the range.
    End,
}

/// Details about an offending index that was not on a UTF-8 character boundary.
///
/// Returned by [`Error::boundary`] when the error kind is
/// [`ErrorKind::NotOnCharBoundary`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BoundaryInfo {
    /// Which endpoint (`start` or `end`) was invalid.
    pub which: Endpoint,

    /// The offending byte index.
    pub index: usize,

    /// The start byte index of the enclosing UTF-8 codepoint.
    pub char_start: usize,

    /// The end byte index (exclusive) of the enclosing UTF-8 codepoint.
    pub char_end: usize,
}

fn enclosing_char_span<T: TextStorage + ?Sized>(text: &T, index: usize) -> Option<(usize, usize)> {
    let len = text.len();
    if index > len {
        return None;
    }
    if text.is_char_boundary(index) {
        return Some((index, index));
    }

    // Previous boundary (max 3 bytes back)
    let mut s = index;
    for _ in 0..4 {
        // We can never wrap around `0` here, as when `s == 0` before the decrement,
        // the previous call to `text.is_char_boundary` (either in this loop or above)
        // will have returned `true`.
        if s == 0 {
            unreachable!("`s` should never reach 0 before finding a char boundary");
        }
        s -= 1;
        if text.is_char_boundary(s) {
            break;
        }
    }

    // Next boundary (max 3 bytes forward)
    let mut e = index;
    for _ in 0..4 {
        if e >= len {
            break;
        }
        e += 1;
        if text.is_char_boundary(e) {
            break;
        }
    }

    if s <= e { Some((s, e)) } else { None }
}
