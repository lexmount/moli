// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! CSS font-family parsing and representation.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use crate::GenericFamily;

/// Kinds of errors that can occur when parsing CSS `font-family` strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseFontFamilyErrorKind {
    /// The source string does not conform to the supported syntax.
    InvalidSyntax,
    /// A quoted family name was missing a closing quote.
    UnterminatedString,
}

/// Error returned when parsing CSS `font-family` strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseFontFamilyError {
    kind: ParseFontFamilyErrorKind,
    at: usize,
    span: Option<(usize, usize)>,
}

impl ParseFontFamilyError {
    const fn new(kind: ParseFontFamilyErrorKind, at: usize) -> Self {
        Self {
            kind,
            at,
            span: None,
        }
    }

    const fn with_span(mut self, span: (usize, usize)) -> Self {
        self.span = Some(span);
        self
    }

    /// Returns the error kind.
    pub const fn kind(self) -> ParseFontFamilyErrorKind {
        self.kind
    }

    /// Returns the byte offset into the source where the error was detected.
    pub const fn byte_offset(self) -> usize {
        self.at
    }

    /// Returns the byte span (start, end) for the token associated with this error, if available.
    pub const fn byte_span(self) -> Option<(usize, usize)> {
        self.span
    }
}

impl fmt::Display for ParseFontFamilyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self.kind {
            ParseFontFamilyErrorKind::InvalidSyntax => "invalid font-family syntax",
            ParseFontFamilyErrorKind::UnterminatedString => "unterminated string in font-family",
        };
        write!(f, "{msg} at byte {}", self.at)
    }
}

impl core::error::Error for ParseFontFamilyError {}

/// A single named or generic font family.
///
/// This corresponds to one entry in a CSS `font-family` list.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/font-family>
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FontFamilyName<'a> {
    /// A named font family.
    Named(Cow<'a, str>),
    /// A generic font family.
    Generic(GenericFamily),
}

impl<'a> FontFamilyName<'a> {
    /// Creates a named font family from a borrowed string.
    pub const fn named(name: &'a str) -> Self {
        Self::Named(Cow::Borrowed(name))
    }

    /// Parses a font family containing a name or a generic family.
    ///
    /// # Example
    /// ```
    /// # extern crate alloc;
    /// use alloc::borrow::Cow;
    /// use parlance::FontFamilyName::{self, *};
    /// use parlance::GenericFamily::*;
    ///
    /// assert_eq!(FontFamilyName::parse("Palatino Linotype"), Some(Named(Cow::Borrowed("Palatino Linotype"))));
    /// assert_eq!(FontFamilyName::parse("monospace"), Some(Generic(Monospace)));
    ///
    /// // Note that you can quote a generic family to capture it as a named family:
    /// assert_eq!(FontFamilyName::parse("'monospace'"), Some(Named(Cow::Borrowed("monospace"))));
    /// ```
    pub fn parse(s: &'a str) -> Option<Self> {
        Self::parse_css_list(s).next()?.ok()
    }

    /// Parses a comma separated list of font families.
    ///
    /// Whitespace is ignored and a trailing comma is permitted, but empty entries (such as `,,`)
    /// are rejected.
    ///
    /// # Example
    /// ```
    /// # extern crate alloc;
    /// use alloc::borrow::Cow;
    /// use alloc::vec::Vec;
    /// use parlance::FontFamilyName::{self, *};
    /// use parlance::ParseFontFamilyError;
    /// use parlance::GenericFamily::*;
    ///
    /// let source = "Arial, 'Times New Roman', serif";
    ///
    /// let parsed_families: Result<Vec<_>, ParseFontFamilyError> =
    ///     FontFamilyName::parse_css_list(source).collect();
    /// let families = [
    ///     Named(Cow::Borrowed("Arial")),
    ///     Named(Cow::Borrowed("Times New Roman")),
    ///     Generic(Serif),
    /// ];
    ///
    /// assert_eq!(parsed_families.unwrap().as_slice(), &families);
    /// ```
    pub fn parse_css_list(
        s: &'a str,
    ) -> impl Iterator<Item = Result<FontFamilyName<'a>, ParseFontFamilyError>> + 'a + Clone {
        ParseCssList {
            source: s.as_bytes(),
            len: s.len(),
            pos: 0,
            done: false,
        }
    }

    /// Returns an owned (`'static`) version of this font family name.
    ///
    /// This is useful when you want to store a `FontFamilyName` without borrowing from an input
    /// string slice.
    ///
    /// This allocates only if the name is a borrowed string (for example, `Named(Cow::Borrowed)`).
    #[must_use]
    #[inline]
    pub fn into_owned(self) -> FontFamilyName<'static> {
        match self {
            Self::Named(name) => FontFamilyName::Named(Cow::Owned(name.into_owned())),
            Self::Generic(g) => FontFamilyName::Generic(g),
        }
    }
}

impl From<GenericFamily> for FontFamilyName<'_> {
    fn from(f: GenericFamily) -> Self {
        FontFamilyName::Generic(f)
    }
}

impl fmt::Display for FontFamilyName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "{name:?}"),
            Self::Generic(family) => write!(f, "{family}"),
        }
    }
}

/// CSS `font-family` property value.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/font-family>
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FontFamily<'a> {
    /// Font family list in CSS format.
    Source(Cow<'a, str>),
    /// Single font family.
    Single(FontFamilyName<'a>),
    /// Ordered list of font families.
    List(Cow<'a, [FontFamilyName<'a>]>),
}

impl<'a> FontFamily<'a> {
    /// Creates a `font-family` value consisting of a single named family.
    pub const fn named(name: &'a str) -> Self {
        Self::Single(FontFamilyName::named(name))
    }

    /// Returns an owned (`'static`) version of this `font-family` value.
    ///
    /// This is useful when you want to store a `FontFamily` without borrowing from an input string
    /// slice.
    ///
    /// This allocates only if the value is borrowed (for example, `Source(Cow::Borrowed)` or
    /// `List(Cow::Borrowed)`).
    #[must_use]
    #[inline]
    pub fn into_owned(self) -> FontFamily<'static> {
        match self {
            Self::Source(source) => FontFamily::Source(Cow::Owned(source.into_owned())),
            Self::Single(name) => FontFamily::Single(name.into_owned()),
            Self::List(list) => {
                let out: Vec<FontFamilyName<'static>> = match list {
                    Cow::Borrowed(slice) => slice
                        .iter()
                        .cloned()
                        .map(FontFamilyName::into_owned)
                        .collect(),
                    Cow::Owned(vec) => vec.into_iter().map(FontFamilyName::into_owned).collect(),
                };
                FontFamily::List(Cow::Owned(out))
            }
        }
    }
}

impl From<GenericFamily> for FontFamily<'_> {
    fn from(f: GenericFamily) -> Self {
        FontFamily::Single(f.into())
    }
}

impl<'a> From<FontFamilyName<'a>> for FontFamily<'a> {
    fn from(f: FontFamilyName<'a>) -> Self {
        FontFamily::Single(f)
    }
}

impl<'a> From<&'a str> for FontFamily<'a> {
    fn from(s: &'a str) -> Self {
        FontFamily::Source(Cow::Borrowed(s))
    }
}

impl<'a> From<&'a [FontFamilyName<'a>]> for FontFamily<'a> {
    fn from(fs: &'a [FontFamilyName<'a>]) -> Self {
        FontFamily::List(Cow::Borrowed(fs))
    }
}

#[derive(Clone)]
struct ParseCssList<'a> {
    source: &'a [u8],
    len: usize,
    pos: usize,
    done: bool,
}

impl<'a> Iterator for ParseCssList<'a> {
    type Item = Result<FontFamilyName<'a>, ParseFontFamilyError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut pos = self.pos;
        while pos < self.len && self.source[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.pos = pos;
        if pos >= self.len {
            self.done = true;
            return None;
        }
        if self.source[pos] == b',' {
            self.done = true;
            return Some(Err(ParseFontFamilyError::new(
                ParseFontFamilyErrorKind::InvalidSyntax,
                pos,
            )));
        }

        let first = self.source[pos];
        let mut start = pos;
        if matches!(first, b'"' | b'\'') {
            let quote = first;
            let opening_quote = pos;
            pos += 1;
            start += 1;
            while pos < self.len {
                if self.source[pos] == quote {
                    let name = match self
                        .source
                        .get(start..pos)
                        .and_then(|bytes| core::str::from_utf8(bytes).ok())
                    {
                        Some(s) => s,
                        None => {
                            self.done = true;
                            return Some(Err(ParseFontFamilyError::new(
                                ParseFontFamilyErrorKind::InvalidSyntax,
                                start,
                            )));
                        }
                    };
                    pos += 1;
                    while pos < self.len && self.source[pos].is_ascii_whitespace() {
                        pos += 1;
                    }
                    if pos < self.len {
                        if self.source[pos] != b',' {
                            self.done = true;
                            return Some(Err(ParseFontFamilyError::new(
                                ParseFontFamilyErrorKind::InvalidSyntax,
                                pos,
                            )));
                        }
                        pos += 1;
                    }
                    self.pos = pos;
                    return Some(Ok(FontFamilyName::Named(Cow::Borrowed(name))));
                }
                pos += 1;
            }
            self.done = true;
            return Some(Err(ParseFontFamilyError::new(
                ParseFontFamilyErrorKind::UnterminatedString,
                opening_quote,
            )
            .with_span((opening_quote, self.len))));
        }
        let mut end = start;
        while pos < self.len {
            if self.source[pos] == b',' {
                pos += 1;
                break;
            }
            pos += 1;
            end += 1;
        }
        self.pos = pos;
        let name = match self
            .source
            .get(start..end)
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
        {
            Some(s) => s.trim(),
            None => {
                self.done = true;
                return Some(Err(ParseFontFamilyError::new(
                    ParseFontFamilyErrorKind::InvalidSyntax,
                    start,
                )));
            }
        };
        Some(Ok(match GenericFamily::parse(name) {
            Some(family) => FontFamilyName::Generic(family),
            _ => FontFamilyName::Named(Cow::Borrowed(name)),
        }))
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::borrow::Cow;

    use super::{FontFamily, FontFamilyName, GenericFamily, ParseFontFamilyErrorKind};

    #[test]
    fn parse_generic_family_is_generic_when_unquoted() {
        assert_eq!(
            FontFamilyName::parse("monospace"),
            Some(FontFamilyName::Generic(GenericFamily::Monospace))
        );
    }

    #[test]
    fn parse_generic_family_is_case_insensitive_when_unquoted() {
        assert_eq!(
            FontFamilyName::parse("SERIF"),
            Some(FontFamilyName::Generic(GenericFamily::Serif))
        );

        let families: Result<alloc::vec::Vec<_>, _> =
            FontFamilyName::parse_css_list(r#"Arial, SERIF, "serif""#).collect();
        assert_eq!(
            families.unwrap(),
            alloc::vec![
                FontFamilyName::Named(Cow::Borrowed("Arial")),
                FontFamilyName::Generic(GenericFamily::Serif),
                FontFamilyName::Named(Cow::Borrowed("serif")),
            ]
        );
    }

    #[test]
    fn parse_generic_family_is_named_when_quoted() {
        assert_eq!(
            FontFamilyName::parse("'monospace'"),
            Some(FontFamilyName::Named(Cow::Borrowed("monospace")))
        );
        assert_eq!(
            FontFamilyName::parse("\"monospace\""),
            Some(FontFamilyName::Named(Cow::Borrowed("monospace")))
        );
    }

    #[test]
    fn parse_css_list_unterminated_string_reports_offset_and_span() {
        let err = FontFamilyName::parse_css_list("'monospace")
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), ParseFontFamilyErrorKind::UnterminatedString);
        assert_eq!(err.byte_offset(), 0);
        assert_eq!(err.byte_span(), Some((0, 10)));
    }

    #[test]
    fn parse_css_list_rejects_empty_entries() {
        let err = FontFamilyName::parse_css_list("Arial,,serif")
            .collect::<Result<alloc::vec::Vec<_>, _>>()
            .unwrap_err();
        assert_eq!(err.kind(), ParseFontFamilyErrorKind::InvalidSyntax);
        assert_eq!(err.byte_offset(), 6);
        assert_eq!(err.byte_span(), None);
    }

    #[test]
    fn parse_css_list_rejects_leading_comma() {
        let err = FontFamilyName::parse_css_list(", Arial")
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), ParseFontFamilyErrorKind::InvalidSyntax);
        assert_eq!(err.byte_offset(), 0);
        assert_eq!(err.byte_span(), None);
    }

    #[test]
    fn parse_css_list_trailing_comma_is_ok() {
        let families: Result<alloc::vec::Vec<_>, _> =
            FontFamilyName::parse_css_list("Arial,").collect();
        assert_eq!(
            families.unwrap(),
            alloc::vec![FontFamilyName::Named(Cow::Borrowed("Arial"))]
        );
    }

    #[test]
    fn parse_quoted_name_preserves_inner_whitespace() {
        let families: Result<alloc::vec::Vec<_>, _> =
            FontFamilyName::parse_css_list("'  Times New Roman  '").collect();
        assert_eq!(
            families.unwrap(),
            alloc::vec![FontFamilyName::Named(Cow::Borrowed("  Times New Roman  "))]
        );
    }

    #[test]
    fn parse_css_list_requires_commas_between_quoted_and_unquoted() {
        let err = FontFamilyName::parse_css_list(r#""Times New Roman" serif"#)
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), ParseFontFamilyErrorKind::InvalidSyntax);
        assert_eq!(err.byte_offset(), 18);
        assert_eq!(err.byte_span(), None);
    }

    #[test]
    fn font_family_name_into_owned_preserves_value() {
        let borrowed = FontFamilyName::named("Arial");
        let owned = borrowed.into_owned();
        assert_eq!(owned, FontFamilyName::Named(Cow::Owned("Arial".into())));

        let borrowed = FontFamilyName::Generic(GenericFamily::Serif);
        let owned = borrowed.into_owned();
        assert_eq!(owned, FontFamilyName::Generic(GenericFamily::Serif));
    }

    #[test]
    fn font_family_into_owned_preserves_value() {
        let borrowed = FontFamily::from("Arial, serif");
        let owned = borrowed.into_owned();
        assert_eq!(owned, FontFamily::Source(Cow::Owned("Arial, serif".into())));

        let borrowed = FontFamily::named("Arial");
        let owned = borrowed.into_owned();
        assert_eq!(
            owned,
            FontFamily::Single(FontFamilyName::Named(Cow::Owned("Arial".into())))
        );

        let list = [
            FontFamilyName::named("Arial"),
            FontFamilyName::Generic(GenericFamily::Serif),
        ];
        let borrowed = FontFamily::from(list.as_slice());
        let owned = borrowed.into_owned();
        assert_eq!(
            owned,
            FontFamily::List(Cow::Owned(alloc::vec![
                FontFamilyName::Named(Cow::Owned("Arial".into())),
                FontFamilyName::Generic(GenericFamily::Serif),
            ]))
        );
    }
}
