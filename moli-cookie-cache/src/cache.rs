use std::{fs, path::Path};

use anyhow::{Context, Result};
use moli_cookie_jar::{
    CookiePriority, StoredCookie, StoredCookiePartitionKey, StoredCookieSameSite,
    StoredCookieSourceScheme, cookie_creation_time,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use typed_num::Num;

use crate::atomic_file::write_file_atomically;

type CookieCacheVersion = Num<1>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CookieCacheFile {
    version: CookieCacheVersion,
    cookies: Vec<CachedCookie>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCookie {
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
    http_only: bool,
    expires_unix: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creation_unix_micros: Option<i64>,
    same_site: CachedSameSite,
    priority: Option<CachedCookiePriority>,
    #[serde(default)]
    partitioned: bool,
    #[serde(default)]
    partition_key: Option<CachedCookiePartitionKey>,
    source_scheme: CachedSourceScheme,
    source_port: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CachedSameSite {
    Unspecified,
    None,
    Lax,
    Strict,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CachedCookiePriority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CachedSourceScheme {
    Unset,
    NonSecure,
    Secure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCookiePartitionKey {
    top_level_site: String,
    has_cross_site_ancestor: bool,
}

pub fn load_cookie_cache(path: impl AsRef<Path>) -> Result<Vec<StoredCookie>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let bytes = fs::read(path)
        .with_context(|| format!("failed to read cookie cache `{}`", path.display()))?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }

    let cache: CookieCacheFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse cookie cache `{}`", path.display()))?;

    Ok(cache
        .cookies
        .into_iter()
        .filter_map(CachedCookie::into_unexpired_stored_cookie)
        .collect())
}

pub fn save_cookie_cache(
    path: impl AsRef<Path>,
    cookies: impl IntoIterator<Item = StoredCookie>,
) -> Result<()> {
    let path = path.as_ref();
    let cache = CookieCacheFile {
        version: CookieCacheVersion::default(),
        cookies: cookies
            .into_iter()
            .filter_map(CachedCookie::from_stored_cookie)
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&cache).context("failed to serialize cookie cache")?;

    write_file_atomically(path, &bytes, "cookie cache")
}

impl CachedCookie {
    fn into_unexpired_stored_cookie(self) -> Option<StoredCookie> {
        let expires = self
            .expires_unix
            .and_then(|timestamp| OffsetDateTime::from_unix_timestamp(timestamp).ok());
        if expires.is_some_and(|expiry| expiry <= OffsetDateTime::now_utc()) {
            return None;
        }

        if self.partitioned && self.partition_key.is_none() {
            return None;
        }
        let creation_time = self
            .creation_unix_micros
            .and_then(cookie_creation_time::from_unix_micros);

        Some(StoredCookie {
            name: self.name,
            value: self.value,
            domain: self.domain,
            host_only: self.host_only,
            path: self.path,
            secure: self.secure,
            http_only: self.http_only,
            expires,
            same_site: self.same_site.into(),
            priority: self.priority.map(Into::into),
            partition_key: self.partition_key.map(|key| {
                StoredCookiePartitionKey::site(key.top_level_site, key.has_cross_site_ancestor)
            }),
            source_scheme: self.source_scheme.into(),
            source_port: self.source_port,
            creation_index: 0,
            creation_time,
            last_access_index: 0,
        })
    }
}

impl CachedCookie {
    fn from_stored_cookie(cookie: StoredCookie) -> Option<Self> {
        let partition_key = match cookie.partition_key {
            Some(StoredCookiePartitionKey::Site {
                top_level_site,
                has_cross_site_ancestor,
            }) => Some(CachedCookiePartitionKey {
                top_level_site,
                has_cross_site_ancestor,
            }),
            Some(StoredCookiePartitionKey::Opaque { .. }) => return None,
            None => None,
        };
        Some(Self {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain,
            host_only: cookie.host_only,
            path: cookie.path,
            secure: cookie.secure,
            http_only: cookie.http_only,
            expires_unix: cookie.expires.map(|expires| expires.unix_timestamp()),
            creation_unix_micros: cookie
                .creation_time
                .and_then(cookie_creation_time::to_unix_micros),
            same_site: cookie.same_site.into(),
            priority: cookie.priority.map(Into::into),
            partitioned: partition_key.is_some(),
            partition_key,
            source_scheme: cookie.source_scheme.into(),
            source_port: cookie.source_port,
        })
    }
}

impl From<StoredCookieSameSite> for CachedSameSite {
    fn from(value: StoredCookieSameSite) -> Self {
        match value {
            StoredCookieSameSite::Unspecified => Self::Unspecified,
            StoredCookieSameSite::None => Self::None,
            StoredCookieSameSite::Lax => Self::Lax,
            StoredCookieSameSite::Strict => Self::Strict,
        }
    }
}

impl From<CachedSameSite> for StoredCookieSameSite {
    fn from(value: CachedSameSite) -> Self {
        match value {
            CachedSameSite::Unspecified => Self::Unspecified,
            CachedSameSite::None => Self::None,
            CachedSameSite::Lax => Self::Lax,
            CachedSameSite::Strict => Self::Strict,
        }
    }
}

impl From<CookiePriority> for CachedCookiePriority {
    fn from(value: CookiePriority) -> Self {
        match value {
            CookiePriority::Low => Self::Low,
            CookiePriority::Medium => Self::Medium,
            CookiePriority::High => Self::High,
        }
    }
}

impl From<CachedCookiePriority> for CookiePriority {
    fn from(value: CachedCookiePriority) -> Self {
        match value {
            CachedCookiePriority::Low => Self::Low,
            CachedCookiePriority::Medium => Self::Medium,
            CachedCookiePriority::High => Self::High,
        }
    }
}

impl From<StoredCookieSourceScheme> for CachedSourceScheme {
    fn from(value: StoredCookieSourceScheme) -> Self {
        match value {
            StoredCookieSourceScheme::Unset => Self::Unset,
            StoredCookieSourceScheme::NonSecure => Self::NonSecure,
            StoredCookieSourceScheme::Secure => Self::Secure,
        }
    }
}

impl From<CachedSourceScheme> for StoredCookieSourceScheme {
    fn from(value: CachedSourceScheme) -> Self {
        match value {
            CachedSourceScheme::Unset => Self::Unset,
            CachedSourceScheme::NonSecure => Self::NonSecure,
            CachedSourceScheme::Secure => Self::Secure,
        }
    }
}
