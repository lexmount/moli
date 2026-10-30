use std::path::{Path, PathBuf};

use crate::ProfilePartitionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfilePaths {
    pub root: PathBuf,
    pub lock_path: PathBuf,
    pub manifest_path: PathBuf,
    pub partition_root: PathBuf,
    pub cookies_path: PathBuf,
    pub local_storage_path: PathBuf,
    pub storage_buckets_path: PathBuf,
    pub service_worker_resources_path: PathBuf,
    pub cache_storage_root: PathBuf,
    pub opfs_root: PathBuf,
    pub indexeddb_root: PathBuf,
    pub http_cache_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfilePartitionPaths {
    pub id: ProfilePartitionId,
    pub partition_root: PathBuf,
    pub cookies_path: PathBuf,
    pub local_storage_path: PathBuf,
    pub storage_buckets_path: PathBuf,
    pub service_worker_resources_path: PathBuf,
    pub cache_storage_root: PathBuf,
    pub opfs_root: PathBuf,
    pub indexeddb_root: PathBuf,
    pub http_cache_root: PathBuf,
}

impl BrowserProfilePaths {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let default_partition =
            BrowserProfilePartitionPaths::new(&root, ProfilePartitionId::default_partition());
        Self {
            lock_path: root.join("moli-profile.lock"),
            manifest_path: root.join("profile.json"),
            partition_root: default_partition.partition_root,
            cookies_path: default_partition.cookies_path,
            local_storage_path: default_partition.local_storage_path,
            storage_buckets_path: default_partition.storage_buckets_path,
            service_worker_resources_path: default_partition.service_worker_resources_path,
            cache_storage_root: default_partition.cache_storage_root,
            opfs_root: default_partition.opfs_root,
            indexeddb_root: default_partition.indexeddb_root,
            http_cache_root: default_partition.http_cache_root,
            root,
        }
    }

    pub fn default_partition_paths(&self) -> BrowserProfilePartitionPaths {
        self.partition(&ProfilePartitionId::default_partition())
    }

    pub fn partition(&self, id: &ProfilePartitionId) -> BrowserProfilePartitionPaths {
        BrowserProfilePartitionPaths::new(&self.root, id.clone())
    }
}

impl BrowserProfilePartitionPaths {
    fn new(root: &Path, id: ProfilePartitionId) -> Self {
        let partition_root = root.join("partitions").join(id.as_str());
        Self {
            cookies_path: partition_root.join("cookies.json"),
            local_storage_path: partition_root.join("localstorage.json"),
            storage_buckets_path: partition_root.join("storage-buckets.json"),
            service_worker_resources_path: partition_root.join("service-worker-resources.json"),
            cache_storage_root: partition_root.join("cache-storage"),
            opfs_root: partition_root.join("opfs"),
            indexeddb_root: partition_root.join("idb-lite"),
            http_cache_root: partition_root.join("http-cache"),
            partition_root,
            id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{BrowserProfilePaths, ProfilePartitionId};

    #[test]
    fn browser_profile_paths_use_partitioned_layout() {
        let paths = BrowserProfilePaths::new("/tmp/moli-profile");
        assert_eq!(paths.root, PathBuf::from("/tmp/moli-profile"));
        assert_eq!(
            paths.lock_path,
            PathBuf::from("/tmp/moli-profile/moli-profile.lock")
        );
        assert_eq!(
            paths.manifest_path,
            PathBuf::from("/tmp/moli-profile/profile.json")
        );
        assert_eq!(
            paths.partition_root,
            PathBuf::from("/tmp/moli-profile/partitions/default")
        );
        assert_eq!(
            paths.cookies_path,
            PathBuf::from("/tmp/moli-profile/partitions/default/cookies.json")
        );
        assert_eq!(
            paths.local_storage_path,
            PathBuf::from("/tmp/moli-profile/partitions/default/localstorage.json")
        );
        assert_eq!(
            paths.storage_buckets_path,
            PathBuf::from("/tmp/moli-profile/partitions/default/storage-buckets.json")
        );
        assert_eq!(
            paths.service_worker_resources_path,
            PathBuf::from("/tmp/moli-profile/partitions/default/service-worker-resources.json")
        );
        assert_eq!(
            paths.cache_storage_root,
            PathBuf::from("/tmp/moli-profile/partitions/default/cache-storage")
        );
        assert_eq!(
            paths.opfs_root,
            PathBuf::from("/tmp/moli-profile/partitions/default/opfs")
        );
        assert_eq!(
            paths.indexeddb_root,
            PathBuf::from("/tmp/moli-profile/partitions/default/idb-lite")
        );
        assert_eq!(
            paths.http_cache_root,
            PathBuf::from("/tmp/moli-profile/partitions/default/http-cache")
        );
    }

    #[test]
    fn browser_profile_paths_build_partition_paths_by_id() {
        let paths = BrowserProfilePaths::new("/tmp/moli-profile");

        let partition = paths.partition(&ProfilePartitionId::new("tenant-a").unwrap());

        assert_eq!(partition.id.as_str(), "tenant-a");
        assert_eq!(
            partition.partition_root,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a")
        );
        assert_eq!(
            partition.cookies_path,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/cookies.json")
        );
        assert_eq!(
            partition.local_storage_path,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/localstorage.json")
        );
        assert_eq!(
            partition.storage_buckets_path,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/storage-buckets.json")
        );
        assert_eq!(
            partition.service_worker_resources_path,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/service-worker-resources.json")
        );
        assert_eq!(
            partition.cache_storage_root,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/cache-storage")
        );
        assert_eq!(
            partition.opfs_root,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/opfs")
        );
        assert_eq!(
            partition.indexeddb_root,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/idb-lite")
        );
        assert_eq!(
            partition.http_cache_root,
            PathBuf::from("/tmp/moli-profile/partitions/tenant-a/http-cache")
        );
    }

    #[test]
    fn browser_profile_default_partition_paths_match_legacy_fields() {
        let paths = BrowserProfilePaths::new("/tmp/moli-profile");

        let partition = paths.default_partition_paths();

        assert!(partition.id.is_default());
        assert_eq!(partition.partition_root, paths.partition_root);
        assert_eq!(partition.cookies_path, paths.cookies_path);
        assert_eq!(partition.local_storage_path, paths.local_storage_path);
        assert_eq!(partition.storage_buckets_path, paths.storage_buckets_path);
        assert_eq!(
            partition.service_worker_resources_path,
            paths.service_worker_resources_path
        );
        assert_eq!(partition.cache_storage_root, paths.cache_storage_root);
        assert_eq!(partition.opfs_root, paths.opfs_root);
        assert_eq!(partition.indexeddb_root, paths.indexeddb_root);
        assert_eq!(partition.http_cache_root, paths.http_cache_root);
    }
}
