use std::{collections::BTreeMap, fs, path::Path};

#[cfg(target_os = "macos")]
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use moli_cookie_jar::{
    CookiePriority, StoredCookie, StoredCookiePartitionKey, StoredCookieSameSite,
    StoredCookieSourceScheme,
};
#[cfg(target_os = "macos")]
use moli_crypto::derive_pbkdf2_hmac_sha1;
use moli_crypto::{aes_128_cbc_pkcs7_decrypt, aes_256_gcm_decrypt, sha256_digest};
use rusqlite::Connection;
use rusty_leveldb::{DB, LdbIterator, Options};
use tempfile::TempDir;
use time::OffsetDateTime;

use crate::ChromeCryptoKey;

const CHROME_EPOCH_OFFSET_MICROS: i64 = 11_644_473_600_000_000;

pub(crate) fn validate_source(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!(
            "Chrome profile directory `{}` does not exist",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn import_cookies(
    profile: &Path,
    key_source: &ChromeCryptoKey,
) -> Result<Vec<StoredCookie>> {
    let source = [profile.join("Network/Cookies"), profile.join("Cookies")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow!(
                "Chrome Cookies database was not found under `{}`",
                profile.display()
            )
        })?;
    let temp = tempfile::NamedTempFile::new().context("failed to create Cookies snapshot")?;
    fs::copy(&source, temp.path()).with_context(|| {
        format!(
            "failed to snapshot Chrome Cookies database `{}`",
            source.display()
        )
    })?;
    let connection =
        Connection::open(temp.path()).context("failed to open Chrome Cookies snapshot")?;
    let schema_version = connection
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let has_partition_key = cookie_column_exists(&connection, "top_frame_site_key")?;
    let has_cross_site_ancestor = cookie_column_exists(&connection, "has_cross_site_ancestor")?;
    let partition_key_column = if has_partition_key {
        "top_frame_site_key"
    } else {
        "NULL"
    };
    let cross_site_column = if has_cross_site_ancestor {
        "has_cross_site_ancestor"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT host_key,name,value,encrypted_value,path,expires_utc,is_secure,is_httponly,samesite,priority,source_scheme,source_port,{partition_key_column},{cross_site_column} FROM cookies"
    );
    let mut statement = connection
        .prepare(&sql)
        .context("Chrome Cookies database has an unsupported schema")?;
    let rows = statement.query_map([], |row| {
        Ok(ChromeCookieRow {
            host: row.get(0)?,
            name: row.get(1)?,
            value: row.get(2)?,
            encrypted_value: row.get(3)?,
            path: row.get(4)?,
            expires_utc: row.get(5)?,
            secure: row.get::<_, i64>(6)? != 0,
            http_only: row.get::<_, i64>(7)? != 0,
            same_site: row.get(8)?,
            priority: row.get(9)?,
            source_scheme: row.get(10)?,
            source_port: row.get(11)?,
            partition_key: row.get(12)?,
            has_cross_site_ancestor: row.get::<_, i64>(13)? != 0,
        })
    })?;
    let mut cookies = Vec::new();
    let mut key = None;
    for row in rows {
        let mut row = row?;
        let value = if row.encrypted_value.is_empty() {
            std::mem::take(&mut row.value)
        } else {
            if key.is_none() {
                key = Some(resolve_crypto_key(key_source)?);
            }
            decrypt_cookie_value(
                &row.encrypted_value,
                key.as_deref().expect("key was resolved"),
                &row.host,
                schema_version,
            )
            .with_context(|| {
                format!(
                    "failed to decrypt Chrome cookie `{}` for `{}`",
                    row.name, row.host
                )
            })?
        };
        cookies.push(row.into_stored(value));
    }
    Ok(cookies)
}

struct ChromeCookieRow {
    host: String,
    name: String,
    value: String,
    encrypted_value: Vec<u8>,
    path: String,
    expires_utc: i64,
    secure: bool,
    http_only: bool,
    same_site: i64,
    priority: i64,
    source_scheme: i64,
    source_port: i32,
    partition_key: Option<String>,
    has_cross_site_ancestor: bool,
}

impl ChromeCookieRow {
    fn into_stored(self, value: String) -> StoredCookie {
        StoredCookie {
            name: self.name,
            value,
            host_only: !self.host.starts_with('.'),
            domain: self.host,
            path: self.path,
            secure: self.secure,
            http_only: self.http_only,
            expires: chrome_expiry(self.expires_utc),
            same_site: match self.same_site {
                0 => StoredCookieSameSite::None,
                1 => StoredCookieSameSite::Lax,
                2 => StoredCookieSameSite::Strict,
                _ => StoredCookieSameSite::Unspecified,
            },
            priority: Some(match self.priority {
                0 => CookiePriority::Low,
                2 => CookiePriority::High,
                _ => CookiePriority::Medium,
            }),
            partition_key: self
                .partition_key
                .filter(|key| !key.is_empty())
                .map(|key| StoredCookiePartitionKey::site(key, self.has_cross_site_ancestor)),
            source_scheme: match self.source_scheme {
                1 => StoredCookieSourceScheme::NonSecure,
                2 => StoredCookieSourceScheme::Secure,
                _ => StoredCookieSourceScheme::Unset,
            },
            source_port: self.source_port,
            creation_index: 0,
            last_access_index: 0,
        }
    }
}

fn cookie_column_exists(connection: &Connection, name: &str) -> Result<bool> {
    let mut statement = connection.prepare("PRAGMA table_info(cookies)")?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn chrome_expiry(micros: i64) -> Option<OffsetDateTime> {
    if micros == 0 {
        return None;
    }
    let unix_micros = micros.checked_sub(CHROME_EPOCH_OFFSET_MICROS)?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_micros) * 1_000).ok()
}

fn resolve_crypto_key(source: &ChromeCryptoKey) -> Result<Vec<u8>> {
    match source {
        ChromeCryptoKey::Base64(value) => STANDARD
            .decode(value)
            .context("--chrome-crypto-key base64 value is invalid"),
        ChromeCryptoKey::System => system_crypto_key(),
    }
}

#[cfg(target_os = "macos")]
fn system_crypto_key() -> Result<Vec<u8>> {
    let output = Command::new("security")
        .args(["find-generic-password", "-w", "-s", "Chrome Safe Storage"])
        .output()
        .context("failed to ask macOS Keychain for Chrome Safe Storage")?;
    if !output.status.success() {
        bail!("macOS Keychain did not grant access to Chrome Safe Storage");
    }
    let password =
        String::from_utf8(output.stdout).context("Chrome Safe Storage password was not UTF-8")?;
    let mut key = [0_u8; 16];
    derive_pbkdf2_hmac_sha1(
        password.trim_end().as_bytes(),
        b"saltysalt",
        std::num::NonZeroU32::new(1003).expect("Chrome iteration count is non-zero"),
        &mut key,
    );
    Ok(key.to_vec())
}

#[cfg(not(target_os = "macos"))]
fn system_crypto_key() -> Result<Vec<u8>> {
    bail!(
        "system Chrome key import is currently available on macOS; use --chrome-crypto-key base64:<raw-key> on this platform"
    )
}

fn decrypt_cookie_value(
    encrypted: &[u8],
    key: &[u8],
    host: &str,
    schema_version: i64,
) -> Result<String> {
    let payload = encrypted
        .strip_prefix(b"v10")
        .or_else(|| encrypted.strip_prefix(b"v11"))
        .ok_or_else(|| anyhow!("unsupported encrypted cookie prefix"))?;
    let plaintext = if key.len() == 32 && payload.len() > 12 + 16 {
        aes_256_gcm_decrypt(key, &payload[..12], &payload[12..])
            .map_err(|_| anyhow!("AES-GCM authentication failed"))?
    } else if key.len() == 16 {
        aes_128_cbc_pkcs7_decrypt(key, &[b' '; 16], payload)
            .map_err(|_| anyhow!("AES-CBC decryption failed"))?
    } else {
        bail!("Chrome crypto key must decode to 16 or 32 bytes");
    };
    let plaintext = if schema_version >= 24 {
        let digest = sha256_digest(host.as_bytes());
        plaintext
            .strip_prefix(&digest)
            .ok_or_else(|| anyhow!("decrypted cookie host digest does not match"))?
    } else {
        &plaintext
    };
    String::from_utf8(plaintext.to_vec()).context("decrypted cookie value is not UTF-8")
}

pub(crate) fn import_local_storage(profile: &Path, destination: &Path) -> Result<usize> {
    let source = profile.join("Local Storage/leveldb");
    if !source.is_dir() {
        return Ok(0);
    }
    let snapshot = TempDir::new().context("failed to create localStorage snapshot directory")?;
    copy_dir(&source, snapshot.path())?;
    let options = Options {
        create_if_missing: false,
        ..Options::default()
    };
    let mut database = DB::open(snapshot.path(), options)
        .map_err(|error| anyhow!("failed to open Chrome localStorage LevelDB snapshot: {error}"))?;
    let mut origins = BTreeMap::<String, BTreeMap<Vec<u16>, Vec<u16>>>::new();
    let mut iterator = database
        .new_iter()
        .map_err(|error| anyhow!("failed to iterate Chrome localStorage: {error}"))?;
    iterator.seek_to_first();
    let mut imported = 0;
    while let Some((key, value)) = iterator.next() {
        let Some((origin, item_key)) = decode_local_storage_key(&key) else {
            continue;
        };
        let Some(item_value) = decode_chrome_string(&value) else {
            continue;
        };
        origins
            .entry(origin)
            .or_default()
            .insert(item_key, item_value);
        imported += 1;
    }
    let origins = origins
        .into_iter()
        .map(|(origin, entries)| {
            let entries = entries
                .into_iter()
                .map(|(key, value)| {
                    serde_json::json!({
                        "key": json_dom_string(key),
                        "value": json_dom_string(value),
                    })
                })
                .collect::<Vec<_>>();
            (origin, serde_json::json!({ "entries": entries }))
        })
        .collect::<serde_json::Map<_, _>>();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "origins": origins,
    }))?;
    moli_browser_profile::write_file_atomically(destination, &bytes, "Chrome localStorage import")?;
    Ok(imported)
}

fn json_dom_string(units: Vec<u16>) -> serde_json::Value {
    match String::from_utf16(&units) {
        Ok(text) => serde_json::Value::String(text),
        Err(_) => serde_json::json!({ "utf16": units }),
    }
}

fn decode_local_storage_key(key: &[u8]) -> Option<(String, Vec<u16>)> {
    let key = key.strip_prefix(b"_")?;
    let separator = key.iter().position(|byte| *byte == 0)?;
    let origin = std::str::from_utf8(&key[..separator]).ok()?.to_owned();
    let item = decode_chrome_string(&key[separator + 1..])?;
    Some((origin, item))
}

fn decode_chrome_string(value: &[u8]) -> Option<Vec<u16>> {
    let (&encoding, bytes) = value.split_first()?;
    match encoding {
        0 => {
            if bytes.len() % 2 != 0 {
                return None;
            }
            Some(
                bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect(),
            )
        }
        1 => Some(bytes.iter().map(|byte| u16::from(*byte)).collect()),
        _ => None,
    }
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read `{}`", source.display()))?
    {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{chrome_expiry, decode_chrome_string, decode_local_storage_key};

    #[test]
    fn decodes_chrome_local_storage_latin1_key() {
        let decoded = decode_local_storage_key(b"_https://example.test\0\x01token").unwrap();
        assert_eq!(decoded.0, "https://example.test");
        assert_eq!(String::from_utf16(&decoded.1).unwrap(), "token");
    }

    #[test]
    fn decodes_chrome_utf16_string() {
        assert_eq!(
            decode_chrome_string(&[0, b'a', 0, 0xac, 0x20]),
            Some(vec![97, 8364])
        );
    }

    #[test]
    fn converts_chrome_epoch() {
        assert_eq!(
            chrome_expiry(11_644_473_601_000_000)
                .unwrap()
                .unix_timestamp(),
            1
        );
    }
}
