// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;
use core::str::FromStr;

/// A compact, zero-allocation language tag.
///
/// Parses the `language [-script] [-region]` prefix of a BCP 47 language tag.
///
/// This type captures only the `language` + optional `script` + optional `region` subtags from a
/// BCP 47 language tag. Parsing is case-insensitive; output is normalized to:
/// - language: 2–3 lowercase letters (e.g. `EN` → `en`)
/// - script: 4 titlecase letters (e.g. `LATN` → `Latn`)
/// - region: 2 uppercase letters or 3 digits (e.g. `us` → `US`, `419` → `419`)
///
/// Trailing subtags (variants, extensions, private use) are validated for basic structural
/// conformance but discarded. Extended language subtags (`extlang`) are not supported and will
/// error.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Language {
    bytes: [u8; 12],
    len: u8,
    language_len: u8,
    script_len: u8,
    region_len: u8,
}

impl Language {
    /// The maximum length of the canonical `language[-Script][-REGION]` form.
    pub const MAX_LEN: usize = 12;

    /// The “undefined” language (`und`).
    pub const UND: Self = Self::from_language_bytes(*b"und", 3);

    /// Parses a language tag, keeping only language/script/region.
    #[inline(always)]
    pub fn parse(s: &str) -> Result<Self, ParseLanguageError> {
        s.parse()
    }

    /// Parses the `language[-Script][-REGION]` prefix of a tag, returning the remainder.
    ///
    /// The remainder starts at the first unconsumed subtag (without a leading `-`/`_`). This can
    /// be used to inspect or process variants, extensions, or private-use subtags without forcing
    /// this type to model them.
    ///
    /// ```
    /// use parlance::Language;
    ///
    /// let (lang, rest) = Language::parse_prefix("tr-Latin-TR").unwrap();
    /// assert_eq!(lang.as_str(), "tr");
    /// assert_eq!(rest, "Latin-TR");
    /// ```
    pub fn parse_prefix(s: &str) -> Result<(Self, &str), ParseLanguageError> {
        let (lang, rest) = parse_language_prefix(s)?;
        // Normalize the remainder by dropping any leading separators.
        let rest = rest.trim_start_matches(['-', '_']);
        Ok((lang, rest))
    }

    /// Returns the canonical string form (`language[-Script][-REGION]`).
    #[must_use]
    #[inline(always)]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("only ASCII")
    }

    /// Returns the primary language subtag (lowercase).
    #[must_use]
    #[inline(always)]
    pub fn language(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.language_len as usize]).expect("only ASCII")
    }

    /// Returns the script subtag (titlecase), if present.
    #[must_use]
    pub fn script(&self) -> Option<&str> {
        if self.script_len == 0 {
            return None;
        }
        let start = self.language_len as usize + 1;
        let end = start + self.script_len as usize;
        Some(core::str::from_utf8(&self.bytes[start..end]).expect("only ASCII"))
    }

    /// Returns the region subtag (uppercase or digits), if present.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        if self.region_len == 0 {
            return None;
        }
        let mut start = self.language_len as usize;
        if self.script_len != 0 {
            start += 1 + self.script_len as usize;
        }
        start += 1;
        let end = start + self.region_len as usize;
        Some(core::str::from_utf8(&self.bytes[start..end]).expect("only ASCII"))
    }

    const fn from_language_bytes(bytes: [u8; 3], len: u8) -> Self {
        let mut out = Self {
            bytes: [0; 12],
            len: 0,
            language_len: len,
            script_len: 0,
            region_len: 0,
        };
        out.bytes[0] = bytes[0];
        out.bytes[1] = bytes[1];
        out.bytes[2] = bytes[2];
        out.len = len;
        out
    }
}

fn parse_language_prefix(s: &str) -> Result<(Language, &str), ParseLanguageError> {
    let bytes = s.as_bytes();
    let mut pos = 0_usize;

    let (language_start, language_end) =
        next_part_bounds(s, &mut pos).ok_or(ParseLanguageError::InvalidLanguage)?;
    let language = &s[language_start..language_end];
    let language_bytes = language.as_bytes();
    if !(2..=3).contains(&language_bytes.len())
        || !language_bytes.iter().all(|b| b.is_ascii_alphabetic())
    {
        return Err(ParseLanguageError::InvalidLanguage);
    }

    let mut out = Language {
        bytes: [0; 12],
        len: 0,
        #[allow(
            clippy::cast_possible_truncation,
            reason = "language subtag length is 2 or 3"
        )]
        language_len: language_bytes.len() as u8,
        script_len: 0,
        region_len: 0,
    };

    // language: lower
    for (i, b) in language_bytes.iter().enumerate() {
        out.bytes[i] = b.to_ascii_lowercase();
    }
    out.len = out.language_len;

    // optional script or region
    let Some((start, end)) = next_part_bounds(s, &mut pos) else {
        return Ok((out, ""));
    };

    let b = &bytes[start..end];
    if b.len() == 4 {
        if !b.iter().all(|c| c.is_ascii_alphabetic()) {
            // BCP 47 variant: 4 alphanum starting with a digit.
            if is_variant(b) {
                return Ok((out, &s[start..]));
            }
            return Err(ParseLanguageError::InvalidScript);
        }
        out.bytes[out.len as usize] = b'-';
        out.len += 1;
        out.script_len = 4;
        // titlecase script
        out.bytes[out.len as usize] = b[0].to_ascii_uppercase();
        out.bytes[out.len as usize + 1] = b[1].to_ascii_lowercase();
        out.bytes[out.len as usize + 2] = b[2].to_ascii_lowercase();
        out.bytes[out.len as usize + 3] = b[3].to_ascii_lowercase();
        out.len += 4;
    } else if b.len() == 2 || b.len() == 3 {
        let is_alpha2 = b.len() == 2 && b.iter().all(|c| c.is_ascii_alphabetic());
        let is_digit3 = b.len() == 3 && b.iter().all(|c| c.is_ascii_digit());
        if !(is_alpha2 || is_digit3) {
            return Err(ParseLanguageError::InvalidRegion);
        }
        out.bytes[out.len as usize] = b'-';
        out.len += 1;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "region subtag length is 2 or 3"
        )]
        {
            out.region_len = b.len() as u8;
        }
        if is_alpha2 {
            out.bytes[out.len as usize] = b[0].to_ascii_uppercase();
            out.bytes[out.len as usize + 1] = b[1].to_ascii_uppercase();
        } else {
            out.bytes[out.len as usize] = b[0];
            out.bytes[out.len as usize + 1] = b[1];
            out.bytes[out.len as usize + 2] = b[2];
        }
        out.len += out.region_len;
    } else {
        return Ok((out, &s[start..]));
    }

    // optional region (if not already set)
    if out.region_len != 0 {
        return Ok((out, &s[pos..]));
    }
    let Some((start, end)) = next_part_bounds(s, &mut pos) else {
        return Ok((out, ""));
    };
    let b = &bytes[start..end];
    if b.len() == 2 || b.len() == 3 {
        let is_alpha2 = b.len() == 2 && b.iter().all(|c| c.is_ascii_alphabetic());
        let is_digit3 = b.len() == 3 && b.iter().all(|c| c.is_ascii_digit());
        if !(is_alpha2 || is_digit3) {
            return Err(ParseLanguageError::InvalidRegion);
        }
        out.bytes[out.len as usize] = b'-';
        out.len += 1;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "region subtag length is 2 or 3"
        )]
        {
            out.region_len = b.len() as u8;
        }
        if is_alpha2 {
            out.bytes[out.len as usize] = b[0].to_ascii_uppercase();
            out.bytes[out.len as usize + 1] = b[1].to_ascii_uppercase();
        } else {
            out.bytes[out.len as usize] = b[0];
            out.bytes[out.len as usize + 1] = b[1];
            out.bytes[out.len as usize + 2] = b[2];
        }
        out.len += out.region_len;
        Ok((out, &s[pos..]))
    } else {
        Ok((out, &s[start..]))
    }
}

fn next_part_bounds(s: &str, pos: &mut usize) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    while *pos < bytes.len() && matches!(bytes[*pos], b'-' | b'_') {
        *pos += 1;
    }
    if *pos >= bytes.len() {
        return None;
    }
    let start = *pos;
    while *pos < bytes.len() && !matches!(bytes[*pos], b'-' | b'_') {
        *pos += 1;
    }
    let end = *pos;
    Some((start, end))
}

impl fmt::Debug for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Language").field(&self.as_str()).finish()
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error returned when parsing a [`Language`] fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseLanguageError {
    /// The input did not contain a valid primary language subtag.
    InvalidLanguage,
    /// The script subtag was malformed or appeared in an invalid position.
    InvalidScript,
    /// The region subtag was malformed or appeared in an invalid position.
    InvalidRegion,
    /// The tag contained an invalid or unsupported subtag sequence.
    InvalidSubtag,
}

impl fmt::Display for ParseLanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLanguage => f.write_str("invalid primary language subtag"),
            Self::InvalidScript => f.write_str("invalid script subtag"),
            Self::InvalidRegion => f.write_str("invalid region subtag"),
            Self::InvalidSubtag => f.write_str("invalid language subtag sequence"),
        }
    }
}

impl core::error::Error for ParseLanguageError {}

impl FromStr for Language {
    type Err = ParseLanguageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (lang, rest) = parse_language_prefix(s)?;
        validate_remainder(rest)?;
        Ok(lang)
    }
}

fn validate_remainder(rest: &str) -> Result<(), ParseLanguageError> {
    #[derive(Clone, Copy)]
    enum State {
        Base,
        Extension,
        Private,
    }

    let mut state = State::Base;
    let mut needs_payload = false;

    for part in rest.split(['-', '_']).filter(|p| !p.is_empty()) {
        let bytes = part.as_bytes();

        match state {
            State::Base => {
                if is_variant(bytes) {
                    continue;
                }

                if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
                    state = if bytes[0].eq_ignore_ascii_case(&b'x') {
                        State::Private
                    } else {
                        State::Extension
                    };
                    needs_payload = true;
                    continue;
                }

                if bytes.len() == 4 && bytes.iter().all(|c| c.is_ascii_alphabetic()) {
                    return Err(ParseLanguageError::InvalidScript);
                }

                if bytes.len() == 2 && bytes.iter().all(|c| c.is_ascii_alphabetic()) {
                    return Err(ParseLanguageError::InvalidRegion);
                }

                if bytes.len() == 3 && bytes.iter().all(|c| c.is_ascii_digit()) {
                    return Err(ParseLanguageError::InvalidRegion);
                }

                return Err(ParseLanguageError::InvalidSubtag);
            }
            State::Extension => {
                if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
                    if needs_payload {
                        return Err(ParseLanguageError::InvalidSubtag);
                    }
                    state = if bytes[0].eq_ignore_ascii_case(&b'x') {
                        State::Private
                    } else {
                        State::Extension
                    };
                    needs_payload = true;
                    continue;
                }

                if (2..=8).contains(&bytes.len()) && bytes.iter().all(|b| b.is_ascii_alphanumeric())
                {
                    needs_payload = false;
                    continue;
                }

                return Err(ParseLanguageError::InvalidSubtag);
            }
            State::Private => {
                if (1..=8).contains(&bytes.len()) && bytes.iter().all(|b| b.is_ascii_alphanumeric())
                {
                    needs_payload = false;
                    continue;
                }
                return Err(ParseLanguageError::InvalidSubtag);
            }
        }
    }

    if needs_payload {
        return Err(ParseLanguageError::InvalidSubtag);
    }

    Ok(())
}

fn is_variant(bytes: &[u8]) -> bool {
    // BCP 47 variant: 5-8 alphanum or 4 alphanum starting with a digit.
    if bytes.len() == 4 {
        bytes[0].is_ascii_digit() && bytes.iter().all(|b| b.is_ascii_alphanumeric())
    } else if (5..=8).contains(&bytes.len()) {
        bytes.iter().all(|b| b.is_ascii_alphanumeric())
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{Language, ParseLanguageError};

    #[test]
    fn parse_language_only() {
        let lang = Language::parse("EN").unwrap();
        assert_eq!(lang.as_str(), "en");
        assert_eq!(lang.language(), "en");
        assert_eq!(lang.script(), None);
        assert_eq!(lang.region(), None);
    }

    #[test]
    fn parse_with_script_and_region() {
        let lang = Language::parse("zh-Hans-CN").unwrap();
        assert_eq!(lang.as_str(), "zh-Hans-CN");
        assert_eq!(lang.language(), "zh");
        assert_eq!(lang.script(), Some("Hans"));
        assert_eq!(lang.region(), Some("CN"));
    }

    #[test]
    fn parse_with_region_only() {
        let lang = Language::parse("es_419").unwrap();
        assert_eq!(lang.as_str(), "es-419");
        assert_eq!(lang.region(), Some("419"));
    }

    #[test]
    fn parse_ignores_variants_after_region() {
        let lang = Language::parse("en-Latn-US-posix").unwrap();
        assert_eq!(lang.as_str(), "en-Latn-US");
    }

    #[test]
    fn invalid_language_errors() {
        assert_eq!(
            Language::parse("").unwrap_err(),
            ParseLanguageError::InvalidLanguage
        );
        assert_eq!(
            Language::parse("e").unwrap_err(),
            ParseLanguageError::InvalidLanguage
        );
        assert_eq!(
            Language::parse("en1").unwrap_err(),
            ParseLanguageError::InvalidLanguage
        );
    }

    #[test]
    fn invalid_script_errors() {
        assert_eq!(
            Language::parse("en-La1n").unwrap_err(),
            ParseLanguageError::InvalidScript
        );
        assert_eq!(
            Language::parse("en-Latn-US-Latn").unwrap_err(),
            ParseLanguageError::InvalidScript
        );
    }

    #[test]
    fn invalid_region_errors() {
        assert_eq!(
            Language::parse("en-U1").unwrap_err(),
            ParseLanguageError::InvalidRegion
        );
        assert_eq!(
            Language::parse("en-Latn-US-CA").unwrap_err(),
            ParseLanguageError::InvalidRegion
        );
        assert_eq!(
            Language::parse("en-Latin-US").unwrap_err(),
            ParseLanguageError::InvalidRegion
        );
    }

    #[test]
    fn invalid_subtag_errors() {
        assert_eq!(
            Language::parse("en-abc$e").unwrap_err(),
            ParseLanguageError::InvalidSubtag
        );
    }

    #[test]
    fn parse_prefix_stops_at_unrecognized_subtag() {
        let (lang, rest) = Language::parse_prefix("tr-Latin-TR").unwrap();
        assert_eq!(lang.as_str(), "tr");
        assert_eq!(rest, "Latin-TR");
    }

    #[test]
    fn parse_prefix_returns_variants_and_extensions() {
        let (lang, rest) = Language::parse_prefix("en-Latn-US-posix-u-ca-gregory").unwrap();
        assert_eq!(lang.as_str(), "en-Latn-US");
        assert_eq!(rest, "posix-u-ca-gregory");
    }

    #[test]
    fn parse_4_char_digit_variant() {
        // BCP 47 variants can be 4 alphanumeric chars if they start with a digit.
        // These should be treated as variants (in remainder), not rejected as invalid scripts.
        let lang = Language::parse("de-1996").unwrap();
        assert_eq!(lang.as_str(), "de");
        assert_eq!(lang.script(), None);
        assert_eq!(lang.region(), None);

        // With script before the variant
        let lang = Language::parse("de-Latn-1996").unwrap();
        assert_eq!(lang.as_str(), "de-Latn");
        assert_eq!(lang.script(), Some("Latn"));

        // With region before the variant
        let lang = Language::parse("de-DE-1996").unwrap();
        assert_eq!(lang.as_str(), "de-DE");
        assert_eq!(lang.region(), Some("DE"));

        // parse_prefix should return the variant in the remainder
        let (lang, rest) = Language::parse_prefix("de-1996").unwrap();
        assert_eq!(lang.as_str(), "de");
        assert_eq!(rest, "1996");
    }
}
