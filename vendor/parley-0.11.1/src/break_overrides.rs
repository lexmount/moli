// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Overrides for line-break opportunities.

use core::ops::RangeInclusive;

/// Context for a potential line break opportunity between two adjacent code points.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct LineBreakContext {
    /// The code point before the `before` code point, if any.
    pub before_before: Option<char>,
    /// The code point before the potential break.
    pub before: char,
    /// The code point after the potential break.
    pub after: char,
}

/// Line break opportunity override.
///
/// Called for each adjacent pair of Unicode code points in the text, in order,
/// with a [`LineBreakContext`] describing the potential break.
///
/// Returning:
///    - `Some(true)`  : forces a line break opportunity between the pair
///    - `Some(false)` : suppresses any opportunity
///    - `None`        : defers to the default (ICU) behavior
///
/// Mandatory breaks are unaffected (e.g. `\n`).
///
/// This is typically used to force preferential line breaking decisions when it
/// comes to ASCII punctuation like `/`, `-`, etc. For example, to prevent break
/// opportunities within "1/2".
///
/// A separate use case is to match the line breaking behavior of existing systems
/// such as web browsers. See [`CHROMIUM_LINE_BREAK_OVERRIDE`] for a ready-made
/// override function that mirrors Chromium's behavior.
pub type LineBreakOverrideFn = dyn Fn(LineBreakContext) -> Option<bool> + Send + Sync;

/// A line break override function mirroring Chromium's preferred line breaking behavior.
///
/// See: <https://source.chromium.org/chromium/chromium/src/+/main:third_party/blink/renderer/platform/text/character_property_data_generator.cc;l=449-495>
///
/// # Differences from other browsers
///
/// ## Compared to Safari
///
/// Chromium (and this table) differs from Safari in 9 cases:
///  - A "-" followed by one of "!|$|)|/|:|;|?|]|}" is suppressed in Chromium, but broken in Safari.
///
/// ## Compared to Firefox
///
/// Firefox always defers to the default ICU behavior.
pub static CHROMIUM_LINE_BREAK_OVERRIDE: &LineBreakOverrideFn =
    &(chromium_override as fn(LineBreakContext) -> Option<bool>);

fn chromium_override(cx: LineBreakContext) -> Option<bool> {
    let LineBreakContext {
        before_before,
        before,
        after,
        ..
    } = cx;
    // CSS "does not fully define where soft wrap opportunities occur".
    // (https://www.w3.org/TR/css-text-3/#line-breaking)
    // We find that Chrome always treats the position after a space sequence
    // as a line break opportunity, despite this not matching UAX-14
    // (see LB13: https://www.unicode.org/reports/tr14/#LB13; we're not currently
    // aware of any others).
    //
    // See also https://github.com/linebender/parley/pull/485, and https://github.com/linebender/parley/issues/619.
    //
    // See `LazyLineBreakIterator::NextBreakablePosition`
    // <https://source.chromium.org/chromium/chromium/src/+/main:third_party/blink/renderer/platform/text/text_break_iterator.cc;l=282-303>
    //
    // Note that we'd need different handling in the `after == ' '` case if we ever get the equivalent of
    // CSS's whitespace-collapse: break-spaces.
    if before == ' ' && after != ' ' {
        return Some(true);
    }
    // Before consulting 'before' / 'after' pair table, check for the special "-" case.
    //
    // Chromium doesn't allow breaking when it looks like the minus sign is for a negative
    // number like "Subtract -5 from X". But, Chromium does allow breaking when the minus
    // sign is part of a long URL like "AAAA-2222".
    //
    // See <https://github.com/chromium/chromium/blob/c6bee15e8f336c8feabf539d8bbb540c134ec20a/third_party/blink/renderer/platform/text/text_break_iterator.cc#L224-L240>
    if before == '-' && after.is_ascii_digit() {
        return Some(before_before.is_some_and(|c| c.is_ascii_alphanumeric()));
    }
    CHROMIUM_LINE_BREAK_TABLE.lookup(before, after)
}

/// A static table mirroring Chromium's preferred line breaking behavior for `before` / `after`
/// printable ASCII code points.
static CHROMIUM_LINE_BREAK_TABLE: AsciiLineBreakTable<5> =
    AsciiLineBreakTableBuilder::chromium().build::<5>();

/// A line break override table for ASCII character pairs.
///
/// See [`LineBreakOverrideFn`] for more details.
///
/// All table operations are `const`, which means that derived tables don't
/// need a runtime construction step.
///
/// The table is row-deduplicated. Each `before` character maps to
/// one of `N` distinct rows, so tables can be kept tiny.
///
/// # Example
///
/// ```
/// # use parley::{CHROMIUM_LINE_BREAK_OVERRIDE, FontContext, LayoutContext};
/// # let mut font_cx = FontContext::default();
/// # let mut layout_cx: LayoutContext<[u8; 4]> = LayoutContext::new();
/// let text = "Hello there!";
/// let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
/// // Emulate Chromium:
/// builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));
/// let layout = builder.build(text);
/// # let _ = layout;
/// ```
#[derive(Clone)]
pub struct AsciiLineBreakTable<const N: usize> {
    /// Maps each `before` character (`0..128`) to a row in `rows`.
    row_of: [u8; 128],
    /// The distinct rows.
    rows: [Row; N],
}

impl<const N: usize> AsciiLineBreakTable<N> {
    /// Look up the break override for a `(before, after)` pair.
    ///
    /// Return semantics copied from [`LineBreakOverrideFn`].
    pub const fn lookup(&self, before: char, after: char) -> Option<bool> {
        let (b, a) = (before as u32, after as u32);
        if b >= 128 || a >= 128 {
            return None;
        }
        let row = &self.rows[self.row_of[b as usize] as usize];
        if row.overridden & (1_u128 << a) == 0 {
            return None;
        }
        Some(row.allow & (1_u128 << a) != 0)
    }
}

/// A single deduplicated row of a [`AsciiLineBreakTable`].
#[derive(Clone, Copy)]
struct Row {
    /// Bits set means the pair has an override.
    overridden: u128,
    /// Bits set means a break is allowed.
    allow: u128,
}

/// A builder for an [`AsciiLineBreakTable`].
///
/// Construction is `const`. We create a dense `128 x 128` grid of overrides,
/// then compress it into a row-deduplicated table via [`AsciiLineBreakTableBuilder::build`].
#[derive(Clone)]
pub struct AsciiLineBreakTableBuilder {
    /// Bit `after` set in row `before` means the pair has an explicit override.
    overridden: [u128; 128],
    /// Bit `after` set in row `before` means a break is allowed for that pair.
    allow: [u128; 128],
}

impl AsciiLineBreakTableBuilder {
    /// A builder that defers every pair to the default ICU behavior.
    pub const fn new() -> Self {
        Self {
            overridden: [0; 128],
            allow: [0; 128],
        }
    }

    /// Override every pair in the cartesian product of the two inclusive ASCII
    /// ranges.
    ///
    /// # Panics
    ///
    /// All range bounds must be printable ASCII (`<= '\u{7f}'`).
    pub const fn with_pairs(
        mut self,
        before: RangeInclusive<char>,
        after: RangeInclusive<char>,
        allow_break: bool,
    ) -> Self {
        assert!(
            *before.end() as u32 <= 0x7f && *after.end() as u32 <= 0x7f,
            "only printable ASCII is supported",
        );
        let mut before_pos = *before.start() as u32;
        while before_pos <= *before.end() as u32 {
            let mut after_pos = *after.start() as u32;
            while after_pos <= *after.end() as u32 {
                let bit = 1_u128 << after_pos;
                self.overridden[before_pos as usize] |= bit;
                if allow_break {
                    self.allow[before_pos as usize] |= bit;
                } else {
                    self.allow[before_pos as usize] &= !bit;
                }
                after_pos += 1;
            }
            before_pos += 1;
        }
        self
    }

    /// Compress the dense grid into a row-deduplicated [`AsciiLineBreakTable`]
    /// with at most `N` distinct rows.
    ///
    /// # Panics
    ///
    /// Panics if `N` does not exactly match the number of distinct rows required.
    pub const fn build<const N: usize>(&self) -> AsciiLineBreakTable<N> {
        // Row 0 is reserved as the "defer everything" row.
        let mut rows = [Row {
            overridden: 0,
            allow: 0,
        }; N];
        let mut len = 1_usize;
        let mut row_of = [0_u8; 128];

        let mut b = 0;
        while b < 128 {
            let (ov, al) = (self.overridden[b], self.allow[b]);
            let mut idx = 0;
            let mut found = usize::MAX;
            while idx < len {
                if rows[idx].overridden == ov && rows[idx].allow == al {
                    found = idx;
                    break;
                }
                idx += 1;
            }
            let r = if found != usize::MAX {
                found
            } else {
                assert!(
                    len < N,
                    "N is too small to contain table. Repeat with a larger N."
                );
                rows[len] = Row {
                    overridden: ov,
                    allow: al,
                };
                len += 1;
                len - 1
            };
            row_of[b] = r as u8;
            b += 1;
        }

        assert!(
            len == N,
            "N is larger than required. Repeat with a smaller N."
        );

        AsciiLineBreakTable { row_of, rows }
    }

    /// See [`CHROMIUM_LINE_BREAK_TABLE`] for more details.
    const fn chromium() -> Self {
        // The printable ASCII range `'!'..=0x7F`.
        const ALL: RangeInclusive<char> = '!'..='\u{7f}';

        Self::new()
            .with_pairs(ALL, ALL, false)
            .with_pairs(ALL, '('..='(', true)
            .with_pairs(ALL, '<'..='<', true)
            .with_pairs(ALL, '['..='[', true)
            .with_pairs(ALL, '{'..='{', true)
            .with_pairs('-'..='-', ALL, true)
            .with_pairs('?'..='?', ALL, true)
            .with_pairs('-'..='-', '$'..='$', false)
            .with_pairs(ALL, '!'..='!', false)
            .with_pairs('?'..='?', '"'..='"', false)
            .with_pairs('?'..='?', '\''..='\'', false)
            .with_pairs(ALL, ')'..=')', false)
            .with_pairs(ALL, ','..=',', false)
            .with_pairs(ALL, '.'..='.', false)
            .with_pairs(ALL, '/'..='/', false)
            .with_pairs('-'..='-', '0'..='9', false)
            .with_pairs(ALL, ':'..=':', false)
            .with_pairs(ALL, ';'..=';', false)
            .with_pairs(ALL, '?'..='?', false)
            .with_pairs(ALL, ']'..=']', false)
            .with_pairs(ALL, '}'..='}', false)
            .with_pairs('$'..='$', ALL, false)
            .with_pairs('\''..='\'', ALL, false)
            .with_pairs('('..='(', ALL, false)
            .with_pairs('/'..='/', ALL, false)
            .with_pairs('0'..='9', ALL, false)
            .with_pairs('<'..='<', ALL, false)
            .with_pairs('@'..='@', ALL, false)
            .with_pairs('A'..='Z', ALL, false)
            .with_pairs('['..='[', ALL, false)
            .with_pairs('^'..='`', ALL, false)
            .with_pairs('a'..='z', ALL, false)
            .with_pairs('{'..='{', ALL, false)
            .with_pairs('\u{7f}'..='\u{7f}', ALL, false)
    }
}

impl Default for AsciiLineBreakTableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::AsciiLineBreakTableBuilder;
    use super::CHROMIUM_LINE_BREAK_TABLE;
    use super::LineBreakContext;
    use super::chromium_override;

    fn cx(before_before: Option<char>, before: char, after: char) -> LineBreakContext {
        LineBreakContext {
            before_before,
            before,
            after,
        }
    }

    #[test]
    fn chromium_hyphen_digit_depends_on_preceding_char() {
        // A break between '-' and a digit is allowed only when the character
        // preceding the '-' is ASCII alphanumeric.
        assert_eq!(chromium_override(cx(Some('D'), '-', '1')), Some(true));
        assert_eq!(chromium_override(cx(Some('4'), '-', '5')), Some(true));
        // Otherwise the '-' may be a minus sign, so the break is suppressed.
        assert_eq!(chromium_override(cx(Some(' '), '-', '1')), Some(false));
        assert_eq!(chromium_override(cx(Some('('), '-', '1')), Some(false));
        // No preceding character (start of text) behaves like a non-alphanumeric
        // context, matching Chromium's `last_last_ch == 0`.
        assert_eq!(chromium_override(cx(None, '-', '1')), Some(false));
    }

    #[test]
    fn chromium_ignores_uax_14_lb13() {
        // Blink allows a break after a space run unconditionally.
        // See comment in non-test code for more details.
        for after in ['}', ')', ']', '!', '.', ',', '/', ':', ';', '?', 'b', '('] {
            assert_eq!(
                chromium_override(cx(None, ' ', after)),
                Some(true),
                "expected a break after the space, before {after:?}"
            );
        }
    }

    #[test]
    fn chromium_hyphen_non_digit_defers_to_table() {
        // '-' followed by a non-digit ignores `before_before` and uses the table.
        assert_eq!(chromium_override(cx(Some('D'), '-', 'b')), Some(true));
        assert_eq!(chromium_override(cx(None, '-', 'b')), Some(true));
        // Non-ASCII after '-' defers to ICU.
        assert_eq!(chromium_override(cx(Some('D'), '-', 'é')), None);
    }

    #[test]
    fn chromium_suppresses_ascii_punctuation_breaks() {
        let t = &CHROMIUM_LINE_BREAK_TABLE;
        // No break before or after a slash.
        assert_eq!(t.lookup('a', '/'), Some(false));
        assert_eq!(t.lookup('/', 'b'), Some(false));
        // No break around other punctuation.
        assert_eq!(t.lookup('a', '.'), Some(false));
        assert_eq!(t.lookup('a', ':'), Some(false));
        // No break between letters.
        assert_eq!(t.lookup('a', 'b'), Some(false));
    }

    #[test]
    fn chromium_allows_some_breaks() {
        let t = &CHROMIUM_LINE_BREAK_TABLE;
        // Break allowed before opening punctuation, but only when the preceding
        // character's row was not later suppressed.
        assert_eq!(t.lookup(')', '('), Some(true));
        assert_eq!(t.lookup(')', '<'), Some(true));
        assert_eq!(t.lookup('a', '('), Some(false));
        // Break allowed after '-' and '?' except when...
        assert_eq!(t.lookup('-', 'b'), Some(true));
        assert_eq!(t.lookup('?', 'b'), Some(true));
        // ...break not allowed after '-' before a digit, or '?' before a quote.
        assert_eq!(t.lookup('-', '5'), Some(false));
        assert_eq!(t.lookup('?', '"'), Some(false));
    }

    #[test]
    fn non_ascii_pairs_defer_to_icu() {
        let t = &CHROMIUM_LINE_BREAK_TABLE;
        assert_eq!(t.lookup('a', 'é'), None);
        assert_eq!(t.lookup('é', 'a'), None);
        // Space (0x20) is below the printable range, so it defers too.
        assert_eq!(t.lookup('a', ' '), None);
    }

    #[test]
    fn empty_table_defers_to_icu() {
        let t = AsciiLineBreakTableBuilder::new().build::<1>();
        assert_eq!(t.lookup('a', '/'), None);
        assert_eq!(t.lookup('a', '('), None);
    }

    #[test]
    fn dedup_matches_dense_for_chromium() {
        let builder = AsciiLineBreakTableBuilder::chromium();
        for b in 0..128_u32 {
            for a in 0..128_u32 {
                let bit = 1_u128 << a;
                let dense = if builder.overridden[b as usize] & bit == 0 {
                    None
                } else {
                    Some(builder.allow[b as usize] & bit != 0)
                };
                let (before, after) = (char::from_u32(b).unwrap(), char::from_u32(a).unwrap());
                assert_eq!(
                    CHROMIUM_LINE_BREAK_TABLE.lookup(before, after),
                    dense,
                    "mismatch at (before={b:#04x}, after={a:#04x})",
                );
            }
        }
    }
}
