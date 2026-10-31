use std::fs;

use anyhow::Result;
use rusqlite::{Connection, params};

use super::*;

fn storage_request(profile_dir: &Path) -> ImportRequest<'_> {
    ImportRequest {
        profile_dir,
        includes: "storage".parse().unwrap(),
        chrome_profile_dir: None,
        chrome_crypto_key: None,
        firefox_profile_dir: None,
        cookie_jars: &[],
    }
}

fn chrome_storage(profile: &Path, entries: &[(&[u8], &[u8])]) -> Result<()> {
    let path = profile.join("Local Storage/leveldb");
    fs::create_dir_all(&path)?;
    let mut database = rusty_leveldb::DB::open(path, rusty_leveldb::Options::default())?;
    database.put(b"VERSION", b"1")?;
    for (key, value) in entries {
        database.put(key, value)?;
    }
    database.flush()?;
    Ok(())
}

fn firefox_storage(profile: &Path, entries: &[(&str, &str)]) -> Result<PathBuf> {
    let directory = profile.join("storage/default/https+++example.com/ls");
    fs::create_dir_all(&directory)?;
    let path = directory.join("data.sqlite");
    let connection = Connection::open(&path)?;
    connection.execute_batch(
        "CREATE TABLE database (origin TEXT);
         INSERT INTO database VALUES ('https://example.com');
         CREATE TABLE data (key TEXT, conversion_type INTEGER, compression_type INTEGER, value BLOB);",
    )?;
    for (key, value) in entries {
        connection.execute(
            "INSERT INTO data VALUES (?1, 1, 0, ?2)",
            params![key, value.as_bytes()],
        )?;
    }
    Ok(path)
}

fn existing_storage(destination: &Path) -> Result<PathBuf> {
    with_local_storage(destination, |store| {
        let keep = area_key("https://keep.example");
        store.try_set_item(&keep, "keep", "untouched")?;
        let key = area_key("https://example.com");
        store.try_set_item(&key, "preserved", "keep")?;
        store.try_set_item(&key, "shared", "old shared")?;
        store.try_set_item_utf16(
            &key,
            &"raw-value".encode_utf16().collect::<Vec<_>>(),
            &[55296],
        )?;
        store.try_set_item_utf16(
            &key,
            &[55296],
            &"old surrogate".encode_utf16().collect::<Vec<_>>(),
        )?;
        Ok(())
    })?;
    Ok(moli_browser_profile::BrowserProfilePaths::new(destination).local_storage_path)
}

fn area_key(origin: &str) -> String {
    moli_web_storage::web_storage_partitioned_area_key(origin, origin)
}

fn with_local_storage<T>(
    destination: &Path,
    inspect: impl FnOnce(&moli_web_storage::WebStorageStore) -> Result<T>,
) -> Result<T> {
    let profile = BrowserProfile::open(destination)?;
    let mut runtime = moli_sqlite::Runtime::new()?;
    let (store, owner) = moli_web_storage::new_sqlite_web_storage_store(
        profile.default_partition().local_storage_path().to_owned(),
        moli_sqlite::OpenMode::Create,
        runtime.handle(),
    )?;
    let result = inspect(&store);
    owner.close()?;
    runtime.shutdown()?;
    result
}

pub(super) fn read_local_items(
    destination: &Path,
    origin: &str,
) -> Result<Vec<(Vec<u16>, Vec<u16>)>> {
    with_local_storage(destination, |store| {
        Ok(store.try_items_utf16(&area_key(origin))?)
    })
}

fn seed_cookies(destination: &Path) -> Result<PathBuf> {
    let profile = BrowserProfile::open(destination)?;
    let mut runtime = moli_sqlite::Runtime::new()?;
    let path = profile.default_partition().cookies_path().to_owned();
    let (_store, owner) = moli_cookie_db::CookieDatabase::open(
        path.clone(),
        moli_sqlite::OpenMode::Create,
        runtime.handle(),
    )?;
    owner.close()?;
    runtime.shutdown()?;
    Ok(path)
}

#[test]
fn merges_both_browsers_with_existing_storage_by_utf16_key() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let destination = temp.path().join("moli");
    let chrome = temp.path().join("chrome");
    let firefox = temp.path().join("firefox");
    let storage = existing_storage(&destination)?;
    chrome_storage(
        &chrome,
        &[
            (b"_https://example.com\0\x01chrome-only", b"\x01chrome"),
            (b"_https://example.com\0\x01shared", b"\x01chrome"),
            (b"_https://example.com\0\x00\x00\xd8", b"\x00\x00\xdc"),
            (b"_https://chrome.example\0\x01token", b"\x01chrome"),
        ],
    )?;
    firefox_storage(
        &firefox,
        &[("shared", "firefox"), ("firefox-only", "firefox")],
    )?;
    let mut request = storage_request(&destination);
    request.chrome_profile_dir = Some(&chrome);
    request.firefox_profile_dir = Some(&firefox);
    let summary = import_session_state(&request)?;
    assert_eq!(
        summary,
        ImportSummary {
            storage: 5,
            ..ImportSummary::default()
        }
    );
    let mut expected = [
        ("chrome-only", "chrome"),
        ("firefox-only", "firefox"),
        ("preserved", "keep"),
        ("shared", "firefox"),
    ]
    .into_iter()
    .map(|(k, v)| {
        (
            k.encode_utf16().collect::<Vec<_>>(),
            v.encode_utf16().collect::<Vec<_>>(),
        )
    })
    .collect::<Vec<_>>();
    expected.push(("raw-value".encode_utf16().collect(), vec![55296]));
    expected.push((vec![55296], vec![56320]));
    expected.sort();
    assert_eq!(
        read_local_items(&destination, "https://example.com")?,
        expected
    );
    assert_eq!(
        read_local_items(&destination, "https://keep.example")?,
        vec![(
            "keep".encode_utf16().collect::<Vec<_>>(),
            "untouched".encode_utf16().collect::<Vec<_>>()
        )],
    );
    assert_eq!(
        read_local_items(&destination, "https://chrome.example")?,
        vec![(
            "token".encode_utf16().collect::<Vec<_>>(),
            "chrome".encode_utf16().collect::<Vec<_>>()
        )],
    );
    let bytes = fs::read(&storage)?;
    assert!(bytes.starts_with(b"SQLite format 3\0"));
    assert_eq!(import_session_state(&request)?, summary);
    assert_eq!(fs::read(storage)?, bytes);
    Ok(())
}

#[test]
fn empty_browser_storage_does_not_create_or_replace_a_file() -> Result<()> {
    for existing in [false, true] {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("moli");
        let chrome = temp.path().join("chrome");
        let firefox = temp.path().join("firefox");
        chrome_storage(&chrome, &[])?;
        firefox_storage(&firefox, &[])?;
        let storage =
            moli_browser_profile::BrowserProfilePaths::new(&destination).local_storage_path;
        let before = if existing {
            Some(fs::read(existing_storage(&destination)?)?)
        } else {
            None
        };
        let mut request = storage_request(&destination);
        request.chrome_profile_dir = Some(&chrome);
        request.firefox_profile_dir = Some(&firefox);
        assert_eq!(import_session_state(&request)?, ImportSummary::default());
        assert_eq!(fs::read(storage).ok(), before);
    }
    Ok(())
}

#[test]
fn rejects_missing_source_directories_before_creating_a_profile() -> Result<()> {
    for browser in ["Chrome", "Firefox"] {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("moli");
        let missing = temp.path().join("missing");
        let mut request = storage_request(&destination);
        if browser == "Chrome" {
            request.chrome_profile_dir = Some(&missing);
        } else {
            request.firefox_profile_dir = Some(&missing);
        }
        let error = import_session_state(&request).unwrap_err();
        assert!(error.to_string().contains(browser));
        assert!(!destination.exists());
    }
    Ok(())
}

#[test]
fn later_source_failures_leave_existing_storage_and_cookies_unchanged() -> Result<()> {
    for corrupt_firefox in [true, false] {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("moli");
        let chrome = temp.path().join("chrome");
        let firefox = temp.path().join("firefox");
        let storage = existing_storage(&destination)?;
        let cookies = seed_cookies(&destination)?;
        let before_storage = fs::read(&storage)?;
        let before_cookies = fs::read(&cookies)?;
        let database = firefox_storage(&firefox, &[("new", "firefox")])?;
        let jars = [temp.path().join("missing-cookie-jar")];
        let mut request = storage_request(&destination);
        request.firefox_profile_dir = Some(&firefox);
        if corrupt_firefox {
            chrome_storage(
                &chrome,
                &[(b"_https://example.com\0\x01new", b"\x01chrome")],
            )?;
            request.chrome_profile_dir = Some(&chrome);
            fs::write(database, b"corrupt SQLite file")?;
        } else {
            request.includes = "all".parse().unwrap();
            request.cookie_jars = &jars;
        }
        let error = format!("{:#}", import_session_state(&request).unwrap_err());
        assert!(
            error.contains(if corrupt_firefox {
                "data.sqlite"
            } else {
                "missing-cookie-jar"
            }),
            "{error}"
        );
        assert_eq!(fs::read(storage)?, before_storage);
        assert_eq!(fs::read(cookies)?, before_cookies);
    }
    Ok(())
}

#[test]
fn invalid_destination_databases_are_rejected_before_either_store_is_changed() -> Result<()> {
    for target in ["cookies", "local", "local-schema"] {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("moli");
        let firefox = temp.path().join("firefox");
        let storage = existing_storage(&destination)?;
        let cookies = seed_cookies(&destination)?;
        if target == "local-schema" {
            Connection::open(&storage)?.pragma_update(None, "user_version", 999)?;
        } else {
            fs::write(
                if target == "cookies" {
                    &cookies
                } else {
                    &storage
                },
                b"corrupt database",
            )?;
        }
        let before_storage = fs::read(&storage)?;
        let before_cookies = fs::read(&cookies)?;
        firefox_storage(&firefox, &[("new", "firefox")])?;
        let jars = [temp.path().join("cookies.txt")];
        fs::write(
            &jars[0],
            "example.com\tFALSE\t/\tFALSE\t0\tsession\tnew-value\n",
        )?;
        let mut request = storage_request(&destination);
        request.includes = "all".parse().unwrap();
        request.firefox_profile_dir = Some(&firefox);
        request.cookie_jars = &jars;
        assert!(import_session_state(&request).is_err(), "accepted {target}");
        assert_eq!(fs::read(storage)?, before_storage);
        assert_eq!(fs::read(cookies)?, before_cookies);
    }
    Ok(())
}

#[test]
fn bundled_sqlite_omits_unused_extensions() -> Result<()> {
    // Inspect the linked engine so dependency upgrades cannot silently
    // reintroduce extensions excluded by .cargo/config.toml.
    let connection = rusqlite::Connection::open_in_memory()?;
    for option in [
        "ENABLE_FTS3",
        "ENABLE_FTS3_PARENTHESIS",
        "ENABLE_FTS4",
        "ENABLE_FTS5",
        "ENABLE_RTREE",
        "ENABLE_DBSTAT_VTAB",
        "ENABLE_STAT4",
        "ENABLE_LOAD_EXTENSION",
    ] {
        let enabled: bool =
            connection.query_row("SELECT sqlite_compileoption_used(?1)", [option], |row| {
                row.get(0)
            })?;
        assert!(!enabled, "bundled SQLite unexpectedly enables {option}");
    }
    let omits_extension_loading: bool = connection.query_row(
        "SELECT sqlite_compileoption_used('OMIT_LOAD_EXTENSION')",
        [],
        |row| row.get(0),
    )?;
    assert!(omits_extension_loading);
    Ok(())
}

#[test]
fn imports_a_netscape_cookie_jar() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let jar = temp.path().join("cookies.txt");
    let destination = temp.path().join("moli-profile");
    let paths = moli_browser_profile::BrowserProfilePaths::new(&destination);
    fs::create_dir_all(&paths.partition_root)?;
    for name in ["cookies.json", "localstorage.json"] {
        fs::write(
            paths.partition_root.join(name),
            b"old JSON must stay untouched",
        )?;
    }
    fs::write(
        &jar,
        "# Netscape HTTP Cookie File\nexample.com\tFALSE\t/\tFALSE\t0\tsession\tvalue\n",
    )?;

    let summary = import_session_state(&ImportRequest {
        profile_dir: &destination,
        includes: "all".parse().expect("valid import selection"),
        chrome_profile_dir: None,
        chrome_crypto_key: None,
        firefox_profile_dir: None,
        cookie_jars: &[jar],
    })?;

    let profile = BrowserProfile::open(&destination)?;
    let mut runtime = moli_sqlite::Runtime::new()?;
    let (cookies, owner) = moli_cookie_db::CookieDatabase::open(
        profile.default_partition().cookies_path().to_owned(),
        moli_sqlite::OpenMode::Existing,
        runtime.handle(),
    )?;
    let cookies = cookies.lock().unwrap().cookies();
    owner.close()?;
    runtime.shutdown()?;
    assert_eq!(summary.cookies, 1);
    assert_eq!(summary.cookies_rejected, 0);
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "session");
    for name in ["cookies.json", "localstorage.json"] {
        assert_eq!(
            fs::read(paths.partition_root.join(name))?,
            b"old JSON must stay untouched"
        );
    }
    Ok(())
}

fn imported_cookies(destination: &Path) -> Result<Vec<moli_cookie_jar::StoredCookie>> {
    let profile = BrowserProfile::open(destination)?;
    let mut runtime = moli_sqlite::Runtime::new()?;
    let (store, owner) = moli_cookie_db::CookieDatabase::open(
        profile.default_partition().cookies_path().to_owned(),
        moli_sqlite::OpenMode::Existing,
        runtime.handle(),
    )?;
    let cookies = store.lock()?.cookies();
    owner.close()?;
    runtime.shutdown()?;
    Ok(cookies)
}

#[test]
fn cookie_import_counts_accepted_and_rejected_writes_including_replacements() -> Result<()> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("profile");
    let jars = [root.path().join("cookies.txt")];
    let request = ImportRequest {
        profile_dir: &destination,
        includes: "cookies".parse().unwrap(),
        chrome_profile_dir: None,
        chrome_crypto_key: None,
        firefox_profile_dir: None,
        cookie_jars: &jars,
    };
    fs::write(&jars[0], "old.test\tFALSE\t/\tFALSE\t0\told\tkeep\n")?;
    assert_eq!(import_session_state(&request)?.cookies, 1);
    fs::write(
        &jars[0],
        format!(
            "example.com\tFALSE\t/\tFALSE\t0\tvalid\tfirst\n\
         example.com\tFALSE\t/\tFALSE\t0\tvalid\tlast\n\
         example.com\tFALSE\t/\tFALSE\t0\t__Secure-a\trejected\n\
         example.com\tFALSE\t/\tFALSE\t0\t__Secure-b\trejected\n\
         example.com\tFALSE\t/\tFALSE\t0\toversized\t{}\n",
            "x".repeat(4097)
        ),
    )?;
    let summary = import_session_state(&request)?;
    assert_eq!(summary.cookies, 2);
    assert_eq!(summary.cookies_rejected, 3);
    assert_eq!(summary.to_string(), "cookies accepted=2, rejected=3");
    let cookies = imported_cookies(&destination)?;
    assert_eq!(cookies.len(), 2);
    assert!(cookies.iter().any(|c| c.name == "old" && c.value == "keep"));
    assert!(
        cookies
            .iter()
            .any(|c| c.name == "valid" && c.value == "last")
    );
    Ok(())
}

#[test]
fn rejected_cookie_imports_do_not_report_successful_writes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("profile");
    let jars = [root.path().join("cookies.txt")];
    fs::write(
        &jars[0],
        "example.com\tFALSE\t/\tFALSE\t0\t__Secure-invalid\tvalue\n",
    )?;
    let summary = import_session_state(&ImportRequest {
        profile_dir: &destination,
        includes: "cookies".parse().unwrap(),
        chrome_profile_dir: None,
        chrome_crypto_key: None,
        firefox_profile_dir: None,
        cookie_jars: &jars,
    })?;
    assert_eq!(summary.cookies, 0);
    assert_eq!(summary.cookies_rejected, 1);
    assert_eq!(summary.to_string(), "cookies accepted=0, rejected=1");
    assert!(imported_cookies(&destination)?.is_empty());
    Ok(())
}

#[test]
fn quota_eviction_is_distinct_from_accepting_cookie_imports() -> Result<()> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("profile");
    let jars = [root.path().join("cookies.txt")];
    fs::write(
        &jars[0],
        (0..181)
            .map(|i| format!("example.com\tFALSE\t/\tFALSE\t0\tk{i}\tvalue\n"))
            .collect::<String>(),
    )?;
    let summary = import_session_state(&ImportRequest {
        profile_dir: &destination,
        includes: "cookies".parse().unwrap(),
        chrome_profile_dir: None,
        chrome_crypto_key: None,
        firefox_profile_dir: None,
        cookie_jars: &jars,
    })?;
    assert_eq!(summary.cookies, 181);
    assert_eq!(summary.cookies_rejected, 0);
    let cookies = imported_cookies(&destination)?;
    assert_eq!(cookies.len(), 180);
    assert!(!cookies.iter().any(|c| c.name == "k0"));
    assert_eq!(summary.to_string(), "cookies accepted=181, rejected=0");
    Ok(())
}
