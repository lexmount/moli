use std::{
    sync::atomic::AtomicU64,
    sync::{Arc, Weak},
};

use anyhow::{Result, anyhow};
#[cfg(test)]
use tokio::sync::oneshot;
use url::Url;

use crate::{
    local_executor::JsLocalExecutor, network::ResourceRequestClient,
    render_runtime::RenderRuntimeOwner, types::ScriptExecutionReport,
};

use super::{
    DocumentStartScript, ExternalRawDocumentBodyStream, PageVmInitStage,
    RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner,
    RendererBrowserContextRuntimeOwnerAccess, RendererDocumentCommitPermit,
    RendererDocumentIsolateAccountingDiagnostics, RendererInspectorSessionRestoreSnapshot,
    RendererOwnerCommand, RendererOwnerHandle, RendererOwnerReply, RendererPageCreationArtifacts,
    RendererPageCreationDiagnostics, RendererPageHandle, RendererPageReservationToken,
    RendererPageState, RendererPendingDownloadActivation, RendererPerformanceMetricSnapshot,
    RendererReservedServiceWorkerClient,
};

pub(crate) struct PageVmStateCapture {
    pub(crate) final_url: Url,
    pub(crate) document_title: String,
    pub(crate) report: Arc<ScriptExecutionReport>,
    pub(crate) navigation_response: Option<PageVmNavigationResponse>,
    pub(crate) idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub(crate) service_worker_client_id: u64,
    pub(crate) dedicated_worker_running_worker_isolate_count: usize,
    pub(crate) performance_metric_snapshot: RendererPerformanceMetricSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct PageVmNavigationResponse {
    pub(crate) requested_url: Url,
    pub(crate) redirected: bool,
    pub(crate) redirect_count: usize,
    pub(crate) redirect_chain: Vec<crate::protocol_types::NavigationRedirect>,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct JsRuntime {
    inner: Arc<JsRuntimeInner>,
}

/// Thread-affine owner returned by the standalone JS runtime constructor.
/// Explicit browser-context callers keep their owner at the outer root and use
/// `JsRuntime` as a renderer handle only.
#[derive(Debug)]
pub struct JsRuntimeOwner {
    runtime: Option<JsRuntime>,
    _browser_context_owner: RendererBrowserContextRuntimeOwner,
}

#[derive(Debug)]
struct JsRuntimeInner {
    renderer_owner: RendererOwnerHandle,
    browser_context_runtime: RendererBrowserContextRuntime,
    _render_runtime: RenderRuntimeOwner,
}

#[derive(Clone)]
pub(crate) struct RendererProducerShutdownHandle {
    renderer_owner_id: u64,
    runtime: Weak<JsRuntimeInner>,
}

impl std::fmt::Debug for RendererProducerShutdownHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererProducerShutdownHandle")
            .field("renderer_owner_id", &self.renderer_owner_id)
            .finish_non_exhaustive()
    }
}

impl RendererProducerShutdownHandle {
    pub(crate) fn renderer_owner_id(&self) -> u64 {
        self.renderer_owner_id
    }

    pub(crate) fn cancel_page_producers(&self) {
        if let Some(runtime) = self.runtime.upgrade() {
            cancel_js_runtime_page_producers(&runtime);
        }
    }

    pub(crate) fn is_live(&self) -> bool {
        self.runtime.strong_count() != 0
    }
}

fn cancel_js_runtime_page_producers(inner: &JsRuntimeInner) {
    inner.renderer_owner.terminate_for_context_owner_shutdown();
}

impl Default for JsRuntimeOwner {
    fn default() -> Self {
        JsRuntime::initialize()
    }
}

impl JsRuntime {
    pub fn initialize() -> JsRuntimeOwner {
        let browser_context_owner = RendererBrowserContextRuntime::new();
        let runtime = Self::initialize_with_browser_context_owner_access(
            &browser_context_owner.owner_access(),
        )
        .expect("standalone browser context owner must accept its JS runtime");
        JsRuntimeOwner {
            runtime: Some(runtime),
            _browser_context_owner: browser_context_owner,
        }
    }

    /// Creates a JS runtime and atomically registers its producer shutdown
    /// capability with the matching browser-context owner root.
    pub fn initialize_with_browser_context_owner_access(
        browser_context_owner_access: &RendererBrowserContextRuntimeOwnerAccess,
    ) -> Result<Self> {
        let runtime = Self::initialize_with_browser_context_runtime_inner(
            browser_context_owner_access.runtime(),
        );
        browser_context_owner_access
            .register_renderer_producer(&runtime)
            .map_err(|error| anyhow!("browser context producer owner unavailable: {error}"))?;
        Ok(runtime)
    }

    pub fn initialize_with_page_vm_document_isolate_for_diagnostics() -> JsRuntimeOwner {
        Self::initialize()
    }

    fn initialize_with_browser_context_runtime_inner(
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> Self {
        moli_v8_init::ensure_v8_initialized_with_flags(
            Some(crate::v8_platform::initialization_flags()),
            crate::v8_platform::create_platform,
        );

        let local_executor = JsLocalExecutor::new();
        let next_page_id = Arc::new(AtomicU64::new(1));
        let (renderer_owner, render_runtime) = RendererOwnerHandle::new(
            local_executor.clone(),
            next_page_id.clone(),
            browser_context_runtime.clone(),
        );

        Self {
            inner: Arc::new(JsRuntimeInner {
                renderer_owner,
                browser_context_runtime,
                _render_runtime: render_runtime,
            }),
        }
    }

    pub fn browser_context_runtime(&self) -> RendererBrowserContextRuntime {
        self.inner.browser_context_runtime.clone()
    }

    /// Cancels Page/DedicatedWorker producers before an outer owner root joins
    /// browser resource runtimes.
    pub fn terminate_resource_producers_for_owner_shutdown(&self) {
        cancel_js_runtime_page_producers(&self.inner);
        self.inner
            .browser_context_runtime
            .terminate_resource_producers_for_owner_shutdown();
    }

    pub(crate) fn producer_shutdown_handle(&self) -> RendererProducerShutdownHandle {
        RendererProducerShutdownHandle {
            renderer_owner_id: self.renderer_owner_id_for_diagnostics(),
            runtime: Arc::downgrade(&self.inner),
        }
    }

    #[cfg(test)]
    pub(crate) fn install_owner_command_dispatch_gate_for_testing(
        &self,
    ) -> (
        crossbeam_channel::Receiver<()>,
        crossbeam_channel::Sender<()>,
    ) {
        self.inner
            .renderer_owner
            .install_command_dispatch_gate_for_testing()
    }

    #[cfg(test)]
    pub(crate) fn publish_next_command_output_before_owner_settlement_for_testing(&self) {
        self.inner
            .renderer_owner
            .publish_next_command_output_before_settlement_for_testing();
    }

    #[cfg(test)]
    pub(crate) fn close_owner_command_admission_for_testing(&self) {
        self.inner
            .renderer_owner
            .close_command_admission_for_testing();
    }

    #[cfg(test)]
    pub(crate) fn renderer_page_count_for_testing(&self) -> usize {
        self.inner.renderer_owner.len()
    }

    #[cfg(test)]
    pub(crate) fn start_minimal_html_page_for_reservation_testing(
        &self,
        page_reservation: RendererPageReservationToken,
        loader: &ResourceRequestClient,
    ) -> Result<PendingHtmlPage> {
        let url = Url::parse("https://reservation.test/")
            .expect("static reservation test URL should parse");
        self.start_create_html_page_from_response_with_inspector_session_restores(
            page_reservation,
            url.clone(),
            url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            None,
            None,
            crate::RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn enqueue_owner_command_probe_for_testing(
        &self,
    ) -> Result<oneshot::Receiver<Result<RendererOwnerReply>>> {
        self.inner
            .renderer_owner
            .enqueue_command_with_reply(RendererOwnerCommand::TestingDeferredPageVmDropPendingCount)
    }

    #[cfg(test)]
    pub(crate) fn try_attach_page_slot_for_testing(
        &self,
    ) -> (
        Result<()>,
        super::page_context_cancel::RendererPageContextCancelReceiver,
    ) {
        let page_id = self.inner.renderer_owner.allocate_page_id();
        let (cancel_tx, cancel_rx) =
            super::page_context_cancel::renderer_page_context_cancel_channel();
        let slot = super::RendererPageSlotHandle::new(
            Arc::downgrade(&self.inner.renderer_owner.state),
            super::RendererPageEntry::removed(page_id),
            cancel_tx,
            Default::default(),
        );
        let result = self
            .inner
            .renderer_owner
            .state
            .page_table
            .insert_new_slot(page_id, slot)
            .map(|_| ());
        (result, cancel_rx)
    }

    pub fn document_isolate_model_for_diagnostics(&self) -> &'static str {
        "page-vm"
    }

    pub fn document_isolate_accounting_for_diagnostics(
        &self,
    ) -> RendererDocumentIsolateAccountingDiagnostics {
        crate::script_vm::renderer_document_isolate_accounting_diagnostics()
    }

    pub fn renderer_owner_id_for_diagnostics(&self) -> u64 {
        self.inner.renderer_owner.state.owner_local_host_id.as_u64()
    }

    pub fn shares_renderer_owner_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(
            &self.inner.renderer_owner.state,
            &other.inner.renderer_owner.state,
        )
    }

    pub fn renderer_owner_handle(&self) -> RendererOwnerHandle {
        self.inner.renderer_owner.clone()
    }

    pub fn set_renderer_output_transport_sender(
        &self,
        sender: crate::runtime::RendererOutputTransportSender,
    ) {
        self.inner
            .renderer_owner
            .set_renderer_output_transport_sender(sender);
    }

    pub async fn create_html_page_from_response(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        html: String,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        self.create_html_page_from_response_with_inspector_session_restores(
            requested_url,
            final_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            web_storage,
            html,
            indexed_db_manager,
            storage_bucket_store,
            document_start_scripts,
            runtime_bindings,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            false,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            Vec::new(),
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_html_page_from_response_with_inspector_session_restores(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        html: String,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        root_frame_id: Option<String>,
        main_document_commit: Option<crate::RendererMainDocumentCommit>,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        let mut request = self
            .inner
            .renderer_owner
            .build_create_html_page_request_with_env(
                self.reserve_page_for_creation(),
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                loader,
                web_storage,
                final_url,
                html,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                PageVmInitStage::Load,
            );
        request.indexed_db_manager = indexed_db_manager;
        request.storage_bucket_store = storage_bucket_store;
        request.root_frame_id = root_frame_id;
        request.main_document_commit = main_document_commit;
        let reply = self
            .inner
            .renderer_owner
            .dispatch_command(RendererOwnerCommand::CreateHtmlPage(request))
            .await?;
        self.inner
            .renderer_owner
            .materialize_page_created_reply_parts(reply)
    }

    /// Fire-and-defer variant of [`Self::create_html_page_from_response`].
    ///
    /// Performs the synchronous request preparation, enqueues the renderer
    /// command, and returns a [`PendingHtmlPage`] without awaiting the reply.
    /// The renderer thread can then process the heavy V8/parse work in
    /// parallel with conn-side bookkeeping; the caller awaits the result via
    /// [`PendingHtmlPage::await_ready`] when the page is actually needed.
    #[allow(clippy::too_many_arguments)]
    pub fn start_create_html_page_from_response(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        html: String,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
    ) -> Result<PendingHtmlPage> {
        self.start_create_html_page_from_response_with_inspector_session_restores(
            self.reserve_page_for_creation(),
            requested_url,
            final_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            web_storage,
            html,
            indexed_db_manager,
            storage_bucket_store,
            document_start_scripts,
            runtime_bindings,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            false,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            Vec::new(),
            None,
            None,
            crate::RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_create_html_page_from_response_with_inspector_session_restores(
        &self,
        page_reservation: RendererPageReservationToken,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        html: String,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        root_frame_id: Option<String>,
        top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
        top_level_navigation_dispatch: crate::RendererTopLevelNavigationDispatch,
        main_document_commit: Option<crate::RendererMainDocumentCommit>,
    ) -> Result<PendingHtmlPage> {
        let mut request = self
            .inner
            .renderer_owner
            .build_create_html_page_request_with_env(
                page_reservation,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                loader,
                web_storage,
                final_url,
                html,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                PageVmInitStage::Load,
            );
        request.indexed_db_manager = indexed_db_manager;
        request.storage_bucket_store = storage_bucket_store;
        request.root_frame_id = root_frame_id;
        request.top_level_storage_key = top_level_storage_key;
        request.top_level_navigation_dispatch = top_level_navigation_dispatch;
        request.main_document_commit = main_document_commit;
        let reply_rx = self
            .inner
            .renderer_owner
            .enqueue_command_with_reply(RendererOwnerCommand::CreateHtmlPage(request))?;
        Ok(PendingHtmlPage {
            runtime: self.clone(),
            reply_rx,
        })
    }

    /// Reserves the exact owner-local identity of a Page before its creation
    /// command can enter the renderer queue.
    ///
    /// Protocol coordinators that must route pre-install output bind this
    /// identity to their target first, then pass it to
    /// [`Self::start_create_html_page_from_response_with_inspector_session_restores`].
    pub fn reserve_page_for_creation(&self) -> RendererPageReservationToken {
        self.inner.renderer_owner.allocate_page_reservation_token()
    }

    pub async fn create_streaming_raw_page_from_external_body(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        navigation_redirect_chain: Vec<crate::protocol_types::NavigationRedirect>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        raw_body: ExternalRawDocumentBodyStream,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        wpt_extensions_enabled: bool,
        stage: PageVmInitStage,
        top_level_navigation_dispatch: crate::RendererTopLevelNavigationDispatch,
        navigation_reply_policy: crate::RendererNavigationReplyPolicy,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        self.create_streaming_raw_page_from_external_body_with_inspector_session_restores(
            requested_url,
            final_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            navigation_redirect_chain,
            response_status,
            response_headers,
            loader,
            web_storage,
            raw_body,
            indexed_db_manager,
            storage_bucket_store,
            document_start_scripts,
            runtime_bindings,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            false,
            cpu_throttling_rate,
            emulated_media,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            Vec::new(),
            wpt_extensions_enabled,
            stage,
            crate::RendererReplyBoundary::Stage,
            top_level_navigation_dispatch,
            navigation_reply_policy,
            None,
            None,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_streaming_raw_page_from_external_body_with_inspector_session_restores(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        navigation_redirect_chain: Vec<crate::protocol_types::NavigationRedirect>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        raw_body: ExternalRawDocumentBodyStream,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        wpt_extensions_enabled: bool,
        stage: PageVmInitStage,
        reply_boundary: crate::RendererReplyBoundary,
        top_level_navigation_dispatch: crate::RendererTopLevelNavigationDispatch,
        navigation_reply_policy: crate::RendererNavigationReplyPolicy,
        root_frame_id: Option<String>,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
        main_document_commit: Option<crate::RendererMainDocumentCommit>,
        lifecycle_decider: Option<crate::RendererLifecycleDecider>,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        let prepared = self
            .prepare_streaming_raw_document_from_external_body_with_inspector_session_restores(
                self.reserve_page_for_creation(),
                requested_url,
                final_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                navigation_redirect_chain,
                response_status,
                response_headers,
                loader,
                web_storage,
                raw_body,
                indexed_db_manager,
                storage_bucket_store,
                document_start_scripts,
                runtime_bindings,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                runtime_inspector_session_restore_snapshots,
                wpt_extensions_enabled,
                stage,
                reply_boundary,
                top_level_navigation_dispatch,
                navigation_reply_policy,
                root_frame_id,
                reserved_service_worker_client,
                main_document_commit,
                lifecycle_decider,
            )
            .await?;
        let permit = prepared.issue_commit_permit();
        prepared.commit(permit).await
    }

    /// Moves a streaming response and all document bootstrap inputs onto the
    /// renderer owner lane without starting the parser or author scripts.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_streaming_raw_document_from_external_body_with_inspector_session_restores(
        &self,
        page_reservation: RendererPageReservationToken,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        navigation_redirect_chain: Vec<crate::protocol_types::NavigationRedirect>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        raw_body: ExternalRawDocumentBodyStream,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        wpt_extensions_enabled: bool,
        stage: PageVmInitStage,
        reply_boundary: crate::RendererReplyBoundary,
        top_level_navigation_dispatch: crate::RendererTopLevelNavigationDispatch,
        navigation_reply_policy: crate::RendererNavigationReplyPolicy,
        root_frame_id: Option<String>,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
        main_document_commit: Option<crate::RendererMainDocumentCommit>,
        lifecycle_decider: Option<crate::RendererLifecycleDecider>,
    ) -> Result<PreparedRendererDocument> {
        let mut request = self
            .inner
            .renderer_owner
            .build_create_streaming_raw_page_request(
                requested_url,
                final_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                navigation_redirect_chain,
                response_status,
                response_headers,
                loader,
                web_storage,
                raw_body,
                document_start_scripts,
                runtime_bindings,
                runtime_inspector_session_restore_snapshots,
                extra_http_headers,
                locale_override,
                timezone_override,
                script_execution_disabled,
                bypass_content_security_policy,
                cpu_throttling_rate,
                emulated_media,
                viewport_surface,
                network_offline,
                blocked_url_patterns,
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                stage,
            );
        request.indexed_db_manager = indexed_db_manager;
        request.storage_bucket_store = storage_bucket_store;
        request.root_frame_id = root_frame_id;
        request.main_document_commit = main_document_commit;
        request.wpt_extensions_enabled = wpt_extensions_enabled;
        request.reply_boundary = reply_boundary;
        request.top_level_navigation_dispatch = top_level_navigation_dispatch;
        request.navigation_reply_policy = navigation_reply_policy;
        request.reserved_service_worker_client = reserved_service_worker_client;
        request.lifecycle_decider = lifecycle_decider;
        let reply = self
            .inner
            .renderer_owner
            .dispatch_command(RendererOwnerCommand::PrepareStreamingRawDocument {
                token: page_reservation,
                request,
            })
            .await?;
        match reply {
            RendererOwnerReply::PreparedRendererDocumentStored {
                renderer_devtools_agent_token,
            } => Ok(PreparedRendererDocument::new(
                self.clone(),
                page_reservation,
                renderer_devtools_agent_token,
            )),
            _ => Err(anyhow!(
                "renderer owner returned non-prepare reply for prepared document request"
            )),
        }
    }
}

impl JsRuntimeOwner {
    pub fn handle(&self) -> JsRuntime {
        self.runtime
            .as_ref()
            .expect("standalone JS runtime owner was already shut down")
            .clone()
    }
}

impl std::ops::Deref for JsRuntimeOwner {
    type Target = JsRuntime;

    fn deref(&self) -> &Self::Target {
        self.runtime
            .as_ref()
            .expect("standalone JS runtime owner was already shut down")
    }
}

impl Drop for JsRuntimeOwner {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.terminate_resource_producers_for_owner_shutdown();
            // Join the renderer before the browser-context owner broadcasts
            // and joins its network runtimes. Leaving this to implicit field
            // drop would reverse that structured-concurrency order.
            drop(runtime);
        }
        self._browser_context_owner.shutdown_and_join();
    }
}

/// An opaque handle to owner-local streaming document inputs held before the
/// renderer commit barrier.
///
/// Dropping this handle schedules cancellation. Only a permit issued for this
/// exact handle can consume the owner-local residence and start bootstrap.
pub struct PreparedRendererDocument {
    runtime: JsRuntime,
    token: RendererPageReservationToken,
    renderer_devtools_agent_token: super::RendererDevToolsAgentToken,
    cancel_on_drop: bool,
}

impl PreparedRendererDocument {
    fn new(
        runtime: JsRuntime,
        token: RendererPageReservationToken,
        renderer_devtools_agent_token: super::RendererDevToolsAgentToken,
    ) -> Self {
        Self {
            runtime,
            token,
            renderer_devtools_agent_token,
            cancel_on_drop: true,
        }
    }

    pub fn token(&self) -> RendererPageReservationToken {
        self.token
    }

    pub fn renderer_devtools_agent_token(&self) -> super::RendererDevToolsAgentToken {
        self.renderer_devtools_agent_token
    }

    pub fn issue_commit_permit(&self) -> RendererDocumentCommitPermit {
        RendererDocumentCommitPermit::new(self.token)
    }

    /// Replaces the live target configuration consumed when the first
    /// execution contexts are created.
    pub async fn update_commit_configuration(
        &self,
        configuration: super::RendererPreparedDocumentCommitConfiguration,
    ) -> Result<()> {
        let reply = self
            .runtime
            .inner
            .renderer_owner
            .dispatch_command(
                RendererOwnerCommand::UpdatePreparedRendererDocumentCommitConfiguration {
                    token: self.token,
                    configuration,
                },
            )
            .await?;
        match reply {
            RendererOwnerReply::PreparedRendererDocumentCommitConfigurationUpdated => Ok(()),
            _ => Err(anyhow!(
                "renderer owner returned non-update reply for prepared document configuration"
            )),
        }
    }

    pub async fn commit(
        mut self,
        permit: RendererDocumentCommitPermit,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        anyhow::ensure!(
            permit.prepared_document() == self.token,
            "renderer document commit permit does not belong to this prepared document"
        );
        self.cancel_on_drop = false;
        let reply = self
            .runtime
            .inner
            .renderer_owner
            .dispatch_command(RendererOwnerCommand::CommitPreparedRendererDocument { permit })
            .await?;
        self.runtime
            .inner
            .renderer_owner
            .materialize_page_created_reply_parts(reply)
    }

    pub async fn cancel(mut self) -> Result<()> {
        let reply = self
            .runtime
            .inner
            .renderer_owner
            .dispatch_command(RendererOwnerCommand::CancelPreparedRendererDocument {
                token: self.token,
            })
            .await?;
        match reply {
            RendererOwnerReply::PreparedRendererDocumentCanceled => {
                self.cancel_on_drop = false;
                Ok(())
            }
            _ => Err(anyhow!(
                "renderer owner returned non-cancel reply for prepared document request"
            )),
        }
    }
}

impl Drop for PreparedRendererDocument {
    fn drop(&mut self) {
        if !self.cancel_on_drop {
            return;
        }
        let _ = self
            .runtime
            .inner
            .renderer_owner
            .enqueue_command_with_reply(RendererOwnerCommand::CancelPreparedRendererDocument {
                token: self.token,
            });
    }
}

/// Handle to an in-flight `CreateHtmlPage` renderer command kicked off via
/// [`JsRuntime::start_create_html_page_from_response`]. The renderer thread
/// is already (or about to be) processing the work; the caller `await`s
/// [`Self::await_ready`] when the resulting page is needed.
pub struct PendingHtmlPage {
    runtime: JsRuntime,
    reply_rx: tokio::sync::oneshot::Receiver<Result<RendererOwnerReply>>,
}

impl PendingHtmlPage {
    pub async fn await_ready(
        self,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        let reply = self.reply_rx.await.map_err(|_| {
            anyhow!("renderer reply channel closed before pending html page ready")
        })??;
        self.runtime
            .inner
            .renderer_owner
            .materialize_page_created_reply_parts(reply)
    }
}
