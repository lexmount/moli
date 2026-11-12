// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;
use core::str::FromStr;

/// An ISO 15924 script identifier (four ASCII letters).
///
/// This type stores the canonical `Titlecase` form (e.g. `Latn`, `Cyrl`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Script {
    raw: [u8; 4],
}

impl Script {
    /// The “unknown” script (`Zzzz`).
    pub const UNKNOWN: Self = Self::from_bytes(*b"Zzzz");

    /// The “common” script (`Zyyy`).
    pub const COMMON: Self = Self::from_bytes(*b"Zyyy");

    /// The “inherited” script (`Zinh`).
    pub const INHERITED: Self = Self::from_bytes(*b"Zinh");

    /// Creates a `Script` from raw ISO 15924 bytes.
    ///
    /// The input must be four ASCII bytes in canonical form. This function does not validate.
    ///
    /// The ASCII bytes can be provided as a byte array literal:
    /// ```
    /// # use parlance::Script;
    /// const LATIN: Script = Script::from_bytes(*b"Latn");
    /// ```
    #[must_use]
    #[inline(always)]
    pub const fn from_bytes(raw: [u8; 4]) -> Self {
        Self { raw }
    }

    /// Creates a `Script` from a 4-byte string literal.
    ///
    /// This was intended for `const` construction and does not validate the input.
    /// This method has been deprecated in favour of [`Self::from_bytes`].
    #[must_use]
    #[inline(always)]
    #[deprecated(
        since = "0.1.1",
        note = r#"You should use `from_bytes(*b"....")` instead."#
    )]
    pub const fn from_str_unchecked(s: &str) -> Self {
        let bytes = s.as_bytes();
        Self::from_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    /// Returns the raw ISO 15924 bytes.
    #[must_use]
    #[inline(always)]
    pub const fn to_bytes(self) -> [u8; 4] {
        self.raw
    }

    /// Returns the canonical string form (e.g. `Latn`).
    #[must_use]
    #[inline(always)]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.raw).expect("only ASCII")
    }

    /// Parses an ISO 15924 script identifier.
    ///
    /// Parsing is case-insensitive; output is normalized to `Titlecase` (e.g. `LATN` → `Latn`).
    #[inline(always)]
    pub fn parse(s: &str) -> Result<Self, ParseScriptError> {
        s.parse()
    }

    /// Whether this is a cursive script according to [CSS Text 3 Appendix D][css-cursive-scripts].
    ///
    /// As per that specification, "these scripts do not admit gaps between their letters for either
    /// justification or letter-spacing."
    ///
    /// See also [CSS Text 3 § 6.4.4][css-cursive-justification] on cursive justification.
    ///
    /// [css-cursive-scripts]: https://www.w3.org/TR/css-text-3/#script-groups
    /// [css-cursive-justification]: https://www.w3.org/TR/css-text-3/#justify-cursive
    #[inline(always)]
    pub const fn is_cursive(self) -> bool {
        // Script identifiers from UAX #31: <https://www.unicode.org/reports/tr31/>.
        matches!(
            &self.raw,
            b"Arab" | // Arabic
            b"Rohg" | // Hanifi Rohingya
            b"Mand" | // Mandaic
            b"Mong" | // Mongolian
            b"Nkoo" | // N'ko
            b"Phag" | // Phags Pa
            b"Syrc" // Syriac
        )
    }
}

impl fmt::Debug for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Script").field(&self.as_str()).finish()
    }
}

impl fmt::Display for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Script {
    type Err = ParseScriptError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != 4 {
            return Err(ParseScriptError::InvalidLength);
        }
        if !bytes.iter().all(|b| b.is_ascii_alphabetic()) {
            return Err(ParseScriptError::InvalidBytes);
        }
        let mut raw = [0_u8; 4];
        raw[0] = bytes[0].to_ascii_uppercase();
        raw[1] = bytes[1].to_ascii_lowercase();
        raw[2] = bytes[2].to_ascii_lowercase();
        raw[3] = bytes[3].to_ascii_lowercase();
        Ok(Self { raw })
    }
}

/// An error returned from parsing a [`Script`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseScriptError {
    /// The input was not exactly four bytes.
    InvalidLength,
    /// The input contained non-ASCII alphabetic bytes.
    InvalidBytes,
}

impl fmt::Display for ParseScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => f.write_str("invalid script length"),
            Self::InvalidBytes => f.write_str("invalid script bytes"),
        }
    }
}

impl core::error::Error for ParseScriptError {}
