//! Storage contracts retained when the old JSON/working-copy tests were retired.
use super::*;

struct Disk(std::path::PathBuf);
impl Disk {
    fn new() -> Self {
        let mut token = [0; 16];
        moli_crypto::fill_secure_random(&mut token).unwrap();
        let path = std::env::temp_dir().join(format!(
            "moli-idb-sql-contract-{}",
            moli_crypto::sha256_hex(token)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn create(&self) -> Database {
        let connection = moli_sqlite::open_disk(
            &self.0.join("db.sqlite"),
            moli_sqlite::OpenMode::Create,
            Default::default(),
        )
        .unwrap();
        Database::create(connection, identity()).unwrap()
    }
    fn reopen(&self) -> Database {
        let connection = moli_sqlite::open_disk(
            &self.0.join("db.sqlite"),
            moli_sqlite::OpenMode::Existing,
            Default::default(),
        )
        .unwrap();
        Database::open(connection, identity()).unwrap()
    }
}
impl Drop for Disk {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn create_store(database: &mut Database, auto_increment: bool) {
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
        .unwrap();
    database
        .create_object_store(
            TX,
            "a",
            ObjectStoreOptions {
                auto_increment,
                ..Default::default()
            },
        )
        .unwrap();
    database.commit(TX).unwrap();
}

#[test]
fn all_key_types_survive_sqlite_reopen_and_remain_distinct() {
    let disk = Disk::new();
    let mut keys = vec![
        Key::number(f64::NEG_INFINITY).unwrap(),
        Key::number(-0.0).unwrap(),
        Key::number(f64::from_bits(1)).unwrap(),
        Key::number(1.25).unwrap(),
        Key::number(f64::MAX).unwrap(),
        Key::number(f64::INFINITY).unwrap(),
        Key::Date(0),
        Key::String(vec![0xd800]),
        Key::String(vec![0xdc00]),
        Key::from("\u{fffd}"),
        Key::Binary(vec![0, 255]),
        Key::Array(vec![Key::Date(42), Key::Binary(vec![7])]),
    ];
    let mut deep = Key::from(1);
    for _ in 0..100 {
        deep = Key::Array(vec![deep]);
    }
    keys.push(deep);
    {
        let mut database = disk.create();
        create_store(&mut database, false);
        begin(&mut database, TransactionMode::ReadWrite);
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(
                put(
                    &mut database,
                    Some(key.clone()),
                    &[index as u8],
                    vec![],
                    None
                )
                .unwrap(),
                *key
            );
        }
        database.commit(TX).unwrap();
        database.close().unwrap();
    }
    let mut database = disk.reopen();
    begin(&mut database, TransactionMode::ReadOnly);
    let rows = database
        .query(
            TX,
            "a",
            &Query {
                limit: 32,
                keys_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    let actual = rows
        .into_iter()
        .map(|row| {
            assert!(row.value.is_none());
            row.key
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, keys);
    let Key::Number(zero) = &actual[1] else {
        panic!("zero key lost its type")
    };
    assert_eq!(zero.value().to_bits(), (-0.0_f64).to_bits());
    for (index, key) in keys.iter().enumerate() {
        assert_eq!(
            database.get(TX, "a", key).unwrap().unwrap().wire_bytes(),
            &[index as u8]
        );
    }
    assert_eq!(
        database
            .get(TX, "a", &Key::from(0))
            .unwrap()
            .unwrap()
            .wire_bytes(),
        &[1]
    );
    database.commit(TX).unwrap();
}

#[test]
fn overdeep_keys_do_not_poison_sqlite_requests_generator_or_reopen() {
    for persistent in [false, true] {
        let disk = Disk::new();
        let mut database = if persistent { disk.create() } else { empty() };
        create_store(&mut database, true);
        begin(&mut database, TransactionMode::ReadWrite);
        let mut invalid = Key::from(1);
        for _ in 0..=crate::MAX_KEY_DEPTH {
            invalid = Key::Array(vec![invalid]);
        }
        for add_only in [false, true] {
            for quota in [None, Some(u64::MAX)] {
                let outcome = database.write(
                    TX,
                    "a",
                    RecordWrite {
                        key: Some(invalid.clone()),
                        value: vec![1].into(),
                        indexes: vec![],
                        add_only,
                        quota,
                    },
                );
                assert!(
                    matches!(outcome, Err(IndexedDbError::Data(_))),
                    "{outcome:?}"
                );
                assert_eq!(database.count(TX, "a").unwrap(), 0);
            }
        }
        assert_eq!(
            put(&mut database, None, &[2], vec![], None).unwrap(),
            Key::from(1)
        );
        database.commit(TX).unwrap();
        begin(&mut database, TransactionMode::ReadWrite);
        assert_eq!(
            put(&mut database, None, &[3], vec![], None).unwrap(),
            Key::from(2)
        );
        database.commit(TX).unwrap();
        if persistent {
            database.close().unwrap();
            database = disk.reopen();
        }
        begin(&mut database, TransactionMode::ReadOnly);
        let rows = database
            .query(
                TX,
                "a",
                &Query {
                    limit: 3,
                    keys_only: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            rows.into_iter().map(|row| row.key).collect::<Vec<_>>(),
            [Key::from(1), Key::from(2)]
        );
        assert_eq!(
            database
                .get(TX, "a", &Key::from(1))
                .unwrap()
                .unwrap()
                .wire_bytes(),
            &[2]
        );
        assert_eq!(
            database
                .get(TX, "a", &Key::from(2))
                .unwrap()
                .unwrap()
                .wire_bytes(),
            &[3]
        );
        database.commit(TX).unwrap();
    }
}
