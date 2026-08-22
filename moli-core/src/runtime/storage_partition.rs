use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use moli_browser_profile::{
    BrowserProfile, load_cookie_cache as load_profile_cookie_cache,
    save_cookie_cache as save_profile_cookie_cache,
};
use moli_cookie_jar::{
    BrowserCookieStore, CookieSource, SharedBrowserCookieStore, StoredCookie,
    new_shared_browser_cookie_store,
};
use moli_renderer_v8::{
    SharedIndexedDbManager, SharedServiceWorkerResourceStore, downgrade_indexed_db_manager,
};
use moli_storage_service::{
    SharedStorageBucketStore, SharedStorageService, StorageService,
    new_shared_json_storage_bucket_store_with_storage_service,
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager,
};

use crate::{
    network::{
        SharedWebStorageStore, new_shared_json_web_storage_store, new_shared_web_storage_store,
    },
    storage::WeakIndexedDbManager,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoragePartitionPersistence {
    Memory,
    ProfileBacked,
}

pub struct StoragePartitionState {
    persistence: StoragePartitionPersistence,
    id: &'static str,
    cookie_store: SharedBrowserCookieStore,
    web_storage_store: SharedWebStorageStore,
    session_storage_store: SharedWebStorageStore,
    indexed_db_manager: SharedIndexedDbManager,
    storage_service: SharedStorageService,
    storage_bucket_store: SharedStorageBucketStore,
    service_worker_resource_store: SharedServiceWorkerResourceStore,
    http_cache_root: Option<PathBuf>,
    profile: Option<Arc<BrowserProfile>>,
}

#[derive(Clone)]
pub struct StoragePartitionSharedStorageHandles {
    web_storage_store: SharedWebStorageStore,
    indexed_db_manager: SharedIndexedDbManager,
    storage_bucket_store: SharedStorageBucketStore,
    service_worker_resource_store: SharedServiceWorkerResourceStore,
}

impl StoragePartitionSharedStorageHandles {
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
            .finish()
    }
}

impl StoragePartitionState {
    pub fn open(profile_dir: Option<&Path>) -> Result<Self> {
        let profile = profile_dir
            .map(BrowserProfile::open)
            .transpose()?
            .map(Arc::new);
        Self::from_profile(profile)
    }

    fn from_profile(profile: Option<Arc<BrowserProfile>>) -> Result<Self> {
        let cookie_store = new_shared_browser_cookie_store();
        let session_storage_store = new_shared_web_storage_store();
        let (
            persistence,
            id,
            web_storage_store,
            indexed_db_manager,
            storage_service,
            storage_bucket_store,
            service_worker_resource_store,
            http_cache_root,
        ) = if let Some(profile) = profile.as_ref() {
            let partition = profile.default_partition();
            let cookies_path = partition.cookies_path();
            let cookies = load_profile_cookie_cache(cookies_path).with_context(|| {
                anyhow!(
                    "failed to load browser profile cookies `{}`",
                    cookies_path.display()
                )
            })?;
            import_cookies_into_store(&cookie_store, cookies)?;

            let local_storage_path = partition.local_storage_path();
            let web_storage_store = new_shared_json_web_storage_store(local_storage_path)
                .with_context(|| {
                    anyhow!(
                        "failed to initialize browser profile localStorage `{}`",
                        local_storage_path.display()
                    )
                })?;
            let indexed_db_root = partition.indexed_db_root();
            let indexed_db_manager =
                moli_renderer_v8::new_indexed_db_manager(Some(indexed_db_root.to_path_buf()))
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
                web_storage_store,
                indexed_db_manager,
                storage_service,
                storage_bucket_store,
                service_worker_resource_store,
                Some(partition.http_cache_root().to_path_buf()),
            )
        } else {
            let indexed_db_manager = moli_renderer_v8::new_indexed_db_manager(None)
                .map_err(|error| anyhow!(error))
                .context("failed to initialize in-memory IndexedDB")?;
            let storage_service = StorageService::in_memory();
            let storage_bucket_store =
                new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
                    storage_service.clone(),
                    &indexed_db_manager,
                );
            (
                StoragePartitionPersistence::Memory,
                "default",
                new_shared_web_storage_store(),
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
            web_storage_store,
            session_storage_store,
            indexed_db_manager,
            storage_service,
            storage_bucket_store,
            service_worker_resource_store,
            http_cache_root,
            profile,
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

    pub fn profile_cookie_cache_paths(&self) -> Vec<PathBuf> {
        self.profile
            .as_ref()
            .map(|profile| profile.default_partition().cookies_path().to_path_buf())
            .into_iter()
            .collect()
    }

    pub fn cookies(&self) -> Result<Vec<StoredCookie>> {
        snapshot_cookie_store(&self.cookie_store)
    }

    pub fn import_cookies(&self, cookies: impl IntoIterator<Item = StoredCookie>) -> Result<usize> {
        import_cookies_into_store(&self.cookie_store, cookies)
    }

    pub fn commit_cookie_delta(
        &self,
        initial_cookies: &[StoredCookie],
        final_cookies: Option<Vec<StoredCookie>>,
    ) -> Result<()> {
        let Some(final_cookies) = final_cookies else {
            return Ok(());
        };
        {
            let mut cookie_store = self.cookie_store.lock();
            commit_cookie_delta_to_store(&mut cookie_store, initial_cookies, final_cookies)?;
        }
        self.flush()
    }

    pub fn flush(&self) -> Result<()> {
        let Some(profile) = self.profile.as_ref() else {
            return Ok(());
        };
        let cookies_path = profile.default_partition().cookies_path();
        save_profile_cookie_cache(cookies_path, self.cookies()?).with_context(|| {
            anyhow!(
                "failed to save browser profile cookies `{}`",
                cookies_path.display()
            )
        })?;
        Ok(())
    }
}

pub(crate) fn snapshot_cookie_store(
    cookie_store: &SharedBrowserCookieStore,
) -> Result<Vec<StoredCookie>> {
    let mut cookie_store = cookie_store.lock();
    Ok(cookie_store.cookies())
}

pub(crate) fn import_cookies_into_store(
    cookie_store: &SharedBrowserCookieStore,
    cookies: impl IntoIterator<Item = StoredCookie>,
) -> Result<usize> {
    let mut cookie_store = cookie_store.lock();
    let mut accepted = 0usize;
    for cookie in cookies {
        let report = cookie_store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp);
        if report.is_accepted() {
            accepted += 1;
        }
    }
    Ok(accepted)
}

fn commit_cookie_delta_to_store(
    cookie_store: &mut BrowserCookieStore,
    initial_cookies: &[StoredCookie],
    final_cookies: Vec<StoredCookie>,
) -> Result<()> {
    let final_cookies = final_cookies
        .into_iter()
        .filter(|cookie| !cookie.is_expired())
        .collect::<Vec<_>>();
    let current_cookies = cookie_store.cookies();

    for initial in initial_cookies.iter().filter(|cookie| !cookie.is_expired()) {
        let cookie_still_exists_in_session = final_cookies
            .iter()
            .any(|cookie| same_cookie_key(cookie, initial));
        if cookie_still_exists_in_session {
            continue;
        }
        let current_still_matches_initial = current_cookies
            .iter()
            .any(|cookie| same_cookie_key(cookie, initial) && cookie == initial);
        if current_still_matches_initial {
            cookie_store.delete_cookies(
                Some(initial.name.as_str()),
                Some(initial.domain.as_str()),
                Some(initial.path.as_str()),
                None,
            );
        }
    }

    for cookie in final_cookies {
        let report = cookie_store.upsert_with_request_url_report(cookie, None, CookieSource::Cdp);
        if !report.is_accepted() {
            return Err(anyhow!("failed to commit cookie into storage partition"));
        }
    }
    Ok(())
}

fn same_cookie_key(left: &StoredCookie, right: &StoredCookie) -> bool {
    left.name == right.name
        && left.domain == right.domain
        && left.path == right.path
        && left.partition_key == right.partition_key
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;
    use moli_browser_profile::{
        BrowserProfilePaths, load_cookie_cache as load_profile_cookie_cache,
    };
    use moli_cookie_jar::{StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme};
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
    fn memory_partition_owns_ephemeral_stores() -> Result<()> {
        let partition = StoragePartitionState::open(None)?;

        assert_eq!(partition.persistence(), StoragePartitionPersistence::Memory);
        assert_eq!(partition.id(), "default");
        assert!(partition.http_cache_root().is_none());
        assert!(partition.profile_cookie_cache_paths().is_empty());
        let service = partition.storage_service.clone();
        let facade_service = partition.storage_bucket_store().lock().storage_service();
        assert!(Arc::ptr_eq(&service, &facade_service));

        Ok(())
    }

    #[test]
    fn memory_partition_flush_is_noop() -> Result<()> {
        let partition = StoragePartitionState::open(None)?;

        partition.import_cookies(vec![stored_cookie("sid", "memory")])?;
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
        assert_eq!(
            partition.profile_cookie_cache_paths(),
            vec![paths.cookies_path.clone()]
        );
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
        let paths = BrowserProfilePaths::new(&profile_dir.path);
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;

        partition.import_cookies(vec![stored_cookie("sid", "persisted")])?;
        partition.flush()?;

        let cookies = load_profile_cookie_cache(&paths.cookies_path)?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sid");
        assert_eq!(cookies[0].value, "persisted");
        Ok(())
    }

    #[test]
    fn profile_partition_flush_preserves_cookie_scope_metadata() -> Result<()> {
        let profile_dir = TempProfileDir::new("flush-cookie-scope");
        let paths = BrowserProfilePaths::new(&profile_dir.path);
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

        let persisted = load_profile_cookie_cache(&paths.cookies_path)?;
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
            local_store
                .lock()
                .set_item(origin, "local", "profile-backed");
            let session_store = partition.session_storage_store();
            session_store
                .lock()
                .set_item(origin, "session", "memory-only");
        }

        let reopened = StoragePartitionState::open(Some(&profile_dir.path))?;
        let local_store = reopened.web_storage_store();
        let session_store = reopened.session_storage_store();
        assert_eq!(
            local_store.lock().get_item(origin, "local").as_deref(),
            Some("profile-backed")
        );
        assert_eq!(session_store.lock().get_item(origin, "session"), None);
        Ok(())
    }

    #[test]
    fn profile_partition_commit_cookie_delta_persists_deletions() -> Result<()> {
        let profile_dir = TempProfileDir::new("commit-delete");
        let paths = BrowserProfilePaths::new(&profile_dir.path);
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
        let initial_cookies = vec![stored_cookie("sid", "old"), stored_cookie("theme", "dark")];

        partition.import_cookies(initial_cookies.clone())?;
        partition
            .commit_cookie_delta(&initial_cookies, Some(vec![stored_cookie("theme", "dark")]))?;

        let cookies = load_profile_cookie_cache(&paths.cookies_path)?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "theme");
        assert_eq!(cookies[0].value, "dark");
        Ok(())
    }

    #[test]
    fn profile_partition_commit_cookie_delta_uses_full_cookie_key() -> Result<()> {
        let profile_dir = TempProfileDir::new("commit-cookie-key");
        let paths = BrowserProfilePaths::new(&profile_dir.path);
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
        let root = scoped_cookie("sid", "root", "example.com", "/");
        let app = scoped_cookie("sid", "app", "example.com", "/app");
        let subdomain = scoped_cookie("sid", "sub", "sub.example.com", "/");
        let initial_cookies = vec![root.clone(), app.clone(), subdomain.clone()];

        partition.import_cookies(initial_cookies.clone())?;
        partition.commit_cookie_delta(
            &initial_cookies,
            Some(vec![
                app.clone(),
                scoped_cookie("sid", "sub-new", "sub.example.com", "/"),
            ]),
        )?;

        let cookies = load_profile_cookie_cache(&paths.cookies_path)?;
        assert!(
            find_cookie(&cookies, "sid", "example.com", "/").is_none(),
            "missing root-path cookie should be deleted"
        );
        assert_eq!(
            find_cookie(&cookies, "sid", "example.com", "/app").map(|cookie| cookie.value.as_str()),
            Some("app")
        );
        assert_eq!(
            find_cookie(&cookies, "sid", "sub.example.com", "/")
                .map(|cookie| cookie.value.as_str()),
            Some("sub-new")
        );
        Ok(())
    }

    #[test]
    fn profile_partition_commit_cookie_delta_preserves_concurrent_update() -> Result<()> {
        let profile_dir = TempProfileDir::new("commit-concurrent");
        let paths = BrowserProfilePaths::new(&profile_dir.path);
        let partition = StoragePartitionState::open(Some(&profile_dir.path))?;
        let initial_cookies = vec![stored_cookie("sid", "old")];

        partition.import_cookies(initial_cookies.clone())?;
        partition.import_cookies(vec![stored_cookie("sid", "newer")])?;
        partition.commit_cookie_delta(&initial_cookies, Some(Vec::new()))?;

        let cookies = load_profile_cookie_cache(&paths.cookies_path)?;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sid");
        assert_eq!(cookies[0].value, "newer");
        Ok(())
    }
}
