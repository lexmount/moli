//! Import browser cookies and local storage into a Moli browser profile.

mod chrome;
mod firefox;
mod sqlite;
mod storage;

use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use moli_browser_profile::BrowserProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportIncludes(u8);

impl ImportIncludes {
    pub const COOKIES: u8 = 1;
    pub const STORAGE: u8 = 2;
    pub const INDEXED_DB: u8 = 4;
    pub const ALL: u8 = Self::COOKIES | Self::STORAGE;

    pub fn contains(self, kind: u8) -> bool {
        self.0 & kind != 0
    }
}

impl FromStr for ImportIncludes {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut bits = 0;
        for item in value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            bits |= match item.to_ascii_lowercase().as_str() {
                "all" => Self::ALL,
                "cookies" => Self::COOKIES,
                "storage" | "localstorage" => Self::STORAGE,
                "indexeddb" => Self::INDEXED_DB,
                other => return Err(format!("unknown import kind `{other}`")),
            };
        }
        if bits == 0 {
            return Err("at least one import kind is required".to_owned());
        }
        Ok(Self(bits))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChromeCryptoKey {
    System,
    Base64(String),
}

impl FromStr for ChromeCryptoKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("system") {
            return Ok(Self::System);
        }
        value
            .strip_prefix("base64:")
            .filter(|key| !key.is_empty())
            .map(|key| Self::Base64(key.to_owned()))
            .ok_or_else(|| "crypto key must be `system` or `base64:<raw-key>`".to_owned())
    }
}

pub struct ImportRequest<'a> {
    pub profile_dir: &'a Path,
    pub includes: ImportIncludes,
    pub chrome_profile_dir: Option<&'a Path>,
    pub chrome_crypto_key: Option<&'a ChromeCryptoKey>,
    pub firefox_profile_dir: Option<&'a Path>,
    pub cookie_jars: &'a [PathBuf],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub cookies: usize,
    pub storage: usize,
}

impl fmt::Display for ImportSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.storage > 0 {
            parts.push(format!("storage={}", self.storage));
        }
        if self.cookies > 0 {
            parts.push(format!("cookies={}", self.cookies));
        }
        formatter.write_str(&parts.join(", "))
    }
}

pub fn import_session_state(request: &ImportRequest<'_>) -> Result<ImportSummary> {
    validate_request(request)?;
    let profile = BrowserProfile::open(request.profile_dir).with_context(|| {
        format!(
            "failed to open destination Moli profile `{}`",
            request.profile_dir.display()
        )
    })?;
    let partition = profile.default_partition();
    let mut imported_cookies = Vec::new();
    let mut imported_storage = storage::LocalStorage::new();

    if let Some(source) = request.chrome_profile_dir {
        if request.includes.contains(ImportIncludes::COOKIES) {
            imported_cookies.extend(chrome::import_cookies(
                source,
                request
                    .chrome_crypto_key
                    .unwrap_or(&ChromeCryptoKey::System),
            )?);
        }
        if request.includes.contains(ImportIncludes::STORAGE) {
            storage::merge(&mut imported_storage, chrome::read_local_storage(source)?);
        }
    }

    if let Some(source) = request.firefox_profile_dir {
        if request.includes.contains(ImportIncludes::COOKIES) {
            imported_cookies.extend(firefox::import_cookies(source)?);
        }
        if request.includes.contains(ImportIncludes::STORAGE) {
            storage::merge(&mut imported_storage, firefox::read_local_storage(source)?);
        }
    }

    if request.includes.contains(ImportIncludes::COOKIES) {
        for jar in request.cookie_jars {
            imported_cookies.extend(
                moli_cookie_cache::load_cookie_file(jar)
                    .with_context(|| format!("failed to import cookie jar `{}`", jar.display()))?,
            );
        }
    }

    let summary = ImportSummary {
        cookies: imported_cookies.len(),
        storage: imported_storage.values().map(|entries| entries.len()).sum(),
    };
    // Read and merge every source and existing destination before writing.
    // Each file is replaced atomically; the two files are not a transaction.
    let cookies = if imported_cookies.is_empty() {
        None
    } else {
        let mut cookies = moli_cookie_cache::load_cookie_cache(partition.cookies_path())?;
        cookies.extend(imported_cookies);
        Some(cookies)
    };
    let storage = storage::prepare(partition.local_storage_path(), imported_storage)?;
    if let Some(bytes) = storage {
        moli_browser_profile::write_profile_file(
            partition.local_storage_path(),
            &bytes,
            "localStorage import",
        )?;
    }
    if let Some(cookies) = cookies {
        moli_cookie_cache::save_cookie_cache(partition.cookies_path(), cookies)?;
    }

    Ok(summary)
}

fn validate_request(request: &ImportRequest<'_>) -> Result<()> {
    if request.includes.contains(ImportIncludes::INDEXED_DB) {
        bail!(
            "IndexedDB import is not safe yet: browser LevelDB/V8 formats are not compatible with Moli's JSON backend"
        );
    }
    if !request.cookie_jars.is_empty() && !request.includes.contains(ImportIncludes::COOKIES) {
        bail!("cookie jars can only import cookies; include `cookies` in the import selection");
    }
    for (browser, source) in [
        ("Chrome", request.chrome_profile_dir),
        ("Firefox", request.firefox_profile_dir),
    ] {
        if let Some(source) = source
            && !source.is_dir()
        {
            bail!(
                "{browser} profile directory `{}` does not exist",
                source.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
