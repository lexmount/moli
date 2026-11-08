use super::*;
use moli_cookie_jar::CookieSource;
use moli_sqlite::Runtime;
use time::{Duration as TimeDuration, OffsetDateTime};
use url::Url;

fn cookie(name: &str) -> StoredCookie {
    StoredCookie {
        name: name.into(),
        value: "value".into(),
        domain: "cookie.test".into(),
        host_only: true,
        path: "/".into(),
        secure: true,
        http_only: false,
        expires: None,
        same_site: StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        source_scheme: StoredCookieSourceScheme::Secure,
        source_port: 443,
        creation_index: 0,
        creation_time: None,
        last_access_index: 0,
    }
}
fn open(root: &std::path::Path, runtime: &Runtime) -> (SharedBrowserCookieStore, CookieDatabase) {
    CookieDatabase::open(
        root.join("cookies.sqlite"),
        OpenMode::Create,
        runtime.handle(),
    )
    .unwrap()
}
fn put(store: &SharedBrowserCookieStore, cookie: StoredCookie) {
    assert!(
        store
            .lock()
            .unwrap()
            .upsert_with_request_url_report(cookie, None, CookieSource::Cdp)
            .is_accepted()
    );
}

#[test]
fn unused_cookie_handles_flush_and_close_without_creating_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cookies.sqlite");
    let runtime = Runtime::new().unwrap();
    let (store, owner) =
        CookieDatabase::deferred(path.clone(), OpenMode::Create, runtime.handle()).unwrap();
    let clone = store.clone();
    assert!(clone.lock_if_initialized().is_none());
    owner.flush().unwrap();
    owner.close().unwrap();
    assert!(root.path().read_dir().unwrap().next().is_none());
    assert!(
        clone.lock().is_err(),
        "closed handles must not reopen or restore the jar"
    );
    assert!(root.path().read_dir().unwrap().next().is_none());
}

#[test]
fn concurrent_first_cookie_access_restores_once_before_accepting_mutations() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cookies.sqlite");
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    put(&store, cookie("restored"));
    owner.close().unwrap();
    let (store, owner) =
        CookieDatabase::deferred(path, OpenMode::Create, runtime.handle()).unwrap();
    assert!(store.lock_if_initialized().is_none());
    std::thread::scope(|scope| {
        for index in 0..8 {
            let store = store.clone();
            scope.spawn(move || {
                let mut jar = store.lock().unwrap();
                assert!(jar.cookies().iter().any(|row| row.name == "restored"));
                assert!(
                    jar.upsert_with_request_url_report(
                        cookie(&format!("writer{index}")),
                        None,
                        CookieSource::Cdp
                    )
                    .is_accepted()
                );
            });
        }
    });
    assert_eq!(store.lock().unwrap().cookies().len(), 9);
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.lock().unwrap().cookies().len(), 9);
    owner.close().unwrap();
}

#[test]
fn cookie_open_failure_is_deferred_and_never_becomes_an_empty_jar() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("not-a-directory");
    std::fs::write(&parent, b"keep").unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = CookieDatabase::deferred(
        parent.join("cookies.sqlite"),
        OpenMode::Create,
        runtime.handle(),
    )
    .unwrap();
    owner.flush().unwrap();
    let error = store.lock().unwrap_err().to_string();
    assert_eq!(store.lock().unwrap_err().to_string(), error);
    assert!(owner.flush().is_err());
    assert!(owner.close().is_err());
    assert_eq!(std::fs::read(parent).unwrap(), b"keep");
}

#[test]
fn complete_metadata_and_unsigned_ordering_indices_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let mut row = cookie("metadata");
    row.creation_time = Some(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
            + TimeDuration::nanoseconds(123456789),
    );
    row.http_only = true;
    row.host_only = false;
    row.same_site = StoredCookieSameSite::None;
    row.priority = Some(CookiePriority::High);
    row.expires = Some(
        OffsetDateTime::from_unix_timestamp(4_000_000_000).unwrap()
            + TimeDuration::nanoseconds(123456789),
    );
    row.creation_index = u64::MAX - 2;
    row.last_access_index = u64::MAX - 1;
    let mut expected = row.clone();
    expected.creation_time = Some(
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
            + TimeDuration::microseconds(123_456),
    );
    owner
        .owner
        .client()
        .query(move |connection| {
            codec::upsert(connection, &Key::from(&row), &row)?;
            Ok(())
        })
        .unwrap();
    owner.close().unwrap();
    drop(store);
    drop(owner);
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.lock().unwrap().cookies(), vec![expected.clone()]);
    assert_eq!(
        owner.owner.client().query(codec::read).unwrap(),
        vec![expected]
    );
    put(&store, cookie("next"));
    let next = store
        .lock()
        .unwrap()
        .cookies()
        .into_iter()
        .find(|c| c.name == "next")
        .unwrap();
    assert_eq!(next.creation_index, u64::MAX - 1);
    owner.close().unwrap();
}
#[test]
fn session_cookies_restore_but_opaque_partition_cookies_do_not() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    put(&store, cookie("session"));
    let mut opaque = cookie("opaque");
    opaque.partition_key = Some(StoredCookiePartitionKey::Opaque {
        nonce: 42,
        has_cross_site_ancestor: true,
    });
    put(&store, opaque);
    owner.close().unwrap();
    drop(store);
    drop(owner);
    let (store, owner) = open(root.path(), &runtime);
    let rows = store.lock().unwrap().cookies();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "session");
    assert_eq!(rows[0].expires, None);
    owner.flush().unwrap();
    assert_eq!(owner.owner.client().query(codec::read).unwrap().len(), 1);
    owner.close().unwrap();
}
#[test]
fn complete_partition_keys_and_paths_have_distinct_rows() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    put(&store, cookie("same"));
    for ancestor in [false, true] {
        let mut row = cookie("same");
        row.partition_key = Some(StoredCookiePartitionKey::Site {
            top_level_site: "https://top.test".into(),
            has_cross_site_ancestor: ancestor,
        });
        put(&store, row);
    }
    let mut row = cookie("same");
    row.path = "/other".into();
    put(&store, row);
    owner.close().unwrap();
    drop(owner);
    let (store, owner) = open(root.path(), &runtime);
    assert_eq!(store.lock().unwrap().cookies().len(), 4);
    owner.close().unwrap();
}
#[test]
fn reads_update_memory_ordering_without_persisting_touches_and_clones_have_no_writer() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    put(&store, cookie("live"));
    owner.flush().unwrap();
    let before = owner.owner.client().query(codec::read).unwrap()[0].last_access_index;
    let mut snapshot = store.lock().unwrap().clone();
    snapshot.upsert(cookie("snapshot-only"), CookieSource::Cdp);
    store
        .lock()
        .unwrap()
        .cookie_header(&Url::parse("https://cookie.test/").unwrap());
    owner.flush().unwrap();
    let rows = owner.owner.client().query(codec::read).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "live");
    assert!(store.lock().unwrap().cookies()[0].last_access_index > before);
    assert_eq!(rows[0].last_access_index, before);
    owner.close().unwrap();
}
#[test]
fn document_expiry_and_quota_eviction_remove_persisted_rows() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let url = Url::parse("https://cookie.test/").unwrap();
    store
        .lock()
        .unwrap()
        .set_document_cookie(&url, "expire=live; Secure; Path=/");
    owner.flush().unwrap();
    store
        .lock()
        .unwrap()
        .set_document_cookie(&url, "expire=gone; Secure; Path=/; Max-Age=0");
    owner.flush().unwrap();
    assert!(owner.owner.client().query(codec::read).unwrap().is_empty());
    for i in 0..181 {
        let mut row = cookie(&format!("k{i}"));
        row.secure = false;
        row.source_scheme = StoredCookieSourceScheme::NonSecure;
        row.source_port = 80;
        put(&store, row);
    }
    owner.flush().unwrap();
    let rows = owner.owner.client().query(codec::read).unwrap();
    assert_eq!(rows.len(), 180);
    assert!(rows.iter().all(|c| c.name != "k0"));
    owner.close().unwrap();
}
#[test]
fn persistence_failure_discards_the_batch_but_keeps_the_jar_and_accepts_new_changes() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    owner.owner.client().query(|connection| {connection.execute_batch("CREATE TRIGGER fail_cookie BEFORE INSERT ON cookies BEGIN SELECT RAISE(FAIL,'injected Cookie write failure'); END;")?;Ok(())}).unwrap();
    put(&store, cookie("changed"));
    put(&store, cookie("lost"));
    assert!(owner.flush().is_err());
    assert_eq!(store.lock().unwrap().cookies().len(), 2);
    assert!(
        owner.flush().is_err(),
        "flush does not replay a discarded batch"
    );
    assert!(owner.owner.client().query(codec::read).unwrap().is_empty());
    let mut changed = cookie("changed");
    changed.value = "newer".into();
    put(&store, changed);
    put(&store, cookie("another"));
    owner
        .owner
        .client()
        .query(|connection| {
            connection.execute_batch("DROP TRIGGER fail_cookie")?;
            Ok(())
        })
        .unwrap();
    owner.flush().unwrap();
    let rows = owner.owner.client().query(codec::read).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|cookie| cookie.name != "lost"));
    assert_eq!(store.lock().unwrap().cookies().len(), 3);
    assert_eq!(
        rows.iter().find(|c| c.name == "changed").unwrap().value,
        "newer"
    );
    owner.close().unwrap();
}
#[test]
fn source_metadata_and_host_only_changes_replace_the_same_canonical_identity() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    put(&store, cookie("same"));
    owner.flush().unwrap();
    let first = store.lock().unwrap().cookies()[0].creation_index;
    let mut next = cookie("same");
    next.value = "replaced".into();
    next.host_only = false;
    next.secure = false;
    next.source_scheme = StoredCookieSourceScheme::NonSecure;
    next.source_port = 1234;
    let request = Url::parse("https://cookie.test/").unwrap();
    assert!(
        store
            .lock()
            .unwrap()
            .upsert_with_request_url_report(next, Some(&request), CookieSource::Cdp)
            .is_accepted()
    );
    owner.flush().unwrap();
    let rows = owner.owner.client().query(codec::read).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "replaced");
    assert!(!rows[0].host_only);
    assert_eq!(rows[0].source_port, 1234);
    assert_eq!(rows[0].creation_index, first);
    owner.close().unwrap();
}
#[test]
fn unknown_database_is_not_recreated_or_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cookies.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE user_data(value);INSERT INTO user_data VALUES (42);PRAGMA user_version=99;").unwrap();
    drop(connection);
    let runtime = Runtime::new().unwrap();
    assert!(CookieDatabase::open(path.clone(), OpenMode::Existing, runtime.handle()).is_err());
    let connection = Connection::open(path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM user_data", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
}
#[test]
fn retrying_a_published_put_with_a_later_delete_does_not_resurrect_the_cookie() {
    let mut connection = Connection::open_in_memory().unwrap();
    codec::initialize(&mut connection).unwrap();
    let row = cookie("transient");
    let mut batch = Batch::change(StoredCookieMutation::Upsert {
        cookie: row.clone(),
    });
    batch.write(&mut connection).unwrap();
    batch.merge(Batch::change(StoredCookieMutation::Delete(
        StoredCookieIdentity::from(&row),
    )));
    batch.write(&mut connection).unwrap();
    batch.write(&mut connection).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM cookies", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn sqlite_cookie_creation_time_preserves_navigation_policy_across_reopen() {
    use moli_cookie_jar::StoredCookieEffectiveSameSite;

    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(123_456_000)
        .unwrap();
    let (store, owner) = open(root.path(), &runtime);
    for (name, created) in [
        ("recent", Some(now - TimeDuration::seconds(30))),
        ("old", Some(now - TimeDuration::minutes(3))),
        ("unknown", None),
    ] {
        let mut row = cookie(name);
        row.creation_time = created;
        put(&store, row);
    }
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    for row in store.lock().unwrap().cookies() {
        let (created, mode) = match row.name.as_str() {
            "recent" => (
                Some(now - TimeDuration::seconds(30)),
                StoredCookieEffectiveSameSite::LaxAllowUnsafe,
            ),
            "old" => (
                Some(now - TimeDuration::minutes(3)),
                StoredCookieEffectiveSameSite::Lax,
            ),
            "unknown" => (None, StoredCookieEffectiveSameSite::Lax),
            _ => unreachable!(),
        };
        assert_eq!(row.creation_time, created);
        assert_eq!(row.effective_same_site(), mode);
    }
    owner.close().unwrap();
}

#[test]
fn version_one_cookie_database_migrates_without_granting_a_new_cookie_age() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cookies.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE cookies(
        partition_key BLOB NOT NULL,domain TEXT NOT NULL,path TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,
        host_only INTEGER NOT NULL,secure INTEGER NOT NULL,http_only INTEGER NOT NULL,
        expires_seconds INTEGER,expires_nanos INTEGER,same_site INTEGER NOT NULL,priority INTEGER,
        source_scheme INTEGER NOT NULL,source_port INTEGER NOT NULL,creation_index BLOB NOT NULL,last_access_index BLOB NOT NULL,
        PRIMARY KEY(partition_key,domain,path,name)) WITHOUT ROWID;
        INSERT INTO cookies VALUES(X'00','cookie.test','/','legacy','keep',1,1,0,NULL,NULL,0,NULL,2,443,X'0000000000000011',X'0000000000000022');
        PRAGMA user_version=1;").unwrap();
    connection
        .pragma_update(None, "application_id", codec::APPLICATION_ID)
        .unwrap();
    drop(connection);
    let runtime = Runtime::new().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let legacy = store.lock().unwrap().cookies().pop().unwrap();
    assert_eq!(legacy.value, "keep");
    assert_eq!(legacy.creation_index, 17);
    assert_eq!(legacy.last_access_index, 34);
    assert_eq!(legacy.creation_time, None);
    assert_eq!(
        legacy.effective_same_site(),
        moli_cookie_jar::StoredCookieEffectiveSameSite::Lax
    );
    assert_eq!(
        owner
            .owner
            .client()
            .query(|connection| {
                Ok(connection
                    .pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0))?)
            })
            .unwrap(),
        2
    );
    let mut replacement = cookie("legacy");
    replacement.creation_time = Some(OffsetDateTime::now_utc());
    put(&store, replacement);
    let mut fresh = cookie("fresh");
    fresh.creation_time = Some(
        OffsetDateTime::now_utc()
            .replace_nanosecond(123_456_000)
            .unwrap(),
    );
    put(&store, fresh.clone());
    owner.close().unwrap();
    let (store, owner) = open(root.path(), &runtime);
    let rows = store.lock().unwrap().cookies();
    assert_eq!(
        rows.iter()
            .find(|row| row.name == "legacy")
            .unwrap()
            .creation_time,
        None
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.name == "fresh")
            .unwrap()
            .creation_time,
        fresh.creation_time
    );
    owner.close().unwrap();
}
