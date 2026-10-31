use super::*;
use moli_browser_profile::{BrowserProfileManifest, ProfileBackendKind, ProfileBackendManifest};

#[test]
fn cookie_and_local_storage_open_independently_on_first_data_access() -> Result<()> {
    for cookie_first in [true, false] {
        let profile = TempProfileDir::new(if cookie_first {
            "cookie-first"
        } else {
            "local-first"
        });
        let partition = StoragePartitionState::open(Some(&profile.path))?;
        let handles = partition.shared_storage_handles();
        let cookies = handles.cookie_store();
        let local = handles.web_storage_store();
        let cookie_path = partition.cookie_database_path();
        let local_path = partition.local_storage_database_path();
        assert!(!cookie_path.exists());
        assert!(!local_path.exists());
        partition.flush()?;
        assert!(!cookie_path.exists());
        assert!(!local_path.exists());
        if cookie_first {
            assert!(cookies.lock()?.cookies().is_empty());
            assert!(cookie_path.is_file());
            assert!(!local_path.exists());
        } else {
            assert_eq!(local.try_len("https://first.test")?, 0);
            assert!(local_path.is_file());
            assert!(!cookie_path.exists());
        }
        partition.import_cookies([stored_cookie("saved", "cookie")])?;
        local.try_set_item("https://first.test", "saved", "local")?;
        partition.shutdown()?;
        drop(partition);
        let reopened = StoragePartitionState::open(Some(&profile.path))?;
        assert_eq!(reopened.cookies()?[0].name, "saved");
        assert_eq!(
            reopened
                .web_storage_store()
                .get_item("https://first.test", "saved")
                .as_deref(),
            Some("local")
        );
    }
    Ok(())
}

#[test]
fn profile_switches_to_sqlite_without_importing_old_json() -> Result<()> {
    let profile = TempProfileDir::new("ignore-old-json");
    let paths = BrowserProfilePaths::new(&profile.path);
    fs::create_dir_all(&paths.partition_root)?;
    let legacy_cookie = paths.partition_root.join("cookies.json");
    let legacy_local = paths.partition_root.join("localstorage.json");
    moli_browser_profile::save_cookie_cache(&legacy_cookie, [stored_cookie("old", "cookie")])?;
    let origin = "https://example.com";
    let key = moli_web_storage::web_storage_partitioned_area_key(origin, origin);
    let local_bytes = serde_json::to_vec(&serde_json::json!({
        "version": 1, "origins": {&key: {"entries": [{"key": "old", "value": "local"}]}}
    }))?;
    fs::write(&legacy_local, &local_bytes)?;
    let cookie_bytes = fs::read(&legacy_cookie)?;
    let mut manifest = BrowserProfileManifest::default_for_paths(&paths);
    manifest.version = 6;
    let partition = &mut manifest.partitions[0];
    for (backend, name) in [
        (&mut partition.cookies, "cookies.json"),
        (&mut partition.local_storage, "localstorage.json"),
    ] {
        *backend = ProfileBackendManifest {
            backend: ProfileBackendKind::Json,
            path: Some(format!("partitions/default/{name}")),
        };
    }
    fs::write(&paths.manifest_path, serde_json::to_vec(&manifest)?)?;

    {
        let partition = StoragePartitionState::open(Some(&profile.path))?;
        assert!(partition.cookies()?.is_empty());
        let local = partition.web_storage_store();
        assert_eq!(
            local.try_get_item_utf16(&key, &"old".encode_utf16().collect::<Vec<_>>())?,
            None
        );
        partition.import_cookies([stored_cookie("new", "cookie")])?;
        local.try_set_item(&key, "new", "local")?;
        partition.flush()?;
        assert_eq!(partition.cookie_database_path(), paths.cookies_path);
        assert_eq!(
            partition.local_storage_database_path(),
            paths.local_storage_path
        );
    }
    let reopened = StoragePartitionState::open(Some(&profile.path))?;
    let cookies = reopened.cookies()?;
    assert_eq!(cookies.len(), 1);
    assert_eq!(
        (cookies[0].name.as_str(), cookies[0].value.as_str()),
        ("new", "cookie")
    );
    let local = reopened.web_storage_store();
    assert_eq!(
        local.try_get_item_utf16(&key, &"new".encode_utf16().collect::<Vec<_>>())?,
        Some("local".encode_utf16().collect())
    );
    assert_eq!(
        local.try_get_item_utf16(&key, &"old".encode_utf16().collect::<Vec<_>>())?,
        None
    );
    assert_eq!(fs::read(&legacy_cookie)?, cookie_bytes);
    assert_eq!(fs::read(&legacy_local)?, local_bytes);
    Ok(())
}

#[test]
fn invalid_old_storage_files_do_not_block_new_databases() -> Result<()> {
    let profile = TempProfileDir::new("invalid-old-files");
    let paths = BrowserProfilePaths::new(&profile.path);
    fs::create_dir_all(paths.partition_root.join("localstorage.json"))?;
    let ignored = ["cookies.json", ".storage-sqlite-v7.json"];
    for name in ignored {
        fs::write(paths.partition_root.join(name), b"not valid JSON or SQLite")?;
    }
    let partition = StoragePartitionState::open(Some(&profile.path))?;
    assert!(partition.cookies()?.is_empty());
    assert_eq!(
        partition.web_storage_store().try_get_item_utf16(
            "https://example.com",
            &"key".encode_utf16().collect::<Vec<_>>()
        )?,
        None
    );
    partition.flush()?;
    for name in ignored {
        assert_eq!(
            fs::read(paths.partition_root.join(name))?,
            b"not valid JSON or SQLite"
        );
    }
    assert!(paths.partition_root.join("localstorage.json").is_dir());
    for path in [&paths.cookies_path, &paths.local_storage_path] {
        assert!(fs::read(path)?.starts_with(b"SQLite format 3\0"));
    }
    Ok(())
}

#[test]
fn corrupt_current_databases_fail_without_falling_back_to_old_data() -> Result<()> {
    for cookie in [true, false] {
        let profile = TempProfileDir::new(if cookie {
            "corrupt-cookie"
        } else {
            "corrupt-local"
        });
        let paths = BrowserProfilePaths::new(&profile.path);
        drop(StoragePartitionState::open(Some(&profile.path))?);
        let path = if cookie {
            &paths.cookies_path
        } else {
            &paths.local_storage_path
        };
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, b"corrupt current database")?;
        let partition = StoragePartitionState::open(Some(&profile.path))?;
        if cookie {
            assert!(partition.cookies().is_err());
            assert!(!paths.local_storage_path.exists());
        } else {
            assert!(
                partition
                    .web_storage_store()
                    .try_len("https://failure.test")
                    .is_err()
            );
            assert!(!paths.cookies_path.exists());
        }
        assert!(partition.shutdown().is_err());
        assert_eq!(fs::read(path)?, b"corrupt current database");
    }
    Ok(())
}
