// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{string::ToString, vec::Vec};
use parlance::BidiLevel;

use crate::{Analysis, AnalysisOptions, Analyzer, BaseDirection, BidiObject, InlineBoxBidi};

fn options(base_direction: BaseDirection) -> AnalysisOptions<'static> {
    AnalysisOptions {
        base_direction,
        ..AnalysisOptions::default()
    }
}

fn verify_against_replacement_characters(text: &str, indices: &[usize], direction: BaseDirection) {
    let mut analyzer = Analyzer::new();
    let mut analysis = Analysis::new();
    analyzer.analyze(text, &options(direction), &mut analysis);
    let source_info = analysis.char_info().to_vec();
    let mut objects = indices
        .iter()
        .map(|&index| BidiObject::new(index))
        .collect::<Vec<_>>();
    analyzer.analyze_with_objects(text, &options(direction), &mut objects, &mut analysis);

    // Compare with the explicit replacement-character reference path using the same resolver.
    // Insert in reverse order so the supplied source offsets remain valid.
    let mut replacement_text = text.to_string();
    for &index in indices.iter().rev() {
        replacement_text.insert(index, '\u{fffc}');
    }
    let mut reference = Analysis::new();
    analyzer.analyze(&replacement_text, &options(direction), &mut reference);
    let level_at = |analysis: &Analysis, index: usize| {
        let levels = analysis.bidi_levels();
        if levels.is_empty() {
            analysis.paragraph_level()
        } else {
            levels[index]
        }
    };

    assert_eq!(
        analysis.char_info(),
        source_info,
        "source segmentation: {text:?}"
    );
    assert_eq!(analysis.paragraph_level(), reference.paragraph_level());
    assert!(
        analysis.bidi_levels().is_empty() || analysis.bidi_levels().len() == text.chars().count(),
        "virtual characters must not appear in source levels"
    );
    for (ordinal, object) in objects.iter().enumerate() {
        let replacement_index = text[..object.index()].chars().count() + ordinal;
        assert_eq!(
            object.index(),
            indices[ordinal],
            "source anchors must be retained"
        );
        assert_eq!(
            object.level(),
            level_at(&reference, replacement_index),
            "object {ordinal}: {text:?}/{indices:?}/{direction:?}"
        );
    }
    for (char_index, (byte_index, _)) in text.char_indices().enumerate() {
        let preceding_objects = indices.iter().filter(|&&index| index <= byte_index).count();
        assert_eq!(
            level_at(&analysis, char_index),
            level_at(&reference, char_index + preceding_objects),
            "source character {char_index}: {text:?}/{indices:?}/{direction:?}"
        );
    }
}

#[test]
fn objects_match_explicit_replacement_characters() {
    for text in [
        "",
        "hello world",
        "123 / 456",
        "aאבb",
        "אב 12 + 34 x",
        "ع١٢(34) x",
        "😀éאבx",
        "אב\nx\r\nאב",
        "אב\u{2028}x\u{2029}אב",
        "\u{202e}x\u{202c}",
        "\u{202b}x\u{202c}",
        "\u{2066}אב\u{2069}",
        "\u{2067}x\u{2069}",
        "\u{2068}אבx\u{2069}",
        "\u{2067}\u{202e}x\u{202c}\u{2069}",
    ] {
        let boundaries = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(core::iter::once(text.len()))
            .collect::<Vec<_>>();
        for direction in [BaseDirection::Auto, BaseDirection::Ltr, BaseDirection::Rtl] {
            verify_against_replacement_characters(text, &[], direction);
            verify_against_replacement_characters(text, &boundaries, direction);
            for &index in &boundaries {
                verify_against_replacement_characters(text, &[index], direction);
                verify_against_replacement_characters(text, &[index, index], direction);
            }
        }
    }
}

#[test]
fn objects_inside_and_after_overrides_have_distinct_levels() {
    let text = "😀\u{202e}אב\u{202c}x";
    let mut objects = [
        BidiObject::new("😀\u{202e}".len()),
        BidiObject::new(text.len()),
    ];
    let mut analysis = Analysis::new();
    Analyzer::new().analyze_with_objects(
        text,
        &options(BaseDirection::Ltr),
        &mut objects,
        &mut analysis,
    );
    assert_eq!(objects[0].level(), BidiLevel::new(1));
    assert_eq!(objects[1].level(), BidiLevel::new(0));
}

#[test]
fn reused_objects_and_analysis_reset_on_ltr_fast_path() {
    let mut analyzer = Analyzer::new();
    let mut analysis = Analysis::new();
    let mut objects = [BidiObject::new(0)];
    analyzer.analyze_with_objects(
        "אב",
        &options(BaseDirection::Rtl),
        &mut objects,
        &mut analysis,
    );
    assert_eq!(objects[0].level(), BidiLevel::new(1));
    assert!(!analysis.bidi_levels().is_empty());
    analyzer.analyze_with_objects(
        "hello",
        &options(BaseDirection::Auto),
        &mut objects,
        &mut analysis,
    );
    assert_eq!(objects[0].level(), BidiLevel::new(0));
    assert!(analysis.bidi_levels().is_empty());
    analyzer.analyze("next", &options(BaseDirection::Auto), &mut analysis);
    assert!(analysis.bidi_levels().is_empty());
    assert_eq!(analysis.char_info().len(), 4);
}

#[test]
#[should_panic(expected = "object index must be a character boundary within text")]
fn rejects_anchor_inside_utf8_character() {
    verify_against_replacement_characters("😀", &[2], BaseDirection::Ltr);
}

#[test]
#[should_panic(expected = "object index must be a character boundary within text")]
fn rejects_anchor_past_eof() {
    verify_against_replacement_characters("", &[1], BaseDirection::Ltr);
}

#[test]
#[should_panic(expected = "objects must be sorted by index")]
fn rejects_unsorted_anchors() {
    verify_against_replacement_characters("ab", &[2, 0], BaseDirection::Ltr);
}

#[test]
fn transparent_markers_follow_assigned_items_and_next_participants() {
    use InlineBoxBidi::{InheritNext as Next, InheritPrevious as Previous, Neutral};
    for (text, direction, index, participation, expected) in [
        (
            "aאבb",
            BaseDirection::Ltr,
            1,
            &[Previous, Next, Previous][..],
            &[0, 1, 1][..],
        ),
        (
            "aאבb",
            BaseDirection::Ltr,
            1,
            &[Next, Neutral, Next, Previous][..],
            &[0, 0, 1, 1][..],
        ),
        (
            "ab",
            BaseDirection::Rtl,
            2,
            &[Previous, Next, Previous][..],
            &[2, 1, 1][..],
        ),
        (
            "",
            BaseDirection::Rtl,
            0,
            &[Next, Previous][..],
            &[1, 1][..],
        ),
    ] {
        let mut analyzer = Analyzer::new();
        let mut source = Analysis::new();
        analyzer.analyze(text, &options(direction), &mut source);
        let mut analysis = Analysis::new();
        let mut objects = participation
            .iter()
            .map(|&participation| BidiObject::with_participation(index, participation))
            .collect::<Vec<_>>();
        analyzer.analyze_with_objects(text, &options(direction), &mut objects, &mut analysis);
        assert_eq!(analysis.char_info(), source.char_info());
        for ((object, &participation), &expected) in objects.iter().zip(participation).zip(expected)
        {
            assert_eq!(object.participation(), participation);
            assert_eq!(object.index(), index);
            assert_eq!(
                object.level(),
                BidiLevel::new(expected),
                "{text:?}/{direction:?}"
            );
        }
        if !participation.contains(&Neutral) {
            assert_eq!(
                analysis.bidi_levels(),
                source.bidi_levels(),
                "transparent markers do not change text levels"
            );
        }
    }
}

#[test]
fn reused_transparent_markers_reset_on_ltr_fast_path() {
    let mut analyzer = Analyzer::new();
    let mut analysis = Analysis::new();
    let mut objects = [
        BidiObject::with_participation(0, InlineBoxBidi::InheritNext),
        BidiObject::with_participation(0, InlineBoxBidi::InheritPrevious),
    ];
    analyzer.analyze_with_objects(
        "אב",
        &options(BaseDirection::Rtl),
        &mut objects,
        &mut analysis,
    );
    assert!(
        objects
            .iter()
            .all(|object| object.level() == BidiLevel::new(1))
    );
    analyzer.analyze_with_objects(
        "hello",
        &options(BaseDirection::Auto),
        &mut objects,
        &mut analysis,
    );
    assert!(
        objects
            .iter()
            .all(|object| object.level() == BidiLevel::new(0))
    );
    assert!(analysis.bidi_levels().is_empty());
}
