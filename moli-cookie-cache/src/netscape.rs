use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result};
use moli_cookie_jar::{StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme};
use time::OffsetDateTime;

pub fn load_cookie_file(path: impl AsRef<Path>) -> Result<Vec<StoredCookie>> {
    let path = path.as_ref();
    let file = File::open(path)
        .with_context(|| format!("failed to open cookie file `{}`", path.display()))?;
    parse_netscape_cookie_file(BufReader::new(file))
        .with_context(|| format!("failed to parse cookie file `{}`", path.display()))
}

pub(crate) fn parse_netscape_cookie_file(
    reader: impl std::io::BufRead,
) -> Result<Vec<StoredCookie>> {
    let mut cookies = Vec::new();
    for parsed in netscape_cookie_file_parser::parse(reader)? {
        let cookie = stored_cookie_from_netscape_cookie(parsed, cookies.len() as u64)?;
        if !cookie.is_expired() {
            cookies.push(cookie);
        }
    }
    Ok(cookies)
}

fn stored_cookie_from_netscape_cookie(
    cookie: netscape_cookie_file_parser::Cookie,
    creation_index: u64,
) -> Result<StoredCookie> {
    let secure = cookie.secure;
    let name = utf8_cookie_field("name", cookie.name)?;
    anyhow::ensure!(!name.is_empty(), "cookie name is empty");
    let domain = utf8_cookie_field("domain", cookie.domain)?.to_ascii_lowercase();
    anyhow::ensure!(!domain.is_empty(), "cookie domain is empty");
    let path = utf8_cookie_field("path", cookie.path)?;
    anyhow::ensure!(path.starts_with('/'), "cookie path must start with `/`");

    Ok(StoredCookie {
        name,
        value: utf8_cookie_field("value", cookie.value)?,
        domain,
        host_only: !cookie.tail_match,
        path,
        secure,
        http_only: cookie.http_only,
        expires: parse_netscape_expires(cookie.expires)?,
        same_site: StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        // Netscape files only record the Secure attribute. Keep non-Secure
        // cookies unbound to a source scheme so they work over HTTP and HTTPS.
        source_scheme: if secure {
            StoredCookieSourceScheme::Secure
        } else {
            StoredCookieSourceScheme::Unset
        },
        source_port: -1,
        creation_index,
        last_access_index: 0,
    })
}

fn utf8_cookie_field(field: &'static str, value: Vec<u8>) -> Result<String> {
    String::from_utf8(value).with_context(|| format!("cookie {field} is not valid UTF-8"))
}

fn parse_netscape_expires(timestamp: u64) -> Result<Option<OffsetDateTime>> {
    if timestamp == 0 {
        return Ok(None);
    }
    let timestamp = i64::try_from(timestamp).context("cookie expiry timestamp is too large")?;
    Ok(Some(OffsetDateTime::from_unix_timestamp(timestamp)?))
}
