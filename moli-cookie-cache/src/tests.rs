use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use moli_cookie_jar::{
    CookiePriority, StoredCookie, StoredCookiePartitionKey, StoredCookieSameSite,
    StoredCookieSourceScheme,
};
use time::OffsetDateTime;

use super::*;
use crate::{atomic_file::write_file_atomically, netscape::parse_netscape_cookie_file};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-cookie-cache-{name}-{}-{nonce}",
            std::process::id()
        ));
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn atomic_write_uses_unique_temp_name_instead_of_fixed_tmp_path() -> Result<()> {
    let temp = TempDir::new("unique");
    let target = temp.path.join("profile.json");
    let mut fixed_tmp = target.as_os_str().to_owned();
    fixed_tmp.push(".tmp");
    let fixed_tmp = PathBuf::from(fixed_tmp);
    fs::create_dir_all(&temp.path)?;
    fs::write(&fixed_tmp, b"stale fixed tmp")?;

    write_file_atomically(&target, b"new profile", "profile test")?;

    assert_eq!(fs::read(&target)?, b"new profile");
    assert_eq!(fs::read(&fixed_tmp)?, b"stale fixed tmp");
    Ok(())
}

#[test]
fn profile_cookie_cache_round_trips_unexpired_cookies() -> Result<()> {
    let temp = TempDir::new("roundtrip");
    let target = temp.path.join("cookies.json");
    let mut cookie = stored_cookie("sid", "fresh");
    cookie.http_only = true;
    cookie.same_site = StoredCookieSameSite::Lax;
    cookie.priority = Some(CookiePriority::High);
    cookie.expires = Some(OffsetDateTime::now_utc() + time::Duration::days(1));

    save_cookie_cache(&target, vec![cookie])?;

    let loaded = load_cookie_cache(&target)?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "sid");
    assert_eq!(loaded[0].value, "fresh");
    assert_eq!(loaded[0].domain, "example.com");
    assert!(loaded[0].host_only);
    assert!(loaded[0].http_only);
    assert_eq!(loaded[0].same_site, StoredCookieSameSite::Lax);
    assert_eq!(loaded[0].priority, Some(CookiePriority::High));
    assert_eq!(loaded[0].source_scheme, StoredCookieSourceScheme::NonSecure);
    Ok(())
}

#[test]
fn profile_cookie_cache_round_trips_site_partition_key() -> Result<()> {
    let temp = TempDir::new("partitioned-roundtrip");
    let target = temp.path.join("cookies.json");
    let mut cookie = stored_cookie("chip", "one");
    cookie.secure = true;
    cookie.source_scheme = StoredCookieSourceScheme::Secure;
    cookie.partition_key = Some(StoredCookiePartitionKey::site(
        "https://top.example".to_owned(),
        true,
    ));

    save_cookie_cache(&target, vec![cookie])?;

    let loaded = load_cookie_cache(&target)?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].partition_key,
        Some(StoredCookiePartitionKey::site(
            "https://top.example".to_owned(),
            true,
        ))
    );
    Ok(())
}

#[test]
fn profile_cookie_cache_does_not_persist_opaque_partition_key() -> Result<()> {
    let temp = TempDir::new("opaque-partition");
    let target = temp.path.join("cookies.json");
    let mut opaque = stored_cookie("opaque", "hidden");
    opaque.partition_key = Some(StoredCookiePartitionKey::opaque(17, false));

    save_cookie_cache(&target, vec![opaque, stored_cookie("plain", "kept")])?;

    let loaded = load_cookie_cache(&target)?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "plain");
    Ok(())
}

#[test]
fn profile_cookie_cache_drops_legacy_partitioned_cookie_without_key() -> Result<()> {
    let temp = TempDir::new("legacy-partitioned");
    let target = temp.path.join("cookies.json");
    fs::create_dir_all(&temp.path)?;
    fs::write(
        &target,
        br#"{
            "version": 1,
            "cookies": [{
                "name": "chip",
                "value": "stale",
                "domain": "example.com",
                "host_only": true,
                "path": "/",
                "secure": true,
                "http_only": false,
                "expires_unix": null,
                "same_site": "none",
                "priority": null,
                "partitioned": true,
                "source_scheme": "secure",
                "source_port": 443
            }]
        }"#,
    )?;

    assert!(load_cookie_cache(&target)?.is_empty());
    Ok(())
}

#[test]
fn profile_cookie_cache_skips_expired_cookies() -> Result<()> {
    let temp = TempDir::new("expired");
    let target = temp.path.join("cookies.json");
    let mut expired = stored_cookie("old", "gone");
    expired.expires = Some(OffsetDateTime::now_utc() - time::Duration::days(1));
    let mut fresh = stored_cookie("new", "kept");
    fresh.expires = Some(OffsetDateTime::now_utc() + time::Duration::days(1));

    save_cookie_cache(&target, vec![expired, fresh])?;

    let loaded = load_cookie_cache(&target)?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "new");
    Ok(())
}

#[test]
fn netscape_cookie_parser_imports_httponly_tailmatch_cookie() -> Result<()> {
    let cookies = parse_netscape_cookie_file(std::io::Cursor::new(
        "#HttpOnly_.example.com\tTRUE\t/account/\tTRUE\t2147483647\tsession\tfixture",
    ))?;

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].domain, "example.com");
    assert!(!cookies[0].host_only);
    assert_eq!(cookies[0].path, "/account/");
    assert!(cookies[0].secure);
    assert!(cookies[0].http_only);
    assert_eq!(cookies[0].name, "session");
    assert_eq!(cookies[0].value, "fixture");
    assert!(cookies[0].expires.is_some());
    Ok(())
}

#[test]
fn netscape_cookie_writer_round_trips_representable_cookies() -> Result<()> {
    let temp = TempDir::new("netscape-write-roundtrip");
    let target = temp.path.join("cookies.txt");
    let mut domain_cookie = stored_cookie("session", "fixture");
    domain_cookie.host_only = false;
    domain_cookie.path = "/account/".to_owned();
    domain_cookie.secure = true;
    domain_cookie.http_only = true;
    domain_cookie.expires = Some(OffsetDateTime::now_utc() + time::Duration::days(1));
    let host_cookie = stored_cookie("host", "session value");

    let report = save_cookie_file(&target, vec![domain_cookie, host_cookie])?;

    assert_eq!(report.written, 2);
    assert_eq!(report.skipped_expired, 0);
    assert_eq!(report.skipped_partitioned, 0);
    let contents = fs::read_to_string(&target)?;
    assert!(contents.starts_with("# Netscape HTTP Cookie File\n"));
    assert!(contents.contains("#HttpOnly_.example.com\tTRUE\t/account/\tTRUE\t"));
    let loaded = load_cookie_file(&target)?;
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].domain, "example.com");
    assert!(!loaded[0].host_only);
    assert_eq!(loaded[0].path, "/account/");
    assert!(loaded[0].secure);
    assert!(loaded[0].http_only);
    assert_eq!(loaded[0].name, "session");
    assert_eq!(loaded[0].value, "fixture");
    assert!(loaded[0].expires.is_some());
    assert!(loaded[1].host_only);
    assert_eq!(loaded[1].name, "host");
    assert_eq!(loaded[1].value, "session value");
    assert_eq!(loaded[1].expires, None);
    Ok(())
}

#[test]
fn netscape_cookie_writer_omits_expired_and_partitioned_cookies() -> Result<()> {
    let temp = TempDir::new("netscape-write-filter");
    let target = temp.path.join("cookies.txt");
    let mut expired = stored_cookie("expired", "gone");
    expired.expires = Some(OffsetDateTime::now_utc() - time::Duration::days(1));
    let mut partitioned = stored_cookie("partitioned", "scoped");
    partitioned.partition_key = Some(StoredCookiePartitionKey::site(
        "https://top.example".to_owned(),
        false,
    ));

    let report = save_cookie_file(
        &target,
        vec![expired, partitioned, stored_cookie("plain", "kept")],
    )?;

    assert_eq!(report.written, 1);
    assert_eq!(report.skipped_expired, 1);
    assert_eq!(report.skipped_partitioned, 1);
    let loaded = load_cookie_file(&target)?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "plain");
    Ok(())
}

#[test]
fn netscape_cookie_writer_rejects_unrepresentable_fields() {
    let temp = TempDir::new("netscape-write-invalid");
    let target = temp.path.join("cookies.txt");
    let cookie = stored_cookie("session", "line\tbreak");

    let error = save_cookie_file(&target, vec![cookie])
        .expect_err("tabs cannot be encoded in Netscape cookie records");

    assert!(
        error
            .to_string()
            .contains("cookie value cannot be represented"),
        "error={error:#}"
    );
    assert!(!target.exists());
}

#[test]
fn netscape_cookie_parser_preserves_trailing_slash_path_scope() -> Result<()> {
    let cookies = parse_netscape_cookie_file(std::io::Cursor::new(
        "example.com\tFALSE\t/account/\tFALSE\t0\ttoken\tvalue",
    ))?;

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].path, "/account/");
    Ok(())
}

#[test]
fn netscape_cookie_parser_rejects_empty_domain() {
    let error =
        parse_netscape_cookie_file(std::io::Cursor::new(".\tFALSE\t/\tFALSE\t0\tname\tvalue"))
            .expect_err("cookie domain must survive Netscape import normalization");

    assert!(
        error.to_string().contains("cookie domain is empty"),
        "error={error:#}"
    );
}

#[test]
fn netscape_cookie_parser_rejects_relative_path() {
    let error = parse_netscape_cookie_file(std::io::Cursor::new(
        "example.com\tFALSE\trelative\tFALSE\t0\tname\tvalue",
    ))
    .expect_err("cookie path must be absolute in the StoredCookie model");

    assert!(
        error
            .to_string()
            .contains("cookie path must start with `/`"),
        "error={error:#}"
    );
}

#[test]
fn netscape_cookie_parser_rejects_empty_name() {
    let error = parse_netscape_cookie_file(std::io::Cursor::new(
        "example.com\tFALSE\t/\tFALSE\t0\t\tvalue",
    ))
    .expect_err("empty cookie names cannot enter the StoredCookie model");

    assert!(
        error.to_string().contains("cookie name is empty"),
        "error={error:#}"
    );
}

#[test]
fn netscape_cookie_parser_imports_session_cookie() -> Result<()> {
    let cookies = parse_netscape_cookie_file(std::io::Cursor::new(
        "example.com\tFALSE\t/\tFALSE\t0\ttoken\tvalue with spaces",
    ))?;

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].domain, "example.com");
    assert!(cookies[0].host_only);
    assert_eq!(cookies[0].name, "token");
    assert_eq!(cookies[0].value, "value with spaces");
    assert_eq!(cookies[0].expires, None);
    Ok(())
}

#[test]
fn netscape_cookie_parser_skips_expired_cookie() -> Result<()> {
    let cookies = parse_netscape_cookie_file(std::io::Cursor::new(
        "example.com\tFALSE\t/\tFALSE\t1\told\tgone\nexample.com\tFALSE\t/\tFALSE\t2147483647\tnew\tkept",
    ))?;

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "new");
    assert_eq!(cookies[0].creation_index, 0);
    Ok(())
}

#[test]
fn netscape_cookie_parser_rejects_non_utf8_cookie_value() {
    let error = parse_netscape_cookie_file(std::io::Cursor::new(
        b"example.com\tFALSE\t/\tFALSE\t0\tname\tvalue-\xff".to_vec(),
    ))
    .expect_err("non-UTF-8 cookie values cannot enter the string cookie store");

    assert!(
        error
            .to_string()
            .contains("cookie value is not valid UTF-8"),
        "error={error:#}"
    );
}

#[test]
fn netscape_cookie_parser_reports_line_number_for_malformed_record() {
    let error = parse_netscape_cookie_file(std::io::Cursor::new(
        "# Netscape HTTP Cookie File\nexample.com\tFALSE\n",
    ))
    .expect_err("malformed Netscape cookie records should report line context");

    assert!(error.to_string().contains("line 2"), "error={error:#}");
}

fn stored_cookie(name: &str, value: &str) -> StoredCookie {
    StoredCookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "example.com".to_owned(),
        host_only: true,
        path: "/".to_owned(),
        secure: false,
        http_only: false,
        expires: None,
        same_site: StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        source_scheme: StoredCookieSourceScheme::NonSecure,
        source_port: -1,
        creation_index: 0,
        last_access_index: 0,
    }
}
