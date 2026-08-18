use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use moli_fetch::FetchCancelHandle;
use parking_lot::Mutex;

use crate::{
    network::{BrowserResourceRuntime, RendererResourceTaskRunner, ResourceRequestClient},
    types::SubresourceResourceType,
};

static NEXT_RESOURCE_LOAD_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_RESOURCE_LOAD_REGISTRY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResourceLoadId(u64);

impl ResourceLoadId {
    fn next() -> Self {
        Self(
            NEXT_RESOURCE_LOAD_ID
                .fetch_add(1, Ordering::Relaxed)
                .checked_add(1)
                .expect("resource load id exhausted"),
        )
    }

    fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceLoadDisposition {
    Ordinary,
    Keepalive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceLoadKind {
    Script,
    Stylesheet,
    Image,
    Font,
    Media,
    TextTrack,
    Fetch,
    Xhr,
    EventSource,
    Beacon,
    CspReport,
    WebSocket,
    Dictionary,
    Manifest,
}

impl From<SubresourceResourceType> for ResourceLoadKind {
    fn from(resource_type: SubresourceResourceType) -> Self {
        match resource_type {
            SubresourceResourceType::Script => Self::Script,
            SubresourceResourceType::Stylesheet => Self::Stylesheet,
            SubresourceResourceType::Image => Self::Image,
            SubresourceResourceType::Font => Self::Font,
            SubresourceResourceType::Audio
            | SubresourceResourceType::Video
            | SubresourceResourceType::Media => Self::Media,
            SubresourceResourceType::TextTrack => Self::TextTrack,
            SubresourceResourceType::Fetch => Self::Fetch,
            SubresourceResourceType::Xhr => Self::Xhr,
            SubresourceResourceType::EventSource => Self::EventSource,
            SubresourceResourceType::Ping => Self::Beacon,
            SubresourceResourceType::CspReport => Self::CspReport,
            SubresourceResourceType::WebSocket => Self::WebSocket,
            SubresourceResourceType::Dictionary => Self::Dictionary,
            SubresourceResourceType::Manifest => Self::Manifest,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceLoadRegistryDiagnostics {
    pub(crate) registry_id: u64,
    pub(crate) active_ordinary_load_count: usize,
    pub(crate) active_keepalive_load_count: usize,
    pub(crate) detached: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResourceLoadRetirement {
    pub(crate) ordinary_load_count: usize,
    pub(crate) keepalive_load_count: usize,
}

struct ResourceLoadRegistryState {
    detached: bool,
    loads: HashMap<ResourceLoadId, Weak<ResourceLoadRegistration>>,
}

struct ResourceLoadRegistryInner {
    id: u64,
    task_runner: RendererResourceTaskRunner,
    state: Mutex<ResourceLoadRegistryState>,
}

/// Context-owned registry for every load started by one Document or Worker.
///
/// The registry owns no V8 or DOM values. Async state keeps a
/// [`ResourceLoadLease`]; context retirement walks the weak registrations,
/// cancels ordinary transport, and transfers only explicitly marked keepalive
/// transport to the browser-runtime registry. It intentionally does not own a
/// browser runtime itself: a live context may replace its request client, while
/// each lease must retain the exact runtime captured when that request began.
#[derive(Clone)]
pub(crate) struct ResourceLoadRegistry {
    inner: Arc<ResourceLoadRegistryInner>,
}

impl ResourceLoadRegistry {
    pub(crate) fn new(task_runner: RendererResourceTaskRunner) -> Self {
        Self {
            inner: Arc::new(ResourceLoadRegistryInner {
                id: NEXT_RESOURCE_LOAD_REGISTRY_ID
                    .fetch_add(1, Ordering::Relaxed)
                    .checked_add(1)
                    .expect("resource load registry id exhausted"),
                task_runner,
                state: Mutex::new(ResourceLoadRegistryState {
                    detached: false,
                    loads: HashMap::new(),
                }),
            }),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.inner.id
    }

    pub(crate) fn task_runner(&self) -> RendererResourceTaskRunner {
        self.inner.task_runner.clone()
    }

    pub(crate) fn register(
        &self,
        kind: ResourceLoadKind,
        disposition: ResourceLoadDisposition,
        request_client: ResourceRequestClient,
        cancel_handle: Option<FetchCancelHandle>,
    ) -> Option<ResourceLoadLease> {
        let registration =
            self.new_registration(kind, disposition, request_client, cancel_handle, false);
        let mut state = self.inner.state.lock();
        if state.detached {
            return None;
        }
        state
            .loads
            .insert(registration.id, Arc::downgrade(&registration));
        Some(ResourceLoadLease { registration })
    }

    /// Registers a network-only keepalive spawned from an already detached
    /// request context.
    ///
    /// This record deliberately bypasses the context registry: it can be
    /// created while `begin_detach()` is transitioning the owner, and it must
    /// never regain a Document/Worker completion route.
    pub(crate) fn register_detached_keepalive(
        &self,
        kind: ResourceLoadKind,
        request_client: ResourceRequestClient,
        cancel_handle: Option<FetchCancelHandle>,
    ) -> ResourceLoadLease {
        let registration = self.new_registration(
            kind,
            ResourceLoadDisposition::Keepalive,
            request_client,
            cancel_handle,
            true,
        );
        let cancel_handle = registration.lifecycle.lock().cancel_handle.clone();
        registration
            .browser_runtime
            .register_detached_keepalive_load(registration.id, registration.kind, cancel_handle);
        ResourceLoadLease { registration }
    }

    fn new_registration(
        &self,
        kind: ResourceLoadKind,
        disposition: ResourceLoadDisposition,
        request_client: ResourceRequestClient,
        cancel_handle: Option<FetchCancelHandle>,
        detached_keepalive: bool,
    ) -> Arc<ResourceLoadRegistration> {
        let id = ResourceLoadId::next();
        let browser_runtime = request_client.browser_resource_runtime();
        Arc::new(ResourceLoadRegistration {
            id,
            kind,
            disposition,
            request_client,
            browser_runtime,
            task_runner: self.inner.task_runner.clone(),
            lifecycle: Mutex::new(ResourceLoadLifecycle {
                cancel_handle,
                consumer_cancel: None,
                detached_keepalive,
                registry: Arc::downgrade(&self.inner),
            }),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        })
    }

    /// Seals this registry and transfers its existing loads to a replacement
    /// Document authority without changing their captured request policy.
    ///
    /// `document.open()` replaces Moli's exact Document owner
    /// while preserving the LocalWindow and its active fetch group. New loads
    /// must use the replacement Document's policy, but already-started Fetch,
    /// XHR, image, and WebSocket work must remain cancellable with that same
    /// LocalWindow. Moving only the lifecycle registration preserves both
    /// requirements: request clients and completion state stay untouched.
    pub(crate) fn transfer_existing_loads_to(&self, replacement: &Self) -> usize {
        assert_ne!(
            self.id(),
            replacement.id(),
            "resource loads require a distinct replacement registry"
        );
        assert!(
            self.inner
                .task_runner
                .shares_executor_with(&replacement.inner.task_runner),
            "replacement registry must share the source resource task runner"
        );

        let registrations = {
            let mut source = self.inner.state.lock();
            assert!(
                !source.detached,
                "only an active resource registry can be superseded"
            );
            source.detached = true;
            source
                .loads
                .drain()
                .filter_map(|(_, registration)| registration.upgrade())
                .collect::<Vec<_>>()
        };

        let replacement_registry = Arc::downgrade(&replacement.inner);
        let mut transferred = Vec::with_capacity(registrations.len());
        for registration in registrations {
            if registration.finished.load(Ordering::Acquire)
                || registration.cancelled.load(Ordering::Acquire)
            {
                continue;
            }
            {
                let mut lifecycle = registration.lifecycle.lock();
                if registration.finished.load(Ordering::Acquire)
                    || registration.cancelled.load(Ordering::Acquire)
                {
                    continue;
                }
                lifecycle.registry = replacement_registry.clone();
            }
            transferred.push(registration);
        }

        // Never hold the target registry lock while acquiring a load's
        // lifecycle lock. Completion and cancellation acquire those locks in
        // the opposite order (lifecycle, then registry) when removing
        // themselves. Publishing in a second phase keeps that order acyclic.
        let mut target = replacement.inner.state.lock();
        assert!(
            !target.detached,
            "resource loads cannot transfer into a detached registry"
        );
        let mut published = 0;
        for registration in transferred {
            if registration.finished.load(Ordering::Acquire)
                || registration.cancelled.load(Ordering::Acquire)
            {
                continue;
            }
            target
                .loads
                .insert(registration.id, Arc::downgrade(&registration));
            published += 1;
        }
        published
    }

    pub(crate) fn begin_detach(&self) -> ResourceLoadRetirement {
        let registrations = {
            let mut state = self.inner.state.lock();
            if state.detached {
                return ResourceLoadRetirement::default();
            }
            state.detached = true;
            let registrations = state
                .loads
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>();
            state.loads.clear();
            registrations
        };

        let mut retirement = ResourceLoadRetirement::default();
        for registration in registrations {
            match registration.disposition {
                ResourceLoadDisposition::Ordinary => {
                    retirement.ordinary_load_count += 1;
                    registration.cancel();
                }
                ResourceLoadDisposition::Keepalive => {
                    retirement.keepalive_load_count += 1;
                    registration.detach_keepalive();
                }
            }
        }
        retirement
    }

    pub(crate) fn diagnostics(&self) -> ResourceLoadRegistryDiagnostics {
        let mut state = self.inner.state.lock();
        state
            .loads
            .retain(|_, registration| registration.strong_count() > 0);
        let mut diagnostics = ResourceLoadRegistryDiagnostics {
            registry_id: self.id(),
            detached: state.detached,
            ..ResourceLoadRegistryDiagnostics::default()
        };
        for registration in state.loads.values().filter_map(Weak::upgrade) {
            match registration.disposition {
                ResourceLoadDisposition::Ordinary => {
                    diagnostics.active_ordinary_load_count += 1;
                }
                ResourceLoadDisposition::Keepalive => {
                    diagnostics.active_keepalive_load_count += 1;
                }
            }
        }
        diagnostics
    }
}

struct ResourceLoadLifecycle {
    cancel_handle: Option<FetchCancelHandle>,
    consumer_cancel: Option<Box<dyn FnOnce() + Send + 'static>>,
    detached_keepalive: bool,
    registry: Weak<ResourceLoadRegistryInner>,
}

struct ResourceLoadRegistration {
    id: ResourceLoadId,
    kind: ResourceLoadKind,
    disposition: ResourceLoadDisposition,
    request_client: ResourceRequestClient,
    browser_runtime: BrowserResourceRuntime,
    task_runner: RendererResourceTaskRunner,
    lifecycle: Mutex<ResourceLoadLifecycle>,
    cancelled: AtomicBool,
    finished: AtomicBool,
}

impl ResourceLoadRegistration {
    fn cancel(&self) {
        if self.finished.load(Ordering::Acquire) || self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let (cancel_handle, consumer_cancel, detached_keepalive, registry) = {
            let mut lifecycle = self.lifecycle.lock();
            (
                lifecycle.cancel_handle.take(),
                lifecycle.consumer_cancel.take(),
                lifecycle.detached_keepalive,
                lifecycle.registry.clone(),
            )
        };
        if let Some(cancel_handle) = cancel_handle {
            cancel_handle.cancel();
        }
        if let Some(consumer_cancel) = consumer_cancel {
            consumer_cancel();
        }
        if let Some(registry) = registry.upgrade() {
            registry.state.lock().loads.remove(&self.id);
        }
        if detached_keepalive {
            self.browser_runtime.remove_detached_keepalive_load(self.id);
        }
    }

    fn detach_keepalive(&self) {
        if self.finished.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let cancel_handle = {
            let mut lifecycle = self.lifecycle.lock();
            if lifecycle.detached_keepalive {
                return;
            }
            lifecycle.detached_keepalive = true;
            lifecycle.consumer_cancel.take();
            lifecycle.cancel_handle.clone()
        };
        self.browser_runtime
            .register_detached_keepalive_load(self.id, self.kind, cancel_handle);
        // `finish()` or `cancel()` can win after the lifecycle flag is stored
        // but before the detached registry insertion. Recheck after insertion
        // so that either interleaving removes the browser-owned tail.
        if self.finished.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire) {
            self.browser_runtime.remove_detached_keepalive_load(self.id);
        }
    }

    fn finish(&self) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        let (detached_keepalive, registry) = {
            let mut lifecycle = self.lifecycle.lock();
            lifecycle.consumer_cancel.take();
            (lifecycle.detached_keepalive, lifecycle.registry.clone())
        };
        if let Some(registry) = registry.upgrade() {
            registry.state.lock().loads.remove(&self.id);
        }
        if detached_keepalive {
            self.browser_runtime.remove_detached_keepalive_load(self.id);
        }
    }
}

impl Drop for ResourceLoadRegistration {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Movable request authority retained through pending, running, response
/// interception, authentication and streaming states.
///
/// Clones represent the same logical consumer. Context retirement invalidates
/// all clones at once; the last clone completing removes the registry entry.
#[derive(Clone)]
pub(crate) struct ResourceLoadLease {
    registration: Arc<ResourceLoadRegistration>,
}

impl ResourceLoadLease {
    pub(crate) fn id_for_diagnostics(&self) -> u64 {
        self.registration.id.as_u64()
    }

    pub(crate) fn registry_id(&self) -> u64 {
        self.registration
            .lifecycle
            .lock()
            .registry
            .upgrade()
            .map_or(0, |registry| registry.id)
    }

    pub(crate) fn kind(&self) -> ResourceLoadKind {
        self.registration.kind
    }

    pub(crate) fn disposition(&self) -> ResourceLoadDisposition {
        self.registration.disposition
    }

    pub(crate) fn request_client(&self) -> ResourceRequestClient {
        self.registration.request_client.clone()
    }

    pub(crate) fn task_runner(&self) -> RendererResourceTaskRunner {
        self.registration.task_runner.clone()
    }

    pub(crate) fn network_offline(&self) -> bool {
        self.registration
            .request_client
            .page_network_policy()
            .snapshot()
            .network_offline()
    }

    pub(crate) fn blocks_url(&self, url: &url::Url) -> bool {
        self.registration
            .request_client
            .page_network_policy()
            .snapshot()
            .blocks_url(url)
    }

    pub(crate) fn attach_cancel_handle(&self, cancel_handle: FetchCancelHandle) {
        let (cancel_immediately, detached_keepalive) = {
            let mut lifecycle = self.registration.lifecycle.lock();
            if self.registration.cancelled.load(Ordering::Acquire)
                || self.registration.finished.load(Ordering::Acquire)
            {
                (true, false)
            } else {
                lifecycle.cancel_handle = Some(cancel_handle.clone());
                (false, lifecycle.detached_keepalive)
            }
        };
        if cancel_immediately {
            cancel_handle.cancel();
            return;
        }
        if detached_keepalive {
            self.registration
                .browser_runtime
                .attach_detached_keepalive_cancel_handle(self.registration.id, cancel_handle);
        }
    }

    pub(crate) fn attach_consumer_cancel(&self, consumer_cancel: impl FnOnce() + Send + 'static) {
        let mut consumer_cancel = Some(consumer_cancel);
        let cancel_immediately = {
            let mut lifecycle = self.registration.lifecycle.lock();
            if self.registration.cancelled.load(Ordering::Acquire)
                || self.registration.finished.load(Ordering::Acquire)
            {
                true
            } else {
                assert!(
                    lifecycle.consumer_cancel.is_none(),
                    "resource load may own only one context consumer cancellation hook"
                );
                lifecycle.consumer_cancel = Some(Box::new(
                    consumer_cancel.take().expect("consumer cancellation hook"),
                ));
                false
            }
        };
        if cancel_immediately {
            consumer_cancel.take().expect("consumer cancellation hook")();
        }
    }

    pub(crate) fn cancel(&self) {
        self.registration.cancel();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.registration.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn response_completion_is_committed(&self) -> bool {
        self.registration
            .lifecycle
            .lock()
            .cancel_handle
            .as_ref()
            .is_some_and(FetchCancelHandle::response_completion_is_committed)
    }

    pub(crate) fn is_detached_keepalive(&self) -> bool {
        self.registration.lifecycle.lock().detached_keepalive
    }

    pub(crate) fn finish(&self) {
        self.registration.finish();
    }
}

impl fmt::Debug for ResourceLoadLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceLoadLease")
            .field("id", &self.registration.id)
            .field("registry_id", &self.registry_id())
            .field("kind", &self.kind())
            .field("disposition", &self.disposition())
            .field("cancelled", &self.is_cancelled())
            .field("detached_keepalive", &self.is_detached_keepalive())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_fetch::FetchConfig;

    struct TestResourceLoadRegistry {
        registry: ResourceLoadRegistry,
        request_client_owner: crate::network::ResourceRequestClientOwner,
    }

    impl std::ops::Deref for TestResourceLoadRegistry {
        type Target = ResourceLoadRegistry;

        fn deref(&self) -> &Self::Target {
            &self.registry
        }
    }

    fn registry() -> TestResourceLoadRegistry {
        TestResourceLoadRegistry {
            registry: ResourceLoadRegistry::new(
                RendererResourceTaskRunner::from_current_tokio()
                    .expect("resource-load registry test must own a Tokio runtime"),
            ),
            request_client_owner: ResourceRequestClient::new(&FetchConfig::default())
                .expect("test resource loader"),
        }
    }

    fn lease(
        registry: &TestResourceLoadRegistry,
        disposition: ResourceLoadDisposition,
        cancel_handle: FetchCancelHandle,
    ) -> ResourceLoadLease {
        registry
            .register(
                ResourceLoadKind::Fetch,
                disposition,
                registry.request_client_owner.handle(),
                Some(cancel_handle),
            )
            .expect("active registry should accept loads")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn document_retirement_cancels_ordinary_loads() {
        let registry = registry();
        let cancel_handle = FetchCancelHandle::new();
        let lease = lease(
            &registry,
            ResourceLoadDisposition::Ordinary,
            cancel_handle.clone(),
        );

        assert_eq!(
            registry.begin_detach(),
            ResourceLoadRetirement {
                ordinary_load_count: 1,
                keepalive_load_count: 0,
            }
        );
        assert!(cancel_handle.is_cancelled());
        assert!(lease.is_cancelled());
        assert!(
            registry
                .register(
                    ResourceLoadKind::Fetch,
                    ResourceLoadDisposition::Ordinary,
                    lease.request_client(),
                    None,
                )
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn document_open_transfers_existing_loads_without_rebinding_request_state() {
        let source = registry();
        let replacement = ResourceLoadRegistry::new(source.task_runner());
        let ordinary_cancel = FetchCancelHandle::new();
        let keepalive_cancel = FetchCancelHandle::new();
        let ordinary = lease(
            &source,
            ResourceLoadDisposition::Ordinary,
            ordinary_cancel.clone(),
        );
        let keepalive = lease(
            &source,
            ResourceLoadDisposition::Keepalive,
            keepalive_cancel.clone(),
        );
        let source_registry_id = source.id();
        let replacement_registry_id = replacement.id();

        assert_eq!(source.transfer_existing_loads_to(&replacement), 2);
        assert_eq!(ordinary.registry_id(), replacement_registry_id);
        assert_eq!(keepalive.registry_id(), replacement_registry_id);
        assert_ne!(source_registry_id, replacement_registry_id);
        assert!(!ordinary_cancel.is_cancelled());
        assert!(!keepalive_cancel.is_cancelled());
        assert!(!keepalive.is_detached_keepalive());
        assert_eq!(
            replacement.diagnostics(),
            ResourceLoadRegistryDiagnostics {
                registry_id: replacement_registry_id,
                active_ordinary_load_count: 1,
                active_keepalive_load_count: 1,
                detached: false,
            }
        );
        assert_eq!(
            source.begin_detach(),
            ResourceLoadRetirement::default(),
            "the superseded source registry must already be sealed and empty"
        );

        assert_eq!(
            replacement.begin_detach(),
            ResourceLoadRetirement {
                ordinary_load_count: 1,
                keepalive_load_count: 1,
            }
        );
        assert!(ordinary_cancel.is_cancelled());
        assert!(!keepalive_cancel.is_cancelled());
        assert!(keepalive.is_detached_keepalive());
        keepalive.finish();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn keepalive_transfers_without_retaining_context_registry() {
        let registry = registry();
        let cancel_handle = FetchCancelHandle::new();
        let lease = lease(
            &registry,
            ResourceLoadDisposition::Keepalive,
            cancel_handle.clone(),
        );
        let runtime = lease.request_client().browser_resource_runtime();

        assert_eq!(
            registry.begin_detach(),
            ResourceLoadRetirement {
                ordinary_load_count: 0,
                keepalive_load_count: 1,
            }
        );
        assert!(!cancel_handle.is_cancelled());
        assert!(lease.is_detached_keepalive());
        assert_eq!(
            runtime.detached_keepalive_diagnostics().active_load_count,
            1
        );
        lease.finish();
        assert_eq!(
            runtime.detached_keepalive_diagnostics().active_load_count,
            0
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_abort_still_cancels_detached_keepalive() {
        let registry = registry();
        let cancel_handle = FetchCancelHandle::new();
        let lease = lease(
            &registry,
            ResourceLoadDisposition::Keepalive,
            cancel_handle.clone(),
        );
        registry.begin_detach();

        lease.cancel();

        assert!(cancel_handle.is_cancelled());
        assert_eq!(
            lease
                .request_client()
                .browser_resource_runtime()
                .detached_keepalive_diagnostics()
                .active_load_count,
            0
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn late_transport_handle_attaches_to_detached_keepalive() {
        let registry = registry();
        let initial_cancel = FetchCancelHandle::new();
        let lease = lease(
            &registry,
            ResourceLoadDisposition::Keepalive,
            initial_cancel,
        );
        let runtime = lease.request_client().browser_resource_runtime();
        registry.begin_detach();

        let continued_cancel = FetchCancelHandle::new();
        lease.attach_cancel_handle(continued_cancel.clone());

        assert!(!continued_cancel.is_cancelled());
        assert_eq!(
            runtime.detached_keepalive_diagnostics().active_load_count,
            1
        );
        lease.finish();
        assert_eq!(
            runtime.detached_keepalive_diagnostics().active_load_count,
            0
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transport_handle_attached_after_abort_is_cancelled_immediately() {
        let registry = registry();
        let initial_cancel = FetchCancelHandle::new();
        let lease = lease(&registry, ResourceLoadDisposition::Ordinary, initial_cancel);
        registry.begin_detach();

        let late_cancel = FetchCancelHandle::new();
        lease.attach_cancel_handle(late_cancel.clone());

        assert!(late_cancel.is_cancelled());
    }
}
