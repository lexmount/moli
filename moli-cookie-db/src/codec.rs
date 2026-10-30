use super::*;
use rusqlite::{Connection, params, types::Type};
use time::{Duration, OffsetDateTime};

pub(super) const APPLICATION_ID: i32 = 0x4d4c434b; // MLCK
pub(super) fn initialize(connection: &mut Connection) -> Result<(), WritebackError> {
    let application: i32 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if application == 0 && version == 0 {
        let objects: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if objects != 0 {
            return Err(WritebackError::new("unidentified Cookie database"));
        }
        let tx = connection.transaction()?;
        tx.execute_batch("CREATE TABLE cookies(
            partition_key BLOB NOT NULL,domain TEXT NOT NULL,path TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,
            host_only INTEGER NOT NULL CHECK(host_only IN (0,1)),secure INTEGER NOT NULL CHECK(secure IN (0,1)),http_only INTEGER NOT NULL CHECK(http_only IN (0,1)),
            expires_seconds INTEGER,expires_nanos INTEGER CHECK(expires_nanos>=0 AND expires_nanos<1000000000),
            same_site INTEGER NOT NULL CHECK(same_site BETWEEN 0 AND 3),priority INTEGER CHECK(priority BETWEEN 0 AND 2),
            source_scheme INTEGER NOT NULL CHECK(source_scheme BETWEEN 0 AND 2),source_port INTEGER NOT NULL,
            creation_index BLOB NOT NULL CHECK(length(creation_index)=8),last_access_index BLOB NOT NULL CHECK(length(last_access_index)=8),
            creation_seconds INTEGER,creation_nanos INTEGER CHECK(creation_nanos>=0 AND creation_nanos<1000000000),
            CHECK((creation_seconds IS NULL)=(creation_nanos IS NULL)),
            CHECK((expires_seconds IS NULL)=(expires_nanos IS NULL)),PRIMARY KEY(partition_key,domain,path,name)) WITHOUT ROWID;")?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    } else if application == APPLICATION_ID && version == 1 {
        let tx = connection.transaction()?;
        tx.execute_batch(
            "ALTER TABLE cookies ADD COLUMN creation_seconds INTEGER;
            ALTER TABLE cookies ADD COLUMN creation_nanos INTEGER
            CHECK(creation_nanos>=0 AND creation_nanos<1000000000)
            CHECK((creation_seconds IS NULL)=(creation_nanos IS NULL));",
        )?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    } else if application != APPLICATION_ID || version != 2 {
        return Err(WritebackError::new(format!(
            "unsupported Cookie database identity/version {application}/{version}"
        )));
    }
    connection.prepare("SELECT partition_key,domain,path,name,value,host_only,secure,http_only,expires_seconds,expires_nanos,same_site,priority,source_scheme,source_port,creation_index,last_access_index,creation_seconds,creation_nanos FROM cookies LIMIT 0")?;
    Ok(())
}
pub(super) fn partition_bytes(key: Option<&StoredCookiePartitionKey>) -> Vec<u8> {
    match key {
        None => vec![0],
        Some(StoredCookiePartitionKey::Site {
            top_level_site,
            has_cross_site_ancestor,
        }) => {
            let mut bytes = vec![1, u8::from(*has_cross_site_ancestor)];
            bytes.extend_from_slice(top_level_site.as_bytes());
            bytes
        }
        Some(StoredCookiePartitionKey::Opaque {
            nonce,
            has_cross_site_ancestor,
        }) => {
            let mut bytes = vec![2, u8::from(*has_cross_site_ancestor)];
            bytes.extend_from_slice(&nonce.to_be_bytes());
            bytes
        }
    }
}
fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, Type::Blob, Box::new(WritebackError::new(message)))
}
fn partition_from_bytes(bytes: Vec<u8>) -> rusqlite::Result<Option<StoredCookiePartitionKey>> {
    match bytes.as_slice() {
        [0] => Ok(None),
        [1, ancestor, site @ ..] if *ancestor <= 1 && !site.is_empty() => {
            Ok(Some(StoredCookiePartitionKey::Site {
                top_level_site: String::from_utf8(site.to_vec())
                    .map_err(|_| invalid("invalid Cookie site encoding"))?,
                has_cross_site_ancestor: *ancestor == 1,
            }))
        }
        [2, ancestor, nonce @ ..] if *ancestor <= 1 && nonce.len() == 8 => {
            Ok(Some(StoredCookiePartitionKey::Opaque {
                nonce: u64::from_be_bytes(nonce.try_into().unwrap()),
                has_cross_site_ancestor: *ancestor == 1,
            }))
        }
        _ => Err(invalid("invalid Cookie partition key")),
    }
}
fn index(bytes: Vec<u8>) -> rusqlite::Result<u64> {
    Ok(u64::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| invalid("invalid Cookie ordering index"))?,
    ))
}
fn flag(row: &rusqlite::Row<'_>, position: usize) -> rusqlite::Result<bool> {
    match row.get::<_, i64>(position)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("invalid Cookie flag")),
    }
}
pub(super) fn read(connection: &mut Connection) -> Result<Vec<StoredCookie>, WritebackError> {
    let mut statement=connection.prepare("SELECT partition_key,domain,path,name,value,host_only,secure,http_only,expires_seconds,expires_nanos,same_site,priority,source_scheme,source_port,creation_index,last_access_index,creation_seconds,creation_nanos FROM cookies ORDER BY creation_index")?;
    let rows = statement.query_map([], |row| {
        let seconds: Option<i64> = row.get(8)?;
        let nanos: Option<i64> = row.get(9)?;
        let expires = match (seconds, nanos) {
            (None, None) => None,
            (Some(seconds), Some(nanos)) if (0..1_000_000_000).contains(&nanos) => Some(
                OffsetDateTime::from_unix_timestamp(seconds)
                    .ok()
                    .and_then(|time| time.checked_add(Duration::nanoseconds(nanos)))
                    .ok_or_else(|| invalid("invalid Cookie expiry"))?,
            ),
            _ => return Err(invalid("invalid Cookie expiry fields")),
        };
        let same_site = match row.get::<_, i64>(10)? {
            0 => StoredCookieSameSite::Unspecified,
            1 => StoredCookieSameSite::Lax,
            2 => StoredCookieSameSite::Strict,
            3 => StoredCookieSameSite::None,
            _ => return Err(invalid("invalid Cookie SameSite")),
        };
        let priority = match row.get::<_, Option<i64>>(11)? {
            None => None,
            Some(0) => Some(CookiePriority::Low),
            Some(1) => Some(CookiePriority::Medium),
            Some(2) => Some(CookiePriority::High),
            _ => return Err(invalid("invalid Cookie priority")),
        };
        let source_scheme = match row.get::<_, i64>(12)? {
            0 => StoredCookieSourceScheme::Unset,
            1 => StoredCookieSourceScheme::NonSecure,
            2 => StoredCookieSourceScheme::Secure,
            _ => return Err(invalid("invalid Cookie source scheme")),
        };
        Ok(StoredCookie {
            partition_key: partition_from_bytes(row.get(0)?)?,
            domain: row.get(1)?,
            path: row.get(2)?,
            name: row.get(3)?,
            value: row.get(4)?,
            host_only: flag(row, 5)?,
            secure: flag(row, 6)?,
            http_only: flag(row, 7)?,
            expires,
            same_site,
            priority,
            source_scheme,
            source_port: row.get(13)?,
            creation_index: index(row.get(14)?)?,
            creation_time: match (
                row.get::<_, Option<i64>>(16)?,
                row.get::<_, Option<i64>>(17)?,
            ) {
                (None, None) => None,
                (Some(seconds), Some(nanos)) if (0..1_000_000_000).contains(&nanos) => Some(
                    OffsetDateTime::from_unix_timestamp(seconds)
                        .ok()
                        .and_then(|time| time.checked_add(Duration::nanoseconds(nanos)))
                        .ok_or_else(|| invalid("invalid Cookie creation time"))?,
                ),
                _ => return Err(invalid("invalid Cookie creation time fields")),
            },
            last_access_index: index(row.get(15)?)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
pub(super) fn upsert(
    connection: &Connection,
    key: &Key,
    cookie: &StoredCookie,
) -> Result<(), WritebackError> {
    let same_site = match cookie.same_site {
        StoredCookieSameSite::Unspecified => 0,
        StoredCookieSameSite::Lax => 1,
        StoredCookieSameSite::Strict => 2,
        StoredCookieSameSite::None => 3,
    };
    let priority = cookie.priority.map(|p| match p {
        CookiePriority::Low => 0,
        CookiePriority::Medium => 1,
        CookiePriority::High => 2,
    });
    let source = match cookie.source_scheme {
        StoredCookieSourceScheme::Unset => 0,
        StoredCookieSourceScheme::NonSecure => 1,
        StoredCookieSourceScheme::Secure => 2,
    };
    connection.execute("INSERT INTO cookies(partition_key,domain,path,name,value,host_only,secure,http_only,expires_seconds,expires_nanos,same_site,priority,source_scheme,source_port,creation_index,last_access_index,creation_seconds,creation_nanos)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
        ON CONFLICT(partition_key,domain,path,name) DO UPDATE SET value=excluded.value,host_only=excluded.host_only,secure=excluded.secure,http_only=excluded.http_only,expires_seconds=excluded.expires_seconds,expires_nanos=excluded.expires_nanos,same_site=excluded.same_site,priority=excluded.priority,source_scheme=excluded.source_scheme,source_port=excluded.source_port,creation_index=excluded.creation_index,last_access_index=excluded.last_access_index,creation_seconds=excluded.creation_seconds,creation_nanos=excluded.creation_nanos",
        params![key.partition,key.domain,key.path,key.name,cookie.value,cookie.host_only,cookie.secure,cookie.http_only,cookie.expires.map(|e|e.unix_timestamp()),cookie.expires.map(|e|i64::from(e.nanosecond())),same_site,priority,source,cookie.source_port,cookie.creation_index.to_be_bytes().as_slice(),cookie.last_access_index.to_be_bytes().as_slice(),cookie.creation_time.map(|t| t.unix_timestamp()),cookie.creation_time.map(|t| i64::from(t.nanosecond()))])?;
    Ok(())
}
