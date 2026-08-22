use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::{
    BrowserProfileLock, BrowserProfileManifest, BrowserProfilePaths, DEFAULT_PROFILE_PARTITION_ID,
    ensure_profile_manifest,
};

#[derive(Debug)]
pub struct BrowserProfile {
    paths: BrowserProfilePaths,
    manifest: BrowserProfileManifest,
    _lock: BrowserProfileLock,
}

impl BrowserProfile {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_paths(BrowserProfilePaths::new(root))
    }

    pub fn open_paths(paths: BrowserProfilePaths) -> Result<Self> {
        let profile_lock = BrowserProfileLock::acquire(&paths)?;
        let manifest = ensure_profile_manifest(&paths).with_context(|| {
            anyhow!(
                "failed to initialize browser profile manifest `{}`",
                paths.manifest_path.display()
            )
        })?;
        Ok(Self {
            paths,
            manifest,
            _lock: profile_lock,
        })
    }

    pub fn paths(&self) -> &BrowserProfilePaths {
        &self.paths
    }

    pub fn manifest(&self) -> &BrowserProfileManifest {
        &self.manifest
    }

    pub fn default_partition(&self) -> BrowserProfilePartition<'_> {
        BrowserProfilePartition { paths: &self.paths }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BrowserProfilePartition<'a> {
    paths: &'a BrowserProfilePaths,
}

impl<'a> BrowserProfilePartition<'a> {
    pub fn id(&self) -> &'static str {
        DEFAULT_PROFILE_PARTITION_ID
    }

    pub fn partition_root(&self) -> &'a Path {
        &self.paths.partition_root
    }

    pub fn cookies_path(&self) -> &'a Path {
        &self.paths.cookies_path
    }

    pub fn local_storage_path(&self) -> &'a Path {
        &self.paths.local_storage_path
    }

    pub fn storage_buckets_path(&self) -> &'a Path {
        &self.paths.storage_buckets_path
    }

    pub fn service_worker_resources_path(&self) -> &'a Path {
        &self.paths.service_worker_resources_path
    }

    pub fn cache_storage_root(&self) -> &'a Path {
        &self.paths.cache_storage_root
    }

    pub fn opfs_root(&self) -> &'a Path {
        &self.paths.opfs_root
    }

    pub fn indexed_db_root(&self) -> &'a Path {
        &self.paths.indexeddb_root
    }

    pub fn http_cache_root(&self) -> &'a Path {
        &self.paths.http_cache_root
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;

    use super::BrowserProfile;
    use crate::{BrowserProfilePaths, PROFILE_MANIFEST_VERSION};

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
                "moli-browser-profile-{name}-{}-{nonce}",
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
    fn browser_profile_open_acquires_lock_and_ensures_manifest() -> Result<()> {
        let profile_dir = TempProfileDir::new("open");
        let paths = BrowserProfilePaths::new(&profile_dir.path);

        let profile = BrowserProfile::open(&profile_dir.path)?;

        assert!(paths.lock_path.exists());
        assert!(paths.manifest_path.exists());
        assert_eq!(profile.paths(), &paths);
        assert_eq!(profile.manifest().version, PROFILE_MANIFEST_VERSION);

        let partition = profile.default_partition();
        assert_eq!(partition.id(), "default");
        assert_eq!(partition.partition_root(), paths.partition_root.as_path());
        assert_eq!(partition.cookies_path(), paths.cookies_path.as_path());
        assert_eq!(
            partition.local_storage_path(),
            paths.local_storage_path.as_path()
        );
        assert_eq!(
            partition.storage_buckets_path(),
            paths.storage_buckets_path.as_path()
        );
        assert_eq!(
            partition.service_worker_resources_path(),
            paths.service_worker_resources_path.as_path()
        );
        assert_eq!(
            partition.cache_storage_root(),
            paths.cache_storage_root.as_path()
        );
        assert_eq!(partition.opfs_root(), paths.opfs_root.as_path());
        assert_eq!(partition.indexed_db_root(), paths.indexeddb_root.as_path());
        assert_eq!(partition.http_cache_root(), paths.http_cache_root.as_path());

        drop(profile);
        assert!(paths.manifest_path.exists());

        let _reopened = BrowserProfile::open(&profile_dir.path)?;
        Ok(())
    }

    #[test]
    fn browser_profile_open_refuses_second_writer_until_owner_drops() -> Result<()> {
        let profile_dir = TempProfileDir::new("exclusive");

        let first = BrowserProfile::open(&profile_dir.path)?;
        let error = BrowserProfile::open(&profile_dir.path).expect_err("second owner should fail");
        assert!(
            format!("{error:?}").contains("already locked"),
            "error: {error:?}"
        );

        drop(first);
        let _second = BrowserProfile::open(&profile_dir.path)?;
        Ok(())
    }
}
