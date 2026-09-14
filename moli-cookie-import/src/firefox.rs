use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use moli_cookie_jar::{
    CookiePriority, StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme,
};
use time::OffsetDateTime;

use crate::{sqlite::SqliteSnapshot, storage::LocalStorage};

pub(crate) fn import_cookies(profile: &Path) -> Result<Vec<StoredCookie>> {
    let source = profile.join("cookies.sqlite");
    if !source.is_file() {
        return Ok(Vec::new());
    }
    let snapshot = SqliteSnapshot::open(&source, "Firefox cookies")?;
    let mut statement = snapshot.connection
        .prepare("SELECT name,value,host,path,expiry,isSecure,isHttpOnly,sameSite,schemeMap,originAttributes FROM moz_cookies")
        .context("Firefox cookies database has an unsupported schema")?;
    let rows = statement.query_map([], |row| {
        Ok(FirefoxCookieRow {
            name: row.get(0)?,
            value: row.get(1)?,
            host: row.get(2)?,
            path: row.get(3)?,
            expiry: row.get(4)?,
            secure: row.get::<_, i64>(5)? != 0,
            http_only: row.get::<_, i64>(6)? != 0,
            same_site: row.get(7)?,
            scheme_map: row.get(8)?,
            origin_attributes: row.get(9)?,
        })
    })?;
    let mut cookies = Vec::new();
    for row in rows {
        let row = row?;
        // Do not flatten container/private/partitioned cookies into Moli's
        // default partition, which would change their isolation semantics.
        if !row.origin_attributes.is_empty() {
            continue;
        }
        let expires = if row.expiry <= 0 {
            None
        } else {
            Some(
                OffsetDateTime::from_unix_timestamp(row.expiry)
                    .with_context(|| format!("invalid Firefox cookie expiry for `{}`", row.name))?,
            )
        };
        cookies.push(StoredCookie {
            name: row.name,
            value: row.value,
            host_only: !row.host.starts_with('.'),
            domain: row.host,
            path: row.path,
            secure: row.secure,
            http_only: row.http_only,
            expires,
            same_site: match row.same_site {
                0 => StoredCookieSameSite::None,
                1 => StoredCookieSameSite::Lax,
                2 => StoredCookieSameSite::Strict,
                _ => StoredCookieSameSite::Unspecified,
            },
            priority: Some(CookiePriority::Medium),
            partition_key: None,
            source_scheme: if row.scheme_map & 0x02 != 0 || row.secure {
                StoredCookieSourceScheme::Secure
            } else if row.scheme_map & 0x01 != 0 {
                StoredCookieSourceScheme::NonSecure
            } else {
                StoredCookieSourceScheme::Unset
            },
            source_port: -1,
            creation_index: cookies.len() as u64,
            last_access_index: 0,
        });
    }
    Ok(cookies)
}

pub(crate) fn read_local_storage(profile: &Path) -> Result<LocalStorage> {
    let root = profile.join("storage/default");
    if !root.is_dir() {
        return Ok(LocalStorage::new());
    }
    let mut databases = Vec::new();
    collect_local_storage_databases(&root, &mut databases)?;
    databases.sort();
    let mut origins = LocalStorage::new();
    for source in databases {
        let snapshot = SqliteSnapshot::open(&source, "Firefox localStorage")?;
        let connection = &snapshot.connection;
        let origin: String = connection
            .query_row("SELECT origin FROM database LIMIT 1", [], |row| row.get(0))
            .with_context(|| format!("failed to read origin from `{}`", source.display()))?;
        let mut statement =
            connection.prepare("SELECT key,conversion_type,compression_type,value FROM data")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        let entries = origins.entry(origin).or_default();
        for row in rows {
            let (key, conversion, compression, value) = row?;
            let value = decode_local_storage_value(value, conversion, compression)
                .with_context(|| format!("failed to decode Firefox localStorage `{key}`"))?;
            entries.insert(key.encode_utf16().collect(), value.into());
        }
    }
    Ok(origins)
}

struct FirefoxCookieRow {
    name: String,
    value: String,
    host: String,
    path: String,
    expiry: i64,
    secure: bool,
    http_only: bool,
    same_site: i64,
    scheme_map: i64,
    origin_attributes: String,
}

fn collect_local_storage_databases(
    directory: &Path,
    databases: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| {
        format!(
            "failed to read Firefox storage directory `{}`",
            directory.display()
        )
    })? {
        let path = entry?.path();
        if path.is_dir() {
            collect_local_storage_databases(&path, databases)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("data.sqlite")
            && path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some("ls")
        {
            databases.push(path);
        }
    }
    Ok(())
}

fn decode_local_storage_value(value: Vec<u8>, conversion: i64, compression: i64) -> Result<String> {
    let value = match compression {
        0 => value,
        1 => snap::raw::Decoder::new()
            .decompress_vec(&value)
            .context("invalid Snappy payload")?,
        other => bail!("unsupported Firefox localStorage compression type {other}"),
    };
    match conversion {
        0 => {
            if value.len() % 2 != 0 {
                bail!("UTF-16 value has an odd byte length");
            }
            let units = value
                .chunks_exact(2)
                .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            String::from_utf16(&units).context("invalid UTF-16 value")
        }
        1 => String::from_utf8(value).context("invalid UTF-8 value"),
        other => bail!("unsupported Firefox localStorage conversion type {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};

    #[test]
    fn decodes_utf8_and_snappy_values() -> Result<()> {
        assert_eq!(
            decode_local_storage_value(b"hello".to_vec(), 1, 0)?,
            "hello"
        );
        let compressed = snap::raw::Encoder::new().compress_vec("世界".as_bytes())?;
        assert_eq!(decode_local_storage_value(compressed, 1, 1)?, "世界");
        Ok(())
    }

    #[test]
    fn decodes_native_utf16_values() -> Result<()> {
        let bytes = "hello"
            .encode_utf16()
            .flat_map(u16::to_ne_bytes)
            .collect::<Vec<_>>();
        assert_eq!(decode_local_storage_value(bytes, 0, 0)?, "hello");
        Ok(())
    }

    #[test]
    fn imports_firefox_cookies_and_local_storage_databases() -> Result<()> {
        check_firefox_database_imports("DELETE")
    }

    #[test]
    fn imports_firefox_cookies_and_local_storage_from_wal() -> Result<()> {
        check_firefox_database_imports("WAL")
    }

    fn check_firefox_database_imports(journal_mode: &str) -> Result<()> {
        let temp = tempfile::tempdir()?;
        let profile = temp.path().join("firefox");
        fs::create_dir_all(&profile)?;
        let cookies_db = Connection::open(profile.join("cookies.sqlite"))?;
        cookies_db.pragma_update(None, "journal_mode", journal_mode)?;
        cookies_db.pragma_update(None, "wal_autocheckpoint", 0)?;
        cookies_db.execute_batch(
            "CREATE TABLE moz_cookies (
                name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER,
                isSecure INTEGER, isHttpOnly INTEGER, sameSite INTEGER,
                schemeMap INTEGER, originAttributes TEXT
            );",
        )?;
        cookies_db.execute(
            "INSERT INTO moz_cookies VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                "session",
                "value",
                ".example.com",
                "/",
                2_147_483_647_i64,
                1,
                1,
                1,
                2,
                ""
            ],
        )?;
        let storage_dir = profile.join("storage/default/https+++example.com/ls");
        fs::create_dir_all(&storage_dir)?;
        let storage_db = Connection::open(storage_dir.join("data.sqlite"))?;
        storage_db.pragma_update(None, "journal_mode", journal_mode)?;
        storage_db.pragma_update(None, "wal_autocheckpoint", 0)?;
        storage_db.execute_batch(
            "CREATE TABLE database (origin TEXT NOT NULL);
             CREATE TABLE data (
                key TEXT PRIMARY KEY, utf16_length INTEGER NOT NULL,
                conversion_type INTEGER NOT NULL, compression_type INTEGER NOT NULL,
                last_access_time INTEGER NOT NULL DEFAULT 0, value BLOB NOT NULL
             );",
        )?;
        storage_db.execute("INSERT INTO database VALUES ('https://example.com')", [])?;
        let compressed = snap::raw::Encoder::new().compress_vec(b"stored-value")?;
        storage_db.execute(
            "INSERT INTO data VALUES (?1,?2,?3,?4,0,?5)",
            params!["stored-key", 12, 1, 1, compressed],
        )?;

        // Keep both connections open so WAL data is not checkpointed on close.
        if journal_mode == "WAL" {
            assert!(profile.join("cookies.sqlite-wal").is_file());
            assert!(storage_dir.join("data.sqlite-wal").is_file());
        }

        let cookies = import_cookies(&profile)?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        assert_eq!(cookies[0].value, "value");
        assert_eq!(cookies[0].same_site, StoredCookieSameSite::Lax);
        let destination = temp.path().join("localstorage.json");
        let origins = read_local_storage(&profile)?;
        assert_eq!(origins["https://example.com"].len(), 1);
        let bytes = crate::storage::prepare(&destination, origins)?.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(
            json["origins"]["https://example.com"]["entries"][0]["value"],
            "stored-value"
        );
        Ok(())
    }
}
