// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::string::String;
use alloc::sync::Arc;
use core::ops::Range;

use crate::{Error, TextRange};

/// A borrowed contiguous chunk of text from a [`TextStorage`].
///
/// The [`TextRange`] is expressed in the storage's global byte coordinate space, while
/// [`Self::text`] returns the borrowed UTF-8 text for just this chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextChunk<'a> {
    range: TextRange,
    text: &'a str,
}

impl<'a> TextChunk<'a> {
    /// Creates a text chunk for `range`.
    ///
    /// # Panics
    ///
    /// Panics if the byte length of `text` does not match `range`.
    #[must_use]
    pub fn new(range: TextRange, text: &'a str) -> Self {
        assert_eq!(
            range.len(),
            text.len(),
            "text chunk length must match its range"
        );
        Self { range, text }
    }

    /// Returns this chunk's range in the storage's global byte coordinate space.
    #[must_use]
    #[inline]
    pub const fn range(self) -> TextRange {
        self.range
    }

    /// Returns this chunk's borrowed UTF-8 text.
    #[must_use]
    #[inline]
    pub const fn text(self) -> &'a str {
        self.text
    }
}

/// A block of text that will be wrapped by an [`AttributedText`].
///
/// [`AttributedText`]: crate::AttributedText
pub trait TextStorage {
    /// The length of the underlying text.
    fn len(&self) -> usize;

    /// Return `true` if the underlying text is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return whether `index` is a UTF-8 character boundary in the text.
    ///
    /// Implementors may compute this by any means appropriate for the
    /// underlying representation (e.g. [`str::is_char_boundary`] for contiguous
    /// strings, or by inspecting rope chunk boundaries).
    fn is_char_boundary(&self, index: usize) -> bool;

    /// Returns this storage as a contiguous string slice when available.
    ///
    /// Rope-like and sparse storage implementations should return `None`.
    ///
    /// For non-contiguous storage or selecting a text range, see [`TextStorage::chunks`].
    fn as_str(&self) -> Option<&str> {
        None
    }

    /// Returns a validated [`TextRange`] for this storage.
    ///
    /// This is equivalent to calling [`TextRange::new`] with `self`.
    #[inline]
    fn validate_range(&self, range: Range<usize>) -> Result<TextRange, Error> {
        TextRange::new(self, range)
    }

    /// Iterates over contiguous, non-empty chunks of text covering `range`.
    ///
    /// The provided range must have been validated against this storage. Implementations should
    /// yield chunks in order, without gaps or overlaps, and should yield no chunks for an empty
    /// range.
    ///
    /// For contiguous storage implementations, you can use [`TextStorage::as_str`] to get the
    /// entire underlying string slice.
    fn chunks(&self, range: TextRange) -> impl Iterator<Item = TextChunk<'_>>;
}

impl TextStorage for String {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn is_char_boundary(&self, index: usize) -> bool {
        self.as_str().is_char_boundary(index)
    }

    fn as_str(&self) -> Option<&str> {
        Some(self.as_ref())
    }

    fn chunks(&self, range: TextRange) -> impl Iterator<Item = TextChunk<'_>> {
        contiguous_chunks(self.as_ref(), range)
    }
}

impl TextStorage for str {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn is_char_boundary(&self, index: usize) -> bool {
        Self::is_char_boundary(self, index)
    }

    fn as_str(&self) -> Option<&str> {
        Some(self)
    }

    fn chunks(&self, range: TextRange) -> impl Iterator<Item = TextChunk<'_>> {
        contiguous_chunks(self, range)
    }
}

impl TextStorage for &str {
    fn len(&self) -> usize {
        (*self).len()
    }

    fn is_char_boundary(&self, index: usize) -> bool {
        (*self).is_char_boundary(index)
    }

    fn as_str(&self) -> Option<&str> {
        Some(*self)
    }

    fn chunks(&self, range: TextRange) -> impl Iterator<Item = TextChunk<'_>> {
        contiguous_chunks(self, range)
    }
}

impl TextStorage for Arc<str> {
    fn len(&self) -> usize {
        str::len(self)
    }

    fn is_char_boundary(&self, index: usize) -> bool {
        str::is_char_boundary(self, index)
    }

    fn as_str(&self) -> Option<&str> {
        Some(self.as_ref())
    }

    fn chunks(&self, range: TextRange) -> impl Iterator<Item = TextChunk<'_>> {
        contiguous_chunks(self.as_ref(), range)
    }
}

fn contiguous_chunks(text: &str, range: TextRange) -> impl Iterator<Item = TextChunk<'_>> {
    (!range.is_empty())
        .then(|| TextChunk::new(range, &text[range.as_range()]))
        .into_iter()
}

#[cfg(test)]
mod tests {
    use super::{TextChunk, TextStorage};
    use alloc::string::ToString;
    use alloc::sync::Arc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::ops::Range;

    use crate::TextRange;

    #[derive(Debug)]
    struct ChunkedText {
        chunks: Vec<&'static str>,
        len: usize,
    }

    impl ChunkedText {
        fn new(chunks: &[&'static str]) -> Self {
            let chunks = chunks.to_vec();
            let len = chunks.iter().map(|chunk| chunk.len()).sum();
            Self { chunks, len }
        }
    }

    impl TextStorage for ChunkedText {
        fn len(&self) -> usize {
            self.len
        }

        fn is_char_boundary(&self, index: usize) -> bool {
            if index > self.len {
                return false;
            }

            let mut chunk_start = 0;
            for chunk in &self.chunks {
                let chunk_end = chunk_start + chunk.len();
                if index < chunk_end {
                    return chunk.is_char_boundary(index - chunk_start);
                }
                if index == chunk_end {
                    return true;
                }
                chunk_start = chunk_end;
            }

            index == self.len
        }

        fn chunks(&self, range: TextRange) -> impl Iterator<Item = TextChunk<'_>> {
            let mut chunk_start = 0;
            self.chunks.iter().filter_map(move |&chunk| {
                let offset = chunk_start;
                let start = range.start().max(chunk_start);
                chunk_start += chunk.len();
                let end = range.end().min(chunk_start);
                (start < end).then(|| {
                    let text = &chunk[start - offset..end - offset];
                    TextChunk::new(TextRange::new_unchecked(start, end), text)
                })
            })
        }
    }

    fn assert_boundaries<T: TextStorage>(t: &T, trues: &[usize], falses: &[usize]) {
        for &i in trues {
            assert!(t.is_char_boundary(i), "index {i} should be boundary");
        }
        for &i in falses {
            assert!(!t.is_char_boundary(i), "index {i} should not be boundary");
        }
    }

    fn collect_chunks<T: TextStorage + ?Sized>(
        text: &T,
        range: Range<usize>,
    ) -> Vec<(Range<usize>, &str)> {
        let range = TextRange::new(text, range).expect("valid range");
        text.chunks(range)
            .map(|chunk| (chunk.range().as_range(), chunk.text()))
            .collect()
    }

    #[test]
    fn is_char_boundary_ascii() {
        let s = "abc";
        // All byte positions 0..=len are char boundaries in pure ASCII
        assert_boundaries(&s, &[0, 1, 2, 3], &[4]);
    }

    #[test]
    fn is_char_boundary_multibyte() {
        let s = "éclair"; // first codepoint is 2 bytes
        assert_boundaries(&s, &[0, 2, s.len()], &[1]);

        let owned = s.to_string();
        assert_boundaries(&owned, &[0, 2, owned.len()], &[1]);

        let arc: Arc<str> = Arc::from(s);
        assert_boundaries(&arc, &[0, 2, arc.len()], &[1]);
    }

    #[test]
    fn is_char_boundary_emoji_flag() {
        let s = "🇯🇵"; // two 4-byte codepoints
        assert_eq!(s.len(), 8);
        // Boundaries at 0, 4, 8
        assert_boundaries(&s, &[0, 4, 8], &[1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn validates_range_from_str_directly() {
        let s = "éclair";
        let range = TextRange::new(s, 0..2).unwrap();
        assert_eq!(range.as_range(), 0..2);

        let range = s.validate_range(2..s.len()).unwrap();
        assert_eq!(range.as_range(), 2..s.len());
    }

    #[test]
    fn contiguous_storage_has_fast_path() {
        let borrowed = "hello";
        assert_eq!(TextStorage::as_str(&borrowed), Some("hello"));

        let owned = borrowed.to_string();
        assert_eq!(TextStorage::as_str(&owned), Some("hello"));

        let arc: Arc<str> = Arc::from(borrowed);
        assert_eq!(TextStorage::as_str(&arc), Some("hello"));
    }

    #[test]
    fn contiguous_chunks_cover_full_range() {
        let s = "abc";
        assert_eq!(collect_chunks(s, 0..3), vec![(0..3, "abc")]);
    }

    #[test]
    fn contiguous_chunks_cover_multibyte_subrange() {
        let s = "aé日z";
        assert_eq!(collect_chunks(s, 1..6), vec![(1..6, "é日")]);
    }

    #[test]
    fn empty_range_yields_no_chunks() {
        let s = "abc";
        assert!(collect_chunks(s, 1..1).is_empty());
    }

    #[test]
    fn chunked_storage_boundaries() {
        let text = ChunkedText::new(&["ab", "é", "日z"]);
        assert_boundaries(&text, &[0, 1, 2, 4, 7, 8], &[3, 5, 6, 9]);
    }

    #[test]
    fn chunked_storage_has_no_contiguous_fast_path() {
        let text = ChunkedText::new(&["ab", "é", "日z"]);
        assert_eq!(text.as_str(), None);
    }

    #[test]
    fn chunked_chunks_cover_single_storage_chunk() {
        let text = ChunkedText::new(&["ab", "é", "日z"]);
        assert_eq!(collect_chunks(&text, 4..7), vec![(4..7, "日")]);
    }

    #[test]
    fn chunked_chunks_cover_multiple_storage_chunks() {
        let text = ChunkedText::new(&["ab", "é", "日z"]);
        assert_eq!(
            collect_chunks(&text, 1..7),
            vec![(1..2, "b"), (2..4, "é"), (4..7, "日")]
        );
    }

    #[test]
    fn chunked_storage_rejects_endpoint_inside_multibyte_scalar() {
        let text = ChunkedText::new(&["ab", "é", "日z"]);
        assert!(TextRange::new(&text, 3..4).is_err());
        assert!(TextRange::new(&text, 4..6).is_err());
    }
}
