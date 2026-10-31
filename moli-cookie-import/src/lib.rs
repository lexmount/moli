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
use moli_cookie_jar::{StoredCookieSetRejectionReason, StoredCookieSetReport};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieImportRejection {
    pub reason: StoredCookieSetRejectionReason,
    pub count: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    /// Accepted write attempts, including replacements of the same identity.
    pub cookies: usize,
    pub cookies_rejected: usize,
    /// Unexpired cookies in the destination after import, including preexisting
    /// cookies. Zero when no cookie import was attempted. Quotas and replacement
    /// can make this smaller than the number of accepted writes.
    pub cookies_stored: usize,
    /// Counts for all reported reasons; a rejected write can have several reasons.
    pub cookie_rejections: Vec<CookieImportRejection>,
    pub storage: usize,
}

impl ImportSummary {
    fn record_cookie(&mut self, report: StoredCookieSetReport) {
        if report.is_accepted() {
            self.cookies += 1;
        } else {
            self.cookies_rejected += 1;
            for reason in report.rejection_reasons {
                if let Some(rejection) = self
                    .cookie_rejections
                    .iter_mut()
                    .find(|r| r.reason == reason)
                {
                    rejection.count += 1;
                } else {
                    self.cookie_rejections
                        .push(CookieImportRejection { reason, count: 1 });
                }
            }
        }
    }
}

impl fmt::Display for ImportSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.storage > 0 {
            parts.push(format!("storage={}", self.storage));
        }
        if self.cookies > 0 || self.cookies_rejected > 0 {
            parts.push(format!(
                "cookies accepted={}, rejected={}, stored in destination={}",
                self.cookies, self.cookies_rejected, self.cookies_stored
            ));
        }
        if !self.cookie_rejections.is_empty() {
            let reasons = self
                .cookie_rejections
                .iter()
                .map(|r| format!("{:?}={}", r.reason, r.count))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("cookie rejections=[{reasons}]"));
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

    let mut summary = ImportSummary {
        storage: imported_storage.values().map(|entries| entries.len()).sum(),
        ..ImportSummary::default()
    };
    // Read all sources and validate the destination before accepting changes.
    // Each database commits independently; this is not a cross-file transaction.
    let mut runtime = moli_sqlite::Runtime::new()?;
    let cookies = if imported_cookies.is_empty() {
        None
    } else {
        Some(moli_cookie_db::CookieDatabase::open(
            partition.cookies_path().to_owned(),
            moli_sqlite::OpenMode::Create,
            runtime.handle(),
        )?)
    };
    if imported_storage.values().any(|entries| !entries.is_empty()) {
        moli_web_storage::import_sqlite_web_storage(
            partition.local_storage_path().to_owned(),
            runtime.handle(),
            storage::into_sqlite_areas(imported_storage),
        )?;
    }
    if let Some((store, owner)) = cookies {
        {
            let mut store = store.lock()?;
            for cookie in imported_cookies {
                summary.record_cookie(store.upsert_with_request_url_report(
                    cookie,
                    None,
                    moli_cookie_jar::CookieSource::Cdp,
                ));
            }
            summary.cookies_stored = store.cookies().len();
        }
        owner.close()?;
    }
    runtime.shutdown()?;

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
