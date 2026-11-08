use super::*;
use crate::CursorDirection;

fn paged(database: &Database, mut query: Query) -> Vec<crate::RecordSnapshot> {
    let mut result = Vec::new();
    loop {
        let page = database.query_page(TX, "a", &query).unwrap();
        assert!(page.records.len() <= 1000);
        assert!(!page.has_more || !page.records.is_empty());
        query.limit -= page.records.len() as u32;
        if let Some(last) = page.records.last() {
            query.seek = Some(Seek {
                key: last.key.clone(),
                primary_key: Some(last.primary_key.clone()),
                inclusive: false,
            });
            query.skip = 0;
        }
        result.extend(page.records);
        if !page.has_more || query.limit == 0 {
            return result;
        }
    }
}

#[test]
fn query_pages_preserve_index_tuples_direction_uniqueness_range_and_count() {
    let mut database = database();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(2))
        .unwrap();
    database
        .create_index(
            TX,
            "a",
            "group",
            IndexOptions {
                key_path: KeyPath::String("group".into()),
                unique: false,
                multi_entry: false,
            },
        )
        .unwrap();
    database.commit(TX).unwrap();
    begin(&mut database, TransactionMode::ReadWrite);
    for id in 1_i64..=2505 {
        put(
            &mut database,
            Some(id.into()),
            &[1, 2, 3],
            vec![IndexKeys {
                name: "group".into(),
                keys: vec![if id <= 2001 { "duplicate" } else { "tail" }.into()],
            }],
            None,
        )
        .unwrap();
    }
    database.commit(TX).unwrap();
    begin(&mut database, TransactionMode::ReadOnly);
    for source in [QuerySource::ObjectStore, QuerySource::Index("group".into())] {
        for direction in [
            CursorDirection::Next,
            CursorDirection::Prev,
            CursorDirection::NextUnique,
            CursorDirection::PrevUnique,
        ] {
            for limit in [0, 1, 1000, 1001, u32::MAX] {
                for keys_only in [false, true] {
                    let query = Query {
                        source: source.clone(),
                        direction,
                        limit,
                        keys_only,
                        ..Default::default()
                    };
                    let expected = database.query(TX, "a", &query).unwrap();
                    assert_eq!(paged(&database, query), expected);
                }
            }
        }
    }
    let range = crate::KeyRange {
        lower: Some(11.into()),
        upper: Some(2411.into()),
        lower_open: true,
        upper_open: true,
    };
    let query = Query {
        range,
        limit: u32::MAX,
        ..Default::default()
    };
    let records = paged(&database, query);
    assert_eq!(records.len(), 2399);
    assert_eq!(records.first().unwrap().primary_key, 12.into());
    assert_eq!(records.last().unwrap().primary_key, 2410.into());
}

#[test]
fn query_page_soft_targets_deliver_large_rows_and_key_pages_skip_corrupt_bodies() {
    let mut database = database();
    begin(&mut database, TransactionMode::ReadWrite);
    put(
        &mut database,
        Some(1.into()),
        &vec![1; 2 * 1024 * 1024],
        vec![],
        None,
    )
    .unwrap();
    put(&mut database, Some(2.into()), &[2], vec![], None).unwrap();
    database.commit(TX).unwrap();
    begin(&mut database, TransactionMode::ReadOnly);
    let query = Query {
        limit: u32::MAX,
        ..Default::default()
    };
    let first = database.query_page(TX, "a", &query).unwrap();
    assert_eq!(first.records.len(), 1);
    assert!(first.has_more);
    assert_eq!(
        first.records[0].value.as_ref().unwrap().wire_bytes().len(),
        2 * 1024 * 1024
    );
    assert_eq!(paged(&database, query.clone()).len(), 2);
    database.commit(TX).unwrap();
    database
        .connection
        .execute("UPDATE record_chunks SET bytes=x'00' WHERE chunk_no=0", [])
        .unwrap();
    begin(&mut database, TransactionMode::ReadOnly);
    let keys = Query {
        keys_only: true,
        ..query.clone()
    };
    let page = database.query_page(TX, "a", &keys).unwrap();
    assert_eq!(page.records.len(), 2);
    assert!(!page.has_more);
    assert!(page.records.iter().all(|record| record.value.is_none()));
    assert!(database.query_page(TX, "a", &query).is_err());
}
