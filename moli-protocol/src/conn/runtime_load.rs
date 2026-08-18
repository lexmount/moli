use data_url::DataUrl;
use moli_core::{
    RendererOutputFence,
    page::{RendererMainDocumentCommit, RendererPageCreationDiagnostics, RendererRuntimeRealmInfo},
    runtime::{
        CommittedDocumentResourceSource, PageVmInitStage, PreparedDocumentPage,
        PreparedDocumentPageCommitConfiguration, PreparedDocumentPageCommitPermit,
        RendererPageReservationToken, RendererReplyBoundary,
    },
};
use moli_fetch::{
    BrowserNavigationRequestKind, FetchCancelHandle, FetchConfig, NetworkFetchFailureContext,
    NetworkFetchResult, NetworkObservationJournal, RawResponse, Request, ResponseHead,
    StreamingRawResponse, url_pattern_matches,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::*;
use crate::conn::state::{
    InitialDocumentPageBuildWaiter, RendererPageResidenceIdentity, TargetPageAbsenceReason,
};
use crate::domains::network::{
    CompletedDocumentProgressTransfer, CompletedDownloadProgressTransfer,
    CompletedMainDocumentNetworkEvents, MainDocumentBodyNetworkProgress,
    MainDocumentBodyProgressSource,
};

const BLOCKED_BY_CLIENT_ERROR_TEXT: &str = "net::ERR_BLOCKED_BY_CLIENT";
const HTTP_RESPONSE_CODE_FAILURE_ERROR_TEXT: &str = "net::ERR_HTTP_RESPONSE_CODE_FAILURE";
const CAPTURED_RAW_REPLAY_CHUNK_SIZE: usize = 64 * 1024;

fn apply_navigation_request_load_policy(
    load_inputs: TargetNavigationLoadInputs,
    policy: NavigationRequestLoadPolicy,
) -> TargetNavigationLoadInputs {
    match policy {
        NavigationRequestLoadPolicy::DocumentInitiated => load_inputs,
        NavigationRequestLoadPolicy::BrowserInitiated => load_inputs.without_inferred_referrer(),
        NavigationRequestLoadPolicy::Reload => {
            load_inputs.with_browser_navigation_kind(BrowserNavigationRequestKind::Reload)
        }
    }
}
const EXTERNAL_RAW_BODY_CHANNEL_CAPACITY: usize = 8;
const ABOUT_BLANK_DOCUMENT_HTML: &str = "<!doctype html><html><head></head><body></body></html>";

fn escape_error_page_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn network_error_page_html(unreachable_url: &Url, error_text: &str) -> String {
    let title = unreachable_url
        .host_str()
        .unwrap_or(unreachable_url.as_str());
    let title = escape_error_page_html(title);
    let url = escape_error_page_html(unreachable_url.as_str());
    let error_text = escape_error_page_html(error_text);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><main><h1>This site can’t be reached</h1><p>The webpage at <strong>{url}</strong> could not be loaded.</p><div>{error_text}</div></main></body></html>"
    )
}

fn http_error_page_html(unreachable_url: &Url, status: u16) -> String {
    let title = unreachable_url
        .host_str()
        .unwrap_or(unreachable_url.as_str());
    let title = escape_error_page_html(title);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><main><h1>This page isn't working</h1><p>If the problem continues, contact the site owner.</p><p>HTTP ERROR {status}</p></main></body></html>"
    )
}

fn response_status_may_use_http_error_page(status: u16) -> bool {
    (400..600).contains(&status)
}

#[allow(clippy::too_many_arguments)]
async fn prepare_browser_owned_error_page_navigation_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    unreachable_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    error_text: String,
    body: CapturedBody,
    reply_boundary: RendererReplyBoundary,
) -> Result<ResponseCommitReady, String> {
    let error_page_url = Url::parse(NETWORK_ERROR_PAGE_URL)
        .expect("the browser-owned network error page URL must be valid");
    let error_page = NetworkErrorPageNavigation::new(error_text, unreachable_url.clone());
    let head = ResponseHead {
        final_url: error_page_url,
        status: 200,
        headers: vec![(
            "content-type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        )],
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };
    prepare_captured_document_response_with_engine_async(
        engine,
        page_reservation,
        load_inputs,
        unreachable_url.clone(),
        request_method,
        request_headers,
        head,
        body,
        MainDocumentBodyProgressSource::default(),
        NetworkObservationJournal::default(),
        Some(error_page),
        true,
        reply_boundary,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn prepare_network_error_page_navigation_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    unreachable_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    error_text: String,
    reply_boundary: RendererReplyBoundary,
) -> Result<NavigationLoadOutcome, String> {
    let body = CapturedBody::from_string(network_error_page_html(&unreachable_url, &error_text));
    prepare_browser_owned_error_page_navigation_with_engine_async(
        engine,
        page_reservation,
        load_inputs,
        unreachable_url,
        request_method,
        request_headers,
        error_text,
        body,
        reply_boundary,
    )
    .await
    .map(NavigationLoadOutcome::response_commit_ready)
}

fn response_headers_indicate_xml_document(headers: &[(String, String)]) -> bool {
    moli_web_mime::response_document_content_type(headers)
        .is_some_and(|mime| moli_web_mime::is_dom_parser_xml_mime(&mime))
}

#[derive(Clone, Debug)]
pub(crate) struct InitialDocumentPageOwner {
    pub(crate) browser_context_id: String,
    pub(crate) target_id: String,
}

pub(crate) struct PendingInitialDocumentPageBuild {
    kind: PendingInitialDocumentPageBuildKind,
}

enum PendingInitialDocumentPageBuildKind {
    Build {
        owner: InitialDocumentPageOwner,
        load_inputs: Box<TargetNavigationLoadInputs>,
        override_mode: NavigationLoadInputOverrideMode,
        engine: Box<Option<NavigationEngine>>,
        pending: moli_core::runtime::PendingBuiltDocumentPage,
    },
    Join {
        waiter: InitialDocumentPageBuildWaiter,
    },
}

pub(crate) enum CompletedInitialDocumentPageBuild {
    Built {
        owner: InitialDocumentPageOwner,
        load_inputs: Box<TargetNavigationLoadInputs>,
        override_mode: NavigationLoadInputOverrideMode,
        engine: Box<Option<NavigationEngine>>,
        built: Box<moli_core::runtime::BuiltDocumentPage>,
    },
    Joined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitialDocumentPageInstallResult {
    Installed,
    Stale,
}

#[derive(Debug)]
pub(crate) enum FailedInitialDocumentPageBuild {
    Build {
        owner: InitialDocumentPageOwner,
        message: String,
    },
    Joined {
        message: String,
    },
}

impl FailedInitialDocumentPageBuild {
    fn message(&self) -> &str {
        match self {
            Self::Build { message, .. } | Self::Joined { message } => message,
        }
    }

    fn into_message(self) -> String {
        match self {
            Self::Build { message, .. } | Self::Joined { message } => message,
        }
    }

    fn build_owner(&self) -> Option<&InitialDocumentPageOwner> {
        match self {
            Self::Build { owner, .. } => Some(owner),
            Self::Joined { .. } => None,
        }
    }
}

impl std::fmt::Display for FailedInitialDocumentPageBuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl PendingInitialDocumentPageBuild {
    pub async fn wait(
        self,
    ) -> Result<CompletedInitialDocumentPageBuild, FailedInitialDocumentPageBuild> {
        match self.kind {
            PendingInitialDocumentPageBuildKind::Build {
                owner,
                load_inputs,
                override_mode,
                engine,
                pending,
            } => {
                let built = match pending.await_ready().await {
                    Ok(built) => built,
                    Err(error) => {
                        return Err(FailedInitialDocumentPageBuild::Build {
                            owner,
                            message: format!("initial document page build failed: {error}"),
                        });
                    }
                };
                Ok(CompletedInitialDocumentPageBuild::Built {
                    owner,
                    load_inputs,
                    override_mode,
                    engine,
                    built: Box::new(built),
                })
            }
            PendingInitialDocumentPageBuildKind::Join { waiter } => match waiter.wait().await {
                Ok(()) => Ok(CompletedInitialDocumentPageBuild::Joined),
                Err(message) => Err(FailedInitialDocumentPageBuild::Joined { message }),
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct LoadedPageCreationDiagnosticsParts {
    pub(crate) initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    pub(crate) renderer_output_predecessor: Option<RendererOutputFence>,
}

fn loaded_page_creation_diagnostics_parts(
    diagnostics: RendererPageCreationDiagnostics,
) -> LoadedPageCreationDiagnosticsParts {
    LoadedPageCreationDiagnosticsParts {
        initial_runtime_realms: diagnostics.initial_runtime_realms,
        renderer_output_predecessor: diagnostics.renderer_output_predecessor,
    }
}

pub struct ResponseCommitReady {
    prepared_page: Option<PreparedDocumentPage>,
    body_capture: Option<ResponseCommitBodyCapture>,
    body_completion_sink: Option<BackgroundNavigationBodyCompletionSink>,
    body_progress_source: MainDocumentBodyProgressSource,
    body_network_progress_state: Option<MainDocumentBodyNetworkProgress>,
    synthetic_body: bool,
    requested_url: Url,
    final_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    response_from_cache: bool,
    navigation_engine: Option<NavigationEngine>,
    timing_started: Option<std::time::Instant>,
    main_document_commit: Option<Arc<RendererMainDocumentCommit>>,
    network_error_page: Option<NetworkErrorPageNavigation>,
}

enum ResponseCommitBodyCapture {
    Pending(tokio::task::JoinHandle<Result<CapturedBody, String>>),
    Ready(CapturedBody),
}

impl ResponseCommitBodyCapture {
    fn abort(self) {
        if let Self::Pending(task) = self {
            task.abort();
        }
    }

    async fn resolve(self) -> Result<CapturedBody, String> {
        match self {
            Self::Pending(task) => task
                .await
                .map_err(|error| format!("main document body capture task failed: {error}"))?,
            Self::Ready(body) => Ok(body),
        }
    }
}

impl std::fmt::Debug for ResponseCommitReady {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponseCommitReady")
            .field("requested_url", &self.requested_url)
            .field("final_url", &self.final_url)
            .field(
                "renderer_devtools_agent_token",
                &self
                    .prepared_page
                    .as_ref()
                    .map(PreparedDocumentPage::renderer_devtools_agent_token),
            )
            .finish_non_exhaustive()
    }
}

impl ResponseCommitReady {
    pub(crate) fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub(crate) fn renderer_devtools_agent_token(
        &self,
    ) -> moli_core::page::RendererDevToolsAgentToken {
        self.prepared_page
            .as_ref()
            .expect("response commit-ready value must retain its prepared Page")
            .renderer_devtools_agent_token()
    }

    pub(crate) fn renderer_page_residence_identity(&self) -> RendererPageResidenceIdentity {
        let prepared_page = self
            .prepared_page
            .as_ref()
            .expect("response commit-ready value must retain its prepared Page");
        RendererPageResidenceIdentity::from_parts(
            prepared_page.renderer_owner_local_host_id(),
            prepared_page.renderer_page_id(),
        )
    }

    pub(crate) async fn update_commit_configuration(
        &self,
        configuration: PreparedDocumentPageCommitConfiguration,
    ) -> Result<(), String> {
        self.prepared_page
            .as_ref()
            .expect("response commit-ready value must retain its prepared Page")
            .update_commit_configuration(configuration)
            .await
            .map_err(|error| {
                format!(
                    "failed to attach commit-time target configuration for page `{}`: {error:#}",
                    self.requested_url
                )
            })
    }

    pub(crate) fn issue_commit_permit(&self) -> PreparedDocumentPageCommitPermit {
        self.prepared_page
            .as_ref()
            .expect("response commit-ready value must retain its prepared Page")
            .issue_commit_permit()
    }

    pub(crate) fn with_navigation_engine(mut self, engine: NavigationEngine) -> Self {
        self.navigation_engine = Some(engine);
        self
    }

    pub(crate) async fn commit(
        mut self,
        permit: PreparedDocumentPageCommitPermit,
    ) -> Result<LoadedNavigation, String> {
        let prepared_page = self
            .prepared_page
            .take()
            .expect("response commit-ready value must retain its prepared Page");
        let built = match prepared_page.commit(permit).await {
            Ok(built) => built,
            Err(error) => {
                if let Some(body_capture) = self.body_capture.take() {
                    body_capture.abort();
                }
                return Err(format!(
                    "failed to execute scripts for page `{}`: {error:#}",
                    self.requested_url
                ));
            }
        };
        let body_capture = self
            .body_capture
            .take()
            .expect("response commit-ready value must retain its body capture");
        let body_network_progress_state = self
            .body_network_progress_state
            .take()
            .expect("response commit-ready value must retain body network progress");
        let document_progress_transfer = if let Some(sink) = self.body_completion_sink.take() {
            let timing_started = self.timing_started;
            let timing_enabled = timing_started.is_some();
            let body_timing_url = self.requested_url.to_string();
            let body_progress_source = self.body_progress_source.clone();
            let final_url = self.final_url.clone();
            let response_headers = self.response_headers.clone();
            let response_from_cache = self.response_from_cache;
            tokio::task::spawn_local(async move {
                let body = body_capture.resolve().await;
                if timing_enabled {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url = %body_timing_url,
                        stage = "body_capture_ready",
                        elapsed_ms = timing_started
                            .map(|started| started.elapsed().as_millis())
                            .unwrap_or_default(),
                    );
                }
                sink.send(
                    body,
                    body_progress_source,
                    final_url,
                    response_headers,
                    response_from_cache,
                );
            });
            CompletedDocumentProgressTransfer::new_pending_body(body_network_progress_state)
        } else {
            let captured_body = body_capture.resolve().await.map_err(|error| {
                format!(
                    "failed to execute scripts for page `{}`: {error}",
                    self.requested_url
                )
            })?;
            self.body_progress_source
                .emit_body_finished(captured_body.len());
            CompletedDocumentProgressTransfer::new_captured(
                captured_body,
                self.synthetic_body,
                body_network_progress_state,
            )
        };
        let diagnostics = loaded_page_creation_diagnostics_parts(built.page_creation_diagnostics);
        Ok(LoadedNavigation {
            page: built.page,
            pending_download: built.pending_download,
            page_creation_artifacts: built.page_creation_artifacts,
            requested_url: self.requested_url.clone(),
            final_url: self.final_url.clone(),
            request_method: self.request_method.clone(),
            request_headers: std::mem::take(&mut self.request_headers),
            response_status: self.response_status,
            response_headers: std::mem::take(&mut self.response_headers),
            response_from_cache: self.response_from_cache,
            initial_runtime_realms: diagnostics.initial_runtime_realms,
            renderer_output_predecessor: diagnostics.renderer_output_predecessor,
            main_document_commit: self.main_document_commit.take(),
            document_progress_transfer,
            navigation_engine: self.navigation_engine.take(),
            network_error_page: self.network_error_page.take(),
        })
    }
}

impl Drop for ResponseCommitReady {
    fn drop(&mut self) {
        if let Some(body_capture) = self.body_capture.take() {
            body_capture.abort();
        }
    }
}

pub struct PausedResponsePreparedDocument {
    prepared_page: PreparedDocumentPage,
    renderer_body_tx: mpsc::Sender<Vec<u8>>,
    renderer_completion_tx: oneshot::Sender<anyhow::Result<()>>,
    body_progress_source: MainDocumentBodyProgressSource,
    body_network_progress_state: MainDocumentBodyNetworkProgress,
    requested_url: Url,
    final_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    response_from_cache: bool,
    negotiated_http_version: Option<moli_fetch::NegotiatedHttpVersion>,
    network_observation_journal: NetworkObservationJournal,
    engine: NavigationEngine,
    timing_started: Option<std::time::Instant>,
    main_document_commit: Option<Arc<RendererMainDocumentCommit>>,
}

impl std::fmt::Debug for PausedResponsePreparedDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PausedResponsePreparedDocument")
            .field("requested_url", &self.requested_url)
            .field("final_url", &self.final_url)
            .field(
                "renderer_devtools_agent_token",
                &self.prepared_page.renderer_devtools_agent_token(),
            )
            .finish_non_exhaustive()
    }
}

impl PausedResponsePreparedDocument {
    #[cfg(test)]
    pub(crate) fn renderer_devtools_agent_token(
        &self,
    ) -> moli_core::page::RendererDevToolsAgentToken {
        self.prepared_page.renderer_devtools_agent_token()
    }

    pub(crate) fn resume_streaming(
        self,
        response: StreamingRawResponse,
        body_completion_sink: Option<BackgroundNavigationBodyCompletionSink>,
    ) -> (NavigationEngine, NavigationLoadOutcome) {
        let network_extra_info_available = !self.network_observation_journal.is_empty();
        self.body_progress_source.emit_response_metadata(
            &self.request_method,
            &self.request_headers,
            response.request_cookie_report.as_ref(),
            &response.redirect_chain,
            &self.final_url,
            self.response_status,
            &self.response_headers,
            &response.cookie_set_reports,
            &self.network_observation_journal,
            network_extra_info_available,
            self.response_from_cache,
            self.negotiated_http_version,
        );
        let body_capture_task = spawn_streaming_body_capture(
            response,
            None,
            self.renderer_body_tx,
            self.renderer_completion_tx,
        );
        let ready = ResponseCommitReady {
            prepared_page: Some(self.prepared_page),
            body_capture: Some(ResponseCommitBodyCapture::Pending(body_capture_task)),
            body_completion_sink,
            body_progress_source: self.body_progress_source,
            body_network_progress_state: Some(self.body_network_progress_state),
            synthetic_body: false,
            requested_url: self.requested_url,
            final_url: self.final_url,
            request_method: self.request_method,
            request_headers: self.request_headers,
            response_status: self.response_status,
            response_headers: self.response_headers,
            response_from_cache: self.response_from_cache,
            navigation_engine: None,
            timing_started: self.timing_started,
            main_document_commit: self.main_document_commit,
            network_error_page: None,
        };
        (
            self.engine,
            NavigationLoadOutcome::response_commit_ready(ready),
        )
    }
}

async fn first_nonempty_response_body_chunk(
    response: &mut StreamingRawResponse,
) -> Result<Option<Vec<u8>>, String> {
    loop {
        match response.next_chunk().await {
            Some(chunk) if chunk.is_empty() => continue,
            Some(chunk) => return Ok(Some(chunk)),
            None => {
                response
                    .finish()
                    .await
                    .map_err(|error| format!("failed to read page body from stream: {error:#}"))?;
                return Ok(None);
            }
        }
    }
}

fn spawn_streaming_body_capture(
    mut response: StreamingRawResponse,
    initial_chunk: Option<Vec<u8>>,
    body_tx: mpsc::Sender<Vec<u8>>,
    completion_tx: oneshot::Sender<anyhow::Result<()>>,
) -> tokio::task::JoinHandle<Result<CapturedBody, String>> {
    tokio::spawn(async move {
        let mut body = CapturedBodyWriter::default();
        let mut renderer_body_tx = Some(body_tx);
        if let Some(chunk) = initial_chunk {
            body.append(&chunk)
                .map_err(|error| format!("failed to capture page body: {error}"))?;
            if let Some(body_tx) = renderer_body_tx.as_ref()
                && body_tx.send(chunk).await.is_err()
            {
                renderer_body_tx = None;
            }
        }
        while let Some(chunk) = response.next_chunk().await {
            body.append(&chunk)
                .map_err(|error| format!("failed to capture page body: {error}"))?;
            if let Some(body_tx) = renderer_body_tx.as_ref()
                && body_tx.send(chunk).await.is_err()
            {
                renderer_body_tx = None;
            }
        }
        let finish_result = response
            .finish()
            .await
            .map_err(|error| format!("failed to read page body from stream: {error:#}"));
        let completion_result = finish_result
            .as_ref()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!(error.clone()));
        let _ = completion_tx.send(completion_result);
        finish_result?;
        body.finish()
            .map_err(|error| format!("failed to finish captured page body: {error}"))
    })
}

fn spawn_captured_body_replay(
    body: CapturedBody,
    body_tx: mpsc::Sender<Vec<u8>>,
    completion_tx: oneshot::Sender<anyhow::Result<()>>,
) -> tokio::task::JoinHandle<Result<CapturedBody, String>> {
    tokio::spawn(async move {
        let replay_result = async {
            let mut reader = body
                .chunk_reader(CAPTURED_RAW_REPLAY_CHUNK_SIZE)
                .map_err(|error| error.to_string())?;
            let mut renderer_body_tx = Some(body_tx);
            while let Some(chunk) = reader.next_chunk().map_err(|error| error.to_string())? {
                if let Some(body_tx) = renderer_body_tx.as_ref()
                    && body_tx.send(chunk).await.is_err()
                {
                    renderer_body_tx = None;
                }
            }
            Ok::<(), String>(())
        }
        .await;
        let completion_result = replay_result
            .as_ref()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!(error.clone()));
        let _ = completion_tx.send(completion_result);
        replay_result?;
        Ok(body)
    })
}

pub(crate) struct BackgroundNavigationLoadJob {
    engine: NavigationEngine,
    page_reservation: RendererPageReservationToken,
    cancellation: FetchCancelHandle,
    early_result: Option<BackgroundNavigationEarlyResult>,
    load_inputs: TargetNavigationLoadInputs,
    method: String,
    raw_url: String,
    body: Option<Vec<u8>>,
    request_headers: Vec<(String, String)>,
    body_progress_source: MainDocumentBodyProgressSource,
    /// Browser-context transport/cache state shared with the resident engine.
    ///
    /// The detached job must not retain a source Page's request-policy view;
    /// its own NavigationEngine supplies the target Page policy.
    shared_resource_runtime: Option<moli_core::network::BrowserResourceRuntime>,
}

pub(crate) struct BackgroundStreamingResponseNavigationLoadJob {
    engine: NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: TargetNavigationLoadInputs,
    requested_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    response: StreamingRawResponse,
    network_observation_journal: NetworkObservationJournal,
    response_code: Option<u16>,
    response_headers_override: Vec<(String, String)>,
    body_progress_source: MainDocumentBodyProgressSource,
    shared_resource_runtime: Option<moli_core::network::BrowserResourceRuntime>,
}

pub(crate) struct BackgroundNavigationEarlyResult {
    sender: BackgroundEventSender,
    navigate_id: u64,
    session_id: Option<String>,
    result_payload: Value,
}

pub(crate) struct BackgroundNavigationBodyCompletionSink {
    sender:
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    none_session_owner_route: Option<CdpSessionRoute>,
}

impl BackgroundNavigationBodyCompletionSink {
    pub(crate) fn new(
        sender: tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundNavigationCompletion,
        >,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        none_session_owner_route: Option<CdpSessionRoute>,
    ) -> Self {
        Self {
            sender,
            token,
            state,
            none_session_owner_route,
        }
    }

    fn send(
        self,
        body: Result<CapturedBody, String>,
        body_progress_source: MainDocumentBodyProgressSource,
        final_url: Url,
        response_headers: Vec<(String, String)>,
        response_from_cache: bool,
    ) {
        let _ = self.sender.send(
            crate::domains::page::BackgroundNavigationCompletion::main_document_body(
                self.token,
                self.state,
                self.none_session_owner_route,
                body,
                false,
                body_progress_source,
                final_url,
                response_headers,
                response_from_cache,
            ),
        );
    }
}

impl BackgroundNavigationEarlyResult {
    pub(crate) fn new(
        sender: BackgroundEventSender,
        navigate_id: u64,
        session_id: Option<String>,
        result_payload: Value,
    ) -> Self {
        Self {
            sender,
            navigate_id,
            session_id,
            result_payload,
        }
    }

    fn emit(self) -> bool {
        let session_id = self.session_id;
        self.sender
            .send(BackgroundProtocolEvent::command_success(
                Some(self.navigate_id),
                session_id.as_deref(),
                self.result_payload,
            ))
            .is_ok()
    }
}

impl BackgroundNavigationLoadJob {
    fn emit_early_result_for_successful_document(
        early_result: &mut Option<BackgroundNavigationEarlyResult>,
        navigation: &Result<NavigationLoadOutcome, String>,
    ) -> bool {
        let is_successful_document = match navigation {
            Ok(NavigationLoadOutcome::ResponseCommitReady(navigation)) => {
                navigation.network_error_page.is_none()
            }
            Ok(NavigationLoadOutcome::Loaded(navigation)) => {
                navigation.network_error_page.is_none()
            }
            _ => false,
        };
        if !is_successful_document {
            return false;
        }
        early_result
            .take()
            .is_some_and(BackgroundNavigationEarlyResult::emit)
    }

    pub(crate) async fn run(
        mut self,
        body_completion_sink: Option<BackgroundNavigationBodyCompletionSink>,
    ) -> (
        NavigationEngine,
        Result<NavigationLoadOutcome, String>,
        bool,
    ) {
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let timing_url = self.raw_url.clone();
        let mut engine = self.engine;
        let mut early_result = self.early_result.take();
        if let Some(resource_runtime) = self.shared_resource_runtime.take()
            && let Err(error) = engine.adopt_registered_resource_runtime(resource_runtime)
        {
            return (engine, Err(error.to_string()), false);
        }
        let mut early_result_sent = false;
        let navigation = async {
            if let Some(navigation) = load_inline_html_navigation_with_engine_async(
                &mut engine,
                self.page_reservation,
                &self.load_inputs,
                &self.method,
                &self.raw_url,
                self.request_headers.clone(),
                RendererReplyBoundary::DocumentCommit,
            )
            .await
            {
                early_result_sent =
                    Self::emit_early_result_for_successful_document(&mut early_result, &navigation);
                return navigation;
            }

            if let Some(navigation) = load_data_url_navigation_with_engine_async(
                &mut engine,
                self.page_reservation,
                &self.load_inputs,
                &self.method,
                &self.raw_url,
                self.request_headers.clone(),
                RendererReplyBoundary::DocumentCommit,
            )
            .await
            {
                early_result_sent =
                    Self::emit_early_result_for_successful_document(&mut early_result, &navigation);
                return navigation;
            }

            ensure_url_not_blocked_for_load_inputs(&self.load_inputs, &self.raw_url)?;
            if self.load_inputs.network_offline {
                return Err("Network emulation offline".to_owned());
            }

            let requested_url = Url::parse(&self.raw_url).map_err(|error| {
                format!("failed to parse request url `{}`: {error}", self.raw_url)
            })?;
            let resource_storage = self.load_inputs.resource_storage_handles();
            let navigation_response = engine
                .fetch_navigation_streaming_raw_response_bytes_with_storage_async(
                    resource_storage.into_navigation_storage(),
                    self.load_inputs.navigation_initiator_url.as_ref(),
                    self.load_inputs.browser_navigation_kind,
                    self.load_inputs.infer_navigation_referrer,
                    &self.method,
                    &self.raw_url,
                    self.body,
                    self.request_headers.clone(),
                    None,
                    self.cancellation.clone(),
                )
                .await;
            let navigation_response = match navigation_response {
                Ok(response) => response,
                Err(error) => {
                    if let Some(failure) = error.downcast_ref::<NetworkFetchFailureContext>() {
                        let unreachable_url = failure
                            .request_context()
                            .map(|request_context| request_context.current_url().clone())
                            .unwrap_or_else(|| requested_url.clone());
                        if let Some(request_context) = failure.request_context() {
                            self.body_progress_source.emit_failed_request_progress(
                                request_context.request_method(),
                                request_context
                                    .request_body()
                                    .and_then(|body| std::str::from_utf8(body).ok()),
                                request_context.request_headers(),
                                request_context.redirect_chain(),
                                failure.observation_journal(),
                            );
                        } else {
                            self.body_progress_source
                                .emit_failed_initial_request_extra_info(
                                    failure.observation_journal(),
                                );
                        }
                        tracing::debug!(
                            url = %self.raw_url,
                            error = ?error,
                            network_error_text = failure.network_error_text(),
                            "main document transport failed before response metadata"
                        );
                        return prepare_network_error_page_navigation_with_engine_async(
                            &mut engine,
                            self.page_reservation,
                            &self.load_inputs,
                            unreachable_url,
                            self.method,
                            self.request_headers,
                            failure.network_error_text().to_owned(),
                            RendererReplyBoundary::DocumentCommit,
                        )
                        .await;
                    }
                    return Err(format!("failed to fetch page `{}`: {error}", self.raw_url));
                }
            };
            let (response, network_observation_journal) = navigation_response
                .fetch_result
                .into_parts_with_observation_journal();
            let reserved_service_worker_client = navigation_response.reserved_service_worker_client;
            let document_fetch_context_seed = navigation_response.document_fetch_context_seed;
            let defer_early_result_for_http_error_body =
                response_status_may_use_http_error_page(response.status);
            if !super::downloads::response_headers_indicate_download(&response.headers)
                && !defer_early_result_for_http_error_body
                && let Some(early_result) = early_result.take()
            {
                early_result_sent = early_result.emit();
            }
            let navigation = build_navigation_from_streaming_raw_response_with_engine_async(
                &mut engine,
                self.page_reservation,
                &self.load_inputs,
                requested_url,
                self.method,
                self.request_headers,
                response,
                network_observation_journal,
                None,
                Vec::new(),
                self.body_progress_source,
                body_completion_sink,
                reserved_service_worker_client,
                CommittedDocumentResourceSource::Navigation(Box::new(document_fetch_context_seed)),
                RendererReplyBoundary::DocumentCommit,
            )
            .await;
            if defer_early_result_for_http_error_body {
                early_result_sent =
                    Self::emit_early_result_for_successful_document(&mut early_result, &navigation);
            }
            navigation
        }
        .await;
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %timing_url,
                stage = "background_navigation_job_done",
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        (engine, navigation, early_result_sent)
    }
}

impl BackgroundStreamingResponseNavigationLoadJob {
    pub(crate) async fn run(
        mut self,
        body_completion_sink: Option<BackgroundNavigationBodyCompletionSink>,
    ) -> (NavigationEngine, Result<NavigationLoadOutcome, String>) {
        let mut engine = self.engine;
        if let Some(resource_runtime) = self.shared_resource_runtime.take()
            && let Err(error) = engine.adopt_registered_resource_runtime(resource_runtime)
        {
            return (engine, Err(error.to_string()));
        }
        let navigation = build_navigation_from_streaming_raw_response_with_engine_async(
            &mut engine,
            self.page_reservation,
            &self.load_inputs,
            self.requested_url,
            self.request_method,
            self.request_headers,
            self.response,
            self.network_observation_journal,
            self.response_code,
            self.response_headers_override,
            self.body_progress_source,
            body_completion_sink,
            None,
            CommittedDocumentResourceSource::Synthetic,
            RendererReplyBoundary::DocumentCommit,
        )
        .await;
        (engine, navigation)
    }
}

#[cfg(test)]
pub(crate) fn decode_data_url_body(raw_url: &str) -> Option<Result<Vec<u8>, String>> {
    decode_data_url_response(raw_url).map(|result| result.map(|response| response.body))
}

pub(crate) struct DecodedDataUrlResponse {
    pub content_type: String,
    pub body: Vec<u8>,
}

struct DecodedDataUrlNavigationResponse {
    requested_url: Url,
    response: RawResponse,
}

struct InlineHtmlNavigationSource {
    document_url: Url,
    html: String,
    response_headers: Vec<(String, String)>,
}

pub(crate) fn decode_data_url_response(
    raw_url: &str,
) -> Option<Result<DecodedDataUrlResponse, String>> {
    let data_url = DataUrl::process(raw_url).ok()?;
    let content_type = data_url.mime_type().to_string();
    Some(
        data_url
            .decode_to_vec()
            .map(|(body, _fragment)| DecodedDataUrlResponse { content_type, body })
            .map_err(|_| "failed to decode data url body".to_owned()),
    )
}

fn decoded_data_url_navigation_response(
    raw_url: &str,
) -> Option<Result<DecodedDataUrlNavigationResponse, String>> {
    let decoded = decode_data_url_response(raw_url)?;
    Some(decoded.and_then(|decoded| {
        let requested_url =
            Url::parse(raw_url).map_err(|error| format!("failed to parse data url: {error}"))?;
        let response = RawResponse::from_head_and_body(
            ResponseHead {
                final_url: requested_url.clone(),
                status: 200,
                headers: vec![("Content-Type".to_owned(), decoded.content_type)],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            decoded.body,
        );
        Ok(DecodedDataUrlNavigationResponse {
            requested_url,
            response,
        })
    }))
}

pub(crate) fn decode_text_html_data_url(raw_url: &str) -> Option<Result<String, String>> {
    if let Some(payload) = raw_url.strip_prefix("data:text/html,")
        && payload.contains('#')
    {
        // CDP/Page tests and callers historically pass raw inline HTML here.
        // Preserve unescaped fragment markers inside that legacy HTML payload.
        return Some(Ok(payload.to_owned()));
    }

    let data_url = DataUrl::process(raw_url).ok()?;
    if !data_url.mime_type().matches("text", "html") {
        return None;
    }
    Some(
        data_url
            .decode_to_vec()
            .map_err(|_| "failed to decode text/html data url body".to_owned())
            .and_then(|(body, _fragment)| {
                String::from_utf8(body).map_err(|error| error.to_string())
            }),
    )
}

fn inline_html_navigation_source(
    raw_url: &str,
) -> Option<Result<InlineHtmlNavigationSource, String>> {
    if raw_url == "about:blank" {
        return Some(
            Url::parse(raw_url)
                .map(|document_url| InlineHtmlNavigationSource {
                    document_url,
                    html: ABOUT_BLANK_DOCUMENT_HTML.to_owned(),
                    response_headers: vec![("content-type".into(), "text/html".into())],
                })
                .map_err(|error| format!("failed to parse about:blank url: {error}")),
        );
    }

    let html = decode_text_html_data_url(raw_url)?;
    Some(html.and_then(|html| {
        let content_type = DataUrl::process(raw_url)
            .ok()
            .map(|data_url| data_url.mime_type().to_string())
            .unwrap_or_else(|| "text/html".to_owned());
        Url::parse(raw_url)
            .map(|document_url| InlineHtmlNavigationSource {
                document_url,
                html,
                response_headers: vec![("Content-Type".into(), content_type)],
            })
            .map_err(|error| format!("failed to parse data url: {error}"))
    }))
}

async fn load_inline_html_navigation_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    method: &str,
    raw_url: &str,
    request_headers: Vec<(String, String)>,
    reply_boundary: RendererReplyBoundary,
) -> Option<Result<NavigationLoadOutcome, String>> {
    let source = inline_html_navigation_source(raw_url)?;
    Some(
        async {
            let InlineHtmlNavigationSource {
                document_url,
                html,
                response_headers,
            } = source?;
            let head = ResponseHead {
                final_url: document_url.clone(),
                status: 200,
                headers: response_headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            };
            prepare_navigation_from_captured_raw_response_with_engine_async(
                engine,
                page_reservation,
                load_inputs,
                document_url,
                method.to_owned(),
                request_headers,
                head,
                CapturedBody::from_string(html),
                MainDocumentBodyProgressSource::default(),
                NetworkObservationJournal::default(),
                None,
                true,
                reply_boundary,
            )
            .await
            .map(NavigationLoadOutcome::response_commit_ready)
        }
        .await,
    )
}

async fn load_data_url_navigation_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    method: &str,
    raw_url: &str,
    request_headers: Vec<(String, String)>,
    reply_boundary: RendererReplyBoundary,
) -> Option<Result<NavigationLoadOutcome, String>> {
    let source = decoded_data_url_navigation_response(raw_url)?;
    Some(
        async {
            let DecodedDataUrlNavigationResponse {
                requested_url,
                response,
            } = source?;
            let head = response.head();
            let body = CapturedBody::from_bytes(response.clone_body_bytes());
            prepare_navigation_from_captured_raw_response_with_engine_async(
                engine,
                page_reservation,
                load_inputs,
                requested_url,
                method.to_owned(),
                request_headers,
                head,
                body,
                MainDocumentBodyProgressSource::default(),
                NetworkObservationJournal::default(),
                None,
                false,
                reply_boundary,
            )
            .await
            .map(NavigationLoadOutcome::response_commit_ready)
        }
        .await,
    )
}

async fn build_navigation_from_streaming_raw_response_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    requested_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    mut response: StreamingRawResponse,
    network_observation_journal: NetworkObservationJournal,
    response_code: Option<u16>,
    response_headers_override: Vec<(String, String)>,
    body_progress_source: MainDocumentBodyProgressSource,
    body_completion_sink: Option<BackgroundNavigationBodyCompletionSink>,
    reserved_service_worker_client: Option<moli_core::runtime::RendererReservedServiceWorkerClient>,
    resource_source: CommittedDocumentResourceSource,
    reply_boundary: RendererReplyBoundary,
) -> Result<NavigationLoadOutcome, String> {
    let timing_enabled = moli_trace::cdp_nav_timing_enabled();
    let timing_started = std::time::Instant::now();
    let network_extra_info_available = !network_observation_journal.is_empty();
    let response_status = response_code.unwrap_or(response.status);
    let has_header_override = !response_headers_override.is_empty();
    let response_headers = if has_header_override {
        response_headers_override
    } else {
        response.headers.clone()
    };
    let response_cookie_reports = if has_header_override {
        load_inputs.store_response_cookie_reports(&response.final_url, &response_headers)
    } else {
        response.cookie_set_reports.clone()
    };
    let initial_request_cookie_report = response.request_cookie_report.clone();
    let response_from_cache = response.from_cache;
    let negotiated_http_version = response.negotiated_http_version;
    let final_url = response.final_url.clone();
    let redirect_chain = response
        .redirect_chain
        .clone()
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    let network_events = CompletedMainDocumentNetworkEvents::new(
        request_method.clone(),
        request_headers.clone(),
        initial_request_cookie_report.clone(),
        response_status,
        response_headers.clone(),
        response_cookie_reports.clone(),
        redirect_chain.clone(),
        network_extra_info_available,
        response_from_cache,
    )
    .with_negotiated_http_version(negotiated_http_version)
    .with_network_observation_journal(network_observation_journal.clone());

    if super::downloads::response_headers_indicate_download(&response_headers) {
        return Ok(NavigationLoadOutcome::download(DownloadNavigation {
            final_url,
            progress_transfer: CompletedDownloadProgressTransfer::new_streaming(
                response,
                network_events,
            ),
        }));
    }

    let (fetch_subresource_interception_enabled, fetch_subresource_interception_resource_type) =
        load_inputs.fetch_subresource_interception;
    body_progress_source.emit_response_metadata(
        &request_method,
        &request_headers,
        initial_request_cookie_report.as_ref(),
        &response.redirect_chain,
        &final_url,
        response_status,
        &response_headers,
        &response_cookie_reports,
        &network_observation_journal,
        network_extra_info_available,
        response_from_cache,
        negotiated_http_version,
    );
    if timing_enabled {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %requested_url,
            stage = "response_metadata_ready",
            elapsed_ms = timing_started.elapsed().as_millis(),
        );
    }
    let body_network_progress_state =
        body_progress_source.body_network_progress_for_completed_events(network_events);
    let body_progress_source_for_body_finish = body_progress_source.clone();
    let mut initial_body_chunk = if response_status_may_use_http_error_page(response_status) {
        match first_nonempty_response_body_chunk(&mut response).await? {
            Some(chunk) => Some(chunk),
            None => {
                let body =
                    CapturedBody::from_string(http_error_page_html(&final_url, response_status));
                return prepare_browser_owned_error_page_navigation_with_engine_async(
                    engine,
                    page_reservation,
                    load_inputs,
                    final_url,
                    request_method,
                    request_headers,
                    HTTP_RESPONSE_CODE_FAILURE_ERROR_TEXT.to_owned(),
                    body,
                    reply_boundary,
                )
                .await
                .map(NavigationLoadOutcome::response_commit_ready);
            }
        }
    } else {
        None
    };

    if response_headers_indicate_xml_document(&response_headers) {
        let redirected = response.redirected;
        let mut body_writer = CapturedBodyWriter::default();
        if let Some(chunk) = initial_body_chunk.take() {
            body_writer
                .append(&chunk)
                .map_err(|error| format!("failed to capture XML page body: {error}"))?;
        }
        while let Some(chunk) = response.next_chunk().await {
            body_writer
                .append(&chunk)
                .map_err(|error| format!("failed to capture XML page body: {error}"))?;
        }
        response
            .finish()
            .await
            .map_err(|error| format!("failed to read XML page body from stream: {error}"))?;
        let captured_body = body_writer
            .finish()
            .map_err(|error| format!("failed to finish captured XML page body: {error}"))?;
        let response_text = captured_body
            .materialize_bytes()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| format!("failed to materialize XML page body: {error}"))?;
        let page_storage = load_inputs.page_storage_handles();
        let main_document_commit = load_inputs
            .main_document_commit_for_final_url(&final_url, None)
            .map(Arc::new);
        let prepared_page = engine
            .prepare_document_page_from_response_with_storage_and_inspector_session_restores_async(
                page_reservation,
                page_storage.into_navigation_storage(),
                requested_url.clone(),
                final_url.clone(),
                load_inputs.navigation_initiator_url.clone(),
                redirected,
                redirect_chain.len(),
                response_status,
                response_headers.clone(),
                response_text,
                load_inputs.document_start_scripts.clone(),
                load_inputs.runtime_bindings.clone(),
                load_inputs
                    .runtime_inspector_session_restore_snapshots
                    .clone(),
                load_inputs.extra_http_headers.clone(),
                load_inputs.locale_override.clone(),
                load_inputs.timezone_override.clone(),
                load_inputs.script_execution_disabled,
                load_inputs.bypass_content_security_policy,
                load_inputs.cpu_throttling_rate,
                load_inputs.emulated_media.clone(),
                load_inputs.viewport_surface,
                load_inputs.network_offline,
                load_inputs.blocked_url_patterns.clone(),
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                load_inputs.root_frame_id.clone(),
                resource_source,
                main_document_commit.as_deref().cloned(),
            )
            .await
            .map_err(|error| format!("failed to prepare XML page `{}`: {error}", requested_url))?;
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %requested_url,
                stage = "response_commit_ready",
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }
        return Ok(NavigationLoadOutcome::response_commit_ready(
            ResponseCommitReady {
                prepared_page: Some(prepared_page),
                body_capture: Some(ResponseCommitBodyCapture::Ready(captured_body)),
                body_completion_sink,
                body_progress_source: body_progress_source_for_body_finish,
                body_network_progress_state: Some(body_network_progress_state),
                synthetic_body: false,
                requested_url,
                final_url,
                request_method,
                request_headers,
                response_status,
                response_headers,
                response_from_cache,
                navigation_engine: None,
                timing_started: timing_enabled.then_some(timing_started),
                main_document_commit,
                network_error_page: None,
            },
        ));
    }

    let (body_tx, body_rx) = mpsc::channel(EXTERNAL_RAW_BODY_CHANNEL_CAPACITY);
    let (completion_tx, completion_rx) = oneshot::channel();
    let raw_body = moli_core::runtime::ExternalRawDocumentBodyStream::new(body_rx, completion_rx);
    let page_storage = load_inputs.page_storage_handles();
    let main_document_commit = load_inputs
        .main_document_commit_for_final_url(&final_url, None)
        .map(Arc::new);
    let prepared_future = engine
        .prepare_streaming_raw_page_from_external_body_with_storage_and_inspector_session_restores_async(
            page_reservation,
            page_storage.into_navigation_storage(),
            requested_url.clone(),
            final_url.clone(),
            load_inputs.navigation_initiator_url.clone(),
            response.redirected,
            redirect_chain.len(),
            response_status,
            response_headers.clone(),
            raw_body,
            load_inputs.document_start_scripts.clone(),
            load_inputs.runtime_bindings.clone(),
            load_inputs
                .runtime_inspector_session_restore_snapshots
                .clone(),
            load_inputs.extra_http_headers.clone(),
            load_inputs.locale_override.clone(),
            load_inputs.timezone_override.clone(),
            load_inputs.script_execution_disabled,
            load_inputs.bypass_content_security_policy,
            load_inputs.cpu_throttling_rate,
            load_inputs.emulated_media.clone(),
            load_inputs.viewport_surface,
            load_inputs.network_offline,
            load_inputs.blocked_url_patterns.clone(),
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            PageVmInitStage::DomContentLoaded,
            reply_boundary,
            load_inputs.root_frame_id.clone(),
            resource_source,
            reserved_service_worker_client,
            main_document_commit.as_deref().cloned(),
        );
    let prepared_future = async {
        prepared_future
            .await
            .map_err(|error| format!("failed to prepare streaming raw page: {error:#}"))
    };
    let body_capture_task =
        spawn_streaming_body_capture(response, initial_body_chunk, body_tx, completion_tx);
    let prepare_await_started = std::time::Instant::now();
    let prepared_page = match prepared_future.await {
        Ok(prepared_page) => prepared_page,
        Err(error) => {
            body_capture_task.abort();
            return Err(format!(
                "failed to prepare page `{}`: {error}",
                requested_url
            ));
        }
    };
    if timing_enabled {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %requested_url,
            stage = "response_commit_ready",
            phase_ms = prepare_await_started.elapsed().as_millis(),
            elapsed_ms = timing_started.elapsed().as_millis(),
        );
    }
    Ok(NavigationLoadOutcome::response_commit_ready(
        ResponseCommitReady {
            prepared_page: Some(prepared_page),
            body_capture: Some(ResponseCommitBodyCapture::Pending(body_capture_task)),
            body_completion_sink,
            body_progress_source: body_progress_source_for_body_finish,
            body_network_progress_state: Some(body_network_progress_state),
            synthetic_body: false,
            requested_url,
            final_url,
            request_method,
            request_headers,
            response_status,
            response_headers,
            response_from_cache,
            navigation_engine: None,
            timing_started: timing_enabled.then_some(timing_started),
            main_document_commit,
            network_error_page: None,
        },
    ))
}

impl CdpConnection {
    fn navigation_load_inputs_for_navigation(
        &self,
        navigation: &NavigationDispatchState,
    ) -> TargetNavigationLoadInputs {
        apply_navigation_request_load_policy(
            self.navigation_load_inputs_for_session_owner(
                navigation.navigate_session_id.as_deref(),
            ),
            navigation.request_load_policy,
        )
        .with_main_document_commit_seed(RendererMainDocumentCommitSeed::from_navigation(navigation))
    }

    pub(crate) fn prepared_document_commit_configuration_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        final_url: &Url,
    ) -> PreparedDocumentPageCommitConfiguration {
        let idle_override = self.idle_override_for_navigation(session_id, final_url);
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        let runtime_isolated_worlds = self
            .prepare_loaded_navigation_commit_for_session_owner(session_id)
            .map(|commit_state| commit_state.isolated_worlds)
            .unwrap_or_default();
        PreparedDocumentPageCommitConfiguration {
            document_start_scripts: load_inputs.document_start_scripts,
            runtime_bindings: load_inputs.runtime_bindings,
            runtime_inspector_session_restore_snapshots: load_inputs
                .runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds,
            permission_overrides: load_inputs.permission_overrides,
            extra_http_headers: load_inputs.extra_http_headers,
            locale_override: load_inputs.locale_override,
            timezone_override: load_inputs.timezone_override,
            script_execution_disabled: load_inputs.script_execution_disabled,
            bypass_content_security_policy: load_inputs.bypass_content_security_policy,
            cpu_throttling_rate: load_inputs.cpu_throttling_rate,
            emulated_media: load_inputs.emulated_media,
            idle_override,
            viewport_surface: load_inputs.viewport_surface,
            network_offline: load_inputs.network_offline,
            blocked_url_patterns: load_inputs.blocked_url_patterns,
            fetch_subresource_interception: load_inputs.fetch_subresource_interception,
        }
    }

    fn idle_override_for_navigation(
        &mut self,
        session_id: Option<&str>,
        final_url: &Url,
    ) -> Option<moli_core::page::EmulatedIdleOverride> {
        // Commit configuration is assembled after the document navigation has
        // entered its pending state, where ordinary protocol access to the old
        // document is intentionally blocked. The outgoing page remains the
        // owner of frame-host state until commit and is the source Chromium
        // preserves when a same-site navigation reuses that frame host.
        let page = self
            .runtime_session_owner_slot(session_id)
            .ok()?
            .loaded_page()?;
        moli_site::same_site_urls(page.final_url(), final_url, true)
            .then(|| page.idle_override())
            .flatten()
    }

    pub(crate) async fn prepare_paused_streaming_response_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        response: &StreamingRawResponse,
        network_observation_journal: &NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<Option<PausedResponsePreparedDocument>, String> {
        if super::downloads::response_headers_indicate_download(&response.headers)
            || response_headers_indicate_xml_document(&response.headers)
            || response_status_may_use_http_error_page(response.status)
        {
            return Ok(None);
        }
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        if load_inputs.browser_context_id.is_none() {
            return Ok(None);
        }
        let requested_url = navigation.requested_url.clone();
        let network_extra_info_available = !network_observation_journal.is_empty();
        let request_method = navigation.request_method.clone();
        let request_headers = navigation.request_headers.clone();
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let final_url = response.final_url.clone();
        let response_status = response.status;
        let response_headers = response.headers.clone();
        let response_cookie_reports = response.cookie_set_reports.clone();
        let response_from_cache = response.from_cache;
        let negotiated_http_version = response.negotiated_http_version;
        let initial_request_cookie_report = response.request_cookie_report.clone();
        let redirect_chain = response
            .redirect_chain
            .clone()
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let network_events = CompletedMainDocumentNetworkEvents::new(
            request_method.clone(),
            request_headers.clone(),
            initial_request_cookie_report,
            response_status,
            response_headers.clone(),
            response_cookie_reports.clone(),
            redirect_chain.clone(),
            network_extra_info_available,
            response_from_cache,
        )
        .with_negotiated_http_version(negotiated_http_version)
        .with_network_observation_journal(network_observation_journal.clone());
        let body_network_progress_state =
            body_progress_source.body_network_progress_for_completed_events(network_events);
        let (renderer_body_tx, renderer_body_rx) =
            mpsc::channel(EXTERNAL_RAW_BODY_CHANNEL_CAPACITY);
        let (renderer_completion_tx, renderer_completion_rx) = oneshot::channel();
        let raw_body = moli_core::runtime::ExternalRawDocumentBodyStream::new(
            renderer_body_rx,
            renderer_completion_rx,
        );
        let shared_resource_runtime =
            self.shared_resource_runtime_for_navigation_load_inputs(&load_inputs);
        let mut engine = self.background_navigation_engine_for_load_inputs(&load_inputs);
        if let Some(resource_runtime) = shared_resource_runtime {
            engine
                .adopt_registered_resource_runtime(resource_runtime)
                .map_err(|error| error.to_string())?;
        }
        let (fetch_subresource_interception_enabled, fetch_subresource_interception_resource_type) =
            load_inputs.fetch_subresource_interception;
        let page_storage = load_inputs.page_storage_handles();
        let main_document_commit = load_inputs
            .main_document_commit_for_final_url(&final_url, None)
            .map(Arc::new);
        let page_reservation = self.reserve_renderer_page_for_session_owner(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            &engine,
        );
        let prepared_page = engine
            .prepare_streaming_raw_page_from_external_body_with_storage_and_inspector_session_restores_async(
                page_reservation,
                page_storage.into_navigation_storage(),
                requested_url.clone(),
                final_url.clone(),
                load_inputs.navigation_initiator_url.clone(),
                response.redirected,
                redirect_chain.len(),
                response_status,
                response_headers.clone(),
                raw_body,
                load_inputs.document_start_scripts.clone(),
                load_inputs.runtime_bindings.clone(),
                load_inputs
                    .runtime_inspector_session_restore_snapshots
                    .clone(),
                load_inputs.extra_http_headers.clone(),
                load_inputs.locale_override.clone(),
                load_inputs.timezone_override.clone(),
                load_inputs.script_execution_disabled,
                load_inputs.bypass_content_security_policy,
                load_inputs.cpu_throttling_rate,
                load_inputs.emulated_media.clone(),
                load_inputs.viewport_surface,
                load_inputs.network_offline,
                load_inputs.blocked_url_patterns.clone(),
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                PageVmInitStage::DomContentLoaded,
                RendererReplyBoundary::DocumentCommit,
                load_inputs.root_frame_id.clone(),
                CommittedDocumentResourceSource::Synthetic,
                None,
                main_document_commit.as_deref().cloned(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to prepare response-stage page `{}`: {error:#}",
                    requested_url
                )
            })?;
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %requested_url,
                stage = "response_stage_document_prepared",
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        Ok(Some(PausedResponsePreparedDocument {
            prepared_page,
            renderer_body_tx,
            renderer_completion_tx,
            body_progress_source,
            body_network_progress_state,
            requested_url,
            final_url,
            request_method,
            request_headers,
            response_status,
            response_headers,
            response_from_cache,
            negotiated_http_version,
            network_observation_journal: network_observation_journal.clone(),
            engine,
            timing_started,
            main_document_commit,
        }))
    }

    pub(crate) fn start_initial_document_page_ensure_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<PendingInitialDocumentPageBuild>, String> {
        let runtime_slot = match self.runtime_session_owner_slot(session_id) {
            Ok(slot) => slot,
            Err(_) => return Ok(None),
        };
        if runtime_slot.has_loaded_page() {
            return Ok(None);
        }
        if !self.runtime_session_owner_target_is_initial_about_blank(session_id) {
            return Ok(None);
        }
        // Session attachment is a target operation, not a Document command.
        // Chromium's Target.attachToTarget binds to the existing
        // DevToolsAgentHost even while its frame is navigating. If the target
        // already has a replacement navigation, that navigation owns the next
        // Page installation; starting an initial about:blank build would race
        // it, while rejecting the ensure would incorrectly reject attachment.
        // Treat this as an already-satisfied ensure and let the exact
        // target-owned navigation install the replacement Document.
        if self.has_pending_document_navigation_for_session_owner(session_id) {
            return Ok(None);
        }

        self.start_initial_empty_document_page_build_for_session_owner(session_id)
    }

    pub(crate) fn runtime_session_owner_target_is_initial_about_blank(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if let Some(is_on_initial_empty_document) =
            self.runtime_session_owner_record_is_on_initial_empty_document(session_id)
        {
            return is_on_initial_empty_document;
        }
        self.runtime_session_owner_target_url(session_id)
            .as_deref()
            .and_then(|raw_url| Url::parse(raw_url).ok())
            .as_ref()
            .is_some_and(moli_url::is_about_blank)
    }

    pub(crate) fn runtime_session_owner_should_start_initial_document_navigation(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if !self.runtime_session_owner_initial_empty_document_has_replacement_url(session_id) {
            return false;
        }
        if self.runtime_session_owner_initial_empty_document_has_pending_cross_document_navigation(
            session_id,
        ) {
            return false;
        }
        true
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_replacement_url(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if !self.runtime_session_owner_target_is_initial_about_blank(session_id) {
            return false;
        }
        let Some(target_url) = self.runtime_session_owner_target_url(session_id) else {
            return false;
        };
        let Some(initial_url) =
            self.runtime_session_owner_record_initial_empty_document_url(session_id)
        else {
            return false;
        };
        target_url != initial_url
    }

    fn start_initial_empty_document_page_build_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<PendingInitialDocumentPageBuild>, String> {
        let runtime_slot = match self.runtime_session_owner_slot(session_id) {
            Ok(slot) => slot,
            Err(_) => return Ok(None),
        };
        if runtime_slot.has_loaded_page() {
            return Ok(None);
        }
        if runtime_slot.has_initial_document_page_build_in_progress() {
            let waiter = runtime_slot
                .initial_document_page_build_waiter()
                .ok_or_else(|| "InitialDocumentPageBuildInProgressWithoutWaiter".to_owned())?;
            return Ok(Some(PendingInitialDocumentPageBuild {
                kind: PendingInitialDocumentPageBuildKind::Join { waiter },
            }));
        }

        let owner = self
            .initial_document_page_owner_for_session(session_id)
            .ok_or_else(|| "TargetNotLoaded".to_owned())?;
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        let requested_url = self
            .runtime_session_owner_initial_empty_document_url(session_id)
            .unwrap_or_else(|| Url::parse("about:blank").expect("about:blank should be valid"));
        let (fetch_subresource_interception_enabled, fetch_subresource_interception_resource_type) =
            load_inputs.fetch_subresource_interception;
        let mut engine = self.background_navigation_engine_for_load_inputs(&load_inputs);
        let page_storage = load_inputs.page_storage_handles();
        let top_level_storage_key =
            self.runtime_session_owner_initial_empty_document_storage_key(session_id);
        self.runtime_session_owner_slot_mut(session_id)?
            .start_initial_document_page_build();
        let page_reservation = engine.reserve_page_for_creation();
        let renderer_page = RendererPageResidenceIdentity::from_parts(
            page_reservation.local_host_id(),
            page_reservation.page_id(),
        );
        if !self
            .runtime_session_owner_slot_mut(session_id)?
            .bind_initial_document_page_build_renderer_page(renderer_page)
        {
            let message =
                "initial document Page reservation no longer matches its target build".to_owned();
            let _ = self.runtime_session_owner_slot_mut(session_id).map(|slot| {
                slot.fail_initial_document_page_build(message.clone());
                slot.mark_loaded_page_absent(
                    TargetPageAbsenceReason::InitialDocumentPageBuildPending,
                );
            });
            return Err(message);
        }
        // The renderer command below can open the Page output stream and
        // publish bootstrap observations before this protocol turn regains
        // control. Bind the reserved Page to its target before enqueueing that
        // command; binding after `start_*` would leave a real cross-thread race
        // where the concrete FIFO receives its first publication ownerless.
        let page_owner = self
            .pending_target_page_residence_identity_for_session(session_id)
            .ok_or_else(|| "initial document Page reservation lost its target owner".to_owned())?;
        self.bind_renderer_page_output_owner(renderer_page, page_owner);
        let pending = engine
            .start_build_html_page_from_response_with_storage_and_inspector_session_restores(
                page_reservation,
                page_storage.into_navigation_storage(),
                requested_url.clone(),
                requested_url,
                load_inputs.navigation_initiator_url.clone(),
                false,
                0,
                200,
                vec![("content-type".into(), "text/html".into())],
                ABOUT_BLANK_DOCUMENT_HTML.into(),
                load_inputs.document_start_scripts.clone(),
                load_inputs.runtime_bindings.clone(),
                load_inputs
                    .runtime_inspector_session_restore_snapshots
                    .clone(),
                load_inputs.extra_http_headers.clone(),
                load_inputs.locale_override.clone(),
                load_inputs.timezone_override.clone(),
                load_inputs.script_execution_disabled,
                load_inputs.bypass_content_security_policy,
                load_inputs.cpu_throttling_rate,
                load_inputs.emulated_media.clone(),
                load_inputs.viewport_surface,
                load_inputs.network_offline,
                load_inputs.blocked_url_patterns.clone(),
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                load_inputs.root_frame_id.clone(),
                top_level_storage_key,
                None,
            )
            .map_err(|error| {
                let message = format!("failed to start initial document page build: {error}");
                let _ = self.runtime_session_owner_slot_mut(session_id).map(|slot| {
                    slot.fail_initial_document_page_build(message.clone());
                    slot.mark_loaded_page_absent(
                        TargetPageAbsenceReason::InitialDocumentPageBuildPending,
                    );
                });
                message
            })?;
        Ok(Some(PendingInitialDocumentPageBuild {
            kind: PendingInitialDocumentPageBuildKind::Build {
                owner,
                load_inputs: Box::new(load_inputs),
                override_mode: NavigationLoadInputOverrideMode::FreshlyBuiltPage,
                engine: Box::new(Some(engine)),
                pending,
            },
        }))
    }

    pub(crate) fn reset_failed_initial_document_page_build_for_owner(
        &mut self,
        failed: FailedInitialDocumentPageBuild,
    ) -> String {
        let message = failed.message().to_owned();
        if let Some(owner) = failed.build_owner()
            && let Some(browser_context) = self.browser_context_by_id_mut(&owner.browser_context_id)
        {
            if browser_context.active_target_id() == Some(owner.target_id.as_str()) {
                if browser_context
                    .active_target
                    .runtime_slot
                    .has_initial_document_page_build_in_progress()
                {
                    browser_context
                        .active_target
                        .runtime_slot
                        .fail_initial_document_page_build(message.clone());
                    browser_context
                        .active_target
                        .runtime_slot
                        .mark_loaded_page_absent(
                            TargetPageAbsenceReason::InitialDocumentPageBuildPending,
                        );
                }
            } else if let Some(target) = browser_context.background_target_mut(&owner.target_id)
                && target
                    .runtime_slot
                    .has_initial_document_page_build_in_progress()
            {
                target
                    .runtime_slot
                    .fail_initial_document_page_build(message.clone());
                target.runtime_slot.mark_loaded_page_absent(
                    TargetPageAbsenceReason::InitialDocumentPageBuildPending,
                );
            }
        }
        failed.into_message()
    }

    fn runtime_session_owner_initial_empty_document_url(
        &self,
        session_id: Option<&str>,
    ) -> Option<Url> {
        if let Some(raw_url) =
            self.runtime_session_owner_record_initial_empty_document_url(session_id)
        {
            return Url::parse(&raw_url).ok().filter(moli_url::is_about_blank);
        }
        self.runtime_session_owner_target_url(session_id)
            .as_deref()
            .and_then(|raw_url| Url::parse(raw_url).ok())
            .filter(moli_url::is_about_blank)
    }

    fn complete_initial_document_page_build_for_page_owner(
        &mut self,
        owner: &InitialDocumentPageOwner,
    ) {
        if let Some(browser_context) = self.browser_context_by_id_mut(&owner.browser_context_id) {
            if browser_context.active_target_id() == Some(owner.target_id.as_str()) {
                browser_context
                    .active_target
                    .runtime_slot
                    .complete_initial_document_page_build();
            } else if let Some(target) = browser_context.background_target_mut(&owner.target_id) {
                target.runtime_slot.complete_initial_document_page_build();
            }
        }
    }

    fn complete_stale_initial_document_page_build_for_page_owner(
        &mut self,
        owner: &InitialDocumentPageOwner,
    ) {
        self.complete_initial_document_page_build_for_page_owner(owner);
    }

    fn initial_document_page_owner_can_install_current_page(
        &self,
        owner: &InitialDocumentPageOwner,
    ) -> bool {
        self.browser_context_by_id(&owner.browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.can_install_current_initial_empty_document_page(&owner.target_id)
            })
    }

    fn fail_initial_document_page_build_for_owner(
        &mut self,
        owner: &InitialDocumentPageOwner,
        message: String,
    ) {
        if let Some(browser_context) = self.browser_context_by_id_mut(&owner.browser_context_id) {
            if browser_context.active_target_id() == Some(owner.target_id.as_str()) {
                let runtime_slot = &mut browser_context.active_target.runtime_slot;
                if runtime_slot.has_initial_document_page_build_in_progress() {
                    runtime_slot.fail_initial_document_page_build(message);
                    runtime_slot.mark_loaded_page_absent(
                        TargetPageAbsenceReason::InitialDocumentPageBuildPending,
                    );
                }
            } else if let Some(target) = browser_context.background_target_mut(&owner.target_id) {
                let runtime_slot = &mut target.runtime_slot;
                if runtime_slot.has_initial_document_page_build_in_progress() {
                    runtime_slot.fail_initial_document_page_build(message);
                    runtime_slot.mark_loaded_page_absent(
                        TargetPageAbsenceReason::InitialDocumentPageBuildPending,
                    );
                }
            }
        }
    }

    pub(crate) async fn complete_initial_document_page_build_for_owner(
        &mut self,
        completed: CompletedInitialDocumentPageBuild,
    ) -> Result<(), String> {
        self.complete_initial_document_page_build_for_owner_with_creation_diagnostics(completed)
            .await
            .map(|_| ())
    }

    pub(crate) async fn complete_initial_document_page_build_for_owner_with_creation_diagnostics(
        &mut self,
        completed: CompletedInitialDocumentPageBuild,
    ) -> Result<LoadedPageCreationDiagnosticsParts, String> {
        let CompletedInitialDocumentPageBuild::Built {
            owner,
            load_inputs,
            override_mode,
            engine,
            built,
        } = completed
        else {
            return Ok(LoadedPageCreationDiagnosticsParts::default());
        };
        let load_inputs = *load_inputs;
        let engine = *engine;
        let built = *built;
        let diagnostics = loaded_page_creation_diagnostics_parts(built.page_creation_diagnostics);
        let page_creation_artifacts = built.page_creation_artifacts;
        let mut page = built.page;
        if !self.initial_document_page_owner_can_install_current_page(&owner) {
            let _ = page.close_async().await;
            self.complete_stale_initial_document_page_build_for_page_owner(&owner);
            return Ok(LoadedPageCreationDiagnosticsParts::default());
        }
        apply_navigation_load_input_overrides_async(&mut page, &load_inputs, override_mode)
            .await
            .inspect_err(|message| {
                self.fail_initial_document_page_build_for_owner(&owner, message.clone());
            })?;
        let install_result = self
            .install_initial_loaded_page_for_page_owner_async(&owner, page, page_creation_artifacts)
            .await
            .inspect_err(|message| {
                self.fail_initial_document_page_build_for_owner(&owner, message.clone());
            })?;
        if install_result == InitialDocumentPageInstallResult::Stale {
            self.complete_stale_initial_document_page_build_for_page_owner(&owner);
            return Ok(LoadedPageCreationDiagnosticsParts::default());
        }
        if let Some(engine) = engine {
            self.adopt_loaded_navigation_engine_for_page_owner(&owner, engine);
        }
        self.complete_initial_document_page_build_for_page_owner(&owner);
        Ok(diagnostics)
    }

    pub async fn load_navigation_via_runtime_async(
        &mut self,
        raw_url: &str,
    ) -> Result<LoadedNavigation, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(None);
        self.load_navigation_via_runtime_with_load_inputs_async(None, raw_url, load_inputs)
            .await
    }

    /// Builds a complete navigation fixture through the exact target/session
    /// policy path used by a real `Page.navigate` command.
    ///
    /// Protocol tests use this instead of constructing a `Page` off-owner and
    /// inserting it later. The latter cannot model renderer output ownership:
    /// the Page stream is opened while the page is built and must already be
    /// bound to its target before any concrete publication is consumed.
    #[cfg(test)]
    pub(crate) async fn load_navigation_via_runtime_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        raw_url: &str,
    ) -> Result<LoadedNavigation, String> {
        let load_inputs = self.navigation_fixture_load_inputs_for_session_owner(session_id)?;
        self.load_navigation_via_runtime_with_load_inputs_async(session_id, raw_url, load_inputs)
            .await
    }

    async fn load_navigation_via_runtime_with_load_inputs_async(
        &mut self,
        session_id: Option<&str>,
        raw_url: &str,
        load_inputs: TargetNavigationLoadInputs,
    ) -> Result<LoadedNavigation, String> {
        let request_headers = load_inputs.extra_http_headers.clone();
        let navigation = self
            .load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
                session_id,
                load_inputs,
                "GET",
                raw_url,
                None,
                request_headers,
                MainDocumentBodyProgressSource::default(),
            )
            .await?;
        self.commit_navigation_load_outcome_for_session_owner_async(session_id, navigation)
            .await
    }

    async fn commit_navigation_load_outcome_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        navigation: NavigationLoadOutcome,
    ) -> Result<LoadedNavigation, String> {
        match navigation {
            NavigationLoadOutcome::ResponseCommitReady(navigation) => {
                let navigation = *navigation;
                let configuration = self.prepared_document_commit_configuration_for_session_owner(
                    session_id,
                    navigation.final_url(),
                );
                navigation
                    .update_commit_configuration(configuration)
                    .await?;
                let permit = navigation.issue_commit_permit();
                navigation.commit(permit).await
            }
            NavigationLoadOutcome::Loaded(navigation) => Ok(*navigation),
            NavigationLoadOutcome::Download(_) => {
                Err("navigation resolved to a download".to_owned())
            }
            NavigationLoadOutcome::NetworkFailure(error_text) => Err(error_text),
        }
    }

    pub async fn load_navigation_request_via_runtime_async(
        &mut self,
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
    ) -> Result<NavigationLoadOutcome, String> {
        self.load_navigation_request_via_runtime_with_network_events_async(
            None,
            method,
            raw_url,
            body,
            request_headers,
            MainDocumentBodyProgressSource::default(),
            NavigationRequestLoadPolicy::DocumentInitiated,
        )
        .await
    }

    pub(crate) async fn load_navigation_request_via_runtime_with_network_events_async(
        &mut self,
        session_id: Option<&str>,
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
        body_progress_source: MainDocumentBodyProgressSource,
        request_load_policy: NavigationRequestLoadPolicy,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = apply_navigation_request_load_policy(
            self.navigation_load_inputs_for_session_owner(session_id),
            request_load_policy,
        );
        self.load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
            session_id,
            load_inputs,
            method,
            raw_url,
            body.map(String::into_bytes),
            request_headers,
            body_progress_source,
        )
        .await
    }

    pub(crate) async fn load_navigation_request_via_runtime_with_network_events_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        self.load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
            navigation.navigate_session_id.as_deref(),
            self.navigation_load_inputs_for_navigation(navigation),
            &navigation.request_method,
            navigation.requested_url.as_str(),
            navigation.clone_request_body_bytes(),
            navigation.request_headers.clone(),
            body_progress_source,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
        &mut self,
        session_id: Option<&str>,
        load_inputs: TargetNavigationLoadInputs,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        if load_inputs.browser_context_id.is_none() {
            let page_reservation = self.engine.reserve_page_for_creation();
            if let Some(navigation) = self
                .load_inline_html_navigation_async(
                    page_reservation,
                    &load_inputs,
                    method,
                    raw_url,
                    request_headers.clone(),
                )
                .await
            {
                return navigation;
            }
            if let Some(navigation) = load_data_url_navigation_with_engine_async(
                &mut self.engine,
                page_reservation,
                &load_inputs,
                method,
                raw_url,
                request_headers.clone(),
                RendererReplyBoundary::Stage,
            )
            .await
            {
                return navigation;
            }
        } else {
            let mut inline_engine = self.background_navigation_engine_for_load_inputs(&load_inputs);
            let page_reservation = self.reserve_renderer_page_for_session_owner(
                session_id,
                &load_inputs,
                &inline_engine,
            );
            if let Some(navigation) = self
                .load_inline_html_navigation_with_engine_async(
                    &mut inline_engine,
                    page_reservation,
                    &load_inputs,
                    method,
                    raw_url,
                    request_headers.clone(),
                    RendererReplyBoundary::Stage,
                )
                .await
            {
                return navigation
                    .map(|navigation| navigation.with_navigation_engine(inline_engine));
            }
            if let Some(navigation) = load_data_url_navigation_with_engine_async(
                &mut inline_engine,
                page_reservation,
                &load_inputs,
                method,
                raw_url,
                request_headers.clone(),
                RendererReplyBoundary::Stage,
            )
            .await
            {
                return navigation
                    .map(|navigation| navigation.with_navigation_engine(inline_engine));
            }
        }

        ensure_url_not_blocked_for_load_inputs(&load_inputs, raw_url)?;
        if load_inputs.network_offline {
            return Err("Network emulation offline".to_owned());
        }

        let requested_url = Url::parse(raw_url)
            .map_err(|error| format!("failed to parse request url `{raw_url}`: {error}"))?;
        let response = self
            .fetch_navigation_streaming_raw_response_with_load_inputs_async(
                &load_inputs,
                method,
                raw_url,
                body,
                request_headers.clone(),
                None,
            )
            .await?;
        self.build_navigation_from_streaming_raw_response_with_load_inputs_async(
            session_id,
            &load_inputs,
            requested_url,
            method.to_owned(),
            request_headers,
            response,
            None,
            Vec::new(),
            body_progress_source,
        )
        .await
    }

    pub(crate) fn navigation_load_job_for_navigation(
        &mut self,
        token: &DocumentNavigationToken,
        navigation: &NavigationDispatchState,
        body_progress_source: MainDocumentBodyProgressSource,
        early_result: Option<BackgroundNavigationEarlyResult>,
    ) -> Option<BackgroundNavigationLoadJob> {
        let cancellation = self.document_navigation_cancellation_handle(token)?;
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        // Ensure the connection's resident browser resource runtime exists,
        // then share only that transport/cache owner with the background job.
        // Page policy remains owned by the job's NavigationEngine.
        let shared_resource_runtime =
            self.shared_resource_runtime_for_navigation_load_inputs(&load_inputs);
        let engine = self.background_navigation_engine_for_load_inputs(&load_inputs);
        let page_reservation = self.reserve_renderer_page_for_session_owner(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            &engine,
        );
        Some(BackgroundNavigationLoadJob {
            engine,
            page_reservation,
            cancellation,
            early_result,
            load_inputs,
            method: navigation.request_method.clone(),
            raw_url: navigation.requested_url.as_str().to_owned(),
            body: navigation.clone_request_body_bytes(),
            request_headers: navigation.request_headers.clone(),
            body_progress_source,
            shared_resource_runtime,
        })
    }

    pub(crate) fn background_navigation_load_job_for_navigation(
        &mut self,
        token: &DocumentNavigationToken,
        navigation: &NavigationDispatchState,
        body_progress_source: MainDocumentBodyProgressSource,
        early_result: Option<BackgroundNavigationEarlyResult>,
    ) -> Option<BackgroundNavigationLoadJob> {
        let job = self.navigation_load_job_for_navigation(
            token,
            navigation,
            body_progress_source,
            early_result,
        )?;
        self.arm_background_navigation_completion(token, None)
            .then_some(job)
    }

    pub(crate) fn background_streaming_response_navigation_load_job_for_navigation(
        &mut self,
        navigation: &NavigationDispatchState,
        response: StreamingRawResponse,
        network_observation_journal: NetworkObservationJournal,
        response_code: Option<u16>,
        response_headers_override: Vec<(String, String)>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> BackgroundStreamingResponseNavigationLoadJob {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        let shared_resource_runtime =
            self.shared_resource_runtime_for_navigation_load_inputs(&load_inputs);
        let engine = self.background_navigation_engine_for_load_inputs(&load_inputs);
        let page_reservation = self.reserve_renderer_page_for_session_owner(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            &engine,
        );
        BackgroundStreamingResponseNavigationLoadJob {
            engine,
            page_reservation,
            load_inputs,
            requested_url: navigation.requested_url.clone(),
            request_method: navigation.request_method.clone(),
            request_headers: navigation.request_headers.clone(),
            response,
            network_observation_journal,
            response_code,
            response_headers_override,
            body_progress_source,
            shared_resource_runtime,
        }
    }

    /// Reserves and binds the exact future Page before detached navigation work
    /// can enqueue a renderer prepare command.
    ///
    /// A prepared document may open its concrete output stream while the
    /// navigation future is still running. The protocol target therefore must
    /// own the reservation at job construction time rather than trying to infer
    /// it from `ResponseCommitReady` after the future completes.
    fn reserve_renderer_page_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        load_inputs: &TargetNavigationLoadInputs,
        engine: &NavigationEngine,
    ) -> RendererPageReservationToken {
        let page_reservation = engine.reserve_page_for_creation();
        self.bind_renderer_page_reservation_for_session_owner(
            session_id,
            load_inputs,
            page_reservation,
        );
        page_reservation
    }

    fn bind_renderer_page_reservation_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        load_inputs: &TargetNavigationLoadInputs,
        page_reservation: RendererPageReservationToken,
    ) {
        if load_inputs.browser_context_id.is_none() {
            return;
        }
        // A browser-context fixture without a concrete target is not yet a
        // Page owner. Its eventual target installation must bind the
        // reservation; recording `target_id: None` here would conflict with
        // that exact owner when the stream-open control is later consumed.
        if let Some((browser_context_id, Some(_target_id))) =
            self.target_owner_identity_for_session(session_id)
        {
            assert_eq!(
                load_inputs.browser_context_id.as_deref(),
                Some(browser_context_id.as_str()),
                "navigation Page reservation must bind inside its captured browser context"
            );
            let renderer_page = RendererPageResidenceIdentity::from_parts(
                page_reservation.local_host_id(),
                page_reservation.page_id(),
            );
            let page_owner = self
                .reserve_target_page_residence_identity_for_session(session_id, renderer_page)
                .expect("navigation Page reservation must retain its target owner");
            self.bind_renderer_page_output_owner(renderer_page, page_owner);
        }
    }

    pub(super) fn background_navigation_engine_for_load_inputs(
        &self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> NavigationEngine {
        // Same-browser-context CDP background targets retain only another
        // NavigationEngine wrapper; their page contexts still live under the
        // same renderer owner. Different browser contexts keep distinct
        // renderer owners.
        let mut engine = if self
            .engine
            .browser_context_runtime()
            .shares_state_with(&load_inputs.renderer_runtime.runtime())
        {
            NavigationEngine::new_with_runtime_config_and_shared_renderer_owner(
                self.navigation_runtime_config_for_load_inputs(load_inputs),
                &self.engine,
            )
            .expect("active shared BrowserContext owner must be live")
        } else if let Some(engine) = self
            .retained_background_navigation_engine_for_load_inputs(load_inputs)
            .cloned()
        {
            engine
        } else {
            NavigationEngine::new_with_runtime_config_and_browser_context_access(
                self.navigation_runtime_config_for_load_inputs(load_inputs),
                load_inputs.renderer_runtime.clone(),
            )
            .expect("navigation load BrowserContext owner must be live")
        };
        self.apply_fetch_overrides_to_background_navigation_engine(&mut engine, load_inputs);
        if !self.active_resource_runtime_matches_navigation_load_inputs(load_inputs) {
            // Sharing the renderer owner must not implicitly share a transport
            // runtime that was rejected above for a different target policy.
            // The next request rebuilds from this engine's effective fetch
            // configuration and the target's storage partition.
            engine.reset_resource_runtime_without_loaded_page();
        }
        engine.set_bypass_service_worker(load_inputs.bypass_service_worker);
        // The engine may publish lifecycle or resource activity before the
        // DCL-bound navigation result is adopted into a target slot. Bind its
        // scheduler sender at construction time so that early tail work queues
        // a wake instead of disappearing in the handoff window.
        self.apply_scheduler_senders_to_navigation_engine(&engine);
        engine
    }

    fn navigation_runtime_config_for_load_inputs(
        &self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> moli_core::runtime::NavigationRuntimeConfig {
        let mut config = self.engine.runtime_config();
        *config.fetch_config_mut() = self.fetch_config_for_load_inputs(load_inputs);
        config
    }

    fn retained_background_navigation_engine_for_load_inputs(
        &self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> Option<&NavigationEngine> {
        let browser_context_id = load_inputs.browser_context_id.as_ref()?;
        let target_id = load_inputs.root_frame_id.as_ref()?;
        self.retained_background_navigation_engines
            .get(&(browser_context_id.clone(), target_id.clone()))
    }

    fn apply_fetch_overrides_to_background_navigation_engine(
        &self,
        engine: &mut NavigationEngine,
        load_inputs: &TargetNavigationLoadInputs,
    ) {
        engine.set_browser_identity_override(
            load_inputs
                .browser_identity_override
                .clone()
                .or_else(|| self.global_browser_identity_override.clone())
                .unwrap_or_else(|| self.base_browser_identity.clone()),
        );
        engine.set_http_proxy_override(
            load_inputs
                .http_proxy_override
                .clone()
                .or_else(|| self.base_http_proxy.clone()),
        );
        engine.set_http_no_proxy_override(
            load_inputs
                .http_no_proxy_override
                .clone()
                .or_else(|| self.base_http_no_proxy.clone()),
        );
        engine.set_tls_verify_host(
            load_inputs
                .tls_verify_host_override
                .unwrap_or(self.base_tls_verify_host),
        );
    }

    fn shared_resource_runtime_for_navigation_load_inputs(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> Option<moli_core::network::BrowserResourceRuntime> {
        if !self.active_resource_runtime_matches_navigation_load_inputs(load_inputs) {
            return None;
        }
        let resource_storage = load_inputs.resource_storage_handles();
        self.engine
            .ensure_resource_runtime_ready_for_navigation_storage(
                resource_storage.into_navigation_storage(),
            )
            .ok()?;
        self.engine
            .resource_request_client()
            .map(|client| client.browser_resource_runtime())
    }

    fn active_resource_runtime_matches_navigation_load_inputs(
        &self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> bool {
        if !self
            .engine
            .browser_context_runtime()
            .shares_state_with(&load_inputs.renderer_runtime.runtime())
        {
            return false;
        }
        let fetch_config = self.fetch_config_for_load_inputs(load_inputs);
        fetch_config.browser_identity() == self.engine.fetch_config().browser_identity()
            && fetch_config.http_proxy() == self.http_proxy()
            && fetch_config.http_no_proxy() == self.http_no_proxy()
            && fetch_config.tls_verify_host() == self.tls_verify_host()
    }

    fn fetch_config_for_load_inputs(
        &self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> FetchConfig {
        let mut fetch_config = self.fetch_config().clone();
        fetch_config.set_browser_identity(
            load_inputs
                .browser_identity_override
                .clone()
                .or_else(|| self.global_browser_identity_override.clone())
                .unwrap_or_else(|| self.base_browser_identity.clone()),
        );
        fetch_config.set_http_proxy(
            load_inputs
                .http_proxy_override
                .clone()
                .or_else(|| self.base_http_proxy.clone()),
        );
        fetch_config.set_http_no_proxy(
            load_inputs
                .http_no_proxy_override
                .clone()
                .or_else(|| self.base_http_no_proxy.clone()),
        );
        fetch_config.set_tls_verify_host(
            load_inputs
                .tls_verify_host_override
                .unwrap_or(self.base_tls_verify_host),
        );
        fetch_config
    }

    pub async fn load_page_via_runtime_async(&mut self, raw_url: &str) -> Result<Page, String> {
        let navigation = self.load_navigation_via_runtime_async(raw_url).await?;
        if let Some(engine) = navigation.navigation_engine {
            self.replace_navigation_engine(engine);
        }
        Ok(navigation.page)
    }

    async fn load_inline_html_navigation_async(
        &mut self,
        page_reservation: RendererPageReservationToken,
        load_inputs: &TargetNavigationLoadInputs,
        method: &str,
        raw_url: &str,
        request_headers: Vec<(String, String)>,
    ) -> Option<Result<NavigationLoadOutcome, String>> {
        load_inline_html_navigation_with_engine_async(
            &mut self.engine,
            page_reservation,
            load_inputs,
            method,
            raw_url,
            request_headers,
            RendererReplyBoundary::Stage,
        )
        .await
    }

    async fn load_inline_html_navigation_with_engine_async(
        &mut self,
        engine: &mut NavigationEngine,
        page_reservation: RendererPageReservationToken,
        load_inputs: &TargetNavigationLoadInputs,
        method: &str,
        raw_url: &str,
        request_headers: Vec<(String, String)>,
        reply_boundary: RendererReplyBoundary,
    ) -> Option<Result<NavigationLoadOutcome, String>> {
        load_inline_html_navigation_with_engine_async(
            engine,
            page_reservation,
            load_inputs,
            method,
            raw_url,
            request_headers,
            reply_boundary,
        )
        .await
    }

    /// Builds a loaded document from an already-buffered text response.
    ///
    /// This path is for synthetic or in-memory document sources, including
    /// initial document page build and test/setup helpers. It still uses the
    /// phase-one HTML parser; it is not the old NativeDom static builder and
    /// should not be used for real network document streaming.
    pub async fn build_loaded_navigation_from_buffered_response_async(
        &mut self,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
    ) -> Result<LoadedNavigation, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(None);
        let initial_request_cookie_report =
            load_inputs.request_cookie_report_for_navigation(&requested_url, &request_method, true);
        self.build_loaded_navigation_from_buffered_response_with_request_cookie_report_async(
            &load_inputs,
            requested_url,
            request_method,
            request_headers,
            response_status,
            response_headers,
            response_body,
            None,
            initial_request_cookie_report,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn build_loaded_navigation_from_buffered_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
    ) -> Result<LoadedNavigation, String> {
        let load_inputs = self.navigation_fixture_load_inputs_for_session_owner(session_id)?;
        let initial_request_cookie_report =
            load_inputs.request_cookie_report_for_navigation(&requested_url, &request_method, true);
        let navigation = self
            .build_navigation_from_buffered_body_source_with_load_inputs_async(
                session_id,
                &load_inputs,
                requested_url.clone(),
                requested_url,
                request_method,
                request_headers,
                response_status,
                response_headers,
                CapturedBody::from_string(response_body),
                initial_request_cookie_report,
                NetworkObservationJournal::default(),
                MainDocumentBodyProgressSource::default(),
            )
            .await?;
        self.commit_navigation_load_outcome_for_session_owner_async(session_id, navigation)
            .await
    }

    #[cfg(test)]
    fn navigation_fixture_load_inputs_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Result<TargetNavigationLoadInputs, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        let frame_id = load_inputs.root_frame_id.clone().ok_or_else(|| {
            "navigation fixture requires an installed target root frame".to_owned()
        })?;
        Ok(load_inputs.with_main_document_commit_seed(
            RendererMainDocumentCommitSeed::from_navigation_fixture(
                frame_id,
                DEFAULT_LOADER_ID.to_owned(),
                monotonic_timestamp_seconds(),
            ),
        ))
    }

    /// Replays an already-buffered document response into the phase-one parser
    /// while preserving the request-cookie report captured before a CDP pause.
    ///
    /// This is for synthetic/buffered inputs such as `Fetch.fulfillRequest`,
    /// `Fetch.getResponseBody` materialization, or data URL response-stage
    /// replay. Real network document navigation should prefer the streaming
    /// raw response builders below.
    #[cfg(test)]
    pub(crate) async fn build_loaded_navigation_from_buffered_response_preserving_request_cookie_report_async(
        &mut self,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        initial_request_cookie_report: Option<StoredCookieQueryReport>,
    ) -> Result<LoadedNavigation, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(None);
        self.build_loaded_navigation_from_buffered_response_with_request_cookie_report_async(
            &load_inputs,
            requested_url,
            request_method,
            request_headers,
            response_status,
            response_headers,
            response_body,
            None,
            initial_request_cookie_report,
        )
        .await
    }

    pub(crate) async fn build_navigation_from_buffered_body_source_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: CapturedBody,
        initial_request_cookie_report: Option<StoredCookieQueryReport>,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_buffered_body_source_with_load_inputs_async(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            navigation.requested_url.clone(),
            final_url,
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            response_status,
            response_headers,
            response_body,
            initial_request_cookie_report,
            network_observation_journal,
            body_progress_source,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_navigation_from_buffered_body_source_with_load_inputs_async(
        &mut self,
        session_id: Option<&str>,
        load_inputs: &TargetNavigationLoadInputs,
        requested_url: Url,
        final_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: CapturedBody,
        initial_request_cookie_report: Option<StoredCookieQueryReport>,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let response_cookie_reports =
            load_inputs.store_response_cookie_reports(&final_url, &response_headers);
        let head = ResponseHead {
            final_url,
            status: response_status,
            headers: response_headers,
            request_cookie_report: initial_request_cookie_report,
            cookie_set_reports: response_cookie_reports,
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        };
        let page_reservation = self.engine.reserve_page_for_creation();
        self.bind_renderer_page_reservation_for_session_owner(
            session_id,
            load_inputs,
            page_reservation,
        );
        prepare_navigation_from_captured_raw_response_with_engine_async(
            &mut self.engine,
            page_reservation,
            load_inputs,
            requested_url,
            request_method,
            request_headers,
            head,
            response_body,
            body_progress_source,
            network_observation_journal,
            None,
            false,
            RendererReplyBoundary::Stage,
        )
        .await
        .map(NavigationLoadOutcome::response_commit_ready)
    }

    async fn build_loaded_navigation_from_buffered_response_with_request_cookie_report_async(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        captured_response_body: Option<CapturedBody>,
        initial_request_cookie_report: Option<StoredCookieQueryReport>,
    ) -> Result<LoadedNavigation, String> {
        let response_cookie_reports =
            load_inputs.store_response_cookie_reports(&requested_url, &response_headers);
        let (fetch_subresource_interception_enabled, fetch_subresource_interception_resource_type) =
            load_inputs.fetch_subresource_interception;
        let page_storage = load_inputs.page_storage_handles();
        let main_document_commit = load_inputs
            .main_document_commit_for_final_url(&requested_url, None)
            .map(Arc::new);
        let built = self
            .engine
            .build_html_page_from_response_with_storage_and_inspector_session_restores_async(
                page_storage.into_navigation_storage(),
                requested_url.clone(),
                requested_url.clone(),
                load_inputs.navigation_initiator_url.clone(),
                false,
                0,
                response_status,
                response_headers.clone(),
                response_body.clone(),
                load_inputs.document_start_scripts.clone(),
                load_inputs.runtime_bindings.clone(),
                load_inputs
                    .runtime_inspector_session_restore_snapshots
                    .clone(),
                load_inputs.extra_http_headers.clone(),
                load_inputs.locale_override.clone(),
                load_inputs.timezone_override.clone(),
                load_inputs.script_execution_disabled,
                load_inputs.bypass_content_security_policy,
                load_inputs.cpu_throttling_rate,
                load_inputs.emulated_media.clone(),
                load_inputs.viewport_surface,
                load_inputs.network_offline,
                load_inputs.blocked_url_patterns.clone(),
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                load_inputs.root_frame_id.clone(),
                main_document_commit.as_deref().cloned(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to execute scripts for synthetic response `{}`: {error}",
                    requested_url
                )
            })?;
        let diagnostics = loaded_page_creation_diagnostics_parts(built.page_creation_diagnostics);
        let mut page = built.page;
        apply_navigation_load_input_overrides_async(
            &mut page,
            load_inputs,
            NavigationLoadInputOverrideMode::FreshlyBuiltPage,
        )
        .await?;
        let redirect_chain = Vec::new();
        let network_progress = MainDocumentBodyNetworkProgress::CompletedBody(Box::new(
            CompletedMainDocumentNetworkEvents::new(
                request_method.clone(),
                request_headers.clone(),
                initial_request_cookie_report.clone(),
                response_status,
                response_headers.clone(),
                response_cookie_reports.clone(),
                redirect_chain.clone(),
                false,
                false,
            ),
        ));

        Ok(LoadedNavigation {
            page,
            pending_download: built.pending_download,
            page_creation_artifacts: built.page_creation_artifacts,
            requested_url: requested_url.clone(),
            final_url: requested_url,
            request_method,
            request_headers,
            response_status,
            response_headers,
            response_from_cache: false,
            initial_runtime_realms: diagnostics.initial_runtime_realms,
            renderer_output_predecessor: diagnostics.renderer_output_predecessor,
            main_document_commit,
            document_progress_transfer: CompletedDocumentProgressTransfer::new_captured(
                captured_response_body.unwrap_or_else(|| CapturedBody::from_string(response_body)),
                false,
                network_progress,
            ),
            navigation_engine: None,
            network_error_page: None,
        })
    }

    pub async fn fetch_navigation_response_async(
        &mut self,
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<NavigationResponse>, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(None);
        ensure_url_not_blocked_for_load_inputs(&load_inputs, raw_url)?;
        if load_inputs.network_offline {
            return Err("Network emulation offline".to_owned());
        }
        let resource_storage = load_inputs.resource_storage_handles();
        self.engine
            .set_bypass_service_worker(load_inputs.bypass_service_worker);
        self.engine
            .fetch_navigation_response_with_storage_async(
                resource_storage.into_navigation_storage(),
                load_inputs.navigation_initiator_url.as_ref(),
                load_inputs.browser_navigation_kind,
                load_inputs.infer_navigation_referrer,
                method,
                raw_url,
                body,
                request_headers,
                auth,
            )
            .await
            .map_err(|error| format!("failed to fetch page `{raw_url}`: {error}"))
    }

    pub(crate) async fn fetch_navigation_auth_raw_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        request_load_policy: NavigationRequestLoadPolicy,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        auth: SubresourceAuthCredentials,
    ) -> Result<NetworkFetchResult<RawResponse>, String> {
        let load_inputs = apply_navigation_request_load_policy(
            self.navigation_load_inputs_for_session_owner(session_id),
            request_load_policy,
        );
        ensure_url_not_blocked_for_load_inputs(&load_inputs, raw_url)?;
        if load_inputs.network_offline {
            return Err("Network emulation offline".to_owned());
        }

        let mut request = Request::new_bytes(method, raw_url, body, request_headers)
            .map_err(|error| format!("failed to build request for `{raw_url}`: {error}"))?
            .with_top_level_navigation_cookie_context()
            .with_browser_navigation_kind(load_inputs.browser_navigation_kind);
        if !load_inputs.infer_navigation_referrer {
            request = request.without_inferred_referrer();
        }
        if let Some(initiator_url) = load_inputs.navigation_initiator_url.as_ref() {
            request = request.with_initiator_url(initiator_url);
        }
        request.set_auth(Some(auth.into()));

        let loader = self
            .ensure_resource_request_client_for_navigation_load_inputs(&load_inputs)?
            .clone();
        loader
            .fetch_raw_with_network_metadata(request)
            .await
            .map_err(|error| format!("failed to fetch page `{raw_url}`: {error}"))
    }

    pub async fn fetch_navigation_streaming_raw_response_async(
        &mut self,
        method: &str,
        raw_url: &str,
        body: Option<String>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(None);
        self.fetch_navigation_streaming_raw_response_with_load_inputs_async(
            &load_inputs,
            method,
            raw_url,
            body.map(String::into_bytes),
            request_headers,
            auth,
        )
        .await
    }

    pub(crate) async fn fetch_navigation_streaming_raw_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        request_load_policy: NavigationRequestLoadPolicy,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
        let load_inputs = apply_navigation_request_load_policy(
            self.navigation_load_inputs_for_session_owner(session_id),
            request_load_policy,
        );
        self.fetch_navigation_streaming_raw_response_with_load_inputs_async(
            &load_inputs,
            method,
            raw_url,
            body,
            request_headers,
            auth,
        )
        .await
    }

    async fn fetch_navigation_streaming_raw_response_with_load_inputs_async(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
        ensure_url_not_blocked_for_load_inputs(load_inputs, raw_url)?;
        if load_inputs.network_offline {
            return Err("Network emulation offline".to_owned());
        }

        let mut request = Request::new_bytes(method, raw_url, body, request_headers)
            .map_err(|error| format!("failed to build request for `{raw_url}`: {error}"))?
            .with_top_level_navigation_cookie_context()
            .with_browser_navigation_kind(load_inputs.browser_navigation_kind);
        if !load_inputs.infer_navigation_referrer {
            request = request.without_inferred_referrer();
        }
        if let Some(initiator_url) = load_inputs.navigation_initiator_url.as_ref() {
            request = request.with_initiator_url(initiator_url);
        }
        request.set_auth(auth.map(Into::into));

        let loader = self
            .ensure_resource_request_client_for_navigation_load_inputs(load_inputs)?
            .clone();
        loader
            .fetch_raw_stream_with_cancel_and_network_metadata(request, FetchCancelHandle::new())
            .await
            .map_err(|error| format!("failed to fetch page `{raw_url}`: {error}"))
    }

    pub async fn build_navigation_from_network_response_async(
        &mut self,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: NetworkFetchResult<NavigationResponse>,
    ) -> Result<LoadedNavigation, String> {
        self.build_navigation_from_network_response_for_session_owner_async(
            None,
            requested_url,
            request_method,
            request_headers,
            response,
        )
        .await
    }

    pub(crate) async fn build_navigation_from_network_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: NetworkFetchResult<NavigationResponse>,
    ) -> Result<LoadedNavigation, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        let (response, network_observation_journal) =
            response.into_parts_with_observation_journal();
        let network_extra_info_available = !network_observation_journal.is_empty();
        let (fetch_subresource_interception_enabled, fetch_subresource_interception_resource_type) =
            load_inputs.fetch_subresource_interception;
        let (response_head, response_body, response_body_bytes) = response.into_parts();
        let final_url = response_head.final_url.clone();
        let response_status = response_head.status;
        let response_headers = response_head.headers.clone();
        let captured_response_body = CapturedBody::from_bytes(response_body_bytes);
        let initial_request_cookie_report = response_head.request_cookie_report.clone();
        let response_cookie_reports = response_head.cookie_set_reports.clone();
        let response_from_cache = response_head.from_cache;
        let negotiated_http_version = response_head.negotiated_http_version;
        let redirected = response_head.redirected;
        let redirect_chain: Vec<_> = response_head
            .redirect_chain
            .clone()
            .into_iter()
            .map(Into::into)
            .collect();
        let page_storage = load_inputs.page_storage_handles();
        let main_document_commit = load_inputs
            .main_document_commit_for_final_url(&final_url, None)
            .map(Arc::new);
        let built = self
            .engine
            .build_html_page_from_response_with_storage_and_inspector_session_restores_async(
                page_storage.into_navigation_storage(),
                requested_url.clone(),
                final_url.clone(),
                load_inputs.navigation_initiator_url.clone(),
                redirected,
                redirect_chain.len(),
                response_status,
                response_headers.clone(),
                response_body,
                load_inputs.document_start_scripts.clone(),
                load_inputs.runtime_bindings.clone(),
                load_inputs
                    .runtime_inspector_session_restore_snapshots
                    .clone(),
                load_inputs.extra_http_headers.clone(),
                load_inputs.locale_override.clone(),
                load_inputs.timezone_override.clone(),
                load_inputs.script_execution_disabled,
                load_inputs.bypass_content_security_policy,
                load_inputs.cpu_throttling_rate,
                load_inputs.emulated_media.clone(),
                load_inputs.viewport_surface,
                load_inputs.network_offline,
                load_inputs.blocked_url_patterns.clone(),
                fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type,
                load_inputs.root_frame_id.clone(),
                main_document_commit.as_deref().cloned(),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to execute scripts for page `{}`: {error}",
                    requested_url
                )
            })?;
        let diagnostics = loaded_page_creation_diagnostics_parts(built.page_creation_diagnostics);
        let mut page = built.page;
        apply_navigation_load_input_overrides_async(
            &mut page,
            &load_inputs,
            NavigationLoadInputOverrideMode::FreshlyBuiltPage,
        )
        .await?;
        let network_progress = MainDocumentBodyNetworkProgress::CompletedBody(Box::new(
            CompletedMainDocumentNetworkEvents::new(
                request_method.clone(),
                request_headers.clone(),
                initial_request_cookie_report.clone(),
                response_status,
                response_headers.clone(),
                response_cookie_reports.clone(),
                redirect_chain.clone(),
                network_extra_info_available,
                response_from_cache,
            )
            .with_negotiated_http_version(negotiated_http_version)
            .with_network_observation_journal(network_observation_journal),
        ));

        Ok(LoadedNavigation {
            page,
            pending_download: built.pending_download,
            page_creation_artifacts: built.page_creation_artifacts,
            requested_url,
            final_url,
            request_method,
            request_headers,
            response_status,
            response_headers,
            response_from_cache,
            initial_runtime_realms: diagnostics.initial_runtime_realms,
            renderer_output_predecessor: diagnostics.renderer_output_predecessor,
            main_document_commit,
            document_progress_transfer: CompletedDocumentProgressTransfer::new_captured(
                captured_response_body,
                false,
                network_progress,
            ),
            navigation_engine: None,
            network_error_page: None,
        })
    }

    /// Builds navigation from raw bytes that are already fully buffered.
    ///
    /// This keeps buffered/synthetic cases explicit. It is not the main network
    /// document path; true network responses should use the streaming raw
    /// builders so parser work can start before body EOF.
    pub async fn build_navigation_from_buffered_raw_response_async(
        &mut self,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: RawResponse,
    ) -> Result<NavigationLoadOutcome, String> {
        self.build_navigation_from_buffered_raw_response_for_session_owner_async(
            None,
            requested_url,
            request_method,
            request_headers,
            NetworkFetchResult::without_request_observation(response),
        )
        .await
    }

    pub(crate) async fn build_navigation_from_buffered_raw_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: NetworkFetchResult<RawResponse>,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        self.build_navigation_from_buffered_raw_response_with_load_inputs_async(
            session_id,
            &load_inputs,
            requested_url,
            request_method,
            request_headers,
            response,
        )
        .await
    }

    pub(crate) async fn build_navigation_from_buffered_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        response: NetworkFetchResult<RawResponse>,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_buffered_raw_response_with_load_inputs_async(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            navigation.requested_url.clone(),
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            response,
        )
        .await
    }

    async fn build_navigation_from_buffered_raw_response_with_load_inputs_async(
        &mut self,
        session_id: Option<&str>,
        load_inputs: &TargetNavigationLoadInputs,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: NetworkFetchResult<RawResponse>,
    ) -> Result<NavigationLoadOutcome, String> {
        let (response, network_observation_journal) =
            response.into_parts_with_observation_journal();
        if super::downloads::response_headers_indicate_download(&response.headers) {
            return Ok(NavigationLoadOutcome::download(
                self.build_download_from_raw_response(
                    request_method,
                    request_headers,
                    response,
                    network_observation_journal,
                ),
            ));
        }

        let head = response.head();
        let body = CapturedBody::from_bytes(response.clone_body_bytes());
        self.build_navigation_from_captured_raw_response_with_load_inputs_async(
            session_id,
            load_inputs,
            requested_url,
            request_method,
            request_headers,
            head,
            body,
            network_observation_journal,
            MainDocumentBodyProgressSource::default(),
        )
        .await
    }

    pub(crate) async fn build_navigation_from_captured_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        head: ResponseHead,
        body: CapturedBody,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_captured_raw_response_with_load_inputs_async(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            navigation.requested_url.clone(),
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            head,
            body,
            network_observation_journal,
            body_progress_source,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_navigation_from_captured_raw_response_with_load_inputs_async(
        &mut self,
        session_id: Option<&str>,
        load_inputs: &TargetNavigationLoadInputs,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        head: ResponseHead,
        body: CapturedBody,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        if super::downloads::response_headers_indicate_download(&head.headers) {
            let body_bytes = body.materialize_bytes().map_err(|error| {
                format!("failed to materialize captured download body: {error}")
            })?;
            return Ok(NavigationLoadOutcome::download(
                self.build_download_from_raw_response(
                    request_method,
                    request_headers,
                    RawResponse::from_head_and_body(head, body_bytes),
                    network_observation_journal,
                ),
            ));
        }

        let page_reservation = self.engine.reserve_page_for_creation();
        self.bind_renderer_page_reservation_for_session_owner(
            session_id,
            load_inputs,
            page_reservation,
        );
        prepare_navigation_from_captured_raw_response_with_engine_async(
            &mut self.engine,
            page_reservation,
            load_inputs,
            requested_url,
            request_method,
            request_headers,
            head,
            body,
            body_progress_source,
            network_observation_journal,
            None,
            false,
            RendererReplyBoundary::Stage,
        )
        .await
        .map(NavigationLoadOutcome::response_commit_ready)
    }

    pub async fn build_navigation_from_streaming_raw_response_async(
        &mut self,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: StreamingRawResponse,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        self.build_navigation_from_streaming_raw_response_for_session_owner_async(
            None,
            requested_url,
            request_method,
            request_headers,
            NetworkFetchResult::without_request_observation(response),
            body_progress_source,
        )
        .await
    }

    pub(crate) async fn build_navigation_from_streaming_raw_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: NetworkFetchResult<StreamingRawResponse>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        self.build_navigation_from_streaming_raw_response_with_load_inputs_async(
            session_id,
            &load_inputs,
            requested_url,
            request_method,
            request_headers,
            response,
            None,
            Vec::new(),
            body_progress_source,
        )
        .await
    }

    pub(crate) async fn build_navigation_from_streaming_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        response: NetworkFetchResult<StreamingRawResponse>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_streaming_raw_response_with_load_inputs_async(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            navigation.requested_url.clone(),
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            response,
            None,
            Vec::new(),
            body_progress_source,
        )
        .await
    }

    pub(crate) async fn build_navigation_from_streaming_raw_response_with_response_override_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        response: NetworkFetchResult<StreamingRawResponse>,
        response_code: Option<u16>,
        response_headers_override: Vec<(String, String)>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_streaming_raw_response_with_load_inputs_async(
            navigation.navigate_session_id.as_deref(),
            &load_inputs,
            navigation.requested_url.clone(),
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            response,
            response_code,
            response_headers_override,
            body_progress_source,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_navigation_from_streaming_raw_response_with_load_inputs_async(
        &mut self,
        session_id: Option<&str>,
        load_inputs: &TargetNavigationLoadInputs,
        requested_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: NetworkFetchResult<StreamingRawResponse>,
        response_code: Option<u16>,
        response_headers_override: Vec<(String, String)>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let (response, network_observation_journal) =
            response.into_parts_with_observation_journal();
        if load_inputs.browser_context_id.is_none() {
            let page_reservation = self.engine.reserve_page_for_creation();
            return build_navigation_from_streaming_raw_response_with_engine_async(
                &mut self.engine,
                page_reservation,
                load_inputs,
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
                response_code,
                response_headers_override,
                body_progress_source,
                None,
                None,
                CommittedDocumentResourceSource::Synthetic,
                RendererReplyBoundary::Stage,
            )
            .await;
        }

        let mut engine = self.background_navigation_engine_for_load_inputs(load_inputs);
        let page_reservation =
            self.reserve_renderer_page_for_session_owner(session_id, load_inputs, &engine);
        let navigation = build_navigation_from_streaming_raw_response_with_engine_async(
            &mut engine,
            page_reservation,
            load_inputs,
            requested_url,
            request_method,
            request_headers,
            response,
            network_observation_journal,
            response_code,
            response_headers_override,
            body_progress_source,
            None,
            None,
            CommittedDocumentResourceSource::Synthetic,
            RendererReplyBoundary::Stage,
        )
        .await?;
        Ok(navigation.with_navigation_engine(engine))
    }

    pub(crate) async fn collect_navigation_streaming_raw_response_async(
        &mut self,
        response: NetworkFetchResult<StreamingRawResponse>,
    ) -> Result<NetworkFetchResult<RawResponse>, String> {
        let (response, network_observation_journal) =
            response.into_parts_with_observation_journal();
        let response = response
            .into_materialized_raw_response()
            .await
            .map_err(|error| format!("failed to read page body from stream: {error}"))?;
        Ok(NetworkFetchResult::with_observation_journal(
            response,
            network_observation_journal,
        ))
    }

    fn build_download_from_raw_response(
        &self,
        request_method: String,
        request_headers: Vec<(String, String)>,
        response: RawResponse,
        network_observation_journal: NetworkObservationJournal,
    ) -> DownloadNavigation {
        let network_extra_info_available = !network_observation_journal.is_empty();
        let (head, body) = response.into_body();
        let body = body
            .try_into_materialized_bytes()
            .expect("RawResponse body should remain materialized at the download boundary");
        let response_from_cache = head.from_cache;
        let negotiated_http_version = head.negotiated_http_version;
        let final_url = head.final_url;
        let network_events = CompletedMainDocumentNetworkEvents::new(
            request_method,
            request_headers,
            head.request_cookie_report,
            head.status,
            head.headers,
            head.cookie_set_reports,
            head.redirect_chain.into_iter().map(Into::into).collect(),
            network_extra_info_available,
            response_from_cache,
        )
        .with_negotiated_http_version(negotiated_http_version)
        .with_network_observation_journal(network_observation_journal);
        DownloadNavigation {
            final_url,
            progress_transfer: CompletedDownloadProgressTransfer::new(body, network_events),
        }
    }

    #[cfg(test)]
    pub(crate) fn current_navigation_initiator_url(&self) -> Option<Url> {
        let browser_context = self.browser_context.as_ref()?;

        if let Some(loaded_page) = browser_context.loaded_page() {
            let url = loaded_page.final_url().clone();
            if url.host_str().is_some() {
                return Some(url);
            }
        }

        let url = Url::parse(browser_context.target_url()).ok()?;
        url.host_str().is_some().then_some(url)
    }
}

async fn prepare_navigation_from_captured_raw_response_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    requested_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    head: ResponseHead,
    body: CapturedBody,
    body_progress_source: MainDocumentBodyProgressSource,
    network_observation_journal: NetworkObservationJournal,
    network_error_page: Option<NetworkErrorPageNavigation>,
    synthetic_body: bool,
    reply_boundary: RendererReplyBoundary,
) -> Result<ResponseCommitReady, String> {
    let network_extra_info_available = !network_observation_journal.is_empty();
    if network_error_page.is_none()
        && response_status_may_use_http_error_page(head.status)
        && body.len() == 0
    {
        body_progress_source.emit_response_metadata(
            &request_method,
            &request_headers,
            head.request_cookie_report.as_ref(),
            &head.redirect_chain,
            &head.final_url,
            head.status,
            &head.headers,
            &head.cookie_set_reports,
            &network_observation_journal,
            network_extra_info_available,
            head.from_cache,
            head.negotiated_http_version,
        );
        let status = head.status;
        let unreachable_url = head.final_url;
        let body = CapturedBody::from_string(http_error_page_html(&unreachable_url, status));
        return prepare_browser_owned_error_page_navigation_with_engine_async(
            engine,
            page_reservation,
            load_inputs,
            unreachable_url,
            request_method,
            request_headers,
            HTTP_RESPONSE_CODE_FAILURE_ERROR_TEXT.to_owned(),
            body,
            reply_boundary,
        )
        .await;
    }
    prepare_captured_document_response_with_engine_async(
        engine,
        page_reservation,
        load_inputs,
        requested_url,
        request_method,
        request_headers,
        head,
        body,
        body_progress_source,
        network_observation_journal,
        network_error_page,
        synthetic_body,
        reply_boundary,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn prepare_captured_document_response_with_engine_async(
    engine: &mut NavigationEngine,
    page_reservation: RendererPageReservationToken,
    load_inputs: &TargetNavigationLoadInputs,
    requested_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    head: ResponseHead,
    body: CapturedBody,
    body_progress_source: MainDocumentBodyProgressSource,
    network_observation_journal: NetworkObservationJournal,
    network_error_page: Option<NetworkErrorPageNavigation>,
    synthetic_body: bool,
    reply_boundary: RendererReplyBoundary,
) -> Result<ResponseCommitReady, String> {
    let network_extra_info_available = !network_observation_journal.is_empty();
    body_progress_source.emit_response_metadata(
        &request_method,
        &request_headers,
        head.request_cookie_report.as_ref(),
        &head.redirect_chain,
        &head.final_url,
        head.status,
        &head.headers,
        &head.cookie_set_reports,
        &network_observation_journal,
        network_extra_info_available,
        head.from_cache,
        head.negotiated_http_version,
    );
    let (fetch_subresource_interception_enabled, fetch_subresource_interception_resource_type) =
        load_inputs.fetch_subresource_interception;
    let response_from_cache = head.from_cache;
    let negotiated_http_version = head.negotiated_http_version;
    let final_url = head.final_url;
    let response_status = head.status;
    let response_headers = head.headers;
    let initial_request_cookie_report = head.request_cookie_report;
    let response_cookie_reports = head.cookie_set_reports;
    let redirected = head.redirected;
    let redirect_chain = head
        .redirect_chain
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    let body_network_progress_state = body_progress_source
        .body_network_progress_for_completed_events(
            CompletedMainDocumentNetworkEvents::new(
                request_method.clone(),
                request_headers.clone(),
                initial_request_cookie_report,
                response_status,
                response_headers.clone(),
                response_cookie_reports,
                redirect_chain.clone(),
                network_extra_info_available,
                response_from_cache,
            )
            .with_negotiated_http_version(negotiated_http_version)
            .with_network_observation_journal(network_observation_journal),
        );

    let (body_tx, body_rx) = mpsc::channel(EXTERNAL_RAW_BODY_CHANNEL_CAPACITY);
    let (completion_tx, completion_rx) = oneshot::channel();
    let raw_body = moli_core::runtime::ExternalRawDocumentBodyStream::new(body_rx, completion_rx);
    let page_storage = load_inputs.page_storage_handles();
    let main_document_commit = load_inputs
        .main_document_commit_for_final_url(&final_url, network_error_page.as_ref())
        .map(Arc::new);
    let prepared_future = engine
        .prepare_streaming_raw_page_from_external_body_with_storage_and_inspector_session_restores_async(
            page_reservation,
            page_storage.into_navigation_storage(),
            requested_url.clone(),
            final_url.clone(),
            load_inputs.navigation_initiator_url.clone(),
            redirected,
            redirect_chain.len(),
            response_status,
            response_headers.clone(),
            raw_body,
            load_inputs.document_start_scripts.clone(),
            load_inputs.runtime_bindings.clone(),
            load_inputs
                .runtime_inspector_session_restore_snapshots
                .clone(),
            load_inputs.extra_http_headers.clone(),
            load_inputs.locale_override.clone(),
            load_inputs.timezone_override.clone(),
            load_inputs.script_execution_disabled,
            load_inputs.bypass_content_security_policy,
            load_inputs.cpu_throttling_rate,
            load_inputs.emulated_media.clone(),
            load_inputs.viewport_surface,
            load_inputs.network_offline,
            load_inputs.blocked_url_patterns.clone(),
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            PageVmInitStage::DomContentLoaded,
            reply_boundary,
            load_inputs.root_frame_id.clone(),
            CommittedDocumentResourceSource::Synthetic,
            None,
            main_document_commit.as_deref().cloned(),
        );
    let body_capture_task = spawn_captured_body_replay(body, body_tx, completion_tx);
    let prepared_page = match prepared_future.await {
        Ok(prepared_page) => prepared_page,
        Err(error) => {
            body_capture_task.abort();
            return Err(format!(
                "failed to prepare captured page `{}`: {error:#}",
                requested_url
            ));
        }
    };

    Ok(ResponseCommitReady {
        prepared_page: Some(prepared_page),
        body_capture: Some(ResponseCommitBodyCapture::Pending(body_capture_task)),
        body_completion_sink: None,
        body_progress_source,
        body_network_progress_state: Some(body_network_progress_state),
        synthetic_body,
        requested_url,
        final_url,
        request_method,
        request_headers,
        response_status,
        response_headers,
        response_from_cache,
        navigation_engine: None,
        timing_started: None,
        main_document_commit,
        network_error_page,
    })
}

fn ensure_url_not_blocked_for_load_inputs(
    load_inputs: &TargetNavigationLoadInputs,
    raw_url: &str,
) -> Result<(), String> {
    if load_inputs
        .blocked_url_patterns
        .iter()
        .any(|pattern| url_pattern_matches(pattern, raw_url))
    {
        Err(BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigationLoadInputOverrideMode {
    FreshlyBuiltPage,
    ExistingPage,
}

async fn apply_navigation_load_input_overrides_async(
    page: &mut moli_core::page::Page,
    load_inputs: &TargetNavigationLoadInputs,
    mode: NavigationLoadInputOverrideMode,
) -> Result<(), String> {
    if !load_inputs.permission_overrides.is_empty()
        || mode == NavigationLoadInputOverrideMode::ExistingPage
    {
        page.set_permission_overrides_async(&load_inputs.permission_overrides)
            .await
            .map_err(|error| format!("failed to apply page permission overrides: {error}"))?;
    }
    if mode == NavigationLoadInputOverrideMode::FreshlyBuiltPage {
        return Ok(());
    }
    page.set_locale_override_async(load_inputs.locale_override.as_deref())
        .await
        .map_err(|error| format!("failed to apply page locale override: {error}"))?;
    page.set_timezone_override_async(load_inputs.timezone_override.as_deref())
        .await
        .map_err(|error| format!("failed to apply page timezone override: {error}"))?;
    page.set_script_execution_disabled_async(load_inputs.script_execution_disabled)
        .await
        .map_err(|error| format!("failed to apply page script execution override: {error}"))?;
    page.set_bypass_content_security_policy_async(load_inputs.bypass_content_security_policy)
        .await
        .map_err(|error| format!("failed to apply page CSP bypass override: {error}"))?;
    page.set_cpu_throttling_rate_async(load_inputs.cpu_throttling_rate)
        .await
        .map_err(|error| format!("failed to apply page CPU throttling rate: {error}"))?;
    page.set_emulated_media_async(&load_inputs.emulated_media)
        .await
        .map_err(|error| format!("failed to apply page emulated media: {error}"))?;
    page.set_viewport_surface_async(load_inputs.viewport_surface)
        .await
        .map_err(|error| format!("failed to apply page viewport surface: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BackgroundNavigationEarlyResult, decode_data_url_body, decode_data_url_response,
        decode_text_html_data_url, decoded_data_url_navigation_response,
        inline_html_navigation_source,
    };
    use serde_json::json;

    #[test]
    fn decode_text_html_data_url_uses_data_url_processor() {
        assert_eq!(
            decode_text_html_data_url("data:text/html,%3Cmain%3Edecoded%3C/main%3E")
                .expect("text/html data url")
                .expect("decoded body"),
            "<main>decoded</main>"
        );
        assert_eq!(
            decode_text_html_data_url(
                "data:text/html;charset=utf-8;base64,PHRpdGxlPmI2NDwvdGl0bGU+"
            )
            .expect("text/html data url")
            .expect("decoded body"),
            "<title>b64</title>"
        );
        assert_eq!(
            decode_text_html_data_url("data:text/html,<style>#x{display:flex}</style>")
                .expect("legacy raw text/html data url")
                .expect("decoded body"),
            "<style>#x{display:flex}</style>"
        );
        assert!(decode_text_html_data_url("data:text/plain,plain").is_none());
    }

    #[test]
    fn inline_html_data_url_navigation_response_reports_content_type() {
        let source = inline_html_navigation_source("data:text/html,<main>hello</main>")
            .expect("inline html data url")
            .expect("navigation source");

        assert_eq!(source.html, "<main>hello</main>");
        assert_eq!(
            source.response_headers,
            vec![("Content-Type".to_owned(), "text/html".to_owned())]
        );
    }

    #[test]
    fn decode_data_url_body_handles_plain_and_base64_payloads() {
        assert_eq!(
            decode_data_url_body("data:,hello%20world#fragment")
                .expect("data url")
                .expect("decoded body"),
            b"hello world"
        );
        assert_eq!(
            decode_data_url_body("data:application/octet-stream;base64,AP9h")
                .expect("data url")
                .expect("decoded body"),
            vec![0, 255, b'a']
        );
    }

    #[test]
    fn decode_data_url_response_reports_mime_type() {
        let plain = decode_data_url_response("data:,hello%20world")
            .expect("data url")
            .expect("decoded body");
        assert_eq!(plain.content_type, "text/plain;charset=US-ASCII");
        assert_eq!(plain.body, b"hello world");

        let binary = decode_data_url_response("data:application/octet-stream;base64,AP9h")
            .expect("data url")
            .expect("decoded body");
        assert_eq!(binary.content_type, "application/octet-stream");
        assert_eq!(binary.body, vec![0, 255, b'a']);
    }

    #[test]
    fn decoded_data_url_navigation_response_builds_synthetic_raw_response() {
        let navigation_response =
            decoded_data_url_navigation_response("data:image/png;base64,AP9h")
                .expect("data url")
                .expect("navigation response");

        assert_eq!(
            navigation_response.requested_url.as_str(),
            "data:image/png;base64,AP9h"
        );
        assert_eq!(navigation_response.response.status, 200);
        assert_eq!(
            navigation_response.response.headers,
            vec![("Content-Type".to_owned(), "image/png".to_owned())]
        );
        assert_eq!(navigation_response.response.body_bytes(), &[0, 255, b'a']);
        assert!(navigation_response.response.request_cookie_report.is_none());
        assert!(navigation_response.response.cookie_set_reports.is_empty());
        assert!(!navigation_response.response.redirected);
        assert!(navigation_response.response.redirect_chain.is_empty());
    }

    #[test]
    fn background_navigation_early_result_emits_typed_command_response() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let early_result = BackgroundNavigationEarlyResult::new(
            sender,
            42,
            Some("SID-nav".to_owned()),
            json!({ "frameId": "FRAME-1", "loaderId": "LOADER-1" }),
        );

        assert!(early_result.emit());
        let event = receiver
            .try_recv()
            .expect("early navigation result should be sent");

        assert_eq!(event.protocol_message_id(), Some(42));
        assert!(
            event.protocol_message().is_none(),
            "early Page.navigate result should stay as a typed command response until wire projection"
        );
        assert_eq!(
            event.into_protocol_message(),
            json!({
                "id": 42,
                "result": { "frameId": "FRAME-1", "loaderId": "LOADER-1" },
                "sessionId": "SID-nav",
            })
        );
    }
}
