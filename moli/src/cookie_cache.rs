use std::path::Path;

use anyhow::{Context, Result};
use moli_cookie_cache::{
    load_cookie_cache as load_persisted_cookie_cache,
    save_cookie_cache as save_persisted_cookie_cache,
};
use moli_cookie_jar::StoredCookie;
use moli_core::runtime::Browser;

pub use moli_cookie_cache::load_cookie_file;

pub fn load_browser_cookie_cache(browser: &Browser, path: impl AsRef<Path>) -> Result<usize> {
    browser.import_cookies(load_cookie_cache(path)?)
}

pub fn load_cookie_cache(path: impl AsRef<Path>) -> Result<Vec<StoredCookie>> {
    load_persisted_cookie_cache(path)
}

pub fn load_browser_cookie_file(browser: &Browser, path: impl AsRef<Path>) -> Result<usize> {
    browser.import_cookies(load_cookie_file(path)?)
}

pub fn save_browser_cookie_file(
    browser: &Browser,
    path: impl AsRef<Path>,
) -> Result<moli_cookie_cache::CookieFileSaveReport> {
    let cookies = browser
        .cookies()
        .context("failed to snapshot browser cookies for cookie jar")?;
    moli_cookie_cache::save_cookie_file(path, cookies)
}

pub fn save_browser_cookie_cache(browser: &Browser, path: impl AsRef<Path>) -> Result<()> {
    save_cookie_cache(
        path,
        browser
            .cookies()
            .context("failed to snapshot browser cookies for cache")?,
    )
}

pub fn save_cookie_cache(
    path: impl AsRef<Path>,
    cookies: impl IntoIterator<Item = StoredCookie>,
) -> Result<()> {
    save_persisted_cookie_cache(path, cookies)
}
