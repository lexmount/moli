use super::{
    inspector::{
        DocumentInspectorBinding, RendererInspectorIsolateBackend,
        RendererInspectorIsolateBackendHandle,
    },
    runtime_bindings::{
        PromiseRejectDispatchSlot, failed_access_check_callback, promise_reject_callback,
        promise_trace_hook,
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
    native_bridge::{
        JsContextHost, JsContextHostBridgeRef, RuntimeObservableContextToken,
        SharedPrebootstrappedChildDefaultContexts,
    },
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
    cell::{OnceCell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

static DOCUMENT_ISOLATE_CREATED_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_DESTROYED_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_LIVE_COUNT: AtomicU64 = AtomicU64::new(0);
static DOCUMENT_ISOLATE_RESERVED_COUNT: AtomicU64 = AtomicU64::new(0);

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
    pub(super) resource_owner_id: ResourceOwnerId,
    pub(super) promise_reject_dispatch: PromiseRejectDispatchSlot,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_document_isolate: RendererDocumentIsolateHandle,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) document_runtime: Box<DocumentRuntime>,
    pub(super) root_frame_id: Option<String>,
    pub(super) context_host: Rc<RefCell<JsContextHost>>,
    pub(super) prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) post_domcontentloaded_page_task_tx: PageTaskSender,
    pub(super) page_runtime_wake_tx: PageRuntimeWakeSender,
    pub(super) storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) reuse_main_window_proxy: bool,
}

pub(super) struct ScriptVmContextBootstrap {
    pub(super) context: v8::Global<v8::Context>,
    pub(super) runtime_observable_context_token: RuntimeObservableContextToken,
    pub(super) bridge_ref: JsContextHostBridgeRef,
}

pub(crate) struct RendererDocumentIsolateBootstrap {
    pub(super) renderer_document_isolate: RendererDocumentIsolateHandle,
    pub(super) bridge_bindings: NativeBridgeBindings,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) reuse_main_window_proxy: bool,
}

impl RendererDocumentIsolateBootstrap {
    pub(crate) fn renderer_devtools_agent_token(
        &self,
    ) -> crate::runtime::RendererDevToolsAgentToken {
        self.page_inspector.agent_token()
    }

    pub(crate) fn clone_renderer_document_isolate_handle_for_owner_retention(
        &self,
    ) -> RendererDocumentIsolateHandle {
        self.renderer_document_isolate.clone()
    }

    pub(crate) fn renderer_page_script_environment(&self) -> Option<RendererPageScriptEnvironment> {
        self.renderer_page_script_environment.clone()
    }

    pub(crate) fn inspector_isolate_backend_handle(&self) -> RendererInspectorIsolateBackendHandle {
        self.renderer_document_isolate
            .inspector_isolate_backend_handle()
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
    page_id: u64,
    renderer_document_isolate: RendererDocumentIsolateHandle,
    page_runtime_task_source: PageRuntimeTaskSource,
    output_journal: crate::runtime::RendererTurnOutputJournal,
    global_proxy: Rc<OnceCell<v8::Global<v8::Object>>>,
}

impl std::fmt::Debug for RendererPageScriptEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPageScriptEnvironment")
            .field("page_id", &self.page_id)
            .field(
                "isolate_identity_key",
                &self.renderer_document_isolate.identity_key(),
            )
            .field(
                "runtime_task_source_identity_key",
                &self.page_runtime_task_source.identity_key(),
            )
            .field("output_stream", &self.output_journal.stream())
            .field("has_global_proxy", &self.global_proxy.get().is_some())
            .finish()
    }
}

impl RendererPageScriptEnvironment {
    pub(crate) fn new(
        page_id: u64,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        page_runtime_task_source: PageRuntimeTaskSource,
        output_journal: crate::runtime::RendererTurnOutputJournal,
    ) -> Self {
        Self {
            page_id,
            renderer_document_isolate,
            page_runtime_task_source,
            output_journal,
            global_proxy: Rc::new(OnceCell::new()),
        }
    }

    pub(crate) fn page_id(&self) -> u64 {
        self.page_id
    }

    pub(crate) fn page_runtime_task_source(&self) -> PageRuntimeTaskSource {
        self.page_runtime_task_source.clone()
    }

    pub(crate) fn output_journal(&self) -> crate::runtime::RendererTurnOutputJournal {
        self.output_journal.clone()
    }

    pub(crate) fn clear_page_runtime_tasks(&self) {
        self.page_runtime_task_source.clear();
    }

    pub(crate) fn retire_output_stream(&self) {
        self.output_journal
            .retire(crate::runtime::RendererOutputStreamCloseReason::ResidenceRetired);
    }

    pub(crate) fn isolate_identity_key(&self) -> usize {
        self.renderer_document_isolate.identity_key()
    }

    pub(crate) fn bootstrap_replacement_document_isolate(
        &self,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let bridge_bindings = self.renderer_document_isolate.build_bridge_bindings()?;
        let isolate_backend = self
            .renderer_document_isolate
            .inspector_isolate_backend_handle();
        Ok(RendererDocumentIsolateBootstrap {
            renderer_document_isolate: self.renderer_document_isolate.clone(),
            bridge_bindings,
            renderer_document_isolate_teardown:
                RendererDocumentIsolateTeardown::owner_reserved_page(),
            page_inspector: DocumentInspectorBinding::new(isolate_backend)
                .with_output_journal(self.output_journal()),
            renderer_page_script_environment: Some(self.clone()),
            reuse_main_window_proxy: true,
        })
    }

    pub(super) fn install_initial_main_window_proxy(
        &self,
        global_proxy: v8::Global<v8::Object>,
    ) -> Result<()> {
        self.global_proxy
            .set(global_proxy)
            .map_err(|_| anyhow!("page script environment already retains its main WindowProxy"))
    }

    pub(super) fn with_main_window_proxy<T>(
        &self,
        op: impl FnOnce(&v8::Global<v8::Object>) -> T,
    ) -> Result<T> {
        let global_proxy = self.global_proxy.get().ok_or_else(|| {
            anyhow!("replacement context is missing its page-owned main WindowProxy")
        })?;
        Ok(op(global_proxy))
    }
}

pub(crate) struct ScriptVmDefaultWorldBootstrap {
    pub(super) resource_owner_id: ResourceOwnerId,
    pub(super) promise_reject_dispatch: PromiseRejectDispatchSlot,
    pub(super) page_inspector: DocumentInspectorBinding,
    pub(super) renderer_document_isolate: RendererDocumentIsolateHandle,
    pub(super) renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
    pub(super) renderer_page_script_environment: Option<RendererPageScriptEnvironment>,
    pub(super) page_default_context: v8::Global<v8::Context>,
    pub(super) bridge_ref: JsContextHostBridgeRef,
    pub(super) runtime_observable_context_token: RuntimeObservableContextToken,
    pub(super) baseline_globals: super::ScriptGlobalsBaseline,
    pub(super) document_runtime: Box<DocumentRuntime>,
    pub(super) root_frame_id: Option<String>,
    pub(super) context_host: Rc<RefCell<JsContextHost>>,
    pub(super) prebootstrapped_child_default_contexts: SharedPrebootstrappedChildDefaultContexts,
    pub(super) page_context_cancel_tx: RendererPageContextCancelSender,
    pub(super) post_domcontentloaded_page_task_tx: PageTaskSender,
    pub(super) page_runtime_wake_tx: PageRuntimeWakeSender,
    pub(super) storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
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
        renderer_document_isolate: &RendererDocumentIsolateHandle,
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

#[derive(Clone)]
pub(crate) struct RendererDocumentIsolateHandle {
    inner: Rc<RefCell<RendererDocumentIsolateHolder>>,
}

impl std::fmt::Debug for RendererDocumentIsolateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererDocumentIsolateHandle")
            .finish_non_exhaustive()
    }
}

impl RendererDocumentIsolateHandle {
    #[cfg(test)]
    pub(crate) fn new_standalone_without_owner_reservation_for_test(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        Self::new_with_foreground_wake(
            V8ForegroundTaskWake::page(v8_foreground_task_sender),
            RendererDocumentIsolateTeardown::standalone_test(),
            None,
        )
    }

    pub(crate) fn new_owner_reserved_page(
        v8_foreground_task_sender: RendererPageV8ForegroundTaskSender,
        devtools_target_shutdown_registry: &RendererDevToolsTargetShutdownRegistry,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        Self::new_with_foreground_wake(
            V8ForegroundTaskWake::page(v8_foreground_task_sender),
            RendererDocumentIsolateTeardown::owner_reserved_page(),
            Some(devtools_target_shutdown_registry),
        )
    }

    fn new_with_foreground_wake(
        foreground_wake: V8ForegroundTaskWake,
        renderer_document_isolate_teardown: RendererDocumentIsolateTeardown,
        devtools_target_shutdown_registry: Option<&RendererDevToolsTargetShutdownRegistry>,
    ) -> Result<RendererDocumentIsolateBootstrap> {
        let (renderer_document_isolate, bridge_bindings) =
            RendererDocumentIsolateHolder::new_holder(
                foreground_wake,
                devtools_target_shutdown_registry,
            )?;
        let renderer_document_isolate = Self {
            inner: Rc::new(RefCell::new(renderer_document_isolate)),
        };
        let isolate_backend = renderer_document_isolate.inspector_isolate_backend_handle();
        Ok(RendererDocumentIsolateBootstrap {
            renderer_document_isolate,
            bridge_bindings,
            renderer_document_isolate_teardown,
            page_inspector: DocumentInspectorBinding::new(isolate_backend),
            renderer_page_script_environment: None,
            reuse_main_window_proxy: false,
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
        let mut holder = self.inner.borrow_mut();
        let RendererDocumentIsolateHolder {
            isolate, bootstrap, ..
        } = &mut *holder;
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        with_entered_owned_isolate(isolate, |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let global_template = bootstrap.global_template(scope);
            let cross_origin_window_global_template =
                bootstrap.cross_origin_window_global_template(scope);
            Ok(NativeBridgeBindings::build(
                scope,
                isolate_ptr,
                global_template,
                cross_origin_window_global_template,
            ))
        })
    }

    pub(super) fn with_renderer_document_isolate_and_inspector_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererInspectorIsolateBackend) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        let RendererDocumentIsolateHolder {
            isolate,
            inspector_backend,
            ..
        } = &mut *holder;
        let inspector_backend = inspector_backend
            .as_mut()
            .expect("document isolate Inspector backend missing before ScriptVm drop");
        with_entered_owned_isolate_value(isolate, |isolate| op(isolate, inspector_backend))
    }

    pub(super) fn with_entered_renderer_document_isolate_and_inspector_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &mut RendererInspectorIsolateBackend) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let RendererDocumentIsolateHolder {
            isolate,
            inspector_backend,
            ..
        } = &mut *holder;
        let inspector_backend = inspector_backend
            .as_mut()
            .ok_or_else(|| anyhow!("document isolate Inspector backend unavailable"))?;
        with_entered_owned_isolate(isolate, |isolate| op(isolate, inspector_backend))
    }

    pub(super) fn with_renderer_document_isolate_mut<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
    ) -> T {
        let mut holder = self.inner.borrow_mut();
        with_entered_owned_isolate_value(&mut holder.isolate, op)
    }

    pub(super) fn with_entered_renderer_document_isolate<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        with_entered_owned_isolate(&mut holder.isolate, op)
    }

    pub(super) fn with_entered_renderer_document_isolate_and_bootstrap<T>(
        &self,
        op: impl FnOnce(&mut v8::OwnedIsolate, &IsolateBootstrapCache) -> Result<T>,
    ) -> Result<T> {
        let mut holder = self.inner.borrow_mut();
        let RendererDocumentIsolateHolder {
            isolate, bootstrap, ..
        } = &mut *holder;
        with_entered_owned_isolate(isolate, |isolate| op(isolate, &*bootstrap))
    }

    pub(super) fn unregister_renderer_document_isolate_platform(&self) {
        self.inner.borrow_mut()._platform_registration.unregister();
    }

    pub(super) fn renderer_document_isolate_inspector_default_context_registry_count(
        &self,
    ) -> usize {
        self.inner.borrow().inspector_backend.as_ref().map_or(
            0,
            RendererInspectorIsolateBackend::default_context_registry_count,
        )
    }
}

pub(super) struct RendererDocumentIsolateHolder {
    // Unregister before destroying the Inspector backend so owner shutdown
    // never observes a target whose isolate has already been disposed.
    _devtools_target_shutdown_registration: Option<RendererDevToolsTargetShutdownRegistration>,
    // Inspector backend/session teardown touches V8 objects, so it must drop before the
    // isolate. `ScriptVm::drop` normally performs explicit context destruction;
    // this field order is the final safety net for partial construction paths.
    inspector_backend: Option<RendererInspectorIsolateBackend>,
    bootstrap: IsolateBootstrapCache,
    _platform_registration: V8PlatformIsolateRegistration,
    isolate: v8::OwnedIsolate,
    // Declared after the isolate so destroyed/live accounting changes only
    // after `OwnedIsolate::drop` has completed disposal.
    _accounting: RendererDocumentIsolateAccountingGuard,
}

impl RendererDocumentIsolateHolder {
    fn new_holder(
        foreground_wake: V8ForegroundTaskWake,
        devtools_target_shutdown_registry: Option<&RendererDevToolsTargetShutdownRegistry>,
    ) -> Result<(Self, NativeBridgeBindings)> {
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
        if moli_trace::dom_binding_timing_enabled() {
            isolate.set_promise_hook(promise_trace_hook);
        }
        isolate.set_promise_reject_callback(promise_reject_callback);
        isolate.set_failed_access_check_callback_function(failed_access_check_callback);

        let platform_registration = V8PlatformIsolateRegistration::register(
            &mut isolate,
            foreground_wake.into_platform_wake(),
        );
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        let isolate_bootstrap;
        let bridge_bindings;
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();

            let bootstrap_start = timing_enabled.then(std::time::Instant::now);
            isolate_bootstrap = IsolateBootstrapCache::build(scope)?;
            if timing_enabled {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    stage = "isolate_bootstrap_cache_build",
                    elapsed_ms = bootstrap_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                    "IsolateBootstrapCache::build (246 constructor specs + global template)"
                );
            }

            let bridge_start = timing_enabled.then(std::time::Instant::now);
            let global_template = isolate_bootstrap.global_template(scope);
            let cross_origin_window_global_template =
                isolate_bootstrap.cross_origin_window_global_template(scope);
            bridge_bindings = NativeBridgeBindings::build(
                scope,
                isolate_ptr,
                global_template,
                cross_origin_window_global_template,
            );
            if timing_enabled {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    stage = "native_bridge_bindings_build",
                    elapsed_ms = bridge_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                    "NativeBridgeBindings::build"
                );
            }
        }

        let inspector_start = timing_enabled.then(std::time::Instant::now);
        let inspector_backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let devtools_target_shutdown_registration = devtools_target_shutdown_registry
            .map(|registry| registry.register(inspector_backend.devtools_target()))
            .transpose()
            .map_err(|error| anyhow!(error))?;
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "inspector_backend_new",
                elapsed_ms = inspector_start.unwrap().elapsed().as_secs_f64() * 1000.0,
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
                isolate,
            ),
            bridge_bindings,
        ))
    }

    pub(super) fn new(
        devtools_target_shutdown_registration: Option<RendererDevToolsTargetShutdownRegistration>,
        inspector_backend: RendererInspectorIsolateBackend,
        bootstrap: IsolateBootstrapCache,
        platform_registration: V8PlatformIsolateRegistration,
        isolate: v8::OwnedIsolate,
    ) -> Self {
        Self {
            _devtools_target_shutdown_registration: devtools_target_shutdown_registration,
            inspector_backend: Some(inspector_backend),
            bootstrap,
            _platform_registration: platform_registration,
            isolate,
            _accounting: RendererDocumentIsolateAccountingGuard::new(),
        }
    }
}

impl Drop for RendererDocumentIsolateHolder {
    fn drop(&mut self) {
        // Fields drop in declaration order after this method. Enter now so the
        // inspector and bootstrap globals are released in their owning
        // isolate, then the platform registration is canceled, and finally
        // `OwnedIsolate::drop` observes itself as current and disposes it.
        unsafe {
            self.isolate.enter();
        }
    }
}

struct EnteredIsolateGuard(*mut v8::OwnedIsolate);

impl Drop for EnteredIsolateGuard {
    fn drop(&mut self) {
        unsafe {
            (*self.0).exit();
        }
    }
}

fn with_entered_owned_isolate<T>(
    isolate: &mut v8::OwnedIsolate,
    op: impl FnOnce(&mut v8::OwnedIsolate) -> Result<T>,
) -> Result<T> {
    unsafe {
        isolate.enter();
    }
    let _guard = EnteredIsolateGuard(isolate);
    op(isolate)
}

fn with_entered_owned_isolate_value<T>(
    isolate: &mut v8::OwnedIsolate,
    op: impl FnOnce(&mut v8::OwnedIsolate) -> T,
) -> T {
    unsafe {
        isolate.enter();
    }
    let _guard = EnteredIsolateGuard(isolate);
    op(isolate)
}

pub(super) struct IsolateBootstrapCache {
    pub(super) context_assets: ContextBootstrapAssets,
}

impl IsolateBootstrapCache {
    pub(super) fn build(scope: &mut v8::PinScope<'_, '_, ()>) -> Result<Self> {
        Ok(Self {
            context_assets: ContextBootstrapAssets::build(scope)?,
        })
    }

    pub(super) fn global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        self.context_assets.global_template(scope)
    }

    pub(super) fn cross_origin_window_global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        self.context_assets
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
}
