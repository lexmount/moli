use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsError, DevToolsErrorKind, DevToolsFrameId,
    DevToolsGetNavigationHistoryCommand, DevToolsGetNavigationHistoryResult,
    DevToolsHistoryTraversalDestination, DevToolsLoaderId, DevToolsNavigateCommand,
    DevToolsNavigateResult, DevToolsNavigationHistoryEntry, DevToolsNavigationId,
    DevToolsNavigationWait, DevToolsProtocol, DevToolsReloadCommand, DevToolsTargetId,
    DevToolsTraverseHistoryCommand, DevToolsTraverseHistoryResult, SameDocumentNavigationEvent,
    webdriver_bidi_navigation_id_from_loader_id,
};
use chromiumoxide_cdp::cdp::browser_protocol::page::{
    NavigateParams, NavigateToHistoryEntryParams, ReloadParams,
};
use moli_core::page::{
    ChildFrameDocumentOpenedSnapshot, CompletedPageCommand, PendingPageCommand,
    SameDocumentHistoryUpdate,
};
use moli_core::runtime::NavigationEngine;
use moli_fetch::NET_ERR_ABORTED_ERROR_TEXT;
use moli_url_policy::{LocalFileNavigationAccess, route_navigation_url};
use serde_json::{Value, json};
use url::Url;

use crate::conn::{
    BackgroundNavigationLoadJob, BackgroundProtocolEvent, BrowserContext, CapturedBody,
    CdpConnection, CdpSessionRoute, Cmd, CommandDispatchContext, DocumentNavigationToken,
    FetchRequestStage, NavigationDispatchState, NavigationLoadOutcome, NavigationRequestLoadPolicy,
    NavigationResultProjection, NavigationSourceDocumentSecurityContext, PendingFetchNavigation,
    ResponseStageUrlMatchPolicy, monotonic_timestamp_seconds,
};
use moli_cookie_jar::{NetworkCookieRequestContext, StoredCookieQueryReport};

use crate::domains::{
    activity,
    command_output::{CommandOutputBuffer, CommandOutputPlan},
    fetch, network,
};

use super::{
    LOADER_ID, PageCommandTaskStep,
    child_frame_activity::{
        PagePreparedChildFrameActivity, PagePreparedChildFrameDocumentActivity,
        PagePreparedChildFrameTreeEvent,
    },
    child_frame_security_identity,
    lifecycle::{
        NavigationStartInitiator, emit_child_frame_document_open_completed_background_events,
        emit_child_frame_document_opened_background_events, emit_child_frame_lifecycle_terminal,
        emit_child_frame_navigation_commit, emit_navigation_started_background_events,
    },
    navigation_commit::{commit_download_navigation_async, commit_loaded_navigation_async},
};

pub(super) struct PendingNavigateLoadCommand {
    prefix_events: Vec<BackgroundProtocolEvent>,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    job: BackgroundNavigationLoadJob,
}

pub(super) struct CompletedNavigateLoadCommand {
    prefix_events: Vec<BackgroundProtocolEvent>,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    engine: NavigationEngine,
    navigation: Result<NavigationLoadOutcome, String>,
}

pub(super) struct PendingChildFrameNavigateCommand {
    prefix_events: Vec<BackgroundProtocolEvent>,
    pending: PendingPageCommand,
    activity_binding: crate::conn::TargetRootDocumentProtocolAttachmentIdentity,
    url: String,
    wait: DevToolsNavigationWait,
    result_payload: Value,
}

pub(super) struct PendingSameDocumentNavigateCommand {
    pending: PendingPageCommand,
    result_payload: Value,
}

pub(super) struct CompletedChildFrameNavigateCommand {
    prefix_events: Vec<BackgroundProtocolEvent>,
    completed: Result<CompletedPageCommand, String>,
    activity_binding: crate::conn::TargetRootDocumentProtocolAttachmentIdentity,
    url: String,
    wait: DevToolsNavigationWait,
    result_payload: Value,
}

pub(super) struct CompletedSameDocumentNavigateCommand {
    completed: Result<CompletedPageCommand, String>,
    result_payload: Value,
}

pub(super) struct PendingContinueNavigationWithoutRequestPauseCommand {
    prefix_events: Vec<BackgroundProtocolEvent>,
    pending: PendingFetchNavigation,
}

pub(super) struct CompletedContinueNavigationWithoutRequestPauseCommand {
    prefix_events: Vec<BackgroundProtocolEvent>,
    pending: PendingFetchNavigation,
}

struct HistoryTraversalUrlFallback {
    entry_id: i32,
    url: String,
    result_projection: NavigationResultProjection,
    reloaded_after_crash_session_ids: Vec<Option<String>>,
    allow_background_navigation: bool,
}

pub(super) struct PendingSameDocumentHistoryTraversalCommand {
    pending: PendingPageCommand,
    fallback: HistoryTraversalUrlFallback,
}

pub(super) struct CompletedSameDocumentHistoryTraversalCommand {
    completed: Result<CompletedPageCommand, String>,
    fallback: HistoryTraversalUrlFallback,
}

impl CompletedChildFrameNavigateCommand {
    pub(super) fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::renderer_output_predecessor)
    }
}

impl CompletedSameDocumentNavigateCommand {
    pub(super) fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::renderer_output_predecessor)
    }
}

impl CompletedSameDocumentHistoryTraversalCommand {
    pub(super) fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.completed
            .as_ref()
            .ok()
            .and_then(CompletedPageCommand::renderer_output_predecessor)
    }
}

impl PendingNavigateLoadCommand {
    pub(super) async fn wait(self) -> CompletedNavigateLoadCommand {
        let (engine, navigation, _early_result_sent) = self.job.run(None).await;
        CompletedNavigateLoadCommand {
            prefix_events: self.prefix_events,
            token: self.token,
            state: self.state,
            engine,
            navigation,
        }
    }
}

impl PendingChildFrameNavigateCommand {
    pub(super) async fn wait(self) -> CompletedChildFrameNavigateCommand {
        CompletedChildFrameNavigateCommand {
            prefix_events: self.prefix_events,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
            activity_binding: self.activity_binding,
            url: self.url,
            wait: self.wait,
            result_payload: self.result_payload,
        }
    }
}

impl PendingSameDocumentNavigateCommand {
    pub(super) async fn wait(self) -> CompletedSameDocumentNavigateCommand {
        CompletedSameDocumentNavigateCommand {
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
            result_payload: self.result_payload,
        }
    }
}

impl PendingContinueNavigationWithoutRequestPauseCommand {
    pub(super) async fn wait(self) -> CompletedContinueNavigationWithoutRequestPauseCommand {
        CompletedContinueNavigationWithoutRequestPauseCommand {
            prefix_events: self.prefix_events,
            pending: self.pending,
        }
    }
}

impl PendingSameDocumentHistoryTraversalCommand {
    pub(super) async fn wait(self) -> CompletedSameDocumentHistoryTraversalCommand {
        CompletedSameDocumentHistoryTraversalCommand {
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
            fallback: self.fallback,
        }
    }
}

pub(super) enum NavigateCommandStart {
    CompletePlan(CommandOutputPlan),
    CompleteImmediate(CommandOutputPlan),
    PendingLoad(Box<PendingNavigateLoadCommand>),
    PendingChildFrame(Box<PendingChildFrameNavigateCommand>),
    PendingSameDocument(Box<PendingSameDocumentNavigateCommand>),
    PendingContinueWithoutRequestPause(Box<PendingContinueNavigationWithoutRequestPauseCommand>),
}

const CHILD_FRAME_NAVIGATION_LOAD_GATE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30);

#[derive(Debug, Clone)]
struct DirectNavigationResult {
    protocol: DevToolsProtocol,
    result_kind: DevToolsNavigationCommandResultKind,
    url: String,
    frame_id: Option<DevToolsFrameId>,
    loader_id: Option<DevToolsLoaderId>,
    navigation_id: Option<DevToolsNavigationId>,
    error_text: Option<String>,
    is_download: Option<bool>,
}

impl DirectNavigationResult {
    fn navigate(
        protocol: DevToolsProtocol,
        frame_id: Option<&str>,
        loader_id: Option<&str>,
        url: impl Into<String>,
    ) -> Self {
        let navigation_id = if protocol == DevToolsProtocol::WebDriverBidi {
            loader_id.map(webdriver_bidi_navigation_id_from_loader_id)
        } else {
            None
        };
        Self {
            protocol,
            result_kind: DevToolsNavigationCommandResultKind::Navigate,
            url: url.into(),
            frame_id: if protocol == DevToolsProtocol::WebDriverBidi {
                None
            } else {
                frame_id.map(DevToolsFrameId::from)
            },
            loader_id: if protocol == DevToolsProtocol::WebDriverBidi {
                None
            } else {
                loader_id.map(DevToolsLoaderId::from)
            },
            navigation_id,
            error_text: None,
            is_download: None,
        }
    }

    fn empty() -> Self {
        Self {
            protocol: DevToolsProtocol::Cdp,
            result_kind: DevToolsNavigationCommandResultKind::Empty,
            url: String::new(),
            frame_id: None,
            loader_id: None,
            navigation_id: None,
            error_text: None,
            is_download: None,
        }
    }

    fn traverse_history(protocol: DevToolsProtocol, same_document: bool) -> Self {
        Self {
            protocol,
            result_kind: DevToolsNavigationCommandResultKind::TraverseHistory { same_document },
            url: String::new(),
            frame_id: None,
            loader_id: None,
            navigation_id: None,
            error_text: None,
            is_download: None,
        }
    }

    fn set_history_traversal_same_document(&mut self, same_document: bool) {
        if matches!(
            self.result_kind,
            DevToolsNavigationCommandResultKind::TraverseHistory { .. }
        ) {
            self.result_kind =
                DevToolsNavigationCommandResultKind::TraverseHistory { same_document };
        }
    }

    fn set_navigation_identity(&mut self, frame_id: &str, loader_id: &str) {
        if self.protocol != DevToolsProtocol::WebDriverBidi {
            self.frame_id = Some(DevToolsFrameId::from(frame_id));
            self.loader_id = Some(DevToolsLoaderId::from(loader_id));
        }
        self.navigation_id = (self.protocol == DevToolsProtocol::WebDriverBidi)
            .then(|| webdriver_bidi_navigation_id_from_loader_id(loader_id));
    }

    fn set_url(&mut self, url: String) {
        self.url = url;
    }

    fn set_cdp_navigation_aborted(&mut self, frame_id: &str) {
        self.frame_id = Some(DevToolsFrameId::from(frame_id));
        self.loader_id = None;
        self.error_text = Some(NET_ERR_ABORTED_ERROR_TEXT.to_owned());
        self.is_download = Some(false);
    }

    fn ensure_navigation_id_from_loader(&mut self, loader_id: Option<String>) {
        if self.navigation_id.is_none() {
            self.navigation_id = loader_id
                .as_deref()
                .map(webdriver_bidi_navigation_id_from_loader_id);
        }
    }

    fn into_result(self) -> DevToolsCommandResult {
        match self.result_kind {
            DevToolsNavigationCommandResultKind::Empty => DevToolsCommandResult::Empty,
            DevToolsNavigationCommandResultKind::TraverseHistory { same_document } => {
                DevToolsCommandResult::TraverseHistory(DevToolsTraverseHistoryResult {
                    same_document,
                })
            }
            DevToolsNavigationCommandResultKind::Navigate => {
                DevToolsCommandResult::Navigate(DevToolsNavigateResult {
                    navigation_id: self.navigation_id,
                    frame_id: self.frame_id,
                    loader_id: self.loader_id,
                    url: self.url,
                    error_text: self.error_text,
                    is_download: self.is_download,
                })
            }
        }
    }
}

fn send_background_navigation_started(
    conn: &mut CdpConnection,
    token: DocumentNavigationToken,
    owner_session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
    url: &str,
    initiator: NavigationStartInitiator,
) {
    for session_id in conn.page_event_session_ids_for_session_owner(owner_session_id) {
        let mut events = Vec::new();
        emit_navigation_started_background_events(
            &mut events,
            session_id.as_deref(),
            frame_id,
            loader_id,
            url,
            initiator,
        );
        for event in events {
            conn.send_navigation_background_protocol_event(token.clone(), event);
        }
    }
}

fn emit_navigation_started_for_session_owner(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner_session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
    url: &str,
    initiator: NavigationStartInitiator,
) {
    for session_id in conn.page_event_session_ids_for_session_owner(owner_session_id) {
        emit_navigation_started_background_events(
            out,
            session_id.as_deref(),
            frame_id,
            loader_id,
            url,
            initiator,
        );
    }
}

pub(crate) struct MaterializedNavigationCompletion {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    navigation: network::MaterializedNavigationLoadOutcome,
    engine: Option<NavigationEngine>,
}

impl MaterializedNavigationCompletion {
    pub(crate) fn new(
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        navigation: network::MaterializedNavigationLoadOutcome,
    ) -> Self {
        Self {
            token,
            state,
            navigation,
            engine: None,
        }
    }

    pub(crate) fn with_navigation_engine(mut self, engine: NavigationEngine) -> Self {
        self.engine = Some(engine);
        self
    }

    pub(crate) fn is_current_for_connection(&self, conn: &CdpConnection) -> bool {
        conn.accepts_pending_document_navigation_for_session_owner(
            self.state.navigate_session_id.as_deref(),
            &self.token,
        )
    }

    pub(crate) fn requested_url(&self) -> &str {
        self.state.requested_url.as_str()
    }

    pub(crate) fn navigate_id(&self) -> Option<u64> {
        self.state.navigate_id
    }

    pub(crate) fn navigate_session_id(&self) -> Option<&str> {
        self.state.navigate_session_id.as_deref()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        DocumentNavigationToken,
        NavigationDispatchState,
        network::MaterializedNavigationLoadOutcome,
        Option<NavigationEngine>,
    ) {
        (self.token, self.state, self.navigation, self.engine)
    }
}

pub struct BackgroundMainDocumentBodyCompletion {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    none_session_owner_route: Option<CdpSessionRoute>,
    body: Result<CapturedBody, String>,
    synthetic: bool,
    body_progress_source: network::MainDocumentBodyProgressSource,
    final_url: Url,
    response_headers: Vec<(String, String)>,
    response_from_cache: bool,
}

impl BackgroundMainDocumentBodyCompletion {
    pub(crate) fn new(
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        none_session_owner_route: Option<CdpSessionRoute>,
        body: Result<CapturedBody, String>,
        synthetic: bool,
        body_progress_source: network::MainDocumentBodyProgressSource,
        final_url: Url,
        response_headers: Vec<(String, String)>,
        response_from_cache: bool,
    ) -> Self {
        Self {
            token,
            state,
            none_session_owner_route,
            body,
            synthetic,
            body_progress_source,
            final_url,
            response_headers,
            response_from_cache,
        }
    }

    pub(crate) fn navigate_session_id(&self) -> Option<&str> {
        self.state.navigate_session_id.as_deref()
    }

    pub(crate) fn none_session_owner_route(&self) -> Option<CdpSessionRoute> {
        self.none_session_owner_route.clone()
    }

    pub(crate) fn is_current_for_connection(&self, conn: &CdpConnection) -> bool {
        conn.accepts_document_body_completion_for_session_owner(
            self.state.navigate_session_id.as_deref(),
            &self.token,
        )
    }

    pub(crate) fn record_if_current(self, conn: &mut CdpConnection) {
        if !self.is_current_for_connection(conn) {
            return;
        }
        match self.body {
            Ok(body) => {
                let _ = conn.record_main_document_resource_body_for_session_owner(
                    self.state.navigate_session_id.as_deref(),
                    self.state.frame_id.clone(),
                    self.state.loader_id.clone(),
                    self.final_url,
                    self.response_headers,
                    self.response_from_cache,
                    body.clone(),
                );
                let encoded_data_length = body.len();
                network::record_completed_main_document_response_body(
                    conn,
                    &self.state,
                    self.synthetic,
                    &body,
                );
                self.body_progress_source
                    .emit_body_finished(encoded_data_length);
            }
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "background main document body capture failed after lifecycle commit"
                );
                network::record_failed_main_document_response_body(conn, &self.state, error);
            }
        }
    }
}

pub enum BackgroundNavigationCompletion {
    Lifecycle(Box<BackgroundNavigationLifecycleCompletion>),
    MainDocumentBody(Box<BackgroundMainDocumentBodyCompletion>),
}

impl BackgroundNavigationCompletion {
    pub fn requested_url(&self) -> &str {
        match self {
            Self::Lifecycle(completion) => completion.state.requested_url.as_str(),
            Self::MainDocumentBody(completion) => completion.state.requested_url.as_str(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Lifecycle(_) => "lifecycle",
            Self::MainDocumentBody(_) => "main_document_body",
        }
    }
}

pub struct BackgroundNavigationLifecycleCompletion {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    none_session_owner_route: Option<CdpSessionRoute>,
    engine: NavigationEngine,
    navigation: Result<NavigationLoadOutcome, String>,
    ready_at: std::time::Instant,
}

impl BackgroundNavigationLifecycleCompletion {
    pub(crate) fn new(
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        none_session_owner_route: Option<CdpSessionRoute>,
        engine: NavigationEngine,
        navigation: Result<NavigationLoadOutcome, String>,
    ) -> Self {
        Self {
            token,
            state,
            none_session_owner_route,
            engine,
            navigation,
            ready_at: std::time::Instant::now(),
        }
    }

    pub(crate) fn navigate_session_id(&self) -> Option<&str> {
        self.state.navigate_session_id.as_deref()
    }

    pub(crate) fn navigation_token(&self) -> &DocumentNavigationToken {
        &self.token
    }

    pub(crate) fn none_session_owner_route(&self) -> Option<CdpSessionRoute> {
        self.none_session_owner_route.clone()
    }

    pub(crate) fn is_current_for_connection(&self, conn: &CdpConnection) -> bool {
        conn.accepts_pending_document_navigation_for_session_owner(
            self.state.navigate_session_id.as_deref(),
            &self.token,
        )
    }

    pub(crate) fn requested_url(&self) -> &str {
        self.state.requested_url.as_str()
    }

    pub(crate) fn ready_elapsed_ms(&self) -> u128 {
        self.ready_at.elapsed().as_millis()
    }

    /// Materialize the background navigation outcome into a queued
    /// `MaterializedNavigationCompletion`. `retain_engine` should be `false`
    /// when the completion's token is no longer current — stale completions
    /// still flow into the materialized queue so the drain can emit a
    /// terminal abort response for any outstanding `navigate_id`, but the
    /// stale background engine must not keep a renderer owner alive.
    pub(crate) fn materialize_with_engine_retention(
        self,
        conn: &mut CdpConnection,
        retain_engine: bool,
    ) -> MaterializedNavigationCompletion {
        let Self {
            token,
            state,
            none_session_owner_route: _,
            engine,
            navigation,
            ready_at: _,
        } = self;
        let should_retain_engine = retain_engine
            && matches!(
                navigation,
                Ok(NavigationLoadOutcome::ResponseCommitReady(_)
                    | NavigationLoadOutcome::Loaded(_))
            );
        let navigation = network::materialize_navigation_load_result(conn, &state, navigation);
        let completion = MaterializedNavigationCompletion::new(token, state, navigation);
        if should_retain_engine {
            completion.with_navigation_engine(engine)
        } else {
            completion
        }
    }
}

impl BackgroundNavigationCompletion {
    pub(crate) fn new(
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        none_session_owner_route: Option<CdpSessionRoute>,
        engine: NavigationEngine,
        navigation: Result<NavigationLoadOutcome, String>,
    ) -> Self {
        Self::Lifecycle(Box::new(BackgroundNavigationLifecycleCompletion::new(
            token,
            state,
            none_session_owner_route,
            engine,
            navigation,
        )))
    }

    pub(crate) fn main_document_body(
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        none_session_owner_route: Option<CdpSessionRoute>,
        body: Result<CapturedBody, String>,
        synthetic: bool,
        body_progress_source: network::MainDocumentBodyProgressSource,
        final_url: Url,
        response_headers: Vec<(String, String)>,
        response_from_cache: bool,
    ) -> Self {
        Self::MainDocumentBody(Box::new(BackgroundMainDocumentBodyCompletion::new(
            token,
            state,
            none_session_owner_route,
            body,
            synthetic,
            body_progress_source,
            final_url,
            response_headers,
            response_from_cache,
        )))
    }
}

pub(crate) fn current_navigation_initiator_url(browser_context: &BrowserContext) -> Option<Url> {
    if let Some(loaded_page) = browser_context.loaded_page() {
        let url = loaded_page.final_url().clone();
        if url.host_str().is_some() {
            return Some(url);
        }
    }

    let url = Url::parse(browser_context.target_url()).ok()?;
    url.host_str().is_some().then_some(url)
}

pub(crate) fn navigation_cookie_access_report(
    conn: &CdpConnection,
    request_url: &Url,
    method: &str,
    previous_request_url: Option<&Url>,
    _request_load_policy: NavigationRequestLoadPolicy,
    initiator_url_override: Option<&Url>,
) -> Option<StoredCookieQueryReport> {
    let browser_context = conn.browser_context.as_ref()?;
    // Once a navigation commits, the target identity and loaded page already point
    // at the destination document. Redirect diagnostics still need the
    // pre-commit initiator, so callers may override it with a snapshot taken
    // before mutating browser context state.
    let initiator_url = initiator_url_override
        .cloned()
        .or_else(|| current_navigation_initiator_url(browser_context));
    let request_context = network::navigation_cookie_request_context(
        request_url,
        method,
        previous_request_url,
        initiator_url.as_ref(),
    );
    navigation_cookie_access_report_for_context(conn, request_url, request_context)
}

fn navigation_cookie_access_report_for_context(
    conn: &CdpConnection,
    request_url: &Url,
    request_context: NetworkCookieRequestContext,
) -> Option<StoredCookieQueryReport> {
    let browser_context = conn.browser_context.as_ref()?;
    browser_context.observe_request_cookie_access_report(request_url, request_context)
}

pub(super) fn try_start_navigate_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: NavigateParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let reloaded_after_crash_session_ids =
        reloaded_after_crash_session_ids_for_session_owner(conn, cmd.session_id);
    let frame_id = params
        .frame_id
        .as_ref()
        .map(|frame_id| frame_id.as_ref().to_owned())
        .or_else(|| {
            conn.target_owner_identity_for_session(cmd.session_id)
                .and_then(|(_, target_id)| target_id)
        })
        .unwrap_or_else(|| "FRAME-0".to_owned());
    let shared_command = build_cdp_navigate_command(
        cmd,
        Some(frame_id.as_str()),
        &params.url,
        params.referrer.as_deref(),
    );
    let result_payload = cdp_navigate_result_payload(
        None,
        Some(frame_id.as_str()),
        Some(LOADER_ID),
        &shared_command.url,
    );
    let result_projection =
        NavigationResultProjection::new(shared_command.context.protocol, result_payload);
    start_devtools_page_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::Navigate(shared_command),
        DevToolsNavigationStartOptions {
            result_projection,
            reloaded_after_crash_session_ids,
            allow_background_navigation: true,
        },
    )
}

fn build_cdp_navigate_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    url: &str,
    referrer: Option<&str>,
) -> DevToolsNavigateCommand {
    DevToolsNavigateCommand {
        context: cmd.devtools_command_context(target_id, Option::<&str>::None),
        url: url.to_owned(),
        referrer: referrer.map(str::to_owned),
        wait: DevToolsNavigationWait::DocumentInstalled,
    }
}

fn cdp_navigate_result_payload(
    navigation_id: Option<&str>,
    frame_id: Option<&str>,
    loader_id: Option<&str>,
    url: &str,
) -> Value {
    CommandOutputPlan::devtools_result_payload(DevToolsCommandResult::Navigate(
        DevToolsNavigateResult {
            navigation_id: navigation_id.map(Into::into),
            frame_id: frame_id.map(DevToolsFrameId::from),
            loader_id: loader_id.map(DevToolsLoaderId::from),
            url: url.to_owned(),
            error_text: None,
            is_download: None,
        },
    ))
}

fn webdriver_bidi_navigate_result_payload(loader_id: Option<&str>, url: &str) -> Value {
    json!({
        "navigation": loader_id
            .map(webdriver_bidi_navigation_id_from_loader_id)
            .map(|navigation_id| Value::String(navigation_id.into_string()))
            .unwrap_or(Value::Null),
        "url": url,
    })
}

fn protocol_neutral_navigate_result_payload(
    protocol: DevToolsProtocol,
    frame_id: Option<&str>,
    loader_id: Option<&str>,
    url: &str,
) -> Value {
    if protocol == DevToolsProtocol::WebDriverBidi {
        webdriver_bidi_navigate_result_payload(loader_id, url)
    } else {
        cdp_navigate_result_payload(None, frame_id, loader_id, url)
    }
}

fn update_navigation_result_payload_identity(
    result_projection: &mut NavigationResultProjection,
    frame_id: &str,
    loader_id: &str,
) {
    let protocol = result_projection.protocol();
    let Some(payload) = result_projection.payload_mut().as_object_mut() else {
        return;
    };
    match protocol {
        DevToolsProtocol::Cdp | DevToolsProtocol::WebDriverClassic => {
            if payload.contains_key("frameId") {
                payload.insert("frameId".to_owned(), json!(frame_id));
                payload.insert("loaderId".to_owned(), json!(loader_id));
            }
        }
        DevToolsProtocol::WebDriverBidi => {
            if payload.contains_key("navigation") {
                payload.insert(
                    "navigation".to_owned(),
                    json!(webdriver_bidi_navigation_id_from_loader_id(loader_id).into_string()),
                );
            }
        }
    }
}

struct DevToolsNavigationStartOptions {
    result_projection: NavigationResultProjection,
    reloaded_after_crash_session_ids: Vec<Option<String>>,
    allow_background_navigation: bool,
}

fn start_devtools_page_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCommand,
    options: DevToolsNavigationStartOptions,
) -> PageCommandTaskStep {
    match command {
        DevToolsCommand::Navigate(command) => {
            let start = start_devtools_navigate_command(
                conn,
                command_id,
                command_session_id,
                &command,
                options.result_projection,
                options.allow_background_navigation,
            );
            finish_started_navigation_command_for_parts(
                conn,
                command_id,
                command_session_id,
                start,
                &options.reloaded_after_crash_session_ids,
            )
        }
        DevToolsCommand::Reload(command) => {
            start_devtools_reload_command(conn, command_id, command_session_id, &command, options)
        }
        DevToolsCommand::TraverseHistory(command) => start_devtools_traverse_history_command(
            conn,
            command_id,
            command_session_id,
            &command,
            options,
        ),
        _ => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "UnsupportedDevToolsCommand",
        )),
    }
}

pub(crate) async fn execute_devtools_navigation_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
    background_command_id: Option<u64>,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    let wait = devtools_navigation_wait(&command);
    let route = match devtools_navigation_target_route(conn, &command) {
        Ok(route) => route,
        Err(error) => return (Err(error), Vec::new(), None),
    };
    let (mut step, mut direct_result) = start_protocol_neutral_navigation_command(
        conn,
        route.clone(),
        command,
        background_command_id,
    );
    let mut command_context = crate::conn::CommandDispatchContext::default();
    loop {
        match step {
            PageCommandTaskStep::Complete(plan) => {
                let (status, events) = plan.into_command_status_and_background_events();
                if let Some(Err(error)) = status {
                    direct_result = Err(error);
                }
                if wait == DevToolsNavigationWait::None
                    && let Ok(direct_result) = direct_result.as_mut()
                {
                    fill_navigation_id_from_current_loader_for_route(conn, &route, direct_result);
                }
                let mut ordered_events = command_context.take_protocol_events_before_events(events);
                ordered_events.extend(command_context.take_post_response_events());
                return (
                    direct_result.map(DirectNavigationResult::into_result),
                    ordered_events,
                    command_context.take_renderer_output_predecessor(),
                );
            }
            PageCommandTaskStep::Pending(pending) => {
                let completed = pending.wait().await;
                let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
                if let Ok(result) = direct_result.as_mut()
                    && let Err(error) = direct_navigation_result_from_completed(
                        route_scope.conn_mut(),
                        &completed,
                        result,
                    )
                {
                    direct_result = Err(error);
                }
                step = super::complete_pending_page_command(
                    route_scope.conn_mut(),
                    completed,
                    &mut command_context,
                )
                .await;
            }
        }
    }
}

pub(crate) fn execute_devtools_get_navigation_history_command(
    conn: &mut CdpConnection,
    command: DevToolsGetNavigationHistoryCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target_id = command
        .context
        .target_id
        .as_ref()
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    let route = conn
        .target_session_route_for_target_id(target_id.as_str())
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    let snapshot = {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .target_session_owner_navigation_history_snapshot(None)
    };
    let Some((current_index, entries)) = snapshot else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    };
    Ok(DevToolsCommandResult::GetNavigationHistory(
        DevToolsGetNavigationHistoryResult {
            current_index,
            entries: entries
                .into_iter()
                .map(|entry| DevToolsNavigationHistoryEntry {
                    id: entry.id,
                    url: entry.url,
                    user_typed_url: entry.user_typed_url,
                    title: entry.title,
                    transition_type: entry.transition_type,
                })
                .collect(),
        },
    ))
}

fn devtools_navigation_target_route(
    conn: &CdpConnection,
    command: &DevToolsCommand,
) -> Result<CdpSessionRoute, DevToolsError> {
    let target_id = match command {
        DevToolsCommand::Navigate(command) => command.context.target_id.as_ref(),
        DevToolsCommand::Reload(command) => command.context.target_id.as_ref(),
        DevToolsCommand::TraverseHistory(command) => command.context.target_id.as_ref(),
        _ => None,
    }
    .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str()) {
        return Ok(route);
    }
    if matches!(command, DevToolsCommand::Navigate(_))
        && let Some(route) = conn.target_session_route_for_child_frame_id(target_id.as_str())
    {
        return Ok(route);
    }
    if matches!(command, DevToolsCommand::TraverseHistory(_))
        && conn.has_attached_child_frame_id(target_id.as_str())
    {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "ChildFrameContextNotSupportedForTraverseHistory",
        ));
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchTarget",
    ))
}

fn start_protocol_neutral_navigation_command(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
    command: DevToolsCommand,
    background_command_id: Option<u64>,
) -> (
    PageCommandTaskStep,
    Result<DirectNavigationResult, DevToolsError>,
) {
    let mut route_scope = conn.scoped_none_session_owner_route_override(route);
    let conn = route_scope.conn_mut();
    let wait = devtools_navigation_wait(&command);
    let result_url = match &command {
        DevToolsCommand::Navigate(command) => command.url.clone(),
        DevToolsCommand::Reload(_) => match conn.runtime_session_owner_target_url(None) {
            Some(url) => url,
            None => {
                let error = DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "TargetNotLoaded");
                return (
                    PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                        error.clone(),
                    )),
                    Err(error),
                );
            }
        },
        DevToolsCommand::TraverseHistory(_) => String::new(),
        _ => String::new(),
    };
    let result_kind = DevToolsNavigationCommandResultKind::Navigate;
    let mut direct_result = match &command {
        DevToolsCommand::Navigate(command) => {
            let loader_id = if child_frame_navigation_target_id(conn, command).is_some() {
                None
            } else {
                Some(LOADER_ID)
            };
            DirectNavigationResult::navigate(
                command.context.protocol,
                command
                    .context
                    .target_id
                    .as_ref()
                    .map(|target_id| target_id.as_str()),
                loader_id,
                command.url.clone(),
            )
        }
        DevToolsCommand::Reload(command) => DirectNavigationResult::navigate(
            command.context.protocol,
            None,
            Some(LOADER_ID),
            result_url.clone(),
        ),
        DevToolsCommand::TraverseHistory(command) => {
            DirectNavigationResult::traverse_history(command.context.protocol, false)
        }
        _ => DirectNavigationResult {
            protocol: DevToolsProtocol::Cdp,
            result_kind,
            url: result_url.clone(),
            frame_id: None,
            loader_id: None,
            navigation_id: None,
            error_text: None,
            is_download: None,
        },
    };
    let reloaded_after_crash_session_ids =
        reloaded_after_crash_session_ids_for_session_owner(conn, None);
    let step = match command {
        DevToolsCommand::Navigate(command) => {
            if Url::parse(&command.url).is_err() {
                let error =
                    DevToolsError::new(DevToolsErrorKind::Internal, "Invalid navigation URL");
                return (
                    PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                        error.clone(),
                    )),
                    Err(error),
                );
            }
            let frame_id = command
                .context
                .target_id
                .as_ref()
                .map(|target_id| target_id.as_str());
            let result_payload = protocol_neutral_navigate_result_payload(
                command.context.protocol,
                frame_id,
                Some(LOADER_ID),
                &command.url,
            );
            let result_projection =
                NavigationResultProjection::new(command.context.protocol, result_payload);
            start_devtools_page_command(
                conn,
                background_command_id,
                None,
                DevToolsCommand::Navigate(command),
                DevToolsNavigationStartOptions {
                    result_projection,
                    reloaded_after_crash_session_ids,
                    allow_background_navigation: wait == DevToolsNavigationWait::None,
                },
            )
        }
        DevToolsCommand::Reload(mut command) => {
            let result_payload = protocol_neutral_navigate_result_payload(
                command.context.protocol,
                None,
                Some(LOADER_ID),
                &result_url,
            );
            let result_projection =
                NavigationResultProjection::new(command.context.protocol, result_payload);
            command.context.session_id = None;
            start_devtools_page_command(
                conn,
                background_command_id,
                None,
                DevToolsCommand::Reload(command),
                DevToolsNavigationStartOptions {
                    result_projection,
                    reloaded_after_crash_session_ids,
                    allow_background_navigation: wait == DevToolsNavigationWait::None,
                },
            )
        }
        DevToolsCommand::TraverseHistory(mut command) => {
            let result_protocol = command.context.protocol;
            command.context.session_id = None;
            match resolve_devtools_history_traversal_destination(conn, None, &command.destination) {
                Ok(ResolvedDevToolsHistoryTraversal::Noop) => {
                    return (
                        PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_result(
                            DevToolsCommandResult::Empty,
                        )),
                        Ok(DirectNavigationResult::empty()),
                    );
                }
                Ok(ResolvedDevToolsHistoryTraversal::Entry {
                    entry_id,
                    url,
                    same_document_delta,
                }) => {
                    direct_result
                        .set_history_traversal_same_document(same_document_delta.is_some());
                    direct_result.set_url(url.clone());
                    command.destination =
                        DevToolsHistoryTraversalDestination::Entry { entry_id, url };
                }
                Err(error) => {
                    return (
                        PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                            error.clone(),
                        )),
                        Err(error),
                    );
                }
            }
            start_devtools_page_command(
                conn,
                background_command_id,
                None,
                DevToolsCommand::TraverseHistory(command),
                DevToolsNavigationStartOptions {
                    result_projection: NavigationResultProjection::new(result_protocol, json!({})),
                    reloaded_after_crash_session_ids,
                    allow_background_navigation: wait == DevToolsNavigationWait::None,
                },
            )
        }
        _ => {
            let error =
                DevToolsError::new(DevToolsErrorKind::Unsupported, "UnsupportedDevToolsCommand");
            return (
                PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                    error.clone(),
                )),
                Err(error),
            );
        }
    };
    (step, Ok(direct_result))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevToolsNavigationCommandResultKind {
    Navigate,
    Empty,
    TraverseHistory { same_document: bool },
}

fn devtools_navigation_wait(command: &DevToolsCommand) -> DevToolsNavigationWait {
    match command {
        DevToolsCommand::Navigate(command) => command.wait,
        DevToolsCommand::Reload(command) => command.wait,
        DevToolsCommand::TraverseHistory(command) => command.wait,
        _ => DevToolsNavigationWait::Load,
    }
}

fn direct_navigation_result_from_completed(
    conn: &CdpConnection,
    completed: &super::CompletedPageCommandDispatch,
    result: &mut DirectNavigationResult,
) -> Result<(), DevToolsError> {
    match completed.kind.as_ref() {
        super::CompletedPageCommandKind::Navigate(completed) => {
            direct_navigation_result_from_completed_load(conn, completed, result)
        }
        super::CompletedPageCommandKind::ChildFrameNavigate(completed) => {
            direct_navigation_result_from_completed_child_frame(completed, result)
        }
        super::CompletedPageCommandKind::SameDocumentNavigate(completed) => {
            direct_navigation_result_from_completed_same_document(completed, result)
        }
        super::CompletedPageCommandKind::ContinueNavigationWithoutRequestPause(completed) => {
            direct_navigation_result_from_fetch_continuation(completed, result)
        }
        super::CompletedPageCommandKind::TraverseSameDocumentHistory(completed) => {
            direct_navigation_result_from_completed_same_document_traversal(completed, result)
        }
        _ => Ok(()),
    }
}

fn direct_navigation_result_from_completed_same_document(
    completed: &CompletedSameDocumentNavigateCommand,
    result: &mut DirectNavigationResult,
) -> Result<(), DevToolsError> {
    if let Err(message) = &completed.completed {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            message.clone(),
        ));
    }
    result.loader_id = None;
    result.navigation_id = None;
    Ok(())
}

fn direct_navigation_result_from_completed_same_document_traversal(
    completed: &CompletedSameDocumentHistoryTraversalCommand,
    result: &mut DirectNavigationResult,
) -> Result<(), DevToolsError> {
    match &completed.completed {
        Ok(completion) => {
            if completion.bool_reply_value() == Some(false) {
                result.set_history_traversal_same_document(false);
            }
            Ok(())
        }
        Err(message) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            message.clone(),
        )),
    }
}

fn direct_navigation_result_from_completed_load(
    conn: &CdpConnection,
    completed: &CompletedNavigateLoadCommand,
    result: &mut DirectNavigationResult,
) -> Result<(), DevToolsError> {
    if !conn.accepts_pending_document_navigation_for_session_owner(
        completed.state.navigate_session_id.as_deref(),
        &completed.token,
    ) {
        if superseded_cdp_page_navigate_payload(&completed.state).is_some() {
            result.set_cdp_navigation_aborted(&completed.state.frame_id);
            return Ok(());
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Navigation aborted",
        ));
    }
    match &completed.navigation {
        Ok(navigation) => {
            result.set_navigation_identity(&completed.state.frame_id, &completed.state.loader_id);
            if result.protocol != DevToolsProtocol::WebDriverBidi
                && matches!(navigation, NavigationLoadOutcome::Download(_))
            {
                result.loader_id = None;
            }
            Ok(())
        }
        Err(message) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            message.clone(),
        )),
    }
}

fn direct_navigation_result_from_completed_child_frame(
    completed: &CompletedChildFrameNavigateCommand,
    result: &mut DirectNavigationResult,
) -> Result<(), DevToolsError> {
    if let Err(message) = &completed.completed {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            message.clone(),
        ));
    }
    result.set_url(completed.url.clone());
    Ok(())
}

fn direct_navigation_result_from_fetch_continuation(
    completed: &CompletedContinueNavigationWithoutRequestPauseCommand,
    result: &mut DirectNavigationResult,
) -> Result<(), DevToolsError> {
    if completed.pending.document_navigation_token.is_none() {
        if superseded_cdp_page_navigate_payload(&completed.pending.navigation).is_some() {
            result.set_cdp_navigation_aborted(&completed.pending.navigation.frame_id);
            return Ok(());
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Navigation aborted",
        ));
    }
    let state = &completed.pending.navigation;
    if !result.url.is_empty() {
        result.set_navigation_identity(&state.frame_id, &state.loader_id);
    }
    Ok(())
}

fn superseded_cdp_page_navigate_payload(state: &NavigationDispatchState) -> Option<Value> {
    if state.result_projection.protocol() != DevToolsProtocol::Cdp {
        return None;
    }
    let mut result = state.result_projection.payload().clone();
    let payload = result.as_object_mut()?;
    if !payload.contains_key("frameId") {
        return None;
    }
    payload.insert("frameId".to_owned(), json!(state.frame_id));
    payload.remove("loaderId");
    payload.insert("errorText".to_owned(), json!(NET_ERR_ABORTED_ERROR_TEXT));
    payload.insert("isDownload".to_owned(), json!(false));
    Some(result)
}

pub(crate) fn push_superseded_navigation_result(
    out: &mut CommandOutputBuffer,
    state: &NavigationDispatchState,
) {
    if state.navigate_id.is_none() {
        return;
    }
    if let Some(result) = superseded_cdp_page_navigate_payload(state) {
        out.push_result_after_messages(result);
    } else {
        out.push_error_after_messages(-32000, "Navigation aborted");
    }
}

fn fill_navigation_id_from_current_loader_for_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    result: &mut DirectNavigationResult,
) {
    if result.navigation_id.is_some() {
        return;
    }
    let loader_id = {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
        route_scope
            .conn_mut()
            .current_document_loader_id_for_session_owner(None)
    };
    result.ensure_navigation_id_from_loader(loader_id);
}

fn start_devtools_navigate_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: &DevToolsNavigateCommand,
    result_projection: NavigationResultProjection,
    allow_background_navigation: bool,
) -> NavigateCommandStart {
    if let Some(frame_id) = child_frame_navigation_target_id(conn, command) {
        return start_child_frame_navigate_command(
            conn,
            command_id,
            command_session_id,
            frame_id,
            command.url.as_str(),
            command.wait,
            protocol_neutral_navigate_result_payload(
                command.context.protocol,
                Some(frame_id),
                None,
                &command.url,
            ),
        );
    }
    if session_owner_navigation_is_same_document_fragment(
        conn,
        command_session_id,
        command.url.as_str(),
    ) {
        return start_top_level_same_document_navigate_command(conn, command_session_id, command);
    }
    start_navigate_to_url_command_with_background_policy(
        conn,
        command_id,
        command_session_id,
        &command.url,
        command.referrer.as_deref(),
        result_projection,
        allow_background_navigation,
        NavigationRequestLoadPolicy::BrowserInitiated,
        NavigationStartInitiator::Browser,
    )
}

fn session_owner_navigation_is_same_document_fragment(
    conn: &CdpConnection,
    session_id: Option<&str>,
    target: &str,
) -> bool {
    let Some(current) = conn.runtime_session_owner_target_url(session_id) else {
        return false;
    };
    let (Ok(mut current), Ok(mut target)) = (Url::parse(&current), Url::parse(target)) else {
        return false;
    };
    // Chromium treats an exact repeat of a fragment-free URL as a new
    // document navigation. Removing or changing a fragment is same-document,
    // including a repeat of the exact same fragment URL.
    if current.fragment().is_none() && target.fragment().is_none() {
        return false;
    }
    current.set_fragment(None);
    target.set_fragment(None);
    current == target
}

fn start_top_level_same_document_navigate_command(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    command: &DevToolsNavigateCommand,
) -> NavigateCommandStart {
    let result_payload = protocol_neutral_navigate_result_payload(
        command.context.protocol,
        command
            .context
            .target_id
            .as_ref()
            .map(|target| target.as_str()),
        None,
        &command.url,
    );
    start_top_level_same_document_navigate(
        conn,
        command_session_id,
        command.url.clone(),
        result_payload,
    )
}

fn start_top_level_same_document_navigate(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    url: String,
    result_payload: Value,
) -> NavigateCommandStart {
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    match page.start_top_level_same_document_navigation(url) {
        Ok(pending) => NavigateCommandStart::PendingSameDocument(Box::new(
            PendingSameDocumentNavigateCommand {
                pending,
                result_payload,
            },
        )),
        Err(error) => {
            NavigateCommandStart::CompletePlan(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn child_frame_navigation_target_id<'a>(
    conn: &CdpConnection,
    command: &'a DevToolsNavigateCommand,
) -> Option<&'a str> {
    let target_id = command.context.target_id.as_ref()?.as_str();
    if conn.target_session_route_for_target_id(target_id).is_some() {
        return None;
    }
    conn.has_attached_child_frame_id(target_id)
        .then_some(target_id)
}

fn start_child_frame_navigate_command(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    command_session_id: Option<&str>,
    frame_id: &str,
    url: &str,
    wait: DevToolsNavigationWait,
    result_payload: Value,
) -> NavigateCommandStart {
    let Some(source_document) =
        conn.target_root_document_lifecycle_identity_for_session(command_session_id)
    else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    let Some(activity_binding) = conn
        .target_root_document_protocol_attachment_identity_for_session(
            command_session_id,
            source_document,
        )
    else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    match page.start_child_frame_navigation_to_url(frame_id, url) {
        Ok(pending) => {
            NavigateCommandStart::PendingChildFrame(Box::new(PendingChildFrameNavigateCommand {
                prefix_events: Vec::new(),
                pending,
                activity_binding,
                url: url.to_owned(),
                wait,
                result_payload,
            }))
        }
        Err(error) => {
            NavigateCommandStart::CompletePlan(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

pub(super) fn try_start_reload_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params = match cmd.get_params::<ReloadParams>() {
        Ok(Some(params)) => params,
        Ok(None) => ReloadParams::default(),
        Err(_) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    let Some((_, target_id)) = conn.target_owner_identity_for_session(cmd.session_id) else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let Some(target_id) = target_id else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };
    let reloaded_after_crash_session_ids =
        reloaded_after_crash_session_ids_for_session_owner(conn, cmd.session_id);
    let shared_command = build_cdp_reload_command(cmd, Some(target_id.as_str()), params);
    let result_projection =
        NavigationResultProjection::new(shared_command.context.protocol, json!({}));
    start_devtools_page_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::Reload(shared_command),
        DevToolsNavigationStartOptions {
            result_projection,
            reloaded_after_crash_session_ids,
            allow_background_navigation: true,
        },
    )
}

fn build_cdp_reload_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    params: ReloadParams,
) -> DevToolsReloadCommand {
    DevToolsReloadCommand {
        context: cmd.devtools_command_context(target_id, Option::<&str>::None),
        ignore_cache: params.ignore_cache.unwrap_or(false),
        script_to_evaluate_on_load: params.script_to_evaluate_on_load,
        wait: DevToolsNavigationWait::DocumentInstalled,
    }
}

fn start_devtools_reload_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: &DevToolsReloadCommand,
    options: DevToolsNavigationStartOptions,
) -> PageCommandTaskStep {
    let context_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let command_session_id = command_session_id.or(context_session_id);
    let Some(url) = conn.runtime_session_owner_target_url(command_session_id) else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };
    if conn
        .mark_next_navigation_history_replace_current_for_session_owner(command_session_id)
        .is_none()
    {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    }
    let start = start_navigate_to_url_command_with_background_policy(
        conn,
        command_id,
        command_session_id,
        url.as_str(),
        None,
        options.result_projection,
        options.allow_background_navigation,
        NavigationRequestLoadPolicy::Reload,
        NavigationStartInitiator::Browser,
    );
    finish_started_navigation_command_for_parts(
        conn,
        command_id,
        command_session_id,
        start,
        &options.reloaded_after_crash_session_ids,
    )
}

pub(super) fn try_start_navigate_to_history_entry_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: NavigateToHistoryEntryParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let Ok(entry_id) = i32::try_from(params.entry_id) else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
    };
    let Some(url) =
        conn.target_session_owner_navigation_history_entry_url(cmd.session_id, entry_id)
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "Navigation history entry not found",
        ));
    };
    let shared_command = build_cdp_traverse_history_command(cmd, None, entry_id, url);
    let result_projection =
        NavigationResultProjection::new(shared_command.context.protocol, json!({}));
    start_devtools_page_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::TraverseHistory(shared_command),
        DevToolsNavigationStartOptions {
            result_projection,
            reloaded_after_crash_session_ids: reloaded_after_crash_session_ids_for_session_owner(
                conn,
                cmd.session_id,
            ),
            allow_background_navigation: true,
        },
    )
}

fn build_cdp_traverse_history_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    entry_id: i32,
    url: String,
) -> DevToolsTraverseHistoryCommand {
    DevToolsTraverseHistoryCommand {
        context: cmd.devtools_command_context(target_id, Option::<&str>::None),
        destination: DevToolsHistoryTraversalDestination::Entry { entry_id, url },
        wait: DevToolsNavigationWait::DocumentInstalled,
    }
}

enum ResolvedDevToolsHistoryTraversal {
    Noop,
    Entry {
        entry_id: i32,
        url: String,
        same_document_delta: Option<i64>,
    },
}

fn resolve_devtools_history_traversal_destination(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    destination: &DevToolsHistoryTraversalDestination,
) -> Result<ResolvedDevToolsHistoryTraversal, DevToolsError> {
    let Some((current_index, entries)) =
        conn.target_session_owner_navigation_history_snapshot(session_id)
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "TargetNotLoaded",
        ));
    };
    let target_index = match destination {
        DevToolsHistoryTraversalDestination::Entry { entry_id, .. } => entries
            .iter()
            .position(|entry| entry.id == *entry_id)
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchHistoryEntry, "NoSuchHistoryEntry")
            })?,
        DevToolsHistoryTraversalDestination::Delta(delta) => {
            let target_index = current_index as i128 + i128::from(*delta);
            if target_index < 0 || target_index >= entries.len() as i128 {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchHistoryEntry,
                    "NoSuchHistoryEntry",
                ));
            }
            usize::try_from(target_index).map_err(|_| {
                DevToolsError::new(DevToolsErrorKind::NoSuchHistoryEntry, "NoSuchHistoryEntry")
            })?
        }
    };
    if target_index == current_index {
        return Ok(ResolvedDevToolsHistoryTraversal::Noop);
    }
    let current_entry = entries.get(current_index).ok_or_else(|| {
        DevToolsError::new(DevToolsErrorKind::NoSuchHistoryEntry, "NoSuchHistoryEntry")
    })?;
    let target_entry = entries.get(target_index).ok_or_else(|| {
        DevToolsError::new(DevToolsErrorKind::NoSuchHistoryEntry, "NoSuchHistoryEntry")
    })?;
    let same_document_delta = (current_entry.document_sequence_number.is_some()
        && current_entry.document_sequence_number == target_entry.document_sequence_number)
        .then(|| {
            let current_index = i64::try_from(current_index).map_err(|_| {
                DevToolsError::new(DevToolsErrorKind::NoSuchHistoryEntry, "NoSuchHistoryEntry")
            })?;
            let target_index = i64::try_from(target_index).map_err(|_| {
                DevToolsError::new(DevToolsErrorKind::NoSuchHistoryEntry, "NoSuchHistoryEntry")
            })?;
            Ok::<_, DevToolsError>(target_index - current_index)
        })
        .transpose()?;
    Ok(ResolvedDevToolsHistoryTraversal::Entry {
        entry_id: target_entry.id,
        url: target_entry.url.clone(),
        same_document_delta,
    })
}

fn start_same_document_history_traversal_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    delta: i64,
    fallback: HistoryTraversalUrlFallback,
) -> PageCommandTaskStep {
    let page = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page())
        .ok_or_else(|| anyhow::anyhow!("TargetNotLoaded"));
    let page = match page {
        Ok(page) => page,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                error.to_string(),
            ));
        }
    };
    let pending = match page.start_top_level_history_traversal_by_delta(delta) {
        Ok(pending) => pending,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                error.to_string(),
            ));
        }
    };
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id,
        owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
        kind: Box::new(super::PendingPageCommandKind::TraverseSameDocumentHistory(
            Box::new(PendingSameDocumentHistoryTraversalCommand { pending, fallback }),
        )),
    })
}

pub(super) fn complete_pending_same_document_history_traversal_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    completed: CompletedSameDocumentHistoryTraversalCommand,
) -> PageCommandTaskStep {
    let CompletedSameDocumentHistoryTraversalCommand {
        completed,
        fallback,
    } = completed;
    let completion = match completed {
        Ok(completion) => completion,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let result = conn
        .runtime_session_owner_slot_mut(session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
        .ok_or_else(|| anyhow::anyhow!("TargetNotLoaded"))
        .and_then(|page| page.finish_top_level_history_traversal_by_delta(completion));
    match result {
        Ok(true) => PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_result(
            DevToolsCommandResult::Empty,
        )),
        Ok(false) => start_history_traversal_url_fallback(conn, command_id, session_id, fallback),
        Err(error) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn start_history_traversal_url_fallback(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    fallback: HistoryTraversalUrlFallback,
) -> PageCommandTaskStep {
    let HistoryTraversalUrlFallback {
        entry_id,
        url,
        result_projection,
        reloaded_after_crash_session_ids,
        allow_background_navigation,
    } = fallback;
    start_resolved_history_traversal_command(
        conn,
        command_id,
        command_session_id,
        entry_id,
        &url,
        DevToolsNavigationStartOptions {
            result_projection,
            reloaded_after_crash_session_ids,
            allow_background_navigation,
        },
        HistoryTraversalStartSource::BrowserCommand,
    )
}

pub(super) fn start_session_owner_history_traversal_from_renderer(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    delta: i64,
) -> PageCommandTaskStep {
    let reloaded_after_crash_session_ids =
        reloaded_after_crash_session_ids_for_session_owner(conn, session_id);
    let destination = DevToolsHistoryTraversalDestination::Delta(delta);
    let (entry_id, url) =
        match resolve_devtools_history_traversal_destination(conn, session_id, &destination) {
            Ok(ResolvedDevToolsHistoryTraversal::Entry { entry_id, url, .. }) => (entry_id, url),
            Ok(ResolvedDevToolsHistoryTraversal::Noop) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::success());
            }
            Err(error) => {
                tracing::debug!(
                    session_id,
                    delta,
                    error = error.message.as_str(),
                    "ignoring renderer history traversal without a browser-side destination"
                );
                return PageCommandTaskStep::Complete(CommandOutputPlan::success());
            }
        };
    start_resolved_history_traversal_command(
        conn,
        None,
        session_id,
        entry_id,
        &url,
        DevToolsNavigationStartOptions {
            result_projection: NavigationResultProjection::Cdp(json!({})),
            reloaded_after_crash_session_ids,
            allow_background_navigation: false,
        },
        HistoryTraversalStartSource::Renderer,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryTraversalStartSource {
    BrowserCommand,
    Renderer,
}

fn start_resolved_history_traversal_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    entry_id: i32,
    url: &str,
    options: DevToolsNavigationStartOptions,
    source: HistoryTraversalStartSource,
) -> PageCommandTaskStep {
    if conn
        .mark_next_navigation_history_traverse_to_entry_for_session_owner(
            command_session_id,
            entry_id,
        )
        .is_none()
    {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    }
    let initiator = match source {
        HistoryTraversalStartSource::BrowserCommand => NavigationStartInitiator::Browser,
        HistoryTraversalStartSource::Renderer => NavigationStartInitiator::Renderer,
    };
    let DevToolsNavigationStartOptions {
        result_projection,
        reloaded_after_crash_session_ids,
        allow_background_navigation,
    } = options;
    let start = start_navigate_to_url_command_with_background_policy(
        conn,
        command_id,
        command_session_id,
        url,
        None,
        result_projection,
        allow_background_navigation,
        NavigationRequestLoadPolicy::BrowserInitiated,
        initiator,
    );
    finish_started_navigation_command_for_parts(
        conn,
        command_id,
        command_session_id,
        start,
        &reloaded_after_crash_session_ids,
    )
}

fn start_devtools_traverse_history_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: &DevToolsTraverseHistoryCommand,
    options: DevToolsNavigationStartOptions,
) -> PageCommandTaskStep {
    let context_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let command_session_id = command_session_id.or(context_session_id);
    let (entry_id, url, same_document_delta) = match resolve_devtools_history_traversal_destination(
        conn,
        command_session_id,
        &command.destination,
    ) {
        Ok(ResolvedDevToolsHistoryTraversal::Noop) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_result(
                DevToolsCommandResult::Empty,
            ));
        }
        Ok(ResolvedDevToolsHistoryTraversal::Entry {
            entry_id,
            url,
            same_document_delta,
        }) => (entry_id, url, same_document_delta),
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
        }
    };
    if let Some(delta) = same_document_delta {
        let fallback = HistoryTraversalUrlFallback {
            entry_id,
            url,
            result_projection: options.result_projection,
            reloaded_after_crash_session_ids: options.reloaded_after_crash_session_ids,
            allow_background_navigation: options.allow_background_navigation,
        };
        return start_same_document_history_traversal_command(
            conn,
            command_id,
            command_session_id,
            delta,
            fallback,
        );
    }
    start_history_traversal_url_fallback(
        conn,
        command_id,
        command_session_id,
        HistoryTraversalUrlFallback {
            entry_id,
            url,
            result_projection: options.result_projection,
            reloaded_after_crash_session_ids: options.reloaded_after_crash_session_ids,
            allow_background_navigation: options.allow_background_navigation,
        },
    )
}

pub(super) fn finish_started_navigation_command_for_parts(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    mut start: NavigateCommandStart,
    reloaded_after_crash_session_ids: &[Option<String>],
) -> PageCommandTaskStep {
    match &mut start {
        NavigateCommandStart::CompletePlan(_) => {}
        NavigateCommandStart::CompleteImmediate(plan) => {
            clear_crash_state_after_navigation_into_plan(
                conn,
                plan,
                command_session_id,
                reloaded_after_crash_session_ids,
            )
        }
        NavigateCommandStart::PendingLoad(pending) => clear_crash_state_after_navigation(
            conn,
            &mut pending.prefix_events,
            command_session_id,
            reloaded_after_crash_session_ids,
        ),
        NavigateCommandStart::PendingChildFrame(_) => {}
        NavigateCommandStart::PendingSameDocument(_) => {}
        NavigateCommandStart::PendingContinueWithoutRequestPause(pending) => {
            clear_crash_state_after_navigation(
                conn,
                &mut pending.prefix_events,
                command_session_id,
                reloaded_after_crash_session_ids,
            )
        }
    }
    match start {
        NavigateCommandStart::CompletePlan(plan) => PageCommandTaskStep::Complete(plan),
        NavigateCommandStart::CompleteImmediate(plan) => PageCommandTaskStep::Complete(plan),
        NavigateCommandStart::PendingLoad(pending) => {
            PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
                command_id,
                owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
                kind: Box::new(super::PendingPageCommandKind::Navigate(pending)),
            })
        }
        NavigateCommandStart::PendingChildFrame(pending) => {
            PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
                command_id,
                owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
                kind: Box::new(super::PendingPageCommandKind::ChildFrameNavigate(pending)),
            })
        }
        NavigateCommandStart::PendingSameDocument(pending) => {
            PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
                command_id,
                owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
                kind: Box::new(super::PendingPageCommandKind::SameDocumentNavigate(pending)),
            })
        }
        NavigateCommandStart::PendingContinueWithoutRequestPause(pending) => {
            PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
                command_id,
                owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
                kind: Box::new(
                    super::PendingPageCommandKind::ContinueNavigationWithoutRequestPause(pending),
                ),
            })
        }
    }
}

pub(super) fn start_session_owner_navigation_from_renderer(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    url: &str,
    request_method: &str,
    request_body: Option<&[u8]>,
    request_headers: &[(String, String)],
    browser_navigation_kind: moli_fetch::BrowserNavigationRequestKind,
) -> NavigateCommandStart {
    let reloaded_after_crash_session_ids =
        reloaded_after_crash_session_ids_for_session_owner(conn, session_id);
    let result_payload = cdp_navigate_result_payload(
        None,
        conn.runtime_session_owner_frame_id(session_id).as_deref(),
        None,
        url,
    );
    let start = if request_method.eq_ignore_ascii_case("GET")
        && session_owner_navigation_is_same_document_fragment(conn, session_id, url)
    {
        // A renderer-owned top-level navigation follows the same fragment
        // classification as Page.navigate. In particular, a freshly created
        // popup's `about:blank#fragment` target must not discard its initial
        // Document by trying to fetch the non-fetchable about: URL.
        start_top_level_same_document_navigate(conn, session_id, url.to_owned(), result_payload)
    } else {
        let result_projection = NavigationResultProjection::Cdp(result_payload);
        start_navigate_to_url_command_with_background_policy_and_request(
            conn,
            None,
            session_id,
            url,
            None,
            result_projection,
            request_method,
            request_body.map(<[u8]>::to_vec),
            request_headers.to_vec(),
            // The renderer has already settled the browsing-context action;
            // fetching and committing the destination is a new navigation
            // lifecycle, not work that the current protocol projection may
            // await inline. In particular, popup creation is projected while
            // the opener's concrete output cursor is being completed. Waiting
            // for the popup Page here would make that opener turn depend on a
            // second renderer stream whose publications the scheduler cannot
            // ingest until this call returns. The background navigation gate
            // gives the load its own exact completion and wake. This preserves
            // Blink's observable boundary: `LocalDOMWindow::open()` may invoke
            // navigation synchronously, but it returns without waiting for
            // that navigation's network load or Document commit.
            true,
            match browser_navigation_kind {
                moli_fetch::BrowserNavigationRequestKind::Navigate => {
                    NavigationRequestLoadPolicy::DocumentInitiated
                }
                moli_fetch::BrowserNavigationRequestKind::Reload => {
                    NavigationRequestLoadPolicy::Reload
                }
            },
            NavigationStartInitiator::Renderer,
        )
    };
    clear_crash_state_for_renderer_navigation(
        conn,
        start,
        session_id,
        &reloaded_after_crash_session_ids,
    )
}

#[cfg(test)]
fn merge_child_frame_tree_attachments_into_events(
    events: &mut Vec<PagePreparedChildFrameTreeEvent>,
    additional: Vec<(String, String)>,
) {
    for (frame_id, parent_frame_id) in additional {
        match events
            .iter_mut()
            .rev()
            .find(|event| prepared_child_frame_tree_event_frame_id(event) == frame_id)
        {
            Some(PagePreparedChildFrameTreeEvent::Attached {
                parent_frame_id: existing_parent_frame_id,
                ..
            }) => {
                *existing_parent_frame_id = parent_frame_id;
            }
            Some(PagePreparedChildFrameTreeEvent::Detached { .. }) | None => {
                events.push(PagePreparedChildFrameTreeEvent::Attached {
                    frame_id,
                    parent_frame_id,
                });
            }
        }
    }
}

fn emit_prepared_child_frame_document_open_prefix_for_session(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    document: &mut PagePreparedChildFrameDocumentActivity,
) {
    let mut document_opened_events = std::mem::take(&mut document.document_opened_events);
    for tree_event in std::mem::take(&mut document.child_frame_tree_events) {
        if let PagePreparedChildFrameTreeEvent::Detached { frame_id } = &tree_event {
            let mut event_index = 0;
            while event_index < document_opened_events.len() {
                if document_opened_events[event_index].frame_id != *frame_id {
                    event_index += 1;
                    continue;
                }
                let event = document_opened_events.remove(event_index);
                emit_renderer_command_child_frame_document_opened_background_events_with_security(
                    conn,
                    out,
                    session_id,
                    event,
                    document.timestamp,
                    &document.security_origin,
                    &document.secure_context_type,
                );
            }
        }
        emit_prepared_child_frame_tree_background_events(conn, out, session_id, vec![tree_event]);
    }
    for event in document_opened_events {
        emit_renderer_command_child_frame_document_opened_background_events_with_security(
            conn,
            out,
            session_id,
            event,
            document.timestamp,
            &document.security_origin,
            &document.secure_context_type,
        );
    }
}

fn emit_renderer_command_child_frame_document_opened_background_events_with_security(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner_session_id: Option<&str>,
    event: ChildFrameDocumentOpenedSnapshot,
    timestamp: f64,
    parent_security_origin: &str,
    parent_secure_context_type: &str,
) {
    let (security_origin, secure_context_type) = child_frame_security_identity(
        &event.url,
        event.security_origin_inherited,
        event.security_origin_opaque,
        parent_security_origin,
        parent_secure_context_type,
    );
    for session_id in conn.subscribed_page_event_session_ids_for_session_owner(owner_session_id) {
        let lifecycle_enabled = conn
            .target_page_session_state_for_session(session_id.as_deref())
            .is_some_and(|state| state.page_lifecycle_events);
        emit_child_frame_document_opened_background_events(
            out,
            session_id.as_deref(),
            lifecycle_enabled,
            &event.frame_id,
            event.parent_frame_id.as_deref(),
            event.name.as_deref(),
            event.loader_id.as_deref().unwrap_or(LOADER_ID),
            &event.url,
            &security_origin,
            &secure_context_type,
            timestamp,
        );
    }
}

#[cfg(test)]
fn prepared_child_frame_tree_event_frame_id(event: &PagePreparedChildFrameTreeEvent) -> &str {
    match event {
        PagePreparedChildFrameTreeEvent::Attached { frame_id, .. }
        | PagePreparedChildFrameTreeEvent::Detached { frame_id } => frame_id,
    }
}

pub(crate) async fn emit_prepared_child_frame_activity(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    activity: PagePreparedChildFrameActivity,
    browser_initiated_frame_id: Option<&str>,
) {
    let binding_is_current =
        conn.target_root_document_protocol_attachment_identity_is_current(activity.binding());
    if !binding_is_current {
        return;
    }
    let session_id = activity.binding().session_id().map(str::to_owned);
    let (binding, mut document) = activity.into_parts();
    let root_document = binding.root_document();
    let mut activity_events = Vec::new();
    let timing_enabled = moli_trace::cdp_nav_timing_enabled();
    let timing_started = timing_enabled.then(std::time::Instant::now);
    let load_count = document.loads.len();
    let document_network_count = document.document_networks.len();
    let document_opened_count = document.document_opened_events.len();
    let frame_tree_event_count = document.child_frame_tree_events.len();
    let page_event_session_ids =
        conn.subscribed_page_event_session_ids_for_session_owner(session_id.as_deref());
    emit_prepared_child_frame_document_open_prefix_for_session(
        conn,
        &mut activity_events,
        session_id.as_deref(),
        &mut document,
    );
    if !conn.target_root_document_protocol_attachment_identity_is_current(&binding) {
        return;
    }
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            stage = "child_frame_activity_opening_events_emitted",
            frame_tree_events = frame_tree_event_count,
            document_networks = document_network_count,
            document_opened = document_opened_count,
            loads = load_count,
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    for document_network in document.document_networks {
        network::emit_child_document_navigation_network_background_events(
            conn,
            &mut activity_events,
            session_id.as_deref(),
            &document_network.frame_id,
            &document_network.loader_id,
            &document_network.loader_id,
            document_network.timestamp,
            &document_network.snapshot,
        );
    }
    for load in document.loads {
        if load.document_open_replacement {
            for event_session_id in &page_event_session_ids {
                let lifecycle_enabled = conn
                    .target_page_session_state_for_session(event_session_id.as_deref())
                    .is_some_and(|state| state.page_lifecycle_events);
                emit_child_frame_document_open_completed_background_events(
                    &mut activity_events,
                    event_session_id.as_deref(),
                    lifecycle_enabled,
                    &load.navigation_commit.frame_id,
                    &load.navigation_commit.loader_id,
                    document.timestamp,
                );
            }
            continue;
        }
        let (security_origin, secure_context_type) = child_frame_security_identity(
            &load.navigation_commit.url,
            load.navigation_commit.security_origin_inherited,
            load.navigation_commit.security_origin_opaque,
            &document.security_origin,
            &document.secure_context_type,
        );
        let navigation_start_initiator =
            if browser_initiated_frame_id == Some(load.navigation_start.frame_id.as_str()) {
                NavigationStartInitiator::Browser
            } else {
                NavigationStartInitiator::RendererChildFrame
            };
        for event_session_id in &page_event_session_ids {
            emit_navigation_started_background_events(
                &mut activity_events,
                event_session_id.as_deref(),
                &load.navigation_start.frame_id,
                &load.navigation_start.loader_id,
                &load.navigation_start.url,
                navigation_start_initiator,
            );
        }
        if let Some(document_network) = load.document_network.as_ref() {
            network::emit_child_document_navigation_network_background_events(
                conn,
                &mut activity_events,
                session_id.as_deref(),
                &document_network.frame_id,
                &document_network.loader_id,
                &document_network.loader_id,
                document_network.timestamp,
                &document_network.snapshot,
            );
        }
        for event_session_id in &page_event_session_ids {
            emit_child_frame_navigation_commit(
                &mut activity_events,
                event_session_id.as_deref(),
                &load.navigation_commit.frame_id,
                load.navigation_commit.parent_frame_id.as_deref(),
                load.navigation_commit.name.as_deref(),
                &load.navigation_commit.loader_id,
                &load.navigation_commit.url,
                &security_origin,
                &secure_context_type,
            );
            let lifecycle_enabled = conn
                .target_page_session_state_for_session(event_session_id.as_deref())
                .is_some_and(|state| state.page_lifecycle_events);
            emit_child_frame_lifecycle_terminal(
                &mut activity_events,
                event_session_id.as_deref(),
                lifecycle_enabled,
                &load.lifecycle_terminal.frame_id,
                &load.lifecycle_terminal.loader_id,
                load.lifecycle_terminal.timestamp,
            );
        }
    }
    out.extend(
        activity_events
            .into_iter()
            .filter_map(|event| event.bind_to_root_document_route(conn, root_document)),
    );
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            stage = "child_frame_activity_emitted",
            loads = load_count,
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
}

pub(crate) fn emit_prepared_child_frame_tree_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner_session_id: Option<&str>,
    events: Vec<PagePreparedChildFrameTreeEvent>,
) {
    for event in events {
        match event {
            PagePreparedChildFrameTreeEvent::Attached {
                frame_id,
                parent_frame_id,
            } => {
                let is_new_attachment = conn
                    .with_target_owner_state_for_session_mut(owner_session_id, |owner_state| {
                        owner_state.insert_attached_child_frame_id(frame_id.clone())
                    })
                    .unwrap_or(false);
                if is_new_attachment {
                    for session_id in
                        conn.subscribed_page_event_session_ids_for_session_owner(owner_session_id)
                    {
                        out.push(BackgroundProtocolEvent::page_frame_attached(
                            session_id.as_deref(),
                            frame_id.clone(),
                            parent_frame_id.clone(),
                        ));
                    }
                }
            }
            PagePreparedChildFrameTreeEvent::Detached { frame_id } => {
                let was_attached = conn
                    .with_target_owner_state_for_session_mut(owner_session_id, |owner_state| {
                        owner_state.remove_attached_child_frame_id(&frame_id)
                    })
                    .unwrap_or(false);
                if was_attached {
                    for session_id in
                        conn.subscribed_page_event_session_ids_for_session_owner(owner_session_id)
                    {
                        out.push(BackgroundProtocolEvent::page_frame_detached(
                            session_id.as_deref(),
                            frame_id.clone(),
                        ));
                    }
                }
            }
        }
    }
}

pub(super) fn get_navigation_history_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let Some((current_index, entries)) =
        conn.target_session_owner_navigation_history_snapshot(cmd.session_id)
    else {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    };
    let entries = entries
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "url": entry.url,
                "userTypedURL": entry.user_typed_url,
                "title": entry.title,
                "transitionType": entry.transition_type,
            })
        })
        .collect::<Vec<_>>();
    CommandOutputPlan::result(json!({
            "currentIndex": current_index,
            "entries": entries,
    }))
}

pub(super) fn try_start_reset_navigation_history_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    match conn.can_reset_navigation_history_for_session_owner(cmd.session_id) {
        Some(true) => {}
        Some(false) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "History cannot be pruned",
            ));
        }
        None => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "NoDocumentLoaded",
            ));
        }
    }
    let pending = {
        let Some(page) = conn
            .runtime_session_owner_slot_mut(cmd.session_id)
            .ok()
            .and_then(|slot| slot.loaded_page_mut())
        else {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "NoDocumentLoaded",
            ));
        };
        match page.start_reset_navigation_history() {
            Ok(pending) => pending,
            Err(error) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
        }
    };
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope: crate::conn::CommandOwnerScope::capture(conn, cmd.session_id),
        kind: Box::new(super::PendingPageCommandKind::ResetNavigationHistory { pending }),
    })
}

pub(super) fn complete_reset_navigation_history_command(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    completed: Result<CompletedPageCommand, String>,
) -> PageCommandTaskStep {
    let completion = match completed {
        Ok(completion) => completion,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let Some(page) = conn
        .runtime_session_owner_slot_mut(session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "NoDocumentLoaded"));
    };
    match page.finish_reset_navigation_history(completion) {
        Ok(true) => {}
        Ok(false) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "History cannot be pruned",
            ));
        }
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                error.to_string(),
            ));
        }
    }
    match conn.reset_navigation_history_for_session_owner(session_id) {
        Some(true) => PageCommandTaskStep::Complete(CommandOutputPlan::success()),
        Some(false) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "History cannot be pruned",
        )),
        None => PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "NoDocumentLoaded")),
    }
}

fn reloaded_after_crash_session_ids_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Vec<Option<String>> {
    if !conn
        .target_owner_state_for_session(session_id)
        .is_some_and(|owner_state| owner_state.target_crash_state.is_crashed())
    {
        return Vec::new();
    }
    conn.page_event_session_ids_for_session_owner(session_id)
        .into_iter()
        .filter(|event_session_id| {
            conn.target_runtime_session_state_for_session(event_session_id.as_deref())
                .is_some_and(|state| state.inspector_target_crashed_delivered())
        })
        .collect()
}

fn clear_crash_state_after_navigation(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner_session_id: Option<&str>,
    reloaded_after_crash_session_ids: &[Option<String>],
) {
    let _ = conn.with_target_owner_state_for_session_mut(owner_session_id, |owner_state| {
        owner_state.target_crash_state.clear();
    });
    for session_id in reloaded_after_crash_session_ids {
        out.push(inspector_target_reloaded_after_crash_event(
            session_id.as_deref(),
        ));
    }
}

fn clear_crash_state_after_navigation_into_plan(
    conn: &mut CdpConnection,
    plan: &mut CommandOutputPlan,
    owner_session_id: Option<&str>,
    reloaded_after_crash_session_ids: &[Option<String>],
) {
    let _ = conn.with_target_owner_state_for_session_mut(owner_session_id, |owner_state| {
        owner_state.target_crash_state.clear();
    });
    for session_id in reloaded_after_crash_session_ids {
        plan.push_background_event(inspector_target_reloaded_after_crash_event(
            session_id.as_deref(),
        ));
    }
}

fn inspector_target_reloaded_after_crash_event(
    session_id: Option<&str>,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::inspector_target_reloaded_after_crash(session_id)
}

fn clear_crash_state_for_renderer_navigation(
    conn: &mut CdpConnection,
    mut start: NavigateCommandStart,
    owner_session_id: Option<&str>,
    reloaded_after_crash_session_ids: &[Option<String>],
) -> NavigateCommandStart {
    match &mut start {
        NavigateCommandStart::CompletePlan(_) => {}
        NavigateCommandStart::CompleteImmediate(plan) => {
            clear_crash_state_after_navigation_into_plan(
                conn,
                plan,
                owner_session_id,
                reloaded_after_crash_session_ids,
            )
        }
        NavigateCommandStart::PendingLoad(pending) => clear_crash_state_after_navigation(
            conn,
            &mut pending.prefix_events,
            owner_session_id,
            reloaded_after_crash_session_ids,
        ),
        NavigateCommandStart::PendingChildFrame(_) => {}
        NavigateCommandStart::PendingSameDocument(_) => {}
        NavigateCommandStart::PendingContinueWithoutRequestPause(pending) => {
            clear_crash_state_after_navigation(
                conn,
                &mut pending.prefix_events,
                owner_session_id,
                reloaded_after_crash_session_ids,
            )
        }
    }
    start
}

pub(super) fn start_navigate_to_url_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    url: &str,
    referrer: Option<&str>,
    result_payload: Value,
) -> NavigateCommandStart {
    start_navigate_to_url_command_with_background_policy(
        conn,
        command_id,
        command_session_id,
        url,
        referrer,
        NavigationResultProjection::Cdp(result_payload),
        true,
        NavigationRequestLoadPolicy::BrowserInitiated,
        NavigationStartInitiator::Browser,
    )
}

fn start_navigate_to_url_command_with_background_policy(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    url: &str,
    referrer: Option<&str>,
    result_projection: NavigationResultProjection,
    allow_background_navigation: bool,
    request_load_policy: NavigationRequestLoadPolicy,
    initiator: NavigationStartInitiator,
) -> NavigateCommandStart {
    start_navigate_to_url_command_with_background_policy_and_request(
        conn,
        command_id,
        command_session_id,
        url,
        referrer,
        result_projection,
        "GET",
        None,
        Vec::new(),
        allow_background_navigation,
        request_load_policy,
        initiator,
    )
}

fn overlay_navigation_request_headers(
    base: &mut Vec<(String, String)>,
    overlay: Vec<(String, String)>,
) {
    for (name, value) in overlay {
        base.retain(|(existing_name, _)| !existing_name.eq_ignore_ascii_case(&name));
        base.push((name, value));
    }
}

#[allow(clippy::too_many_arguments)]
fn start_navigate_to_url_command_with_background_policy_and_request(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    url: &str,
    referrer: Option<&str>,
    result_projection: NavigationResultProjection,
    request_method: &str,
    request_body: Option<Vec<u8>>,
    request_headers: Vec<(String, String)>,
    allow_background_navigation: bool,
    request_load_policy: NavigationRequestLoadPolicy,
    initiator: NavigationStartInitiator,
) -> NavigateCommandStart {
    let mut out = Vec::new();
    let timestamp = monotonic_timestamp_seconds();
    let mut fetch_request_stage = FetchRequestStage::Request;
    let Some(requested_url) = Url::parse(url).ok() else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -32000,
            "Invalid navigation URL",
        ));
    };
    if let Err(error) = route_navigation_url(&requested_url, LocalFileNavigationAccess::Denied) {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -32000,
            error.to_string(),
        ));
    }
    let mut navigation_preflight = conn.prepare_navigation_request_for_session_owner(
        command_session_id,
        &requested_url,
        referrer,
        url.starts_with("data:"),
    );
    let frame_id = navigation_preflight
        .as_ref()
        .map(|preflight| preflight.frame_id.clone())
        .or_else(|| {
            conn.browser_context
                .as_ref()
                .and_then(|bc| bc.active_target_id())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "FRAME-0".to_owned());
    let session_id = command_session_id.map(str::to_owned).or_else(|| {
        navigation_preflight
            .as_ref()
            .and_then(|preflight| preflight.session_id.clone())
    });
    let document_loader_id = navigation_preflight
        .as_ref()
        .map(|preflight| preflight.document_loader_id.clone())
        .unwrap_or_else(|| LOADER_ID.to_owned());
    let inherited_security_origin = navigation_preflight
        .as_ref()
        .map(|preflight| preflight.inherited_security_origin.clone())
        .unwrap_or_else(|| "null".to_owned());
    let inherited_secure_context_type = navigation_preflight
        .as_ref()
        .map(|preflight| preflight.inherited_secure_context_type.clone())
        .unwrap_or_else(|| "Secure".to_owned());
    let mut navigation_state = NavigationDispatchState {
        navigate_id: command_id,
        navigate_session_id: command_session_id.map(str::to_owned),
        result_projection,
        frame_id: frame_id.clone(),
        session_id: None,
        request_id: None,
        loader_id: document_loader_id.clone(),
        request_announced: false,
        requested_url: requested_url.clone(),
        request_method: request_method.to_owned(),
        request_body: request_body
            .as_deref()
            .map(|body| String::from_utf8_lossy(body).into_owned()),
        request_body_bytes: request_body,
        request_headers,
        request_load_policy,
        timestamp,
        source_document_security: NavigationSourceDocumentSecurityContext::new(
            inherited_security_origin,
            inherited_secure_context_type,
        ),
    };
    let mut pending_fetch_navigation = None;

    if let Some(preflight) = navigation_preflight.take() {
        navigation_state.session_id = session_id.clone();
        let mut preflight_headers = preflight.request_headers;
        overlay_navigation_request_headers(
            &mut preflight_headers,
            std::mem::take(&mut navigation_state.request_headers),
        );
        navigation_state.request_headers = preflight_headers;
        navigation_state.loader_id = preflight.document_loader_id.clone();
        update_navigation_result_payload_identity(
            &mut navigation_state.result_projection,
            &preflight.frame_id,
            &preflight.document_loader_id,
        );
        if let Some(loader_id) = preflight.document_request_id {
            navigation_state.request_id = Some(loader_id.clone());
        }
        if preflight.document_fetch_request_stage.is_some() || preflight.document_auth_required {
            let request_stage = preflight
                .document_fetch_request_stage
                .unwrap_or(FetchRequestStage::Response);
            fetch_request_stage = request_stage;
            let Some(fetch_request_id) = preflight.fetch_navigation_request_id else {
                return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
                    -31998,
                    "TargetNotLoaded",
                ));
            };
            pending_fetch_navigation = Some(PendingFetchNavigation {
                fetch_request_id,
                interception_session_id: preflight
                    .document_fetch_event_session_id
                    .or_else(|| session_id.clone()),
                document_navigation_token: None,
                navigation: navigation_state.clone(),
                request_cookie_report: None,
                intercept_response: preflight.document_fetch_response_stage_candidate,
                response_stage_url_match_policy: if preflight
                    .document_fetch_response_stage_candidate
                {
                    ResponseStageUrlMatchPolicy::MatchFinalUrl
                } else {
                    ResponseStageUrlMatchPolicy::AlreadyMatched
                },
                auth_required_blocked_intercepts: preflight
                    .document_auth_required_blocked_intercepts
                    .clone(),
            });
        }
    }

    let navigation_loader_id = navigation_state.loader_id.as_str();
    let document_navigation_token = conn.start_document_navigation_for_session_owner(
        command_session_id,
        navigation_loader_id.to_owned(),
    );
    let Some(document_navigation_token) = document_navigation_token else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -31998,
            "TargetNotLoaded",
        ));
    };
    if let Some(pending) = pending_fetch_navigation.as_mut() {
        pending.document_navigation_token = Some(document_navigation_token.clone());
    }
    if pending_fetch_navigation.is_some() {
        emit_navigation_started_for_session_owner(
            conn,
            &mut out,
            command_session_id,
            &frame_id,
            navigation_loader_id,
            url,
            initiator,
        );
    } else if allow_background_navigation && conn.background_event_sender().is_some() {
        send_background_navigation_started(
            conn,
            document_navigation_token.clone(),
            session_id.as_deref(),
            &frame_id,
            navigation_loader_id,
            url,
            initiator,
        );
    } else {
        emit_navigation_started_for_session_owner(
            conn,
            &mut out,
            command_session_id,
            &frame_id,
            navigation_loader_id,
            url,
            initiator,
        );
    }

    if let Some(mut pending) = pending_fetch_navigation {
        if fetch_request_stage == FetchRequestStage::Request {
            pending.request_cookie_report = navigation_cookie_access_report(
                conn,
                &pending.navigation.requested_url,
                &pending.navigation.request_method,
                None,
                pending.navigation.request_load_policy,
                None,
            );
            if pending.navigation.request_id.is_some() {
                network::emit_fetch_navigation_initial_request_for_pause_background_events(
                    conn,
                    &mut out,
                    &pending.navigation,
                    pending.request_cookie_report.as_ref(),
                );
            }
            pending.navigation.request_announced = pending.navigation.request_id.is_some();
            let _ = conn.register_pending_fetch_navigation_request_for_session_owner(
                pending.navigation.navigate_session_id.as_deref(),
                pending.clone(),
            );
            out.push(fetch::request_paused_background_event(
                pending.interception_session_id.as_deref(),
                &pending,
            ));
            let mut output = CommandOutputBuffer::default();
            output.extend_background_events_after_messages(out);
            return NavigateCommandStart::CompleteImmediate(output.into_plan());
        }
        if pending.navigation.request_id.is_some() {
            pending.request_cookie_report = navigation_cookie_access_report(
                conn,
                &pending.navigation.requested_url,
                &pending.navigation.request_method,
                None,
                pending.navigation.request_load_policy,
                None,
            );
            network::emit_fetch_navigation_initial_request_for_pause_background_events(
                conn,
                &mut out,
                &pending.navigation,
                pending.request_cookie_report.as_ref(),
            );
        }
        pending.navigation.request_announced = pending.navigation.request_id.is_some();
        return NavigateCommandStart::PendingContinueWithoutRequestPause(Box::new(
            PendingContinueNavigationWithoutRequestPauseCommand {
                prefix_events: out,
                pending,
            },
        ));
    }

    if allow_background_navigation
        && let Some(sender) =
            conn.background_navigation_completion_sender_for_session_owner(command_session_id)
    {
        let body_progress_source = if navigation_state.request_id.is_some() {
            let request_cookie_report = navigation_cookie_access_report(
                conn,
                &navigation_state.requested_url,
                &navigation_state.request_method,
                None,
                navigation_state.request_load_policy,
                None,
            );
            network::start_observed_main_document_navigation_progress_background_events(
                conn,
                &mut out,
                &navigation_state,
                request_cookie_report.as_ref(),
            )
        } else {
            network::MainDocumentBodyProgressSource::default()
        };
        let mut completion_state = navigation_state;
        completion_state.request_announced = completion_state.request_id.is_some();
        let early_result = conn.background_event_sender().and_then(|sender| {
            completion_state.navigate_id.map(|navigate_id| {
                crate::conn::BackgroundNavigationEarlyResult::new(
                    sender,
                    navigate_id,
                    completion_state.navigate_session_id.clone(),
                    completion_state.result_projection.payload().clone(),
                )
            })
        });
        let Some(job) = conn.background_navigation_load_job_for_navigation(
            &document_navigation_token,
            &completion_state,
            body_progress_source,
            early_result,
        ) else {
            return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
                -31998,
                "NavigationRequestNotCurrent",
            ));
        };
        let none_session_owner_route = completion_state
            .navigate_session_id
            .is_none()
            .then(|| conn.none_session_owner_route_override())
            .flatten();
        tokio::task::spawn_local(async move {
            let body_completion_sink = crate::conn::BackgroundNavigationBodyCompletionSink::new(
                sender.clone(),
                document_navigation_token.clone(),
                completion_state.clone(),
                none_session_owner_route.clone(),
            );
            let (engine, navigation, early_result_sent) = job.run(Some(body_completion_sink)).await;
            if early_result_sent {
                completion_state.navigate_id = None;
            }
            if moli_trace::cdp_nav_timing_enabled() {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url = %completion_state.requested_url,
                    stage = "background_lifecycle_completion_send",
                );
            }
            let _ = sender.send(BackgroundNavigationCompletion::new(
                document_navigation_token,
                completion_state,
                none_session_owner_route,
                engine,
                navigation,
            ));
        });
        let mut output = CommandOutputBuffer::default();
        output.extend_background_events_after_messages(out);
        return NavigateCommandStart::CompleteImmediate(output.into_plan());
    }

    let body_progress_source = if navigation_state.request_id.is_some() {
        let request_cookie_report = navigation_cookie_access_report(
            conn,
            &navigation_state.requested_url,
            &navigation_state.request_method,
            None,
            navigation_state.request_load_policy,
            None,
        );
        network::start_observed_main_document_navigation_progress_background_events(
            conn,
            &mut out,
            &navigation_state,
            request_cookie_report.as_ref(),
        )
    } else {
        network::MainDocumentBodyProgressSource::default()
    };
    navigation_state.request_announced = navigation_state.request_id.is_some();
    let Some(job) = conn.navigation_load_job_for_navigation(
        &document_navigation_token,
        &navigation_state,
        body_progress_source,
        None,
    ) else {
        return NavigateCommandStart::CompletePlan(CommandOutputPlan::error(
            -31998,
            "NavigationRequestNotCurrent",
        ));
    };
    NavigateCommandStart::PendingLoad(Box::new(PendingNavigateLoadCommand {
        prefix_events: out,
        token: document_navigation_token,
        state: navigation_state,
        job,
    }))
}

pub(super) fn start_initial_document_navigation_for_session_owner(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    result_payload: Value,
) -> Result<NavigateCommandStart, CommandOutputPlan> {
    let Some(url) = conn.runtime_session_owner_target_url(command_session_id) else {
        if conn.browser_context.is_none() {
            return Err(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
        }
        return Err(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };
    Ok(start_navigate_to_url_command(
        conn,
        command_id,
        command_session_id,
        url.as_str(),
        None,
        result_payload,
    ))
}

pub(super) async fn complete_pending_navigate_load_command(
    conn: &mut CdpConnection,
    completed: CompletedNavigateLoadCommand,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    let CompletedNavigateLoadCommand {
        prefix_events,
        token,
        state,
        engine,
        navigation,
    } = completed;
    let is_current = conn.accepts_pending_document_navigation_for_session_owner(
        state.navigate_session_id.as_deref(),
        &token,
    );
    let should_retain_engine = is_current
        && matches!(
            navigation,
            Ok(NavigationLoadOutcome::ResponseCommitReady(_) | NavigationLoadOutcome::Loaded(_))
        );
    let navigation = network::materialize_navigation_load_result(conn, &state, navigation);
    let completion = MaterializedNavigationCompletion::new(token, state, navigation);
    let completion = if should_retain_engine {
        completion.with_navigation_engine(engine)
    } else {
        completion
    };
    let mut output = CommandOutputBuffer::default();
    output.extend_background_events_after_messages(prefix_events);
    conn.drain_materialized_navigation_completion_into_buffer(
        &mut output,
        completion,
        command_context,
    )
    .await;
    PageCommandTaskStep::Complete(output.into_plan())
}

pub(super) async fn complete_pending_child_frame_navigate_command(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    completed: CompletedChildFrameNavigateCommand,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let CompletedChildFrameNavigateCommand {
        prefix_events,
        completed,
        activity_binding,
        url,
        wait,
        mut result_payload,
    } = completed;
    if !conn.target_root_document_protocol_attachment_identity_is_current(&activity_binding) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "NoSuchTarget"));
    }
    let completion = match completed {
        Ok(completion) => completion,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let (navigated, renderer_output) = {
        let Some(page) = conn
            .runtime_session_owner_slot_mut(command_session_id)
            .ok()
            .and_then(|slot| slot.loaded_page_mut())
        else {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "NoSuchTarget"));
        };
        match page.finish_child_frame_navigation_to_url_command_turn(completion) {
            Ok(navigated) => navigated,
            Err(error) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
        }
    };
    command_context.consume_renderer_command_turn_output(renderer_output);
    if !navigated {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "NoSuchTarget"));
    }

    if wait != DevToolsNavigationWait::None {
        let child_gate = match conn.start_child_frame_lifecycle_work_for_session_owner(
            command_session_id,
            CHILD_FRAME_NAVIGATION_LOAD_GATE_TIMEOUT,
        ) {
            Ok(pending) => pending,
            Err(message) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
            }
        };
        let child_gate = match child_gate.wait().await {
            Ok(completed) => completed,
            Err(message) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
            }
        };
        let (completed, renderer_output) = match conn
            .complete_child_frame_lifecycle_work_command_turn_for_session_owner(child_gate)
        {
            Ok(completed) => completed,
            Err(message) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
            }
        };
        command_context.consume_renderer_command_turn_output(renderer_output);
        if !completed {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "ChildFrameNavigationTimeout",
            ));
        }
        // Child-frame events were frozen by their renderer producer turns and
        // are ordered before this response by the exact cursor above. Do not
        // rescan the Page's historical activity queues here: doing so would
        // reintroduce a second producer and race the concrete FIFO.
        if result_payload.get("url").is_some()
            && let Some(payload) = result_payload.as_object_mut()
        {
            payload.insert("url".to_owned(), json!(url));
        }
    }

    let mut output = CommandOutputBuffer::default();
    output.extend_background_events_after_messages(prefix_events);
    output.push_result_after_messages(result_payload);
    PageCommandTaskStep::Complete(output.into_plan())
}

pub(super) async fn complete_pending_same_document_navigate_command(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    completed: CompletedSameDocumentNavigateCommand,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let CompletedSameDocumentNavigateCommand {
        completed,
        result_payload,
    } = completed;
    let completion = match completed {
        Ok(completion) => completion,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let (navigated, output) = {
        let Some(page) = conn
            .runtime_session_owner_slot_mut(command_session_id)
            .ok()
            .and_then(|slot| slot.loaded_page_mut())
        else {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "NoSuchTarget"));
        };
        match page.finish_top_level_same_document_navigation_command_turn(completion) {
            Ok(navigated) => navigated,
            Err(error) => {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
        }
    };
    command_context.consume_renderer_command_turn_output(output);
    if !navigated {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "Navigation is not same-document",
        ));
    }

    let mut plan = CommandOutputPlan::default();
    plan.push_result(result_payload);
    PageCommandTaskStep::Complete(plan)
}

pub(super) async fn complete_pending_continue_navigation_without_request_pause_command(
    conn: &mut CdpConnection,
    completed: CompletedContinueNavigationWithoutRequestPauseCommand,
) -> PageCommandTaskStep {
    let CompletedContinueNavigationWithoutRequestPauseCommand {
        prefix_events,
        pending,
    } = completed;
    let mut output = CommandOutputBuffer::default();
    output.extend_background_events_after_messages(prefix_events);
    fetch::continue_navigation_without_request_pause_into_buffer_async(conn, &mut output, pending)
        .await;
    PageCommandTaskStep::Complete(output.into_plan())
}

pub(crate) async fn complete_materialized_navigation_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    navigation: network::MaterializedNavigationLoadOutcome,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    let navigation_session_id = state.navigate_session_id.clone();
    let navigation_loader_id = state.loader_id.clone();
    match navigation {
        network::MaterializedNavigationLoadOutcome::ResponseCommitReady(navigation) => {
            let navigation = *navigation;
            let configuration = conn.prepared_document_commit_configuration_for_session_owner(
                state.navigate_session_id.as_deref(),
                navigation.final_url(),
            );
            if let Err(error) = navigation.update_commit_configuration(configuration).await {
                push_navigation_commit_error(out, &state, error);
            } else {
                let renderer_page = navigation.renderer_page_residence_identity();
                let candidate = conn.prepare_renderer_agent_candidate_token_for_session_owner(
                    state.navigate_session_id.as_deref(),
                    &token,
                    navigation.renderer_devtools_agent_token(),
                );
                match candidate.and_then(|candidate| {
                    conn.commit_renderer_agent_candidate_for_session_owner(
                        state.navigate_session_id.as_deref(),
                        candidate,
                        renderer_page,
                    )
                }) {
                    Ok(transaction) => {
                        let permit = navigation.issue_commit_permit();
                        match navigation.commit(permit).await {
                            Ok(navigation) => {
                                let navigation = network::materialize_loaded_navigation_progress(
                                    conn, &state, navigation,
                                );
                                commit_loaded_navigation_async(
                                    conn,
                                    out,
                                    Some(&token),
                                    state,
                                    navigation,
                                    Some(transaction),
                                    command_context,
                                )
                                .await;
                            }
                            Err(error) => {
                                if let Err(rollback_error) = conn
                                    .rollback_committed_renderer_agent_candidate_for_session_owner(
                                        state.navigate_session_id.as_deref(),
                                        transaction,
                                    )
                                {
                                    tracing::warn!(
                                        %rollback_error,
                                        session_id = state.navigate_session_id.as_deref(),
                                        "failed to roll back renderer channel after prepared document commit failure"
                                    );
                                }
                                push_navigation_commit_error(out, &state, error);
                            }
                        }
                    }
                    Err(error) => {
                        tracing::debug!(
                            %error,
                            session_id = state.navigate_session_id.as_deref(),
                            loader_id = token.loader_id,
                            "dropping superseded response commit-ready navigation"
                        );
                        push_navigation_commit_error(out, &state, error);
                    }
                }
            }
        }
        network::MaterializedNavigationLoadOutcome::Loaded(navigation) => {
            commit_loaded_navigation_async(
                conn,
                out,
                Some(&token),
                state,
                *navigation,
                None,
                command_context,
            )
            .await;
        }
        network::MaterializedNavigationLoadOutcome::Download(navigation) => {
            let _ = conn.clear_pending_navigation_history_update_for_session_owner(
                state.navigate_session_id.as_deref(),
            );
            commit_download_navigation_async(conn, out, state, navigation, command_context).await;
        }
        network::MaterializedNavigationLoadOutcome::Failed(navigation) => {
            let _ = conn.clear_pending_navigation_history_update_for_session_owner(
                state.navigate_session_id.as_deref(),
            );
            let network::MaterializedFailedDocumentProgress {
                error_text,
                document_policy,
                response_mode,
                progress_gate,
            } = navigation;
            if document_policy.invalidates_committed_document() {
                let _ = conn
                    .discard_loaded_page_after_failed_navigation_for_session_owner_async(
                        state.navigate_session_id.as_deref(),
                        &state.requested_url,
                    )
                    .await;
            }
            activity::MainDocumentFailedNavigationActivity::new(
                state,
                progress_gate,
                response_mode,
            )
            .emit_navigation_error_into_buffer(out, &error_text);
        }
    }
    let primary_protocol_session_id = conn
        .runtime_session_owner_primary_session_id(navigation_session_id.as_deref())
        .or_else(|| navigation_session_id.clone());
    let (routed_renderer_output, renderer_call_replacements) = conn
        .finish_renderer_document_navigation_for_session_owner(
            navigation_session_id.as_deref(),
            &token,
        )
        .map(|finish| (finish.released_output, finish.renderer_call_replacements))
        .unwrap_or_default();
    if !routed_renderer_output.is_empty() {
        let mut background_events = Vec::new();
        crate::domains::runtime::push_routed_renderer_runtime_inspector_message_batch_background_events(
            conn,
            &mut background_events,
            routed_renderer_output,
            primary_protocol_session_id.as_deref(),
        );
        out.extend_background_events_after_messages(background_events);
    }
    if let Some(renderer_call_replacements) = renderer_call_replacements {
        let (new_attachment_id, terminations, replays) = renderer_call_replacements.into_parts();
        conn.terminate_prepared_renderer_calls_after_navigation(
            terminations,
            "Inspected target navigated or closed",
        );
        match conn
            .replay_prepared_renderer_calls_after_navigation_async(replays, new_attachment_id)
            .await
        {
            Ok(events) => out.extend_background_events_after_messages(events),
            Err(error) => tracing::warn!(
                %error,
                session_id = navigation_session_id.as_deref(),
                "failed to replay renderer Inspector commands after navigation"
            ),
        }
    }
    conn.clear_pending_document_navigation_for_session_owner_if_loader_matches(
        navigation_session_id.as_deref(),
        &navigation_loader_id,
    );
}

fn push_navigation_commit_error(
    out: &mut CommandOutputBuffer,
    state: &NavigationDispatchState,
    error: impl Into<String>,
) {
    let error = error.into();
    if state.navigate_id.is_some() {
        out.push_error_after_messages(-32000, error);
    } else {
        tracing::warn!(
            %error,
            session_id = state.navigate_session_id.as_deref(),
            "navigation commit failed after early Page.navigate result"
        );
    }
}

fn emit_same_document_navigation_background_event(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    url: Url,
    navigation_type: &str,
    history_update: SameDocumentHistoryUpdate,
) {
    let Some(frame_id) =
        conn.record_same_document_navigation_for_session_owner(session_id, &url, history_update)
    else {
        return;
    };
    for event_session_id in conn.page_event_session_ids_for_session_owner(session_id) {
        let event = SameDocumentNavigationEvent {
            target_id: DevToolsTargetId::from(frame_id.as_str()),
            frame_id: DevToolsFrameId::from(frame_id.as_str()),
            url: url.as_str().to_owned(),
            navigation_type: navigation_type.to_owned(),
        };
        out.push(BackgroundProtocolEvent::page_same_document_navigation(
            event_session_id.as_deref(),
            event,
        ));
    }
}

pub(crate) async fn emit_same_document_navigation_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    navigations: Vec<super::PagePreparedSameDocumentNavigation>,
) {
    for navigation in navigations {
        let source_document = navigation.source_document();
        if !conn
            .target_page_residence_identity_is_current_for_session(session_id, navigation.owner())
        {
            tracing::debug!(
                session_id,
                ?source_document,
                browser_context_id = navigation.owner().browser_context_id(),
                target_id = navigation.owner().target_id(),
                page_attachment_id = navigation.owner().page_attachment_id().get(),
                "dropping same-document navigation produced by a stale Page residence"
            );
            continue;
        }
        let navigation = navigation.into_navigation();
        let Ok(url) = Url::parse(&navigation.url) else {
            continue;
        };
        if conn.has_pending_document_navigation_for_session_owner(session_id) {
            tracing::debug!(
                session_id,
                url = url.as_str(),
                navigation_type = navigation.navigation_type.as_str(),
                "dropping renderer same-document navigation while cross-document navigation is pending"
            );
            continue;
        }
        emit_same_document_navigation_background_event(
            conn,
            out,
            session_id,
            url,
            &navigation.navigation_type,
            navigation.history_update,
        );
    }
}

#[cfg(test)]
mod child_frame_attachment_tests {
    use crate::devtools_runtime::{
        DevToolsCommand, DevToolsHistoryTraversalDestination, DevToolsNavigationWait,
        DevToolsProtocol,
    };
    use serde_json::{Value, json};

    use crate::conn::{CdpConnection, Cmd};

    use super::{
        DevToolsNavigationStartOptions, NavigationResultProjection, PageCommandTaskStep,
        PagePreparedChildFrameTreeEvent, build_cdp_navigate_command, build_cdp_reload_command,
        build_cdp_traverse_history_command, merge_child_frame_tree_attachments_into_events,
        start_devtools_page_command, update_navigation_result_payload_identity,
    };

    #[test]
    fn navigation_identity_update_uses_typed_protocol_not_payload_keys() {
        let mut bidi = NavigationResultProjection::WebDriverBidi(json!({
            "frameId": "payload-collision",
            "navigation": "old-navigation"
        }));
        update_navigation_result_payload_identity(&mut bidi, "FRAME-new", "LOADER-new");
        assert_eq!(bidi.payload()["frameId"], "payload-collision");
        assert_eq!(bidi.payload()["navigation"], "navigation-LOADER-new");
        assert!(bidi.payload().get("loaderId").is_none());

        let mut cdp = NavigationResultProjection::Cdp(json!({
            "frameId": "old-frame",
            "navigation": "payload-collision"
        }));
        update_navigation_result_payload_identity(&mut cdp, "FRAME-new", "LOADER-new");
        assert_eq!(cdp.payload()["frameId"], "FRAME-new");
        assert_eq!(cdp.payload()["loaderId"], "LOADER-new");
        assert_eq!(cdp.payload()["navigation"], "payload-collision");

        let mut classic = NavigationResultProjection::WebDriverClassic(json!({
            "frameId": "old-frame"
        }));
        update_navigation_result_payload_identity(&mut classic, "FRAME-new", "LOADER-new");
        assert_eq!(classic.payload()["frameId"], "FRAME-new");
        assert_eq!(classic.payload()["loaderId"], "LOADER-new");
        assert!(classic.payload().get("navigation").is_none());

        let mut cdp_reload = NavigationResultProjection::Cdp(json!({}));
        update_navigation_result_payload_identity(&mut cdp_reload, "FRAME-new", "LOADER-new");
        assert_eq!(cdp_reload.payload(), &json!({}));
    }

    #[test]
    fn cdp_navigate_builds_protocol_neutral_navigate_command() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(11),
            "Page.navigate",
            &params,
            Some("SID-1"),
            r#"{"id":11,"method":"Page.navigate"}"#,
        );

        let command = build_cdp_navigate_command(
            &cmd,
            Some("TID-1"),
            "https://example.test/",
            Some("https://referrer.test/"),
        );

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-1")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-1")
        );
        assert_eq!(command.context.browser_context_id, None);
        assert_eq!(command.url, "https://example.test/");
        assert_eq!(command.referrer.as_deref(), Some("https://referrer.test/"));
        assert_eq!(command.wait, DevToolsNavigationWait::DocumentInstalled);
    }

    #[test]
    fn devtools_page_entry_routes_navigate_command_to_navigation_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(12),
            "Page.navigate",
            &params,
            Some("SID-2"),
            r#"{"id":12,"method":"Page.navigate"}"#,
        );
        let command =
            build_cdp_navigate_command(&cmd, Some("TID-2"), "not a valid navigation url", None);

        let step = start_devtools_page_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::Navigate(command),
            DevToolsNavigationStartOptions {
                result_projection: NavigationResultProjection::Cdp(json!({})),
                reloaded_after_crash_session_ids: Vec::new(),
                allow_background_navigation: true,
            },
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("invalid Page.navigate should complete through the unified page entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(12));
        assert_eq!(out[0]["error"]["message"], "Invalid navigation URL");
    }

    #[test]
    fn cdp_reload_builds_protocol_neutral_reload_command() {
        let params = json!({
            "ignoreCache": true,
            "scriptToEvaluateOnLoad": "globalThis.reloaded = true"
        });
        let cmd = Cmd::for_test(
            Some(13),
            "Page.reload",
            &params,
            Some("SID-3"),
            r#"{"id":13,"method":"Page.reload"}"#,
        );
        let params = cmd
            .get_params()
            .expect("reload params should parse")
            .expect("reload params should be present");

        let command = build_cdp_reload_command(&cmd, Some("TID-3"), params);

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-3")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-3")
        );
        assert!(command.ignore_cache);
        assert_eq!(
            command.script_to_evaluate_on_load.as_deref(),
            Some("globalThis.reloaded = true")
        );
        assert_eq!(command.wait, DevToolsNavigationWait::DocumentInstalled);
    }

    #[test]
    fn devtools_page_entry_routes_reload_command_to_navigation_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(14),
            "Page.reload",
            &params,
            Some("SID-4"),
            r#"{"id":14,"method":"Page.reload"}"#,
        );
        let command = build_cdp_reload_command(&cmd, Some("TID-4"), Default::default());

        let step = start_devtools_page_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::Reload(command),
            DevToolsNavigationStartOptions {
                result_projection: NavigationResultProjection::Cdp(json!({})),
                reloaded_after_crash_session_ids: Vec::new(),
                allow_background_navigation: true,
            },
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("missing Page.reload target should complete through the unified page entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(14));
        assert_eq!(out[0]["error"]["message"], "TargetNotLoaded");
    }

    #[test]
    fn cdp_history_traversal_builds_protocol_neutral_command() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(15),
            "Page.navigateToHistoryEntry",
            &params,
            Some("SID-5"),
            r#"{"id":15,"method":"Page.navigateToHistoryEntry"}"#,
        );

        let command = build_cdp_traverse_history_command(
            &cmd,
            Some("TID-5"),
            7,
            "https://example.test/history".to_owned(),
        );

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-5")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-5")
        );
        assert_eq!(
            command.destination,
            DevToolsHistoryTraversalDestination::Entry {
                entry_id: 7,
                url: "https://example.test/history".to_owned(),
            }
        );
        assert_eq!(command.wait, DevToolsNavigationWait::DocumentInstalled);
    }

    #[test]
    fn devtools_page_entry_routes_history_traversal_to_navigation_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(16),
            "Page.navigateToHistoryEntry",
            &params,
            Some("SID-6"),
            r#"{"id":16,"method":"Page.navigateToHistoryEntry"}"#,
        );
        let command = build_cdp_traverse_history_command(
            &cmd,
            Some("TID-6"),
            9,
            "https://example.test/history".to_owned(),
        );

        let step = start_devtools_page_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::TraverseHistory(command),
            DevToolsNavigationStartOptions {
                result_projection: NavigationResultProjection::Cdp(json!({})),
                reloaded_after_crash_session_ids: Vec::new(),
                allow_background_navigation: true,
            },
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!(
                "missing Page.navigateToHistoryEntry target should complete through the unified page entry"
            );
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(16));
        assert_eq!(out[0]["error"]["message"], "TargetNotLoaded");
    }

    #[test]
    fn frame_tree_attachments_keep_existing_root_parent_reference() {
        let mut events = vec![PagePreparedChildFrameTreeEvent::Attached {
            frame_id: "outer".to_owned(),
            parent_frame_id: "root".to_owned(),
        }];
        merge_child_frame_tree_attachments_into_events(
            &mut events,
            vec![
                ("outer".to_owned(), "root".to_owned()),
                ("inner".to_owned(), "outer".to_owned()),
            ],
        );
        assert_eq!(
            events,
            vec![
                PagePreparedChildFrameTreeEvent::Attached {
                    frame_id: "outer".to_owned(),
                    parent_frame_id: "root".to_owned(),
                },
                PagePreparedChildFrameTreeEvent::Attached {
                    frame_id: "inner".to_owned(),
                    parent_frame_id: "outer".to_owned(),
                },
            ]
        );

        let mut events = vec![PagePreparedChildFrameTreeEvent::Attached {
            frame_id: "inner".to_owned(),
            parent_frame_id: "root".to_owned(),
        }];
        merge_child_frame_tree_attachments_into_events(
            &mut events,
            vec![("inner".to_owned(), "outer".to_owned())],
        );
        assert_eq!(
            events,
            vec![PagePreparedChildFrameTreeEvent::Attached {
                frame_id: "inner".to_owned(),
                parent_frame_id: "outer".to_owned(),
            }]
        );
    }
}
