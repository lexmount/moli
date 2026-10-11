// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Seeded random generation of line-breaking test cases.
//!
//! We use these to ensure that our Line breaking decision exactly matches Chromium (or can be
//! made to do so). This is validated in the `chrome_linebreaking.rs` test.
//!
//! The text is restricted to printable ASCII. There are also no sequences of more
//! than one space, because Parley doesn't yet have web-compatible collapsing of multiple spaces.
//!
//! Each case is produced by one of several [`Strategy`]s:
//!  - [`Strategy::Alphanumeric`] — `[A-Za-z0-9]` words separated by spaces.
//!  - [`Strategy::Templated`] — semi-realistic generated strings to exercise the
//!    most expected special cases.
//!  - [`Strategy::Targeted`] — strings deliberately constructed to hit each
//!    special-rule equivalence class in Chromium's override table.
//!  - [`Strategy::Soup`] — uniform random printable ASCII.
//!
//! The boundary cases are calculated by running `parley_tests/linebreaking_browser_recorder`
//! in the browser, then saved to `parley_tests/linebreaking_browser_recorder/data`, with a
//! file per family.

use rand::{RngExt, SeedableRng, seq::IndexedRandom};
use rand_chacha::ChaCha8Rng;

// --- Shared, non-case types between runners ---

/// A font the harness collects data for.
#[derive(Clone, Copy, Debug)]
pub struct SupportedFont {
    /// The family name for the font, also used as the key for the data files.
    pub family: &'static str,
    /// The raw font file bytes, embedded so the browser harness needs no
    /// network or filesystem access.
    pub bytes: &'static [u8],
}

/// The fonts which we compare Chrome's line breaking against.
pub const FONTS: &[SupportedFont] = &[
    SupportedFont {
        family: "Roboto",
        bytes: include_bytes!("../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf"),
    },
    SupportedFont {
        family: "Arimo",
        bytes: include_bytes!(
            "../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf"
        ),
    },
];

/// We perform all measurements as an integer number of 64th of a pixel subpixels.
///
/// This is particularly useful for binary search, to avoid floating point weirdness.
/// We don't need to go finer, as we know that Chrome's box layout is performed at this granularity.
pub const SUBPIXELS_PER_PX: f64 = 64.0;

/// The width we ask the browser to break at in cases where the first line
/// cannot be broken within. This can happen in two cases:
/// 1) Where the initial width is inside the first "word".
/// 2) Where the initial width is inside the second "word"; in that
///    case, the only thing left on the first line would be the first word.
///
/// We haven't reasoned about whether starting from zero would work here.
pub const PROBE_SUBPIXELS: i64 = 64; // 1px

// --- Character classes ---
// These are byte strings to allow selection using rand's `choose` function.

/// Letters and digits: the body of "plain" words and the filler around punctuation.
const ALPHANUMERIC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
/// Letters only, for token parts that should not contain digits (e.g. URL hosts).
const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
/// Decimal digits.
const DIGIT: &[u8] = b"0123456789";

// --- Case generation ---

const MIN_FONT_SIZE: f32 = 10.0;
const MAX_FONT_SIZE: f32 = 30.0;
const FONT_SIZE_STEP: f32 = 0.02;
#[expect(
    clippy::cast_possible_truncation,
    reason = "We know this doesn't overflow."
)]
const FONT_SIZE_STEPS: u32 = ((MAX_FONT_SIZE - MIN_FONT_SIZE) / FONT_SIZE_STEP) as u32;

/// Inclusive bounds on the container width as a multiplier of the font size.
const MIN_EM_FACTOR: f32 = 7.0;
const MAX_EM_FACTOR: f32 = 32.0;

/// Inclusive bounds on the length (in characters) when we need to generate 'words'.
const MIN_WORD_LEN: usize = 2;
const MAX_WORD_LEN: usize = 10;

/// The minimum length of a text output.
const MIN_LEN: usize = 50;

/// A single seed-derived line-breaking test case.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// The seed this case was generated from.
    pub seed: u64,
    /// The generated ASCII text.
    pub text: String,
    /// Font size in CSS pixels.
    pub font_size: f32,
    /// Initial container width in CSS pixels (`font_size * em_factor`). The
    /// Chromium harness uses this to decide where the first line breaks before tightening
    /// the width down to the minimum that preserves that break. This shall not be used by
    /// the Parley test case validator.
    pub initial_width: f32,
    /// The strategy used to generate this case.
    pub strategy: Strategy,
}

/// A text generation algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// `[A-Za-z0-9]` words, separated by single spaces.
    Alphanumeric,
    /// Realistic strings following a variety of templates.
    Templated,
    /// Strings to target various punctuation.
    Targeted,
    /// Uniform random printable ASCII.
    ///
    /// This avoids having double spaces, which we know
    /// we don't implement properly.
    Soup,
}

impl Case {
    /// Generate the [`Case`] for a given seed.
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        // For some font sizes, Chrome has an order-dependent cache collision issue (for example,
        // font size 16.79 and 16.80 get the same key, so the first to be used defines the
        // size of the other).
        // No reasonable app will have font sizes that close to each other, so we
        // sample in such a way to miss these collisions.
        let font_size =
            MIN_FONT_SIZE + rng.random_range(0..=FONT_SIZE_STEPS) as f32 * FONT_SIZE_STEP;
        // However, in real world conditions, we do want to make sure that we handle font sizes
        // which don't neatly fall on that grid, so add an offset which still maintains the non-collision.
        let offset = rng.random_range(0_f32..0.0095_f32);
        let font_size = font_size + offset;
        let em_factor = rng.random_range(MIN_EM_FACTOR..=MAX_EM_FACTOR);
        let initial_width = font_size * em_factor;

        let strategy = Strategy::select(&mut rng);
        let text = strategy.generate(&mut rng);
        Self {
            seed,
            text,
            font_size,
            initial_width,
            strategy,
        }
    }
}

impl Strategy {
    const WEIGHTS: &[(Self, u32)] = &[
        // We want the alphanumeric case to be slightly rarer.
        (Self::Alphanumeric, 15),
        (Self::Templated, 35),
        (Self::Targeted, 30),
        (Self::Soup, 30),
    ];

    fn select(rng: &mut ChaCha8Rng) -> Self {
        let total: u32 = Self::WEIGHTS.iter().map(|(_, w)| w).sum();
        let mut r = rng.random_range(0..total);
        for &(strategy, weight) in Self::WEIGHTS {
            if r < weight {
                return strategy;
            }
            r -= weight;
        }
        unreachable!()
    }

    /// Generate text according to the given strategy.
    ///
    /// The output text will always be at least [`MIN_LEN`] characters.
    fn generate(&self, rng: &mut ChaCha8Rng) -> String {
        // Note that the strategies each assume that the input text is empty.
        let mut text = String::with_capacity(MIN_LEN + 32);
        match self {
            Self::Alphanumeric => Self::alphanumeric(rng, &mut text),
            Self::Templated => Self::templated(rng, &mut text),
            Self::Targeted => Self::targeted(rng, &mut text),
            Self::Soup => Self::soup(rng, &mut text),
        }
        text
    }

    /// Implements [`Self::Alphanumeric`].
    fn alphanumeric(rng: &mut ChaCha8Rng, out: &mut String) {
        while out.len() < MIN_LEN {
            // Avoid starting with a space (see https://github.com/linebender/parley/issues/638).
            if !out.is_empty() {
                out.push(' ');
            }
            fill(rng, ALPHANUMERIC, out, MIN_WORD_LEN, MAX_WORD_LEN);
        }
    }

    /// Implements [`Self::Soup`].
    fn soup(rng: &mut ChaCha8Rng, out: &mut String) {
        // We can only allow a single space at once, as Chrome and Parley (currently)
        // have different handling of whitespace runs.
        // Initialised to true to avoid starting with a space.
        let mut prev_was_space = true;
        for _ in 0..MIN_LEN {
            let chr = if prev_was_space {
                char::from(rng.random_range(b'!'..=b'~'))
            } else {
                char::from(rng.random_range(b' '..=b'~'))
            };
            prev_was_space = chr == ' ';
            out.push(chr);
        }
        // Avoid a final trailing whitespace.
        if prev_was_space {
            out.push('a');
        }
    }

    /// Implements [`Self::Templated`].
    fn templated(rng: &mut ChaCha8Rng, out: &mut String) {
        while out.len() < MIN_LEN {
            if !out.is_empty() && rng.random_bool(0.5) {
                out.push(' ');
            }
            match rng.random_range(0..12_u32) {
                // URL
                0 => {
                    out.push_str("https://");
                    fill(rng, ALPHA, out, 2, 6);
                    out.push('.');
                    fill(rng, ALPHANUMERIC, out, 2, 3);
                    out.push('/');
                    fill(rng, ALPHANUMERIC, out, 2, 6);
                    out.push('-');
                    fill(rng, ALPHANUMERIC, out, 2, 6);
                    out.push('?');
                    fill(rng, ALPHANUMERIC, out, 1, 4);
                    out.push('=');
                    fill(rng, ALPHANUMERIC, out, 1, 4);
                }
                // File path
                1 => {
                    fill(rng, ALPHANUMERIC, out, 2, 6);
                    out.push('/');
                    fill(rng, ALPHANUMERIC, out, 2, 6);
                    out.push('/');
                    fill(rng, ALPHANUMERIC, out, 2, 6);
                    out.push('.');
                    fill(rng, ALPHANUMERIC, out, 1, 3);
                }
                // ISO date: `dddd-dd-dd`.
                2 => {
                    fill(rng, DIGIT, out, 4, 4);
                    out.push('-');
                    fill(rng, DIGIT, out, 2, 2);
                    out.push('-');
                    fill(rng, DIGIT, out, 2, 2);
                }
                3 => {
                    let parts = rng.random_range(2..=3);
                    for part in 0..parts {
                        if part > 0 {
                            out.push('-');
                        }
                        fill(rng, ALPHA, out, 1, 6);
                    }
                }
                4 => {
                    fill(rng, DIGIT, out, 1, 4);
                    out.push('.');
                    fill(rng, DIGIT, out, 1, 3);
                }
                // Thousands-separated number.
                // TODO: Explore different locales, in case that matters here.
                5 => {
                    fill(rng, DIGIT, out, 1, 3);
                    out.push(',');
                    fill(rng, DIGIT, out, 3, 3);
                }
                // Word with a trailing parenthetical: `word(note)`.
                6 => {
                    fill(rng, ALPHANUMERIC, out, 2, 6);
                    let (open, close) = *[('(', ')'), ('[', ']'), ('{', '}')].choose(rng).unwrap();
                    out.push(open);
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    out.push(close);
                }
                // Quoted string: `"word"` or `'word'`.
                7 => {
                    let quote = char::from(*br#""'"#.choose(rng).unwrap());
                    out.push(quote);
                    fill(rng, ALPHANUMERIC, out, 1, 6);
                    out.push(quote);
                }
                // Question: `word?` or `word?word` (break allowed after `?`).
                8 => {
                    fill(rng, ALPHA, out, 1, 6);
                    out.push('?');
                    if rng.random_bool(0.5) {
                        fill(rng, ALPHA, out, 1, 6);
                    }
                }
                // Question followed by a quote: `?"word"` (no break after `?` here).
                9 => {
                    out.push('?');
                    let quote = char::from(*br#""'"#.choose(rng).unwrap());
                    out.push(quote);
                    fill(rng, ALPHANUMERIC, out, 1, 6);
                    out.push(quote);
                }
                10 => {
                    out.push_str([":-)", ";-)", ":-(", ":-]"].choose(rng).unwrap());
                }
                // Various maths operators.
                11 => {
                    fill(rng, ALPHA, out, 1, 5);
                    out.push(char::from(*b"/=<>+*".choose(rng).unwrap()));
                    fill(rng, ALPHA, out, 1, 5);
                }
                _ => unreachable!(),
            }
        }
    }

    /// Implements [`Self::Targeted`].
    fn targeted(rng: &mut ChaCha8Rng, out: &mut String) {
        let strategy = rng.random_range(0..9_u32);
        while out.len() < MIN_LEN {
            if !out.is_empty() && rng.random_bool(0.5) {
                out.push(' ');
            }
            // These strategies were based on Claude's understanding of the Chrome linebreaking table.
            // They haven't been human verified.
            match strategy {
                0 => {
                    fill(rng, ALPHANUMERIC, out, 1, 4);
                    let (close, open) = *[(')', '('), (']', '['), ('}', '{')].choose(rng).unwrap();
                    out.push(close);
                    out.push(open);
                    fill(rng, ALPHANUMERIC, out, 1, 4);
                }
                1 => {
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    out.push('-');
                    fill(rng, ALPHA, out, 1, 5);
                }
                2 => {
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    out.push('-');
                    fill(rng, DIGIT, out, 1, 4);
                }
                // When it looks like a negation, we avoid a break.
                3 => {
                    out.push('-');
                    fill(rng, DIGIT, out, 1, 4);
                }
                4 => {
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    out.push('-');
                    out.push('$');
                    fill(rng, ALPHANUMERIC, out, 0, 3);
                }
                5 => {
                    fill(rng, ALPHA, out, 1, 5);
                    out.push('?');
                    fill(rng, ALPHA, out, 1, 5);
                }
                6 => {
                    fill(rng, ALPHA, out, 1, 5);
                    out.push('?');
                    let quote = char::from(*br#""'"#.choose(rng).unwrap());
                    out.push(quote);
                    fill(rng, ALPHANUMERIC, out, 1, 4);
                    out.push(quote);
                }
                7 => {
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    out.push(char::from(*b"/.:;,".choose(rng).unwrap()));
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                }
                8 => {
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    let (open, close) = *[('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')]
                        .choose(rng)
                        .unwrap();
                    out.push(open);
                    fill(rng, ALPHANUMERIC, out, 1, 5);
                    out.push(close);
                }
                _ => unreachable!(),
            }
        }
    }
}

/// Append a number of characters between 'lo' and 'hi' chosen randomly from `set`.
fn fill(rng: &mut ChaCha8Rng, set: &[u8], out: &mut String, lo: usize, hi: usize) {
    let n = rng.random_range(lo..=hi);
    for _ in 0..n {
        out.push(char::from(*set.choose(rng).unwrap()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed() {
        for seed in 0..1024 {
            let case = Case::from_seed(seed);
            assert!(
                case.text.chars().all(|b| (' '..='~').contains(&b)),
                "seed {seed}: not fully printable ASCII"
            );
            assert!(
                !case.text.starts_with(' ') && !case.text.ends_with(' '),
                "seed {seed}: leading/trailing space"
            );
            assert!(
                case.text.split(' ').all(|word| !word.is_empty()),
                "seed {seed}: double space; Parley doesn't handle whitespace collapsing"
            );
            assert!(case.text.len() >= MIN_LEN);
            // Generation must be deterministic.
            assert_eq!(case, Case::from_seed(seed));
        }
    }
}
