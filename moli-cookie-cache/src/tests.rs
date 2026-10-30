use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use moli_cookie_jar::{
    BrowserCookieStore, CookiePriority, CookieSource, NetworkCookieRequestContext, StoredCookie,
    StoredCookieExclusionReason, StoredCookiePartitionKey, StoredCookieSameSite,
    StoredCookieSourceScheme,
};
use time::OffsetDateTime;

use super::*;
use crate::{atomic_file::write_file_atomically, netscape::parse_netscape_cookie_file};

#[test]
fn cookie_cache_preserves_the_creation_time_used_by_default_same_site_policy() -> Result<()> {
    let temp = TempDir::new("same-site-age");
    let file = temp.path.join("cookies.json");
    let created =
        OffsetDateTime::from_unix_timestamp(1_000_000_000)? + time::Duration::nanoseconds(123);
    let mut cookie = stored_cookie("sid", "old");
    cookie.creation_time = Some(created);
    save_cookie_cache(&file, vec![cookie])?;
    let restored = load_cookie_cache(&file)?;
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].creation_time, Some(created));
    let mut store = BrowserCookieStore::default();
    let url = "http://example.com/".parse()?;
    assert!(
        store
            .upsert_with_request_url_report(restored[0].clone(), Some(&url), CookieSource::Cdp)
            .is_accepted()
    );
    let initiator = "http://other.test/".parse()?;
    let context = NetworkCookieRequestContext::top_level_navigation("POST")
        .with_initiator_url(&url, &initiator);
    let report = store.cookie_access_report_for_request(&url, context);
    assert!(report.included_cookies.is_empty());
    assert_eq!(report.excluded_cookies.len(), 1);
    assert_eq!(
        report.excluded_cookies[0].cookie.creation_time,
        Some(created)
    );
    assert_eq!(
        report.excluded_cookies[0].exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteUnspecifiedTreatedAsLax]
    );
    Ok(())
}

struct TempDir {
    path: PathBuf,
}

#[test]
fn legacy_cookie_cache_does_not_reset_the_default_cookie_age() -> Result<()> {
    let temp = TempDir::new("legacy-same-site-age");
    let file = temp.path.join("cookies.json");
    fs::create_dir_all(&temp.path)?;
    fs::write(
        &file,
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "cookies": [{
                "name": "sid", "value": "legacy", "domain": "example.com",
                "host_only": true, "path": "/", "secure": false, "http_only": false,
                "expires_unix": null, "same_site": "unspecified", "priority": null,
                "source_scheme": "nonsecure", "source_port": -1
            }]
        }))?,
    )?;
    let restored = load_cookie_cache(&file)?;
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].creation_time, None);
    let mut store = BrowserCookieStore::default();
    let url = "http://example.com/".parse()?;
    assert!(
        store
            .upsert_with_request_url_report(restored[0].clone(), Some(&url), CookieSource::Cdp)
            .is_accepted()
    );
    let initiator = "http://other.test/".parse()?;
    let context = NetworkCookieRequestContext::top_level_navigation("POST")
        .with_initiator_url(&url, &initiator);
    let report = store.cookie_access_report_for_request(&url, context);
    assert!(report.included_cookies.is_empty());
    assert_eq!(report.excluded_cookies.len(), 1);
    assert_eq!(report.excluded_cookies[0].cookie.creation_time, None);
    assert_eq!(
        report.excluded_cookies[0].exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteUnspecifiedTreatedAsLax]
    );
    Ok(())
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

#[cfg(unix)]
#[test]
fn atomic_write_respects_the_existing_targets_write_permissions() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new("readonly");
    fs::create_dir_all(&temp.path)?;
    let target = temp.path.join("cookies.txt");
    fs::write(&target, b"protected content")?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o444))?;
    // Privileged users can write 0444 files. Match the kernel's open result
    // rather than assuming the test process lacks that capability.
    let writable = fs::OpenOptions::new().write(true).open(&target).is_ok();

    let result = write_file_atomically(&target, b"replacement", "readonly test");

    assert_eq!(result.is_ok(), writable, "result={result:?}");
    assert_eq!(
        fs::read(&target)?,
        if writable {
            b"replacement".as_slice()
        } else {
            b"protected content".as_slice()
        }
    );
    assert_eq!(fs::read_dir(&temp.path)?.count(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn atomic_write_keeps_regular_file_contents_when_temp_creation_fails() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new("readonly-directory");
    fs::create_dir_all(&temp.path)?;
    let target = temp.path.join("cookies.txt");
    let probe = temp.path.join("permission-probe");
    fs::write(&target, b"original content")?;
    fs::set_permissions(&temp.path, fs::Permissions::from_mode(0o555))?;
    let can_create = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok();

    let result = write_file_atomically(&target, b"replacement", "atomic failure test");
    fs::set_permissions(&temp.path, fs::Permissions::from_mode(0o755))?;

    assert_eq!(result.is_ok(), can_create, "result={result:?}");
    assert_eq!(
        fs::read(&target)?,
        if can_create {
            b"replacement".as_slice()
        } else {
            b"original content".as_slice()
        }
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn atomic_write_writes_to_a_device_without_replacing_its_symlink() -> Result<()> {
    use std::os::unix::fs::{FileTypeExt, symlink};

    let temp = TempDir::new("device");
    fs::create_dir_all(&temp.path)?;
    let target = temp.path.join("null-device");
    symlink("/dev/null", &target)?;

    write_file_atomically(&target, b"cookie data", "device test")?;

    assert!(fs::symlink_metadata(&target)?.file_type().is_symlink());
    assert!(fs::metadata(&target)?.file_type().is_char_device());
    assert_eq!(fs::read_dir(&temp.path)?.count(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn atomic_write_delivers_bytes_to_a_fifo_without_replacing_it() -> Result<()> {
    use anyhow::Context;
    use std::{
        io::Read,
        os::unix::fs::FileTypeExt,
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };

    let temp = TempDir::new("fifo");
    fs::create_dir_all(&temp.path)?;
    let target = temp.path.join("cookie-pipe");
    assert!(Command::new("mkfifo").arg(&target).status()?.success());
    let mut reader = Command::new("cat")
        .arg(&target)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdout = reader.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let read_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.read_to_end(&mut bytes).map(|_| bytes);
        let _ = tx.send(result);
    });

    // Opening the writer pairs with cat's reader; closing it delivers EOF.
    // Bound the completion wait so replacing the FIFO fails instead of hanging.
    let written = write_file_atomically(&target, b"cookie data", "fifo test");
    let received = rx.recv_timeout(Duration::from_secs(5));
    if received.is_err() {
        let _ = reader.kill();
    }
    let status = reader.wait()?;
    read_thread.join().expect("pipe reader should not panic");
    written?;
    assert!(status.success(), "reader status={status}");
    assert_eq!(
        received.context("FIFO reader did not complete")??,
        b"cookie data"
    );
    assert!(fs::metadata(&target)?.file_type().is_fifo());
    assert_eq!(fs::read_dir(&temp.path)?.count(), 1);
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
    assert_eq!(cookies[0].source_scheme, StoredCookieSourceScheme::Secure);
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
    assert!(!cookies[0].secure);
    assert_eq!(cookies[0].source_scheme, StoredCookieSourceScheme::Unset);
    assert_eq!(cookies[0].name, "token");
    assert_eq!(cookies[0].value, "value with spaces");
    assert_eq!(cookies[0].expires, None);
    Ok(())
}

#[test]
fn netscape_cookie_import_sends_non_secure_cookie_over_http_and_https() -> Result<()> {
    let cookies = parse_netscape_cookie_file(std::io::Cursor::new(
        "# Netscape HTTP Cookie File\nexample.com\tTRUE\t/\tFALSE\t0\ttest_cookie\thello_val\n",
    ))?;
    let mut store = BrowserCookieStore::default();
    for cookie in cookies {
        let report = store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp);
        assert!(report.is_accepted(), "report={report:?}");
    }

    for raw_url in ["http://example.com/cookies", "https://example.com/cookies"] {
        let url = raw_url.parse()?;
        let report = store.cookie_access_report_for_request(
            &url,
            NetworkCookieRequestContext::subresource("GET"),
        );

        assert_eq!(
            report.included_cookies.len(),
            1,
            "url={url}, report={report:?}"
        );
        assert!(
            report.excluded_cookies.is_empty(),
            "url={url}, report={report:?}"
        );
        assert_eq!(report.included_cookies[0].cookie.name, "test_cookie");
        assert_eq!(report.included_cookies[0].cookie.value, "hello_val");
    }
    Ok(())
}

#[test]
fn netscape_cookie_import_sends_secure_cookie_only_over_https() -> Result<()> {
    let cookies = parse_netscape_cookie_file(std::io::Cursor::new(
        "example.com\tFALSE\t/\tTRUE\t0\ttest_cookie\thello_val\n",
    ))?;
    let mut store = BrowserCookieStore::default();
    for cookie in cookies {
        let report = store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp);
        assert!(report.is_accepted(), "report={report:?}");
    }

    let https_url = "https://example.com/cookies".parse()?;
    let https_report = store.cookie_access_report_for_request(
        &https_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    assert_eq!(https_report.included_cookies.len(), 1);
    assert!(https_report.excluded_cookies.is_empty());
    assert_eq!(https_report.included_cookies[0].cookie.name, "test_cookie");
    assert_eq!(https_report.included_cookies[0].cookie.value, "hello_val");

    let http_url = "http://example.com/cookies".parse()?;
    let http_report = store.cookie_access_report_for_request(
        &http_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    assert!(http_report.included_cookies.is_empty());
    assert_eq!(http_report.excluded_cookies.len(), 1);
    assert_eq!(http_report.excluded_cookies[0].cookie.name, "test_cookie");
    assert!(
        http_report.excluded_cookies[0]
            .exclusion_reasons
            .contains(&StoredCookieExclusionReason::SecureOnly)
    );
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

#[test]
fn netscape_cookie_lossy_reader_skips_invalid_records_and_preserves_order() -> Result<()> {
    let input = b"# Netscape HTTP Cookie File\r\n\
example.com\tFALSE\t/\tFALSE\t0\tfirst\tone\r\n\
malformed record\n\
.\tFALSE\t/\tFALSE\t0\tempty-domain\tinvalid\n\
example.com\tFALSE\trelative\tFALSE\t0\tbad-path\tinvalid\n\
example.com\tFALSE\t/\tFALSE\t0\tbad-utf8\t\xff\n\
example.com\tFALSE\t/\tFALSE\t18446744073709551615\tbad-expiry\tinvalid\n\
example.com\tFALSE\t/\tFALSE\t1\texpired\tgone\n\
#HttpOnly_.example.com\tTRUE\t/account/\tTRUE\t2147483647\tsecond\ttwo\n";

    assert!(read_cookie_file(std::io::Cursor::new(input)).is_err());
    let cookies = read_cookie_file_lossy(std::io::Cursor::new(input))?;
    assert_eq!(cookies.len(), 2);
    assert_eq!(cookies[0].name, "first");
    assert_eq!(cookies[0].value, "one");
    assert_eq!(cookies[0].creation_index, 0);
    assert_eq!(cookies[1].name, "second");
    assert_eq!(cookies[1].value, "two");
    assert_eq!(cookies[1].creation_index, 1);
    assert_eq!(cookies[1].domain, "example.com");
    assert_eq!(cookies[1].path, "/account/");
    assert!(!cookies[1].host_only);
    assert!(cookies[1].http_only);
    assert!(cookies[1].secure);
    Ok(())
}

#[test]
fn cookie_file_lossy_readers_propagate_io_errors() {
    struct FailedCookieInput;
    impl std::io::Read for FailedCookieInput {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("cookie fixture read failure"))
        }
    }

    for error in [
        read_cookie_file_lossy(std::io::BufReader::new(FailedCookieInput)),
        read_cookie_input_lossy(std::io::BufReader::new(FailedCookieInput)),
    ] {
        let error = error.expect_err("I/O failures must not be treated as malformed records");
        assert!(error.root_cause().is::<std::io::Error>());
        assert_eq!(
            error.root_cause().to_string(),
            "cookie fixture read failure"
        );
    }
}

#[test]
fn cookie_input_reader_accepts_mixed_http_headers_and_netscape_records() -> Result<()> {
    let input = b"HTTP/1.1 200 OK\r\n\
Set-Cookie: session=abc; Domain=.Example.Test; Path=/account; Secure; HttpOnly; Expires=Tue, 19 Jan 2038 03:14:07 GMT\r\n\
malformed record\n\
example.test\tTRUE\t/\tFALSE\t0\tstored\tjar\n\
sEt-CoOkIe: expired=gone; Domain=example.test; Path=/; Max-Age=0\n";
    let cookies = read_cookie_input_lossy(std::io::Cursor::new(input))?;
    assert_eq!(cookies.len(), 2);
    let header = &cookies[0];
    assert_eq!(header.name, "session");
    assert_eq!(header.value, "abc");
    assert_eq!(header.domain, "example.test");
    assert_eq!(header.path, "/account");
    assert!(!header.host_only);
    assert!(header.secure);
    assert!(header.http_only);
    assert_eq!(header.expires.unwrap().unix_timestamp(), 2147483647);
    assert_eq!(header.same_site, StoredCookieSameSite::Unspecified);
    assert_eq!(header.creation_index, 0);
    assert_eq!(cookies[1].name, "stored");
    assert_eq!(cookies[1].value, "jar");
    assert_eq!(cookies[1].creation_index, 1);
    Ok(())
}

#[test]
fn cookie_input_http_headers_default_the_path_and_honor_max_age() -> Result<()> {
    let input = "Set-Cookie: default=one; Domain=example.test\n\
Set-Cookie: relative=two; Domain=example.test; Path=relative\n\
Set-Cookie: expired=gone; Domain=example.test; Max-Age=-1\n\
Set-Cookie: recent=three; Domain=example.test; Max-Age=3600; Expires=Thu, 01 Jan 1970 00:00:01 GMT\n";
    let cookies = read_cookie_input_lossy(std::io::Cursor::new(input))?;
    assert_eq!(cookies.len(), 3);
    assert!(cookies.iter().all(|cookie| cookie.path == "/"));
    assert_eq!(cookies[0].name, "default");
    assert_eq!(cookies[1].name, "relative");
    assert_eq!(cookies[2].name, "recent");
    assert!(cookies[2].expires.unwrap() > OffsetDateTime::now_utc());
    Ok(())
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
        creation_time: None,
        last_access_index: 0,
    }
}
