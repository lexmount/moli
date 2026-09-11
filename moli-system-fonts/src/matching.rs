// SPDX-License-Identifier: (MIT OR Apache-2.0) AND BSD-3-Clause
// Metric compatibility data adapted from Skia, Copyright 2009-2015 Google Inc.
// Skia's BSD-3-Clause notice is retained at the end of this module.

// Chromium's Linux FCI path distinguishes metric-compatible replacements from
// Fontconfig's best-effort fallbacks. See SkFontConfigInterface_direct.cpp,
// GetFontEquivClass / MatchFont. Keep the order: some CJK replacement names
// occur in multiple groups and Chromium uses the first matching group.
const METRIC_COMPATIBLE_FAMILIES: &[&[&str]] = &[
    &["Arial", "Arimo", "Liberation Sans"],
    &["Times New Roman", "Tinos", "Liberation Serif"],
    &["Courier New", "Cousine", "Liberation Mono"],
    &["Symbol", "Symbol Neu"],
    &[
        "MS PGothic",
        "ＭＳ Ｐゴシック",
        "Noto Sans CJK JP",
        "IPAPGothic",
        "MotoyaG04Gothic",
    ],
    &[
        "MS Gothic",
        "ＭＳ ゴシック",
        "Noto Sans Mono CJK JP",
        "IPAGothic",
        "MotoyaG04GothicMono",
    ],
    &[
        "MS PMincho",
        "ＭＳ Ｐ明朝",
        "Noto Serif CJK JP",
        "IPAPMincho",
        "MotoyaG04Mincho",
    ],
    &[
        "MS Mincho",
        "ＭＳ 明朝",
        "Noto Serif CJK JP",
        "IPAMincho",
        "MotoyaG04MinchoMono",
    ],
    &[
        "Simsun",
        "宋体",
        "Noto Serif CJK SC",
        "MSung GB18030",
        "Song ASC",
    ],
    &[
        "NSimsun",
        "新宋体",
        "Noto Serif CJK SC",
        "MSung GB18030",
        "N Song ASC",
    ],
    &[
        "Simhei",
        "黑体",
        "Noto Sans CJK SC",
        "MYingHeiGB18030",
        "MYingHeiB5HK",
    ],
    &["PMingLiU", "新細明體", "Noto Serif CJK TC", "MSung B5HK"],
    &["MingLiU", "細明體", "Noto Serif CJK TC", "MSung B5HK"],
    &[
        "PMingLiU_HKSCS",
        "新細明體_HKSCS",
        "Noto Serif CJK TC",
        "MSung B5HK",
    ],
    &[
        "MingLiU_HKSCS",
        "細明體_HKSCS",
        "Noto Serif CJK TC",
        "MSung B5HK",
    ],
    &["Cambria", "Caladea"],
    &["Calibri", "Carlito"],
];

pub(super) fn accepts_substitution(requested: &str, configured: &str, matched: &str) -> bool {
    if ["", "sans", "serif", "monospace"]
        .iter()
        .any(|generic| requested.eq_ignore_ascii_case(generic))
        || matched.eq_ignore_ascii_case(requested)
        || matched.eq_ignore_ascii_case(configured)
    {
        return true;
    }
    metric_group(requested).is_some_and(|group| Some(group) == metric_group(matched))
}

fn metric_group(family: &str) -> Option<usize> {
    METRIC_COMPATIBLE_FAMILIES
        .iter()
        .position(|group| group.iter().any(|name| name.eq_ignore_ascii_case(family)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_effort_aliases_do_not_make_missing_named_fonts_available() {
        assert!(!accepts_substitution("Century", "Century", "C059"));
        assert!(!accepts_substitution(
            "Bitstream Vera Sans Mono",
            "Bitstream Vera Sans Mono",
            "DejaVu Sans Mono"
        ));
        assert!(!accepts_substitution("missing", "missing", "DejaVu Sans"));
        assert!(!accepts_substitution("Arial", "Arial", "DejaVu Sans"));
    }

    #[test]
    fn configured_renames_and_requested_family_matches_remain_available() {
        assert!(accepts_substitution("Alias", "Real Font", "Real Font"));
        assert!(accepts_substitution("Real Font", "Alias", "real font"));
        assert!(!accepts_substitution("Alias", "Real Font", "Unrelated"));
    }

    #[test]
    fn metric_compatible_replacements_are_case_insensitive() {
        for (requested, matched) in [
            ("arial", "Liberation Sans"),
            ("Times New Roman", "Tinos"),
            ("Courier New", "Cousine"),
            ("Cambria", "Caladea"),
            ("Calibri", "Carlito"),
            ("MS PGothic", "Noto Sans CJK JP"),
            ("宋体", "Simsun"),
        ] {
            assert!(accepts_substitution(requested, requested, matched));
        }
        assert!(!accepts_substitution("Arial", "Arial", "Liberation Serif"));
    }

    #[test]
    fn shared_cjk_names_use_the_first_equivalence_group() {
        assert!(accepts_substitution(
            "MS PMincho",
            "MS PMincho",
            "Noto Serif CJK JP"
        ));
        assert!(!accepts_substitution(
            "MS Mincho",
            "MS Mincho",
            "Noto Serif CJK JP"
        ));
    }

    #[test]
    fn generic_fallback_requests_allow_platform_defaults() {
        for family in ["", "sans", "serif", "MONOSPACE"] {
            assert!(accepts_substitution(family, family, "Platform Default"));
        }
    }
}

/*
Skia license:

Copyright (c) 2011 Google Inc. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of the copyright holder nor the names of its
    contributors may be used to endorse or promote products derived
    from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/
