use super::*;
use crate::sqlite::tests::{TX, begin, database, put};

fn indexed() -> Database {
    let mut owner = database();
    owner
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    owner
        .create_index(
            TX,
            "a",
            "index",
            IndexOptions {
                key_path: "key".into(),
                unique: false,
                multi_entry: false,
            },
        )
        .unwrap();
    for (primary, key) in [(1, "a"), (2, "a"), (3, "b"), (4, "b"), (5, "c")] {
        put(
            &mut owner,
            Some(primary.into()),
            &[primary as u8],
            vec![IndexKeys {
                name: "index".into(),
                keys: vec![key.into()],
            }],
            None,
        )
        .unwrap();
    }
    owner.commit(TX).unwrap();
    begin(&mut owner, TransactionMode::ReadWrite);
    owner
}

fn query(direction: CursorDirection) -> Query {
    Query {
        source: QuerySource::Index("index".into()),
        direction,
        limit: 100,
        keys_only: true,
        ..Default::default()
    }
}

fn primary_keys(owner: &Database, query: &Query) -> Vec<Key> {
    owner
        .query(TX, "a", query)
        .unwrap()
        .into_iter()
        .map(|entry| entry.primary_key)
        .collect()
}

#[test]
fn index_directions_and_reverse_unique_select_smallest_primary_key() {
    let mut owner = indexed();
    for (direction, expected) in [
        (CursorDirection::Next, vec![1, 2, 3, 4, 5]),
        (CursorDirection::Prev, vec![5, 4, 3, 2, 1]),
        (CursorDirection::NextUnique, vec![1, 3, 5]),
        (CursorDirection::PrevUnique, vec![5, 3, 1]),
    ] {
        assert_eq!(
            primary_keys(&owner, &query(direction)),
            expected.into_iter().map(Key::from).collect::<Vec<_>>()
        );
    }
    owner.abort(TX).unwrap();
}

#[test]
fn tuple_seek_inclusive_targets_and_advance_stay_inside_range() {
    let mut owner = indexed();
    let mut options = query(CursorDirection::Next);
    options.seek = Some(Seek {
        key: "a".into(),
        primary_key: Some(1.into()),
        inclusive: false,
    });
    assert_eq!(primary_keys(&owner, &options), [2, 3, 4, 5].map(Key::from));
    options.seek.as_mut().unwrap().inclusive = true;
    assert_eq!(
        primary_keys(&owner, &options),
        [1, 2, 3, 4, 5].map(Key::from)
    );
    options.direction = CursorDirection::Prev;
    options.seek = Some(Seek {
        key: "b".into(),
        primary_key: Some(3.into()),
        inclusive: false,
    });
    assert_eq!(primary_keys(&owner, &options), [2, 1].map(Key::from));
    options.direction = CursorDirection::PrevUnique;
    assert_eq!(primary_keys(&owner, &options), [1].map(Key::from));
    options.seek = None;
    options.skip = 1;
    options.limit = 1;
    assert_eq!(primary_keys(&owner, &options), [3].map(Key::from));
    options = query(CursorDirection::Next);
    options.seek = Some(Seek {
        key: "b".into(),
        primary_key: None,
        inclusive: true,
    });
    options.range.upper = Some("c".into());
    options.range.upper_open = true;
    assert_eq!(primary_keys(&owner, &options), [3, 4].map(Key::from));
    assert_eq!(
        owner
            .query_count(TX, "a", &options.source, &options.range)
            .unwrap(),
        4
    );
    owner.abort(TX).unwrap();
}

#[test]
fn primary_range_and_count_preserve_all_key_types_and_open_boundaries() {
    let mut owner = database();
    begin(&mut owner, TransactionMode::ReadWrite);
    let mut keys = vec![
        Key::number(f64::NEG_INFINITY).unwrap(),
        Key::number(-0.0).unwrap(),
        Key::number(f64::INFINITY).unwrap(),
        Key::Date(-1),
        Key::String(vec![]),
        Key::String(vec![0, 0xd800]),
        Key::Binary(vec![]),
        Key::Binary(vec![0, 255]),
        Key::Array(vec![]),
        Key::Array(vec![1.into(), "x".into()]),
    ];
    for key in keys.iter().rev() {
        put(&mut owner, Some(key.clone()), b"value", vec![], None).unwrap();
    }
    keys.sort();
    let mut options = Query {
        limit: 100,
        keys_only: true,
        ..Default::default()
    };
    assert_eq!(primary_keys(&owner, &options), keys);
    options.range = KeyRange {
        lower: Some(Key::Date(-1)),
        upper: Some(Key::Binary(vec![])),
        lower_open: true,
        upper_open: false,
    };
    assert_eq!(primary_keys(&owner, &options), keys[4..=6]);
    assert_eq!(
        owner
            .query_count(TX, "a", &QuerySource::ObjectStore, &options.range)
            .unwrap(),
        3
    );
    options.range.upper = Some(1.into());
    assert!(matches!(
        owner.query(TX, "a", &options),
        Err(IndexedDbError::Data(_))
    ));
    owner.abort(TX).unwrap();
}

#[test]
fn cursor_seek_observes_same_transaction_insert_delete_and_value_update() {
    let mut owner = database();
    begin(&mut owner, TransactionMode::ReadWrite);
    for key in [1, 2, 3] {
        put(&mut owner, Some(key.into()), b"old", vec![], None).unwrap();
    }
    let mut options = Query::default();
    let first = owner.query(TX, "a", &options).unwrap().pop().unwrap();
    assert_eq!(first.primary_key, Key::from(1));
    let between = Key::number(1.5).unwrap();
    put(&mut owner, Some(between.clone()), b"inserted", vec![], None).unwrap();
    options.seek = Some(Seek {
        key: first.key,
        primary_key: Some(first.primary_key),
        inclusive: false,
    });
    assert_eq!(
        primary_keys(&owner, &options),
        std::slice::from_ref(&between)
    );
    owner.delete(TX, "a", &between).unwrap();
    owner.delete(TX, "a", &2.into()).unwrap();
    put(&mut owner, Some(3.into()), b"updated", vec![], None).unwrap();
    options.seek = Some(Seek {
        key: between,
        primary_key: None,
        inclusive: false,
    });
    let next = owner.query(TX, "a", &options).unwrap().pop().unwrap();
    assert_eq!(next.primary_key, Key::from(3));
    assert_eq!(next.value.unwrap().wire_bytes(), b"updated");
    owner.abort(TX).unwrap();
}

#[test]
fn key_projections_and_count_never_read_record_bodies() {
    let mut owner = indexed();
    owner
        .connection
        .execute("DELETE FROM record_chunks", [])
        .unwrap();
    for source in [QuerySource::ObjectStore, QuerySource::Index("index".into())] {
        let options = Query {
            source,
            limit: 100,
            keys_only: true,
            ..Default::default()
        };
        assert_eq!(owner.query(TX, "a", &options).unwrap().len(), 5);
        assert_eq!(
            owner
                .query_count(TX, "a", &options.source, &options.range)
                .unwrap(),
            5
        );
        assert!(matches!(
            owner.query(
                TX,
                "a",
                &Query {
                    keys_only: false,
                    ..options
                }
            ),
            Err(IndexedDbError::Corruption(_))
        ));
    }
    owner.abort(TX).unwrap();
}

#[test]
fn query_reports_dangling_index_record_and_inconsistent_unique_flag() {
    for damage in [
        "UPDATE index_entries SET record_id=999",
        "UPDATE index_entries SET is_unique=1",
    ] {
        let mut owner = indexed();
        // Test a uniqueness mismatch on a single key to avoid SQL's unique
        // constraint rejecting the deliberately corrupted metadata first.
        owner.connection.execute("DELETE FROM index_entries WHERE primary_key_sort != (SELECT min(primary_key_sort) FROM index_entries)", []).unwrap();
        owner.connection.execute_batch(damage).unwrap();
        assert!(
            matches!(
                owner.query(TX, "a", &query(CursorDirection::Next)),
                Err(IndexedDbError::Corruption(_))
            ),
            "{damage}"
        );
        owner.abort(TX).unwrap();
    }
}

#[test]
fn seek_queries_use_btree_search_without_a_temp_sort() {
    let mut owner = indexed();
    for direction in [
        CursorDirection::Next,
        CursorDirection::Prev,
        CursorDirection::NextUnique,
        CursorDirection::PrevUnique,
    ] {
        let options = Query {
            seek: Some(Seek {
                key: "a".into(),
                primary_key: Some(1.into()),
                inclusive: false,
            }),
            ..query(direction)
        };
        let compiled = owner.compile_query(TX, &"a".into(), &options).unwrap();
        let mut statement = owner
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql))
            .unwrap();
        let details = statement
            .query_map(params_from_iter(compiled.parameters.iter()), |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("SEARCH e USING PRIMARY KEY")),
            "{direction:?}: {details:?}"
        );
        assert!(
            details
                .iter()
                .all(|detail| !detail.contains("TEMP B-TREE") && !detail.starts_with("SCAN ")),
            "{direction:?}: {details:?}"
        );
    }
    owner.abort(TX).unwrap();
}

#[test]
fn range_delete_removes_only_selected_records_and_their_index_entries() {
    let mut owner = indexed();
    let initial_usage = owner.candidate_metadata(TX).unwrap().usage;
    owner
        .delete_range(
            TX,
            "a",
            &KeyRange {
                lower: Some(1.into()),
                upper: Some(4.into()),
                lower_open: true,
                upper_open: false,
            },
        )
        .unwrap();
    let options = Query {
        limit: 100,
        keys_only: true,
        ..Default::default()
    };
    assert_eq!(primary_keys(&owner, &options), [1, 5].map(Key::from));
    assert_eq!(
        primary_keys(&owner, &query(CursorDirection::Next)),
        [1, 5].map(Key::from)
    );
    assert_eq!(
        owner.candidate_metadata(TX).unwrap().usage,
        initial_usage - 3 * (8 + 1)
    );
    assert_eq!(
        owner
            .connection
            .query_row("SELECT count(*) FROM record_chunks", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    owner.abort(TX).unwrap();
    begin(&mut owner, TransactionMode::ReadOnly);
    assert_eq!(owner.count(TX, "a").unwrap(), 5);
    assert!(matches!(
        owner.delete_range(TX, "a", &KeyRange::default()),
        Err(IndexedDbError::ReadOnly(_))
    ));
    owner.abort(TX).unwrap();
}
