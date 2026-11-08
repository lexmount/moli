use std::io::BufRead;

use anyhow::{Context, Result};
use moli_cookie_jar::StoredCookie;
use netscape_cookie_file_parser::{Cookie, CookiePrefix};
use time::OffsetDateTime;

use crate::netscape::stored_cookie_from_netscape_cookie;

/// Reads Netscape records and scoped `Set-Cookie:` header records in file order.
/// Malformed records are skipped, while I/O errors are returned.
pub fn read_cookie_input_lossy(reader: impl BufRead) -> Result<Vec<StoredCookie>> {
    let mut cookies = Vec::new();
    for line in reader.split(b'\n') {
        let line = line?;
        let parsed = if line
            .get(..11)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"Set-Cookie:"))
        {
            cookie_from_http_header(&line[11..]).ok()
        } else {
            netscape_cookie_file_parser::parse_line(&line)
                .ok()
                .flatten()
        };
        if let Some(parsed) = parsed
            && let Ok(cookie) = stored_cookie_from_netscape_cookie(parsed, cookies.len() as u64)
            && !cookie.is_expired()
        {
            cookies.push(cookie);
        }
    }
    Ok(cookies)
}

fn cookie_from_http_header(line: &[u8]) -> Result<Cookie> {
    let line = std::str::from_utf8(line)?.trim();
    let cookie = cookie::Cookie::parse(line)?;
    let domain = cookie
        .domain()
        .context("cookie header has no Domain attribute")?;
    let path = cookie
        .path()
        .map(|path| path.trim_matches('"'))
        .filter(|path| path.starts_with('/'))
        .unwrap_or("/");
    let expires = if let Some(max_age) = cookie.max_age() {
        if max_age <= time::Duration::ZERO {
            1
        } else {
            OffsetDateTime::now_utc()
                .saturating_add(max_age)
                .unix_timestamp()
                .try_into()?
        }
    } else {
        cookie
            .expires_datetime()
            .map(|date| date.unix_timestamp().max(1) as u64)
            .unwrap_or(0)
    };
    for field in [cookie.name(), cookie.value()] {
        anyhow::ensure!(
            !field.bytes().any(|byte| byte < 0x20 || byte == 0x7f),
            "cookie header contains an invalid name or value"
        );
    }
    Ok(Cookie {
        domain: domain.as_bytes().to_vec(),
        tail_match: true,
        path: path.as_bytes().to_vec(),
        secure: cookie.secure().unwrap_or(false),
        expires,
        name: cookie.name().as_bytes().to_vec(),
        value: cookie.value().as_bytes().to_vec(),
        http_only: cookie.http_only().unwrap_or(false),
        prefix: CookiePrefix::None,
    })
}
