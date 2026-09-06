use std::{
    cell::RefCell,
    fmt,
    marker::PhantomData,
    rc::{Rc, Weak as RcWeak},
    sync::{
        Arc, Weak as ArcWeak,
        atomic::{AtomicU64, Ordering},
    },
};

use moli_cookie_jar::SharedBrowserCookieStore;
use moli_fetch::{FetchClient, FetchClientHandle, FetchConfig, FetchRuntimeJoinReport};
use parking_lot::{Mutex, RwLock};

use super::{
    BrowserResourceRuntimeDiagnostics, SharedMemoryResourceCacheDiagnostics,
    memory_cache::SharedMemoryResourceCache,
};
use crate::network::loads::{
    DetachedKeepaliveLoadDiagnostics, DetachedKeepaliveLoadRegistry, ResourceLoadId,
    ResourceLoadKind,
};

static NEXT_BROWSER_RESOURCE_RUNTIME_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_BROWSER_RESOURCE_OWNER_ROOT_ID: AtomicU64 = AtomicU64::new(0);

/// Request-side transport lease and Context-owned renderer memory cache.
///
/// Clones share the exact libcurl/HTTP runtime and cookie store. Transport
/// replacements under the same owner root and storage retain one bounded
/// memory cache, including live peer Pages' resource partitions. This handle
/// must not acquire mutable Page, Document, Worker, or request-delivery state.
#[derive(Clone)]
pub struct BrowserResourceRuntime {
    inner: Arc<BrowserResourceRuntimeInner>,
}

/// Thread-affine structured owner for one browser resource runtime.
///
/// Request clients receive only [`BrowserResourceRuntime`]. Keeping this owner
/// separate makes it impossible for a `Send + 'static` renderer callback to
/// capture the fetch semantic thread's join responsibility.
#[derive(Debug)]
pub struct BrowserResourceRuntimeOwner {
    runtime_id: u64,
    runtime: ArcWeak<BrowserResourceRuntimeInner>,
    fetch_owner: FetchClient,
    _thread_affine: PhantomData<Rc<()>>,
}

/// Uncloneable registration token carrying a newly created runtime to its
/// browser-context owner root. It exposes no request handle before the owner is
/// registered, preventing temporary-owner construction from orphaning the
/// semantic thread.
#[derive(Debug)]
pub struct BrowserResourceRuntimeOwnerRegistration {
    fetch_owner: FetchClient,
    _thread_affine: PhantomData<Rc<()>>,
}

/// Non-cloneable lifetime root for active and replaced resource runtimes.
///
/// A replaced owner remains retired while any request-side runtime handle is
/// observable. Opportunistic reaping joins it once only the owner's own handle
/// remains. Terminal shutdown broadcasts to every owner before joining any of
/// them.
#[derive(Debug)]
struct BrowserResourceRuntimeOwnerSet {
    root_id: u64,
    binding: BrowserResourceRuntimeBinding,
    active_runtime_id: Option<u64>,
    owners: Vec<BrowserResourceRuntimeOwner>,
    terminal: bool,
    _thread_affine: PhantomData<Rc<()>>,
}

/// The sole strong, thread-affine lifetime root for an owner set.
#[derive(Debug)]
pub struct BrowserResourceRuntimeOwnerRoot {
    inner: Rc<RefCell<BrowserResourceRuntimeOwnerSet>>,
}

/// Weak root-local capability used by NavigationEngine/background wrappers.
///
/// It is cloneable but not `Send`, cannot extend the owner lifetime, and is
/// never stored in renderer context state or request callbacks.
#[derive(Clone, Debug)]
pub struct BrowserResourceRuntimeOwnerRegistrar {
    root_id: u64,
    inner: RcWeak<RefCell<BrowserResourceRuntimeOwnerSet>>,
}

/// Browser-context-owned pointer to the currently configured network backend.
///
/// Existing Document/Worker load leases retain the exact
/// `BrowserResourceRuntime` captured when their request started. Replacing this
/// binding therefore affects only later execution-context creation and later
/// requests. In particular, a persisted Service Worker can start without
/// borrowing an ambient page while still observing an intentional browser
/// context network-runtime rebuild.
#[derive(Clone, Debug)]
pub(crate) struct BrowserResourceRuntimeBinding {
    root_id: u64,
    inner: Arc<RwLock<BrowserResourceRuntime>>,
}

impl BrowserResourceRuntimeBinding {
    fn new(root_id: u64, runtime: BrowserResourceRuntime) -> Self {
        Self {
            root_id,
            inner: Arc::new(RwLock::new(runtime)),
        }
    }

    pub(crate) fn current(&self) -> BrowserResourceRuntime {
        self.inner.read().clone()
    }

    fn replace(&self, runtime: BrowserResourceRuntime) {
        *self.inner.write() = runtime;
    }
}

struct BrowserResourceRuntimeInner {
    id: u64,
    owner_root_id: u64,
    client: FetchClientHandle,
    memory_cache: Arc<Mutex<SharedMemoryResourceCache>>,
    detached_keepalive_loads: DetachedKeepaliveLoadRegistry,
}

impl BrowserResourceRuntimeOwner {
    // The registration deliberately returns the request handle together with
    // its unique owner so callers cannot construct an unowned runtime.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        config: &FetchConfig,
        cookie_store: SharedBrowserCookieStore,
    ) -> BrowserResourceRuntimeOwnerRegistration {
        BrowserResourceRuntimeOwnerRegistration {
            fetch_owner: FetchClient::new(config, cookie_store),
            _thread_affine: PhantomData,
        }
    }
}

impl BrowserResourceRuntime {
    fn owner_root_id(&self) -> u64 {
        self.inner.owner_root_id
    }

    pub fn cookie_store(&self) -> SharedBrowserCookieStore {
        self.inner.client.cookie_store()
    }

    pub fn browser_identity(&self) -> &moli_browser_profile::BrowserIdentityProfile {
        self.inner.client.browser_identity()
    }

    pub fn matches_fetch_config(&self, config: &FetchConfig) -> bool {
        self.inner.client.matches_config(config)
    }

    pub fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn runtime_id_for_diagnostics(&self) -> u64 {
        self.inner.id
    }

    pub fn memory_cache_diagnostics(&self) -> SharedMemoryResourceCacheDiagnostics {
        self.inner.memory_cache.lock().diagnostics()
    }

    pub fn diagnostics(&self) -> BrowserResourceRuntimeDiagnostics {
        BrowserResourceRuntimeDiagnostics {
            runtime_id: self.runtime_id_for_diagnostics(),
            memory_cache: self.memory_cache_diagnostics(),
            detached_keepalive_loads: self.detached_keepalive_diagnostics(),
        }
    }

    pub(crate) fn detached_keepalive_diagnostics(&self) -> DetachedKeepaliveLoadDiagnostics {
        self.inner.detached_keepalive_loads.diagnostics()
    }

    pub(crate) fn register_detached_keepalive_load(
        &self,
        id: ResourceLoadId,
        kind: ResourceLoadKind,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) {
        self.inner
            .detached_keepalive_loads
            .insert(id, kind, cancel_handle);
    }

    pub(crate) fn attach_detached_keepalive_cancel_handle(
        &self,
        id: ResourceLoadId,
        cancel_handle: moli_fetch::FetchCancelHandle,
    ) {
        let _ = self
            .inner
            .detached_keepalive_loads
            .attach_cancel_handle(id, cancel_handle);
    }

    pub(crate) fn remove_detached_keepalive_load(&self, id: ResourceLoadId) {
        let _ = self.inner.detached_keepalive_loads.remove(id);
    }

    pub(in crate::network) fn client(&self) -> &FetchClientHandle {
        &self.inner.client
    }

    pub(in crate::network) fn memory_cache(&self) -> &Mutex<SharedMemoryResourceCache> {
        &self.inner.memory_cache
    }
}

impl BrowserResourceRuntimeOwner {
    pub fn request_shutdown(&self) {
        self.fetch_owner.request_shutdown();
    }

    pub fn join(&mut self) -> FetchRuntimeJoinReport {
        self.fetch_owner.join()
    }

    fn runtime_is_gone(&self) -> bool {
        self.runtime.upgrade().is_none()
    }
}

impl BrowserResourceRuntimeOwnerRegistration {
    fn into_parts(
        self,
        owner_root_id: u64,
        memory_cache: Arc<Mutex<SharedMemoryResourceCache>>,
    ) -> (BrowserResourceRuntime, BrowserResourceRuntimeOwner) {
        // No runtime handle exists before registration. Root identity and cache
        // residence are immutable from construction, not a later mutable bind.
        let runtime_id = NEXT_BROWSER_RESOURCE_RUNTIME_ID
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let runtime = BrowserResourceRuntime {
            inner: Arc::new(BrowserResourceRuntimeInner {
                id: runtime_id,
                owner_root_id,
                client: self.fetch_owner.handle(),
                memory_cache,
                detached_keepalive_loads: DetachedKeepaliveLoadRegistry::default(),
            }),
        };
        let owner = BrowserResourceRuntimeOwner {
            runtime_id,
            runtime: Arc::downgrade(&runtime.inner),
            fetch_owner: self.fetch_owner,
            _thread_affine: PhantomData,
        };
        (runtime, owner)
    }
}

impl BrowserResourceRuntimeOwnerSet {
    fn new(
        root_id: u64,
        binding: BrowserResourceRuntimeBinding,
        owner: BrowserResourceRuntimeOwner,
    ) -> Self {
        assert_eq!(
            root_id, binding.root_id,
            "owner root/binding identity mismatch"
        );
        assert_eq!(
            owner.runtime_id,
            binding.current().runtime_id_for_diagnostics(),
            "initial runtime owner/binding mismatch"
        );
        Self {
            root_id,
            binding,
            active_runtime_id: Some(owner.runtime_id),
            owners: vec![owner],
            terminal: false,
            _thread_affine: PhantomData,
        }
    }

    fn replace_owned(
        &mut self,
        registration: BrowserResourceRuntimeOwnerRegistration,
    ) -> Result<BrowserResourceRuntime, &'static str> {
        if self.terminal {
            return Err("browser resource runtime owner root is shut down");
        }
        let memory_cache = {
            let current = self.binding.current();
            if Arc::ptr_eq(
                &current.cookie_store(),
                &registration.fetch_owner.cookie_store(),
            ) {
                Arc::clone(&current.inner.memory_cache)
            } else {
                Arc::default()
            }
        };
        let (runtime, owner) = registration.into_parts(self.root_id, memory_cache);
        debug_assert_eq!(owner.runtime_id, runtime.runtime_id_for_diagnostics());

        // This is one closed operation over a root-bound binding: no arbitrary
        // callback can re-enter the RefCell between registration and swap.
        self.owners.push(owner);
        self.binding.replace(runtime.clone());
        self.active_runtime_id = Some(runtime.runtime_id_for_diagnostics());
        let _ = self.reap_retired();
        Ok(runtime)
    }

    fn adopt_registered(&mut self, runtime: BrowserResourceRuntime) -> Result<(), &'static str> {
        if self.terminal {
            return Err("browser resource runtime owner root is shut down");
        }
        if runtime.owner_root_id() != self.root_id {
            return Err("browser resource runtime belongs to another owner root");
        }
        let runtime_id = runtime.runtime_id_for_diagnostics();
        if !self
            .owners
            .iter()
            .any(|owner| owner.runtime_id == runtime_id)
        {
            return Err("browser resource runtime owner is no longer registered");
        }
        self.binding.replace(runtime);
        self.active_runtime_id = Some(runtime_id);
        let _ = self.reap_retired();
        Ok(())
    }

    fn current_registered(&mut self) -> Result<BrowserResourceRuntime, &'static str> {
        if self.terminal {
            return Err("browser resource runtime owner root is shut down");
        }
        let _ = self.reap_retired();
        let runtime = self.binding.current();
        let runtime_id = runtime.runtime_id_for_diagnostics();
        if Some(runtime_id) != self.active_runtime_id
            || !self
                .owners
                .iter()
                .any(|owner| owner.runtime_id == runtime_id)
        {
            return Err("browser resource runtime binding has no active registered owner");
        }
        Ok(runtime)
    }

    fn validate_registered(&self, runtime: &BrowserResourceRuntime) -> Result<(), &'static str> {
        if self.terminal {
            return Err("browser resource runtime owner root is shut down");
        }
        if runtime.owner_root_id() != self.root_id {
            return Err("browser resource runtime belongs to another owner root");
        }
        if !self
            .owners
            .iter()
            .any(|owner| owner.runtime_id == runtime.runtime_id_for_diagnostics())
        {
            return Err("browser resource runtime owner is no longer registered");
        }
        Ok(())
    }

    fn reap_retired(&mut self) -> Vec<FetchRuntimeJoinReport> {
        let active_runtime_id = self.active_runtime_id;
        let mut retired = Vec::new();
        let mut index = 0;
        while index < self.owners.len() {
            let owner = &self.owners[index];
            let is_active = Some(owner.runtime_id) == active_runtime_id;
            if !is_active && owner.runtime_is_gone() {
                retired.push(self.owners.swap_remove(index));
            } else {
                index += 1;
            }
        }
        shutdown_and_join_resource_runtime_owners(&mut retired)
    }

    fn shutdown_and_join(&mut self) {
        if self.terminal && self.owners.is_empty() {
            return;
        }
        self.terminal = true;
        self.active_runtime_id = None;
        let _ = shutdown_and_join_resource_runtime_owners(&mut self.owners);
        self.owners.clear();
    }

    #[cfg(test)]
    fn owner_count_for_testing(&self) -> usize {
        self.owners.len()
    }
}

impl BrowserResourceRuntimeOwnerRoot {
    pub(crate) fn new(
        initial: BrowserResourceRuntimeOwnerRegistration,
    ) -> (Self, BrowserResourceRuntimeBinding) {
        let root_id = NEXT_BROWSER_RESOURCE_OWNER_ROOT_ID
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let (runtime, owner) = initial.into_parts(root_id, Arc::default());
        let binding = BrowserResourceRuntimeBinding::new(root_id, runtime);
        let owners = BrowserResourceRuntimeOwnerSet::new(root_id, binding.clone(), owner);
        (
            Self {
                inner: Rc::new(RefCell::new(owners)),
            },
            binding,
        )
    }

    pub(crate) fn registrar(&self) -> BrowserResourceRuntimeOwnerRegistrar {
        let root_id = self.inner.borrow().root_id;
        BrowserResourceRuntimeOwnerRegistrar {
            root_id,
            inner: Rc::downgrade(&self.inner),
        }
    }

    #[cfg(test)]
    pub(crate) fn reap_retired(&self) -> Vec<FetchRuntimeJoinReport> {
        self.inner.borrow_mut().reap_retired()
    }

    pub fn shutdown_and_join(&self) {
        self.inner.borrow_mut().shutdown_and_join();
    }

    #[cfg(test)]
    pub(crate) fn shutdown_and_join_reports_for_testing(&self) -> Vec<FetchRuntimeJoinReport> {
        let mut owners = self.inner.borrow_mut();
        if owners.terminal && owners.owners.is_empty() {
            return Vec::new();
        }
        owners.terminal = true;
        owners.active_runtime_id = None;
        let reports = shutdown_and_join_resource_runtime_owners(&mut owners.owners);
        owners.owners.clear();
        reports
    }

    #[cfg(test)]
    pub fn owner_count_for_testing(&self) -> usize {
        self.inner.borrow().owner_count_for_testing()
    }
}

impl Drop for BrowserResourceRuntimeOwnerRoot {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

impl BrowserResourceRuntimeOwnerRegistrar {
    fn with_owner_set<T>(
        &self,
        operation: impl FnOnce(&BrowserResourceRuntimeOwnerSet) -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let Some(root) = self.inner.upgrade() else {
            return Err("browser resource runtime owner root has been dropped");
        };
        let owners = root.borrow();
        if self.root_id != owners.root_id {
            return Err("browser resource runtime registrar/root identity mismatch");
        }
        operation(&owners)
    }

    pub fn replace_owned(
        &self,
        registration: BrowserResourceRuntimeOwnerRegistration,
    ) -> Result<BrowserResourceRuntime, &'static str> {
        let Some(root) = self.inner.upgrade() else {
            return Err("browser resource runtime owner root has been dropped");
        };
        let mut owners = root.borrow_mut();
        if self.root_id != owners.root_id {
            return Err("browser resource runtime registrar/root identity mismatch");
        }
        owners.replace_owned(registration)
    }

    pub(crate) fn adopt_registered(
        &self,
        runtime: BrowserResourceRuntime,
    ) -> Result<(), &'static str> {
        let Some(root) = self.inner.upgrade() else {
            return Err("browser resource runtime owner root has been dropped");
        };
        let mut owners = root.borrow_mut();
        if self.root_id != owners.root_id {
            return Err("browser resource runtime registrar/root identity mismatch");
        }
        owners.adopt_registered(runtime)
    }

    pub(crate) fn current_registered(&self) -> Result<BrowserResourceRuntime, &'static str> {
        let Some(root) = self.inner.upgrade() else {
            return Err("browser resource runtime owner root has been dropped");
        };
        let mut owners = root.borrow_mut();
        if self.root_id != owners.root_id {
            return Err("browser resource runtime registrar/root identity mismatch");
        }
        owners.current_registered()
    }

    pub(crate) fn validate_registered(
        &self,
        runtime: &BrowserResourceRuntime,
    ) -> Result<(), &'static str> {
        self.with_owner_set(|owners| owners.validate_registered(runtime))
    }

    pub fn reap_retired(&self) {
        if let Some(root) = self.inner.upgrade() {
            let _ = root.borrow_mut().reap_retired();
        }
    }
}

impl Drop for BrowserResourceRuntimeOwnerSet {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

impl Drop for BrowserResourceRuntimeInner {
    fn drop(&mut self) {
        self.client.request_shutdown();
    }
}

fn shutdown_and_join_resource_runtime_owners(
    owners: &mut [BrowserResourceRuntimeOwner],
) -> Vec<FetchRuntimeJoinReport> {
    for owner in owners.iter() {
        owner.request_shutdown();
    }
    owners
        .iter_mut()
        .map(BrowserResourceRuntimeOwner::join)
        .collect()
}

impl fmt::Debug for BrowserResourceRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserResourceRuntime")
            .field("id", &self.inner.id)
            .field("client", &self.inner.client)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier, mpsc as std_mpsc},
        time::Duration,
    };

    use anyhow::Result;
    use moli_cookie_jar::new_shared_browser_cookie_store;
    use moli_fetch::{FetchCancelHandle, RawResponse, Request, RequestResourceType, ResponseHead};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
        time::timeout,
    };

    use super::*;
    use crate::network::{
        ResourceRequestClient,
        backend::{RawSubresourceCacheKey, raw_subresource_memory_cache_key},
    };

    fn registration() -> BrowserResourceRuntimeOwnerRegistration {
        BrowserResourceRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store())
    }

    fn cache_stylesheet(runtime: &BrowserResourceRuntime) -> RawSubresourceCacheKey {
        let request = Request::get("https://cache.test/retained.css")
            .unwrap()
            .with_page_network_policy()
            .with_resource_type(RequestResourceType::CssStyleSheet);
        let key = raw_subresource_memory_cache_key(&request).unwrap();
        let response = RawResponse::from_head_and_body(
            ResponseHead {
                final_url: request.url,
                status: 200,
                headers: vec![("cache-control".to_owned(), "max-age=60".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            b"body { color: red }".to_vec(),
        );
        runtime.memory_cache().lock().insert_raw_subresource(
            key.clone(),
            response,
            u64::MAX,
            Vec::new(),
        );
        key
    }

    #[test]
    fn transport_replacement_retains_context_cache_without_retaining_old_owner() {
        let cookie_store = new_shared_browser_cookie_store();
        let mut config = FetchConfig::default();
        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(
            BrowserResourceRuntimeOwner::new(&config, cookie_store.clone()),
        );
        let original = binding.current();
        let key = cache_stylesheet(&original);
        let diagnostics = original.memory_cache_diagnostics();

        // A new transport/config must not recreate the Context's cache or
        // multiply its budget. The entry has no Vary dependency on this UA.
        config.set_user_agent("Moli/Replaced-Transport");
        let replacement = root
            .registrar()
            .replace_owned(BrowserResourceRuntimeOwner::new(&config, cookie_store))
            .unwrap();
        assert!(!original.shares_state_with(&replacement));
        assert!(replacement.matches_fetch_config(&config));
        assert!(binding.current().shares_state_with(&replacement));
        assert!(std::ptr::eq(
            original.memory_cache(),
            replacement.memory_cache(),
        ));
        assert_eq!(replacement.memory_cache_diagnostics(), diagnostics);
        assert_eq!(root.owner_count_for_testing(), 2);

        drop(original);
        let retired = root.reap_retired();
        assert_eq!(retired.len(), 1);
        assert!(retired[0].is_clean());
        assert_eq!(root.owner_count_for_testing(), 1);
        assert_eq!(
            replacement
                .memory_cache()
                .lock()
                .lookup_raw_subresource(&key, |vary| vary.is_empty())
                .unwrap()
                .body_bytes(),
            b"body { color: red }",
        );
    }

    #[test]
    fn transport_replacement_with_different_storage_does_not_share_cache() {
        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let original = binding.current();
        let key = cache_stylesheet(&original);
        let replacement = root.registrar().replace_owned(registration()).unwrap();

        assert!(!std::ptr::eq(
            original.memory_cache(),
            replacement.memory_cache(),
        ));
        assert_eq!(replacement.memory_cache_diagnostics().entry_count, 0);
        assert!(
            replacement
                .memory_cache()
                .lock()
                .lookup_raw_subresource(&key, |vary| vary.is_empty())
                .is_none()
        );
        assert!(
            original
                .memory_cache()
                .lock()
                .lookup_raw_subresource(&key, |vary| vary.is_empty())
                .is_some()
        );
    }

    #[test]
    fn separate_owner_roots_do_not_share_cache_even_with_same_cookie_store() {
        let cookie_store = new_shared_browser_cookie_store();
        let (_left_root, left) = BrowserResourceRuntimeOwnerRoot::new(
            BrowserResourceRuntimeOwner::new(&FetchConfig::default(), cookie_store.clone()),
        );
        let (_right_root, right) = BrowserResourceRuntimeOwnerRoot::new(
            BrowserResourceRuntimeOwner::new(&FetchConfig::default(), cookie_store),
        );
        let left = left.current();
        let right = right.current();
        let key = cache_stylesheet(&left);

        assert!(!std::ptr::eq(left.memory_cache(), right.memory_cache()));
        assert_eq!(right.memory_cache_diagnostics().entry_count, 0);
        assert!(
            right
                .memory_cache()
                .lock()
                .lookup_raw_subresource(&key, |vary| vary.is_empty())
                .is_none()
        );
    }

    #[test]
    fn retired_owner_reaps_after_last_external_runtime_lease_drops() {
        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let registrar = root.registrar();
        let old_runtime = binding.current();

        registrar
            .replace_owned(registration())
            .expect("replacement runtime should register on the same root");
        assert_eq!(root.owner_count_for_testing(), 2);

        drop(old_runtime);
        root.reap_retired();
        assert_eq!(root.owner_count_for_testing(), 1);

        root.shutdown_and_join();
        assert_eq!(root.owner_count_for_testing(), 0);
    }

    #[test]
    fn retired_owner_reaps_after_external_thread_drops_last_runtime_lease() {
        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let registrar = root.registrar();
        let old_runtime = binding.current();
        let drop_barrier = Arc::new(Barrier::new(2));
        let thread_barrier = Arc::clone(&drop_barrier);
        let external = std::thread::spawn(move || {
            thread_barrier.wait();
            drop(old_runtime);
        });

        registrar
            .replace_owned(registration())
            .expect("replacement runtime should register on the same root");
        assert_eq!(root.owner_count_for_testing(), 2);

        drop_barrier.wait();
        external.join().expect("external lease owner should exit");
        root.reap_retired();
        assert_eq!(root.owner_count_for_testing(), 1);

        root.shutdown_and_join();
        assert_eq!(root.owner_count_for_testing(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn semantic_callback_can_drop_last_retired_runtime_before_external_clean_join()
    -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (request_ready_tx, request_ready_rx) = oneshot::channel();
        let (release_response_tx, release_response_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept test request");
            read_request_head(&mut stream)
                .await
                .expect("read request headers");
            let _ = request_ready_tx.send(());
            let _ = release_response_rx.await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("write callback response");
        });

        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let registrar = root.registrar();
        let old_runtime = binding.current();
        let client = ResourceRequestClient::from_browser_resource_runtime(old_runtime.clone());
        let callback_runtime = old_runtime.clone();
        let (callback_tx, callback_rx) = std_mpsc::channel();
        client.fetch_text_callback(
            Request::get(&format!("http://{addr}/callback"))?,
            move |result| {
                let thread_name = std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .to_owned();
                let body = result.map(|response| response.body_text().to_owned());
                // This is the last strong request-side lease. Its Drop runs on
                // lm-fetch-semantics and may request shutdown, but the unique
                // JoinHandle remains at `root` on this external thread.
                drop(callback_runtime);
                callback_tx
                    .send((thread_name, body))
                    .expect("test should still wait for callback completion");
            },
        )?;
        request_ready_rx.await?;

        let replacement = registrar
            .replace_owned(registration())
            .expect("replacement runtime should register on the same root");
        drop(replacement);
        drop(client);
        drop(old_runtime);
        assert_eq!(root.owner_count_for_testing(), 2);

        release_response_tx
            .send(())
            .expect("release semantic callback response");
        let (thread_name, body) = callback_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("semantic callback should complete");
        assert_eq!(thread_name, "lm-fetch-semantics");
        assert_eq!(body?.as_str(), "ok");

        let retired_reports = root.reap_retired();
        assert_eq!(retired_reports.len(), 1);
        assert!(
            retired_reports[0].is_clean(),
            "retired semantic owner must join cleanly: {:?}",
            retired_reports[0]
        );
        assert_eq!(root.owner_count_for_testing(), 1);

        let active_reports = root.shutdown_and_join_reports_for_testing();
        assert_eq!(active_reports.len(), 1);
        assert!(
            active_reports[0].is_clean(),
            "active semantic owner must join cleanly: {:?}",
            active_reports[0]
        );
        assert_eq!(root.owner_count_for_testing(), 0);
        server.await?;
        Ok(())
    }

    async fn read_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).await? == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "client closed before request headers completed",
                ));
            }
            request.push(byte[0]);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn streaming_response_retains_exact_retired_runtime_until_finish() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (finish_body_tx, finish_body_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept test request");
            read_request_head(&mut stream)
                .await
                .expect("read request headers");
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: application/octet-stream\r\n",
                        "Transfer-Encoding: chunked\r\n",
                        "\r\n",
                        "3\r\n",
                        "old\r\n",
                    )
                    .as_bytes(),
                )
                .await
                .expect("write first body chunk");
            let _ = finish_body_rx.await;
            stream
                .write_all(b"3\r\nrun\r\n0\r\n\r\n")
                .await
                .expect("write terminal body chunk");
        });

        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let registrar = root.registrar();
        let old_runtime = binding.current();
        let client = ResourceRequestClient::from_browser_resource_runtime(old_runtime.clone());
        let mut response = client
            .fetch_raw_stream_with_cancel(
                Request::get(&format!("http://{addr}/stream"))?,
                FetchCancelHandle::new(),
            )
            .await?;

        registrar
            .replace_owned(registration())
            .expect("replacement runtime should register on the same root");
        drop(client);
        drop(old_runtime);
        root.reap_retired();
        assert_eq!(
            root.owner_count_for_testing(),
            2,
            "response body must retain the exact retired runtime after headers"
        );
        assert_eq!(response.next_chunk().await.as_deref(), Some(&b"old"[..]));

        finish_body_tx
            .send(())
            .expect("release terminal body chunk");
        assert_eq!(response.next_chunk().await.as_deref(), Some(&b"run"[..]));
        timeout(Duration::from_secs(3), response.finish()).await??;

        root.reap_retired();
        assert_eq!(
            root.owner_count_for_testing(),
            1,
            "terminal finish should release the runtime lease before response drop"
        );
        drop(response);
        root.shutdown_and_join();
        assert_eq!(root.owner_count_for_testing(), 0);
        server.await?;
        Ok(())
    }
}
