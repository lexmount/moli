use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use moli_browser_profile::{
    BrowserProfile, COOKIE_DATABASE_RELATIVE_PATH, LOCAL_STORAGE_DATABASE_RELATIVE_PATH,
};
use moli_cookie_jar::{CookieSource, SharedBrowserCookieStore, StoredCookie};
use moli_renderer_v8::{
    SharedIndexedDbManager, SharedServiceWorkerResourceStore, downgrade_indexed_db_manager,
};
use moli_storage_service::{
    SharedStorageBucketStore, SharedStorageService, StorageService,
    new_shared_json_storage_bucket_store_with_storage_service,
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager,
};

use crate::{
    network::{SharedWebStorageStore, new_shared_web_storage_store},
    storage::WeakIndexedDbManager,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoragePartitionPersistence {
    Ephemeral,
    ProfileBacked,
}

pub struct StoragePartitionState {
    persistence: StoragePartitionPersistence,
    id: &'static str,
    cookie_store: SharedBrowserCookieStore,
    cookie_database: moli_cookie_db::CookieDatabase,
    local_storage_database: moli_web_storage::SqliteWebStorageOwner,
    shutdown_result: parking_lot::Mutex<Option<std::result::Result<(), String>>>,
    web_storage_store: SharedWebStorageStore,
    session_storage_store: SharedWebStorageStore,
    indexed_db_manager: SharedIndexedDbManager,
    storage_service: SharedStorageService,
    storage_bucket_store: SharedStorageBucketStore,
    service_worker_resource_store: SharedServiceWorkerResourceStore,
    http_cache_root: Option<PathBuf>,
    profile: Option<Arc<BrowserProfile>>,
    sqlite_runtime: parking_lot::Mutex<Option<moli_sqlite::Runtime>>,
    // Drop after storage handles: temporary disk backing has partition scope,
    // while an explicit profile keeps its files after the partition closes.
    temporary_directory: Option<tempfile::TempDir>,
}

#[derive(Clone)]
pub struct StoragePartitionSharedStorageHandles {
    cookie_store: SharedBrowserCookieStore,
    web_storage_store: SharedWebStorageStore,
    indexed_db_manager: SharedIndexedDbManager,
    storage_bucket_store: SharedStorageBucketStore,
    service_worker_resource_store: SharedServiceWorkerResourceStore,
}

impl StoragePartitionSharedStorageHandles {
    pub fn cookie_store(&self) -> SharedBrowserCookieStore {
        self.cookie_store.clone()
    }
    pub fn web_storage_store(&self) -> SharedWebStorageStore {
        self.web_storage_store.clone()
    }

    pub fn indexed_db_manager(&self) -> SharedIndexedDbManager {
        self.indexed_db_manager.clone()
    }

    pub fn storage_bucket_store(&self) -> SharedStorageBucketStore {
        self.storage_bucket_store.clone()
    }

    pub fn service_worker_resource_store(&self) -> SharedServiceWorkerResourceStore {
        self.service_worker_resource_store.clone()
    }
}

impl fmt::Debug for StoragePartitionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoragePartitionState")
            .field("persistence", &self.persistence)
            .field("id", &self.id)
            .field(
                "indexed_db_manager_strong_count",
                &Arc::strong_count(&self.indexed_db_manager),
            )
            .field(
                "storage_service_strong_count",
                &Arc::strong_count(&self.storage_service),
            )
            .field("http_cache_root", &self.http_cache_root)
            .field("profile_backed", &self.profile.is_some())
            .field(
                "temporary_root",
                &self
                    .temporary_directory
                    .as_ref()
                    .map(tempfile::TempDir::path),
            )
            .finish()
    }
}

impl StoragePartitionState {
    pub fn open(profile_dir: Option<&Path>) -> Result<Self> {
        let runtime = moli_sqlite::Runtime::new()?;
        let profile = profile_dir.map(BrowserProfile::open).transpose()?;
        Self::from_profile(profile.map(Arc::new), runtime)
    }

    fn from_profile(
        profile: Option<Arc<BrowserProfile>>,
        sqlite_runtime: moli_sqlite::Runtime,
    ) -> Result<Self> {
        let temporary_directory = if profile.is_none() {
            Some(
                tempfile::Builder::new()
                    .prefix("moli-storage-")
                    .tempdir()
                    .context("failed to create temporary browser storage directory")?,
            )
        } else {
            None
        };
        let root = profile
            .as_ref()
            .map(|profile| profile.default_partition().partition_root())
            .unwrap_or_else(|| {
                temporary_directory
                    .as_ref()
                    .expect("temporary partition directory")
                    .path()
            });
        let mode = moli_sqlite::OpenMode::Create;
        let (cookie_store, cookie_database) = moli_cookie_db::CookieDatabase::deferred(
            root.join(COOKIE_DATABASE_RELATIVE_PATH),
            mode,
            sqlite_runtime.handle(),
        )
        .context("failed to initialize partition Cookie database")?;
        let (web_storage_store, local_storage_database) =
            moli_web_storage::new_lazy_sqlite_web_storage_store(
                root.join(LOCAL_STORAGE_DATABASE_RELATIVE_PATH),
                mode,
                sqlite_runtime.handle(),
            )
            .context("failed to initialize partition localStorage database")?;
        let session_storage_store = new_shared_web_storage_store();
        let (
            persistence,
            id,
            indexed_db_manager,
            storage_service,
            storage_bucket_store,
            service_worker_resource_store,
            http_cache_root,
        ) = if let Some(profile) = profile.as_ref() {
            let partition = profile.default_partition();
            partition.ensure_directory()?;
            let indexed_db_root = partition.indexed_db_root();
            let indexed_db_manager = moli_renderer_v8::new_sqlite_indexed_db_manager_with_runtime(
                Some(indexed_db_root.to_path_buf()),
                Default::default(),
                sqlite_runtime.handle(),
            )
            .map_err(|error| anyhow!(error))
            .with_context(|| {
                anyhow!(
                    "failed to initialize browser profile IndexedDB `{}`",
                    indexed_db_root.display()
                )
            })?;
            let storage_buckets_path = partition.storage_buckets_path();
            let cache_storage_root = partition.cache_storage_root();
            let opfs_root = partition.opfs_root();
            let storage_service = StorageService::on_disk(opfs_root.to_path_buf())
                .map_err(|error| anyhow!(error))
                .with_context(|| {
                    anyhow!(
                        "failed to initialize browser profile OPFS `{}`",
                        opfs_root.display()
                    )
                })?;
            let storage_bucket_store = new_shared_json_storage_bucket_store_with_storage_service(
                storage_buckets_path,
                cache_storage_root,
                &indexed_db_manager,
                storage_service.clone(),
            )
            .with_context(|| {
                anyhow!(
                    "failed to initialize browser profile Storage Buckets `{}` with CacheStorage root `{}`",
                    storage_buckets_path.display(),
                    cache_storage_root.display()
                )
            })?;
            let service_worker_resources_path = partition.service_worker_resources_path();
            let service_worker_resource_store =
                moli_renderer_v8::new_shared_json_service_worker_resource_store(
                    service_worker_resources_path,
                )
                .with_context(|| {
                    anyhow!(
                        "failed to initialize browser profile Service Worker resources `{}`",
                        service_worker_resources_path.display()
                    )
                })?;
            (
                StoragePartitionPersistence::ProfileBacked,
                partition.id(),
                indexed_db_manager,
                storage_service,
                storage_bucket_store,
                service_worker_resource_store,
                Some(partition.http_cache_root().to_path_buf()),
            )
        } else {
            let root = temporary_directory
                .as_ref()
                .expect("an ephemeral partition owns its temporary directory")
                .path();
            let indexed_db_manager = moli_renderer_v8::new_sqlite_indexed_db_manager_with_runtime(
                Some(root.join("idb-lite")),
                Default::default(),
                sqlite_runtime.handle(),
            )
            .map_err(|error| anyhow!(error))
            .context("failed to initialize temporary IndexedDB")?;
            let storage_service = StorageService::on_disk(root.join("opfs"))
                .map_err(|error| anyhow!(error))
                .context("failed to initialize temporary OPFS")?;
            let storage_bucket_store =
                new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
                    storage_service.clone(),
                    &indexed_db_manager,
                );
            (
                StoragePartitionPersistence::Ephemeral,
                "default",
                indexed_db_manager,
                storage_service,
                storage_bucket_store,
                moli_renderer_v8::new_shared_service_worker_resource_store(),
                None,
            )
        };
        Ok(Self {
            persistence,
            id,
            cookie_store,
            cookie_database,
            local_storage_database,
            shutdown_result: parking_lot::Mutex::new(None),
            web_storage_store,
            session_storage_store,
            indexed_db_manager,
            storage_service,
            storage_bucket_store,
            service_worker_resource_store,
            http_cache_root,
            profile,
            sqlite_runtime: parking_lot::Mutex::new(Some(sqlite_runtime)),
            temporary_directory,
        })
    }

    pub(crate) fn persistence(&self) -> StoragePartitionPersistence {
        self.persistence
    }

    pub(crate) fn id(&self) -> &'static str {
        self.id
    }

    pub(crate) fn cookie_store(&self) -> SharedBrowserCookieStore {
        self.cookie_store.clone()
    }

    pub(crate) fn web_storage_store(&self) -> SharedWebStorageStore {
        self.web_storage_store.clone()
    }

    pub(crate) fn session_storage_store(&self) -> SharedWebStorageStore {
        self.session_storage_store.clone()
    }

    pub(crate) fn indexed_db_manager(&self) -> &SharedIndexedDbManager {
        &self.indexed_db_manager
    }

    pub(crate) fn weak_indexed_db_manager(&self) -> WeakIndexedDbManager {
        downgrade_indexed_db_manager(&self.indexed_db_manager)
    }

    pub fn shared_storage_handles(&self) -> StoragePartitionSharedStorageHandles {
        StoragePartitionSharedStorageHandles {
            cookie_store: self.cookie_store.clone(),
            web_storage_store: self.web_storage_store.clone(),
            indexed_db_manager: self.indexed_db_manager.clone(),
            storage_bucket_store: self.storage_bucket_store.clone(),
            service_worker_resource_store: self.service_worker_resource_store.clone(),
        }
    }

    pub(crate) fn storage_bucket_store(&self) -> SharedStorageBucketStore {
        self.storage_bucket_store.clone()
    }

    pub(crate) fn service_worker_resource_store(&self) -> SharedServiceWorkerResourceStore {
        self.service_worker_resource_store.clone()
    }

    pub fn http_cache_root(&self) -> Option<&Path> {
        self.http_cache_root.as_deref()
    }

    #[cfg(test)]
    fn cookie_database_path(&self) -> PathBuf {
        self.profile
            .as_ref()
            .map(|profile| profile.paths().cookies_path.clone())
            .unwrap_or_else(|| {
                self.temporary_directory
                    .as_ref()
                    .expect("temporary partition directory")
                    .path()
                    .join(COOKIE_DATABASE_RELATIVE_PATH)
            })
    }
    #[cfg(test)]
    fn local_storage_database_path(&self) -> PathBuf {
        self.profile
            .as_ref()
            .map(|profile| profile.paths().local_storage_path.clone())
            .unwrap_or_else(|| {
                self.temporary_directory
                    .as_ref()
                    .expect("temporary partition directory")
                    .path()
                    .join(LOCAL_STORAGE_DATABASE_RELATIVE_PATH)
            })
    }

    pub fn cookies(&self) -> Result<Vec<StoredCookie>> {
        snapshot_cookie_store(&self.cookie_store)
    }

    pub fn import_cookies(&self, cookies: impl IntoIterator<Item = StoredCookie>) -> Result<usize> {
        import_cookies_into_store(&self.cookie_store, cookies)
    }

    pub fn flush(&self) -> Result<()> {
        let closed = self.shutdown_result.lock();
        if let Some(result) = closed.as_ref() {
            return result.clone().map_err(anyhow::Error::msg);
        }
        self.flush_open_services()
    }
    fn flush_open_services(&self) -> Result<()> {
        let cookies = self.cookie_database.flush().map_err(anyhow::Error::new);
        let local = self.local_storage_database.flush();
        cookies.and(local)
    }

    /// Called after renderer and network producers have joined. All accepted
    /// bucket cleanup and SQL rollback/commit must finish before profile unlock.
    pub fn shutdown(&self) -> Result<()> {
        let mut closed = self.shutdown_result.lock();
        if let Some(result) = closed.as_ref() {
            return result.clone().map_err(anyhow::Error::msg);
        }
        // Run every stage even when an earlier stage reports failure. In
        // particular, a failed storage barrier must not skip SQL retirement.
        let maintenance = self
            .storage_service
            .drain_maintenance()
            .map_err(anyhow::Error::new);
        let sql = self
            .indexed_db_manager
            .lock()
            .shutdown_sqlite()
            .map_err(anyhow::Error::new);
        // Accepted Blob reads may finish preparing a write while SQL drains.
        // Closed clients reject later continuations before quota sampling;
        // finish sampling already started before releasing the profile lock.
        let late_maintenance = self
            .storage_service
            .drain_maintenance()
            .map_err(anyhow::Error::new);
        let io = self.storage_service.drain_io().map_err(anyhow::Error::new);
        let cookies = self.cookie_database.close().map_err(anyhow::Error::new);
        let local = self.local_storage_database.close();
        let runtime = self
            .sqlite_runtime
            .lock()
            .take()
            .map_or(Ok(()), |mut runtime| {
                runtime.shutdown().map_err(anyhow::Error::new)
            });
        let result = maintenance
            .and(sql)
            .and(late_maintenance)
            .and(io)
            .and(cookies)
            .and(local)
            .and(runtime);
        *closed = Some(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|error| format!("{error:#}")),
        );
        result
    }
}

impl Drop for StoragePartitionState {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::warn!(error = %error, "failed to close browser storage partition");
        }
    }
}

pub(crate) fn snapshot_cookie_store(
    cookie_store: &SharedBrowserCookieStore,
) -> Result<Vec<StoredCookie>> {
    let mut cookie_store = cookie_store.lock()?;
    Ok(cookie_store.cookies())
}

pub(crate) fn import_cookies_into_store(
    cookie_store: &SharedBrowserCookieStore,
    cookies: impl IntoIterator<Item = StoredCookie>,
) -> Result<usize> {
    let mut cookies = cookies.into_iter().peekable();
    if cookies.peek().is_none() {
        return Ok(0);
    }
    let mut cookie_store = cookie_store.lock()?;
    let mut accepted = 0usize;
    for cookie in cookies {
        let report = cookie_store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp);
        if report.is_accepted() {
            accepted += 1;
        }
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    mod directory_tests;
    mod shutdown_tests;
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;
    use moli_browser_profile::BrowserProfilePaths;
    use moli_cookie_jar::{
        CookieSource, StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme,
    };
    use moli_storage_key::MoliStorageKey;
    use moli_storage_service::StorageBucketLocator;

    use super::{StoragePartitionPersistence, StoragePartitionState};

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
                "moli-storage-partition-{name}-{}-{nonce}",
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

    fn stored_cookie(name: &str, value: &str) -> StoredCookie {
        StoredCookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: "example.com".to_owned(),
            host_only: false,
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

    fn scoped_cookie(name: &str, value: &str, domain: &str, path: &str) -> StoredCookie {
        let mut cookie = stored_cookie(name, value);
        cookie.domain = domain.to_owned();
        cookie.path = path.to_owned();
        cookie
    }

    fn find_cookie<'a>(
        cookies: &'a [StoredCookie],
        name: &str,
        domain: &str,
        path: &str,
    ) -> Option<&'a StoredCookie> {
        cookies
            .iter()
            .find(|cookie| cookie.name == name && cookie.domain == domain && cookie.path == path)
    }

    #[test]
    fn ephemeral_partition_owns_temporary_stores() -> Result<()> {
        let partition = StoragePartitionState::open(None)?;
        let root = partition
            .temporary_directory
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();

        assert_eq!(
            partition.persistence(),
            StoragePartitionPersistence::Ephemeral
        );
        assert_eq!(partition.id(), "default");
        assert!(partition.http_cache_root().is_none());
        assert!(!partition.cookie_database_path().exists());
        assert!(!partition.local_storage_database_path().exists());
        partition.flush_open_services()?;
        assert!(!partition.cookie_database_path().exists());
        assert!(!partition.local_storage_database_path().exists());
        let service = partition.storage_service.clone();
        let facade_service = partition.storage_bucket_store().lock().storage_service();
        assert!(Arc::ptr_eq(&service, &facade_service));
        assert!(root.join("idb-lite/format.json").is_file());
        assert!(!root.join("opfs").exists(), "OPFS remains lazy");

        drop(partition);
        assert!(
            !root.exists(),
            "normal partition teardown removes temporary data"
        );

        Ok(())
    }

    #[test]
    fn ephemeral_partition_opfs_is_disk_backed_and_partition_isolated() -> Result<()> {
        let first = StoragePartitionState::open(None)?;
        let second = StoragePartitionState::open(None)?;
        let root = first.temporary_directory.as_ref().unwrap().path();
        assert_ne!(root, second.temporary_directory.as_ref().unwrap().path());
        let locator = StorageBucketLocator::default_bucket("https://temporary-opfs.test");
        let key = moli_storage_service::StorageService::opfs_bucket_key(&locator)?;
        let directory = first.storage_service.ensure_opfs_root(&locator)?;
        let file = first.storage_service.with_opfs(|opfs| {
            let file = opfs.get_file(&key, &directory, "entry.txt", true)?;
            opfs.write_file(&key, &file, b"temporary bytes", None)?;
            Ok::<_, moli_storage_service::OpfsError>(file)
        })?;
        // A separate backend must see the bytes through the host filesystem,
        // without sharing the first service's in-memory namespace.
        let reopened = moli_storage_service::StorageService::on_disk(root.join("opfs"))?;
        assert_eq!(
            reopened
                .with_opfs(|opfs| opfs.read_file(&key, &file))?
                .bytes,
            b"temporary bytes",
        );
        let other_directory = second.storage_service.ensure_opfs_root(&locator)?;
        assert!(
            second
                .storage_service
                .with_opfs(|opfs| { opfs.get_file(&key, &other_directory, "entry.txt", false) })
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn ephemeral_partition_cookie_flush_uses_temporary_sqlite() -> Result<()> {
        let partition = StoragePartitionState::open(None)?;

        partition.import_cookies(vec![stored_cookie("sid", "temporary")])?;
        partition.flush()?;

        assert_eq!(partition.cookies()?.len(), 1);
        Ok(())
    }

    #[test]
    fn profile_partition_owns_manifest_lock_and_default_paths() -> Result<()> {
        let profile_dir = TempProfileDir::new("profile-backed");
        let paths = BrowserProfilePaths::new(&profile_dir.path);

        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;

        assert_eq!(
            partition.persistence(),
            StoragePartitionPersistence::ProfileBacked
        );
        assert_eq!(partition.id(), "default");
        assert_eq!(
            partition.http_cache_root(),
            Some(paths.http_cache_root.as_path())
        );
        assert_eq!(partition.cookie_database_path(), paths.cookies_path.clone());
        assert!(paths.lock_path.exists());
        assert!(paths.manifest_path.exists());
        assert!(
            !paths.opfs_root.exists(),
            "opening a profile partition must not eagerly create its OPFS root"
        );
        let service = partition.storage_service.clone();
        let facade_service = partition.storage_bucket_store().lock().storage_service();
        assert!(Arc::ptr_eq(&service, &facade_service));
        let storage_key = MoliStorageKey::first_party_from_url(
            &url::Url::parse("https://partition-opfs-lazy.test/")?,
            None,
        );
        let locator = StorageBucketLocator::default_bucket(storage_key.serialized_storage_key());
        service.ensure_opfs_root(&locator)?;
        assert!(
            paths.opfs_root.is_dir(),
            "the first actual OPFS operation must create the configured root"
        );

        drop(partition);
        let _reopened = StoragePartitionState::open(Some(&profile_dir.path))?;
        Ok(())
    }

    #[test]
    fn profile_partition_flush_persists_cookies() -> Result<()> {
        let profile_dir = TempProfileDir::new("flush");
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;

        partition.import_cookies(vec![stored_cookie("sid", "persisted")])?;
        partition.flush()?;

        drop(partition);
        let cookies = StoragePartitionState::open(Some(&profile_dir.path))?.cookies()?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sid");
        assert_eq!(cookies[0].value, "persisted");
        Ok(())
    }

    #[test]
    fn profile_partition_flush_preserves_cookie_scope_metadata() -> Result<()> {
        let profile_dir = TempProfileDir::new("flush-cookie-scope");
        let mut scoped = scoped_cookie("scoped", "value", "example.com", "/app");
        scoped.host_only = true;
        scoped.secure = true;
        scoped.http_only = true;
        scoped.same_site = StoredCookieSameSite::Strict;
        scoped.source_scheme = StoredCookieSourceScheme::Secure;
        scoped.source_port = 443;

        {
            let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
            partition.import_cookies(vec![scoped])?;
            partition.flush()?;
        }

        let persisted = StoragePartitionState::open(Some(&profile_dir.path))?.cookies()?;
        let persisted_cookie = find_cookie(&persisted, "scoped", "example.com", "/app")
            .expect("profile cookie cache should preserve scoped cookie");
        assert_eq!(persisted_cookie.value, "value");
        assert!(persisted_cookie.host_only);
        assert!(persisted_cookie.secure);
        assert!(persisted_cookie.http_only);
        assert_eq!(persisted_cookie.same_site, StoredCookieSameSite::Strict);
        assert_eq!(
            persisted_cookie.source_scheme,
            StoredCookieSourceScheme::Secure
        );
        assert_eq!(persisted_cookie.source_port, 443);

        let reopened = StoragePartitionState::open(Some(&profile_dir.path))?;
        let reopened_cookies = reopened.cookies()?;
        let reopened_cookie = find_cookie(&reopened_cookies, "scoped", "example.com", "/app")
            .expect("reopened partition should restore scoped cookie");
        assert_eq!(reopened_cookie.value, "value");
        assert!(reopened_cookie.host_only);
        assert!(reopened_cookie.secure);
        assert!(reopened_cookie.http_only);
        assert_eq!(reopened_cookie.same_site, StoredCookieSameSite::Strict);
        assert_eq!(
            reopened_cookie.source_scheme,
            StoredCookieSourceScheme::Secure
        );
        assert_eq!(reopened_cookie.source_port, 443);
        Ok(())
    }

    #[test]
    fn profile_partition_reopen_keeps_session_storage_memory_only() -> Result<()> {
        let profile_dir = TempProfileDir::new("session-storage-memory");
        let origin = "https://profile-session-storage.test";

        {
            let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
            let local_store = partition.web_storage_store();
            local_store.set_item(origin, "local", "profile-backed");
            let session_store = partition.session_storage_store();
            session_store.set_item(origin, "session", "memory-only");
        }

        let reopened = StoragePartitionState::open(Some(&profile_dir.path))?;
        let local_store = reopened.web_storage_store();
        let session_store = reopened.session_storage_store();
        assert_eq!(
            local_store.get_item(origin, "local").as_deref(),
            Some("profile-backed")
        );
        assert_eq!(session_store.get_item(origin, "session"), None);
        Ok(())
    }

    #[test]
    fn profile_partition_live_cookie_deletions_persist() -> Result<()> {
        let profile_dir = TempProfileDir::new("live-delete");
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
        partition.import_cookies([stored_cookie("sid", "old"), stored_cookie("theme", "dark")])?;
        partition
            .cookie_store()
            .lock()?
            .delete_cookies(Some("sid"), None, None, None);
        partition.flush()?;
        drop(partition);
        let cookies = StoragePartitionState::open(Some(&profile_dir.path))?.cookies()?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "theme");
        assert_eq!(cookies[0].value, "dark");
        Ok(())
    }
    #[test]
    fn profile_partition_live_mutations_use_the_complete_cookie_identity() -> Result<()> {
        let profile_dir = TempProfileDir::new("live-cookie-key");
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
        partition.import_cookies([
            scoped_cookie("sid", "root", "example.com", "/"),
            scoped_cookie("sid", "app", "example.com", "/app"),
            scoped_cookie("sid", "sub", "sub.example.com", "/"),
        ])?;
        partition
            .cookie_store()
            .lock()?
            .delete_cookies_matching(|cookie| {
                cookie.name == "sid" && cookie.domain == "example.com" && cookie.path == "/"
            });
        partition.import_cookies([scoped_cookie("sid", "sub-new", "sub.example.com", "/")])?;
        partition.flush()?;
        drop(partition);
        let cookies = StoragePartitionState::open(Some(&profile_dir.path))?.cookies()?;
        assert!(find_cookie(&cookies, "sid", "example.com", "/").is_none());
        assert_eq!(
            find_cookie(&cookies, "sid", "example.com", "/app").map(|c| c.value.as_str()),
            Some("app")
        );
        assert_eq!(
            find_cookie(&cookies, "sid", "sub.example.com", "/").map(|c| c.value.as_str()),
            Some("sub-new")
        );
        Ok(())
    }
    #[test]
    fn shared_partition_handles_use_the_live_cookie_store_and_shutdown_is_idempotent() -> Result<()>
    {
        let profile_dir = TempProfileDir::new("shared-live-cookie");
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
        let handles = partition.shared_storage_handles();
        let shared = handles.cookie_store();
        assert!(Arc::ptr_eq(&shared, &partition.cookie_store()));
        partition.import_cookies([stored_cookie("sid", "old")])?;
        shared.lock()?.upsert_with_request_url_report(
            stored_cookie("sid", "newer"),
            None,
            CookieSource::Cdp,
        );
        partition.shutdown()?;
        partition.shutdown()?;
        partition.flush()?;
        drop(partition);
        let cookies = StoragePartitionState::open(Some(&profile_dir.path))?.cookies()?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].value, "newer");
        Ok(())
    }
}
