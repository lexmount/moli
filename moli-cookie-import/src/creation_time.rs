use time::OffsetDateTime;

/// Browser creation fields use microseconds, with an epoch specific to each
/// browser. Missing, malformed and future dates must not grant a fresh
/// Lax-allow-unsafe window to an imported cookie.
pub(crate) fn from_browser_micros(
    micros: Option<i64>,
    epoch_offset_micros: i64,
) -> Option<OffsetDateTime> {
    let micros = micros.filter(|micros| *micros > 0)?;
    let unix_micros = micros.checked_sub(epoch_offset_micros)?;
    moli_cookie_jar::cookie_creation_time::from_unix_micros(unix_micros)
        .filter(|created| *created <= OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use moli_browser_profile::BrowserProfile;
    use moli_cookie_jar::{NetworkCookieRequestContext, StoredCookie};
    use rusqlite::{Connection, types::Value};
    use std::path::Path;
    use time::Duration;

    fn source_database(profile: &Path, chrome: bool) -> Result<Connection> {
        std::fs::create_dir_all(profile)?;
        let connection =
            Connection::open(profile.join(if chrome { "Cookies" } else { "cookies.sqlite" }))?;
        connection.execute_batch(if chrome {
            "CREATE TABLE cookies (
                host_key TEXT, name TEXT, value TEXT, encrypted_value BLOB, path TEXT,
                expires_utc INTEGER, is_secure INTEGER, is_httponly INTEGER, samesite INTEGER,
                priority INTEGER, source_scheme INTEGER, source_port INTEGER, creation_utc);
             INSERT INTO cookies VALUES ('cookie.test','sid','value',X'','/',0,1,0,-1,1,2,443,NULL);"
        } else {
            "CREATE TABLE moz_cookies (
                name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, isSecure INTEGER,
                isHttpOnly INTEGER, sameSite INTEGER, schemeMap INTEGER, originAttributes TEXT, creationTime);
             INSERT INTO moz_cookies VALUES ('sid','value','cookie.test','/',2147483647,1,0,-1,2,'',NULL);"
        })?;
        Ok(connection)
    }

    fn set_creation(connection: &Connection, chrome: bool, value: Value) -> Result<()> {
        connection.execute(
            if chrome {
                "UPDATE cookies SET creation_utc=?1"
            } else {
                "UPDATE moz_cookies SET creationTime=?1"
            },
            [value],
        )?;
        Ok(())
    }

    fn read_source(profile: &Path, chrome: bool) -> Result<Vec<StoredCookie>> {
        if chrome {
            crate::chrome::import_cookies(profile, &crate::ChromeCryptoKey::System)
        } else {
            crate::firefox::import_cookies(profile)
        }
    }

    #[test]
    fn browser_creation_times_survive_import_and_sqlite_reopen_without_resetting_age() -> Result<()>
    {
        for chrome in [true, false] {
            let root = tempfile::tempdir()?;
            let source = root.path().join("source");
            let connection = source_database(&source, chrome)?;
            let offset = if chrome {
                crate::chrome::CHROME_EPOCH_OFFSET_MICROS
            } else {
                0
            };
            let now = OffsetDateTime::from_unix_timestamp_nanos(
                OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000 * 1_000,
            )?;
            for (index, age) in [Duration::seconds(30), Duration::minutes(3)]
                .into_iter()
                .enumerate()
            {
                let created = now - age;
                let micros = i64::try_from(created.unix_timestamp_nanos() / 1_000)? + offset;
                set_creation(&connection, chrome, Value::Integer(micros))?;
                assert_eq!(
                    read_source(&source, chrome)?[0].creation_time,
                    Some(created)
                );
                let destination = root.path().join(format!("moli-{index}"));
                let summary = crate::import_session_state(&crate::ImportRequest {
                    profile_dir: &destination,
                    includes: "cookies".parse().unwrap(),
                    chrome_profile_dir: chrome.then_some(source.as_path()),
                    chrome_crypto_key: None,
                    firefox_profile_dir: (!chrome).then_some(source.as_path()),
                    cookie_jars: &[],
                })?;
                assert_eq!(summary.cookies, 1);
                let profile = BrowserProfile::open(&destination)?;
                let runtime = moli_sqlite::Runtime::new()?;
                let (store, owner) = moli_cookie_db::CookieDatabase::open(
                    profile.default_partition().cookies_path().to_owned(),
                    moli_sqlite::OpenMode::Existing,
                    runtime.handle(),
                )?;
                let mut jar = store.lock()?;
                assert_eq!(jar.cookies()[0].creation_time, Some(created));
                let url = "https://cookie.test/".parse().unwrap();
                let initiator = "https://other.test/".parse().unwrap();
                let report = jar.cookie_access_report_for_request(
                    &url,
                    NetworkCookieRequestContext::top_level_navigation("POST")
                        .with_initiator_url(&url, &initiator),
                );
                assert_eq!(report.included_cookies.len(), usize::from(index == 0));
                assert_eq!(report.excluded_cookies.len(), usize::from(index != 0));
                drop(jar);
                owner.close()?;
            }
        }
        Ok(())
    }

    #[test]
    fn malformed_browser_creation_times_stay_unknown_without_rejecting_the_cookie() -> Result<()> {
        for chrome in [true, false] {
            let root = tempfile::tempdir()?;
            let connection = source_database(root.path(), chrome)?;
            let offset = if chrome {
                crate::chrome::CHROME_EPOCH_OFFSET_MICROS
            } else {
                0
            };
            let future = i64::try_from(
                (OffsetDateTime::now_utc() + Duration::days(1)).unix_timestamp_nanos() / 1_000,
            )? + offset;
            for value in [
                Value::Null,
                Value::Integer(0),
                Value::Integer(-1),
                Value::Integer(i64::MAX),
                Value::Integer(future),
                Value::Text("invalid".into()),
                Value::Blob(vec![1, 2, 3]),
                Value::Real(123.5),
            ] {
                set_creation(&connection, chrome, value)?;
                let cookies = read_source(root.path(), chrome)?;
                assert_eq!(cookies.len(), 1);
                assert_eq!(cookies[0].value, "value");
                assert_eq!(cookies[0].creation_time, None);
                assert_eq!(
                    cookies[0].effective_same_site(),
                    moli_cookie_jar::StoredCookieEffectiveSameSite::Lax
                );
            }
        }
        Ok(())
    }
}
