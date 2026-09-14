//! Import browser cookies and local storage into a Moli browser profile.

mod chrome;
mod firefox;

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
    let mut summary = ImportSummary::default();
    let mut imported_cookies = Vec::new();

    if let Some(source) = request.chrome_profile_dir {
        chrome::validate_source(source)?;
        if request.includes.contains(ImportIncludes::COOKIES) {
            imported_cookies.extend(chrome::import_cookies(
                source,
                request
                    .chrome_crypto_key
                    .unwrap_or(&ChromeCryptoKey::System),
            )?);
        }
        if request.includes.contains(ImportIncludes::STORAGE) {
            summary.storage +=
                chrome::import_local_storage(source, partition.local_storage_path())?;
        }
    }

    if let Some(source) = request.firefox_profile_dir {
        firefox::validate_source(source)?;
        if request.includes.contains(ImportIncludes::COOKIES) {
            imported_cookies.extend(firefox::import_cookies(source)?);
        }
        if request.includes.contains(ImportIncludes::STORAGE) {
            summary.storage +=
                firefox::import_local_storage(source, partition.local_storage_path())?;
        }
    }

    if request.includes.contains(ImportIncludes::COOKIES) {
        for jar in request.cookie_jars {
            imported_cookies.extend(
                moli_cookie_cache::load_cookie_file(jar)
                    .with_context(|| format!("failed to import cookie jar `{}`", jar.display()))?,
            );
        }
        summary.cookies = imported_cookies.len();
        if summary.cookies > 0 {
            let mut cookies = moli_cookie_cache::load_cookie_cache(partition.cookies_path())?;
            cookies.extend(imported_cookies);
            moli_cookie_cache::save_cookie_cache(partition.cookies_path(), cookies)?;
        }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;

    use super::*;

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
        let cookies =
            moli_cookie_cache::load_cookie_cache(profile.default_partition().cookies_path())?;
        assert_eq!(summary.cookies, 1);
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        Ok(())
    }
}
