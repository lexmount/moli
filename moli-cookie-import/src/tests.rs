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
    let path = destination.join("partitions/default/localstorage.json");
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "origins": {
                "https://keep.example": {"entries": [{"key": "keep", "value": "untouched"}]},
                "https://example.com": {"entries": [
                    {"key": "preserved", "value": "keep"},
                    {"key": {"utf16": [115, 104, 97, 114, 101, 100]}, "value": "old shared"},
                    {"key": "raw-value", "value": {"utf16": [55296]}},
                    {"key": {"utf16": [55296]}, "value": "old surrogate"}
                ]}
            }
        }))?,
    )?;
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
            cookies: 0,
            storage: 5
        }
    );
    let bytes = fs::read(&storage)?;
    let actual: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        actual,
        serde_json::json!({
            "version": 1,
            "origins": {
                "https://chrome.example": {"entries": [{"key": "token", "value": "chrome"}]},
                "https://keep.example": {"entries": [{"key": "keep", "value": "untouched"}]},
                "https://example.com": {"entries": [
                    {"key": "chrome-only", "value": "chrome"},
                    {"key": "firefox-only", "value": "firefox"},
                    {"key": "preserved", "value": "keep"},
                    {"key": "raw-value", "value": {"utf16": [55296]}},
                    {"key": "shared", "value": "firefox"},
                    {"key": {"utf16": [55296]}, "value": {"utf16": [56320]}}
                ]}
            }
        })
    );
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
        let storage = destination.join("partitions/default/localstorage.json");
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
        let cookies = storage.with_file_name("cookies.json");
        moli_cookie_cache::save_cookie_cache(&cookies, [])?;
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
fn invalid_destination_files_are_rejected_before_either_file_is_written() -> Result<()> {
    for (target, invalid) in [
        ("cookies.json", "{"),
        ("localstorage.json", "{"),
        ("localstorage.json", r#"{"version":2,"origins":{}}"#),
        (
            "localstorage.json",
            r#"{"version":1,"origins":{"https://example.com":{"entries":[{"key":{},"value":"x"}]}}}"#,
        ),
    ] {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("moli");
        let firefox = temp.path().join("firefox");
        let storage = existing_storage(&destination)?;
        let cookies = storage.with_file_name("cookies.json");
        moli_cookie_cache::save_cookie_cache(&cookies, [])?;
        fs::write(storage.with_file_name(target), invalid)?;
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
        assert!(
            import_session_state(&request).is_err(),
            "accepted {target}: {invalid}"
        );
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
    let cookies = moli_cookie_cache::load_cookie_cache(profile.default_partition().cookies_path())?;
    assert_eq!(summary.cookies, 1);
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "session");
    Ok(())
}
