use super::{
    inspector::{
        DocumentInspectorBinding, RendererInspectorIsolateBackend,
        RendererInspectorIsolateBackendHandle,
    },
    runtime_bindings::{
        PromiseRejectDispatchSlot, current_thread_promise_reaction_before_count,
        failed_access_check_callback, promise_activity_and_trace_hook, promise_reject_callback,
    },
};
use crate::{
    context_bootstrap::ContextBootstrapAssets,
    devtools::target::{
        RendererDevToolsTargetShutdownRegistration, RendererDevToolsTargetShutdownRegistry,
    },
    document_runtime::DocumentRuntime,
    exception_reporting::v8_message_listener,
    module_runtime::{
        dynamic_import_callback, dynamic_import_with_phase_callback,
        initialize_import_meta_object_callback,
    },
    native_bridge::bindings::NativeBridgeBindings,
    native_bridge::{PageScriptHost, SharedPrebootstrappedChildDefaultContexts},
    page_task_queue::{
        PageRuntimeTaskSource, PageRuntimeWakeSender, PageTaskSender,
        RendererPageV8ForegroundTaskSender,
    },
    resource_owner::ResourceOwnerId,
    runtime::RendererPageContextCancelSender,
    v8_platform::{V8ForegroundTaskWake, V8PlatformIsolateRegistration},
};
use anyhow::{Result, anyhow};
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

static DOCUMENT_ISOLATE_CREATED_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_DESTROYED_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_LIVE_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_RESERVED_COUNT: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[path = "document_isolate_foreground_tests.rs"]
mod foreground_tests;

#[derive(Clone, Debug, Default)]
pub(crate) struct RendererDeferredContextHostReleaseQueue {
    inner: Rc<RendererDeferredContextHostReleaseQueueInner>,
}

struct RendererDeferredContextHostRelease {
    _host: Option<Rc<RefCell<PageScriptHost>>>,
    retained_v8_handle_state: Vec<Box<dyn std::any::Any>>,
}

impl std::fmt::Debug for RendererDeferredContextHostRelease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererDeferredContextHostRelease")
            .field(
                "retained_v8_handle_state_count",
                &self.retained_v8_handle_state.len(),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct RendererDeferredContextHostReleaseQueueInner {
    pending: RefCell<Vec<RendererDeferredContextHostRelease>>,
    pending_realm_closes: RefCell<Vec<super::WindowRealmRuntimeState>>,
    isolate_shutting_down: Cell<bool>,
}

impl RendererDeferredContextHostReleaseQueue {
    pub(super) fn defer_window_realm_close(&self, realm: super::WindowRealmRuntimeState) {
        if self.inner.isolate_shutting_down.get() {
            drop(realm);
            return;
        }
        self.inner.pending_realm_closes.borrow_mut().push(realm);
    }

    pub(crate) fn defer(
        &self,
        host: Rc<RefCell<PageScriptHost>>,
        retained_v8_handle_state: Vec<Box<dyn std::any::Any>>,
    ) {
        self.defer_release(RendererDeferredContextHostRelease {
            _host: Some(host),
            retained_v8_handle_state,
        });
    }

    pub(crate) fn defer_v8_handle_state(
        &self,
        retained_v8_handle_state: Vec<Box<dyn std::any::Any>>,
    ) {
        self.defer_release(RendererDeferredContextHostRelease {
            _host: None,
            retained_v8_handle_state,
        });
    }

    fn defer_release(&self, release: RendererDeferredContextHostRelease) {
        if self.inner.isolate_shutting_down.get() {
            drop(release);
            return;
        }
        self.inner.pending.borrow_mut().push(release);
    }

    fn drain_on_entered_isolate(&self, isolate: &mut v8::OwnedIsolate) {
        loop {
            let realms = std::mem::take(&mut *self.inner.pending_realm_closes.borrow_mut());
            let pending = std::mem::take(&mut *self.inner.pending.borrow_mut());
            if realms.is_empty() && pending.is_empty() {
                return;
            }
            for realm in realms {
                realm.close_in_entered_isolate(isolate);
            }
            drop(pending);
        }
    }

    fn begin_isolate_shutdown(&self, isolate: &mut v8::OwnedIsolate) {
        self.drain_on_entered_isolate(isolate);
        self.inner.isolate_shutting_down.set(true);
    }
}

pub(crate) fn renderer_document_isolate_accounting_diagnostics()
-> crate::runtime::RendererDocumentIsolateAccountingDiagnostics {
    crate::runtime::RendererDocumentIsolateAccountingDiagnostics {
        created: DOCUMENT_ISOLATE_CREATED_COUNT.load(Ordering::Relaxed),
        destroyed: DOCUMENT_ISOLATE_DESTROYED_COUNT.load(Ordering::Relaxed),
        live: DOCUMENT_ISOLATE_LIVE_COUNT.load(Ordering::Relaxed),
        reserved: DOCUMENT_ISOLATE_RESERVED_COUNT.load(Ordering::Relaxed),
    }
}

#[derive(Debug)]
pub(crate) struct RendererDocumentIsolateReservationAccounting;

impl RendererDocumentIsolateReservationAccounting {
    pub(crate) fn new() -> Self {
        DOCUMENT_ISOLATE_RESERVED_COUNT.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for RendererDocumentIsolateReservationAccounting {
    fn drop(&mut self) {
        let previous = DOCUMENT_ISOLATE_RESERVED_COUNT.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "document isolate reservation count underflow");
    }
}

struct RendererDocumentIsolateAccountingGuard;

impl RendererDocumentIsolateAccountingGuard {
    fn new() -> Self {
        DOCUMENT_ISOLATE_CREATED_COUNT.fetch_add(1, Ordering::Relaxed);
        DOCUMENT_ISOLATE_LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for RendererDocumentIsolateAccountingGuard {
    fn drop(&mut self) {
        DOCUMENT_ISOLATE_DESTROYED_COUNT.fetch_add(1, Ordering::Relaxed);
        let previous = DOCUMENT_ISOLATE_LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "document isolate live count underflow");
    }
}

pub(super) struct ScriptVmPageRealmBootstrap {
    pub(super) initialize_main_world: bool,
    pub(super) inherited_security_token: Option<v8::Global<v8::Value>>,
    pub(super) resource_owner_id: ResourceOwnerId,
    pub(super) promise_reject_dispatch: PromiseRejectDispatchSlot,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) root_frame_id: Option<String>,
    pub(super) prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) post_domcontentloaded_page_task_tx: PageTaskSender,
    pub(super) page_runtime_wake_tx: PageRuntimeWakeSender,
    pub(super) storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) reuse_main_window_proxy: bool,
    // Partial bootstrap can drop before publication. Release bridge owners
    // before their native backing, and keep V8 alive until all of it
    // has been released.
    pub(super) context_host: Rc<RefCell<PageScriptHost>>,
    pub(super) document_runtime: Box<DocumentRuntime>,
    pub(super) renderer_document_isolate: RendererIsolateHandle,
}

pub(super) struct ScriptVmContextBootstrap {
    pub(super) runtime: super::WindowRealmRuntime,
}

impl std::ops::Deref for ScriptVmContextBootstrap {
    type Target = super::WindowRealmRuntimeState;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

pub(crate) struct RendererDocumentIsolateBootstrap {
    pub(crate) initial_document_environment: Option<super::ScriptVmInitialDocumentEnvironment>,
    pub(super) renderer_document_isolate: RendererIsolateHandle,
    pub(super) bridge_bindings: NativeBridgeBindings,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
}

impl RendererDocumentIsolateBootstrap {
    #[cfg(feature = "devtools")]
    pub(crate) fn inspector_endpoint(
        &self,
        runtime: crate::render_runtime::RenderRuntimeHandle,
        token: crate::runtime::RendererPageToken,
    ) -> crate::runtime::RendererPageInspectorEndpoint {
        crate::runtime::RendererPageInspectorEndpoint::new(
            runtime,
            token,
            self.page_inspector.agent_token(),
            self.page_inspector.devtools_target(),
        )
    }

    pub(crate) fn renderer_devtools_agent_token(
        &self,
    ) -> crate::runtime::RendererDevToolsAgentToken {
        self.page_inspector.agent_token()
    }

    pub(crate) fn clone_renderer_document_isolate_handle_for_owner_retention(
        &self,
    ) -> RendererIsolateHandle {
        self.renderer_document_isolate.clone()
    }

    pub(crate) fn renderer_page_script_environment(&self) -> Option<RendererPageScriptEnvironment> {
        self.renderer_page_script_environment.clone()
    }

    pub(crate) fn inspector_isolate_backend_handle(&self) -> RendererInspectorIsolateBackendHandle {
        self.page_inspector.isolate_backend_handle()
    }

    pub(crate) fn with_renderer_page_script_environment(
        mut self,
        environment: RendererPageScriptEnvironment,
    ) -> Self {
        self.renderer_page_script_environment = Some(environment);
        self
    }

    pub(crate) fn with_page_inspector(mut self, page_inspector: DocumentInspectorBinding) -> Self {
        self.page_inspector = page_inspector;
        self
    }
}

#[derive(Clone)]
pub(crate) struct RendererPageScriptEnvironment {
    inner: Rc<RendererPageScriptEnvironmentInner>,
}

#[derive(Clone)]
pub(crate) struct WeakRendererPageScriptEnvironment(Weak<RendererPageScriptEnvironmentInner>);

struct RendererPageScriptEnvironmentInner {
    page_id: u64,
    renderer_document_isolate: RendererIsolateHandle,
    inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
    page_runtime_task_source: PageRuntimeTaskSource,
    output_journal: crate::runtime::RendererTurnOutputJournal,
    global_proxy: OnceCell<v8::Global<v8::Object>>,
    auxiliary_allocator: OnceCell<crate::runtime::RendererAuxiliaryPageAllocator>,
    window_identity: crate::runtime::RendererPageWindowIdentity,
    opener: RefCell<Option<v8::Global<v8::Object>>>,
    document_host: RefCell<Weak<RefCell<PageScriptHost>>>,
    closed: Cell<bool>,
}

impl WeakRendererPageScriptEnvironment {
    pub(crate) fn upgrade(&self) -> Option<RendererPageScriptEnvironment> {
        self.0
            .upgrade()
            .map(|inner| RendererPageScriptEnvironment { inner })
    }
}

impl std::fmt::Debug for RendererPageScriptEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPageScriptEnvironment")
            .field("page_id", &self.inner.page_id)
            .field(
                "isolate_identity_key",
                &self.inner.renderer_document_isolate.identity_key(),
            )
            .field(
                "runtime_task_source_identity_key",
                &self.inner.page_runtime_task_source.identity_key(),
            )
            .field("output_stream", &self.inner.output_journal.stream())
            .field("has_global_proxy", &self.inner.global_proxy.get().is_some())
            .finish()
    }
}

impl RendererPageScriptEnvironment {
    pub(crate) fn downgrade(&self) -> WeakRendererPageScriptEnvironment {
        WeakRendererPageScriptEnvironment(Rc::downgrade(&self.inner))
    }

    pub(crate) fn bootstrap_related_page_document_isolate_in_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        source_bindings: &NativeBridgeBindings,
        sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let isolate_backend = self
            .inner
            .inspector_isolate_backend
            .new_page_handle(scope)?;
        self.inner
            .page_runtime_task_source
            .v8_foreground_task_sender()
            .ok_or_else(|| anyhow!("related Page creator has no foreground route"))?
            .isolate_membership()?
            .admit_related_page(sender)?;
        Ok(RendererDocumentIsolateBootstrap {
            initial_document_environment: None,
            renderer_document_isolate: self.inner.renderer_document_isolate.clone(),
            bridge_bindings: source_bindings.build_peer_in_scope(scope),
            renderer_document_isolate_teardown:
                RendererDocumentIsolateTeardown::owner_reserved_page(),
            page_inspector: DocumentInspectorBinding::new(isolate_backend),
            renderer_page_script_environment: None,
        })
    }

    pub(crate) fn new(
        page_id: u64,
        renderer_document_isolate: RendererIsolateHandle,
        inspector_isolate_backend: RendererInspectorIsolateBackendHandle,
        page_runtime_task_source: PageRuntimeTaskSource,
        output_journal: crate::runtime::RendererTurnOutputJournal,
        window_identity: crate::runtime::RendererPageWindowIdentity,
    ) -> Self {
        let registry = renderer_document_isolate.related_pages.clone();
        let environment = Self {
            inner: Rc::new(RendererPageScriptEnvironmentInner {
                page_id,
                renderer_document_isolate,
                inspector_isolate_backend,
                page_runtime_task_source,
                output_journal,
                global_proxy: OnceCell::new(),
                auxiliary_allocator: OnceCell::new(),
                window_identity,
                opener: RefCell::new(None),
                document_host: RefCell::new(Weak::new()),
                closed: Cell::new(false),
            }),
        };
        registry
            .borrow_mut()
            .insert(page_id, Rc::downgrade(&environment.inner));
        environment
    }

    pub(crate) fn bind_auxiliary_allocator(
        &self,
        allocator: crate::runtime::RendererAuxiliaryPageAllocator,
    ) {
        assert!(self.inner.auxiliary_allocator.set(allocator).is_ok());
    }

    pub(super) fn bind_document_host(&self, host: &Rc<RefCell<PageScriptHost>>) {
        *self.inner.document_host.borrow_mut() = Rc::downgrade(host);
    }

    pub(crate) fn enqueue_related_page_turn_completion(&self) {
        let host = self.inner.document_host.borrow().clone();
        self.inner
            .renderer_document_isolate
            .pending_page_turn_completions
            .borrow_mut()
            .insert(host.as_ptr() as usize, host);
    }

    pub(super) fn finish_related_page_turn_completions(&self) {
        let pending = std::mem::take(
            &mut *self
                .inner
                .renderer_document_isolate
                .pending_page_turn_completions
                .borrow_mut(),
        );
        for host in pending.into_values().filter_map(|host| host.upgrade()) {
            // Native callbacks already use this owner-thread host while V8 is
            // entered. Completion only publishes accepted facts and wakes the
            // Page's existing navigation continuation; it runs no Page tasks.
            unsafe { &*host.as_ptr() }.finish_related_page_turn_completion();
        }
    }

    pub(crate) fn auxiliary_allocator(
        &self,
    ) -> Option<crate::runtime::RendererAuxiliaryPageAllocator> {
        self.inner.auxiliary_allocator.get().cloned()
    }

    pub(crate) fn set_opener(&self, opener: Option<v8::Global<v8::Object>>) {
        *self.inner.opener.borrow_mut() = opener;
    }

    pub(crate) fn opener_in_scope<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.inner
            .opener
            .borrow()
            .as_ref()
            .map(|opener| v8::Local::new(scope, opener))
    }

    pub(crate) fn window_proxy_in_scope<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Result<v8::Local<'s, v8::Object>> {
        self.with_main_window_proxy(|proxy| v8::Local::new(scope, proxy))
    }

    pub(crate) fn named_related_window(
        &self,
        name: &str,
        initiator: &crate::runtime::RendererBrowsingContextName,
    ) -> Option<Self> {
        let mut pages = self
            .inner
            .renderer_document_isolate
            .related_pages
            .borrow_mut();
        pages.retain(|_, page| page.strong_count() != 0);
        let matches = |inner: &RendererPageScriptEnvironmentInner| {
            let current_name = inner.window_identity.name();
            (!inner.closed.get()
                && inner
                    .window_identity
                    .auxiliary_window()
                    .is_none_or(|window| !window.is_closed())
                && current_name.is_related_to(initiator)
                && current_name.get() == name)
                .then_some(())
        };
        if matches(&self.inner).is_some() {
            return Some(self.clone());
        }
        pages
            .values()
            .filter_map(Weak::upgrade)
            .find_map(|inner| matches(&inner).map(|()| Self { inner }))
    }

    pub(crate) fn related_document_hosts(&self) -> Vec<Rc<RefCell<PageScriptHost>>> {
        self.inner
            .renderer_document_isolate
            .related_pages
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .filter_map(|page| page.document_host.borrow().upgrade())
            .collect()
    }

    pub(crate) fn window_identity(&self) -> &crate::runtime::RendererPageWindowIdentity {
        &self.inner.window_identity
    }

    pub(crate) fn page_id(&self) -> u64 {
        self.inner.page_id
    }

    pub(crate) fn close_browsing_context(&self) {
        self.inner.closed.set(true);
    }

    pub(crate) fn browsing_context_is_closed(&self) -> bool {
        self.inner.closed.get()
    }

    pub(crate) fn page_runtime_task_source(&self) -> PageRuntimeTaskSource {
        self.inner.page_runtime_task_source.clone()
    }

    pub(crate) fn output_journal(&self) -> crate::runtime::RendererTurnOutputJournal {
        self.inner.output_journal.clone()
    }

    pub(crate) fn clear_page_runtime_tasks(&self) {
        self.inner.page_runtime_task_source.clear();
    }

    pub(crate) fn retire_output_stream(&self) {
        self.inner
            .output_journal
            .retire(crate::runtime::RendererOutputStreamCloseReason::ResidenceRetired);
    }

    pub(crate) fn isolate_identity_key(&self) -> usize {
        self.inner.renderer_document_isolate.identity_key()
    }

    pub(crate) fn capture_initial_document_environment(
        &self,
        environment: super::ScriptVmInitialDocumentEnvironment,
    ) -> super::ScriptVmCapturedDocumentEnvironment {
        super::ScriptVmCapturedDocumentEnvironment::new(
            environment,
            self.inner.renderer_document_isolate.clone(),
        )
    }

    pub(crate) fn bootstrap_replacement_document_isolate(
        &self,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let bridge_bindings = self
            .inner
            .renderer_document_isolate
            .build_bridge_bindings()?;
        let isolate_backend = self.inner.inspector_isolate_backend.clone();
        Ok(RendererDocumentIsolateBootstrap {
            initial_document_environment: None,
            renderer_document_isolate: self.inner.renderer_document_isolate.clone(),
            bridge_bindings,
            renderer_document_isolate_teardown:
                RendererDocumentIsolateTeardown::owner_reserved_page(),
            page_inspector: DocumentInspectorBinding::new(isolate_backend)
                .with_output_journal(self.output_journal()),
            renderer_page_script_environment: Some(self.clone()),
        })
    }

    pub(super) fn install_initial_main_window_proxy(
        &self,
        global_proxy: v8::Global<v8::Object>,
    ) -> Result<()> {
        self.inner
            .global_proxy
            .set(global_proxy)
            .map_err(|_| anyhow!("page script environment already retains its main WindowProxy"))
    }

    pub(super) fn has_main_window_proxy(&self) -> bool {
        self.inner.global_proxy.get().is_some()
    }

    pub(super) fn with_main_window_proxy<T>(
        &self,
        op: impl FnOnce(&v8::Global<v8::Object>) -> T,
    ) -> Result<T> {
        let global_proxy = self.inner.global_proxy.get().ok_or_else(|| {
            anyhow!("replacement context is missing its page-owned main WindowProxy")
        })?;
        Ok(op(global_proxy))
    }
}

pub(crate) struct ScriptVmDefaultWorldBootstrap {
    pub(super) main_world: super::main_world::MainWorld,
    pub(super) resource_owner_id: ResourceOwnerId,
    pub(super) promise_reject_dispatch: PromiseRejectDispatchSlot,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) root_frame_id: Option<String>,
    pub(super) prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) post_domcontentloaded_page_task_tx: PageTaskSender,
    pub(super) page_runtime_wake_tx: PageRuntimeWakeSender,
    pub(super) storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    // Partial bootstrap can drop before publication. Release bridge owners
    // before their native backing, and keep V8 alive until all of it
    // has been released.
    pub(super) context_host: Rc<RefCell<PageScriptHost>>,
    pub(super) document_runtime: Box<DocumentRuntime>,
    pub(super) renderer_document_isolate: RendererIsolateHandle,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RendererDocumentIsolateTeardown {
    unregister_platform_on_context_teardown: bool,
    #[cfg(test)]
    requires_deferred_lifo_drop: bool,
}

impl RendererDocumentIsolateTeardown {
    fn owner_reserved_page() -> Self {
        #[cfg(test)]
        {
            Self {
                unregister_platform_on_context_teardown: false,
                requires_deferred_lifo_drop: false,
            }
        }
        #[cfg(not(test))]
        {
            Self {
                unregister_platform_on_context_teardown: false,
            }
        }
    }

    #[cfg(test)]
    fn standalone_test() -> Self {
        Self {
            unregister_platform_on_context_teardown: true,
            requires_deferred_lifo_drop: true,
        }
    }

    pub(super) fn unregister_platform_on_context_teardown(
        self,
        renderer_document_isolate: &RendererIsolateHandle,
    ) {
        if self.unregister_platform_on_context_teardown {
            renderer_document_isolate.unregister_renderer_document_isolate_platform();
        }
    }

    pub(super) fn requires_deferred_lifo_script_vm_drop(self) -> bool {
        #[cfg(test)]
        {
            self.requires_deferred_lifo_drop
        }
        #[cfg(not(test))]
        {
            false
        }
    }
}

/// Shared renderer-isolate access reused by related pages and replacement
/// document executions.
#[derive(Clone)]
pub(crate) struct RendererIsolateHandle {
    inner: Rc<RefCell<RendererIsolateHolder>>,
    deferred_context_host_releases: RendererDeferredContextHostReleaseQueue,
    related_pages:
        Rc<RefCell<std::collections::BTreeMap<u64, Weak<RendererPageScriptEnvironmentInner>>>>,
    pending_page_turn_completions: Rc<RefCell<PendingRelatedPageTurnCompletions>>,
}

type PendingRelatedPageTurnCompletions =
    std::collections::BTreeMap<usize, Weak<RefCell<PageScriptHost>>>;

impl std::fmt::Debug for RendererIsolateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererIsolateHandle")
            .finish_non_exhaustive()
    }
}

impl RendererIsolateHandle {
    pub(crate) fn deferred_context_host_release_queue(
        &self,
    ) -> RendererDeferredContextHostReleaseQueue {
        self.deferred_context_host_releases.clone()
    }

    #[cfg(test)]
    pub(crate) fn new_standalone_without_owner_reservation_for_test(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        Self::new_with_foreground_wake(
            V8ForegroundTaskWake::page(v8_foreground_task_sender)?,
            RendererDocumentIsolateTeardown::standalone_test(),
            None,
        )
    }

    pub(crate) fn new_owner_reserved_page(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
        devtools_target_shutdown_registry: &RendererDevToolsTargetShutdownRegistry,
        inspector_io_wake_tx: tokio::sync::mpsc::UnboundedSender<
            crate::devtools::ingress::io::RendererInspectorIoOwnerWake,
        >,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let bootstrap = Self::new_with_foreground_wake(
            V8ForegroundTaskWake::page(v8_foreground_task_sender)?,
            RendererDocumentIsolateTeardown::owner_reserved_page(),
            Some(devtools_target_shutdown_registry),
        )?;
        // Process-environment notifications belong to the isolate and retain
        // an owner wake after any one Page endpoint is closed.
        bootstrap
            .renderer_document_isolate
            .inspector_isolate_backend_handle()
            .devtools_target()
            .io_ref()
            .configure_owner_wake(inspector_io_wake_tx);
        Ok(bootstrap)
    }

    fn new_with_foreground_wake(
        foreground_wake: V8ForegroundTaskWake,
        renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
        devtools_target_shutdown_registry: Option<&RendererDevToolsTargetShutdownRegistry>,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let (renderer_document_isolate, bridge_bindings) =
            RendererIsolateHolder::new_holder(foreground_wake, devtools_target_shutdown_registry)?;
        let deferred_context_host_releases = renderer_document_isolate
            .deferred_context_host_releases
            .clone();
        let renderer_document_isolate = Self {
            deferred_context_host_releases,
            inner: Rc::new(RefCell::new(renderer_document_isolate)),
            related_pages: Rc::new(RefCell::new(Default::default())),
            pending_page_turn_completions: Rc::new(RefCell::new(Default::default())),
        };
        let isolate_backend = renderer_document_isolate.inspector_isolate_backend_handle();
        let isolate_backend =
            renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
                isolate_backend.new_page_handle(isolate)
            })?;
        Ok(RendererDocumentIsolateBootstrap {
            initial_document_environment: None,
            renderer_document_isolate,
            bridge_bindings,
            renderer_document_isolate_teardown,
            page_inspector: DocumentInspectorBinding::new(isolate_backend),
            renderer_page_script_environment: None,
        })
    }

    pub(crate) fn identity_key(&self) -> usize {
        Rc::as_ptr(&self.inner) as usize
    }

    pub(crate) fn inspector_isolate_backend_handle(&self) -> RendererInspectorIsolateBackendHandle {
        self.inner
            .borrow()
            .inspector_backend
            .as_ref()
            .expect("document isolate Inspector backend missing before ScriptVm drop")
            .handle()
    }

    fn build_bridge_bindings(&self) -> Result<NativeBridgeBindings> {
        let holder = self.inner.borrow();
        let isolate_ptr = unsafe { holder.isolate.as_raw_isolate_ptr() };
        Ok(NativeBridgeBindings::deferred(isolate_ptr))
    }

    pub(super) fn with_renderer_document_isolate_and_inspector_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererInspectorIsolateBackend) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            inspector_backend,
            maintenance,
            ..
        } = &mut *holder;
        maintenance.note_isolate_activity();
        let inspector_backend = inspector_backend
            .as_mut()
            .expect("document isolate Inspector backend missing before ScriptVm drop");
        with_entered_owned_isolate(isolate, |isolate| {
            let result = op(isolate, inspector_backend);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            result
        })
    }

    pub(super) fn with_entered_renderer_document_isolate_and_inspector_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererInspectorIsolateBackend) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            inspector_backend,
            maintenance,
            ..
        } = &mut *holder;
        maintenance.note_isolate_activity();
        let inspector_backend = inspector_backend
            .as_mut()
            .ok_or_else(|| anyhow!("document isolate Inspector backend unavailable"))?;
        with_entered_owned_isolate(isolate, |isolate| {
            let result = op(isolate, inspector_backend);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            result
        })
    }

    pub(super) fn with_renderer_document_isolate_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
    ) -> T {
        self.inner.borrow_mut().maintenance.note_isolate_activity();
        self.with_renderer_document_isolate_internal_mut(op)
    }

    /// Enter the document isolate for V8-internal or read-only runtime work
    /// (such as V8 GC foreground tasks, microtask checkpoint entry, or memory
    /// diagnostics) without counting it as script activity that re-arms
    /// maintenance on an otherwise idle page.
    pub(super) fn with_renderer_document_isolate_internal_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        with_entered_owned_isolate(&mut holder.isolate, |isolate| {
            let result = op(isolate);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            result
        })
    }

    /// Enter the document isolate to run one V8 platform foreground task.
    ///
    /// V8's `MemoryPressureLevel::Moderate` performs GC work asynchronously via
    /// foreground tasks. Pure GC foreground tasks must not count as script
    /// activity, whereas foreground continuations that execute author JS (such
    /// as `FinalizationRegistry` cleanup callbacks or async Wasm promise
    /// resolution) advance `activity_generation` via V8's call-completed hook.
    /// When an asynchronous moderate GC finishes, we fold its post-GC valley
    /// downward into the maintenance baselines.
    pub(super) fn with_renderer_document_isolate_v8_foreground_task_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            maintenance,
            ..
        } = &mut *holder;
        with_entered_owned_isolate(isolate, |isolate| {
            maintenance.ensure_promise_activity_hook_if_needed(isolate);
            let was_recalibration_armed = maintenance.is_moderate_recalibration_armed();
            let major_gc_before = maintenance.major_gc_completion_epoch();
            let call_completed_before = maintenance.call_completed_epoch();
            let promise_reaction_before = current_thread_promise_reaction_before_count();
            let result = op(isolate);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            if maintenance.call_completed_epoch() != call_completed_before
                || current_thread_promise_reaction_before_count() != promise_reaction_before
            {
                maintenance.note_isolate_activity();
            }
            maintenance.apply_pending_post_gc_valley();
            if was_recalibration_armed && maintenance.major_gc_completion_epoch() != major_gc_before
            {
                maintenance.recalibrate_baselines_downward(&isolate.get_heap_statistics());
            }
            result
        })
    }

    /// Enter the document isolate to run a microtask checkpoint.
    ///
    /// Empty checkpoints remain silent so idle pages do not re-arm bounded
    /// native-wrapper maintenance; checkpoints that run author JS reactions or
    /// produce checkpoint side effects advance `activity_generation`.
    pub(super) fn with_renderer_document_isolate_microtask_checkpoint_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            maintenance,
            ..
        } = &mut *holder;
        with_entered_owned_isolate(isolate, |isolate| {
            maintenance.ensure_promise_activity_hook_if_needed(isolate);
            let call_completed_before = maintenance.call_completed_epoch();
            let promise_reaction_before = current_thread_promise_reaction_before_count();
            let side_effect_before = maintenance.microtask_checkpoint_side_effect_epoch();
            let result = op(isolate);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            if maintenance.call_completed_epoch() != call_completed_before
                || current_thread_promise_reaction_before_count() != promise_reaction_before
                || maintenance.microtask_checkpoint_side_effect_epoch() != side_effect_before
            {
                maintenance.note_isolate_activity();
            }
            result
        })
    }

    pub(super) fn capture_initial_main_world_maintenance_baselines(&self) {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            maintenance,
            ..
        } = &mut *holder;
        if maintenance.initial_used_global_handles_size.is_some() {
            return;
        }
        with_entered_owned_isolate(isolate, |isolate| {
            maintenance.note_initial_main_world_baselines(&isolate.get_heap_statistics());
        });
    }

    pub(super) fn with_entered_renderer_document_isolate<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> Result<T>,
    ) -> Result<T> {
        self.with_renderer_document_isolate_mut(op)
    }

    pub(super) fn with_entered_renderer_document_isolate_maintenance<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererIsolateMaintenanceState) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            maintenance,
            ..
        } = &mut *holder;
        with_entered_owned_isolate(isolate, |isolate| {
            let result = op(isolate, maintenance);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            result
        })
    }

    pub(super) fn with_entered_renderer_document_isolate_and_bootstrap<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &IsolateBootstrapCache) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let RendererIsolateHolder {
            isolate,
            bootstrap,
            maintenance,
            ..
        } = &mut *holder;
        maintenance.note_isolate_activity();
        with_entered_owned_isolate(isolate, |isolate| {
            {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                bootstrap.initialize_if_needed(&mut scope.init())?;
            }
            let result = op(isolate, &*bootstrap);
            self.deferred_context_host_releases
                .drain_on_entered_isolate(isolate);
            result
        })
    }

    pub(super) fn unregister_renderer_document_isolate_platform(&self) {
        let mut holder = self.inner.borrow_mut();
        holder.unregister_platform();
    }

    #[cfg(any(test, feature = "devtools"))]
    pub(super) fn renderer_document_isolate_inspector_default_context_registry_count(
        &self,
    ) -> usize {
        self.inner.borrow().inspector_backend.as_ref().map_or(
            0,
            RendererInspectorIsolateBackend::default_context_registry_count,
        )
    }
}

fn saturating_fetch_add(counter: &AtomicUsize, delta: usize) {
    let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(delta))
    });
}

fn saturating_fetch_sub(counter: &AtomicUsize, delta: usize) {
    let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_sub(delta))
    });
}

#[derive(Clone, Copy, Debug)]
struct PostGcRecalibrationValley {
    tracked_memory: usize,
    used_global_handles_size: usize,
    detached_contexts: usize,
}

/// Per-isolate counters updated when JS wrappers register or release Rust-heap
/// resources managed by V8 weak finalizers or explicit native backing stores.
///
/// Stored both on `RendererIsolateMaintenanceState` and in a V8 isolate slot so
/// any frame or world in the document isolate can record native pressure without
/// locks, `RefCell` borrows, or V8 handle drop-order hazards. Atomics allow
/// `Send` backing stores (such as `NETWORK_BODY_SOURCES`) to hold a
/// [`NativeMemoryLease`] and release bytes immediately on explicit consumption.
#[derive(Debug, Default)]
pub(crate) struct IsolateNativeMemoryPressure {
    allocated_native_bytes_since_maintenance: AtomicUsize,
    live_native_bytes: AtomicUsize,
    finalizer_registrations_since_maintenance: AtomicUsize,
    live_finalizers: AtomicUsize,
    call_completed_epoch: AtomicU64,
    microtask_checkpoint_side_effect_epoch: AtomicU64,
    atomics_wait_async_used: AtomicBool,
    major_gc_completion_epoch: AtomicU64,
    moderate_recalibration_armed: AtomicBool,
    has_pending_recalibration_valley: AtomicBool,
    pending_valley_used_memory: AtomicUsize,
    pending_valley_used_global_handles_size: AtomicUsize,
    pending_valley_detached_contexts: AtomicUsize,
}

impl IsolateNativeMemoryPressure {
    fn record_finalizer_registration(&self, native_bytes: usize) {
        saturating_fetch_add(&self.finalizer_registrations_since_maintenance, 1);
        saturating_fetch_add(&self.live_finalizers, 1);
        if native_bytes > 0 {
            self.record_native_bytes_allocation(native_bytes);
        }
    }

    fn record_native_bytes_allocation(&self, native_bytes: usize) {
        if native_bytes > 0 {
            saturating_fetch_add(&self.allocated_native_bytes_since_maintenance, native_bytes);
            saturating_fetch_add(&self.live_native_bytes, native_bytes);
        }
    }

    pub(crate) fn release_finalizer(&self, native_bytes: usize) {
        saturating_fetch_sub(&self.live_finalizers, 1);
        if native_bytes > 0 {
            self.release_live_native_bytes(native_bytes);
        }
    }

    pub(crate) fn release_live_native_bytes(&self, native_bytes: usize) {
        if native_bytes > 0 {
            saturating_fetch_sub(&self.live_native_bytes, native_bytes);
        }
    }

    fn reset_since_maintenance(&self) {
        self.allocated_native_bytes_since_maintenance
            .store(0, Ordering::Relaxed);
        self.finalizer_registrations_since_maintenance
            .store(0, Ordering::Relaxed);
    }

    fn record_call_completed(&self) {
        self.call_completed_epoch.fetch_add(1, Ordering::Relaxed);
    }

    fn call_completed_epoch(&self) -> u64 {
        self.call_completed_epoch.load(Ordering::Relaxed)
    }

    pub(super) fn record_microtask_checkpoint_side_effect(&self) {
        self.microtask_checkpoint_side_effect_epoch
            .fetch_add(1, Ordering::Relaxed);
    }

    fn microtask_checkpoint_side_effect_epoch(&self) -> u64 {
        self.microtask_checkpoint_side_effect_epoch
            .load(Ordering::Relaxed)
    }

    fn record_atomics_wait_async_used(&self) {
        self.atomics_wait_async_used.store(true, Ordering::Relaxed);
    }

    fn atomics_wait_async_used(&self) -> bool {
        self.atomics_wait_async_used.load(Ordering::Relaxed)
    }

    fn major_gc_completion_epoch(&self) -> u64 {
        self.major_gc_completion_epoch.load(Ordering::Acquire)
    }

    fn is_moderate_recalibration_armed(&self) -> bool {
        self.moderate_recalibration_armed.load(Ordering::Acquire)
    }

    fn arm_moderate_recalibration(&self) {
        self.moderate_recalibration_armed
            .store(true, Ordering::Release);
    }

    fn disarm_moderate_recalibration_and_clear_valley(&self) {
        self.moderate_recalibration_armed
            .store(false, Ordering::Release);
        self.has_pending_recalibration_valley
            .store(false, Ordering::Release);
    }

    fn record_major_gc_completion_and_take_recalibration(&self) -> bool {
        self.major_gc_completion_epoch
            .fetch_add(1, Ordering::AcqRel);
        self.moderate_recalibration_armed
            .swap(false, Ordering::AcqRel)
    }

    fn record_post_gc_recalibration_valley(&self, stats: &v8::HeapStatistics) {
        let tracked_memory = stats
            .used_heap_size()
            .saturating_add(stats.external_memory());
        self.pending_valley_used_memory
            .store(tracked_memory, Ordering::Relaxed);
        self.pending_valley_used_global_handles_size
            .store(stats.used_global_handles_size(), Ordering::Relaxed);
        self.pending_valley_detached_contexts
            .store(stats.number_of_detached_contexts(), Ordering::Relaxed);
        self.has_pending_recalibration_valley
            .store(true, Ordering::Release);
    }

    fn take_pending_post_gc_valley(&self) -> Option<PostGcRecalibrationValley> {
        if !self
            .has_pending_recalibration_valley
            .swap(false, Ordering::AcqRel)
        {
            return None;
        }
        Some(PostGcRecalibrationValley {
            tracked_memory: self.pending_valley_used_memory.load(Ordering::Relaxed),
            used_global_handles_size: self
                .pending_valley_used_global_handles_size
                .load(Ordering::Relaxed),
            detached_contexts: self
                .pending_valley_detached_contexts
                .load(Ordering::Relaxed),
        })
    }
}

unsafe extern "C" fn major_gc_epilogue_callback(
    mut isolate: v8::UnsafeRawIsolatePtr,
    _gc_type: v8::GCType,
    _flags: v8::GCCallbackFlags,
    _data: *mut std::ffi::c_void,
) {
    if isolate.is_null() {
        return;
    }
    // SAFETY: V8 invokes GC epilogue callbacks synchronously on the isolate
    // thread while the isolate is alive and entered. Only read-only heap
    // statistics and atomics are touched (no JS allocations or mutations).
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut isolate) };
    let Some(pressure) = isolate.get_slot::<Arc<IsolateNativeMemoryPressure>>() else {
        return;
    };
    if !pressure.record_major_gc_completion_and_take_recalibration() {
        return;
    }
    let pressure = Arc::clone(pressure);
    let stats = isolate.get_heap_statistics();
    pressure.record_post_gc_recalibration_valley(&stats);
}

unsafe extern "C" fn call_completed_callback(isolate: v8::UnsafeRawIsolatePtr) {
    if isolate.is_null() {
        return;
    }
    // SAFETY: V8 invokes CallCompletedCallback synchronously on the isolate
    // thread with the live isolate pointer.
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr(&isolate) };
    if let Some(pressure) = isolate.get_slot::<Arc<IsolateNativeMemoryPressure>>() {
        pressure.record_call_completed();
    }
}

unsafe extern "C" fn use_counter_callback(
    isolate: v8::UnsafeRawIsolatePtr,
    feature: v8::UseCounterFeature,
) {
    if isolate.is_null() || feature != v8::UseCounterFeature::kAtomicsWaitAsync {
        return;
    }
    // SAFETY: V8 invokes UseCounterCallback synchronously on the isolate
    // thread with the live isolate pointer.
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr(&isolate) };
    if let Some(pressure) = isolate.get_slot::<Arc<IsolateNativeMemoryPressure>>() {
        pressure.record_atomics_wait_async_used();
    }
}

/// RAII lease tying a Rust-heap backing allocation to its owning document
/// isolate's native-memory pressure tracker.
///
/// Dropping the lease releases its `live_native_bytes` immediately, so explicit
/// consumption (e.g. `Response.text()` / `body.cancel()`) clears native pressure
/// even if the JS wrapper remains retained.
#[derive(Debug)]
pub(crate) struct NativeMemoryLease {
    pressure: Arc<IsolateNativeMemoryPressure>,
    bytes: AtomicUsize,
}

impl NativeMemoryLease {
    fn new(pressure: Arc<IsolateNativeMemoryPressure>, bytes: usize) -> Self {
        Self {
            pressure,
            bytes: AtomicUsize::new(bytes),
        }
    }
}

impl Drop for NativeMemoryLease {
    fn drop(&mut self) {
        let bytes = self.bytes.swap(0, Ordering::Relaxed);
        if bytes > 0 {
            self.pressure.release_live_native_bytes(bytes);
        }
    }
}

pub(crate) fn record_renderer_document_isolate_native_finalizer(
    scope: &mut v8::PinScope<'_, '_>,
    native_bytes: usize,
) -> Option<Arc<IsolateNativeMemoryPressure>> {
    let pressure = scope.get_slot::<Arc<IsolateNativeMemoryPressure>>()?;
    pressure.record_finalizer_registration(native_bytes);
    Some(Arc::clone(pressure))
}

pub(crate) fn record_renderer_document_isolate_native_memory_lease(
    scope: &mut v8::PinScope<'_, '_>,
    native_bytes: usize,
) -> Option<NativeMemoryLease> {
    if native_bytes == 0 {
        return None;
    }
    let pressure = scope.get_slot::<Arc<IsolateNativeMemoryPressure>>()?;
    pressure.record_native_bytes_allocation(native_bytes);
    Some(NativeMemoryLease::new(Arc::clone(pressure), native_bytes))
}

pub(super) struct RendererIsolateMaintenanceState {
    last_maintenance_used_memory: usize,
    initial_used_global_handles_size: Option<usize>,
    last_maintenance_used_global_handles_size: Option<usize>,
    last_maintenance_detached_contexts: usize,
    native_pressure: Arc<IsolateNativeMemoryPressure>,
    activity_generation: u64,
    last_maintenance_activity_generation: u64,
    ticks_with_pending_native_activity: u8,
    promise_activity_hook_installed: bool,
}

impl RendererIsolateMaintenanceState {
    fn new(
        native_pressure: Arc<IsolateNativeMemoryPressure>,
        promise_activity_hook_installed: bool,
    ) -> Self {
        Self {
            last_maintenance_used_memory: 0,
            initial_used_global_handles_size: None,
            last_maintenance_used_global_handles_size: None,
            last_maintenance_detached_contexts: 0,
            native_pressure,
            activity_generation: 0,
            last_maintenance_activity_generation: 0,
            ticks_with_pending_native_activity: 0,
            promise_activity_hook_installed,
        }
    }

    fn note_isolate_activity(&mut self) {
        self.activity_generation = self.activity_generation.wrapping_add(1);
    }

    fn call_completed_epoch(&self) -> u64 {
        self.native_pressure.call_completed_epoch()
    }

    fn microtask_checkpoint_side_effect_epoch(&self) -> u64 {
        self.native_pressure
            .microtask_checkpoint_side_effect_epoch()
    }

    pub(super) fn major_gc_completion_epoch(&self) -> u64 {
        self.native_pressure.major_gc_completion_epoch()
    }

    fn is_moderate_recalibration_armed(&self) -> bool {
        self.native_pressure.is_moderate_recalibration_armed()
    }

    fn ensure_promise_activity_hook_if_needed(&mut self, isolate: &mut v8::OwnedIsolate) {
        if !self.promise_activity_hook_installed && self.native_pressure.atomics_wait_async_used() {
            isolate.set_promise_hook(promise_activity_and_trace_hook);
            self.promise_activity_hook_installed = true;
        }
    }

    fn note_initial_main_world_baselines(&mut self, stats: &v8::HeapStatistics) {
        let used_global_handles_size = stats.used_global_handles_size();
        if self.initial_used_global_handles_size.is_none() {
            self.initial_used_global_handles_size = Some(used_global_handles_size);
            self.last_maintenance_used_global_handles_size = Some(used_global_handles_size);
        }
    }

    pub(super) fn apply_pending_post_gc_valley(&mut self) {
        if let Some(valley) = self.native_pressure.take_pending_post_gc_valley() {
            self.recalibrate_baselines_downward_values(
                valley.tracked_memory,
                valley.used_global_handles_size,
                valley.detached_contexts,
            );
        }
    }

    pub(super) fn memory_pressure_input(
        &self,
        stats: &v8::HeapStatistics,
    ) -> super::RendererDocumentIsolateMemoryPressureInput {
        super::RendererDocumentIsolateMemoryPressureInput {
            used_heap_size: stats.used_heap_size(),
            external_memory: stats.external_memory(),
            heap_size_limit: stats.heap_size_limit(),
            last_maintenance_used_memory: self.last_maintenance_used_memory,
            used_global_handles_size: stats.used_global_handles_size(),
            initial_used_global_handles_size: self.initial_used_global_handles_size,
            last_maintenance_used_global_handles_size: self
                .last_maintenance_used_global_handles_size,
            detached_contexts: stats.number_of_detached_contexts(),
            last_maintenance_detached_contexts: self.last_maintenance_detached_contexts,
            allocated_native_bytes_since_maintenance: self
                .native_pressure
                .allocated_native_bytes_since_maintenance
                .load(Ordering::Relaxed),
            live_native_bytes: self
                .native_pressure
                .live_native_bytes
                .load(Ordering::Relaxed),
            finalizer_registrations_since_maintenance: self
                .native_pressure
                .finalizer_registrations_since_maintenance
                .load(Ordering::Relaxed),
            live_finalizers: self.native_pressure.live_finalizers.load(Ordering::Relaxed),
            has_isolate_activity_since_maintenance: self.activity_generation
                != self.last_maintenance_activity_generation,
            ticks_with_pending_native_activity: self.ticks_with_pending_native_activity,
        }
    }

    pub(super) fn record_collection(&mut self, stats: &v8::HeapStatistics) {
        self.last_maintenance_used_memory = stats
            .used_heap_size()
            .saturating_add(stats.external_memory());
        let used_global_handles_size = stats.used_global_handles_size();
        self.last_maintenance_used_global_handles_size = Some(used_global_handles_size);
        if let Some(initial) = self.initial_used_global_handles_size {
            self.initial_used_global_handles_size = Some(initial.min(used_global_handles_size));
        }
        self.last_maintenance_detached_contexts = stats.number_of_detached_contexts();
        self.native_pressure.reset_since_maintenance();
        self.last_maintenance_activity_generation = self.activity_generation;
        self.ticks_with_pending_native_activity = 0;
        self.native_pressure
            .disarm_moderate_recalibration_and_clear_valley();
    }

    pub(super) fn record_moderate_collection(
        &mut self,
        stats: &v8::HeapStatistics,
        major_gc_epoch_before: u64,
    ) {
        self.record_collection(stats);
        // `MemoryPressureLevel::Moderate` normally performs GC asynchronously
        // in V8 foreground tasks. Keep recalibration armed until the resulting
        // `MarkSweepCompact` epilogue is observed (or a subsequent collection
        // supersedes it); if GC already completed synchronously during
        // `memory_pressure_notification`, `stats` already reflects the valley.
        if self.major_gc_completion_epoch() == major_gc_epoch_before {
            self.native_pressure.arm_moderate_recalibration();
        }
    }

    fn recalibrate_baselines_downward(&mut self, stats: &v8::HeapStatistics) {
        let tracked_memory = stats
            .used_heap_size()
            .saturating_add(stats.external_memory());
        self.recalibrate_baselines_downward_values(
            tracked_memory,
            stats.used_global_handles_size(),
            stats.number_of_detached_contexts(),
        );
    }

    fn recalibrate_baselines_downward_values(
        &mut self,
        tracked_memory: usize,
        used_global_handles_size: usize,
        detached_contexts: usize,
    ) {
        self.last_maintenance_used_memory = self.last_maintenance_used_memory.min(tracked_memory);
        self.last_maintenance_used_global_handles_size = Some(
            self.last_maintenance_used_global_handles_size
                .map_or(used_global_handles_size, |last| {
                    last.min(used_global_handles_size)
                }),
        );
        if let Some(initial) = self.initial_used_global_handles_size {
            self.initial_used_global_handles_size = Some(initial.min(used_global_handles_size));
        }
        self.last_maintenance_detached_contexts = self
            .last_maintenance_detached_contexts
            .min(detached_contexts);
    }

    pub(super) fn record_skipped(
        &mut self,
        stats: &v8::HeapStatistics,
        has_pending_native_activity: bool,
    ) {
        // Track natural memory shrinkage without ratcheting baselines upward so
        // gradual growth still triggers moderate maintenance once total growth
        // since the last maintenance exceeds the threshold. Keep any pending
        // asynchronous moderate recalibration armed until V8 observes the
        // major GC completion epilogue.
        self.apply_pending_post_gc_valley();
        self.recalibrate_baselines_downward(stats);
        if has_pending_native_activity {
            self.ticks_with_pending_native_activity =
                self.ticks_with_pending_native_activity.saturating_add(1);
        } else {
            self.ticks_with_pending_native_activity = 0;
        }
    }
}

/// Owns the renderer isolate and the resources retired with that isolate.
pub(super) struct RendererIsolateHolder {
    deferred_context_host_releases: RendererDeferredContextHostReleaseQueue,
    // Unregister before destroying the Inspector backend so owner shutdown
    // never observes a target whose isolate has already been disposed.
    _devtools_target_shutdown_registration: Option<RendererDevToolsTargetShutdownRegistration>,
    // Inspector backend/session teardown touches V8 objects, so it must drop before the
    // isolate. `ScriptVm::drop` normally performs explicit context destruction;
    // this field order is the final safety net for partial construction paths.
    inspector_backend: Option<RendererInspectorIsolateBackend>,
    bootstrap: IsolateBootstrapCache,
    _platform_registration: V8PlatformIsolateRegistration,
    foreground_router: Option<crate::v8_platform::RendererIsolateForegroundTaskRouter>,
    isolate: v8::OwnedIsolate,
    maintenance: RendererIsolateMaintenanceState,
    // Declared after the isolate so destroyed/live accounting changes only
    // after `OwnedIsolate::drop` has completed disposal.
    _accounting: RendererDocumentIsolateAccountingGuard,
}

impl RendererIsolateHolder {
    fn new_holder(
        foreground_wake: V8ForegroundTaskWake,
        devtools_target_shutdown_registry: Option<&RendererDevToolsTargetShutdownRegistry>,
    ) -> Result<(Self, NativeBridgeBindings)> {
        let foreground_router = foreground_wake.page_router();
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let total_start = timing_enabled.then(std::time::Instant::now);

        let isolate_new_start = timing_enabled.then(std::time::Instant::now);
        // Window agents must not block their event loop with Atomics.wait().
        // Blink configures its main-thread isolates the same way; dedicated
        // workers keep V8's default and may use the blocking operation.
        let mut isolate = v8::Isolate::new(v8::CreateParams::default().allow_atomics_wait(false));
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "v8_isolate_new",
                elapsed_ms = isolate_new_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "v8::Isolate::new (cold, no snapshot)"
            );
        }

        // kExplicit: the owner loop manually checkpoints microtasks at
        // observable page/command boundaries.
        crate::context_bootstrap::install_agent_microtask_checkpoint_tasks(&mut isolate);
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 32);
        // V8 publishes ERROR messages for access-check exceptions before JavaScript gets a
        // chance to catch them. The script, callback, and promise owners already report values
        // that remain uncaught, so treating every ERROR-level listener message as uncaught
        // produces false process diagnostics for ordinary caught Web API exceptions.
        let non_exception_message_levels = v8::MessageErrorLevel::LOG
            | v8::MessageErrorLevel::DEBUG
            | v8::MessageErrorLevel::INFO
            | v8::MessageErrorLevel::WARNING;
        isolate.add_message_listener_with_error_level(
            v8_message_listener,
            non_exception_message_levels,
        );
        isolate.set_host_initialize_import_meta_object_callback(
            initialize_import_meta_object_callback,
        );
        isolate.set_host_import_module_dynamically_callback(dynamic_import_callback);
        isolate.set_host_import_module_with_phase_dynamically_callback(
            dynamic_import_with_phase_callback,
        );
        isolate.set_allow_wasm_code_generation_callback(
            super::security_policy::wasm_code_generation_check_callback,
        );
        isolate.set_modify_code_generation_from_strings_callback(
            super::security_policy::string_code_generation_check_callback,
        );
        let promise_activity_hook_installed = moli_trace::dom_binding_timing_enabled();
        if promise_activity_hook_installed {
            isolate.set_promise_hook(promise_activity_and_trace_hook);
        }
        isolate.set_promise_reject_callback(promise_reject_callback);
        isolate.set_failed_access_check_callback_function(failed_access_check_callback);
        let native_pressure = Arc::new(IsolateNativeMemoryPressure::default());
        isolate.set_slot(Arc::clone(&native_pressure));

        let inspector_start = timing_enabled.then(std::time::Instant::now);
        let inspector_backend = RendererInspectorIsolateBackend::new(&mut isolate)
            .with_shutdown_registry(devtools_target_shutdown_registry.cloned());
        let inspector_elapsed = inspector_start.map(|start| start.elapsed());
        let platform_registration = V8PlatformIsolateRegistration::register(
            &mut isolate,
            foreground_wake.into_platform_wake(),
            inspector_backend
                .devtools_target()
                .io_ref()
                .environment_notifier(),
        );
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        let isolate_bootstrap = IsolateBootstrapCache::default();
        let bridge_bindings = NativeBridgeBindings::deferred(isolate_ptr);

        let devtools_target_shutdown_registration = devtools_target_shutdown_registry
            .map(|registry| registry.register(inspector_backend.devtools_target()))
            .transpose()
            .map_err(|error| anyhow!(error))?;
        // Install V8 callbacks after all fallible initialization succeeds so
        // `RendererIsolateHolder::drop` is guaranteed to remove them.
        isolate.add_gc_epilogue_callback(
            major_gc_epilogue_callback,
            std::ptr::null_mut(),
            v8::GCType::kGCTypeMarkSweepCompact,
        );
        isolate.add_call_completed_callback(call_completed_callback);
        isolate.set_use_counter_callback(use_counter_callback);
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "inspector_backend_new",
                elapsed_ms = inspector_elapsed.unwrap().as_secs_f64() * 1000.0,
                "RendererInspectorIsolateBackend::new"
            );
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "v8_isolate_init_total",
                elapsed_ms = total_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "V8 isolate initialization total (cold, no snapshot)"
            );
        }

        // `v8::Isolate::new` enters the isolate. Document isolates are owned
        // independently by PageVms and may be destroyed in any page order, so
        // no isolate may remain on V8's thread-local enter stack between
        // operations.
        unsafe {
            isolate.exit();
        }

        Ok((
            Self::new(
                devtools_target_shutdown_registration,
                inspector_backend,
                isolate_bootstrap,
                platform_registration,
                foreground_router,
                isolate,
                native_pressure,
                promise_activity_hook_installed,
            ),
            bridge_bindings,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        devtools_target_shutdown_registration: Option<RendererDevToolsTargetShutdownRegistration>,
        inspector_backend: RendererInspectorIsolateBackend,
        bootstrap: IsolateBootstrapCache,
        platform_registration: V8PlatformIsolateRegistration,
        foreground_router: Option<crate::v8_platform::RendererIsolateForegroundTaskRouter>,
        isolate: v8::OwnedIsolate,
        native_pressure: Arc<IsolateNativeMemoryPressure>,
        promise_activity_hook_installed: bool,
    ) -> Self {
        Self {
            deferred_context_host_releases: RendererDeferredContextHostReleaseQueue::default(),
            _devtools_target_shutdown_registration: devtools_target_shutdown_registration,
            inspector_backend: Some(inspector_backend),
            bootstrap,
            _platform_registration: platform_registration,
            foreground_router,
            isolate,
            maintenance: RendererIsolateMaintenanceState::new(
                native_pressure,
                promise_activity_hook_installed,
            ),
            _accounting: RendererDocumentIsolateAccountingGuard::new(),
        }
    }

    fn unregister_platform(&mut self) {
        // Unregister synchronously flushes any active CPU profiler, which must
        // run with this isolate current even when retirement is between turns.
        with_entered_owned_isolate(&mut self.isolate, |_| {
            self._platform_registration.unregister();
            if let Some(router) = &self.foreground_router {
                router.retire();
            }
        });
    }
}

impl Drop for RendererIsolateHolder {
    fn drop(&mut self) {
        self.unregister_platform();
        // Fields drop in declaration order after this method. Enter now so the
        // inspector and bootstrap globals are released in their owning
        // isolate, then the platform registration is canceled, and finally
        // `OwnedIsolate::drop` observes itself as current and disposes it.
        unsafe {
            self.isolate.enter();
        }
        self.isolate
            .remove_gc_epilogue_callback(major_gc_epilogue_callback, std::ptr::null_mut());
        self.isolate
            .remove_call_completed_callback(call_completed_callback);
        self.deferred_context_host_releases
            .begin_isolate_shutdown(&mut self.isolate);
    }
}

struct EnteredIsolateGuard(*mut v8::OwnedIsolate);

impl Drop for EnteredIsolateGuard {
    fn drop(&mut self) {
        unsafe {
            super::inspector::finish_page_close_termination(&mut *self.0);
            (*self.0).exit();
        }
    }
}

fn with_entered_owned_isolate<T>(
    isolate: &mut v8::OwnedIsolate,
    op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
) -> T {
    unsafe {
        isolate.enter();
    }
    let _guard = EnteredIsolateGuard(isolate);
    op(isolate)
}

#[derive(Default)]
pub(super) struct IsolateBootstrapCache {
    context_assets: Option<ContextBootstrapAssets>,
}

impl IsolateBootstrapCache {
    fn initialize_if_needed(&mut self, scope: &mut v8::PinScope<'_, '_, ()>) -> Result<()> {
        if self.context_assets.is_none() {
            self.context_assets = Some(ContextBootstrapAssets::build(scope)?);
        }
        Ok(())
    }

    pub(super) fn global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        self.context_assets
            .as_ref()
            .expect("Context assets are initialized at realm entry")
            .global_template(scope)
    }

    pub(super) fn cross_origin_window_global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        self.context_assets
            .as_ref()
            .expect("Context assets are initialized at realm entry")
            .cross_origin_window_global_template(scope)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    struct ContextSlotDropCounter(Rc<Cell<usize>>);

    impl Drop for ContextSlotDropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get().saturating_add(1));
        }
    }

    struct ContextSlotDropProbe {
        dropped: Rc<Cell<usize>>,
        isolate_handle_was_live: Rc<Cell<usize>>,
        isolate_handle: v8::IsolateHandle,
    }

    impl Drop for ContextSlotDropProbe {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get().saturating_add(1));
            if self.isolate_handle.cancel_terminate_execution() {
                self.isolate_handle_was_live
                    .set(self.isolate_handle_was_live.get().saturating_add(1));
            }
        }
    }

    #[test]
    fn context_annex_weak_handles_are_safe_during_isolate_teardown() {
        crate::ensure_v8_for_test();

        const ISOLATE_COUNT: usize = 4;
        const CONTEXTS_PER_ISOLATE: usize = 32;
        let dropped_slots = Rc::new(Cell::new(0));
        let slots_dropped_with_live_isolate = Rc::new(Cell::new(0));

        for _ in 0..ISOLATE_COUNT {
            let mut isolate = v8::Isolate::new(Default::default());
            let isolate_handle = isolate.thread_safe_handle();
            let mut contexts = Vec::with_capacity(CONTEXTS_PER_ISOLATE);
            {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                for _ in 0..CONTEXTS_PER_ISOLATE {
                    let context = v8::Context::new(scope, Default::default());
                    let replaced = context.set_slot(Rc::new(ContextSlotDropProbe {
                        dropped: Rc::clone(&dropped_slots),
                        isolate_handle_was_live: Rc::clone(&slots_dropped_with_live_isolate),
                        isolate_handle: isolate_handle.clone(),
                    }));
                    assert!(replaced.is_none());
                    contexts.push(v8::Global::new(scope, context));
                }
            }

            // Leave ContextAnnex finalizers pending until OwnedIsolate teardown.
            drop(contexts);
            drop(isolate);
        }

        assert_eq!(dropped_slots.get(), ISOLATE_COUNT * CONTEXTS_PER_ISOLATE);
        assert_eq!(
            slots_dropped_with_live_isolate.get(),
            ISOLATE_COUNT * CONTEXTS_PER_ISOLATE,
            "context annex slots must drop while their weak V8 handles can still be reset"
        );
    }

    #[test]
    fn snapshot_creator_cleans_up_context_annex_before_creating_blob() {
        crate::ensure_v8_for_test();

        let dropped_slots = Rc::new(Cell::new(0));
        let mut snapshot_creator = v8::Isolate::snapshot_creator(None, None);
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut snapshot_creator));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let replaced =
                context.set_slot(Rc::new(ContextSlotDropCounter(Rc::clone(&dropped_slots))));
            assert!(replaced.is_none());
            scope.set_default_context(context);
        }

        let startup_data = snapshot_creator
            .create_blob(v8::FunctionCodeHandling::Clear)
            .expect("snapshot creator should produce a blob");
        assert!(!startup_data.is_empty());
        assert_eq!(dropped_slots.get(), 1);
    }

    #[test]
    fn moderate_recalibration_stays_armed_until_major_gc_and_disarms_once_observed() {
        crate::ensure_v8_for_test();

        let pressure = std::sync::Arc::new(super::IsolateNativeMemoryPressure::default());
        let mut maintenance =
            super::RendererIsolateMaintenanceState::new(std::sync::Arc::clone(&pressure), false);

        let mut isolate = v8::Isolate::new(Default::default());
        let stats = isolate.get_heap_statistics();

        // Async Moderate (major GC epoch unchanged): arms recalibration.
        let epoch_before = maintenance.major_gc_completion_epoch();
        maintenance.record_moderate_collection(&stats, epoch_before);
        assert!(maintenance.is_moderate_recalibration_armed());

        // Skipped ticks before async GC finishes must keep recalibration armed.
        maintenance.record_skipped(&stats, false);
        maintenance.record_skipped(&stats, false);
        assert!(maintenance.is_moderate_recalibration_armed());

        // First major GC completion consumes the armed flag and records a valley.
        assert!(pressure.record_major_gc_completion_and_take_recalibration());
        assert!(!maintenance.is_moderate_recalibration_armed());
        pressure.record_post_gc_recalibration_valley(&stats);

        // Subsequent unrelated major GCs must not re-arm or recapture valleys.
        assert!(!pressure.record_major_gc_completion_and_take_recalibration());
        assert!(pressure.take_pending_post_gc_valley().is_some());
        assert!(pressure.take_pending_post_gc_valley().is_none());

        // Synchronous major GC during `memory_pressure_notification(Moderate)`
        // advances the epoch before `record_moderate_collection`, so
        // recalibration does not stay armed.
        let epoch_before_sync = maintenance.major_gc_completion_epoch();
        assert!(!pressure.record_major_gc_completion_and_take_recalibration());
        maintenance.record_moderate_collection(&stats, epoch_before_sync);
        assert!(!maintenance.is_moderate_recalibration_armed());

        // Explicit/Critical `record_collection` disarms pending recalibration
        // and discards any unapplied valley from an earlier moderate pass.
        let epoch_before_second = maintenance.major_gc_completion_epoch();
        maintenance.record_moderate_collection(&stats, epoch_before_second);
        assert!(maintenance.is_moderate_recalibration_armed());
        pressure.record_post_gc_recalibration_valley(&stats);
        maintenance.record_collection(&stats);
        assert!(!maintenance.is_moderate_recalibration_armed());
        assert!(pressure.take_pending_post_gc_valley().is_none());
    }
}
