use super::session_owner::TargetSessionOwner;
use super::*;
use crate::conn::state::{
    BrowserContextPageStorageHandles, BrowserContextResourceStorageHandles, DevToolsSessionState,
    PageNavigationHistoryEntry, RendererMainDocumentCommitSeed, TargetFetchConfig,
    TargetNetworkPolicyState, TargetOwnerState, TargetPageAbsenceReason,
    TargetPageResidenceIdentity, TargetRuntimeSessionState, TargetRuntimeSlot,
    page_bypass_csp_enabled_for_devtools_sessions,
    prepare_renderer_call_replacements_for_devtools_sessions, runtime_bindings_for_renderer,
};
use crate::conn::{
    BackgroundProtocolEvent, ConnectionNetworkRequestIdAllocator, DocumentStartScript,
    EmulatedDeviceMetrics, FetchInterceptionPattern, FetchRequestStage,
    InitialDocumentPageInstallResult, InitialDocumentPageOwner, IsolatedWorldDefinition,
    LoadedNavigationPageCommit, LoadedNavigationRendererAttachmentCommit, NETWORK_ERROR_PAGE_URL,
    NetworkErrorPageNavigation, PausedDocumentTransfer, PendingFetchAuthNavigation,
    PendingFetchNavigation, PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, RuntimeBindingDefinition,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
#[cfg(test)]
use moli_core::page::RendererServiceWorkerVersionStatus;
use moli_core::page::{
    BidiPreloadChannelHandoff, Page, RendererInspectorProtocolConfiguration,
    RendererInspectorSessionRestoreSnapshot, RendererMainDocumentCommit,
    RendererPageCreationArtifacts, SubresourceResourceType, V8InspectorSessionAttach,
};
use moli_core::runtime::RendererBrowserContextRuntimeOwnerAccess;
use moli_fetch::BrowserNavigationRequestKind;
use url::Url;

pub(super) enum TargetSessionOwnerMut<'a> {
    ActiveTarget {
        browser_context: &'a mut BrowserContext,
        session_id: Option<String>,
        is_auxiliary_target_session: bool,
        is_current_active_browser_context: bool,
    },
    BackgroundTarget {
        browser_context: &'a mut BrowserContext,
        target_id: String,
        session_id: Option<String>,
        is_auxiliary_target_session: bool,
    },
    NoLoadedBrowserContext,
}

pub(super) enum TargetSessionOwnerRef<'a> {
    ActiveTarget {
        browser_context: &'a BrowserContext,
        session_id: Option<String>,
        is_auxiliary_target_session: bool,
    },
    BackgroundTarget {
        browser_context: &'a BrowserContext,
        target_id: String,
        session_id: Option<String>,
        is_auxiliary_target_session: bool,
    },
    NoLoadedBrowserContext,
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
    pub(crate) auxiliary_session_ids: Vec<String>,
}

impl ClosedPageTarget {
    pub(crate) fn inspector_detached_session_ids(&self) -> impl Iterator<Item = &str> {
        self.primary_session_id
            .as_deref()
            .into_iter()
            .chain(self.auxiliary_session_ids.iter().map(String::as_str))
    }

    pub(crate) fn into_detach_cleanup_plan(
        self,
        reason: Option<&str>,
    ) -> crate::conn::TargetClosureCleanupPlan {
        crate::conn::TargetClosureCleanupPlan::from_primary_and_auxiliary_sessions(
            self.target_id,
            reason,
            self.primary_session_id,
            self.auxiliary_session_ids,
        )
    }
}

pub(super) enum TargetSessionStateMut<'a> {
    Active {
        devtools_session_state: &'a mut DevToolsSessionState,
        network_policy: &'a mut TargetNetworkPolicyState,
        tls_verify_host_override: &'a mut Option<bool>,
    },
    Parked {
        devtools_session_state: &'a mut DevToolsSessionState,
        network_policy: &'a mut TargetNetworkPolicyState,
        tls_verify_host_override: &'a mut Option<bool>,
    },
    NoLoaded,
}

pub(crate) struct TargetLoadedNavigationCommitState {
    pub(crate) browser_context_id: String,
    pub(crate) runtime_frontend_enabled: bool,
    pub(crate) renderer_runtime_inspector_session_id: Option<String>,
    pub(crate) runtime_inspector_session_restore_snapshots:
        Vec<RendererInspectorSessionRestoreSnapshot>,
    pub(crate) stored_runtime_bindings: Vec<RuntimeBindingDefinition>,
    pub(crate) session_runtime_bindings: Vec<RuntimeBindingDefinition>,
    pub(crate) isolated_worlds: Vec<IsolatedWorldDefinition>,
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
    pub(crate) browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
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

    fn from_browser_context_active_target(browser_context: &BrowserContext) -> Self {
        Self {
            browser_context_id: Some(browser_context.id.clone()),
            storage_handles: TargetNavigationStorageHandles::from_page_handles(
                browser_context.page_storage_handles(),
            ),
            root_frame_id: browser_context.active_target_id_owned(),
            renderer_runtime: browser_context.renderer_runtime_owner_access(),
            browser_identity_override: browser_context
                .effective_active_browser_identity_override_owned(),
            http_proxy_override: browser_context.http_proxy_override.clone(),
            http_no_proxy_override: browser_context.http_no_proxy_override.clone(),
            tls_verify_host_override: browser_context.tls_verify_host_override,
            navigation_initiator_url: target_navigation_initiator_url(
                browser_context.target_url(),
                browser_context.loaded_page(),
            ),
            browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
            infer_navigation_referrer: true,
            document_start_scripts: browser_context.document_start_script_descriptors(),
            runtime_bindings: runtime_bindings_for_renderer(
                &browser_context.devtools_session_state,
                &browser_context.auxiliary_devtools_session_states,
            ),
            runtime_inspector_session_restore_snapshots:
                runtime_inspector_session_restore_snapshots_for_renderer(
                    &browser_context.devtools_session_state,
                    &browser_context.auxiliary_devtools_session_states,
                ),
            extra_http_headers: browser_context.effective_extra_headers(),
            locale_override: browser_context.effective_active_locale_override_owned(),
            timezone_override: browser_context.effective_active_timezone_override_owned(),
            script_execution_disabled: browser_context.script_execution_disabled,
            bypass_content_security_policy: page_bypass_csp_enabled_for_devtools_sessions(
                &browser_context.devtools_session_state,
                &browser_context.auxiliary_devtools_session_states,
            ),
            cpu_throttling_rate: browser_context.cpu_throttling_rate,
            emulated_media: (&browser_context.emulated_media).into(),
            viewport_surface: browser_context
                .effective_active_emulated_device_metrics()
                .as_ref()
                .map(|metrics| metrics.viewport_surface().to_page_viewport_surface()),
            network_offline: browser_context.network_policy.network_offline()
                || browser_context.effective_active_network_offline(),
            bypass_service_worker: browser_context.network_policy.bypass_service_worker(),
            blocked_url_patterns: browser_context
                .network_policy
                .blocked_url_patterns()
                .to_vec(),
            fetch_subresource_interception: browser_context
                .active_target
                .fetch_owner
                .subresource_interception_config(),
            permission_overrides: Vec::new(),
            main_document_commit_seed: None,
        }
    }

    fn from_browser_context_fallback(browser_context: &BrowserContext) -> Self {
        TargetNavigationLoadInputs::no_loaded_browser_context(
            browser_context.page_storage_handles(),
            browser_context.renderer_runtime_owner_access(),
        )
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

fn apply_locale_header(headers: &mut Vec<(String, String)>, locale: Option<&str>) {
    if let Some(locale) = locale
        && !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
    {
        headers.push(("Accept-Language".to_owned(), locale.to_owned()));
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

fn clear_parked_page_runtime_remote_object_tracking(
    state: &mut crate::conn::state::ParkedPageSessionState,
) {
    state
        .devtools_session_state
        .clear_runtime_remote_object_tracking();
    for state in state.auxiliary_devtools_session_states.values_mut() {
        state.clear_runtime_remote_object_tracking();
    }
}

fn clear_parked_page_loaded_document_session_state(
    state: &mut crate::conn::state::ParkedPageSessionState,
) {
    clear_parked_page_runtime_remote_object_tracking(state);
    state
        .devtools_session_state
        .page_session_state
        .clear_loaded_document_context_state();
    for state in state.auxiliary_devtools_session_states.values_mut() {
        state
            .page_session_state
            .clear_loaded_document_context_state();
    }
}

fn runtime_inspector_session_restore_snapshots_for_renderer(
    primary: &DevToolsSessionState,
    auxiliary: &std::collections::HashMap<String, DevToolsSessionState>,
) -> Vec<RendererInspectorSessionRestoreSnapshot> {
    let mut restores = Vec::new();
    if primary.runtime_session_state.runtime_frontend_enabled
        || primary.console_output_session_state.console_enabled
        || !primary.dom_debugger_event_listener_breakpoints.is_empty()
        || !primary.dom_debugger_xhr_breakpoints.is_empty()
        || primary.inspector_session_state.v8_state.is_some()
    {
        restores.push(RendererInspectorSessionRestoreSnapshot {
            inspector_session_id: None,
            v8_attach: V8InspectorSessionAttach::from_optional_state(
                primary.inspector_session_state.v8_state.clone(),
            ),
            protocol_configuration: RendererInspectorProtocolConfiguration {
                runtime_bindings: primary.runtime_bindings.clone(),
                runtime_frontend_enabled: primary.runtime_session_state.runtime_frontend_enabled,
                console_frontend_enabled: primary.console_output_session_state.console_enabled,
                dom_debugger_event_listener_breakpoints: primary
                    .dom_debugger_event_listener_breakpoints
                    .clone(),
                dom_debugger_xhr_breakpoints: primary.dom_debugger_xhr_breakpoints.clone(),
            },
        });
    }
    let mut auxiliary = auxiliary.iter().collect::<Vec<_>>();
    auxiliary.sort_by_key(|(session_id, _)| *session_id);
    for (session_id, state) in auxiliary {
        if !state.runtime_session_state.runtime_frontend_enabled
            && !state.console_output_session_state.console_enabled
            && state.dom_debugger_event_listener_breakpoints.is_empty()
            && state.dom_debugger_xhr_breakpoints.is_empty()
            && state.inspector_session_state.v8_state.is_none()
        {
            continue;
        }
        restores.push(RendererInspectorSessionRestoreSnapshot {
            inspector_session_id: Some(session_id.clone()),
            v8_attach: V8InspectorSessionAttach::from_optional_state(
                state.inspector_session_state.v8_state.clone(),
            ),
            protocol_configuration: RendererInspectorProtocolConfiguration {
                runtime_bindings: state.runtime_bindings.clone(),
                runtime_frontend_enabled: state.runtime_session_state.runtime_frontend_enabled,
                console_frontend_enabled: state.console_output_session_state.console_enabled,
                dom_debugger_event_listener_breakpoints: state
                    .dom_debugger_event_listener_breakpoints
                    .clone(),
                dom_debugger_xhr_breakpoints: state.dom_debugger_xhr_breakpoints.clone(),
            },
        });
    }
    restores
}

fn renderer_runtime_inspector_session_id(
    is_auxiliary_target_session: bool,
    session_id: Option<&str>,
) -> Option<String> {
    if is_auxiliary_target_session {
        session_id.map(str::to_owned)
    } else {
        None
    }
}

impl<'a> TargetSessionOwnerRef<'a> {
    pub(super) fn devtools_session_state(&self) -> Option<&'a DevToolsSessionState> {
        match self {
            Self::ActiveTarget {
                browser_context,
                session_id,
                is_auxiliary_target_session,
            } => target_devtools_session_state(
                &browser_context.devtools_session_state,
                &browser_context.auxiliary_devtools_session_states,
                *is_auxiliary_target_session,
                session_id.as_deref(),
            ),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                session_id,
                is_auxiliary_target_session,
            } => browser_context
                .parked_page_session_state(target_id)
                .and_then(|state| {
                    target_devtools_session_state(
                        &state.devtools_session_state,
                        &state.auxiliary_devtools_session_states,
                        *is_auxiliary_target_session,
                        session_id.as_deref(),
                    )
                }),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn page_session_state(&self) -> Option<&'a TargetPageSessionState> {
        self.devtools_session_state()
            .map(|state| &state.page_session_state)
    }

    pub(super) fn effective_page_bypass_csp_enabled(&self) -> Option<bool> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(page_bypass_csp_enabled_for_devtools_sessions(
                &browser_context.devtools_session_state,
                &browser_context.auxiliary_devtools_session_states,
            )),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .parked_page_session_state(target_id)
                .map(|state| {
                    page_bypass_csp_enabled_for_devtools_sessions(
                        &state.devtools_session_state,
                        &state.auxiliary_devtools_session_states,
                    )
                }),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn runtime_session_state(&self) -> Option<&'a TargetRuntimeSessionState> {
        self.devtools_session_state()
            .map(|state| &state.runtime_session_state)
    }

    pub(super) fn renderer_runtime_inspector_session_id(&self) -> Option<String> {
        match self {
            Self::ActiveTarget {
                session_id,
                is_auxiliary_target_session,
                ..
            }
            | Self::BackgroundTarget {
                session_id,
                is_auxiliary_target_session,
                ..
            } => renderer_runtime_inspector_session_id(
                *is_auxiliary_target_session,
                session_id.as_deref(),
            ),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn runtime_bindings_for_renderer(&self) -> Vec<RuntimeBindingDefinition> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => runtime_bindings_for_renderer(
                &browser_context.devtools_session_state,
                &browser_context.auxiliary_devtools_session_states,
            ),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .parked_page_session_state(target_id)
                .map(|state| {
                    runtime_bindings_for_renderer(
                        &state.devtools_session_state,
                        &state.auxiliary_devtools_session_states,
                    )
                })
                .unwrap_or_default(),
            Self::NoLoadedBrowserContext => Vec::new(),
        }
    }

    pub(super) fn target_owner_state(&self) -> Option<&'a TargetOwnerState> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(&browser_context.active_target.owner_state),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context.parked_target_owner_state(target_id),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn initial_empty_document_url_if_current(&self) -> Option<String> {
        self.target_owner_state()?
            .initial_empty_document_url_if_current()
            .map(str::to_owned)
    }

    pub(super) fn initial_empty_document_storage_key_if_current(
        &self,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.target_owner_state()?
            .initial_empty_document_storage_key_if_current()
            .cloned()
    }

    pub(super) fn is_on_initial_empty_document(&self) -> Option<bool> {
        self.target_owner_state()?.is_on_initial_empty_document()
    }

    pub(super) fn initial_empty_document_has_pending_cross_document_navigation(&self) -> bool {
        self.target_owner_state()
            .is_some_and(TargetOwnerState::initial_empty_document_pending_cross_document_navigation)
    }

    pub(super) fn aggregate_fetch_config(&self) -> Option<TargetFetchConfig> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(browser_context.active_target.fetch_owner.config_snapshot()),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .parked_page_session_state(target_id)
                .map(|state| state.fetch_config.clone()),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn runtime_slot(&self) -> Option<&'a TargetRuntimeSlot> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(&browser_context.active_target.runtime_slot),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target(target_id)
                .map(|target| target.runtime_slot()),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn owner_identity(&self) -> Option<(String, Option<String>)> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some((
                browser_context.id.clone(),
                browser_context.active_target_id_owned(),
            )),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => Some((browser_context.id.clone(), Some(target_id.clone()))),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn primary_session_id(&self) -> Option<String> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => browser_context.active_session_id_owned(),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target(target_id)
                .and_then(|target| target.session_id().map(str::to_owned)),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn target_url(&self) -> Option<String> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(browser_context.target_url().to_owned()),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target(target_id)
                .map(|target| target.target_url().to_owned()),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn frame_tree_identity(&self) -> Option<(String, String, String, String)> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let document_url = browser_context
                    .active_target
                    .runtime_slot
                    .loaded_page()
                    // Only a browser-owned network error Document diverges
                    // from the user-visible Target/history URL in this fix.
                    // Initial-empty popup identity remains a separate issue.
                    .filter(|page| page.final_url().as_str() == NETWORK_ERROR_PAGE_URL)
                    .map(|page| page.final_url().to_string())
                    .unwrap_or_else(|| browser_context.target_url().to_owned());
                Some((
                    browser_context.active_target_id_owned()?,
                    document_url,
                    browser_context.target_security_origin().to_owned(),
                    browser_context.target_secure_context_type().to_owned(),
                ))
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let target = browser_context.background_target(target_id)?;
                Some((
                    target.target_id().to_owned(),
                    target
                        .loaded_page()
                        .filter(|page| page.final_url().as_str() == NETWORK_ERROR_PAGE_URL)
                        .map(|page| page.final_url().to_string())
                        .unwrap_or_else(|| target.target_identity().url().to_owned()),
                    target.target_identity().security_origin().to_owned(),
                    target.target_identity().secure_context_type().to_owned(),
                ))
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn frame_tree_loader_id(&self) -> Option<String> {
        self.runtime_slot()
            .and_then(TargetRuntimeSlot::committed_document_loader_id)
            .map(str::to_owned)
            .or_else(|| {
                self.target_owner_state()?
                    .initial_empty_document_loader_id_if_current()
                    .map(str::to_owned)
            })
    }

    pub(super) fn emulated_device_metrics(&self) -> Option<EmulatedDeviceMetrics> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => browser_context.effective_active_emulated_device_metrics(),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context.effective_parked_emulated_device_metrics(target_id),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn navigation_load_inputs(&self) -> TargetNavigationLoadInputs {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => TargetNavigationLoadInputs::from_browser_context_active_target(browser_context),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let Some(target) = browser_context.background_target(target_id) else {
                    return TargetNavigationLoadInputs::from_browser_context_fallback(
                        browser_context,
                    );
                };
                let page_state = browser_context
                    .parked_page_session_state(target_id)
                    .cloned()
                    .unwrap_or_default();
                let mut document_start_scripts = Vec::new();
                if let Some(script) =
                    browser_context.generated_surface_override_script_for_parked_target(target_id)
                {
                    document_start_scripts.push(script);
                }
                document_start_scripts
                    .extend(browser_context.default_document_start_script_descriptors());
                if let Some(owner_state) = browser_context.parked_target_owner_state(target_id) {
                    document_start_scripts.extend(owner_state.document_start_scripts.iter().map(
                        |(identifier, script)| {
                            script.with_registry_key(
                                BrowserContext::target_document_start_script_registry_key(
                                    Some(target_id.as_str()),
                                    identifier,
                                ),
                            )
                        },
                    ));
                }
                let runtime_bindings = runtime_bindings_for_renderer(
                    &page_state.devtools_session_state,
                    &page_state.auxiliary_devtools_session_states,
                );
                let runtime_inspector_session_restore_snapshots =
                    runtime_inspector_session_restore_snapshots_for_renderer(
                        &page_state.devtools_session_state,
                        &page_state.auxiliary_devtools_session_states,
                    );
                let mut extra_http_headers = page_state.network_policy.extra_headers().to_vec();
                let locale_override = page_state
                    .locale_override
                    .clone()
                    .or_else(|| browser_context.default_locale_override.clone());
                let header_locale_override = browser_context
                    .locale_override
                    .as_deref()
                    .or(locale_override.as_deref());
                apply_locale_header(&mut extra_http_headers, header_locale_override);

                TargetNavigationLoadInputs {
                    browser_context_id: Some(browser_context.id.clone()),
                    storage_handles: TargetNavigationStorageHandles::from_page_handles(
                        browser_context
                            .page_storage_handles_for_target(target_id)
                            .expect("background target must retain target-owned storage"),
                    ),
                    root_frame_id: Some(target_id.clone()),
                    renderer_runtime: browser_context.renderer_runtime_owner_access(),
                    browser_identity_override: page_state
                        .network_policy
                        .browser_identity_override_owned()
                        .or_else(|| browser_context.default_browser_identity_override.clone()),
                    http_proxy_override: page_state.http_proxy_override,
                    http_no_proxy_override: page_state.http_no_proxy_override,
                    tls_verify_host_override: page_state.tls_verify_host_override,
                    navigation_initiator_url: target_navigation_initiator_url(
                        target.target_url(),
                        target.loaded_page(),
                    ),
                    browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
                    infer_navigation_referrer: true,
                    document_start_scripts,
                    runtime_bindings,
                    runtime_inspector_session_restore_snapshots,
                    extra_http_headers,
                    locale_override,
                    timezone_override: page_state
                        .timezone_override
                        .or_else(|| browser_context.default_timezone_override.clone()),
                    script_execution_disabled: page_state.script_execution_disabled,
                    bypass_content_security_policy: page_bypass_csp_enabled_for_devtools_sessions(
                        &page_state.devtools_session_state,
                        &page_state.auxiliary_devtools_session_states,
                    ),
                    cpu_throttling_rate: page_state.cpu_throttling_rate,
                    emulated_media: (&page_state.emulated_media).into(),
                    viewport_surface: browser_context
                        .effective_parked_emulated_device_metrics(target_id)
                        .as_ref()
                        .map(|metrics| metrics.viewport_surface().to_page_viewport_surface()),
                    network_offline: page_state.network_policy.network_offline()
                        || browser_context.effective_parked_network_offline(target_id),
                    bypass_service_worker: page_state.network_policy.bypass_service_worker(),
                    blocked_url_patterns: page_state.network_policy.blocked_url_patterns().to_vec(),
                    fetch_subresource_interception: page_state
                        .fetch_config
                        .subresource_interception_config(),
                    permission_overrides: Vec::new(),
                    main_document_commit_seed: None,
                }
            }
            Self::NoLoadedBrowserContext => unreachable!(
                "connection-owned no-context load inputs must be provided by CdpConnection"
            ),
        }
    }
}

fn target_devtools_session_state_mut<'a>(
    primary: &'a mut DevToolsSessionState,
    auxiliary: &'a mut std::collections::HashMap<String, DevToolsSessionState>,
    is_auxiliary_target_session: bool,
    session_id: Option<&str>,
) -> &'a mut DevToolsSessionState {
    if is_auxiliary_target_session && let Some(session_id) = session_id {
        return auxiliary.entry(session_id.to_owned()).or_default();
    }
    primary
}

fn target_devtools_session_state<'a>(
    primary: &'a DevToolsSessionState,
    auxiliary: &'a std::collections::HashMap<String, DevToolsSessionState>,
    is_auxiliary_target_session: bool,
    session_id: Option<&str>,
) -> Option<&'a DevToolsSessionState> {
    if is_auxiliary_target_session {
        return session_id.and_then(|session_id| auxiliary.get(session_id));
    }
    Some(primary)
}

impl TargetSessionStateMut<'_> {
    pub(super) fn devtools_session_state_mut(&mut self) -> Option<&mut DevToolsSessionState> {
        match self {
            Self::Active {
                devtools_session_state,
                ..
            }
            | Self::Parked {
                devtools_session_state,
                ..
            } => Some(&mut **devtools_session_state),
            Self::NoLoaded => None,
        }
    }

    pub(super) fn page_session_state_mut(&mut self) -> Option<&mut TargetPageSessionState> {
        match self {
            Self::Active {
                devtools_session_state,
                ..
            }
            | Self::Parked {
                devtools_session_state,
                ..
            } => Some(&mut devtools_session_state.page_session_state),
            Self::NoLoaded => None,
        }
    }

    pub(super) fn runtime_session_state_mut(&mut self) -> Option<&mut TargetRuntimeSessionState> {
        match self {
            Self::Active {
                devtools_session_state,
                ..
            }
            | Self::Parked {
                devtools_session_state,
                ..
            } => Some(&mut devtools_session_state.runtime_session_state),
            Self::NoLoaded => None,
        }
    }

    pub(super) fn network_policy_mut(&mut self) -> Option<&mut TargetNetworkPolicyState> {
        match self {
            Self::Active { network_policy, .. } | Self::Parked { network_policy, .. } => {
                Some(*network_policy)
            }
            Self::NoLoaded => None,
        }
    }

    pub(super) fn tls_verify_host_override_mut(&mut self) -> Option<&mut Option<bool>> {
        match self {
            Self::Active {
                tls_verify_host_override,
                ..
            }
            | Self::Parked {
                tls_verify_host_override,
                ..
            } => Some(*tls_verify_host_override),
            Self::NoLoaded => None,
        }
    }
}

impl<'a> TargetSessionOwnerMut<'a> {
    pub(super) fn target_url(&self) -> Option<String> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(browser_context.target_url().to_owned()),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target(target_id)
                .map(|target| target.target_url().to_owned()),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn runtime_slot_ref(&self) -> Option<&TargetRuntimeSlot> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(&browser_context.active_target.runtime_slot),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target(target_id)
                .map(|target| target.runtime_slot()),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn mutate_session_state_ref<T>(
        &mut self,
        f: impl FnOnce(TargetSessionStateMut<'_>) -> T,
    ) -> T {
        match self {
            Self::ActiveTarget {
                browser_context,
                session_id,
                is_auxiliary_target_session,
                ..
            } => {
                let devtools_session_state = target_devtools_session_state_mut(
                    &mut browser_context.devtools_session_state,
                    &mut browser_context.auxiliary_devtools_session_states,
                    *is_auxiliary_target_session,
                    session_id.as_deref(),
                );
                f(TargetSessionStateMut::Active {
                    devtools_session_state,
                    network_policy: &mut browser_context.network_policy,
                    tls_verify_host_override: &mut browser_context.tls_verify_host_override,
                })
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                session_id,
                is_auxiliary_target_session,
                ..
            } => browser_context.mutate_parked_page_session_state(target_id, |state| {
                let devtools_session_state = target_devtools_session_state_mut(
                    &mut state.devtools_session_state,
                    &mut state.auxiliary_devtools_session_states,
                    *is_auxiliary_target_session,
                    session_id.as_deref(),
                );
                f(TargetSessionStateMut::Parked {
                    devtools_session_state,
                    network_policy: &mut state.network_policy,
                    tls_verify_host_override: &mut state.tls_verify_host_override,
                })
            }),
            Self::NoLoadedBrowserContext => f(TargetSessionStateMut::NoLoaded),
        }
    }

    pub(super) fn mutate_session_state<T>(
        mut self,
        f: impl FnOnce(TargetSessionStateMut<'_>) -> T,
    ) -> T {
        self.mutate_session_state_ref(f)
    }

    pub(super) fn mutate_target_owner_state<T>(
        &mut self,
        f: impl FnOnce(Option<&mut TargetOwnerState>) -> T,
    ) -> T {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => f(Some(&mut browser_context.active_target.owner_state)),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .mutate_parked_target_owner_state(target_id, |owner_state| f(Some(owner_state))),
            Self::NoLoadedBrowserContext => f(None),
        }
    }

    pub(super) fn configure_fetch(
        &mut self,
        command_session_id: Option<String>,
        handle_auth_requests: bool,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.active_target.fetch_owner.configure(
                    command_session_id,
                    handle_auth_requests,
                    patterns,
                );
                Some(
                    browser_context
                        .active_target
                        .fetch_owner
                        .subresource_interception_config(),
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let mut subresource_config = (false, None);
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    state.fetch_config.configure(
                        command_session_id,
                        handle_auth_requests,
                        patterns,
                    );
                    subresource_config = state.fetch_config.subresource_interception_config();
                });
                Some(subresource_config)
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn add_network_intercept(
        &mut self,
        intercept_id: String,
        command_session_id: Option<String>,
        handle_auth_requests: bool,
        auth_url_patterns: Vec<String>,
        patterns: Vec<FetchInterceptionPattern>,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context
                    .active_target
                    .fetch_owner
                    .add_network_intercept(
                        intercept_id,
                        command_session_id,
                        handle_auth_requests,
                        auth_url_patterns,
                        patterns,
                    );
                Some(
                    browser_context
                        .active_target
                        .fetch_owner
                        .subresource_interception_config(),
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let mut subresource_config = (false, None);
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    state.fetch_config.add_network_intercept(
                        intercept_id,
                        command_session_id,
                        handle_auth_requests,
                        auth_url_patterns,
                        patterns,
                    );
                    subresource_config = state.fetch_config.subresource_interception_config();
                });
                Some(subresource_config)
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn remove_network_intercept(
        &mut self,
        intercept_id: &str,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                if !browser_context
                    .active_target
                    .fetch_owner
                    .remove_network_intercept(intercept_id)
                {
                    return None;
                }
                Some(
                    browser_context
                        .active_target
                        .fetch_owner
                        .subresource_interception_config(),
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let mut removed = false;
                let mut subresource_config = (false, None);
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    removed = state.fetch_config.remove_network_intercept(intercept_id);
                    subresource_config = state.fetch_config.subresource_interception_config();
                });
                removed.then_some(subresource_config)
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn reset_fetch_config_for_session_and_drain_pending_state(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<FetchDisableStateWithSubresourceConfig> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let previous_subresource_config = browser_context
                    .active_target
                    .fetch_owner
                    .subresource_interception_config();
                let removed = browser_context
                    .active_target
                    .fetch_owner
                    .remove_fetch_session(session_id);
                let subresource_config = browser_context
                    .active_target
                    .fetch_owner
                    .subresource_interception_config();
                let pending = if removed {
                    browser_context
                        .active_target
                        .fetch_owner
                        .drain_pending_requests_for_disable_session(session_id)
                } else {
                    empty_pending_fetch_state()
                };
                let page_update_required =
                    removed && previous_subresource_config != subresource_config;
                Some((pending, subresource_config, page_update_required))
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let previous_subresource_config = browser_context
                    .parked_page_session_state(target_id)
                    .map(|state| state.fetch_config.subresource_interception_config())
                    .unwrap_or((false, None));
                let mut subresource_config = (false, None);
                let mut removed = false;
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    removed = state.fetch_config.remove_fetch_session(session_id);
                    subresource_config = state.fetch_config.subresource_interception_config();
                });
                let pending = if removed {
                    let mut fetch_state = browser_context.take_parked_fetch_state(target_id);
                    let pending =
                        fetch_state.drain_pending_requests_for_disable_session(session_id);
                    browser_context.replace_parked_fetch_state(target_id.clone(), fetch_state);
                    pending
                } else {
                    empty_pending_fetch_state()
                };
                let page_update_required =
                    removed && previous_subresource_config != subresource_config;
                Some((pending, subresource_config, page_update_required))
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn drain_fetch_pending_state(
        &mut self,
    ) -> Option<(
        Vec<PendingFetchNavigation>,
        Vec<PendingFetchAuthNavigation>,
        Vec<PausedDocumentTransfer>,
        Vec<(String, PendingSubresourceFetchRequest)>,
        Vec<(String, PendingSubresourceFetchAuthRequest)>,
        Vec<(String, PendingSubresourceFetchResponseRequest)>,
    )> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(browser_context.take_active_target_pending_fetch_state()),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let mut fetch_state = browser_context.take_parked_fetch_state(target_id);
                let pending = fetch_state.drain_pending_requests();
                browser_context.replace_parked_fetch_state(target_id.clone(), fetch_state);
                Some(pending)
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) async fn mark_target_crashed_async(&mut self) -> Option<()> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.mark_active_target_crashed_async().await;
                Some(())
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                browser_context.background_target(target_id)?;
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.target_crash_state.mark_crashed();
                    owner_state.navigation_history_state.clear();
                    owner_state.clear_loaded_document_context_state();
                });
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    clear_parked_page_loaded_document_session_state(state);
                });
                let previous = {
                    let target = browser_context.background_target_mut(target_id)?;
                    target.runtime_slot.clear_document_navigation_state();
                    let previous = target
                        .runtime_slot
                        .clear_loaded_page_with_reason(TargetPageAbsenceReason::TargetCrashed);
                    target.runtime_slot.reset_subresource_cursor();
                    target
                        .runtime_slot
                        .reset_all_target_scoped_network_artifacts();
                    previous
                };
                browser_context.replace_parked_fetch_state(target_id.clone(), Default::default());
                browser_context
                    .replace_parked_network_artifacts(target_id.clone(), Default::default());
                if let Some(page) = previous {
                    let _ = page.close_async().await;
                }
                Some(())
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) async fn discard_loaded_page_after_failed_navigation_async(
        &mut self,
        final_url: &Url,
    ) -> Option<()> {
        let next_url = final_url.to_string();
        let security_origin = final_url.origin().ascii_serialization();
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.set_target_url(next_url);
                browser_context.set_target_security_origin(security_origin);
                if let Some(target_id) = browser_context.active_target_id_owned() {
                    browser_context.mark_target_initial_empty_document_exited(&target_id);
                }
                browser_context
                    .active_target
                    .runtime_slot
                    .clear_document_navigation_state();
                browser_context.clear_active_target_runtime_remote_object_tracking();
                let previous = browser_context
                    .clear_loaded_page_with_reason(TargetPageAbsenceReason::NavigationFailed);
                if let Some(page) = previous {
                    let _ = page.close_async().await;
                }
                Some(())
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                browser_context.background_target(target_id)?;
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.mark_initial_empty_document_exited();
                    owner_state.clear_committed_document_navigation_state();
                });
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    clear_parked_page_loaded_document_session_state(state);
                });
                let previous = {
                    let target = browser_context.background_target_mut(target_id)?;
                    target.set_target_url(next_url);
                    target.set_target_security_origin(security_origin);
                    target.runtime_slot.clear_document_navigation_state();
                    let previous = target
                        .runtime_slot
                        .clear_loaded_page_with_reason(TargetPageAbsenceReason::NavigationFailed);
                    target.runtime_slot.reset_subresource_cursor();
                    target.runtime_slot.clear_websocket_artifacts();
                    previous
                };
                if let Some(page) = previous {
                    let _ = page.close_async().await;
                }
                Some(())
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) async fn close_page_target_async(&mut self) -> Option<ClosedPageTarget> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let target_id = browser_context.active_target_id_owned()?;
                let primary_session_id = browser_context.active_session_id_owned();
                let auxiliary_session_ids =
                    browser_context.remove_auxiliary_sessions_for_target(&target_id);
                browser_context
                    .close_active_target_after_page_close_async()
                    .await;
                Some(ClosedPageTarget {
                    target_id,
                    primary_session_id,
                    auxiliary_session_ids,
                })
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let mut target = browser_context.remove_background_target(target_id)?;
                browser_context.forget_target_opener_references_for_target(target_id);
                browser_context.forget_target_window_names_for_target(target_id);
                browser_context.forget_target_popup_id_for_target(target_id);
                let _ = browser_context.take_parked_target_aux_state(target_id);
                let primary_session_id = target.detach_session();
                let auxiliary_session_ids =
                    browser_context.remove_auxiliary_sessions_for_target(target_id);
                target.close_page_async().await;
                Some(ClosedPageTarget {
                    target_id: target_id.clone(),
                    primary_session_id,
                    auxiliary_session_ids,
                })
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn runtime_slot_mut(&mut self) -> Option<&mut TargetRuntimeSlot> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(&mut browser_context.active_target.runtime_slot),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target_mut(target_id)
                .map(|target| &mut target.runtime_slot),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn into_runtime_slot_mut(self) -> Option<&'a mut TargetRuntimeSlot> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(&mut browser_context.active_target.runtime_slot),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target_mut(&target_id)
                .map(|target| &mut target.runtime_slot),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn navigation_history_snapshot(
        &mut self,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let target_url = browser_context.target_url().to_owned();
                let page_snapshot = browser_context
                    .active_target
                    .runtime_slot
                    .loaded_page()
                    .map(|page| (target_url, page.document_title()));
                Some(
                    browser_context
                        .active_target
                        .owner_state
                        .navigation_history_snapshot(page_snapshot),
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let page_snapshot =
                    browser_context
                        .background_target(target_id)
                        .and_then(|target| {
                            target
                                .runtime_slot()
                                .loaded_page()
                                .map(|page| (target.target_url().to_owned(), page.document_title()))
                        });
                Some(
                    browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                        owner_state.navigation_history_snapshot(page_snapshot)
                    }),
                )
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn apply_renderer_document_title(&mut self, title: String) -> Option<bool> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(
                browser_context
                    .active_target
                    .owner_state
                    .commit_document_title(title),
            ),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => Some(
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.commit_document_title(title)
                }),
            ),
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn navigation_history_entry_url(&mut self, entry_id: i32) -> Option<String> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => browser_context.navigation_history_entry_url(entry_id),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let page_snapshot =
                    browser_context
                        .background_target(target_id)
                        .and_then(|target| {
                            target
                                .runtime_slot()
                                .loaded_page()
                                .map(|page| (target.target_url().to_owned(), page.document_title()))
                        });
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.navigation_history_entry_url(page_snapshot, entry_id)
                })
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn reset_navigation_history(&mut self) -> Option<bool> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let target_url = browser_context.target_url().to_owned();
                let page_snapshot = browser_context
                    .active_target
                    .runtime_slot
                    .loaded_page()
                    .map(|page| (target_url, page.document_title()));
                Some(
                    browser_context
                        .active_target
                        .owner_state
                        .reset_navigation_history(page_snapshot),
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let page_snapshot =
                    browser_context
                        .background_target(target_id)
                        .and_then(|target| {
                            target
                                .runtime_slot()
                                .loaded_page()
                                .map(|page| (target.target_url().to_owned(), page.document_title()))
                        });
                Some(
                    browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                        owner_state.reset_navigation_history(page_snapshot)
                    }),
                )
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn can_reset_navigation_history(&mut self) -> Option<bool> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let target_url = browser_context.target_url().to_owned();
                let page_snapshot = browser_context
                    .active_target
                    .runtime_slot
                    .loaded_page()
                    .map(|page| (target_url, page.document_title()));
                Some(
                    browser_context
                        .active_target
                        .owner_state
                        .can_reset_navigation_history(page_snapshot),
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let page_snapshot =
                    browser_context
                        .background_target(target_id)
                        .and_then(|target| {
                            target
                                .runtime_slot()
                                .loaded_page()
                                .map(|page| (target.target_url().to_owned(), page.document_title()))
                        });
                Some(
                    browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                        owner_state.can_reset_navigation_history(page_snapshot)
                    }),
                )
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn mark_next_navigation_history_replace_current(&mut self) -> Option<()> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.mark_next_navigation_history_replace_current();
                Some(())
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.mark_next_navigation_history_replace_current();
                });
                Some(())
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn mark_next_navigation_history_traverse_to_entry(
        &mut self,
        entry_id: i32,
    ) -> Option<()> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.mark_next_navigation_history_traverse_to_entry(entry_id);
                Some(())
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.mark_next_navigation_history_traverse_to_entry(entry_id);
                });
                Some(())
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn record_same_document_navigation(
        &mut self,
        url: &Url,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) -> Option<String> {
        let next_url = url.to_string();
        let security_origin = url.origin().ascii_serialization();
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let frame_id = browser_context.active_target_id_owned()?;
                browser_context
                    .record_same_document_navigation_history(next_url.clone(), history_update);
                browser_context.set_target_url(next_url);
                browser_context.set_target_security_origin(security_origin);
                Some(frame_id)
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let frame_id = target_id.clone();
                let page_snapshot =
                    browser_context
                        .background_target(target_id)
                        .and_then(|target| {
                            target
                                .runtime_slot()
                                .loaded_page()
                                .map(|page| (target.target_url().to_owned(), page.document_title()))
                        });
                let title = page_snapshot
                    .as_ref()
                    .map(|(_, title)| title.clone())
                    .unwrap_or_default();
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.record_same_document_navigation_history(
                        page_snapshot,
                        next_url.clone(),
                        title,
                        history_update,
                    );
                });
                let target = browser_context.background_target_mut(target_id)?;
                target.set_target_url(next_url);
                target.set_target_security_origin(security_origin);
                Some(frame_id)
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn effective_extra_headers_for_target_policy(
        &self,
        mut headers: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let locale_override = match self {
            Self::ActiveTarget {
                browser_context, ..
            } => browser_context.effective_active_locale_override(),
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => browser_context.locale_override.as_deref().or_else(|| {
                browser_context
                    .parked_page_session_state(target_id)
                    .and_then(|state| state.locale_override.as_deref())
                    .or(browser_context.default_locale_override.as_deref())
            }),
            Self::NoLoadedBrowserContext => None,
        };
        headers = match self {
            Self::ActiveTarget {
                browser_context, ..
            }
            | Self::BackgroundTarget {
                browser_context, ..
            } => browser_context.merged_extra_headers_for_target_policy(&headers),
            Self::NoLoadedBrowserContext => headers,
        };
        apply_locale_header(&mut headers, locale_override);
        headers
    }

    pub(super) fn prepare_navigation_request(
        &mut self,
        requested_url: &Url,
        referrer: Option<&str>,
        is_data_url: bool,
        fallback_browser_identity: &moli_browser_profile::BrowserIdentityProfile,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> Option<TargetNavigationRequestPreflight> {
        match self {
            Self::ActiveTarget {
                browser_context,
                session_id: _,
                ..
            } => {
                let frame_id = browser_context
                    .active_target_id_owned()
                    .unwrap_or_else(|| "FRAME-0".to_owned());
                let mut request_headers = browser_context.effective_extra_headers();
                let user_agent = browser_context
                    .effective_active_browser_identity_override()
                    .unwrap_or(fallback_browser_identity)
                    .user_agent();
                apply_user_agent_header(&mut request_headers, user_agent);
                apply_referrer_header(&mut request_headers, referrer);
                let fetch_config = browser_context.active_target.fetch_owner.config_snapshot();
                let default_session_id = browser_context.active_session_id_owned();
                let fetch_snapshot = fetch_config.subresource_interception_snapshot();
                let document_request_pause = (!is_data_url)
                    .then(|| {
                        fetch_snapshot
                            .matching_request_stage_pause_sessions(
                                default_session_id.as_deref(),
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
                                default_session_id.as_deref(),
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
                        document_fetch_response_stage_candidate
                            .then_some(FetchRequestStage::Response)
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
                let active_has_network_event_listeners =
                    browser_context.has_network_event_listeners();
                let observes_document_request = active_has_network_event_listeners
                    || (!is_data_url && (fetch_config.is_enabled() || document_auth_required));
                let needs_fetch_navigation_request_id =
                    document_fetch_request_stage.is_some() || document_auth_required;
                let (document_loader_id, document_request_id, fetch_navigation_request_id) =
                    browser_context.prepare_document_navigation_request_ids(
                        network_request_id_allocator,
                        active_has_network_event_listeners,
                        observes_document_request,
                        needs_fetch_navigation_request_id,
                    );
                Some(TargetNavigationRequestPreflight {
                    frame_id,
                    session_id: default_session_id,
                    document_fetch_event_session_id,
                    inherited_security_origin: browser_context.target_security_origin().to_owned(),
                    inherited_secure_context_type: browser_context
                        .target_secure_context_type()
                        .to_owned(),
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
            Self::BackgroundTarget {
                browser_context,
                target_id,
                session_id: _,
                ..
            } => {
                let target = browser_context.background_target(target_id)?;
                let target_session_id = target.session_id().map(str::to_owned);
                let inherited_security_origin =
                    target.target_identity().security_origin().to_owned();
                let inherited_secure_context_type =
                    target.target_identity().secure_context_type().to_owned();
                let target_has_network_event_listeners =
                    target.runtime_slot().has_network_event_listeners();
                let page_state = browser_context
                    .parked_page_session_state(target_id)
                    .cloned()
                    .unwrap_or_default();
                let mut request_headers = browser_context.merged_extra_headers_for_target_policy(
                    page_state.network_policy.extra_headers(),
                );
                let user_agent = page_state
                    .network_policy
                    .browser_identity_override()
                    .or(browser_context.default_browser_identity_override.as_ref())
                    .unwrap_or(fallback_browser_identity)
                    .user_agent();
                apply_user_agent_header(&mut request_headers, user_agent);
                let locale_override = page_state
                    .locale_override
                    .as_deref()
                    .or(browser_context.default_locale_override.as_deref());
                let header_locale_override = browser_context
                    .locale_override
                    .as_deref()
                    .or(locale_override);
                apply_locale_header(&mut request_headers, header_locale_override);
                apply_referrer_header(&mut request_headers, referrer);
                let fetch_snapshot = page_state.fetch_config.subresource_interception_snapshot();
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
                let document_fetch_response_stage_candidate = !is_data_url
                    && page_state
                        .fetch_config
                        .has_document_response_stage_candidate();
                let document_fetch_request_stage = document_request_pause
                    .as_ref()
                    .map(|_| FetchRequestStage::Request)
                    .or_else(|| {
                        document_response_pause
                            .as_ref()
                            .map(|_| FetchRequestStage::Response)
                    })
                    .or_else(|| {
                        document_fetch_response_stage_candidate
                            .then_some(FetchRequestStage::Response)
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
                    !is_data_url && page_state.fetch_config.matches_auth_required(requested_url);
                let document_auth_required_blocked_intercepts = if document_auth_required {
                    page_state
                        .fetch_config
                        .matching_auth_required_network_intercepts(requested_url)
                } else {
                    Vec::new()
                };
                let observes_document_request = page_state.network_enabled
                    || target_has_network_event_listeners
                    || (!is_data_url
                        && (page_state.fetch_config.is_enabled() || document_auth_required));
                let clear_captured_response_bodies =
                    page_state.network_enabled || target_has_network_event_listeners;
                let needs_fetch_navigation_request_id =
                    document_fetch_request_stage.is_some() || document_auth_required;
                let (document_loader_id, document_request_id, fetch_navigation_request_id) =
                    browser_context
                        .background_target_mut(target_id)?
                        .prepare_document_navigation_request_ids(
                            network_request_id_allocator,
                            clear_captured_response_bodies,
                            observes_document_request,
                            needs_fetch_navigation_request_id,
                        );
                Some(TargetNavigationRequestPreflight {
                    frame_id: target_id.clone(),
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
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn prepare_loaded_navigation_commit(
        &mut self,
    ) -> Option<TargetLoadedNavigationCommitState> {
        match self {
            Self::ActiveTarget {
                browser_context,
                session_id,
                is_auxiliary_target_session,
                ..
            } => {
                let devtools_session_state = target_devtools_session_state(
                    &browser_context.devtools_session_state,
                    &browser_context.auxiliary_devtools_session_states,
                    *is_auxiliary_target_session,
                    session_id.as_deref(),
                );
                Some(TargetLoadedNavigationCommitState {
                    browser_context_id: browser_context.id.clone(),
                    runtime_frontend_enabled: devtools_session_state
                        .map(|state| state.runtime_session_state.runtime_frontend_enabled)
                        .unwrap_or_default(),
                    renderer_runtime_inspector_session_id: renderer_runtime_inspector_session_id(
                        *is_auxiliary_target_session,
                        session_id.as_deref(),
                    ),
                    runtime_inspector_session_restore_snapshots:
                        runtime_inspector_session_restore_snapshots_for_renderer(
                            &browser_context.devtools_session_state,
                            &browser_context.auxiliary_devtools_session_states,
                        ),
                    stored_runtime_bindings: runtime_bindings_for_renderer(
                        &browser_context.devtools_session_state,
                        &browser_context.auxiliary_devtools_session_states,
                    ),
                    session_runtime_bindings: devtools_session_state
                        .map(|state| state.runtime_bindings.clone())
                        .unwrap_or_default(),
                    isolated_worlds: browser_context
                        .active_target
                        .owner_state
                        .isolated_worlds
                        .clone(),
                    fetch_subresource_config: browser_context
                        .active_target
                        .fetch_owner
                        .subresource_interception_config(),
                })
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                session_id,
                is_auxiliary_target_session,
            } => {
                let page_state = browser_context
                    .parked_page_session_state(target_id)
                    .cloned()
                    .unwrap_or_default();
                let devtools_session_state = target_devtools_session_state(
                    &page_state.devtools_session_state,
                    &page_state.auxiliary_devtools_session_states,
                    *is_auxiliary_target_session,
                    session_id.as_deref(),
                );
                let isolated_worlds = browser_context
                    .parked_target_owner_state(target_id)
                    .map(|owner_state| owner_state.isolated_worlds.clone())
                    .unwrap_or_default();
                let browser_context_id = browser_context.id.clone();
                browser_context.background_target(target_id)?;
                Some(TargetLoadedNavigationCommitState {
                    browser_context_id,
                    runtime_frontend_enabled: devtools_session_state
                        .map(|state| state.runtime_session_state.runtime_frontend_enabled)
                        .unwrap_or_default(),
                    renderer_runtime_inspector_session_id: renderer_runtime_inspector_session_id(
                        *is_auxiliary_target_session,
                        session_id.as_deref(),
                    ),
                    runtime_inspector_session_restore_snapshots:
                        runtime_inspector_session_restore_snapshots_for_renderer(
                            &page_state.devtools_session_state,
                            &page_state.auxiliary_devtools_session_states,
                        ),
                    stored_runtime_bindings: runtime_bindings_for_renderer(
                        &page_state.devtools_session_state,
                        &page_state.auxiliary_devtools_session_states,
                    ),
                    session_runtime_bindings: devtools_session_state
                        .map(|state| state.runtime_bindings.clone())
                        .unwrap_or_default(),
                    isolated_worlds,
                    fetch_subresource_config: page_state
                        .fetch_config
                        .subresource_interception_config(),
                })
            }
            Self::NoLoadedBrowserContext => None,
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
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.set_target_url(next_url);
                browser_context.set_target_security_origin(security_origin);
                browser_context.set_target_secure_context_type(secure_context_type);
                if let Some(target_id) = browser_context.active_target_id_owned() {
                    browser_context.mark_target_initial_empty_document_exited(&target_id);
                }
                Some(())
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let target = browser_context.background_target_mut(target_id)?;
                target.set_target_url(next_url);
                target.set_target_security_origin(security_origin);
                target.set_target_secure_context_type(secure_context_type);
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.mark_initial_empty_document_exited();
                });
                Some(())
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) fn clear_pending_navigation_history_update(&mut self) -> Option<()> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                browser_context.clear_pending_navigation_history_update();
                Some(())
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.clear_pending_navigation_history_update()
                });
                Some(())
            }
            Self::NoLoadedBrowserContext => None,
        }
    }

    pub(super) async fn commit_loaded_navigation_page_async(
        &mut self,
        mut page: Page,
        renderer_attachment_commit: LoadedNavigationRendererAttachmentCommit,
        history_url: &Url,
    ) -> Option<anyhow::Result<LoadedNavigationPageCommit>> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                if let Some(target_id) = browser_context.active_target_id_owned() {
                    browser_context.mark_target_initial_empty_document_exited(&target_id);
                }
                Some(
                    browser_context
                        .commit_loaded_navigation_page_async(
                            page,
                            renderer_attachment_commit,
                            history_url,
                        )
                        .await,
                )
            }
            Self::BackgroundTarget {
                browser_context,
                target_id,
                ..
            } => {
                let committed_document_post_response_continuation =
                    page.take_committed_document_post_response_continuation();
                let previous_page_owner = browser_context
                    .background_target(target_id)?
                    .runtime_slot
                    .page_attachment_id()
                    .map(|page_attachment_id| {
                        TargetPageResidenceIdentity::new(
                            browser_context.id.clone(),
                            Some(target_id.clone()),
                            page_attachment_id,
                        )
                    });
                let primary_session_id = browser_context
                    .background_target(target_id)?
                    .session_id()
                    .map(str::to_owned);
                let previous_title = browser_context
                    .parked_target_owner_state(target_id)
                    .and_then(|owner_state| {
                        owner_state.committed_document_title().map(str::to_owned)
                    })
                    .or_else(|| {
                        browser_context
                            .background_target(target_id)?
                            .runtime_slot()
                            .loaded_page()
                            .map(Page::document_title)
                    });
                let committed_document_title = page.document_title();
                let page_snapshot = (history_url.to_string(), page.document_title());
                browser_context.mutate_parked_target_owner_state(target_id, |owner_state| {
                    owner_state.mark_initial_empty_document_exited();
                    if let Some(previous_title) = previous_title {
                        owner_state.refresh_current_navigation_history_title(previous_title);
                    }
                    owner_state.record_loaded_page_navigation_history(page_snapshot);
                    owner_state.clear_committed_document_navigation_state();
                    owner_state.commit_document_title(committed_document_title);
                });
                browser_context.mutate_parked_page_session_state(target_id, |state| {
                    clear_parked_page_loaded_document_session_state(state);
                });
                let (previous_attachment, new_attachment_id) = {
                    let target = browser_context.background_target_mut(target_id)?;
                    let previous_attachment = match renderer_attachment_commit {
                        LoadedNavigationRendererAttachmentCommit::Prepare(
                            renderer_agent_candidate,
                        ) => match target
                            .runtime_slot
                            .commit_loaded_navigation_renderer_attachment(
                                &mut page,
                                renderer_agent_candidate,
                            ) {
                            Ok(previous) => previous,
                            Err(error) => return Some(Err(error.into())),
                        },
                        LoadedNavigationRendererAttachmentCommit::AlreadyCommitted(transaction) => {
                            if let Err(error) = target
                                .runtime_slot
                                .bind_page_to_committed_renderer_agent_candidate(
                                    &mut page,
                                    &transaction,
                                )
                            {
                                return Some(Err(error.into()));
                            }
                            transaction.previous()
                        }
                    };
                    let new_attachment_id = page
                        .renderer_agent_attachment_id()
                        .expect("committed navigation Page must have a renderer attachment");
                    (previous_attachment, new_attachment_id)
                };
                if let Some(previous_attachment) = previous_attachment
                    && previous_attachment.id() != new_attachment_id
                {
                    let replacements =
                        match browser_context.mutate_parked_page_session_state(target_id, |state| {
                            prepare_renderer_call_replacements_for_devtools_sessions(
                                primary_session_id.as_deref(),
                                &mut state.devtools_session_state,
                                &mut state.auxiliary_devtools_session_states,
                                previous_attachment.id(),
                                new_attachment_id,
                            )
                        }) {
                            Ok(replacements) => replacements,
                            Err(error) => return Some(Err(error.into())),
                        };
                    browser_context
                        .background_target_mut(target_id)?
                        .runtime_slot
                        .install_pending_renderer_call_replacements(replacements);
                }
                let previous = {
                    let target = browser_context.background_target_mut(target_id)?;
                    let previous = target.replace_loaded_page(Some(page));
                    target.runtime_slot.reset_subresource_cursor();
                    target.runtime_slot.clear_websocket_artifacts();
                    previous
                };
                let replaced_page_owner = previous.as_ref().and(previous_page_owner);
                if let Some(page) = previous {
                    let _ = page.close_async().await;
                }
                Some(Ok(LoadedNavigationPageCommit {
                    replaced_page_owner,
                    committed_document_post_response_continuation,
                }))
            }
            Self::NoLoadedBrowserContext => None,
        }
    }
}

impl CdpConnection {
    pub(crate) fn effective_page_bypass_csp_enabled_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.effective_page_bypass_csp_enabled())
    }

    pub(crate) fn navigation_load_inputs_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> TargetNavigationLoadInputs {
        let mut inputs = match self.target_session_owner_ref(session_id) {
            Some(TargetSessionOwnerRef::NoLoadedBrowserContext) | None => {
                TargetNavigationLoadInputs::no_loaded_browser_context(
                    self.initial_storage_partition.page_storage_handles(),
                    self.engine.browser_context_owner_access(),
                )
            }
            Some(owner) => owner.navigation_load_inputs(),
        };
        if let Some(browser_context_id) = inputs.browser_context_id.as_deref() {
            inputs.permission_overrides =
                self.effective_permission_overrides_for_browser_context_id(browser_context_id);
        }
        inputs
    }

    pub(crate) fn prepare_navigation_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
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
            .target_session_owner_mut(session_id)
            .and_then(|mut owner| {
                owner.prepare_navigation_request(
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

    pub(crate) fn register_pending_fetch_navigation_request_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        pending: PendingFetchNavigation,
    ) -> Option<()> {
        self.target_session_owner_mut(session_id)?
            .register_pending_fetch_navigation_request(pending)
    }

    pub(crate) fn prepare_loaded_navigation_commit_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<TargetLoadedNavigationCommitState> {
        self.target_session_owner_mut(session_id)?
            .prepare_loaded_navigation_commit()
    }

    pub(crate) fn commit_loaded_navigation_target_identity_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        main_document_commit: &RendererMainDocumentCommit,
        target_url: &Url,
    ) -> Option<()> {
        self.target_session_owner_mut(session_id)?
            .commit_loaded_navigation_target_identity(main_document_commit, target_url)
    }

    pub(crate) async fn commit_loaded_navigation_page_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        page: Page,
        renderer_attachment_commit: LoadedNavigationRendererAttachmentCommit,
        history_url: &Url,
    ) -> Option<anyhow::Result<LoadedNavigationPageCommit>> {
        self.target_session_owner_mut(session_id)?
            .commit_loaded_navigation_page_async(page, renderer_attachment_commit, history_url)
            .await
    }

    pub(crate) fn initial_document_page_owner_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<InitialDocumentPageOwner> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
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
        if browser_context.active_target_id() == Some(owner.target_id.as_str()) {
            let loader_id = browser_context
                .active_target
                .owner_state
                .initial_empty_document_loader_id_if_current()
                .map(str::to_owned);
            browser_context.mark_target_initial_empty_document_materialized(&owner.target_id);
            let previous = browser_context.replace_loaded_page(Some(page));
            if let Some(loader_id) = loader_id {
                let _ = browser_context
                    .active_target
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
            return Ok(InitialDocumentPageInstallResult::Installed);
        }

        if browser_context
            .background_target(&owner.target_id)
            .is_none()
        {
            return Err("TargetNotLoaded".to_owned());
        }
        let loader_id = browser_context
            .parked_target_owner_state(&owner.target_id)
            .and_then(|owner_state| owner_state.initial_empty_document_loader_id_if_current())
            .map(str::to_owned);
        browser_context.mutate_parked_target_owner_state(&owner.target_id, |owner_state| {
            owner_state.mark_initial_empty_document_materialized();
            owner_state.clear_committed_document_navigation_state();
        });
        browser_context.mutate_parked_page_session_state(&owner.target_id, |state| {
            clear_parked_page_loaded_document_session_state(state);
        });
        let previous = {
            let target = browser_context
                .background_target_mut(&owner.target_id)
                .ok_or_else(|| "TargetNotLoaded".to_owned())?;
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
            previous
        };
        browser_context
            .assert_target_materialized_initial_empty_document_has_page(&owner.target_id)?;
        if let Some(page) = previous {
            let _ = page.close_async().await;
        }
        Ok(InitialDocumentPageInstallResult::Installed)
    }

    pub(crate) fn clear_pending_navigation_history_update_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<()> {
        self.target_session_owner_mut(session_id)?
            .clear_pending_navigation_history_update()
    }

    pub(crate) async fn mark_target_crashed_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<()> {
        let target_id = self
            .target_owner_identity_for_session(session_id)
            .and_then(|(_, target_id)| target_id);
        let result = self
            .target_session_owner_mut(session_id)?
            .mark_target_crashed_async()
            .await;
        if result.is_some()
            && let Some(target_id) = target_id.as_deref()
        {
            self.forget_retained_navigation_engine_for_target(target_id);
        }
        result
    }

    pub(crate) async fn discard_loaded_page_after_failed_navigation_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        final_url: &Url,
    ) -> Option<()> {
        let target_id = self
            .target_owner_identity_for_session(session_id)
            .and_then(|(_, target_id)| target_id);
        let result = self
            .target_session_owner_mut(session_id)?
            .discard_loaded_page_after_failed_navigation_async(final_url)
            .await;
        if result.is_some()
            && let Some(target_id) = target_id.as_deref()
        {
            self.forget_retained_navigation_engine_for_target(target_id);
        }
        result
    }

    pub(crate) async fn close_page_target_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<ClosedPageTarget> {
        let target_id = self
            .target_owner_identity_for_session(session_id)
            .and_then(|(_, target_id)| target_id);
        let mut collected_network_data_artifacts = self
            .target_session_owner_ref(session_id)?
            .runtime_slot()?
            .collected_network_data_artifacts();
        if let Some(target_id) = target_id.as_deref()
            && let Some(browser_context) = self.browser_context.as_mut()
        {
            collected_network_data_artifacts.extend(
                browser_context
                    .take_parked_network_artifacts(target_id)
                    .collected_network_data_artifacts(),
            );
        }
        let closed = self
            .target_session_owner_mut(session_id)?
            .close_page_target_async()
            .await?;
        self.record_collected_network_data_artifacts(collected_network_data_artifacts);
        self.forget_retained_navigation_engine_for_target(&closed.target_id);
        Some(closed)
    }

    pub(crate) async fn close_background_page_target_for_target_close_async(
        &mut self,
        target_id: &str,
        out: &mut Vec<BackgroundProtocolEvent>,
        reason: &'static str,
    ) -> Option<ClosedPageTarget> {
        let (
            mut target,
            mut parked_aux_state,
            primary_session_id,
            auxiliary_session_ids,
            collected_network_data_artifacts,
        ) = {
            let browser_context = self.browser_context.as_mut()?;
            let mut target = browser_context.remove_background_target(target_id)?;
            let mut collected_network_data_artifacts =
                target.runtime_slot().collected_network_data_artifacts();
            collected_network_data_artifacts.extend(
                browser_context
                    .take_parked_network_artifacts(target_id)
                    .collected_network_data_artifacts(),
            );
            browser_context.forget_target_opener_references_for_target(target_id);
            browser_context.forget_target_window_names_for_target(target_id);
            browser_context.forget_target_popup_id_for_target(target_id);
            let parked_aux_state = browser_context.take_parked_target_aux_state(target_id);
            let primary_session_id = target.detach_session();
            let auxiliary_session_ids =
                browser_context.remove_auxiliary_sessions_for_target(target_id);
            (
                target,
                parked_aux_state,
                primary_session_id,
                auxiliary_session_ids,
                collected_network_data_artifacts,
            )
        };
        target.close_page_async().await;
        self.record_collected_network_data_artifacts(collected_network_data_artifacts);
        self.forget_retained_navigation_engine_for_target(target_id);

        let mut affected_sessions = Vec::new();
        if let Some(session_id) = primary_session_id.as_deref() {
            affected_sessions.push(session_id);
        }
        for session_id in &auxiliary_session_ids {
            affected_sessions.push(session_id.as_str());
        }
        CdpConnection::fail_pending_inspector_awaits_from_page_session_state_for_sessions_background_events_into(
            out,
            &mut parked_aux_state.page_session_state,
            primary_session_id.as_deref(),
            &affected_sessions,
            reason,
        );

        Some(ClosedPageTarget {
            target_id: target_id.to_owned(),
            primary_session_id,
            auxiliary_session_ids,
        })
    }

    pub(crate) async fn close_active_page_target_for_target_close_async(
        &mut self,
    ) -> Option<ClosedPageTarget> {
        let (
            target_id,
            primary_session_id,
            auxiliary_session_ids,
            promoted_target_id,
            collected_network_data_artifacts,
        ) = {
            let browser_context = self.browser_context.as_mut()?;
            let target_id = browser_context.active_target_id_owned()?;
            let primary_session_id = browser_context.active_session_id_owned();
            let auxiliary_session_ids =
                browser_context.remove_auxiliary_sessions_for_target(&target_id);
            let promoted_target_id = browser_context.last_promotable_background_target_id();
            let collected_network_data_artifacts = browser_context
                .active_target
                .runtime_slot
                .collected_network_data_artifacts();
            browser_context.clear_active_target_session_scoped_state_without_loaded_page();
            browser_context
                .reset_active_target_slot_to_empty_async()
                .await;
            (
                target_id,
                primary_session_id,
                auxiliary_session_ids,
                promoted_target_id,
                collected_network_data_artifacts,
            )
        };
        self.record_collected_network_data_artifacts(collected_network_data_artifacts);
        if let Some(promoted_target_id) = promoted_target_id.as_deref() {
            self.handoff_navigation_engine_for_target_activation(promoted_target_id);
        }
        if let Some(browser_context) = self.browser_context.as_mut() {
            let _ = browser_context
                .promote_last_background_target_to_active_async()
                .await;
        }
        self.refresh_active_browser_context_loader_async().await;

        Some(ClosedPageTarget {
            target_id,
            primary_session_id,
            auxiliary_session_ids,
        })
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
            let is_active_target = self
                .browser_context_by_id(&browser_context_id)
                .is_some_and(|browser_context| browser_context.is_active_target(target_id));
            if is_active_target {
                if let Some(browser_context) = self.browser_context_by_id_mut(&browser_context_id) {
                    if let Some((_target_id, session_id)) = browser_context.active_target_identity()
                        && let Some(session_id) = session_id
                    {
                        page_session_ids.push(session_id);
                    }
                    page_session_ids
                        .extend(browser_context.remove_auxiliary_sessions_for_target(target_id));
                    browser_context
                        .reset_active_target_slot_to_empty_async()
                        .await;
                }
            } else {
                let close_background_target = {
                    let Some(browser_context) = self.browser_context_by_id_mut(&browser_context_id)
                    else {
                        return;
                    };
                    let target = browser_context.remove_background_target(target_id);
                    browser_context.forget_target_opener_references_for_target(target_id);
                    browser_context.forget_target_window_names_for_target(target_id);
                    browser_context.forget_target_popup_id_for_target(target_id);
                    page_session_ids
                        .extend(browser_context.remove_auxiliary_sessions_for_target(target_id));
                    target.map(|mut target| {
                        if let Some(session_id) = target.detach_session() {
                            page_session_ids.push(session_id);
                        }
                        target
                    })
                };
                if let Some(mut target) = close_background_target {
                    target.close_page_async().await;
                }
            }
        }

        for session_id in page_session_ids {
            self.rollback_attached_session_without_event(&session_id);
        }
        self.forget_retained_navigation_engine_for_target(target_id);
    }

    pub(crate) fn target_session_owner_aggregate_fetch_config(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetFetchConfig> {
        self.target_session_owner_ref(session_id)?
            .aggregate_fetch_config()
    }

    pub(crate) fn target_page_session_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&TargetPageSessionState> {
        self.target_session_owner_ref(session_id)?
            .page_session_state()
    }

    pub(crate) fn target_devtools_session_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&DevToolsSessionState> {
        self.target_session_owner_ref(session_id)?
            .devtools_session_state()
    }

    pub(crate) fn target_runtime_session_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&TargetRuntimeSessionState> {
        self.target_session_owner_ref(session_id)?
            .runtime_session_state()
    }

    pub(crate) fn target_runtime_bindings_for_renderer_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Vec<RuntimeBindingDefinition> {
        self.target_session_owner_ref(session_id)
            .map(|owner| owner.runtime_bindings_for_renderer())
            .unwrap_or_default()
    }

    pub(crate) fn target_runtime_bindings_for_current_inspector_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Vec<RuntimeBindingDefinition> {
        self.target_session_owner_ref(session_id)
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

    pub(crate) fn target_owner_state_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&TargetOwnerState> {
        self.target_session_owner_ref(session_id)?
            .target_owner_state()
    }

    pub(crate) fn target_owner_has_bidi_channel_preload_script_for_session(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let target_owner_has_script = self
            .target_owner_state_for_session(session_id)
            .is_some_and(TargetOwnerState::has_bidi_channel_preload_script);
        if target_owner_has_script {
            return true;
        }
        self.target_owner_identity_for_session(session_id)
            .and_then(|(browser_context_id, _)| self.browser_context_by_id(&browser_context_id))
            .is_some_and(|browser_context| {
                browser_context.has_default_bidi_channel_preload_script()
            })
    }

    pub(crate) fn target_owner_bidi_channel_preload_handoffs_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Vec<BidiPreloadChannelHandoff> {
        let mut handoffs = Vec::new();
        if let Some(owner_state) = self.target_owner_state_for_session(session_id) {
            handoffs.extend(
                owner_state
                    .document_start_scripts
                    .iter()
                    .flat_map(|(_, script)| script.bidi_channel_handoffs.clone()),
            );
        }
        if let Some(browser_context) = self
            .target_owner_identity_for_session(session_id)
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
        self.target_session_owner_mut(session_id)?
            .mutate_target_owner_state(|owner_state| owner_state.map(f))
    }

    pub(crate) fn with_target_devtools_session_state_for_session_mut<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut DevToolsSessionState) -> R,
    ) -> Option<R> {
        self.target_session_owner_mut(session_id)?
            .mutate_session_state(|mut state| state.devtools_session_state_mut().map(f))
    }

    pub(crate) fn target_owner_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<(String, Option<String>)> {
        self.target_session_owner_ref(session_id)?.owner_identity()
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
        let (browser_context_id, routed_target_id) =
            self.target_owner_identity_for_session(session_id)?;
        // `None` in a `CdpSessionRoute::ActiveTarget` is a routing shorthand:
        // it means "whichever target is currently active". A Page residence
        // identity must never retain that mutable shorthand. Freeze the
        // concrete target now, otherwise a later `browsingContext.create`
        // can make output from the old Page resolve through the new active
        // target while preserving the same browser-context id and residence.
        let target_id = routed_target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned)
        });
        let page_attachment_id = self
            .runtime_session_owner_slot(session_id)
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
    pub(crate) fn pending_target_page_residence_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetPageResidenceIdentity> {
        let (browser_context_id, routed_target_id) =
            self.target_owner_identity_for_session(session_id)?;
        let target_id = routed_target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned)
        });
        let page_attachment_id = self
            .runtime_session_owner_slot(session_id)
            .ok()?
            .pending_page_attachment_id()?;
        Some(TargetPageResidenceIdentity::new(
            browser_context_id,
            target_id,
            page_attachment_id,
        ))
    }

    /// Binds a renderer Page reservation to its explicit future target Page
    /// attachment and returns that exact residence identity.
    pub(crate) fn reserve_target_page_residence_identity_for_session(
        &mut self,
        session_id: Option<&str>,
        renderer_page: crate::conn::RendererPageResidenceIdentity,
    ) -> Option<TargetPageResidenceIdentity> {
        let (browser_context_id, routed_target_id) =
            self.target_owner_identity_for_session(session_id)?;
        let target_id = routed_target_id.or_else(|| {
            self.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned)
        });
        let page_attachment_id = self
            .runtime_session_owner_slot_mut(session_id)
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
    pub(crate) fn target_page_residence_identity_is_current_for_session(
        &self,
        session_id: Option<&str>,
        expected: &TargetPageResidenceIdentity,
    ) -> bool {
        self.target_page_residence_identity_for_session(session_id)
            .as_ref()
            == Some(expected)
    }

    /// Captures the lifetime of the concrete Page attachment currently
    /// addressed by `session_id`.
    ///
    /// The returned token is owned by that attachment rather than by a numeric
    /// slot generation. Moving the whole target slot preserves it; replacing
    /// or clearing the installed Page terminates it directly.
    pub(crate) fn capture_target_page_residence_token_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::TargetPageResidenceToken> {
        self.runtime_session_owner_slot_mut(session_id)
            .ok()?
            .page_slot_mut()
            .page_residence_token()
    }

    /// Captures the exact protocol attachment currently addressing a Page.
    pub(crate) fn target_page_protocol_attachment_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::TargetPageProtocolAttachmentIdentity> {
        Some(crate::conn::TargetPageProtocolAttachmentIdentity::new(
            self.target_page_residence_identity_for_session(session_id)?,
            session_id.map(str::to_owned),
        ))
    }

    /// Captures the renderer-side identity of the Page currently addressed by
    /// `session_id`.
    ///
    /// Callers must capture this while any `None`-session owner-route override
    /// is active. The returned identity is self-contained and must not be
    /// reconstructed later from a session that may then address another Page.
    pub(crate) fn renderer_page_residence_identity_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::RendererPageResidenceIdentity> {
        self.runtime_session_owner_slot(session_id)
            .ok()?
            .loaded_page()
            .map(crate::conn::RendererPageResidenceIdentity::from_page)
    }

    /// Resolves the exact protocol attachment named by one renderer inspector
    /// output route, while proving that it still belongs to the Page whose
    /// source snapshot is being captured.
    ///
    /// The renderer uses `None` for the target's primary inspector session and
    /// the concrete CDP session id for auxiliary sessions. Deferred protocol
    /// output must translate that renderer-local convention once, at capture
    /// time. Looking it up again during drain could route an old Page's
    /// response or notification through a replacement Page or an unrelated
    /// contextual command session.
    pub(crate) fn target_page_protocol_attachment_identity_for_renderer_inspector_route(
        &self,
        source_session_id: Option<&str>,
        renderer_inspector_session_id: Option<&str>,
    ) -> Option<crate::conn::TargetPageProtocolAttachmentIdentity> {
        let source =
            self.target_page_protocol_attachment_identity_for_session(source_session_id)?;
        let protocol_session_id = renderer_inspector_session_id
            .map(str::to_owned)
            .or_else(|| self.runtime_session_owner_primary_session_id(source_session_id));
        let attachment = self
            .target_page_protocol_attachment_identity_for_session(protocol_session_id.as_deref())?;
        if attachment.page_owner() != source.page_owner()
            || self
                .target_renderer_runtime_inspector_session_id_for_session(
                    protocol_session_id.as_deref(),
                )
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
        self.target_page_residence_identity_is_current_for_session(
            expected.session_id(),
            expected.page_owner(),
        )
    }

    /// Binds renderer-produced child-frame activity to the Page attachment
    /// that captured it and to the exact root Document reported by the same
    /// renderer snapshot.
    pub(crate) fn target_root_document_protocol_attachment_identity_for_session(
        &self,
        session_id: Option<&str>,
        root_document: moli_core::RendererDocumentLifecycleIdentity,
    ) -> Option<crate::conn::TargetRootDocumentProtocolAttachmentIdentity> {
        let binding = crate::conn::TargetRootDocumentProtocolAttachmentIdentity::new(
            self.target_page_protocol_attachment_identity_for_session(session_id)?,
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

    pub(crate) fn target_root_document_lifecycle_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<moli_core::RendererDocumentLifecycleIdentity> {
        self.runtime_session_owner_slot(session_id)
            .ok()?
            .page_slot()
            .renderer_document_lifecycle_binding()
            .map(crate::conn::CommittedRendererDocumentBinding::renderer_document_identity)
    }

    /// Resolves one concrete Page attachment for a target-owned event.
    ///
    /// The primary Page session is preferred, followed by a stable auxiliary
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
                    .auxiliary_session_ids_for_target(target_id)
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

    pub(crate) fn target_devtools_auxiliary_session_id_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<Option<String>> {
        match self.target_session_owner(session_id)? {
            TargetSessionOwner::ActiveTarget {
                is_auxiliary_target_session,
                ..
            }
            | TargetSessionOwner::BackgroundTarget {
                is_auxiliary_target_session,
                ..
            } => Some(
                is_auxiliary_target_session
                    .then(|| session_id.map(str::to_owned))
                    .flatten(),
            ),
            TargetSessionOwner::NoLoadedBrowserContext => Some(None),
        }
    }

    pub(crate) fn runtime_session_owner_slot_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut TargetRuntimeSlot, String> {
        let renderer_inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let slot = self
            .target_session_owner_mut(session_id)
            .and_then(TargetSessionOwnerMut::into_runtime_slot_mut)
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
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.runtime_slot())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())
    }

    pub(crate) fn runtime_session_owner_primary_session_id(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.target_session_owner_ref(session_id)?
            .primary_session_id()
    }

    pub(crate) fn page_event_session_ids_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
        else {
            return vec![session_id.map(str::to_owned)];
        };
        let Some(browser_context) = self.browser_context_by_id(&browser_context_id) else {
            return vec![session_id.map(str::to_owned)];
        };
        let Some(target_id) = target_id else {
            return vec![session_id.map(str::to_owned)];
        };

        let mut session_ids = Vec::new();
        let primary_session_id = if browser_context.active_target_id() == Some(target_id.as_str()) {
            browser_context.active_session_id_owned()
        } else {
            browser_context
                .background_target(&target_id)
                .and_then(|target| target.session_id().map(str::to_owned))
        };
        let primary_event_session_id = primary_session_id.or_else(|| session_id.map(str::to_owned));
        session_ids.push(primary_event_session_id.clone());
        for auxiliary_session_id in browser_context.auxiliary_session_ids_for_target(&target_id) {
            if primary_event_session_id.as_deref() != Some(auxiliary_session_id.as_str()) {
                session_ids.push(Some(auxiliary_session_id));
            }
        }
        session_ids
    }

    pub(crate) fn subscribed_page_event_session_ids_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        self.page_event_session_ids_for_session_owner(session_id)
            .into_iter()
            .filter(|event_session_id| {
                self.target_page_session_state_for_session(event_session_id.as_deref())
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
    pub(crate) fn page_event_protocol_attachments_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<Vec<crate::conn::TargetPageProtocolAttachmentIdentity>> {
        let source = self.target_page_protocol_attachment_identity_for_session(session_id)?;
        let attachments = self
            .subscribed_page_event_session_ids_for_session_owner(session_id)
            .into_iter()
            .map(|event_session_id| {
                self.target_page_protocol_attachment_identity_for_session(
                    event_session_id.as_deref(),
                )
            })
            .collect::<Option<Vec<_>>>()?;
        (!attachments.is_empty()
            && attachments
                .iter()
                .all(|attachment| attachment.page_owner() == source.page_owner()))
        .then_some(attachments)
    }

    pub(crate) fn runtime_session_owner_target_url(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.target_session_owner_ref(session_id)?.target_url()
    }

    pub(crate) fn runtime_session_owner_record_initial_empty_document_url(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.target_session_owner_ref(session_id)?
            .initial_empty_document_url_if_current()
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_storage_key(
        &self,
        session_id: Option<&str>,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.target_session_owner_ref(session_id)?
            .initial_empty_document_storage_key_if_current()
    }

    pub(crate) fn runtime_session_owner_record_is_on_initial_empty_document(
        &self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_ref(session_id)?
            .is_on_initial_empty_document()
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_pending_cross_document_navigation(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.target_session_owner_ref(session_id)
            .is_some_and(|owner| {
                owner.initial_empty_document_has_pending_cross_document_navigation()
            })
    }

    pub(crate) fn target_session_owner_frame_tree_identity(
        &self,
        session_id: Option<&str>,
    ) -> Option<(String, String, String, String)> {
        self.target_session_owner_ref(session_id)?
            .frame_tree_identity()
    }

    pub(crate) fn target_session_owner_frame_tree_loader_id(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.target_session_owner_ref(session_id)?
            .frame_tree_loader_id()
    }

    pub(crate) fn target_session_owner_emulated_device_metrics(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::EmulatedDeviceMetrics> {
        self.target_session_owner_ref(session_id)?
            .emulated_device_metrics()
    }

    pub(crate) fn target_session_owner_navigation_history_snapshot(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        self.target_session_owner_mut(session_id)?
            .navigation_history_snapshot()
    }

    pub(crate) fn apply_renderer_document_title_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        change: &moli_core::RendererDocumentTitleChanged,
    ) -> Option<bool> {
        self.target_root_document_protocol_attachment_identity_for_session(
            session_id,
            change.source_document,
        )?;
        self.target_session_owner_mut(session_id)?
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

    pub(crate) fn reset_navigation_history_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_mut(session_id)?
            .reset_navigation_history()
    }

    pub(crate) fn can_reset_navigation_history_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<bool> {
        self.target_session_owner_mut(session_id)?
            .can_reset_navigation_history()
    }

    pub(crate) fn mark_next_navigation_history_replace_current_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<()> {
        self.target_session_owner_mut(session_id)?
            .mark_next_navigation_history_replace_current()
    }

    pub(crate) fn mark_next_navigation_history_traverse_to_entry_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        entry_id: i32,
    ) -> Option<()> {
        self.target_session_owner_mut(session_id)?
            .mark_next_navigation_history_traverse_to_entry(entry_id)
    }

    pub(crate) fn record_same_document_navigation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        url: &Url,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) -> Option<String> {
        self.target_session_owner_mut(session_id)?
            .record_same_document_navigation(url, history_update)
    }

    pub(super) fn target_session_owner_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwnerMut<'_>> {
        match self.target_session_owner(session_id)? {
            TargetSessionOwner::ActiveTarget {
                browser_context_id,
                is_auxiliary_target_session,
            } => {
                let is_current_active_browser_context = self
                    .browser_context
                    .as_ref()
                    .is_some_and(|browser_context| browser_context.id == browser_context_id);
                self.browser_context_by_id_mut(&browser_context_id)
                    .map(|browser_context| TargetSessionOwnerMut::ActiveTarget {
                        browser_context,
                        session_id: session_id.map(str::to_owned),
                        is_auxiliary_target_session,
                        is_current_active_browser_context,
                    })
            }
            TargetSessionOwner::BackgroundTarget {
                browser_context_id,
                target_id,
                is_auxiliary_target_session,
            } => self
                .browser_context_by_id_mut(&browser_context_id)
                .map(|browser_context| TargetSessionOwnerMut::BackgroundTarget {
                    browser_context,
                    target_id,
                    session_id: session_id.map(str::to_owned),
                    is_auxiliary_target_session,
                }),
            TargetSessionOwner::NoLoadedBrowserContext => {
                Some(TargetSessionOwnerMut::NoLoadedBrowserContext)
            }
        }
    }

    pub(super) fn target_session_owner_ref(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwnerRef<'_>> {
        match self.target_session_owner(session_id)? {
            TargetSessionOwner::ActiveTarget {
                browser_context_id,
                is_auxiliary_target_session,
            } => self
                .browser_context_by_id(&browser_context_id)
                .map(|browser_context| TargetSessionOwnerRef::ActiveTarget {
                    browser_context,
                    session_id: session_id.map(str::to_owned),
                    is_auxiliary_target_session,
                }),
            TargetSessionOwner::BackgroundTarget {
                browser_context_id,
                target_id,
                is_auxiliary_target_session,
            } => self
                .browser_context_by_id(&browser_context_id)
                .map(|browser_context| TargetSessionOwnerRef::BackgroundTarget {
                    browser_context,
                    target_id,
                    session_id: session_id.map(str::to_owned),
                    is_auxiliary_target_session,
                }),
            TargetSessionOwner::NoLoadedBrowserContext => {
                Some(TargetSessionOwnerRef::NoLoadedBrowserContext)
            }
        }
    }

    pub(super) fn with_target_session_owner_mut<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(TargetSessionOwnerMut<'_>) -> R,
    ) -> Option<R> {
        self.target_session_owner_mut(session_id).map(f)
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
        conn.browser_context = Some(browser_context);

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

    #[test]
    fn target_session_owner_mut_mutates_active_and_parked_owner_state() {
        let mut active = BrowserContext::new("BID-active".to_owned());
        {
            let mut owner = TargetSessionOwnerMut::ActiveTarget {
                browser_context: &mut active,
                session_id: None,
                is_auxiliary_target_session: false,
                is_current_active_browser_context: true,
            };
            owner.mutate_target_owner_state(|owner_state| {
                owner_state
                    .expect("active target owner state")
                    .target_crash_state
                    .mark_crashed();
            });
        }
        assert!(
            active
                .active_target
                .owner_state
                .target_crash_state
                .is_crashed()
        );

        let mut parked = BrowserContext::new("BID-parked".to_owned());
        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            owner.mutate_target_owner_state(|owner_state| {
                owner_state
                    .expect("parked target owner state")
                    .target_crash_state
                    .mark_crashed();
            });
        }
        assert!(
            parked
                .parked_target_owner_state_or_default("TID-parked")
                .target_crash_state
                .is_crashed()
        );

        let mut no_loaded = TargetSessionOwnerMut::NoLoadedBrowserContext;
        no_loaded.mutate_target_owner_state(|owner_state| {
            assert!(owner_state.is_none());
        });
    }

    #[test]
    fn renderer_runtime_inspector_session_id_tracks_owner_session_kind() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner-key".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active-primary".to_owned());
        browser_context
            .active_target
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        browser_context
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
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
                .assign_auxiliary_session_to_target("TID-active", "SID-active-aux".to_owned(),)
        );
        assert!(
            browser_context.assign_auxiliary_session_to_target(
                "TID-background",
                "SID-background-aux".to_owned(),
            )
        );
        conn.browser_context = Some(browser_context);

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
            conn.target_renderer_runtime_inspector_session_id_for_session(Some("SID-active-aux")),
            Some("SID-active-aux".to_owned()),
            "auxiliary active-target session owns a distinct renderer inspector session"
        );
        assert_eq!(
            conn.target_renderer_runtime_inspector_session_id_for_session(Some(
                "SID-background-aux"
            )),
            Some("SID-background-aux".to_owned()),
            "auxiliary background-target session owns a distinct renderer inspector session"
        );

        let default_route = conn
            .target_page_protocol_attachment_identity_for_renderer_inspector_route(
                Some("SID-active-aux"),
                None,
            )
            .expect("default inspector route should resolve through the target primary session");
        assert_eq!(default_route.session_id(), Some("SID-active-primary"));

        let auxiliary_route = conn
            .target_page_protocol_attachment_identity_for_renderer_inspector_route(
                Some("SID-active-primary"),
                Some("SID-active-aux"),
            )
            .expect("auxiliary inspector route should retain its exact protocol attachment");
        assert_eq!(auxiliary_route.session_id(), Some("SID-active-aux"));

        assert!(
            conn.target_page_protocol_attachment_identity_for_renderer_inspector_route(
                Some("SID-active-primary"),
                Some("SID-background-aux"),
            )
            .is_none(),
            "a renderer batch must not borrow an inspector session attached to another target"
        );
    }

    #[test]
    fn target_session_owner_mut_mutates_active_and_parked_fetch_state() {
        let mut active = BrowserContext::new("BID-active".to_owned());
        {
            let mut owner = TargetSessionOwnerMut::ActiveTarget {
                browser_context: &mut active,
                session_id: None,
                is_auxiliary_target_session: false,
                is_current_active_browser_context: true,
            };
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-active".to_owned(),
                pending_subresource_fetch(1),
            ));
        }
        assert!(
            active
                .active_target
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-active")
        );

        let mut parked = BrowserContext::new("BID-parked".to_owned());
        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-parked".to_owned(),
                pending_subresource_fetch(2),
            ));
        }
        assert!(
            parked
                .parked_fetch_state("TID-parked")
                .expect("parked fetch state")
                .has_pending_subresource_fetch_for_test("FETCH-parked")
        );

        let mut no_loaded = TargetSessionOwnerMut::NoLoadedBrowserContext;
        assert!(!no_loaded.register_pending_subresource_fetch_request(
            "FETCH-missing".to_owned(),
            pending_subresource_fetch(3),
        ));
    }

    #[test]
    fn target_session_owner_mut_configures_and_resets_active_and_parked_fetch_state() {
        let patterns = vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: None,
            request_stage: FetchRequestStage::Request,
        }];

        let mut active = BrowserContext::new("BID-active".to_owned());
        {
            let mut owner = TargetSessionOwnerMut::ActiveTarget {
                browser_context: &mut active,
                session_id: None,
                is_auxiliary_target_session: false,
                is_current_active_browser_context: true,
            };
            assert_eq!(
                owner.configure_fetch(Some("SID-active".to_owned()), true, patterns.clone()),
                Some((true, None))
            );
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-active".to_owned(),
                pending_subresource_fetch(11),
            ));
            let (pending, subresource_config, page_update_required) = owner
                .reset_fetch_config_for_session_and_drain_pending_state(Some("SID-active"))
                .expect("active pending fetch state should drain");
            assert_eq!(subresource_config, (false, None));
            assert!(page_update_required);
            assert_eq!(pending.3.len(), 1);
        }
        assert!(
            !active
                .active_target
                .fetch_owner
                .config_snapshot()
                .is_enabled()
        );
        assert!(
            !active
                .active_target
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-active")
        );

        let mut parked = BrowserContext::new("BID-parked".to_owned());
        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            assert_eq!(
                owner.configure_fetch(Some("SID-parked".to_owned()), true, patterns),
                Some((true, None))
            );
            assert!(owner.register_pending_subresource_fetch_request(
                "FETCH-parked".to_owned(),
                pending_subresource_fetch(22),
            ));
            let (pending, subresource_config, page_update_required) = owner
                .reset_fetch_config_for_session_and_drain_pending_state(Some("SID-parked"))
                .expect("parked pending fetch state should drain");
            assert_eq!(subresource_config, (false, None));
            assert!(page_update_required);
            assert_eq!(pending.3.len(), 1);
        }
        assert!(
            parked
                .parked_page_session_state("TID-parked")
                .is_none_or(|state| !state.fetch_config.is_enabled())
        );
        assert!(parked.parked_fetch_state("TID-parked").is_none());

        let mut no_loaded = TargetSessionOwnerMut::NoLoadedBrowserContext;
        assert_eq!(
            no_loaded.configure_fetch(Some("SID-missing".to_owned()), true, Vec::new()),
            None
        );
        assert!(
            no_loaded
                .reset_fetch_config_for_session_and_drain_pending_state(Some("SID-missing"))
                .is_none()
        );
    }

    #[test]
    fn target_session_owner_mut_snapshots_active_and_parked_navigation_history() {
        let mut active = BrowserContext::new("BID-active".to_owned());
        active
            .active_target
            .owner_state
            .record_loaded_page_navigation_history((
                "https://active.example/".to_owned(),
                "active".to_owned(),
            ));
        {
            let mut owner = TargetSessionOwnerMut::ActiveTarget {
                browser_context: &mut active,
                session_id: None,
                is_auxiliary_target_session: false,
                is_current_active_browser_context: true,
            };
            let (current_index, entries) = owner
                .navigation_history_snapshot()
                .expect("active history should snapshot");
            assert_eq!(current_index, 0);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].url, "https://active.example/");
        }

        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked.mutate_parked_target_owner_state("TID-parked", |owner_state| {
            owner_state.record_loaded_page_navigation_history((
                "https://parked.example/".to_owned(),
                "parked".to_owned(),
            ));
        });
        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            let (current_index, entries) = owner
                .navigation_history_snapshot()
                .expect("parked history should snapshot");
            assert_eq!(current_index, 0);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].url, "https://parked.example/");
        }

        let mut no_loaded = TargetSessionOwnerMut::NoLoadedBrowserContext;
        assert!(no_loaded.navigation_history_snapshot().is_none());
    }

    #[test]
    fn target_session_owner_mut_prepares_active_navigation_request_preflight() {
        let mut active = BrowserContext::new("BID-active".to_owned());
        active
            .active_target
            .runtime_slot
            .enable_primary_network_events();
        active.record_captured_response_body(
            "REQ-old".to_owned(),
            "old body".to_owned(),
            vec![None],
        );
        assert!(active.has_captured_response_body_for_test("REQ-old"));

        let mut owner = TargetSessionOwnerMut::ActiveTarget {
            browser_context: &mut active,
            session_id: Some("SID-active".to_owned()),
            is_auxiliary_target_session: false,
            is_current_active_browser_context: true,
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
        let mut active = BrowserContext::new("BID-active".to_owned());
        active
            .active_target
            .runtime_slot
            .enable_primary_network_events();
        let mut owner = TargetSessionOwnerMut::ActiveTarget {
            browser_context: &mut active,
            session_id: None,
            is_auxiliary_target_session: false,
            is_current_active_browser_context: true,
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
        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked.locale_override = Some("zh-CN".to_owned());
        parked
            .network_policy
            .set_browser_identity_override(test_browser_identity("Active-Only-UA"));
        parked.default_browser_identity_override =
            Some(test_browser_identity("Browser-Context-Default-UA"));
        parked
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-parked".to_owned(),
                Some("SID-parked".to_owned()),
                "about:blank".to_owned(),
            ));
        parked.mutate_parked_page_session_state("TID-parked", |state| {
            state.network_enabled = true;
            state
                .network_policy
                .push_extra_header(("X-Owner".to_owned(), "parked".to_owned()));
            state.fetch_config.configure(
                Some("SID-parked".to_owned()),
                false,
                vec![FetchInterceptionPattern {
                    url_pattern: "*".to_owned(),
                    resource_type_filter: None,
                    request_stage: FetchRequestStage::Response,
                }],
            );
        });
        parked
            .background_target_mut("TID-parked")
            .expect("background target should exist")
            .runtime_slot
            .record_captured_response_body(
                "REQ-old".to_owned(),
                "old body".to_owned(),
                vec![Some("SID-parked".to_owned())],
            );
        assert!(
            parked
                .background_target("TID-parked")
                .expect("background target should exist")
                .runtime_slot()
                .has_captured_response_body("REQ-old")
        );

        let mut owner = TargetSessionOwnerMut::BackgroundTarget {
            browser_context: &mut parked,
            target_id: "TID-parked".to_owned(),
            session_id: Some("SID-parked".to_owned()),
            is_auxiliary_target_session: false,
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

        assert_eq!(preflight.frame_id, "TID-parked");
        assert_eq!(preflight.session_id.as_deref(), Some("SID-parked"));
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
            !parked
                .background_target("TID-parked")
                .expect("background target should exist")
                .runtime_slot()
                .has_captured_response_body("REQ-old")
        );
        assert!(
            preflight
                .request_headers
                .contains(&("X-Owner".to_owned(), "parked".to_owned()))
        );
        assert!(
            preflight
                .request_headers
                .contains(&("Accept-Language".to_owned(), "zh-CN".to_owned()))
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
        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-parked".to_owned(),
                Some("SID-parked".to_owned()),
                "about:blank".to_owned(),
            ));
        parked.mutate_parked_page_session_state("TID-parked", |state| {
            state.network_enabled = true;
        });

        let mut owner = TargetSessionOwnerMut::BackgroundTarget {
            browser_context: &mut parked,
            target_id: "TID-parked".to_owned(),
            session_id: None,
            is_auxiliary_target_session: false,
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

        assert_eq!(preflight.frame_id, "TID-parked");
        assert_eq!(preflight.session_id.as_deref(), Some("SID-parked"));
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
        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked.locale_override = Some("zh-CN".to_owned());
        parked
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-parked".to_owned(),
                Some("SID-parked".to_owned()),
                "https://parked.example/start".to_owned(),
            ));
        parked.mutate_parked_page_session_state("TID-parked", |state| {
            state.locale_override = Some("fr-FR".to_owned());
            state.timezone_override = Some("Europe/Paris".to_owned());
            state.http_proxy_override = Some("http://proxy.example:8080".to_owned());
            state.http_no_proxy_override = Some("localhost,127.0.0.1".to_owned());
            state.tls_verify_host_override = Some(false);
            state.script_execution_disabled = true;
            state
                .network_policy
                .set_user_agent_override("OwnerUA/1.0".to_owned());
            state.network_policy.set_network_offline(true);
            state
                .network_policy
                .push_blocked_url_pattern("*.blocked.test".to_owned());
            state
                .network_policy
                .push_extra_header(("X-Owner".to_owned(), "parked".to_owned()));
            state.emulated_media.media = Some("print".to_owned());
            state.fetch_config.configure(
                Some("SID-parked".to_owned()),
                false,
                vec![FetchInterceptionPattern {
                    url_pattern: "*".to_owned(),
                    resource_type_filter: None,
                    request_stage: FetchRequestStage::Request,
                }],
            );
        });
        parked.mutate_parked_target_owner_state("TID-parked", |owner_state| {
            owner_state.document_start_scripts.push((
                "1".to_owned(),
                DocumentStartScript {
                    registry_key: None,
                    source: "globalThis.fromParkedPreload = true;".to_owned(),
                    world_name: Some("utility".to_owned()),
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ));
        });
        parked.mutate_parked_page_session_state("TID-parked", |state| {
            state
                .devtools_session_state
                .upsert_runtime_binding_definition(
                    "fromParkedBinding".to_owned(),
                    Some("utility".to_owned()),
                );
        });

        let owner = TargetSessionOwnerRef::BackgroundTarget {
            browser_context: &parked,
            target_id: "TID-parked".to_owned(),
            session_id: Some("SID-parked".to_owned()),
            is_auxiliary_target_session: false,
        };
        let inputs = owner.navigation_load_inputs();

        assert_eq!(inputs.browser_context_id.as_deref(), Some("BID-parked"));
        assert!(
            inputs
                .renderer_runtime
                .runtime()
                .shares_state_with(&parked.renderer_runtime()),
            "background navigation must reuse the browser-context renderer runtime"
        );
        assert_eq!(
            inputs.navigation_initiator_url.as_ref().map(Url::as_str),
            Some("https://parked.example/start")
        );
        assert!(
            inputs
                .document_start_scripts
                .iter()
                .any(|script| script.source == "globalThis.fromParkedPreload = true;")
        );
        assert_eq!(
            inputs.runtime_bindings,
            vec![RuntimeBindingDefinition {
                name: "fromParkedBinding".to_owned(),
                execution_context_name: Some("utility".to_owned()),
            }]
        );
        assert!(
            inputs
                .extra_http_headers
                .contains(&("X-Owner".to_owned(), "parked".to_owned()))
        );
        assert!(
            inputs
                .extra_http_headers
                .contains(&("Accept-Language".to_owned(), "zh-CN".to_owned()))
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
        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-parked".to_owned(),
                Some("SID-parked".to_owned()),
                "about:blank".to_owned(),
            ));
        parked.mutate_parked_page_session_state("TID-parked", |state| {
            state
                .devtools_session_state
                .page_session_state
                .page_lifecycle_events = true;
            state
                .devtools_session_state
                .runtime_session_state
                .runtime_frontend_enabled = true;
            state.fetch_config.configure(
                Some("SID-parked".to_owned()),
                false,
                vec![FetchInterceptionPattern {
                    url_pattern: "*".to_owned(),
                    resource_type_filter: None,
                    request_stage: FetchRequestStage::Request,
                }],
            );
        });

        let commit_state = {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            owner
                .prepare_loaded_navigation_commit()
                .expect("background navigation commit state should prepare")
        };

        assert_eq!(commit_state.browser_context_id, "BID-parked");
        assert!(commit_state.runtime_frontend_enabled);
        assert_eq!(
            parked
                .background_target("TID-parked")
                .expect("background target")
                .target_url(),
            "about:blank",
            "preparing commit state should not mutate target identity"
        );
        let navigation_url = Url::parse("https://nav.example/path").unwrap();
        parked
            .background_target_mut("TID-parked")
            .expect("background target")
            .set_target_secure_context_type("InsecureScheme".to_owned());
        let main_document_commit = RendererMainDocumentCommit {
            frame_id: "TID-parked".to_owned(),
            loader_id: "LOADER-nav".to_owned(),
            url: navigation_url.to_string(),
            unreachable_url: None,
            security_origin: "https://nav.example".to_owned(),
            secure_context_type: "Secure".to_owned(),
            timestamp: 0.0,
        };
        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            owner
                .commit_loaded_navigation_target_identity(&main_document_commit, &navigation_url)
                .expect("background navigation identity should commit")
        };
        assert_eq!(
            parked
                .background_target("TID-parked")
                .expect("background target")
                .target_url(),
            "https://nav.example/path"
        );
        assert_eq!(
            parked
                .background_target("TID-parked")
                .expect("background target")
                .target_identity()
                .secure_context_type(),
            "Secure"
        );
        assert_eq!(commit_state.fetch_subresource_config, (true, None));
    }

    #[test]
    fn target_session_owner_mut_clears_background_navigation_history_update() {
        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-parked".to_owned(),
                Some("SID-parked".to_owned()),
                "about:blank".to_owned(),
            ));
        parked.mutate_parked_target_owner_state("TID-parked", |owner_state| {
            owner_state.record_loaded_page_navigation_history((
                "https://old.example/".to_owned(),
                "old".to_owned(),
            ));
            owner_state.mark_next_navigation_history_replace_current();
        });
        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
            };
            owner
                .clear_pending_navigation_history_update()
                .expect("background history update should clear");
        }
        parked.mutate_parked_target_owner_state("TID-parked", |owner_state| {
            owner_state.record_loaded_page_navigation_history((
                "https://new.example/".to_owned(),
                "new".to_owned(),
            ));
        });
        let (_, entries) = parked.mutate_parked_target_owner_state("TID-parked", |owner_state| {
            owner_state.navigation_history_snapshot(None)
        });
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
        let mut parked = BrowserContext::new("BID-parked".to_owned());
        parked
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-parked".to_owned(),
                Some("SID-parked".to_owned()),
                "about:blank".to_owned(),
            ));
        let initial_attachment_id = parked
            .background_target("TID-parked")
            .expect("background target")
            .page_attachment_id();

        {
            let mut owner = TargetSessionOwnerMut::BackgroundTarget {
                browser_context: &mut parked,
                target_id: "TID-parked".to_owned(),
                session_id: None,
                is_auxiliary_target_session: false,
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

        let target = parked
            .background_target("TID-parked")
            .expect("background target");
        assert!(target.has_loaded_page());
        assert!(
            target.page_attachment_id().is_some()
                && target.page_attachment_id() != initial_attachment_id
        );
        let (_, entries) = parked.mutate_parked_target_owner_state("TID-parked", |owner_state| {
            owner_state.navigation_history_snapshot(None)
        });
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "background commit");
    }

    #[test]
    fn target_page_residence_identity_rejects_context_target_and_attachment_collisions() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-page-residence".to_owned());
        browser_context.set_active_target_id("TID-page-residence");
        browser_context.attach_active_session("SID-page-residence");
        conn.browser_context = Some(browser_context);
        conn.runtime_session_owner_slot_mut(Some("SID-page-residence"))
            .expect("active target runtime slot")
            .set_page_attachment_id_for_test(41);

        let current = conn
            .target_page_residence_identity_for_session(Some("SID-page-residence"))
            .expect("active target should expose its Page residence identity");
        assert!(conn.target_page_residence_identity_is_current_for_session(
            Some("SID-page-residence"),
            &current,
        ));

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
                !conn.target_page_residence_identity_is_current_for_session(
                    Some("SID-page-residence"),
                    &stale,
                ),
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
        conn.browser_context = Some(browser_context);

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
            conn.pending_target_page_residence_identity_for_session(Some("SID-pending-residence"))
                .is_some(),
            "the future Page attachment should remain explicitly addressable"
        );
    }

    #[test]
    fn implicit_active_route_freezes_the_concrete_target_in_page_residence() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-implicit-page-residence".to_owned());
        browser_context.set_active_target_id("TID-original");
        conn.browser_context = Some(browser_context);
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
            !conn.target_page_residence_identity_is_current_for_session(None, &original),
            "an implicit route must not let the old Page identity follow a new active target"
        );
    }

    #[test]
    fn connection_target_owner_reference_reads_and_mutates_active_and_parked_state() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-background".to_owned(),
                Some("SID-background".to_owned()),
                "about:blank".to_owned(),
            ));
        browser_context
            .devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
        browser_context.mutate_parked_page_session_state("TID-background", |state| {
            state
                .devtools_session_state
                .runtime_session_state
                .inspector_enabled = true;
        });
        conn.browser_context = Some(browser_context);

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
            browser_context.assign_auxiliary_session_to_target(
                "TID-page-events",
                "SID-page-enabled".to_owned(),
            )
        );
        assert!(browser_context.assign_auxiliary_session_to_target(
            "TID-page-events",
            "SID-lifecycle-only".to_owned(),
        ));
        conn.browser_context = Some(browser_context);

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
        .expect("Page-enabled auxiliary session should be mutable");
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
        conn.browser_context = Some(browser_context);

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
    fn connection_runtime_slot_reference_reads_and_mutates_active_background_and_auxiliary_slots() {
        let mut conn = CdpConnection::default();
        let mut active = BrowserContext::new("BID-active".to_owned());
        active.set_active_target_id("TID-active".to_owned());
        active.set_target_url("https://active.example/".to_owned());
        active.attach_active_session("SID-active".to_owned());
        active
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-background".to_owned(),
                Some("SID-background".to_owned()),
                "https://background.example/".to_owned(),
            ));
        active.mutate_parked_page_session_state("TID-background", |state| {
            state.devtools_session_state.page_session_state.log_enabled = true;
        });
        assert!(
            active.assign_auxiliary_session_to_target(
                "TID-background",
                "SID-aux-background".to_owned()
            )
        );

        let mut inactive = BrowserContext::new("BID-inactive".to_owned());
        inactive.set_active_target_id("TID-inactive".to_owned());
        inactive.set_target_url("https://inactive.example/".to_owned());
        inactive
            .auxiliary_devtools_session_states
            .entry("SID-aux-inactive".to_owned())
            .or_default()
            .console_output_session_state
            .console_enabled = true;
        assert!(
            inactive
                .assign_auxiliary_session_to_target("TID-inactive", "SID-aux-inactive".to_owned())
        );

        conn.browser_context = Some(active);
        conn.inactive_browser_contexts.push(inactive);

        conn.runtime_session_owner_slot_mut(Some("SID-active"))
            .expect("active runtime slot should be mutable")
            .set_page_attachment_id_for_test(11);
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot should be mutable")
            .set_page_attachment_id_for_test(22);
        conn.runtime_session_owner_slot_mut(Some("SID-aux-inactive"))
            .expect("inactive auxiliary runtime slot should be mutable")
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
            conn.runtime_session_owner_slot(Some("SID-aux-background"))
                .expect("background auxiliary runtime slot should be readable")
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(22)
        );
        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-aux-inactive"))
                .expect("inactive auxiliary runtime slot should be readable")
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
            conn.runtime_session_owner_primary_session_id(Some("SID-aux-background"))
                .as_deref(),
            Some("SID-background")
        );
        assert_eq!(
            conn.runtime_session_owner_target_url(Some("SID-background"))
                .as_deref(),
            Some("https://background.example/")
        );
        assert_eq!(
            conn.runtime_session_owner_target_url(Some("SID-aux-inactive"))
                .as_deref(),
            Some("https://inactive.example/")
        );
        assert_eq!(
            conn.target_owner_identity_for_session(Some("SID-aux-background")),
            Some(("BID-active".to_owned(), Some("TID-background".to_owned())))
        );
        assert_eq!(
            conn.target_owner_identity_for_session(Some("SID-aux-inactive")),
            Some(("BID-inactive".to_owned(), Some("TID-inactive".to_owned())))
        );
        assert!(
            conn.target_page_session_state_for_session(Some("SID-background"))
                .expect("background page session state should be readable")
                .log_enabled
        );
        assert!(
            conn.target_devtools_session_state_for_session(Some("SID-aux-inactive"))
                .expect("inactive auxiliary DevTools session state should be readable")
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
        browser_context
            .background_targets
            .push(crate::conn::BackgroundTarget::with_url(
                "TID-background-dialog".to_owned(),
                Some("SID-background-dialog".to_owned()),
                "https://background.example/dialog".to_owned(),
            ));
        assert!(
            browser_context
                .parked_page_session_state("TID-background-dialog")
                .is_none(),
            "a background target with default protocol settings should not allocate parked overrides"
        );
        conn.browser_context = Some(browser_context);

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
                .parked_page_session_state("TID-background-dialog")
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
