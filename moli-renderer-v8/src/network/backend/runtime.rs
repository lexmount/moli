use std::{
    cell::RefCell,
    fmt,
    marker::PhantomData,
    rc::{Rc, Weak as RcWeak},
    sync::{
        Arc, OnceLock, Weak as ArcWeak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use moli_cookie_jar::SharedBrowserCookieStore;
use moli_disk_pool::DiskPool;
use moli_fetch::{FetchClient, FetchClientHandle, FetchConfig, FetchRuntimeJoinReport};
use moli_parkable_image::{ParkableImageManager, ParkableImagePolicy};
use parking_lot::{Mutex, RwLock};

use super::{
    BrowserParkableImageDiagnostics, BrowserResourceDiskPoolDiagnostics,
    BrowserResourceRuntimeDiagnostics, CorsPreflightCache, SharedMemoryResourceCacheDiagnostics,
    memory_cache::SharedMemoryResourceCache,
};
use crate::network::loads::{
    DetachedKeepaliveLoadDiagnostics, DetachedKeepaliveLoadRegistry, ResourceLoadId,
    ResourceLoadKind,
};
use crate::network::task_runner::{RendererResourceTaskOwner, RendererResourceTaskRunner};

static NEXT_BROWSER_RESOURCE_RUNTIME_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_BROWSER_RESOURCE_OWNER_ROOT_ID: AtomicU64 = AtomicU64::new(0);

/// Long-lived transport and renderer memory-cache state for one browser context.
///
/// Clones intentionally share the libcurl/HTTP runtime, cookie store, and one
/// bounded renderer memory cache. It must not acquire mutable Page, Document,
/// Worker, or request-delivery state.
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
    runtime: BrowserResourceRuntime,
    owner: BrowserResourceRuntimeOwner,
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
    resource_task_owner: RendererResourceTaskOwner,
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
    owner_root_id: AtomicU64,
    client: FetchClientHandle,
    resource_task_runner: OnceLock<RendererResourceTaskRunner>,
    memory_cache: Mutex<SharedMemoryResourceCache>,
    cors_preflight_cache: Mutex<CorsPreflightCache>,
    disk_pool: Option<DiskPool>,
    parkable_images: ParkableImageManager,
    parkable_image_schedule_notify: Arc<tokio::sync::Notify>,
    parkable_image_scheduler_started: AtomicBool,
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
        let fetch_owner = FetchClient::new(config, cookie_store);
        let runtime_id = NEXT_BROWSER_RESOURCE_RUNTIME_ID
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let disk_pool = DiskPool::new(None).ok();
        let parkable_image_schedule_notify = Arc::new(tokio::sync::Notify::new());
        let schedule_notify = Arc::clone(&parkable_image_schedule_notify);
        let parkable_images = ParkableImageManager::new_with_schedule_wakeup(
            disk_pool.clone(),
            ParkableImagePolicy::default(),
            move || schedule_notify.notify_one(),
        );
        let runtime = BrowserResourceRuntime {
            inner: Arc::new(BrowserResourceRuntimeInner {
                id: runtime_id,
                owner_root_id: AtomicU64::new(0),
                client: fetch_owner.handle(),
                resource_task_runner: OnceLock::new(),
                memory_cache: Mutex::new(SharedMemoryResourceCache::default()),
                cors_preflight_cache: Mutex::new(CorsPreflightCache::default()),
                parkable_images,
                disk_pool,
                parkable_image_schedule_notify,
                parkable_image_scheduler_started: AtomicBool::new(false),
                detached_keepalive_loads: DetachedKeepaliveLoadRegistry::default(),
            }),
        };
        let owner = Self {
            runtime_id,
            runtime: Arc::downgrade(&runtime.inner),
            fetch_owner,
            _thread_affine: PhantomData,
        };
        BrowserResourceRuntimeOwnerRegistration {
            runtime,
            owner,
            _thread_affine: PhantomData,
        }
    }
}

impl BrowserResourceRuntime {
    pub(crate) fn resource_task_runner(&self) -> RendererResourceTaskRunner {
        self.inner
            .resource_task_runner
            .get()
            .expect("registered browser resource runtime must have an executor")
            .clone()
    }

    fn bind_owner_root(
        &self,
        root_id: u64,
        runner: RendererResourceTaskRunner,
    ) -> Result<(), &'static str> {
        match self.inner.owner_root_id.compare_exchange(
            0,
            root_id,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                self.inner
                    .resource_task_runner
                    .set(runner)
                    .expect("unregistered resource runtime cannot already have an executor");
                Ok(())
            }
            Err(existing)
                if existing == root_id
                    && self.resource_task_runner().shares_executor_with(&runner) =>
            {
                Ok(())
            }
            Err(_) => Err("browser resource runtime belongs to another owner root"),
        }
    }

    fn owner_root_id(&self) -> u64 {
        self.inner.owner_root_id.load(Ordering::Acquire)
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

    pub(crate) fn clear_cors_preflight_cache(&self, origin: Option<&url::Url>) {
        let mut cache = self.inner.cors_preflight_cache.lock();
        if let Some(origin) = origin {
            cache.clear_for_origin(origin);
        } else {
            cache.clear();
        }
    }

    pub(crate) fn cors_preflight_cache(&self) -> &Mutex<CorsPreflightCache> {
        &self.inner.cors_preflight_cache
    }

    pub fn disk_pool(&self) -> Option<DiskPool> {
        self.inner.disk_pool.clone()
    }

    pub fn disk_pool_diagnostics(&self) -> Option<BrowserResourceDiskPoolDiagnostics> {
        self.inner
            .disk_pool
            .as_ref()
            .map(|pool| pool.diagnostics().into())
    }

    pub fn parkable_image_manager(&self) -> ParkableImageManager {
        self.inner.parkable_images.clone()
    }

    pub fn parkable_image_diagnostics(&self) -> BrowserParkableImageDiagnostics {
        self.inner.parkable_images.diagnostics().into()
    }

    pub(crate) fn ensure_parkable_image_scheduler(
        &self,
        runner: &crate::network::RendererResourceTaskRunner,
    ) {
        if self
            .inner
            .parkable_image_scheduler_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let weak_runtime = Arc::downgrade(&self.inner);
        let schedule_notify = Arc::clone(&self.inner.parkable_image_schedule_notify);
        runner.spawn(run_parkable_image_scheduler(weak_runtime, schedule_notify));
    }

    pub fn diagnostics(&self) -> BrowserResourceRuntimeDiagnostics {
        BrowserResourceRuntimeDiagnostics {
            runtime_id: self.runtime_id_for_diagnostics(),
            memory_cache: self.memory_cache_diagnostics(),
            disk_pool: self.disk_pool_diagnostics(),
            parkable_images: self.parkable_image_diagnostics(),
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

async fn run_parkable_image_scheduler(
    weak_runtime: ArcWeak<BrowserResourceRuntimeInner>,
    schedule_notify: Arc<tokio::sync::Notify>,
) {
    loop {
        // Arm the notification before reading the deadline so a concurrent
        // state transition cannot be lost between inspection and waiting.
        let notified = schedule_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let Some(runtime) = weak_runtime.upgrade() else {
            return;
        };
        let next_deadline = runtime.parkable_images.next_parking_deadline();
        drop(runtime);

        match next_deadline {
            Some(deadline) => {
                let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                tokio::pin!(sleep);
                tokio::select! {
                    () = notified.as_mut() => continue,
                    () = sleep.as_mut() => {}
                }
            }
            None => {
                notified.await;
                continue;
            }
        }

        let Some(runtime) = weak_runtime.upgrade() else {
            return;
        };
        let manager = runtime.parkable_images.clone();
        drop(runtime);
        if tokio::task::spawn_blocking(move || manager.park_images())
            .await
            .is_err()
        {
            return;
        }
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
    fn into_parts(self) -> (BrowserResourceRuntime, BrowserResourceRuntimeOwner) {
        (self.runtime, self.owner)
    }
}

impl BrowserResourceRuntimeOwnerSet {
    fn new(
        root_id: u64,
        binding: BrowserResourceRuntimeBinding,
        owner: BrowserResourceRuntimeOwner,
        resource_task_owner: RendererResourceTaskOwner,
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
            resource_task_owner,
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
        let (runtime, owner) = registration.into_parts();
        runtime.bind_owner_root(self.root_id, self.resource_task_owner.runner())?;
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

    fn shutdown_and_join(&mut self) -> Vec<FetchRuntimeJoinReport> {
        if self.terminal && self.owners.is_empty() {
            return Vec::new();
        }
        self.terminal = true;
        self.active_runtime_id = None;
        for owner in &self.owners {
            owner.request_shutdown();
        }
        self.resource_task_owner.request_shutdown();
        let reports = shutdown_and_join_resource_runtime_owners(&mut self.owners);
        self.resource_task_owner.join();
        self.owners.clear();
        reports
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
        let (runtime, owner) = initial.into_parts();
        // This executor belongs to the browser-context root, rather than one
        // Fetch configuration generation. A live Document may retain its runner
        // while its transport is replaced or an old Fetch owner is reaped.
        let resource_task_owner = RendererResourceTaskOwner::new();
        runtime
            .bind_owner_root(root_id, resource_task_owner.runner())
            .expect("initial browser resource runtime must be unregistered");
        let binding = BrowserResourceRuntimeBinding::new(root_id, runtime);
        let owners = BrowserResourceRuntimeOwnerSet::new(
            root_id,
            binding.clone(),
            owner,
            resource_task_owner,
        );
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
        let _ = self.inner.borrow_mut().shutdown_and_join();
    }

    #[cfg(test)]
    pub(crate) fn shutdown_and_join_reports_for_testing(&self) -> Vec<FetchRuntimeJoinReport> {
        self.inner.borrow_mut().shutdown_and_join()
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
    pub(crate) fn clear_cors_preflight_cache(
        &self,
        origin: Option<&url::Url>,
    ) -> Result<(), &'static str> {
        self.with_owner_set(|owners| {
            if owners.terminal {
                return Err("browser resource runtime owner root is shut down");
            }
            // Workers and in-flight loads can retain retired network leases
            // after a proxy/config change. Clearing only the current binding
            // would leave their permissions observable after cache clearing.
            for owner in &owners.owners {
                if let Some(inner) = owner.runtime.upgrade() {
                    BrowserResourceRuntime { inner }.clear_cors_preflight_cache(origin);
                }
            }
            Ok(())
        })
    }

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
        let _ = self.shutdown_and_join();
    }
}

impl Drop for BrowserResourceRuntimeInner {
    fn drop(&mut self) {
        self.parkable_image_schedule_notify.notify_one();
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
    use moli_fetch::{FetchCancelHandle, Request};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
        time::timeout,
    };

    use super::*;
    use crate::network::ResourceRequestClient;

    fn registration() -> BrowserResourceRuntimeOwnerRegistration {
        BrowserResourceRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store())
    }

    #[test]
    fn resource_tasks_progress_without_polling_the_owner_and_cannot_outlive_shutdown() {
        struct PendingTask(std_mpsc::Sender<()>);
        impl Drop for PendingTask {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }

        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let runtime = binding.current();
        let runner = runtime.resource_task_runner();
        assert!(runner.shares_executor_with(&runtime.clone().resource_task_runner()));
        let (progress_tx, progress_rx) = std_mpsc::channel();
        let (dropped_tx, dropped_rx) = std_mpsc::channel();
        let job = runner.spawn_abortable(async move {
            let _pending = PendingTask(dropped_tx);
            tokio::time::sleep(Duration::from_millis(10)).await;
            progress_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });

        // There is no entered or driven Tokio runtime on this owner thread.
        progress_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("resource timers must progress while the owner is blocked");
        root.shutdown_and_join();
        dropped_rx
            .try_recv()
            .expect("joining the resource owner must drop pending futures");
        assert!(job.is_finished());

        let (late_tx, late_rx) = std_mpsc::channel();
        let late = runner.spawn_abortable(async move {
            let _ = late_tx.send(());
        });
        assert!(late.is_finished());
        assert_eq!(
            late_rx.try_recv(),
            Err(std_mpsc::TryRecvError::Disconnected)
        );
        root.shutdown_and_join();
    }

    #[test]
    fn fetch_runtime_replacement_keeps_the_browser_context_resource_executor() {
        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let old_runtime = binding.current();
        let runner = old_runtime.resource_task_runner();
        let replacement = root.registrar().replace_owned(registration()).unwrap();
        assert!(runner.shares_executor_with(&replacement.resource_task_runner()));

        drop(old_runtime);
        root.reap_retired();
        assert_eq!(root.owner_count_for_testing(), 1);
        let (completed_tx, completed_rx) = std_mpsc::channel();
        runner.spawn(async move {
            completed_tx.send(()).unwrap();
        });
        completed_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a live Document runner must survive replacement of its Fetch runtime");
        root.shutdown_and_join();
    }

    #[test]
    fn preflight_cache_clear_reaches_retired_leases_without_crossing_browser_contexts() {
        use crate::network::CorsPreflightResult;
        use moli_fetch::{Request, RequestCredentialsMode};
        use moli_url::WebOrigin;
        use std::time::Instant;

        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let registrar = root.registrar();
        let old = binding.current();
        let active = registrar.replace_owned(registration()).unwrap();
        assert_eq!(root.owner_count_for_testing(), 2);
        let (other_root, other_binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let unrelated = other_binding.current();
        let origin = url::Url::parse("http://origin.test/path").unwrap();
        let first = Request::new_browser_bytes(
            "PUT",
            "http://target.test/resource",
            None,
            Vec::new(),
            WebOrigin::from_url(&origin),
        )
        .unwrap()
        .with_credentials_mode(RequestCredentialsMode::Omit)
        .with_network_partition_key(Some("partition-a".to_owned()));
        let second = first
            .clone()
            .with_request_origin(WebOrigin::from_url(
                &url::Url::parse("http://other.test").unwrap(),
            ))
            .with_network_partition_key(Some("partition-b".to_owned()));
        let now = Instant::now();
        for runtime in [&old, &active, &unrelated] {
            for request in [&first, &second] {
                runtime.cors_preflight_cache().lock().insert(
                    request,
                    CorsPreflightResult::new(
                        RequestCredentialsMode::Omit,
                        Some(vec!["PUT".to_owned()]),
                        None,
                        &[],
                    ),
                    now,
                );
                assert!(
                    runtime
                        .cors_preflight_cache()
                        .lock()
                        .can_skip(request, &[], now)
                );
            }
        }

        registrar.clear_cors_preflight_cache(Some(&origin)).unwrap();
        for runtime in [&old, &active] {
            assert!(
                !runtime
                    .cors_preflight_cache()
                    .lock()
                    .can_skip(&first, &[], now)
            );
            assert!(
                runtime
                    .cors_preflight_cache()
                    .lock()
                    .can_skip(&second, &[], now)
            );
        }
        assert!(
            unrelated
                .cors_preflight_cache()
                .lock()
                .can_skip(&first, &[], now)
        );

        registrar.clear_cors_preflight_cache(None).unwrap();
        for runtime in [&old, &active] {
            assert!(
                !runtime
                    .cors_preflight_cache()
                    .lock()
                    .can_skip(&second, &[], now)
            );
        }
        assert!(
            unrelated
                .cors_preflight_cache()
                .lock()
                .can_skip(&second, &[], now)
        );
        root.shutdown_and_join();
        other_root.shutdown_and_join();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parkable_image_scheduler_runs_after_the_last_reader_release_grace() -> Result<()> {
        let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration());
        let runtime = binding.current();
        let runner = crate::network::RendererResourceTaskRunner::from_current_tokio()?;
        runtime.ensure_parkable_image_scheduler(&runner);
        let manager = runtime.parkable_image_manager();
        let image = manager.from_frozen_bytes(vec![7; 4096]);

        let snapshot = image.snapshot()?;
        drop(snapshot);
        timeout(Duration::from_secs(5), async {
            while manager.diagnostics().parked_count == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the reader-release deadline should wake the image scheduler");

        assert_eq!(manager.diagnostics().resident_count, 0);
        assert_eq!(manager.diagnostics().parked_count, 1);
        drop((image, manager, runtime, binding));
        root.shutdown_and_join();
        Ok(())
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
            Request::get(&format!("http://{addr}/callback"))?
                .with_request_origin(moli_url::WebOrigin::Opaque),
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
                Request::get(&format!("http://{addr}/stream"))?
                    .with_request_origin(moli_url::WebOrigin::Opaque),
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
