use super::*;
use crate::conn::state::{
    BrowserContextPageStorageHandles, BrowserContextResourceStorageHandles, DevToolsSessionState,
    PageNavigationHistoryEntry, PageTargetHost, RendererMainDocumentCommitSeed, TargetFetchConfig,
    TargetNetworkPolicyState, TargetOwnerState, TargetPageAbsenceReason,
    TargetPageResidenceIdentity, TargetRuntimeSessionState, TargetRuntimeSlot,
};
use crate::conn::{
    BackgroundProtocolEvent, CommandOwnerScope, ConnectionNetworkRequestIdAllocator,
    DocumentStartScript, EmulatedDeviceMetrics, FetchInterceptionPattern, FetchRequestStage,
    InitialDocumentPageInstallResult, InitialDocumentPageOwner, LoadedNavigationPageCommit,
    LoadedNavigationRendererAttachmentCommit, NETWORK_ERROR_PAGE_URL, NetworkErrorPageNavigation,
    PausedDocumentTransfer, PendingFetchAuthNavigation, PendingFetchNavigation,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, RuntimeBindingDefinition,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
#[cfg(test)]
use moli_core::page::RendererServiceWorkerVersionStatus;
use moli_core::page::{
    BidiPreloadChannelHandoff, Page, RendererInspectorSessionRestoreSnapshot,
    RendererMainDocumentCommit, RendererPageCreationArtifacts, SubresourceResourceType,
};
use moli_core::runtime::RendererBrowserContextRuntimeOwnerAccess;
use moli_fetch::BrowserNavigationRequestKind;
use moli_page_types::DevToolsSessionKey;
use url::Url;

pub(super) struct TargetSessionOwnerMut<'a> {
    pub(super) browser_context: &'a mut BrowserContext,
    pub(super) target_id: String,
    /// Frontend session that issued the command. This is distinct from
    /// `session_key`: primary Page work may originate from either the root
    /// frontend (`None`) or the Page's primary wire session.
    pub(super) command_session_id: Option<String>,
    pub(super) session_key: DevToolsSessionKey,
}

pub(super) struct TargetSessionOwnerRef<'a> {
    pub(super) browser_context: &'a BrowserContext,
    pub(super) target_id: String,
    pub(super) session_key: DevToolsSessionKey,
}

type FetchDisableStateWithSubresourceConfig = (
    super::fetch_owner::SessionOwnerPendingFetchState,
    (bool, Option<SubresourceResourceType>),
    bool,
);

fn empty_pending_fetch_state() -> super::fetch_owner::SessionOwnerPendingFetchState {
    (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
}

pub(crate) struct ClosedPageTarget {
    pub(crate) target_id: String,
    pub(crate) primary_session_id: Option<String>,
    pub(crate) attached_session_ids: Vec<String>,
}

impl ClosedPageTarget {
    pub(crate) fn inspector_detached_session_ids(&self) -> impl Iterator<Item = &str> {
        self.primary_session_id
            .as_deref()
            .into_iter()
            .chain(self.attached_session_ids.iter().map(String::as_str))
    }

    pub(crate) fn into_detach_cleanup_plan(
        self,
        reason: Option<&str>,
    ) -> crate::conn::TargetClosureCleanupPlan {
        crate::conn::TargetClosureCleanupPlan::from_primary_and_attached_sessions(
            self.target_id,
            reason,
            self.primary_session_id,
            self.attached_session_ids,
        )
    }
}

pub(super) struct TargetSessionStateMut<'a> {
    pub(super) devtools_session_state: &'a mut DevToolsSessionState,
    pub(super) network_policy: &'a mut TargetNetworkPolicyState,
    pub(super) tls_verify_host_override: &'a mut Option<bool>,
}

pub(crate) struct TargetLoadedNavigationCommitState {
    pub(crate) browser_context_id: String,
    pub(crate) runtime_frontend_enabled: bool,
    pub(crate) renderer_runtime_inspector_session_id: Option<String>,
    pub(crate) runtime_inspector_session_restore_snapshots:
        Vec<RendererInspectorSessionRestoreSnapshot>,
    pub(crate) stored_runtime_bindings: Vec<RuntimeBindingDefinition>,
    pub(crate) session_runtime_bindings: Vec<RuntimeBindingDefinition>,
    pub(crate) fetch_subresource_config: (bool, Option<moli_core::page::SubresourceResourceType>),
}

pub(crate) struct TargetNavigationRequestPreflight {
    pub(crate) frame_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) document_fetch_event_session_id: Option<String>,
    pub(crate) inherited_security_origin: String,
    pub(crate) inherited_secure_context_type: String,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) document_fetch_request_stage: Option<FetchRequestStage>,
    pub(crate) document_fetch_response_stage_candidate: bool,
    pub(crate) document_auth_required: bool,
    pub(crate) document_auth_required_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    pub(crate) document_loader_id: String,
    pub(crate) document_request_id: Option<String>,
    pub(crate) fetch_navigation_request_id: Option<String>,
}

#[derive(Clone)]
pub(crate) struct TargetNavigationStorageHandles {
    page_handles: BrowserContextPageStorageHandles,
}

impl TargetNavigationStorageHandles {
    fn from_page_handles(page_handles: BrowserContextPageStorageHandles) -> Self {
        Self { page_handles }
    }

    fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        BrowserContextResourceStorageHandles {
            cookie_store: self.page_handles.cookie_store.clone(),
            web_storage_store: self.page_handles.web_storage_store.clone(),
            session_storage_store: self.page_handles.session_storage_store.clone(),
        }
    }

    pub(crate) fn page_storage_handles(&self) -> BrowserContextPageStorageHandles {
        self.page_handles.clone()
    }
}

#[derive(Clone)]
pub(crate) struct TargetNavigationLoadInputs {
    pub(crate) browser_context_id: Option<String>,
    storage_handles: TargetNavigationStorageHandles,
    pub(crate) root_frame_id: Option<String>,
    pub(crate) renderer_runtime: RendererBrowserContextRuntimeOwnerAccess,
    /// Browser-side identity used for navigation request headers.
    pub(crate) browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    /// Renderer-agent identity exposed through the committed Document's Navigator.
    pub(crate) navigator_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    pub(crate) http_proxy_override: Option<String>,
    pub(crate) http_no_proxy_override: Option<String>,
    pub(crate) tls_verify_host_override: Option<bool>,
    pub(crate) navigation_initiator_url: Option<Url>,
    pub(crate) browser_navigation_kind: BrowserNavigationRequestKind,
    pub(crate) infer_navigation_referrer: bool,
    pub(crate) document_start_scripts: Vec<DocumentStartScript>,
    pub(crate) runtime_bindings: Vec<RuntimeBindingDefinition>,
    pub(crate) runtime_inspector_session_restore_snapshots:
        Vec<RendererInspectorSessionRestoreSnapshot>,
    pub(crate) extra_http_headers: Vec<(String, String)>,
    pub(crate) locale_override: Option<String>,
    pub(crate) timezone_override: Option<String>,
    pub(crate) script_execution_disabled: bool,
    pub(crate) bypass_content_security_policy: bool,
    pub(crate) cpu_throttling_rate: f64,
    pub(crate) emulated_media: moli_core::page::EmulatedMediaOverrides,
    pub(crate) viewport_surface: Option<moli_core::page::ViewportSurface>,
    pub(crate) network_offline: bool,
    pub(crate) bypass_service_worker: bool,
    pub(crate) cache_disabled: bool,
    pub(crate) blocked_url_patterns: Vec<String>,
    pub(crate) fetch_subresource_interception:
        (bool, Option<moli_core::page::SubresourceResourceType>),
    pub(crate) permission_overrides: Vec<moli_core::page::PermissionOverrideRegistration>,
    main_document_commit_seed: Option<RendererMainDocumentCommitSeed>,
}

impl TargetNavigationLoadInputs {
    pub(crate) fn with_main_document_commit_seed(
        mut self,
        seed: RendererMainDocumentCommitSeed,
    ) -> Self {
        self.main_document_commit_seed = Some(seed);
        self
    }

    pub(crate) fn main_document_commit_for_final_url(
        &self,
        final_url: &Url,
        network_error_page: Option<&NetworkErrorPageNavigation>,
    ) -> Option<RendererMainDocumentCommit> {
        self.main_document_commit_seed
            .as_ref()
            .map(|seed| seed.resolve(final_url, network_error_page))
    }

    pub(crate) fn page_storage_handles(&self) -> BrowserContextPageStorageHandles {
        self.storage_handles.page_storage_handles()
    }

    pub(crate) fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        self.storage_handles.resource_storage_handles()
    }

    pub(crate) fn store_response_cookie_reports(
        &self,
        response_url: &Url,
        response_headers: &[(String, String)],
    ) -> Vec<StoredCookieSetReport> {
        let mut cookie_store = self.storage_handles.page_handles.cookie_store.lock();
        cookie_store.store_response_headers_with_reports(response_url, response_headers)
    }

    pub(crate) fn request_cookie_report_for_navigation(
        &self,
        requested_url: &Url,
        request_method: &str,
        update_access_time: bool,
    ) -> Option<StoredCookieQueryReport> {
        let request_context = crate::domains::network::navigation_cookie_request_context(
            requested_url,
            request_method,
            None,
            self.navigation_initiator_url.as_ref(),
        );
        let mut cookie_store = self.storage_handles.page_handles.cookie_store.lock();
        let report = if update_access_time {
            cookie_store.cookie_access_report_for_request(requested_url, request_context)
        } else {
            cookie_store.observe_cookie_access_report_for_request(requested_url, request_context)
        };
        (!report.included_cookies.is_empty() || !report.excluded_cookies.is_empty())
            .then_some(report)
    }

    fn from_browser_context_target(browser_context: &BrowserContext, target_id: &str) -> Self {
        let target = browser_context
            .page_target(target_id)
            .expect("resolved Page target owner must remain live");
        let page_state = target;
        let mut document_start_scripts = Vec::new();
        let generated_surface_script = if browser_context.is_active_target(target_id) {
            browser_context.generated_surface_override_script_for_active_target()
        } else {
            browser_context.generated_surface_override_script_for_background_state(page_state)
        };
        if let Some(script) = generated_surface_script {
            document_start_scripts.push(script);
        }
        document_start_scripts.extend(browser_context.default_document_start_script_descriptors());
        document_start_scripts.extend(target.owner_state.document_start_scripts.iter().map(
            |(identifier, script)| {
                BrowserContext::target_document_start_script_descriptor(
                    Some(target_id),
                    identifier,
                    script,
                )
            },
        ));

        let effective_network_conditions = page_state
            .effective_emulation_state
            .network_conditions
            .or(browser_context.default_network_conditions)
            .or(browser_context.global_network_conditions);
        let emulated_device_metrics = page_state
            .effective_emulation_state
            .emulated_device_metrics
            .clone()
            .or_else(|| browser_context.default_emulated_device_metrics.clone());
        let effective_policy = page_state.effective_policy();

        Self {
            browser_context_id: Some(browser_context.id.clone()),
            storage_handles: TargetNavigationStorageHandles::from_page_handles(
                browser_context
                    .page_storage_handles_for_target(target_id)
                    .expect("resolved Page target must own session storage"),
            ),
            root_frame_id: Some(target_id.to_owned()),
            renderer_runtime: browser_context.renderer_runtime_owner_access(),
            browser_identity_override: effective_policy
                .browser_identity_override()
                .cloned()
                .or_else(|| browser_context.default_browser_identity_override_owned()),
            navigator_identity_override: page_state
                .effective_renderer_browser_identity_override_owned()
                .or_else(|| browser_context.default_browser_identity_override_owned()),
            http_proxy_override: page_state
                .http_proxy_override
                .clone()
                .or_else(|| browser_context.default_http_proxy_override.clone()),
            http_no_proxy_override: page_state
                .http_no_proxy_override
                .clone()
                .or_else(|| browser_context.default_http_no_proxy_override.clone()),
            tls_verify_host_override: page_state
                .tls_verify_host_override
                .or(browser_context.default_tls_verify_host_override),
            navigation_initiator_url: target_navigation_initiator_url(
                target.target_url(),
                target.loaded_page(),
            ),
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_navigation_referrer: true,
            document_start_scripts,
            runtime_bindings: page_state.devtools_sessions.runtime_bindings_for_renderer(),
            runtime_inspector_session_restore_snapshots: page_state
                .devtools_sessions
                .runtime_inspector_restore_snapshots(),
            extra_http_headers: browser_context
                .merged_extra_headers_for_target_policy(effective_policy.extra_headers()),
            locale_override: effective_policy
                .locale_override()
                .map(str::to_owned)
                .or_else(|| browser_context.default_locale_override.clone()),
            timezone_override: effective_policy
                .timezone_override()
                .map(str::to_owned)
                .or_else(|| browser_context.default_timezone_override.clone()),
            script_execution_disabled: page_state
                .effective_emulation_state
                .script_execution_disabled,
            bypass_content_security_policy: page_state.devtools_sessions.page_bypass_csp_enabled(),
            cpu_throttling_rate: page_state.effective_emulation_state.cpu_throttling_rate,
            emulated_media: (&page_state.effective_emulation_state.emulated_media).into(),
            viewport_surface: emulated_device_metrics
                .as_ref()
                .map(|metrics| metrics.viewport_surface().to_page_viewport_surface()),
            network_offline: page_state.network_policy.network_offline()
                || effective_network_conditions
                    .is_some_and(|conditions| !conditions.navigator_online()),
            bypass_service_worker: effective_policy.bypass_service_worker(),
            cache_disabled: effective_policy.cache_disabled(),
            blocked_url_patterns: effective_policy.blocked_url_patterns().to_vec(),
            fetch_subresource_interception: page_state
                .fetch_owner
                .subresource_interception_config(),
            permission_overrides: Vec::new(),
            main_document_commit_seed: None,
        }
    }

    fn from_browser_context_fallback(browser_context: &BrowserContext) -> Self {
        let mut inputs = TargetNavigationLoadInputs::no_loaded_browser_context(
            browser_context.page_storage_handles(),
            browser_context.renderer_runtime_owner_access(),
        );
        inputs.browser_context_id = Some(browser_context.id.clone());
        inputs.browser_identity_override =
            browser_context.effective_active_browser_identity_override_owned();
        inputs.navigator_identity_override =
            browser_context.effective_active_renderer_browser_identity_override_owned();
        inputs.http_proxy_override = browser_context.effective_active_http_proxy_override_owned();
        inputs.http_no_proxy_override =
            browser_context.effective_active_http_no_proxy_override_owned();
        inputs.tls_verify_host_override =
            browser_context.effective_active_tls_verify_host_override();
        inputs.document_start_scripts = browser_context.default_document_start_script_descriptors();
        inputs.extra_http_headers = browser_context.effective_extra_headers();
        inputs.locale_override = browser_context.effective_active_locale_override_owned();
        inputs.timezone_override = browser_context.effective_active_timezone_override_owned();
        inputs.viewport_surface = browser_context
            .default_emulated_device_metrics
            .as_ref()
            .map(|metrics| metrics.viewport_surface().to_page_viewport_surface());
        inputs.network_offline = browser_context.effective_active_network_offline();
        inputs
    }

    fn no_loaded_browser_context(
        page_handles: BrowserContextPageStorageHandles,
        renderer_runtime: RendererBrowserContextRuntimeOwnerAccess,
    ) -> Self {
        Self {
            browser_context_id: None,
            storage_handles: TargetNavigationStorageHandles::from_page_handles(page_handles),
            root_frame_id: None,
            renderer_runtime,
            browser_identity_override: None,
            navigator_identity_override: None,
            http_proxy_override: None,
            http_no_proxy_override: None,
            tls_verify_host_override: None,
            navigation_initiator_url: None,
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_navigation_referrer: true,
            document_start_scripts: Vec::new(),
            runtime_bindings: Vec::new(),
            runtime_inspector_session_restore_snapshots: Vec::new(),
            extra_http_headers: Vec::new(),
            locale_override: None,
            timezone_override: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            cpu_throttling_rate: 1.0,
            emulated_media: Default::default(),
            viewport_surface: None,
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception: (false, None),
            permission_overrides: Vec::new(),
            main_document_commit_seed: None,
        }
    }

    pub(crate) fn without_inferred_referrer(mut self) -> Self {
        self.infer_navigation_referrer = false;
        self
    }

    pub(crate) fn with_browser_navigation_kind(
        mut self,
        kind: BrowserNavigationRequestKind,
    ) -> Self {
        self.browser_navigation_kind = kind;
        self
    }
}

fn apply_referrer_header(headers: &mut Vec<(String, String)>, referrer: Option<&str>) {
    let Some(referrer) = referrer else {
        return;
    };
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("referer"));
    headers.push(("Referer".to_owned(), referrer.to_owned()));
}

fn apply_user_agent_header(headers: &mut Vec<(String, String)>, user_agent: &str) {
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        headers.push(("User-Agent".to_owned(), user_agent.to_owned()));
    }
}

fn target_navigation_initiator_url(target_url: &str, loaded_page: Option<&Page>) -> Option<Url> {
    if let Some(loaded_page) = loaded_page {
        let url = loaded_page.final_url().clone();
        if url.host_str().is_some() {
            return Some(url);
        }
    }

    let url = Url::parse(target_url).ok()?;
    url.host_str().is_some().then_some(url)
}

fn clear_page_runtime_remote_object_tracking(state: &mut crate::conn::state::PageTargetHost) {
    for session in state.devtools_sessions.states_mut() {
        session.clear_runtime_remote_object_tracking();
    }
}

fn clear_page_loaded_document_session_state(state: &mut crate::conn::state::PageTargetHost) {
    clear_page_runtime_remote_object_tracking(state);
    for session in state.devtools_sessions.states_mut() {
        session
            .page_session_state
            .clear_loaded_document_context_state();
    }
}

fn renderer_runtime_inspector_session_id(session_key: &DevToolsSessionKey) -> Option<String> {
    session_key.wire_session_id().map(str::to_owned)
}

impl<'a> TargetSessionOwnerRef<'a> {
    fn target(&self) -> &'a crate::conn::PageTargetHost {
        self.browser_context
            .page_target(&self.target_id)
            .expect("resolved Page target owner must remain live")
    }

    fn navigation_initiator_url(&self) -> Option<Url> {
        let target = self.target();
        target_navigation_initiator_url(target.target_url(), target.loaded_page())
    }

    pub(super) fn devtools_session_state(&self) -> Option<&'a DevToolsSessionState> {
        self.target().devtools_sessions.session(&self.session_key)
    }

    pub(super) fn page_session_state(&self) -> Option<&'a TargetPageSessionState> {
        self.devtools_session_state()
            .map(|state| &state.page_session_state)
    }

    pub(super) fn effective_page_bypass_csp_enabled(&self) -> bool {
        self.target().devtools_sessions.page_bypass_csp_enabled()
    }

    pub(super) fn runtime_session_state(&self) -> Option<&'a TargetRuntimeSessionState> {
        self.devtools_session_state()
            .map(|state| &state.runtime_session_state)
    }

    pub(super) fn renderer_runtime_inspector_session_id(&self) -> Option<String> {
        renderer_runtime_inspector_session_id(&self.session_key)
    }

    pub(super) fn runtime_bindings_for_renderer(&self) -> Vec<RuntimeBindingDefinition> {
        self.target()
            .devtools_sessions
            .runtime_bindings_for_renderer()
    }

    pub(super) fn target_owner_state(&self) -> &'a TargetOwnerState {
        &self.target().owner_state
    }

    pub(super) fn initial_empty_document_url_if_current(&self) -> Option<String> {
        self.target_owner_state()
            .initial_empty_document_url_if_current()
            .map(str::to_owned)
    }

    pub(super) fn initial_empty_document_storage_key_if_current(
        &self,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.target_owner_state()
            .initial_empty_document_storage_key_if_current()
            .cloned()
    }

    pub(super) fn is_on_initial_empty_document(&self) -> Option<bool> {
        self.target_owner_state().is_on_initial_empty_document()
    }

    pub(super) fn initial_empty_document_has_pending_cross_document_navigation(&self) -> bool {
        self.target_owner_state()
            .initial_empty_document_pending_cross_document_navigation()
    }

    pub(super) fn aggregate_fetch_config(&self) -> TargetFetchConfig {
        self.target().fetch_owner.config_snapshot()
    }

    pub(super) fn runtime_slot(&self) -> &'a TargetRuntimeSlot {
        self.target().runtime_slot()
    }

    pub(super) fn owner_identity(&self) -> (String, Option<String>) {
        (
            self.browser_context.id.clone(),
            Some(self.target_id.clone()),
        )
    }

    pub(super) fn primary_session_id(&self) -> Option<String> {
        self.target().session_id().map(str::to_owned)
    }

    pub(super) fn target_url(&self) -> String {
        self.target().target_url().to_owned()
    }

    pub(super) fn frame_tree_identity(&self) -> (String, String, String, String) {
        let target = self.target();
        let document_url = target
            .loaded_page()
            // Only a browser-owned network error Document diverges from the
            // user-visible Target/history URL.
            .filter(|page| page.final_url().as_str() == NETWORK_ERROR_PAGE_URL)
            .map(|page| page.final_url().to_string())
            .unwrap_or_else(|| target.target_identity().url().to_owned());
        (
            target.target_id().to_owned(),
            document_url,
            target.target_identity().security_origin().to_owned(),
            target.target_identity().secure_context_type().to_owned(),
        )
    }

    pub(super) fn frame_tree_loader_id(&self) -> Option<String> {
        self.runtime_slot()
            .committed_document_loader_id()
            .map(str::to_owned)
            .or_else(|| {
                self.target_owner_state()
                    .initial_empty_document_loader_id_if_current()
                    .map(str::to_owned)
            })
    }

    pub(super) fn emulated_device_metrics(&self) -> Option<EmulatedDeviceMetrics> {
        self.target()
            .effective_emulation_state
            .emulated_device_metrics
            .clone()
            .or_else(|| self.browser_context.default_emulated_device_metrics.clone())
    }

    pub(super) fn navigation_load_inputs(&self) -> TargetNavigationLoadInputs {
        TargetNavigationLoadInputs::from_browser_context_target(
            self.browser_context,
            &self.target_id,
        )
    }
}

impl TargetSessionStateMut<'_> {
    pub(super) fn devtools_session_state_mut(&mut self) -> &mut DevToolsSessionState {
        self.devtools_session_state
    }

    pub(super) fn page_session_state_mut(&mut self) -> &mut TargetPageSessionState {
        &mut self.devtools_session_state.page_session_state
    }

    pub(super) fn runtime_session_state_mut(&mut self) -> &mut TargetRuntimeSessionState {
        &mut self.devtools_session_state.runtime_session_state
    }

    pub(super) fn network_policy_mut(&mut self) -> &mut TargetNetworkPolicyState {
        self.network_policy
    }

    pub(super) fn tls_verify_host_override_mut(&mut self) -> &mut Option<bool> {
        self.tls_verify_host_override
    }
}

impl<'a> TargetSessionOwnerMut<'a> {
    fn target(&self) -> &crate::conn::PageTargetHost {
        self.browser_context
            .page_target(&self.target_id)
            .expect("resolved Page target owner must remain live")
    }

    pub(super) fn effective_policy(&self) -> crate::conn::state::EffectiveTargetPolicy {
        self.target().effective_policy()
    }

    fn target_mut(&mut self) -> &mut crate::conn::PageTargetHost {
        self.browser_context
            .page_target_mut(&self.target_id)
            .expect("resolved Page target owner must remain live")
    }

    fn into_target_mut(self) -> &'a mut crate::conn::PageTargetHost {
        self.browser_context
            .page_target_mut(&self.target_id)
            .expect("resolved Page target owner must remain live")
    }

    fn page_snapshot(&self) -> Option<(String, String)> {
        let target = self.target();
        target
            .runtime_slot()
            .loaded_page()
            .map(|page| (target.target_url().to_owned(), page.document_title()))
    }

    pub(super) fn target_url(&self) -> String {
        self.target().target_url().to_owned()
    }

    pub(super) fn runtime_slot_ref(&self) -> &TargetRuntimeSlot {
        self.target().runtime_slot()
    }

    pub(super) fn mutate_session_state_ref<T>(
        &mut self,
        f: impl FnOnce(TargetSessionStateMut<'_>) -> T,
    ) -> T {
        let target = self
            .browser_context
            .page_target_mut(&self.target_id)
            .expect("resolved Page target owner must remain live");
        let devtools_session_state = target.devtools_sessions.ensure_session(&self.session_key);
        f(TargetSessionStateMut {
            devtools_session_state,
            network_policy: &mut target.network_policy,
            tls_verify_host_override: &mut target.tls_verify_host_override,
        })
    }

    pub(super) fn mutate_page_state<T>(
        &mut self,
        f: impl FnOnce(&mut PageTargetHost, &DevToolsSessionKey) -> T,
    ) -> T {
        let session_key = self.session_key.clone();
        f(self.target_mut(), &session_key)
    }

    pub(super) fn mutate_session_state<T>(
        mut self,
        f: impl FnOnce(TargetSessionStateMut<'_>) -> T,
    ) -> T {
        self.mutate_session_state_ref(f)
    }

    pub(super) fn mutate_target_owner_state<T>(
        &mut self,
        f: impl FnOnce(&mut TargetOwnerState) -> T,
    ) -> T {
        f(&mut self.target_mut().owner_state)
    }

    pub(super) fn configure_fetch(
        &mut self,
        command_session_id: Option<String>,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> (bool, Option<moli_core::page::SubresourceResourceType>) {
        let target = self.target_mut();
        target
            .fetch_owner
            .configure(command_session_id, handle_auth_requests, patterns);
        target.fetch_owner.subresource_interception_config()
    }

    pub(super) fn add_network_intercept(
        &mut self,
        intercept_id: String,
        command_session_id: Option<String>,
        handle_auth_requests: bool,
        auth_url_patterns: Vec<String>,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> (bool, Option<moli_core::page::SubresourceResourceType>) {
        let target = self.target_mut();
        target.fetch_owner.add_network_intercept(
            intercept_id,
            command_session_id,
            handle_auth_requests,
            auth_url_patterns,
            patterns,
        );
        target.fetch_owner.subresource_interception_config()
    }

    pub(super) fn remove_network_intercept(
        &mut self,
        intercept_id: &str,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        let target = self.target_mut();
        target
            .fetch_owner
            .remove_network_intercept(intercept_id)
            .then(|| target.fetch_owner.subresource_interception_config())
    }

    pub(super) fn reset_fetch_config_for_session_and_drain_pending_state(
        &mut self,
        session_id: Option<&str>,
    ) -> FetchDisableStateWithSubresourceConfig {
        let target = self.target_mut();
        let previous_subresource_config = target.fetch_owner.subresource_interception_config();
        let removed = target.fetch_owner.remove_fetch_session(session_id);
        let subresource_config = target.fetch_owner.subresource_interception_config();
        let pending = if removed {
            target
                .fetch_owner
                .drain_pending_requests_for_disable_session(session_id)
        } else {
            empty_pending_fetch_state()
        };
        let page_update_required = removed && previous_subresource_config != subresource_config;
        (pending, subresource_config, page_update_required)
    }

    pub(super) fn drain_fetch_pending_state(
        &mut self,
    ) -> (
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    ) {
        self.target_mut().fetch_owner.drain_pending_requests()
    }

    pub(super) async fn mark_target_crashed_async(&mut self) -> Option<()> {
        let target = self.target_mut();
        target.owner_state.target_crash_state.mark_crashed();
        target.owner_state.navigation_history_state.clear();
        target.owner_state.clear_loaded_document_context_state();
        clear_page_loaded_document_session_state(target);
        target.fetch_owner.clear_pending();
        target.runtime_slot.clear_document_navigation_state();
        let previous = target
            .runtime_slot
            .clear_loaded_page_with_reason(TargetPageAbsenceReason::TargetCrashed);
        target.runtime_slot.reset_subresource_cursor();
        target
            .runtime_slot
            .reset_all_target_scoped_network_artifacts();
        if let Some(page) = previous {
            let _ = page.close_async().await;
        }
        Some(())
    }

    pub(super) async fn discard_loaded_page_after_failed_navigation_async(
        &mut self,
        final_url: &Url,
    ) -> Option<()> {
        let next_url = final_url.to_string();
        let security_origin = final_url.origin().ascii_serialization();
        let target = self.target_mut();
        target.set_target_url(next_url);
        target.set_target_security_origin(security_origin);
        target.owner_state.mark_initial_empty_document_exited();
        target
            .owner_state
            .clear_committed_document_navigation_state();
        clear_page_loaded_document_session_state(target);
        target.runtime_slot.clear_document_navigation_state();
        let previous = target
            .runtime_slot
            .clear_loaded_page_with_reason(TargetPageAbsenceReason::NavigationFailed);
        target.runtime_slot.reset_subresource_cursor();
        target.runtime_slot.clear_websocket_artifacts();
        if let Some(page) = previous {
            let _ = page.close_async().await;
        }
        Some(())
    }

    pub(super) fn runtime_slot_mut(&mut self) -> &mut TargetRuntimeSlot {
        &mut self.target_mut().runtime_slot
    }

    pub(super) fn into_runtime_slot_mut(self) -> &'a mut TargetRuntimeSlot {
        &mut self.into_target_mut().runtime_slot
    }

    pub(super) fn navigation_history_snapshot(
        &mut self,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        let page_snapshot = self.page_snapshot();
        Some(
            self.target_mut()
                .owner_state
                .navigation_history_snapshot(page_snapshot),
        )
    }

    pub(super) fn apply_renderer_document_title(&mut self, title: String) -> Option<bool> {
        Some(self.target_mut().owner_state.commit_document_title(title))
    }

    pub(super) fn navigation_history_entry_url(&mut self, entry_id: i32) -> Option<String> {
        let page_snapshot = self.page_snapshot();
        self.target_mut()
            .owner_state
            .navigation_history_entry_url(page_snapshot, entry_id)
    }

    pub(super) fn reset_navigation_history(&mut self) -> Option<bool> {
        let page_snapshot = self.page_snapshot();
        Some(
            self.target_mut()
                .owner_state
                .reset_navigation_history(page_snapshot),
        )
    }

    pub(super) fn can_reset_navigation_history(&mut self) -> Option<bool> {
        let page_snapshot = self.page_snapshot();
        Some(
            self.target_mut()
                .owner_state
                .can_reset_navigation_history(page_snapshot),
        )
    }

    pub(super) fn mark_next_navigation_history_replace_current(&mut self) -> Option<()> {
        self.target_mut()
            .owner_state
            .mark_next_navigation_history_replace_current();
        Some(())
    }

    pub(super) fn mark_next_navigation_history_traverse_to_entry(
        &mut self,
        entry_id: i32,
    ) -> Option<()> {
        self.target_mut()
            .owner_state
            .mark_next_navigation_history_traverse_to_entry(entry_id);
        Some(())
    }

    pub(super) fn record_same_document_navigation(
        &mut self,
        url: &Url,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) -> Option<String> {
        let next_url = url.to_string();
        let security_origin = url.origin().ascii_serialization();
        let page_snapshot = self.page_snapshot();
        let title = page_snapshot
            .as_ref()
            .map(|(_, title)| title.clone())
            .unwrap_or_default();
        let target = self.target_mut();
        target.owner_state.record_same_document_navigation_history(
            page_snapshot,
            next_url.clone(),
            title,
            history_update,
        );
        target.set_target_url(next_url);
        target.set_target_security_origin(security_origin);
        Some(target.target_id().to_owned())
    }

    pub(super) fn effective_extra_headers_for_target_policy(
        &self,
        headers: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        self.browser_context
            .merged_extra_headers_for_target_policy(&headers)
    }

    pub(super) fn prepare_navigation_request(
        &mut self,
        requested_url: &Url,
        referrer: Option<&str>,
        is_data_url: bool,
        fallback_browser_identity: &moli_browser_profile::BrowserIdentityProfile,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> Option<TargetNavigationRequestPreflight> {
        {
            let target = self.target();
            let frame_id = self.target_id.clone();
            let target_session_id = target.session_id().map(str::to_owned);
            let inherited_security_origin = target.target_identity().security_origin().to_owned();
            let inherited_secure_context_type =
                target.target_identity().secure_context_type().to_owned();
            let target_has_network_event_listeners =
                target.runtime_slot().has_network_event_listeners();
            let effective_policy = target.effective_policy();
            let mut request_headers = self
                .browser_context
                .merged_extra_headers_for_target_policy(effective_policy.extra_headers());
            let user_agent = effective_policy
                .browser_identity_override()
                .or_else(|| self.browser_context.default_browser_identity_override())
                .unwrap_or(fallback_browser_identity)
                .user_agent();
            apply_user_agent_header(&mut request_headers, user_agent);
            apply_referrer_header(&mut request_headers, referrer);
            let fetch_config = target.fetch_owner.config_snapshot();
            let fetch_snapshot = fetch_config.subresource_interception_snapshot();
            let document_request_pause = (!is_data_url)
                .then(|| {
                    fetch_snapshot
                        .matching_request_stage_pause_sessions(
                            target_session_id.as_deref(),
                            DevToolsNetworkResourceType::Document,
                            requested_url,
                        )
                        .into_iter()
                        .next()
                })
                .flatten();
            let document_response_pause = (!is_data_url)
                .then(|| {
                    fetch_snapshot
                        .matching_response_stage_pause_sessions(
                            target_session_id.as_deref(),
                            DevToolsNetworkResourceType::Document,
                            requested_url,
                        )
                        .into_iter()
                        .next()
                })
                .flatten();
            let document_fetch_response_stage_candidate =
                !is_data_url && fetch_config.has_document_response_stage_candidate();
            let document_fetch_request_stage = document_request_pause
                .as_ref()
                .map(|_| FetchRequestStage::Request)
                .or_else(|| {
                    document_response_pause
                        .as_ref()
                        .map(|_| FetchRequestStage::Response)
                })
                .or_else(|| {
                    document_fetch_response_stage_candidate.then_some(FetchRequestStage::Response)
                });
            let document_fetch_event_session_id = document_request_pause
                .as_ref()
                .and_then(|pause| pause.session_id.clone())
                .or_else(|| {
                    document_response_pause
                        .as_ref()
                        .and_then(|pause| pause.session_id.clone())
                });
            let document_auth_required =
                !is_data_url && fetch_config.matches_auth_required(requested_url);
            let document_auth_required_blocked_intercepts = if document_auth_required {
                fetch_config.matching_auth_required_network_intercepts(requested_url)
            } else {
                Vec::new()
            };
            let observes_document_request = target_has_network_event_listeners
                || (!is_data_url && (fetch_config.is_enabled() || document_auth_required));
            let needs_fetch_navigation_request_id =
                document_fetch_request_stage.is_some() || document_auth_required;
            let (document_loader_id, document_request_id, fetch_navigation_request_id) = self
                .browser_context
                .page_target_mut(&self.target_id)
                .expect("resolved Page target owner must remain live")
                .prepare_document_navigation_request_ids(
                    network_request_id_allocator,
                    target_has_network_event_listeners,
                    observes_document_request,
                    needs_fetch_navigation_request_id,
                );
            Some(TargetNavigationRequestPreflight {
                frame_id,
                session_id: target_session_id,
                document_fetch_event_session_id,
                inherited_security_origin,
                inherited_secure_context_type,
                request_headers,
                document_fetch_request_stage,
                document_fetch_response_stage_candidate,
                document_auth_required,
                document_auth_required_blocked_intercepts,
                document_loader_id,
                document_request_id,
                fetch_navigation_request_id,
            })
        }
    }

    pub(super) fn prepare_loaded_navigation_commit(
        &mut self,
    ) -> Option<TargetLoadedNavigationCommitState> {
        {
            let target = self.target();
            let page_state = target;
            let devtools_session_state = page_state.devtools_sessions.session(&self.session_key);
            Some(TargetLoadedNavigationCommitState {
                browser_context_id: self.browser_context.id.clone(),
                runtime_frontend_enabled: devtools_session_state
                    .map(|state| state.runtime_session_state.runtime_frontend_enabled)
                    .unwrap_or_default(),
                renderer_runtime_inspector_session_id: renderer_runtime_inspector_session_id(
                    &self.session_key,
                ),
                runtime_inspector_session_restore_snapshots: page_state
                    .devtools_sessions
                    .runtime_inspector_restore_snapshots(),
                stored_runtime_bindings: page_state
                    .devtools_sessions
                    .runtime_bindings_for_renderer(),
                session_runtime_bindings: devtools_session_state
                    .map(|state| state.runtime_bindings.clone())
                    .unwrap_or_default(),
                fetch_subresource_config: target.fetch_owner.subresource_interception_config(),
            })
        }
    }

    pub(super) fn commit_loaded_navigation_target_identity(
        &mut self,
        main_document_commit: &RendererMainDocumentCommit,
        target_url: &Url,
    ) -> Option<()> {
        let next_url = target_url.to_string();
        let security_origin = main_document_commit.security_origin.clone();
        let secure_context_type = main_document_commit.secure_context_type.clone();
        let target = self.target_mut();
        target.set_target_url(next_url);
        target.set_target_security_origin(security_origin);
        target.set_target_secure_context_type(secure_context_type);
        target.owner_state.mark_initial_empty_document_exited();
        Some(())
    }

    pub(super) fn clear_pending_navigation_history_update(&mut self) -> Option<()> {
        self.target_mut()
            .owner_state
            .clear_pending_navigation_history_update();
        Some(())
    }

    pub(super) async fn commit_loaded_navigation_page_async(
        &mut self,
        page: Page,
        renderer_attachment_commit: LoadedNavigationRendererAttachmentCommit,
        history_url: &Url,
    ) -> Option<anyhow::Result<LoadedNavigationPageCommit>> {
        let browser_context_id = self.browser_context.id.clone();
        Some(
            self.browser_context
                .page_target_mut(&self.target_id)
                .expect("resolved Page target owner must remain live")
                .commit_loaded_navigation_page_async(
                    &browser_context_id,
                    page,
                    renderer_attachment_commit,
                    history_url,
                )
                .await,
        )
    }
}

impl CdpConnection {
    pub(crate) fn effective_page_bypass_csp_enabled_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_ref(session_id)
            .map(|owner| owner.effective_page_bypass_csp_enabled())
    }

    pub(crate) fn navigation_load_inputs_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> TargetNavigationLoadInputs {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.navigation_load_inputs_for_owner(&owner)
    }

    pub(crate) fn navigation_load_inputs_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> TargetNavigationLoadInputs {
        let mut inputs = match self.target_session_owner_ref_for_owner(owner) {
            None => self
                .browser_context
                .as_ref()
                .map(TargetNavigationLoadInputs::from_browser_context_fallback)
                .unwrap_or_else(|| {
                    TargetNavigationLoadInputs::no_loaded_browser_context(
                        self.initial_storage_partition.page_storage_handles(),
                        self.standalone_navigation_engine
                            .ensure()
                            .browser_context_owner_access(),
                    )
                }),
            Some(owner) => owner.navigation_load_inputs(),
        };
        if let Some(browser_context_id) = inputs.browser_context_id.as_deref() {
            inputs.permission_overrides =
                self.effective_permission_overrides_for_browser_context_id(browser_context_id);
        }
        inputs
    }

    pub(crate) fn navigation_initiator_url_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<Url> {
        self.target_session_owner_ref_for_owner(owner)
            .and_then(|owner| owner.navigation_initiator_url())
    }

    pub(crate) fn prepare_navigation_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        requested_url: &Url,
        referrer: Option<&str>,
        is_data_url: bool,
    ) -> Option<TargetNavigationRequestPreflight> {
        let fallback_browser_identity = self
            .global_browser_identity_override
            .clone()
            .unwrap_or_else(|| self.base_browser_identity.clone());
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .target_session_owner_mut_for_owner(owner)
            .and_then(|mut resolved| {
                resolved.prepare_navigation_request(
                    requested_url,
                    referrer,
                    is_data_url,
                    &fallback_browser_identity,
                    &mut network_request_id_allocator,
                )
            });
        self.network_request_id_allocator = network_request_id_allocator;
        result
    }

    pub(crate) fn register_pending_fetch_navigation_request_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        pending: PendingFetchNavigation,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .register_pending_fetch_navigation_request(pending)
    }

    pub(crate) fn prepare_loaded_navigation_commit_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
    ) -> Option<TargetLoadedNavigationCommitState> {
        self.target_session_owner_mut_for_owner(owner)?
            .prepare_loaded_navigation_commit()
    }

    pub(crate) fn commit_loaded_navigation_target_identity_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        main_document_commit: &RendererMainDocumentCommit,
        target_url: &Url,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .commit_loaded_navigation_target_identity(main_document_commit, target_url)
    }

    pub(crate) async fn commit_loaded_navigation_page_for_owner_async(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        page: Page,
        renderer_attachment_commit: LoadedNavigationRendererAttachmentCommit,
        history_url: &Url,
    ) -> Option<anyhow::Result<LoadedNavigationPageCommit>> {
        self.target_session_owner_mut_for_owner(owner)?
            .commit_loaded_navigation_page_async(page, renderer_attachment_commit, history_url)
            .await
    }

    pub(crate) fn initial_document_page_owner_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<InitialDocumentPageOwner> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_owner(owner)?;
        Some(InitialDocumentPageOwner {
            browser_context_id,
            target_id: target_id?,
        })
    }

    pub(crate) async fn install_initial_loaded_page_for_page_owner_async(
        &mut self,
        owner: &InitialDocumentPageOwner,
        page: Page,
        page_creation_artifacts: RendererPageCreationArtifacts,
    ) -> Result<InitialDocumentPageInstallResult, String> {
        let Some(browser_context) = self.browser_context_by_id_mut(&owner.browser_context_id)
        else {
            let _ = page.close_async().await;
            return Ok(InitialDocumentPageInstallResult::Stale);
        };
        if !browser_context.can_install_current_initial_empty_document_page(&owner.target_id) {
            let _ = page.close_async().await;
            return Ok(InitialDocumentPageInstallResult::Stale);
        }
        let Some(target) = browser_context.page_target_mut(&owner.target_id) else {
            let _ = page.close_async().await;
            return Err("TargetNotLoaded".to_owned());
        };
        let loader_id = target
            .owner_state
            .initial_empty_document_loader_id_if_current()
            .map(str::to_owned);
        target
            .owner_state
            .mark_initial_empty_document_materialized();
        target
            .owner_state
            .clear_committed_document_navigation_state();
        clear_page_loaded_document_session_state(target);
        let previous = target.replace_loaded_page(Some(page));
        target.runtime_slot.reset_subresource_cursor();
        target.runtime_slot.clear_websocket_artifacts();
        if let Some(loader_id) = loader_id {
            let _ = target
                .runtime_slot
                .page_slot_mut()
                .bind_renderer_document_lifecycle(
                    page_creation_artifacts,
                    None,
                    owner.target_id.clone(),
                    loader_id,
                );
        }
        browser_context
            .assert_target_materialized_initial_empty_document_has_page(&owner.target_id)?;
        if let Some(page) = previous {
            let _ = page.close_async().await;
        }
        Ok(InitialDocumentPageInstallResult::Installed)
    }

    pub(crate) fn clear_pending_navigation_history_update_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .clear_pending_navigation_history_update()
    }

    pub(crate) async fn mark_target_crashed_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .mark_target_crashed_async()
            .await
    }

    pub(crate) async fn discard_loaded_page_after_failed_navigation_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        final_url: &Url,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .discard_loaded_page_after_failed_navigation_async(final_url)
            .await
    }

    async fn close_page_target_for_target_close_async(
        &mut self,
        target_id: &str,
        out: &mut Vec<BackgroundProtocolEvent>,
        reason: &'static str,
    ) -> Option<ClosedPageTarget> {
        let primary_route = self.target_session_route_for_target_id(target_id)?;
        let primary_owner = CommandOwnerScope::for_route(primary_route);
        let session_owners = self
            .page_event_session_ids_for_owner(&primary_owner)
            .into_iter()
            .map(|session_id| {
                session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| primary_owner.clone())
            })
            .collect::<Vec<_>>();
        let mut claimed_await_events = Vec::new();
        for owner in &session_owners {
            self.fail_pending_inspector_awaits_for_owner_background_events_into(
                out,
                &mut claimed_await_events,
                owner,
                reason,
            );
        }
        out.extend(claimed_await_events);

        let (
            mut target,
            primary_session_id,
            attached_session_ids,
            collected_network_data_artifacts,
        ) = {
            let browser_context = self.browser_context.as_mut()?;
            let target = browser_context.take_page_target_for_close(target_id)?;
            let collected_network_data_artifacts =
                target.runtime_slot.collected_network_data_artifacts();
            let attached_session_ids = target
                .devtools_sessions
                .attached_session_ids()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let primary_session_id = target.session_id().map(str::to_owned);
            (
                target,
                primary_session_id,
                attached_session_ids,
                collected_network_data_artifacts,
            )
        };
        target.close_page_async().await;
        self.record_collected_network_data_artifacts(collected_network_data_artifacts);

        Some(ClosedPageTarget {
            target_id: target_id.to_owned(),
            primary_session_id,
            attached_session_ids,
        })
    }

    pub(crate) async fn close_background_page_target_for_target_close_async(
        &mut self,
        target_id: &str,
        out: &mut Vec<BackgroundProtocolEvent>,
        reason: &'static str,
    ) -> Option<ClosedPageTarget> {
        let is_background = self
            .browser_context
            .as_ref()
            .is_some_and(|browser_context| {
                browser_context.page_target(target_id).is_some()
                    && !browser_context.is_active_target(target_id)
            });
        if !is_background {
            return None;
        }
        self.close_page_target_for_target_close_async(target_id, out, reason)
            .await
    }

    pub(crate) async fn close_active_page_target_for_target_close_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        reason: &'static str,
    ) -> Option<ClosedPageTarget> {
        let target_id = self.browser_context.as_ref()?.active_target_id_owned()?;
        let closed = self
            .close_page_target_for_target_close_async(&target_id, out, reason)
            .await?;
        let selected_target_id = if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.select_last_background_target_async().await
        } else {
            None
        };
        self.refresh_active_browser_context_loader_async().await;
        if let Some(selected_target_id) = selected_target_id {
            self.notify_target_host_activated(&selected_target_id);
            out.extend(
                self.page_screencast_session_ids_for_target(&selected_target_id)
                    .into_iter()
                    .map(|session_id| {
                        BackgroundProtocolEvent::page_screencast_visibility_changed(
                            session_id.as_deref(),
                            true,
                        )
                    }),
            );
        }

        Some(closed)
    }

    pub(crate) async fn rollback_incomplete_popup_target_without_event_async(
        &mut self,
        browser_context_id: Option<&str>,
        target_id: &str,
    ) {
        self.rollback_top_level_target_tab_sessions_without_event(target_id);

        let browser_context_id = browser_context_id.map(str::to_owned).or_else(|| {
            self.browser_contexts()
                .find(|browser_context| {
                    browser_context.is_active_target(target_id)
                        || browser_context.background_target(target_id).is_some()
                })
                .map(|browser_context| browser_context.id.clone())
        });

        let mut page_session_ids = Vec::new();
        if let Some(browser_context_id) = browser_context_id {
            let target = self
                .browser_context_by_id_mut(&browser_context_id)
                .and_then(|browser_context| browser_context.take_page_target_for_close(target_id));
            if let Some(mut target) = target {
                page_session_ids.extend(
                    target
                        .devtools_sessions
                        .attached_session_ids()
                        .map(str::to_owned),
                );
                if let Some(session_id) = target.session_id() {
                    page_session_ids.push(session_id.to_owned());
                }
                target.close_page_async().await;
            }
        }

        for session_id in page_session_ids {
            self.rollback_attached_session_without_event(&session_id);
        }
    }

    pub(crate) fn target_session_owner_aggregate_fetch_config_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetFetchConfig> {
        Some(
            self.target_session_owner_ref_for_owner(owner)?
                .aggregate_fetch_config(),
        )
    }

    pub(crate) fn target_page_session_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&TargetPageSessionState> {
        self.target_session_owner_ref(session_id)?
            .page_session_state()
    }

    pub(crate) fn target_page_session_state_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<&TargetPageSessionState> {
        self.target_session_owner_ref_for_owner(owner)?
            .page_session_state()
    }

    pub(crate) fn target_devtools_session_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&DevToolsSessionState> {
        self.target_session_owner_ref(session_id)?
            .devtools_session_state()
    }

    pub(crate) fn target_devtools_session_state_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<&DevToolsSessionState> {
        self.target_session_owner_ref_for_owner(owner)?
            .devtools_session_state()
    }

    pub(crate) fn target_runtime_session_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&TargetRuntimeSessionState> {
        self.target_session_owner_ref(session_id)?
            .runtime_session_state()
    }

    pub(crate) fn target_runtime_session_state_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<&TargetRuntimeSessionState> {
        self.target_session_owner_ref_for_owner(owner)?
            .runtime_session_state()
    }

    pub(crate) fn target_runtime_bindings_for_renderer_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Vec<RuntimeBindingDefinition> {
        self.target_session_owner_ref_for_owner(owner)
            .map(|owner| owner.runtime_bindings_for_renderer())
            .unwrap_or_default()
    }

    pub(crate) fn target_runtime_bindings_for_current_inspector_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Vec<RuntimeBindingDefinition> {
        self.target_session_owner_ref_for_owner(owner)
            .and_then(|owner| owner.devtools_session_state())
            .map(|state| state.runtime_bindings.clone())
            .unwrap_or_default()
    }

    pub(crate) fn target_renderer_runtime_inspector_session_id_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.renderer_runtime_inspector_session_id())
    }

    pub(crate) fn target_renderer_runtime_inspector_session_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        self.target_session_owner_ref_for_owner(owner)
            .and_then(|owner| owner.renderer_runtime_inspector_session_id())
    }

    pub(crate) fn target_owner_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&TargetOwnerState> {
        Some(
            self.target_session_owner_ref(session_id)?
                .target_owner_state(),
        )
    }

    pub(crate) fn target_owner_state_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<&TargetOwnerState> {
        Some(
            self.target_session_owner_ref_for_owner(owner)?
                .target_owner_state(),
        )
    }

    pub(crate) fn target_owner_has_bidi_channel_preload_script_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        let target_owner_has_script = self
            .target_owner_state_for_owner(owner)
            .is_some_and(TargetOwnerState::has_bidi_channel_preload_script);
        if target_owner_has_script {
            return true;
        }
        self.target_owner_identity_for_owner(owner)
            .and_then(|(browser_context_id, _)| self.browser_context_by_id(&browser_context_id))
            .is_some_and(|browser_context| {
                browser_context.has_default_bidi_channel_preload_script()
            })
    }

    pub(crate) fn target_owner_bidi_channel_preload_handoffs_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Vec<BidiPreloadChannelHandoff> {
        let mut handoffs = Vec::new();
        if let Some(owner_state) = self.target_owner_state_for_owner(owner) {
            handoffs.extend(
                owner_state
                    .document_start_scripts
                    .iter()
                    .flat_map(|(_, script)| script.bidi_channel_handoffs.clone()),
            );
        }
        if let Some(browser_context) = self
            .target_owner_identity_for_owner(owner)
            .and_then(|(browser_context_id, _)| self.browser_context_by_id(&browser_context_id))
        {
            handoffs.extend(
                browser_context
                    .default_document_start_scripts
                    .iter()
                    .flat_map(|(_, script)| script.bidi_channel_handoffs.clone()),
            );
        }
        handoffs
    }

    pub(crate) fn with_target_owner_state_for_session_mut<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut TargetOwnerState) -> R,
    ) -> Option<R> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.with_target_owner_state_for_owner_mut(&owner, f)
    }

    pub(crate) fn with_target_owner_state_for_owner_mut<R>(
        &mut self,
        owner: &CommandOwnerScope,
        f: impl FnOnce(&mut TargetOwnerState) -> R,
    ) -> Option<R> {
        Some(
            self.target_session_owner_mut_for_owner(owner)?
                .mutate_target_owner_state(f),
        )
    }

    pub(crate) fn with_target_devtools_session_state_for_session_mut<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut DevToolsSessionState) -> R,
    ) -> Option<R> {
        self.target_session_owner_mut(session_id)?
            .mutate_session_state(|mut state| Some(f(state.devtools_session_state_mut())))
    }

    pub(crate) fn with_target_devtools_session_state_for_owner_mut<R>(
        &mut self,
        owner: &CommandOwnerScope,
        f: impl FnOnce(&mut DevToolsSessionState) -> R,
    ) -> Option<R> {
        self.target_session_owner_mut_for_owner(owner)?
            .mutate_session_state(|mut state| Some(f(state.devtools_session_state_mut())))
    }

    pub(crate) fn target_owner_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<(String, Option<String>)> {
        Some(self.target_session_owner_ref(session_id)?.owner_identity())
    }

    pub(crate) fn target_owner_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, Option<String>)> {
        if owner.session_id().is_none()
            && let Some(CdpSessionRoute::BrowserContext { browser_context_id }) =
                owner.explicit_route()
        {
            return self
                .browser_context_by_id(browser_context_id)
                .map(|_| (browser_context_id.clone(), None));
        }
        Some(
            self.target_session_owner_ref_for_owner(owner)?
                .owner_identity(),
        )
    }

    /// Captures the exact target-local Page residence currently addressed by
    /// `session_id`.
    ///
    /// The connection actor is mutably borrowed while a renderer output is
    /// taken, so callers can capture this identity immediately before starting
    /// that Page command and attach it to the returned prepared payload. A
    /// target without an installed Page has no current Page residence.
    pub(crate) fn target_page_residence_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetPageResidenceIdentity> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_page_residence_identity_for_owner(&owner)
    }

    pub(crate) fn target_page_residence_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetPageResidenceIdentity> {
        let (browser_context_id, routed_target_id) = self.target_owner_identity_for_owner(owner)?;
        // Context-scoped work may not carry a Page id. Freeze its concrete
        // active target before an async boundary so later foreground changes
        // cannot redirect output from the old Page.
        let target_id = routed_target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned)
        });
        let page_attachment_id = self
            .runtime_session_owner_slot_for_owner(owner)
            .ok()?
            .page_attachment_id()?;
        Some(TargetPageResidenceIdentity::new(
            browser_context_id,
            target_id,
            page_attachment_id,
        ))
    }

    /// Captures the reserved residence of the Page currently being built for
    /// `session_id`.
    ///
    /// Renderer construction can open and publish into its output stream
    /// before protocol commits that Page into the target slot. The reservation
    /// therefore owns an explicit attachment id before renderer work starts;
    /// callers never predict that identity from mutable current-Page state.
    pub(crate) fn pending_target_page_residence_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetPageResidenceIdentity> {
        let (browser_context_id, routed_target_id) = self.target_owner_identity_for_owner(owner)?;
        let target_id = routed_target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned)
        });
        let page_attachment_id = self
            .runtime_session_owner_slot_for_owner(owner)
            .ok()?
            .pending_page_attachment_id()?;
        Some(TargetPageResidenceIdentity::new(
            browser_context_id,
            target_id,
            page_attachment_id,
        ))
    }

    pub(crate) fn reserve_target_page_residence_identity_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        renderer_page: crate::conn::RendererPageResidenceIdentity,
    ) -> Option<TargetPageResidenceIdentity> {
        let (browser_context_id, routed_target_id) = self.target_owner_identity_for_owner(owner)?;
        let target_id = routed_target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned)
        });
        let page_attachment_id = self
            .runtime_session_owner_slot_mut_for_owner(owner)
            .ok()?
            .reserve_renderer_page_attachment(renderer_page);
        Some(TargetPageResidenceIdentity::new(
            browser_context_id,
            target_id,
            page_attachment_id,
        ))
    }

    /// Checks that deferred Page-owned work still addresses the same target
    /// Page-slot residence from which it was captured.
    pub(crate) fn target_page_residence_identity_is_current(
        &self,
        expected: &TargetPageResidenceIdentity,
    ) -> bool {
        self.browser_context_by_id(expected.browser_context_id())
            .is_some_and(|browser_context| {
                browser_context.target_page_residence_is_current(expected)
            })
    }

    pub(crate) fn capture_target_page_residence_token_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<crate::conn::TargetPageResidenceToken> {
        self.runtime_session_owner_slot_mut_for_owner(owner)
            .ok()?
            .page_slot_mut()
            .page_residence_token()
    }

    /// Captures the exact protocol attachment currently addressing a Page.
    pub(crate) fn target_page_protocol_attachment_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::TargetPageProtocolAttachmentIdentity> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_page_protocol_attachment_identity_for_owner(&owner)
    }

    pub(crate) fn target_page_protocol_attachment_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<crate::conn::TargetPageProtocolAttachmentIdentity> {
        Some(crate::conn::TargetPageProtocolAttachmentIdentity::new(
            self.target_page_residence_identity_for_owner(owner)?,
            owner.session_id().map(str::to_owned),
        ))
    }

    /// Captures the renderer-side identity of the Page currently addressed by
    /// `session_id`.
    ///
    /// The returned identity is self-contained and must not be reconstructed
    /// later from a session that may then address another Page.
    #[cfg(test)]
    pub(crate) fn renderer_page_residence_identity_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::RendererPageResidenceIdentity> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.renderer_page_residence_identity_for_owner(&owner)
    }

    pub(crate) fn renderer_page_residence_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<crate::conn::RendererPageResidenceIdentity> {
        self.runtime_session_owner_slot_for_owner(owner)
            .ok()?
            .loaded_page()
            .map(crate::conn::RendererPageResidenceIdentity::from_page)
    }

    /// Resolves the exact protocol attachment named by one renderer inspector
    /// output route, while proving that it still belongs to the Page whose
    /// source snapshot is being captured.
    ///
    /// The renderer uses `None` for the target's primary inspector session and
    /// the concrete CDP session id for attached sessions. Deferred protocol
    /// output must translate that renderer-local convention once, at capture
    /// time. Looking it up again during drain could route an old Page's
    /// response or notification through a replacement Page or an unrelated
    /// contextual command session.
    pub(crate) fn target_page_protocol_attachment_identity_for_renderer_inspector_owner(
        &self,
        source_owner: &crate::conn::CommandOwnerScope,
        renderer_inspector_session_id: Option<&str>,
    ) -> Option<crate::conn::TargetPageProtocolAttachmentIdentity> {
        let source = self.target_page_protocol_attachment_identity_for_owner(source_owner)?;
        let protocol_session_id = renderer_inspector_session_id
            .map(str::to_owned)
            .or_else(|| self.runtime_session_owner_primary_session_id_for_owner(source_owner));
        let protocol_owner = protocol_session_id
            .as_deref()
            .map(CommandOwnerScope::for_session)
            .unwrap_or_else(|| source_owner.clone());
        let attachment =
            self.target_page_protocol_attachment_identity_for_owner(&protocol_owner)?;
        if attachment.page_owner() != source.page_owner()
            || self
                .target_renderer_runtime_inspector_session_id_for_owner(&protocol_owner)
                .as_deref()
                != renderer_inspector_session_id
        {
            return None;
        }
        Some(attachment)
    }

    /// Checks both the target Page residence and the session that originally
    /// captured an attachment-sensitive output.
    pub(crate) fn target_page_protocol_attachment_identity_is_current(
        &self,
        expected: &crate::conn::TargetPageProtocolAttachmentIdentity,
    ) -> bool {
        if let Some(session_id) = expected.session_id() {
            return self
                .target_page_protocol_attachment_identity_for_session(Some(session_id))
                .as_ref()
                == Some(expected);
        }
        self.target_page_residence_identity_is_current(expected.page_owner())
    }

    /// Binds renderer-produced child-frame activity to the Page attachment
    /// that captured it and to the exact root Document reported by the same
    /// renderer snapshot.
    pub(crate) fn target_root_document_protocol_attachment_identity_for_session(
        &self,
        session_id: Option<&str>,
        root_document: moli_core::RendererDocumentLifecycleIdentity,
    ) -> Option<crate::conn::TargetRootDocumentProtocolAttachmentIdentity> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_root_document_protocol_attachment_identity_for_owner(&owner, root_document)
    }

    pub(crate) fn target_root_document_protocol_attachment_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
        root_document: moli_core::RendererDocumentLifecycleIdentity,
    ) -> Option<crate::conn::TargetRootDocumentProtocolAttachmentIdentity> {
        let binding = crate::conn::TargetRootDocumentProtocolAttachmentIdentity::new(
            self.target_page_protocol_attachment_identity_for_owner(owner)?,
            root_document,
        );
        self.target_root_document_protocol_attachment_identity_is_current(&binding)
            .then_some(binding)
    }

    /// Authorizes deferred child-frame owner actions only while both the
    /// protocol attachment and the root renderer Document remain exact.
    pub(crate) fn target_root_document_protocol_attachment_identity_is_current(
        &self,
        expected: &crate::conn::TargetRootDocumentProtocolAttachmentIdentity,
    ) -> bool {
        if !self.target_page_protocol_attachment_identity_is_current(expected.attachment()) {
            return false;
        }
        self.runtime_session_owner_slot(expected.session_id())
            .ok()
            .and_then(|slot| slot.page_slot().renderer_document_lifecycle_binding())
            .map(crate::conn::CommittedRendererDocumentBinding::renderer_document_identity)
            == Some(expected.root_document())
    }

    pub(crate) fn target_root_document_lifecycle_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<moli_core::RendererDocumentLifecycleIdentity> {
        self.runtime_session_owner_slot_for_owner(owner)
            .ok()?
            .page_slot()
            .renderer_document_lifecycle_binding()
            .map(crate::conn::CommittedRendererDocumentBinding::renderer_document_identity)
    }

    /// Resolves one concrete Page attachment for a target-owned event.
    ///
    /// The primary Page session is preferred, followed by a stable attached
    /// session. The implicit `None` attachment is valid only for the currently
    /// active browser context and target; an unattached background target has
    /// no protocol destination.
    pub(crate) fn target_page_protocol_attachment_identity_for_target(
        &self,
        browser_context_id: &str,
        target_id: &str,
    ) -> Option<crate::conn::TargetPageProtocolAttachmentIdentity> {
        let browser_context = self.browser_context_by_id(browser_context_id)?;
        let primary_session_id = if browser_context.is_active_target(target_id) {
            browser_context.active_session_id_owned()
        } else {
            browser_context
                .background_target(target_id)?
                .session_id()
                .map(str::to_owned)
        };
        let session_id = primary_session_id
            .or_else(|| {
                browser_context
                    .attached_session_ids_for_target(target_id)
                    .into_iter()
                    .next()
            })
            .map(Some)
            .or_else(|| {
                (browser_context.is_active_target(target_id)
                    && self
                        .browser_context
                        .as_ref()
                        .is_some_and(|active| active.id == browser_context_id))
                .then_some(None)
            })?;
        let attachment =
            self.target_page_protocol_attachment_identity_for_session(session_id.as_deref())?;
        (attachment.page_owner().browser_context_id() == browser_context_id
            && attachment.page_owner().target_id() == Some(target_id))
        .then_some(attachment)
    }

    pub(crate) fn runtime_context_owner_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<(String, Option<String>)> {
        if let Some(
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            }
            | CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            }
            | CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            },
        ) = self.session_route(session_id)
        {
            return Some((browser_context_id, Some(target_id)));
        }
        self.target_owner_identity_for_session(session_id)
    }

    pub(crate) fn target_devtools_attached_session_id_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<Option<String>> {
        let owner = self.target_session_owner(session_id)?;
        Some(owner.session_key.wire_session_id().map(str::to_owned))
    }

    pub(crate) fn runtime_session_owner_slot_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut TargetRuntimeSlot, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_slot_mut_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_slot_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<&mut TargetRuntimeSlot, String> {
        let renderer_inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let slot = self
            .target_session_owner_mut_for_owner(owner)
            .map(TargetSessionOwnerMut::into_runtime_slot_mut)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        if let Some(page) = slot.loaded_page_mut() {
            page.set_renderer_devtools_command_session_id(renderer_inspector_session_id);
        }
        Ok(slot)
    }

    pub(crate) fn runtime_session_owner_slot(
        &self,
        session_id: Option<&str>,
    ) -> Result<&TargetRuntimeSlot, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_slot_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_slot_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<&TargetRuntimeSlot, String> {
        self.target_session_owner_ref_for_owner(owner)
            .map(|owner| owner.runtime_slot())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())
    }

    pub(crate) fn runtime_session_owner_primary_session_id(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_primary_session_id_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_primary_session_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        self.target_session_owner_ref_for_owner(owner)?
            .primary_session_id()
    }

    pub(crate) fn page_event_session_ids_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Vec<Option<String>> {
        let Some((browser_context_id, target_id)) = self.target_owner_identity_for_owner(owner)
        else {
            return vec![owner.session_id().map(str::to_owned)];
        };
        let Some(browser_context) = self.browser_context_by_id(&browser_context_id) else {
            return vec![owner.session_id().map(str::to_owned)];
        };
        let Some(target_id) = target_id else {
            return vec![owner.session_id().map(str::to_owned)];
        };

        let mut session_ids = Vec::new();
        let primary_session_id = if browser_context.active_target_id() == Some(target_id.as_str()) {
            browser_context.active_session_id_owned()
        } else {
            browser_context
                .background_target(&target_id)
                .and_then(|target| target.session_id().map(str::to_owned))
        };
        let primary_event_session_id =
            primary_session_id.or_else(|| owner.session_id().map(str::to_owned));
        session_ids.push(primary_event_session_id.clone());
        for attached_session_id in browser_context.attached_session_ids_for_target(&target_id) {
            if primary_event_session_id.as_deref() != Some(attached_session_id.as_str()) {
                session_ids.push(Some(attached_session_id));
            }
        }
        session_ids
    }

    pub(crate) fn subscribed_page_event_session_ids_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.subscribed_page_event_session_ids_for_owner(&owner)
    }

    pub(crate) fn subscribed_page_event_session_ids_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Vec<Option<String>> {
        self.page_event_session_ids_for_owner(owner)
            .into_iter()
            .filter(|event_session_id| {
                let event_owner = event_session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| owner.clone());
                self.target_page_session_state_for_owner(&event_owner)
                    .is_some_and(|state| state.page_domain_enabled)
            })
            .collect()
    }

    /// Captures every exact Page attachment that should observe one Page
    /// event produced for `session_id`'s owner.
    ///
    /// The returned identities freeze both the capture-time session and the
    /// Page-slot residence. A deferred output may later authorize these
    /// identities, but must never call `page_event_session_ids_for_session_owner`
    /// again: doing so could route an old Page's historical event through a
    /// replacement Page or a newly active implicit attachment.
    pub(crate) fn page_event_protocol_attachments_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<Vec<crate::conn::TargetPageProtocolAttachmentIdentity>> {
        let source = self.target_page_protocol_attachment_identity_for_owner(owner)?;
        let attachments = self
            .subscribed_page_event_session_ids_for_owner(owner)
            .into_iter()
            .map(|event_session_id| {
                let event_owner = event_session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| owner.clone());
                self.target_page_protocol_attachment_identity_for_owner(&event_owner)
            })
            .collect::<Option<Vec<_>>>()?;
        (!attachments.is_empty()
            && attachments
                .iter()
                .all(|attachment| attachment.page_owner() == source.page_owner()))
        .then_some(attachments)
    }

    /// Captures every exact attachment that had enabled the CDP `Runtime`
    /// domain when one target-owned Runtime fact was ingested.
    ///
    /// The renderer publishes asynchronous exceptions once per target, not
    /// once per Inspector session. Freeze the audience at ingress so a later
    /// detach, target replacement, or `Runtime.enable` cannot retarget that
    /// historical fact.
    pub(crate) fn runtime_event_protocol_attachments_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<Vec<crate::conn::TargetPageProtocolAttachmentIdentity>> {
        let source = self.target_page_protocol_attachment_identity_for_owner(owner)?;
        let attachments = self
            .page_event_session_ids_for_owner(owner)
            .into_iter()
            .filter(|event_session_id| {
                let event_owner = event_session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| owner.clone());
                self.target_runtime_session_state_for_owner(&event_owner)
                    .is_some_and(|state| state.runtime_frontend_enabled)
            })
            .map(|event_session_id| {
                let event_owner = event_session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| owner.clone());
                self.target_page_protocol_attachment_identity_for_owner(&event_owner)
            })
            .collect::<Option<Vec<_>>>()?;
        attachments
            .iter()
            .all(|attachment| attachment.page_owner() == source.page_owner())
            .then_some(attachments)
    }

    pub(crate) fn runtime_session_owner_target_url(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_target_url_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_target_url_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        Some(self.target_session_owner_ref_for_owner(owner)?.target_url())
    }

    pub(crate) fn runtime_session_owner_record_initial_empty_document_url_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        self.target_session_owner_ref_for_owner(owner)?
            .initial_empty_document_url_if_current()
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_storage_key(
        &self,
        session_id: Option<&str>,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.target_session_owner_ref(session_id)?
            .initial_empty_document_storage_key_if_current()
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_storage_key_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.target_session_owner_ref_for_owner(owner)?
            .initial_empty_document_storage_key_if_current()
    }

    pub(crate) fn runtime_session_owner_record_is_on_initial_empty_document_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<bool> {
        self.target_session_owner_ref_for_owner(owner)?
            .is_on_initial_empty_document()
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_pending_cross_document_navigation_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        self.target_session_owner_ref_for_owner(owner)
            .is_some_and(|owner| {
                owner.initial_empty_document_has_pending_cross_document_navigation()
            })
    }

    pub(crate) fn target_session_owner_frame_tree_identity(
        &self,
        session_id: Option<&str>,
    ) -> Option<(String, String, String, String)> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_session_owner_frame_tree_identity_for_owner(&owner)
    }

    pub(crate) fn target_session_owner_frame_tree_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, String, String, String)> {
        Some(
            self.target_session_owner_ref_for_owner(owner)?
                .frame_tree_identity(),
        )
    }

    pub(crate) fn target_session_owner_frame_tree_loader_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        self.target_session_owner_ref_for_owner(owner)?
            .frame_tree_loader_id()
    }

    pub(crate) fn target_session_owner_emulated_device_metrics(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::EmulatedDeviceMetrics> {
        self.target_session_owner_ref(session_id)?
            .emulated_device_metrics()
    }

    pub(crate) fn target_session_owner_emulated_device_metrics_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<crate::conn::EmulatedDeviceMetrics> {
        self.target_session_owner_ref_for_owner(owner)?
            .emulated_device_metrics()
    }

    pub(crate) fn target_session_owner_navigation_history_snapshot(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_session_owner_navigation_history_snapshot_for_owner(&owner)
    }

    pub(crate) fn target_session_owner_navigation_history_snapshot_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        self.target_session_owner_mut_for_owner(owner)?
            .navigation_history_snapshot()
    }

    pub(crate) fn apply_renderer_document_title_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        change: &moli_core::RendererDocumentTitleChanged,
    ) -> Option<bool> {
        self.target_root_document_protocol_attachment_identity_for_owner(
            owner,
            change.source_document,
        )?;
        self.target_session_owner_mut_for_owner(owner)?
            .apply_renderer_document_title(change.title.clone())
    }

    pub(crate) fn target_session_owner_navigation_history_entry_url(
        &mut self,
        session_id: Option<&str>,
        entry_id: i32,
    ) -> Option<String> {
        self.target_session_owner_mut(session_id)?
            .navigation_history_entry_url(entry_id)
    }

    pub(crate) fn reset_navigation_history_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<bool> {
        self.target_session_owner_mut_for_owner(owner)?
            .reset_navigation_history()
    }

    pub(crate) fn can_reset_navigation_history_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_mut(session_id)?
            .can_reset_navigation_history()
    }

    pub(crate) fn mark_next_navigation_history_replace_current_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .mark_next_navigation_history_replace_current()
    }

    pub(crate) fn mark_next_navigation_history_traverse_to_entry_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        entry_id: i32,
    ) -> Option<()> {
        self.target_session_owner_mut_for_owner(owner)?
            .mark_next_navigation_history_traverse_to_entry(entry_id)
    }

    pub(crate) fn record_same_document_navigation_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        url: &Url,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) -> Option<String> {
        self.target_session_owner_mut_for_owner(owner)?
            .record_same_document_navigation(url, history_update)
    }

    pub(super) fn target_session_owner_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwnerMut<'_>> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_session_owner_mut_for_owner(&owner)
    }

    pub(super) fn target_session_owner_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetSessionOwnerMut<'_>> {
        let resolved = self.target_session_owner_for_owner(owner)?;
        self.browser_context_by_id_mut(&resolved.browser_context_id)
            .map(|browser_context| TargetSessionOwnerMut {
                browser_context,
                target_id: resolved.target_id,
                command_session_id: owner.session_id().map(str::to_owned),
                session_key: resolved.session_key,
            })
    }

    pub(super) fn target_session_owner_ref(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwnerRef<'_>> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_session_owner_ref_for_owner(&owner)
    }

    pub(super) fn target_session_owner_ref_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetSessionOwnerRef<'_>> {
        let resolved = self.target_session_owner_for_owner(owner)?;
        self.browser_context_by_id(&resolved.browser_context_id)
            .map(|browser_context| TargetSessionOwnerRef {
                browser_context,
                target_id: resolved.target_id,
                session_key: resolved.session_key,
            })
    }

    pub(super) fn with_target_session_owner_mut<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(TargetSessionOwnerMut<'_>) -> R,
    ) -> Option<R> {
        self.target_session_owner_mut(session_id).map(f)
    }

    pub(super) fn with_target_session_owner_mut_for_owner<R>(
        &mut self,
        owner: &CommandOwnerScope,
        f: impl FnOnce(TargetSessionOwnerMut<'_>) -> R,
    ) -> Option<R> {
        self.target_session_owner_mut_for_owner(owner).map(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_browser_identity(user_agent: &str) -> moli_browser_profile::BrowserIdentityProfile {
        moli_browser_profile::BrowserIdentityProfile::new(
            user_agent,
            moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE,
        )
    }
    use crate::conn::{
        FetchInterceptionPattern, FetchRequestStage, PendingSubresourceFetchOwnerKind,
        PendingSubresourceFetchRequest, ServiceWorkerTargetState,
    };
    use crate::testing::TestContext;
    use moli_core::page::SubresourceResourceType;
    use url::Url;

    #[test]
    fn generated_session_id_skips_caller_supplied_live_session() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-session-id".to_owned());
        browser_context.set_active_target_id("TID-session-id");
        browser_context.attach_active_session("SID-1".to_owned());
        conn.install_browser_context_fixture_for_test(browser_context);

        assert_eq!(conn.gen_session_id(), "SID-2");
        assert_eq!(
            conn.target_owner_identity_for_session(Some("SID-1"))
                .and_then(|(_, target_id)| target_id),
            Some("TID-session-id".to_owned())
        );
    }

    fn pending_subresource_fetch(internal_id: u64) -> PendingSubresourceFetchRequest {
        PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(
                crate::conn::TargetPageResidenceIdentity::new_for_test(
                    "BID-target-session-owner".to_owned(),
                    Some("TID-frame".to_owned()),
                    1,
                ),
            ),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id,
            network_request_id: format!("REQ-{internal_id}"),
            network_request_handle: None,
            frame_id: "TID-frame".to_owned(),
            document_url: Url::parse("https://example.test/page").unwrap(),
            resource_type: SubresourceResourceType::Fetch,
            websocket_socket_id: None,
            request_stage_chain: None,
        }
    }

    fn background_target_context() -> BrowserContext {
        let mut context = BrowserContext::new("BID-background".to_owned());
        context.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            None,
            "about:blank".to_owned(),
        ));
        context
    }

    #[test]
    fn target_session_owner_mut_mutates_active_and_background_owner_state() {
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut active,
                target_id: "TID-active".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            owner.mutate_target_owner_state(|owner_state| {
                owner_state.target_crash_state.mark_crashed();
            });
        }
        assert!(
            active
                .active_page_target()
                .owner_state
                .target_crash_state
                .is_crashed()
        );

        let mut background = background_target_context();
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            owner.mutate_target_owner_state(|owner_state| {
                owner_state.target_crash_state.mark_crashed();
            });
        }
        assert!(
            background
                .background_target("TID-background")
                .expect("background target must exist")
                .owner_state
                .target_crash_state
                .is_crashed()
        );
    }

    #[test]
    fn renderer_runtime_inspector_session_id_tracks_owner_session_kind() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner-key".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active-primary".to_owned());
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        browser_context.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background-primary".to_owned()),
            "about:blank#background".to_owned(),
        ));
        browser_context
            .background_target_mut("TID-background")
            .expect("background target")
            .runtime_slot
            .set_page_attachment_id_for_test(2);
        assert!(
            browser_context
                .assign_attached_session_to_target("TID-active", "SID-active-attached".to_owned(),)
        );
        assert!(browser_context.assign_attached_session_to_target(
            "TID-background",
            "SID-background-attached".to_owned(),
        ));
        conn.install_browser_context_fixture_for_test(browser_context);

        assert_eq!(
            conn.target_renderer_runtime_inspector_session_id_for_session(None),
            None,
            "none-session active target commands use the default renderer inspector session"
        );
        assert_eq!(
            conn.target_renderer_runtime_inspector_session_id_for_session(Some(
                "SID-active-primary"
            )),
            None,
            "primary active-target session uses the target's default renderer inspector session"
        );
        assert_eq!(
            conn.target_renderer_runtime_inspector_session_id_for_session(Some(
                "SID-background-primary"
            )),
            None,
            "primary background-target session uses the target's default renderer inspector session"
        );
        assert_eq!(
            conn.target_renderer_runtime_inspector_session_id_for_session(Some(
                "SID-active-attached"
            )),
            Some("SID-active-attached".to_owned()),
            "attached active-target session owns a distinct renderer inspector session"
        );
        assert_eq!(
            conn.target_renderer_runtime_inspector_session_id_for_session(Some(
                "SID-background-attached"
            )),
            Some("SID-background-attached".to_owned()),
            "attached background-target session owns a distinct renderer inspector session"
        );

        let default_route = conn
            .target_page_protocol_attachment_identity_for_renderer_inspector_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-active-attached"),
                None,
            )
            .expect("default inspector route should resolve through the target primary session");
        assert_eq!(default_route.session_id(), Some("SID-active-primary"));

        let attached_route = conn
            .target_page_protocol_attachment_identity_for_renderer_inspector_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-active-primary"),
                Some("SID-active-attached"),
            )
            .expect("attached inspector route should retain its exact protocol attachment");
        assert_eq!(attached_route.session_id(), Some("SID-active-attached"));

        assert!(
            conn.target_page_protocol_attachment_identity_for_renderer_inspector_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-active-primary"),
                Some("SID-background-attached"),
            )
            .is_none(),
            "a renderer batch must not borrow an inspector session attached to another target"
        );
    }

    #[test]
    fn target_session_owner_mut_mutates_active_and_background_fetch_state() {
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut active,
                target_id: "TID-active".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-active".to_owned(),
                pending_subresource_fetch(1),
            ));
        }
        assert!(
            active
                .active_page_target()
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-active")
        );

        let mut background = background_target_context();
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-background".to_owned(),
                pending_subresource_fetch(2),
            ));
        }
        assert!(
            background
                .background_target("TID-background")
                .expect("background target must exist")
                .fetch_owner
                .pending_state()
                .has_pending_subresource_fetch_for_test("FETCH-background")
        );
    }

    #[test]
    fn target_session_owner_mut_configures_and_resets_active_and_background_fetch_state() {
        let patterns = vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: None,
            request_stage: FetchRequestStage::Request,
        }];

        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut active,
                target_id: "TID-active".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            assert_eq!(
                owner.configure_fetch(Some("SID-active".to_owned()), true, patterns.clone()),
                (true, None)
            );
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-active".to_owned(),
                pending_subresource_fetch(11),
            ));
            let (pending, subresource_config, page_update_required) =
                owner.reset_fetch_config_for_session_and_drain_pending_state(Some("SID-active"));
            assert_eq!(subresource_config, (false, None));
            assert!(page_update_required);
            assert_eq!(pending.3.len(), 1);
        }
        assert!(
            !active
                .active_page_target()
                .fetch_owner
                .config_snapshot()
                .is_enabled()
        );
        assert!(
            !active
                .active_page_target()
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-active")
        );

        let mut background = background_target_context();
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            assert_eq!(
                owner.configure_fetch(Some("SID-background".to_owned()), true, patterns),
                (true, None)
            );
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-background".to_owned(),
                pending_subresource_fetch(22),
            ));
            let (pending, subresource_config, page_update_required) = owner
                .reset_fetch_config_for_session_and_drain_pending_state(Some("SID-background"));
            assert_eq!(subresource_config, (false, None));
            assert!(page_update_required);
            assert_eq!(pending.3.len(), 1);
        }
        assert!(
            background
                .background_target("TID-background")
                .filter(|target| target.has_non_default_session_state())
                .is_none_or(|state| !state.fetch_owner.is_enabled())
        );
        assert!(
            background
                .background_target("TID-background")
                .expect("background target must exist")
                .fetch_owner
                .pending_state()
                .is_empty()
        );
    }

    #[test]
    fn target_session_owner_mut_snapshots_active_and_background_navigation_history() {
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        active
            .active_page_target_mut()
            .owner_state
            .record_loaded_page_navigation_history((
                "https://active.example/".to_owned(),
                "active".to_owned(),
            ));
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut active,
                target_id: "TID-active".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            let (current_index, entries) = owner
                .navigation_history_snapshot()
                .expect("active history should snapshot");
            assert_eq!(current_index, 0);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].url, "https://active.example/");
        }

        let mut background = background_target_context();
        background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .owner_state
            .record_loaded_page_navigation_history((
                "https://background.example/".to_owned(),
                "background".to_owned(),
            ));
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            let (current_index, entries) = owner
                .navigation_history_snapshot()
                .expect("background history should snapshot");
            assert_eq!(current_index, 0);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].url, "https://background.example/");
        }
    }

    #[test]
    fn target_session_owner_mut_prepares_active_navigation_request_preflight() {
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "FRAME-0");
        active
            .active_page_target_mut()
            .runtime_slot
            .enable_primary_network_events();
        active.record_captured_response_body(
            "REQ-old".to_owned(),
            "old body".to_owned(),
            vec![None],
        );
        assert!(active.has_captured_response_body_for_test("REQ-old"));

        let mut owner = TargetSessionOwnerMut {
            browser_context: &mut active,
            target_id: "FRAME-0".to_owned(),
            command_session_id: Some("SID-active".to_owned()),
            session_key: DevToolsSessionKey::Primary,
        };
        owner.configure_fetch(
            Some("SID-active".to_owned()),
            false,
            vec![FetchInterceptionPattern {
                url_pattern: "*".to_owned(),
                resource_type_filter: None,
                request_stage: FetchRequestStage::Response,
            }],
        );
        let mut network_request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let preflight = owner
            .prepare_navigation_request(
                &Url::parse("https://nav.example/doc").unwrap(),
                Some("https://referrer.example/"),
                false,
                &test_browser_identity("Moli/Test-UA"),
                &mut network_request_id_allocator,
            )
            .expect("active preflight should prepare");

        assert_eq!(preflight.frame_id, "FRAME-0");
        assert_eq!(
            preflight.document_fetch_request_stage,
            Some(FetchRequestStage::Response)
        );
        assert_eq!(
            preflight.document_request_id.as_deref(),
            Some("LID-0000000001")
        );
        assert_eq!(
            preflight.fetch_navigation_request_id.as_deref(),
            Some("INT-1")
        );
        assert!(
            preflight
                .request_headers
                .contains(&("User-Agent".to_owned(), "Moli/Test-UA".to_owned()))
        );
        assert!(!active.has_captured_response_body_for_test("REQ-old"));
    }

    #[test]
    fn target_session_owner_mut_observes_active_data_url_navigation_with_network_listener() {
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "FRAME-0");
        active
            .active_page_target_mut()
            .runtime_slot
            .enable_primary_network_events();
        let mut owner = TargetSessionOwnerMut {
            browser_context: &mut active,
            target_id: "FRAME-0".to_owned(),
            command_session_id: None,
            session_key: DevToolsSessionKey::Primary,
        };
        let mut network_request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let preflight = owner
            .prepare_navigation_request(
                &Url::parse("data:image/png;base64,AP9h").unwrap(),
                None,
                true,
                &test_browser_identity("Moli/Test-UA"),
                &mut network_request_id_allocator,
            )
            .expect("active data URL preflight should prepare");

        assert_eq!(preflight.frame_id, "FRAME-0");
        assert_eq!(preflight.document_fetch_request_stage, None);
        assert_eq!(
            preflight.document_request_id.as_deref(),
            Some("LID-0000000001")
        );
        assert_eq!(preflight.fetch_navigation_request_id, None);
        assert!(!preflight.document_auth_required);
        assert!(
            preflight
                .document_auth_required_blocked_intercepts
                .is_empty()
        );
    }

    #[test]
    fn target_session_owner_mut_prepares_background_navigation_request_preflight() {
        let mut background = BrowserContext::new_with_page_for_test("BID-background", "TID-active");
        background
            .active_page_target_mut()
            .set_base_locale_override(Some("zh-CN".to_owned()));
        background
            .active_page_target_mut()
            .network_policy
            .set_browser_identity_override(test_browser_identity("Active-Only-UA"));
        background.replace_default_browser_identity_override_for_test(test_browser_identity(
            "Browser-Context-Default-UA",
        ));
        background.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        ));
        {
            let state = background
                .background_target_mut("TID-background")
                .expect("background target must exist");
            state.runtime_slot.enable_primary_network_events();
            state
                .network_policy
                .push_extra_header(("X-Owner".to_owned(), "background".to_owned()));
            state.fetch_owner.configure(
                Some("SID-background".to_owned()),
                false,
                vec![FetchInterceptionPattern {
                    url_pattern: "*".to_owned(),
                    resource_type_filter: None,
                    request_stage: FetchRequestStage::Response,
                }],
            );
        }
        background
            .background_target_mut("TID-background")
            .expect("background target should exist")
            .runtime_slot
            .record_captured_response_body(
                "REQ-old".to_owned(),
                "old body".to_owned(),
                vec![Some("SID-background".to_owned())],
            );
        assert!(
            background
                .background_target("TID-background")
                .expect("background target should exist")
                .runtime_slot()
                .has_captured_response_body("REQ-old")
        );

        let mut owner = TargetSessionOwnerMut {
            browser_context: &mut background,
            target_id: "TID-background".to_owned(),
            command_session_id: Some("SID-background".to_owned()),
            session_key: DevToolsSessionKey::Primary,
        };
        let mut network_request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let preflight = owner
            .prepare_navigation_request(
                &Url::parse("https://nav.example/doc").unwrap(),
                Some("https://referrer.example/"),
                false,
                &test_browser_identity("Moli/Test-UA"),
                &mut network_request_id_allocator,
            )
            .expect("background preflight should prepare");

        assert_eq!(preflight.frame_id, "TID-background");
        assert_eq!(preflight.session_id.as_deref(), Some("SID-background"));
        assert_eq!(
            preflight.document_fetch_request_stage,
            Some(FetchRequestStage::Response)
        );
        assert_eq!(
            preflight.document_request_id.as_deref(),
            Some("LID-0000000001")
        );
        assert_eq!(
            preflight.fetch_navigation_request_id.as_deref(),
            Some("INT-1")
        );
        assert!(
            !background
                .background_target("TID-background")
                .expect("background target should exist")
                .runtime_slot()
                .has_captured_response_body("REQ-old")
        );
        assert!(
            preflight
                .request_headers
                .contains(&("X-Owner".to_owned(), "background".to_owned()))
        );
        assert!(
            preflight
                .request_headers
                .iter()
                .all(|(name, _)| !name.eq_ignore_ascii_case("accept-language"))
        );
        assert!(preflight.request_headers.contains(&(
            "User-Agent".to_owned(),
            "Browser-Context-Default-UA".to_owned()
        )));
        assert!(
            !preflight
                .request_headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("user-agent")
                    && value == "Active-Only-UA")
        );
        assert!(
            preflight
                .request_headers
                .contains(&("Referer".to_owned(), "https://referrer.example/".to_owned()))
        );
    }

    #[test]
    fn target_session_owner_mut_observes_background_data_url_navigation_with_network_listener() {
        let mut background = BrowserContext::new("BID-background".to_owned());
        background.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        ));
        background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .runtime_slot
            .enable_primary_network_events();

        let mut owner = TargetSessionOwnerMut {
            browser_context: &mut background,
            target_id: "TID-background".to_owned(),
            command_session_id: None,
            session_key: DevToolsSessionKey::Primary,
        };
        let mut network_request_id_allocator = ConnectionNetworkRequestIdAllocator::default();
        let preflight = owner
            .prepare_navigation_request(
                &Url::parse("data:image/png;base64,AP9h").unwrap(),
                None,
                true,
                &test_browser_identity("Moli/Test-UA"),
                &mut network_request_id_allocator,
            )
            .expect("background data URL preflight should prepare");

        assert_eq!(preflight.frame_id, "TID-background");
        assert_eq!(preflight.session_id.as_deref(), Some("SID-background"));
        assert_eq!(preflight.document_fetch_request_stage, None);
        assert_eq!(
            preflight.document_request_id.as_deref(),
            Some("LID-0000000001")
        );
        assert_eq!(preflight.fetch_navigation_request_id, None);
        assert!(!preflight.document_auth_required);
        assert!(
            preflight
                .document_auth_required_blocked_intercepts
                .is_empty()
        );
    }

    #[test]
    fn target_session_owner_ref_snapshots_background_navigation_load_inputs() {
        let mut background = BrowserContext::new_with_page_for_test("BID-background", "TID-active");
        background
            .active_page_target_mut()
            .set_base_locale_override(Some("zh-CN".to_owned()));
        background.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.example/start".to_owned(),
        ));
        {
            let state = background
                .background_target_mut("TID-background")
                .expect("background target must exist");
            state.set_base_locale_override(Some("fr-FR".to_owned()));
            state.set_base_timezone_override(Some("Europe/Paris".to_owned()));
            state.http_proxy_override = Some("http://proxy.example:8080".to_owned());
            state.http_no_proxy_override = Some("localhost,127.0.0.1".to_owned());
            state.tls_verify_host_override = Some(false);
            state.effective_emulation_state.script_execution_disabled = true;
            state
                .network_policy
                .set_user_agent_override("OwnerUA/1.0".to_owned());
            state.network_policy.set_network_offline(true);
            let network = &mut state.devtools_sessions.primary_mut().network_session_state;
            network.network_enabled = true;
            network.blocked_url_patterns = vec!["*.blocked.test".to_owned()];
            state
                .network_policy
                .push_extra_header(("X-Owner".to_owned(), "background".to_owned()));
            state.effective_emulation_state.emulated_media.media = Some("print".to_owned());
            state.fetch_owner.configure(
                Some("SID-background".to_owned()),
                false,
                vec![FetchInterceptionPattern {
                    url_pattern: "*".to_owned(),
                    resource_type_filter: None,
                    request_stage: FetchRequestStage::Request,
                }],
            );
        }
        background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .owner_state
            .document_start_scripts
            .push((
                "1".to_owned(),
                DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: "globalThis.fromBackgroundPreload = true;".to_owned(),
                    world_name: Some("utility".to_owned()),
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ));
        background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .upsert_runtime_binding_definition(
                "fromBackgroundBinding".to_owned(),
                Some("utility".to_owned()),
            );

        let owner = TargetSessionOwnerRef {
            browser_context: &background,
            target_id: "TID-background".to_owned(),
            session_key: DevToolsSessionKey::Primary,
        };
        let inputs = owner.navigation_load_inputs();

        assert_eq!(inputs.browser_context_id.as_deref(), Some("BID-background"));
        assert!(
            inputs
                .renderer_runtime
                .runtime()
                .shares_state_with(&background.renderer_runtime()),
            "background navigation must reuse the browser-context renderer runtime"
        );
        assert_eq!(
            inputs.navigation_initiator_url.as_ref().map(Url::as_str),
            Some("https://background.example/start")
        );
        assert!(
            inputs
                .document_start_scripts
                .iter()
                .any(|script| script.source == "globalThis.fromBackgroundPreload = true;")
        );
        assert_eq!(
            inputs.runtime_bindings,
            vec![RuntimeBindingDefinition {
                devtools_session: Some(moli_page_types::DevToolsSessionKey::Primary),
                name: "fromBackgroundBinding".to_owned(),
                execution_context_name: Some("utility".to_owned()),
            }]
        );
        assert!(
            inputs
                .extra_http_headers
                .contains(&("X-Owner".to_owned(), "background".to_owned()))
        );
        assert!(
            inputs
                .extra_http_headers
                .iter()
                .all(|(name, _)| !name.eq_ignore_ascii_case("accept-language"))
        );
        assert_eq!(inputs.locale_override.as_deref(), Some("fr-FR"));
        assert_eq!(inputs.timezone_override.as_deref(), Some("Europe/Paris"));
        assert_eq!(
            inputs
                .browser_identity_override
                .as_ref()
                .map(|identity| identity.user_agent()),
            Some("OwnerUA/1.0")
        );
        assert_eq!(
            inputs.http_proxy_override.as_deref(),
            Some("http://proxy.example:8080")
        );
        assert_eq!(
            inputs.http_no_proxy_override.as_deref(),
            Some("localhost,127.0.0.1")
        );
        assert_eq!(inputs.tls_verify_host_override, Some(false));
        assert!(inputs.script_execution_disabled);
        assert_eq!(inputs.emulated_media.media.as_deref(), Some("print"));
        assert!(inputs.network_offline);
        assert_eq!(inputs.blocked_url_patterns, vec!["*.blocked.test"]);
        assert_eq!(inputs.fetch_subresource_interception, (true, None));
    }

    #[test]
    fn target_session_owner_mut_prepares_background_navigation_commit_state() {
        let mut background = BrowserContext::new("BID-background".to_owned());
        background.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        ));
        {
            let state = background
                .background_target_mut("TID-background")
                .expect("background target must exist");
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events = true;
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled = true;
            state.fetch_owner.configure(
                Some("SID-background".to_owned()),
                false,
                vec![FetchInterceptionPattern {
                    url_pattern: "*".to_owned(),
                    resource_type_filter: None,
                    request_stage: FetchRequestStage::Request,
                }],
            );
        }

        let commit_state = {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            owner
                .prepare_loaded_navigation_commit()
                .expect("background navigation commit state should prepare")
        };

        assert_eq!(commit_state.browser_context_id, "BID-background");
        assert!(commit_state.runtime_frontend_enabled);
        assert_eq!(
            background
                .background_target("TID-background")
                .expect("background target")
                .target_url(),
            "about:blank",
            "preparing commit state should not mutate target identity"
        );
        let navigation_url = Url::parse("https://nav.example/path").unwrap();
        background
            .background_target_mut("TID-background")
            .expect("background target")
            .set_target_secure_context_type("InsecureScheme".to_owned());
        let main_document_commit = RendererMainDocumentCommit {
            frame_id: "TID-background".to_owned(),
            loader_id: "LOADER-nav".to_owned(),
            url: navigation_url.to_string(),
            unreachable_url: None,
            security_origin: "https://nav.example".to_owned(),
            secure_context_type: "Secure".to_owned(),
            timestamp: 0.0,
        };
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            owner
                .commit_loaded_navigation_target_identity(&main_document_commit, &navigation_url)
                .expect("background navigation identity should commit")
        };
        assert_eq!(
            background
                .background_target("TID-background")
                .expect("background target")
                .target_url(),
            "https://nav.example/path"
        );
        assert_eq!(
            background
                .background_target("TID-background")
                .expect("background target")
                .target_identity()
                .secure_context_type(),
            "Secure"
        );
        assert_eq!(commit_state.fetch_subresource_config, (true, None));
    }

    #[test]
    fn target_session_owner_mut_clears_background_navigation_history_update() {
        let mut background = BrowserContext::new("BID-background".to_owned());
        background.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        ));
        {
            let owner_state = &mut background
                .background_target_mut("TID-background")
                .expect("background target must exist")
                .owner_state;
            owner_state.record_loaded_page_navigation_history((
                "https://old.example/".to_owned(),
                "old".to_owned(),
            ));
            owner_state.mark_next_navigation_history_replace_current();
        }
        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            owner
                .clear_pending_navigation_history_update()
                .expect("background history update should clear");
        }
        background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .owner_state
            .record_loaded_page_navigation_history((
                "https://new.example/".to_owned(),
                "new".to_owned(),
            ));
        let (_, entries) = background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .owner_state
            .navigation_history_snapshot(None);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.url.as_str())
                .collect::<Vec<_>>(),
            vec!["https://old.example/", "https://new.example/"]
        );
    }

    #[tokio::test]
    async fn target_session_owner_mut_commits_loaded_page_to_background_owner() {
        let mut ctx = TestContext::new();
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>background commit</title>")
            .await
            .expect("page should load");
        let page_url = page.final_url().clone();
        let mut background = BrowserContext::new("BID-background".to_owned());
        background.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        ));
        let initial_attachment_id = background
            .background_target("TID-background")
            .expect("background target")
            .page_attachment_id();

        {
            let mut owner = TargetSessionOwnerMut {
                browser_context: &mut background,
                target_id: "TID-background".to_owned(),
                command_session_id: None,
                session_key: DevToolsSessionKey::Primary,
            };
            owner
                .commit_loaded_navigation_page_async(
                    page,
                    LoadedNavigationRendererAttachmentCommit::Prepare(None),
                    &page_url,
                )
                .await
                .expect("background page owner should exist")
                .expect("background page Inspector binding should activate");
        }

        let target = background
            .background_target("TID-background")
            .expect("background target");
        assert!(target.has_loaded_page());
        assert!(
            target.page_attachment_id().is_some()
                && target.page_attachment_id() != initial_attachment_id
        );
        let (_, entries) = background
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .owner_state
            .navigation_history_snapshot(None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "background commit");
    }

    #[test]
    fn target_page_residence_identity_rejects_context_target_and_attachment_collisions() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-page-residence".to_owned());
        browser_context.set_active_target_id("TID-page-residence");
        browser_context.attach_active_session("SID-page-residence");
        conn.install_browser_context_fixture_for_test(browser_context);
        conn.runtime_session_owner_slot_mut(Some("SID-page-residence"))
            .expect("active target runtime slot")
            .set_page_attachment_id_for_test(41);

        let current = conn
            .target_page_residence_identity_for_session(Some("SID-page-residence"))
            .expect("active target should expose its Page residence identity");
        assert!(conn.target_page_residence_identity_is_current(&current));

        for stale in [
            TargetPageResidenceIdentity::new(
                "BID-other".to_owned(),
                Some("TID-page-residence".to_owned()),
                current.page_attachment_id(),
            ),
            TargetPageResidenceIdentity::new(
                "BID-page-residence".to_owned(),
                Some("TID-other".to_owned()),
                current.page_attachment_id(),
            ),
            TargetPageResidenceIdentity::new(
                "BID-page-residence".to_owned(),
                Some("TID-page-residence".to_owned()),
                crate::conn::TargetPageAttachmentId::allocate(),
            ),
        ] {
            assert!(
                !conn.target_page_residence_identity_is_current(&stale),
                "every Page residence identity component must participate in authorization"
            );
        }
    }

    #[test]
    fn target_without_page_has_only_a_pending_page_residence() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-pending-residence".to_owned());
        browser_context.set_active_target_id("TID-pending-residence");
        browser_context.attach_active_session("SID-pending-residence".to_owned());
        conn.install_browser_context_fixture_for_test(browser_context);

        assert_eq!(
            conn.target_page_residence_identity_for_session(Some("SID-pending-residence")),
            None,
            "an empty target slot must not manufacture a current Page identity"
        );

        conn.runtime_session_owner_slot_mut(Some("SID-pending-residence"))
            .expect("active target runtime slot")
            .start_document_navigation(
                "TID-pending-residence".to_owned(),
                "LOADER-pending-residence".to_owned(),
            );

        assert_eq!(
            conn.target_page_residence_identity_for_session(Some("SID-pending-residence")),
            None,
            "a reservation must not masquerade as the current Page"
        );
        assert!(
            conn.pending_target_page_residence_identity_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-pending-residence"),
            )
            .is_some(),
            "the future Page attachment should remain explicitly addressable"
        );
    }

    #[test]
    fn implicit_active_route_freezes_the_concrete_target_in_page_residence() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-implicit-page-residence".to_owned());
        browser_context.set_active_target_id("TID-original");
        conn.install_browser_context_fixture_for_test(browser_context);
        conn.runtime_session_owner_slot_mut(None)
            .expect("implicit active runtime slot")
            .set_page_attachment_id_for_test(1);

        let original = conn
            .target_page_residence_identity_for_session(None)
            .expect("implicit active route should expose its Page residence");
        assert_eq!(original.target_id(), Some("TID-original"));

        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_active_target_id("TID-replacement");
        assert!(
            conn.target_page_residence_identity_is_current(&original),
            "changing visibility must not retire the original stable Page residence"
        );
        assert_ne!(
            conn.target_page_residence_identity_for_session(None),
            Some(original),
            "a fresh implicit lookup must follow the new active target without mutating the old identity"
        );
    }

    #[test]
    fn connection_target_owner_reference_reads_and_mutates_active_and_background_state() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        ));
        browser_context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        browser_context
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .inspector_enabled = true;
        conn.install_browser_context_fixture_for_test(browser_context);

        conn.with_target_owner_state_for_session_mut(Some("SID-active"), |owner_state| {
            owner_state.target_crash_state.mark_crashed();
        })
        .expect("active target owner state should be mutable");
        conn.with_target_devtools_session_state_for_session_mut(Some("SID-background"), |state| {
            state.register_runtime_remote_object_ids(["background-object".to_owned()]);
        })
        .expect("background DevTools session state should be mutable");

        let active_runtime_state = conn
            .target_runtime_session_state_for_session(Some("SID-active"))
            .expect("active runtime state should be readable");
        assert!(active_runtime_state.runtime_frontend_enabled);
        let background_runtime_state = conn
            .target_runtime_session_state_for_session(Some("SID-background"))
            .expect("background runtime state should be readable");
        assert!(background_runtime_state.inspector_enabled);
        assert!(
            conn.target_owner_state_for_session(Some("SID-active"))
                .expect("active owner state should be readable")
                .target_crash_state
                .is_crashed()
        );
        assert!(
            conn.target_devtools_session_state_for_session(Some("SID-background"))
                .expect("background DevTools session state should be readable")
                .has_runtime_remote_object_id("background-object")
        );
        assert!(
            conn.with_target_owner_state_for_session_mut(Some("SID-missing"), |_| ())
                .is_none()
        );
    }

    #[test]
    fn subscribed_page_event_sessions_require_page_domain_enable_per_session() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-page-events".to_owned());
        browser_context.set_active_target_id("TID-page-events".to_owned());
        browser_context.attach_active_session("SID-primary".to_owned());
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-page-events",
                "SID-page-enabled".to_owned(),
            )
        );
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-page-events",
                "SID-lifecycle-only".to_owned(),
            )
        );
        conn.install_browser_context_fixture_for_test(browser_context);

        for session_id in ["SID-primary", "SID-lifecycle-only"] {
            conn.with_target_devtools_session_state_for_session_mut(Some(session_id), |state| {
                state.page_session_state.page_lifecycle_events = true
            })
            .expect("target session should be mutable");
        }
        assert!(
            conn.subscribed_page_event_session_ids_for_session_owner(Some("SID-primary"))
                .is_empty(),
            "Page.setLifecycleEventsEnabled must not subscribe a session to Page events"
        );

        conn.with_target_devtools_session_state_for_session_mut(
            Some("SID-page-enabled"),
            |state| state.page_session_state.page_domain_enabled = true,
        )
        .expect("Page-enabled attached session should be mutable");
        assert_eq!(
            conn.subscribed_page_event_session_ids_for_session_owner(Some("SID-primary")),
            vec![Some("SID-page-enabled".to_owned())]
        );

        conn.with_target_devtools_session_state_for_session_mut(Some("SID-primary"), |state| {
            state.page_session_state.page_domain_enabled = true
        })
        .expect("primary session should be mutable");
        assert_eq!(
            conn.subscribed_page_event_session_ids_for_session_owner(Some("SID-page-enabled")),
            vec![
                Some("SID-primary".to_owned()),
                Some("SID-page-enabled".to_owned()),
            ],
            "Page events should fan out only to Page-enabled sessions on the same target"
        );
    }

    #[test]
    fn runtime_event_attachments_include_every_enabled_session_on_the_exact_page() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-runtime-events".to_owned());
        browser_context.set_active_target_id("TID-runtime-events".to_owned());
        browser_context.attach_active_session("SID-runtime-a".to_owned());
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-runtime-events",
                "SID-runtime-b".to_owned(),
            )
        );
        assert!(browser_context.assign_attached_session_to_target(
            "TID-runtime-events",
            "SID-runtime-disabled".to_owned(),
        ));
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(41);
        conn.install_browser_context_fixture_for_test(browser_context);

        conn.with_target_devtools_session_state_for_session_mut(Some("SID-runtime-b"), |state| {
            state.runtime_session_state.runtime_frontend_enabled = true
        })
        .expect("Runtime-enabled attached session should be mutable");
        assert_eq!(
            conn.runtime_event_protocol_attachments_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-runtime-a")
            )
            .expect("the current Page should expose its Runtime audience")
            .into_iter()
            .map(|attachment| attachment.session_id().map(str::to_owned))
            .collect::<Vec<_>>(),
            vec![Some("SID-runtime-b".to_owned())],
            "a disabled primary must not hide the enabled peer attachment"
        );

        conn.with_target_devtools_session_state_for_session_mut(Some("SID-runtime-a"), |state| {
            state.runtime_session_state.runtime_frontend_enabled = true
        })
        .expect("Runtime-enabled primary session should be mutable");
        assert_eq!(
            conn.runtime_event_protocol_attachments_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-runtime-b")
            )
            .expect("the current Page should expose its Runtime audience")
            .into_iter()
            .map(|attachment| attachment.session_id().map(str::to_owned))
            .collect::<Vec<_>>(),
            vec![
                Some("SID-runtime-a".to_owned()),
                Some("SID-runtime-b".to_owned()),
            ],
            "one target-owned Runtime fact must freeze every enabled attachment"
        );
    }

    #[test]
    fn runtime_context_identity_includes_service_worker_without_page_owner_identity() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-worker-context".to_owned());
        browser_context.insert_service_worker_target(ServiceWorkerTargetState::new(
            41,
            29,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        ));
        assert!(browser_context.assign_session_to_service_worker_target(
            "TID-service-worker",
            "SID-service-worker".to_owned(),
        ));
        conn.install_browser_context_fixture_for_test(browser_context);

        assert_eq!(
            conn.session_route(Some("SID-service-worker")),
            Some(CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id: "BID-worker-context".to_owned(),
                target_id: "TID-service-worker".to_owned(),
            })
        );
        assert_eq!(
            conn.target_owner_identity_for_session(Some("SID-service-worker")),
            None,
            "Service Worker target sessions must not satisfy page/background target owner checks"
        );
        assert_eq!(
            conn.runtime_context_owner_identity_for_session(Some("SID-service-worker")),
            Some((
                "BID-worker-context".to_owned(),
                Some("TID-service-worker".to_owned())
            )),
            "Runtime context events still need the worker target id for realm qualification"
        );
        assert_eq!(
            conn.worker_target_id_for_session(Some("SID-service-worker")),
            Some("TID-service-worker".to_owned())
        );
    }

    #[test]
    fn connection_runtime_slot_reference_reads_and_mutates_active_background_and_attached_slots() {
        let mut conn = CdpConnection::default();
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        active.set_active_target_id("TID-active".to_owned());
        active.set_target_url("https://active.example/".to_owned());
        active.attach_active_session("SID-active".to_owned());
        active.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.example/".to_owned(),
        ));
        active
            .background_target_mut("TID-background")
            .expect("background target must exist")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        assert!(active.assign_attached_session_to_target(
            "TID-background",
            "SID-attached-background".to_owned()
        ));

        let mut inactive = BrowserContext::new("BID-inactive".to_owned());
        inactive.set_active_target_id("TID-inactive".to_owned());
        inactive.set_target_url("https://inactive.example/".to_owned());
        inactive
            .active_page_target_mut()
            .devtools_sessions
            .ensure_attached("SID-attached-inactive")
            .console_output_session_state
            .console_enabled = true;
        assert!(
            inactive.assign_attached_session_to_target(
                "TID-inactive",
                "SID-attached-inactive".to_owned()
            )
        );

        conn.install_browser_context_fixture_for_test(active);
        conn.push_inactive_browser_context_fixture_for_test(inactive);

        conn.runtime_session_owner_slot_mut(Some("SID-active"))
            .expect("active runtime slot should be mutable")
            .set_page_attachment_id_for_test(11);
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot should be mutable")
            .set_page_attachment_id_for_test(22);
        conn.runtime_session_owner_slot_mut(Some("SID-attached-inactive"))
            .expect("inactive attached runtime slot should be mutable")
            .set_page_attachment_id_for_test(33);

        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-active"))
                .expect("active runtime slot should be readable")
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(11)
        );
        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-background"))
                .expect("background runtime slot should be readable")
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(22)
        );
        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-attached-background"))
                .expect("background attached runtime slot should be readable")
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(22)
        );
        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-attached-inactive"))
                .expect("inactive attached runtime slot should be readable")
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(33)
        );
        assert_eq!(
            conn.runtime_session_owner_primary_session_id(Some("SID-active"))
                .as_deref(),
            Some("SID-active")
        );
        assert_eq!(
            conn.runtime_session_owner_primary_session_id(Some("SID-background"))
                .as_deref(),
            Some("SID-background")
        );
        assert_eq!(
            conn.runtime_session_owner_primary_session_id(Some("SID-attached-background"))
                .as_deref(),
            Some("SID-background")
        );
        assert_eq!(
            conn.runtime_session_owner_target_url(Some("SID-background"))
                .as_deref(),
            Some("https://background.example/")
        );
        assert_eq!(
            conn.runtime_session_owner_target_url(Some("SID-attached-inactive"))
                .as_deref(),
            Some("https://inactive.example/")
        );
        assert_eq!(
            conn.target_owner_identity_for_session(Some("SID-attached-background")),
            Some(("BID-active".to_owned(), Some("TID-background".to_owned())))
        );
        assert_eq!(
            conn.target_owner_identity_for_session(Some("SID-attached-inactive")),
            Some(("BID-inactive".to_owned(), Some("TID-inactive".to_owned())))
        );
        assert!(
            conn.target_page_session_state_for_session(Some("SID-background"))
                .expect("background page session state should be readable")
                .log_enabled
        );
        assert!(
            conn.target_devtools_session_state_for_session(Some("SID-attached-inactive"))
                .expect("inactive attached DevTools session state should be readable")
                .console_output_session_state
                .console_enabled
        );
        assert!(
            conn.runtime_session_owner_slot(Some("SID-missing"))
                .is_err()
        );
    }

    #[test]
    fn background_dialog_scope_survives_default_session_state_folding() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-dialog-scope".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.insert_page_target_host(crate::conn::PageTargetHost::with_url(
            "TID-background-dialog".to_owned(),
            Some("SID-background-dialog".to_owned()),
            "https://background.example/dialog".to_owned(),
        ));
        assert!(
            browser_context
                .background_target("TID-background-dialog")
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "a background target with default protocol settings should not allocate background overrides"
        );
        conn.install_browser_context_fixture_for_test(browser_context);

        let observer = conn
            .runtime_session_owner_slot(Some("SID-background-dialog"))
            .expect("background target should own a stable runtime slot")
            .javascript_dialog_scope_observer();
        conn.with_target_devtools_session_state_for_session_mut(
            Some("SID-background-dialog"),
            |state| state.page_session_state.javascript_dialog_state.clear(),
        )
        .expect("background session state mutation should be available");

        let browser_context = conn.browser_context.as_ref().expect("browser context");
        assert!(
            browser_context
                .background_target("TID-background-dialog")
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "clearing an empty dialog list should fold the temporary session state"
        );
        assert!(
            conn.runtime_session_owner_slot(Some("SID-background-dialog"))
                .expect("background runtime slot")
                .observes_javascript_dialog_scope(&observer),
            "folding protocol settings must not retire Page-owned prepared output"
        );

        conn.runtime_session_owner_slot_mut(Some("SID-background-dialog"))
            .expect("background runtime slot")
            .retire_javascript_dialog_scope();
        assert!(
            !conn
                .runtime_session_owner_slot(Some("SID-background-dialog"))
                .expect("background runtime slot")
                .observes_javascript_dialog_scope(&observer)
        );
    }
}
