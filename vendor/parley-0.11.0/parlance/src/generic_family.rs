// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

/// Generic font families, named after CSS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GenericFamily {
    /// Glyphs have finishing strokes, flared or tapering ends, or have actual serifed endings.
    Serif = 0,
    /// Glyphs have stroke endings that are plain.
    SansSerif = 1,
    /// All glyphs have the same fixed width.
    Monospace = 2,
    /// Glyphs in cursive fonts generally have either joining strokes or other
    /// cursive characteristics beyond those of italic typefaces. The glyphs
    /// are partially or completely connected, and the result looks more like
    /// handwritten pen or brush writing than printed letter work.
    Cursive = 3,
    /// Decorative fonts with playful representations of characters.
    Fantasy = 4,
    /// Glyphs are taken from the default user interface font on a given platform.
    SystemUi = 5,
    /// The default user interface serif font.
    UiSerif = 6,
    /// The default user interface sans-serif font.
    UiSansSerif = 7,
    /// The default user interface monospace font.
    UiMonospace = 8,
    /// The default user interface font that has rounded features.
    UiRounded = 9,
    /// Fonts that are specifically designed to render emoji.
    Emoji = 10,
    /// This is for the particular stylistic concerns of representing
    /// mathematics: superscript and subscript, brackets that cross several
    /// lines, nesting expressions, and double struck glyphs with distinct
    /// meanings.
    Math = 11,
    /// A particular style of Chinese characters that are between serif-style
    /// Song and cursive-style Kai forms. This style is often used for
    /// government documents.
    FangSong = 12,
    // NOTICE: If a new value is added, be sure to modify `MAX_VALUE`.
}

impl GenericFamily {
    /// Returns the maximum numeric value for known variants.
    ///
    /// This is primarily intended for use in fixed-size maps keyed by `GenericFamily`.
    pub const MAX_VALUE: u8 = Self::FangSong as u8;

    /// Parses a generic family from a CSS generic family name.
    ///
    /// ```
    /// use parlance::GenericFamily;
    ///
    /// assert_eq!(
    ///     GenericFamily::parse("sans-serif"),
    ///     Some(GenericFamily::SansSerif)
    /// );
    /// assert_eq!(GenericFamily::parse("SERIF"), Some(GenericFamily::Serif));
    /// assert_eq!(GenericFamily::parse("Arial"), None);
    /// ```
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        // The longest generic family name is `ui-sans-serif` (13 bytes). Any
        // input longer than this cannot match, and any shorter input is
        // lowercased once into this fixed-size buffer so we only have to do a
        // single case-sensitive comparison below.
        const MAX_LEN: usize = "ui-sans-serif".len();
        let bytes = s.as_bytes();
        if bytes.len() > MAX_LEN {
            return None;
        }
        let mut buf = [0_u8; MAX_LEN];
        let buf = &mut buf[..bytes.len()];
        buf.copy_from_slice(bytes);
        buf.make_ascii_lowercase();

        Some(match &*buf {
            b"serif" => Self::Serif,
            b"sans-serif" => Self::SansSerif,
            b"monospace" => Self::Monospace,
            b"cursive" => Self::Cursive,
            b"fantasy" => Self::Fantasy,
            b"system-ui" => Self::SystemUi,
            b"ui-serif" => Self::UiSerif,
            b"ui-sans-serif" => Self::UiSansSerif,
            b"ui-monospace" => Self::UiMonospace,
            b"ui-rounded" => Self::UiRounded,
            b"emoji" => Self::Emoji,
            b"math" => Self::Math,
            b"fangsong" => Self::FangSong,
            _ => return None,
        })
    }

    /// Returns a slice containing all generic family variants.
    pub const fn all() -> &'static [Self] {
        &[
            Self::SansSerif,
            Self::Serif,
            Self::Monospace,
            Self::Cursive,
            Self::Fantasy,
            Self::SystemUi,
            Self::UiSerif,
            Self::UiSansSerif,
            Self::UiMonospace,
            Self::UiRounded,
            Self::Emoji,
            Self::Math,
            Self::FangSong,
        ]
    }
}

impl fmt::Display for GenericFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Serif => "serif",
            Self::SansSerif => "sans-serif",
            Self::Monospace => "monospace",
            Self::Cursive => "cursive",
            Self::Fantasy => "fantasy",
            Self::SystemUi => "system-ui",
            Self::UiSerif => "ui-serif",
            Self::UiSansSerif => "ui-sans-serif",
            Self::UiMonospace => "ui-monospace",
            Self::UiRounded => "ui-rounded",
            Self::Emoji => "emoji",
            Self::Math => "math",
            Self::FangSong => "fangsong",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::GenericFamily;

    #[test]
    fn parse_is_ascii_case_insensitive() {
        assert_eq!(GenericFamily::parse("SERIF"), Some(GenericFamily::Serif));
        assert_eq!(
            GenericFamily::parse("Sans-Serif"),
            Some(GenericFamily::SansSerif)
        );
        assert_eq!(
            GenericFamily::parse("MONOSPACE"),
            Some(GenericFamily::Monospace)
        );
        assert_eq!(
            GenericFamily::parse("UI-Rounded"),
            Some(GenericFamily::UiRounded)
        );
        assert_eq!(
            GenericFamily::parse("FangSong"),
            Some(GenericFamily::FangSong)
        );
    }

    #[test]
    fn parse_preserves_lowercase_trim_and_named_behavior() {
        assert_eq!(GenericFamily::parse("serif"), Some(GenericFamily::Serif));
        assert_eq!(
            GenericFamily::parse("  SERIF  "),
            Some(GenericFamily::Serif)
        );
        assert_eq!(GenericFamily::parse("Arial"), None);
    }
}
