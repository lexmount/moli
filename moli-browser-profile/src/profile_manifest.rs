use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{BrowserProfilePaths, DEFAULT_PROFILE_PARTITION_ID, write_profile_file};

pub const PROFILE_MANIFEST_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfileManifest {
    pub version: u32,
    pub partitions: Vec<BrowserProfilePartitionManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfilePartitionManifest {
    pub id: String,
    pub root: String,
    pub cookies: ProfileBackendManifest,
    pub local_storage: ProfileBackendManifest,
    pub session_storage: ProfileBackendManifest,
    pub storage_buckets: ProfileBackendManifest,
    pub service_worker_resources: ProfileBackendManifest,
    pub cache_storage: ProfileBackendManifest,
    pub opfs: ProfileBackendManifest,
    pub indexed_db: ProfileBackendManifest,
    pub http_cache: ProfileBackendManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileBackendManifest {
    pub backend: ProfileBackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileBackendKind {
    Disk,
    Json,
    Memory,
}

impl BrowserProfileManifest {
    pub fn default_for_paths(paths: &BrowserProfilePaths) -> Self {
        let partition_root = relative_profile_path(&paths.root, &paths.partition_root);
        Self {
            version: PROFILE_MANIFEST_VERSION,
            partitions: vec![BrowserProfilePartitionManifest {
                id: DEFAULT_PROFILE_PARTITION_ID.to_owned(),
                root: partition_root,
                cookies: json_backend(&paths.root, &paths.cookies_path),
                local_storage: json_backend(&paths.root, &paths.local_storage_path),
                session_storage: ProfileBackendManifest {
                    backend: ProfileBackendKind::Memory,
                    path: None,
                },
                storage_buckets: json_backend(&paths.root, &paths.storage_buckets_path),
                service_worker_resources: json_backend(
                    &paths.root,
                    &paths.service_worker_resources_path,
                ),
                cache_storage: disk_backend(&paths.root, &paths.cache_storage_root),
                opfs: disk_backend(&paths.root, &paths.opfs_root),
                indexed_db: disk_backend(&paths.root, &paths.indexeddb_root),
                http_cache: disk_backend(&paths.root, &paths.http_cache_root),
            }],
        }
    }
}

pub fn ensure_profile_manifest(paths: &BrowserProfilePaths) -> Result<BrowserProfileManifest> {
    if paths.manifest_path.exists() {
        return load_profile_manifest(paths);
    }

    if !paths.root.as_os_str().is_empty() {
        std::fs::create_dir_all(&paths.root)
            .with_context(|| format!("failed to create profile dir `{}`", paths.root.display()))?;
    }

    let manifest = BrowserProfileManifest::default_for_paths(paths);
    save_profile_manifest(paths, &manifest)?;
    Ok(manifest)
}

pub fn load_profile_manifest(paths: &BrowserProfilePaths) -> Result<BrowserProfileManifest> {
    let bytes = std::fs::read(&paths.manifest_path).with_context(|| {
        format!(
            "failed to read profile manifest `{}`",
            paths.manifest_path.display()
        )
    })?;
    let value: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "failed to parse profile manifest `{}`",
            paths.manifest_path.display()
        )
    })?;
    let version = manifest_version_from_value(paths, &value)?;
    if version < PROFILE_MANIFEST_VERSION {
        let manifest = migrate_profile_manifest(paths, version, value)?;
        validate_profile_manifest(paths, &manifest)?;
        save_profile_manifest(paths, &manifest)?;
        return Ok(manifest);
    }
    if version > PROFILE_MANIFEST_VERSION {
        bail!(
            "unsupported profile manifest version {} in `{}`; this Moli supports version {}",
            version,
            paths.manifest_path.display(),
            PROFILE_MANIFEST_VERSION
        );
    }
    let manifest: BrowserProfileManifest = serde_json::from_value(value).with_context(|| {
        format!(
            "failed to parse profile manifest `{}`",
            paths.manifest_path.display()
        )
    })?;
    validate_profile_manifest(paths, &manifest)?;
    Ok(manifest)
}

fn manifest_version_from_value(paths: &BrowserProfilePaths, value: &Value) -> Result<u32> {
    let Some(version) = value.get("version").and_then(Value::as_u64) else {
        bail!(
            "profile manifest `{}` does not define a numeric version",
            paths.manifest_path.display()
        );
    };
    u32::try_from(version).with_context(|| {
        format!(
            "unsupported profile manifest version {} in `{}`",
            version,
            paths.manifest_path.display()
        )
    })
}

fn migrate_profile_manifest(
    paths: &BrowserProfilePaths,
    version: u32,
    _value: Value,
) -> Result<BrowserProfileManifest> {
    match version {
        0..=4 => Ok(BrowserProfileManifest::default_for_paths(paths)),
        _ => bail!(
            "unsupported profile manifest version {} in `{}`; this Moli supports version {}",
            version,
            paths.manifest_path.display(),
            PROFILE_MANIFEST_VERSION
        ),
    }
}

fn save_profile_manifest(
    paths: &BrowserProfilePaths,
    manifest: &BrowserProfileManifest,
) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(manifest).context("failed to serialize profile manifest")?;
    write_profile_file(&paths.manifest_path, &bytes, "profile manifest")
}

fn validate_profile_manifest(
    paths: &BrowserProfilePaths,
    manifest: &BrowserProfileManifest,
) -> Result<()> {
    let Some(default_partition) = manifest
        .partitions
        .iter()
        .find(|partition| partition.id == DEFAULT_PROFILE_PARTITION_ID)
    else {
        bail!(
            "profile manifest `{}` does not define the default storage partition",
            paths.manifest_path.display()
        );
    };

    if manifest.partitions.len() != 1 {
        bail!(
            "profile manifest `{}` defines unsupported non-default storage partitions; this Moli currently supports only the default storage partition",
            paths.manifest_path.display()
        );
    }

    let expected = BrowserProfileManifest::default_for_paths(paths);
    let expected_default = &expected.partitions[0];
    if default_partition != expected_default {
        bail!(
            "profile manifest `{}` default partition does not match the current profile layout",
            paths.manifest_path.display()
        );
    }
    Ok(())
}

fn json_backend(root: &Path, path: &Path) -> ProfileBackendManifest {
    ProfileBackendManifest {
        backend: ProfileBackendKind::Json,
        path: Some(relative_profile_path(root, path)),
    }
}

fn disk_backend(root: &Path, path: &Path) -> ProfileBackendManifest {
    ProfileBackendManifest {
        backend: ProfileBackendKind::Disk,
        path: Some(relative_profile_path(root, path)),
    }
}

fn relative_profile_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;
    use serde_json::json;

    use super::{BrowserProfileManifest, PROFILE_MANIFEST_VERSION, ensure_profile_manifest};
    use crate::BrowserProfilePaths;

    struct TempProfileDir {
        path: PathBuf,
    }

    impl TempProfileDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-profile-manifest-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { path }
        }
    }

    impl Drop for TempProfileDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn ensure_profile_manifest_creates_root_manifest() -> Result<()> {
        let profile = TempProfileDir::new("create");
        let paths = BrowserProfilePaths::new(&profile.path);

        let manifest = ensure_profile_manifest(&paths)?;

        assert_eq!(manifest, BrowserProfileManifest::default_for_paths(&paths));
        let persisted = fs::read_to_string(&paths.manifest_path)?;
        assert!(persisted.contains("\"version\""));
        assert!(persisted.contains("\"default\""));
        assert!(persisted.contains("\"localStorage\""));
        assert!(persisted.contains("\"sessionStorage\""));
        assert!(persisted.contains("\"storageBuckets\""));
        assert!(persisted.contains("\"serviceWorkerResources\""));
        assert!(persisted.contains("\"cacheStorage\""));
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_rejects_unknown_version() -> Result<()> {
        let profile = TempProfileDir::new("future");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json!({
                "version": PROFILE_MANIFEST_VERSION + 1,
                "partitions": []
            }))?,
        )?;

        let error = ensure_profile_manifest(&paths).expect_err("future version should fail");

        assert!(
            error
                .to_string()
                .contains("unsupported profile manifest version"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_rejects_missing_numeric_version() -> Result<()> {
        let profile = TempProfileDir::new("missing-version");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json!({
                "partitions": []
            }))?,
        )?;

        let error = ensure_profile_manifest(&paths).expect_err("missing version should fail");

        assert!(
            error
                .to_string()
                .contains("does not define a numeric version"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_rejects_unsupported_non_default_partitions() -> Result<()> {
        let profile = TempProfileDir::new("extra-partition");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        let mut manifest = BrowserProfileManifest::default_for_paths(&paths);
        let mut extra_partition = manifest.partitions[0].clone();
        extra_partition.id = "secondary".to_owned();
        extra_partition.root = "partitions/secondary".to_owned();
        manifest.partitions.push(extra_partition);
        fs::write(&paths.manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

        let error =
            ensure_profile_manifest(&paths).expect_err("extra non-default partition should fail");

        assert!(
            error
                .to_string()
                .contains("unsupported non-default storage partitions"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_migrates_version_zero() -> Result<()> {
        let profile = TempProfileDir::new("migrate-v0");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json!({
                "version": 0,
                "partitions": [{
                    "id": "default",
                    "root": "partitions/default"
                }]
            }))?,
        )?;

        let manifest = ensure_profile_manifest(&paths)?;

        assert_eq!(manifest, BrowserProfileManifest::default_for_paths(&paths));
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.manifest_path)?)?;
        assert_eq!(
            persisted["version"],
            serde_json::json!(PROFILE_MANIFEST_VERSION)
        );
        assert_eq!(
            persisted["partitions"][0]["sessionStorage"]["backend"],
            serde_json::json!("memory")
        );
        assert_eq!(
            persisted["partitions"][0]["storageBuckets"]["path"],
            serde_json::json!("partitions/default/storage-buckets.json")
        );
        assert_eq!(
            persisted["partitions"][0]["serviceWorkerResources"]["path"],
            serde_json::json!("partitions/default/service-worker-resources.json")
        );
        assert_eq!(
            persisted["partitions"][0]["cacheStorage"]["path"],
            serde_json::json!("partitions/default/cache-storage")
        );
        assert_eq!(
            persisted["partitions"][0]["opfs"]["path"],
            serde_json::json!("partitions/default/opfs")
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_migrates_version_one() -> Result<()> {
        let profile = TempProfileDir::new("migrate-v1");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json!({
                "version": 1,
                "partitions": [{
                    "id": "default",
                    "root": "partitions/default",
                    "cookies": {
                        "backend": "json",
                        "path": "partitions/default/cookies.json"
                    },
                    "localStorage": {
                        "backend": "json",
                        "path": "partitions/default/localstorage.json"
                    },
                    "sessionStorage": {
                        "backend": "memory"
                    },
                    "indexedDb": {
                        "backend": "disk",
                        "path": "partitions/default/indexeddb"
                    },
                    "httpCache": {
                        "backend": "disk",
                        "path": "partitions/default/http-cache"
                    }
                }]
            }))?,
        )?;

        let manifest = ensure_profile_manifest(&paths)?;

        assert_eq!(manifest, BrowserProfileManifest::default_for_paths(&paths));
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.manifest_path)?)?;
        assert_eq!(
            persisted["version"],
            serde_json::json!(PROFILE_MANIFEST_VERSION)
        );
        assert_eq!(
            persisted["partitions"][0]["storageBuckets"]["path"],
            serde_json::json!("partitions/default/storage-buckets.json")
        );
        assert_eq!(
            persisted["partitions"][0]["serviceWorkerResources"]["path"],
            serde_json::json!("partitions/default/service-worker-resources.json")
        );
        assert_eq!(
            persisted["partitions"][0]["cacheStorage"]["path"],
            serde_json::json!("partitions/default/cache-storage")
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_migrates_version_two() -> Result<()> {
        let profile = TempProfileDir::new("migrate-v2");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json!({
                "version": 2,
                "partitions": [{
                    "id": "default",
                    "root": "partitions/default",
                    "cookies": {
                        "backend": "json",
                        "path": "partitions/default/cookies.json"
                    },
                    "localStorage": {
                        "backend": "json",
                        "path": "partitions/default/localstorage.json"
                    },
                    "sessionStorage": {
                        "backend": "memory"
                    },
                    "storageBuckets": {
                        "backend": "json",
                        "path": "partitions/default/storage-buckets.json"
                    },
                    "indexedDb": {
                        "backend": "disk",
                        "path": "partitions/default/indexeddb"
                    },
                    "httpCache": {
                        "backend": "disk",
                        "path": "partitions/default/http-cache"
                    }
                }]
            }))?,
        )?;

        let manifest = ensure_profile_manifest(&paths)?;

        assert_eq!(manifest, BrowserProfileManifest::default_for_paths(&paths));
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.manifest_path)?)?;
        assert_eq!(
            persisted["version"],
            serde_json::json!(PROFILE_MANIFEST_VERSION)
        );
        assert_eq!(
            persisted["partitions"][0]["cacheStorage"]["path"],
            serde_json::json!("partitions/default/cache-storage")
        );
        assert_eq!(
            persisted["partitions"][0]["serviceWorkerResources"]["path"],
            serde_json::json!("partitions/default/service-worker-resources.json")
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_migrates_version_three() -> Result<()> {
        let profile = TempProfileDir::new("migrate-v3");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json!({
                "version": 3,
                "partitions": [{
                    "id": "default",
                    "root": "partitions/default",
                    "cookies": {
                        "backend": "json",
                        "path": "partitions/default/cookies.json"
                    },
                    "localStorage": {
                        "backend": "json",
                        "path": "partitions/default/localstorage.json"
                    },
                    "sessionStorage": {
                        "backend": "memory"
                    },
                    "storageBuckets": {
                        "backend": "json",
                        "path": "partitions/default/storage-buckets.json"
                    },
                    "cacheStorage": {
                        "backend": "disk",
                        "path": "partitions/default/cache-storage"
                    },
                    "indexedDb": {
                        "backend": "disk",
                        "path": "partitions/default/indexeddb"
                    },
                    "httpCache": {
                        "backend": "disk",
                        "path": "partitions/default/http-cache"
                    }
                }]
            }))?,
        )?;

        let manifest = ensure_profile_manifest(&paths)?;

        assert_eq!(manifest, BrowserProfileManifest::default_for_paths(&paths));
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.manifest_path)?)?;
        assert_eq!(
            persisted["version"],
            serde_json::json!(PROFILE_MANIFEST_VERSION)
        );
        assert_eq!(
            persisted["partitions"][0]["serviceWorkerResources"]["path"],
            serde_json::json!("partitions/default/service-worker-resources.json")
        );
        Ok(())
    }

    #[test]
    fn ensure_profile_manifest_migrates_version_four_with_opfs_backend() -> Result<()> {
        let profile = TempProfileDir::new("migrate-v4");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&paths.root)?;
        let mut previous = serde_json::to_value(BrowserProfileManifest::default_for_paths(&paths))?;
        previous["version"] = serde_json::json!(4);
        previous["partitions"][0]
            .as_object_mut()
            .expect("partition manifest should be an object")
            .remove("opfs");
        fs::write(&paths.manifest_path, serde_json::to_vec_pretty(&previous)?)?;

        let manifest = ensure_profile_manifest(&paths)?;

        assert_eq!(manifest, BrowserProfileManifest::default_for_paths(&paths));
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.manifest_path)?)?;
        assert_eq!(
            persisted["version"],
            serde_json::json!(PROFILE_MANIFEST_VERSION)
        );
        assert_eq!(
            persisted["partitions"][0]["opfs"]["backend"],
            serde_json::json!("disk")
        );
        assert_eq!(
            persisted["partitions"][0]["opfs"]["path"],
            serde_json::json!("partitions/default/opfs")
        );
        Ok(())
    }
}
