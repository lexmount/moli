use super::*;
use moli_browser_profile::{
    BrowserProfilePaths, LEGACY_JSON_PROFILE_MANIFEST_VERSION, PROFILE_MANIFEST_VERSION,
};
use moli_cookie_jar::{StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme};
use moli_sqlite::{OpenMode, Runtime};

fn prepare(root: &Path) -> BrowserProfile {
    let profile = BrowserProfile::open(root).unwrap();
    profile.default_partition().ensure_directory().unwrap();
    let cookie = StoredCookie {
        name: "session".into(),
        value: "retained".into(),
        domain: "migration.test".into(),
        host_only: true,
        path: "/".into(),
        secure: true,
        http_only: false,
        expires: None,
        same_site: StoredCookieSameSite::Lax,
        priority: None,
        partition_key: None,
        source_scheme: StoredCookieSourceScheme::Secure,
        source_port: 443,
        creation_index: 0,
        last_access_index: 0,
    };
    moli_browser_profile::save_cookie_cache(&profile.paths().cookies_path, [cookie]).unwrap();
    let key = moli_web_storage::web_storage_partitioned_area_key(
        "https://migration.test",
        "https://migration.test",
    );
    let json = serde_json::json!({"version":1,"origins":{key:{"entries":[{"key":{"utf16":[55296,0]},"value":{"utf16":[56320,0,55296]}},{"key":"","value":""}]}}});
    fs::write(
        &profile.paths().local_storage_path,
        serde_json::to_vec(&json).unwrap(),
    )
    .unwrap();
    profile
}
fn manifest_version(paths: &BrowserProfilePaths) -> u64 {
    serde_json::from_slice::<serde_json::Value>(&fs::read(&paths.manifest_path).unwrap()).unwrap()["version"].as_u64().unwrap()
}
fn assert_data(paths: &BrowserProfilePaths, runtime: &Runtime) {
    let (cookies, owner) = moli_cookie_db::CookieDatabase::open(
        paths.cookies_sqlite_path.clone(),
        OpenMode::Existing,
        runtime.handle(),
    )
    .unwrap();
    let rows = cookies.lock().cookies();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "session");
    assert_eq!(rows[0].value, "retained");
    assert_eq!(rows[0].expires, None);
    owner.close().unwrap();
    let (store, owner) = moli_web_storage::new_sqlite_web_storage_store(
        paths.local_storage_sqlite_path.clone(),
        OpenMode::Existing,
        runtime.handle(),
    )
    .unwrap();
    let key = moli_web_storage::web_storage_partitioned_area_key(
        "https://migration.test",
        "https://migration.test",
    );
    assert_eq!(
        store.try_get_item_utf16(&key, &[55296, 0]).unwrap(),
        Some(vec![56320, 0, 55296])
    );
    assert_eq!(store.try_get_item_utf16(&key, &[]).unwrap(), Some(vec![]));
    owner.close().unwrap();
}
#[test]
fn publication_switches_the_manifest_only_after_both_databases_are_ready_and_preserves_json_backups()
 {
    let root = tempfile::tempdir().unwrap();
    let mut profile = prepare(root.path());
    let paths = profile.paths().clone();
    let cookie_backup = fs::read(&paths.cookies_path).unwrap();
    let local_backup = fs::read(&paths.local_storage_path).unwrap();
    let runtime = Runtime::new().unwrap();
    migrate(&mut profile, runtime.handle(), |boundary| {
        if boundary != Boundary::AfterManifest {
            assert_eq!(
                manifest_version(&paths),
                u64::from(LEGACY_JSON_PROFILE_MANIFEST_VERSION)
            );
        }
        if boundary == Boundary::BeforeManifest {
            assert!(paths.cookies_sqlite_path.exists());
            assert!(paths.local_storage_sqlite_path.exists());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(profile.manifest().version, PROFILE_MANIFEST_VERSION);
    assert_data(&paths, &runtime);
    assert_eq!(fs::read(&paths.cookies_path).unwrap(), cookie_backup);
    assert_eq!(fs::read(&paths.local_storage_path).unwrap(), local_backup);
    let before = fs::read(&paths.manifest_path).unwrap();
    migrate_profile_storage(&mut profile, runtime.handle()).unwrap();
    assert_eq!(fs::read(&paths.manifest_path).unwrap(), before);
}
#[test]
fn an_empty_profile_creates_empty_databases_without_creating_json_sources() {
    let root = tempfile::tempdir().unwrap();
    let mut profile = BrowserProfile::open(root.path()).unwrap();
    let paths = profile.paths().clone();
    let runtime = Runtime::new().unwrap();
    migrate_profile_storage(&mut profile, runtime.handle()).unwrap();
    assert_eq!(
        manifest_version(&paths),
        u64::from(PROFILE_MANIFEST_VERSION)
    );
    assert!(!paths.cookies_path.exists());
    assert!(!paths.local_storage_path.exists());
    let (cookies, owner) = moli_cookie_db::CookieDatabase::open(
        paths.cookies_sqlite_path,
        OpenMode::Existing,
        runtime.handle(),
    )
    .unwrap();
    assert!(cookies.lock().cookies().is_empty());
    owner.close().unwrap();
}
#[test]
fn malformed_json_does_not_publish_the_new_manifest_or_replace_the_source() {
    let root = tempfile::tempdir().unwrap();
    let mut profile = prepare(root.path());
    let paths = profile.paths().clone();
    fs::write(&paths.local_storage_path, b"{broken").unwrap();
    let runtime = Runtime::new().unwrap();
    assert!(migrate_profile_storage(&mut profile, runtime.handle()).is_err());
    assert_eq!(
        manifest_version(&paths),
        u64::from(LEGACY_JSON_PROFILE_MANIFEST_VERSION)
    );
    assert_eq!(fs::read(&paths.local_storage_path).unwrap(), b"{broken");
    assert!(!paths.local_storage_sqlite_path.exists());
}
#[test]
fn unidentified_existing_files_and_changed_sources_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let mut profile = prepare(root.path());
    let paths = profile.paths().clone();
    let runtime = Runtime::new().unwrap();
    fs::write(&paths.cookies_sqlite_path, b"unidentified user data").unwrap();
    assert!(migrate_profile_storage(&mut profile, runtime.handle()).is_err());
    assert_eq!(
        fs::read(&paths.cookies_sqlite_path).unwrap(),
        b"unidentified user data"
    );
    assert!(!paths.partition_root.join(STATE).exists());
    fs::remove_file(&paths.cookies_sqlite_path).unwrap();
    assert!(
        migrate(&mut profile, runtime.handle(), |boundary| {
            if boundary == Boundary::PublishedCookies {
                bail!("injected interruption")
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    let cookie_database = fs::read(&paths.cookies_sqlite_path).unwrap();
    fs::write(&paths.local_storage_path, b"{}").unwrap();
    assert!(migrate_profile_storage(&mut profile, runtime.handle()).is_err());
    assert_eq!(
        fs::read(&paths.cookies_sqlite_path).unwrap(),
        cookie_database
    );
    assert_eq!(
        manifest_version(&paths),
        u64::from(LEGACY_JSON_PROFILE_MANIFEST_VERSION)
    );
}
#[test]
fn a_foreign_database_with_the_same_source_fingerprint_has_the_wrong_migration_identity() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut source = prepare(first.path());
    let mut destination = prepare(second.path());
    let runtime = Runtime::new().unwrap();
    migrate_profile_storage(&mut source, runtime.handle()).unwrap();
    assert!(
        migrate(&mut destination, runtime.handle(), |boundary| {
            if boundary == Boundary::Owned {
                bail!("interrupt before database creation")
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    fs::copy(
        &source.paths().cookies_sqlite_path,
        &destination.paths().cookies_sqlite_path,
    )
    .unwrap();
    let bytes = fs::read(&destination.paths().cookies_sqlite_path).unwrap();
    assert!(migrate_profile_storage(&mut destination, runtime.handle()).is_err());
    assert_eq!(
        fs::read(&destination.paths().cookies_sqlite_path).unwrap(),
        bytes
    );
    assert_eq!(
        destination.manifest().version,
        LEGACY_JSON_PROFILE_MANIFEST_VERSION
    );
}
#[test]
fn migration_process_exit_child() {
    let Some(root) = std::env::var_os("MOLI_SQLITE_MIGRATION_TEST_PROFILE") else {
        return;
    };
    let at = std::env::var("MOLI_SQLITE_MIGRATION_TEST_BOUNDARY").unwrap();
    let mut profile = BrowserProfile::open(PathBuf::from(root)).unwrap();
    let runtime = Runtime::new().unwrap();
    migrate(&mut profile, runtime.handle(), |boundary| {
        if format!("{boundary:?}") == at {
            std::process::exit(73);
        }
        Ok(())
    })
    .unwrap();
    panic!("child did not reach its requested migration boundary");
}
#[test]
fn process_exit_after_each_publication_boundary_can_resume_without_mixing_backends() {
    for boundary in [
        Boundary::PublishedCookies,
        Boundary::PublishedLocalStorage,
        Boundary::BeforeManifest,
        Boundary::AfterManifest,
    ] {
        let root = tempfile::tempdir().unwrap();
        let profile = prepare(root.path());
        let paths = profile.paths().clone();
        let source_cookies = fs::read(&paths.cookies_path).unwrap();
        let source_local = fs::read(&paths.local_storage_path).unwrap();
        drop(profile);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::storage_partition::migration::tests::migration_process_exit_child",
                "--nocapture",
            ])
            .env("MOLI_SQLITE_MIGRATION_TEST_PROFILE", root.path())
            .env(
                "MOLI_SQLITE_MIGRATION_TEST_BOUNDARY",
                format!("{boundary:?}"),
            )
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "child {boundary:?}: {output:?}"
        );
        assert_eq!(
            manifest_version(&paths),
            u64::from(if boundary == Boundary::AfterManifest {
                PROFILE_MANIFEST_VERSION
            } else {
                LEGACY_JSON_PROFILE_MANIFEST_VERSION
            })
        );
        let mut profile = BrowserProfile::open(root.path()).unwrap();
        let runtime = Runtime::new().unwrap();
        migrate_profile_storage(&mut profile, runtime.handle()).unwrap();
        assert_data(&paths, &runtime);
        assert_eq!(fs::read(&paths.cookies_path).unwrap(), source_cookies);
        assert_eq!(fs::read(&paths.local_storage_path).unwrap(), source_local);
    }
}
