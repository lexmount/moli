use crate::*;
use std::cmp::Ordering as CmpOrdering;

#[test]
fn key_range_dto_matches_ordered_bounds_and_zero_identity() {
    let range = KeyRange {
        lower: Key::number(-0.0),
        upper: Some(Key::Date(0)),
        lower_open: false,
        upper_open: true,
    };
    range.validate().unwrap();
    assert!(range.contains(&Key::from(0)));
    assert!(range.contains(&Key::from(1)));
    assert!(!range.contains(&Key::Date(0)));
    assert!(!range.contains(&Key::from(-1)));
    assert!(KeyRange::default().contains(&Key::Array(vec![])));
    for (lower, upper, lower_open) in [(2, 1, false), (1, 1, true)] {
        assert!(
            KeyRange {
                lower: Some(Key::from(lower)),
                upper: Some(Key::from(upper)),
                lower_open,
                upper_open: false,
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn cursor_direction_labels_and_flags_follow_spec_tokens() {
    let cases = [
        ("next", CursorDirection::Next, false, false),
        ("nextunique", CursorDirection::NextUnique, false, true),
        ("prev", CursorDirection::Prev, true, false),
        ("prevunique", CursorDirection::PrevUnique, true, true),
    ];

    for (label, direction, reverse, unique) in cases {
        assert_eq!(CursorDirection::parse(label), Some(direction));
        assert_eq!(direction.as_str(), label);
        assert_eq!(direction.is_reverse(), reverse);
        assert_eq!(direction.is_unique(), unique);
    }
    assert_eq!(CursorDirection::parse("forward"), None);
    assert_eq!(CursorDirection::default_next(), CursorDirection::Next);
}

#[test]
fn cursor_direction_helpers_reverse_and_deduplicate_by_key() {
    let entries = vec![
        (Key::from("a"), 1),
        (Key::from("a"), 2),
        (Key::from("b"), 3),
    ];

    assert_eq!(
        apply_collection_direction(entries.clone(), CursorDirection::Prev),
        vec![
            (Key::from("b"), 3),
            (Key::from("a"), 2),
            (Key::from("a"), 1),
        ]
    );

    assert_eq!(
        apply_cursor_direction_by_key(entries.clone(), CursorDirection::NextUnique, |entry| {
            &entry.0
        }),
        vec![(Key::from("a"), 1), (Key::from("b"), 3)]
    );
    assert_eq!(
        apply_cursor_direction_by_key(entries, CursorDirection::PrevUnique, |entry| &entry.0),
        vec![(Key::from("b"), 3), (Key::from("a"), 1)]
    );
}

#[test]
fn cursor_direction_comparisons_respect_reverse_order() {
    assert_eq!(
        compare_cursor_direction(CursorDirection::Next, &Key::from(2), &Key::from(1)),
        CmpOrdering::Greater
    );
    assert_eq!(
        compare_cursor_direction(CursorDirection::Prev, &Key::from(2), &Key::from(1)),
        CmpOrdering::Less
    );
    assert_eq!(
        compare_cursor_tuple_direction(
            CursorDirection::Next,
            &Key::from(2),
            &Key::from(1),
            &Key::from(2),
            &Key::from(0),
        ),
        CmpOrdering::Greater
    );
    assert_eq!(
        compare_cursor_tuple_direction(
            CursorDirection::Prev,
            &Key::from(2),
            &Key::from(1),
            &Key::from(2),
            &Key::from(0),
        ),
        CmpOrdering::Less
    );
}

#[test]
fn transaction_mode_uses_indexeddb_labels() {
    use std::str::FromStr;

    assert_eq!(
        TransactionMode::from_str("readonly"),
        Ok(TransactionMode::ReadOnly)
    );
    assert_eq!(
        TransactionMode::from_str("readwrite"),
        Ok(TransactionMode::ReadWrite)
    );
    assert_eq!(
        TransactionMode::from_str("versionchange"),
        Ok(TransactionMode::VersionChange)
    );
    let readonly: &'static str = TransactionMode::ReadOnly.into();
    let readwrite: &'static str = TransactionMode::ReadWrite.into();
    let versionchange: &'static str = TransactionMode::VersionChange.into();
    assert_eq!(readonly, "readonly");
    assert_eq!(readwrite, "readwrite");
    assert_eq!(versionchange, "versionchange");
    assert!(TransactionMode::from_str("read-write").is_err());
}

#[test]
fn reverse_unique_index_cursor_keeps_first_primary_key_for_each_index_key() {
    let entries = vec![
        (Key::from("a"), 1),
        (Key::from("a"), 2),
        (Key::from("b"), 3),
        (Key::from("b"), 4),
        (Key::from("c"), 5),
        (Key::from("c"), 6),
    ];
    let actual =
        apply_cursor_direction_by_key(entries, CursorDirection::PrevUnique, |entry| &entry.0);
    assert_eq!(
        actual,
        vec![
            (Key::from("c"), 5),
            (Key::from("b"), 3),
            (Key::from("a"), 1)
        ]
    );
}
