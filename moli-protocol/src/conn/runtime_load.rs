use data_url::DataUrl;
use moli_core::{
    RendererOutputFence,
    page::{RendererMainDocumentCommit, RendererPageCreationDiagnostics, RendererRuntimeRealmInfo},
    runtime::{CommittedDocumentResourceSource, PageVmInitStage, RendererReplyBoundary},
};
#[cfg(test)]
use moli_core::{
    page::{NavigationResponse, Page},
    runtime::PreparedDocumentPagePolicy,
};
use moli_fetch::{
    BrowserNavigationRequestKind, FetchConfig, NetworkFetchFailureContext, NetworkFetchResult,
    NetworkObservationJournal, RawResponse, ResponseHead, StreamingRawResponse,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::*;
use crate::conn::state::InitialDocumentPageBuildWaiter;
use crate::conn::state::{AdmittedNavigationLoad, PreparedNavigationResponse};
use crate::domains::network::{
    CompletedDocumentProgressTransfer, CompletedDownloadProgressTransfer,
    CompletedMainDocumentNetworkEvents, MainDocumentBodyNetworkProgress,
    MainDocumentBodyProgressSource,
};

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
async fn prepare_browser_owned_error_page_navigation_with_load_async(
    load: &mut AdmittedNavigationLoad,
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
    prepare_captured_document_response_with_load_async(
        load,
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
async fn prepare_network_error_page_navigation_with_load_async(
    load: &mut AdmittedNavigationLoad,
    load_inputs: &TargetNavigationLoadInputs,
    unreachable_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    error_text: String,
    reply_boundary: RendererReplyBoundary,
) -> Result<NavigationLoadOutcome, String> {
    let body = CapturedBody::from_string(network_error_page_html(&unreachable_url, &error_text));
    prepare_browser_owned_error_page_navigation_with_load_async(
        load,
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

pub(crate) struct PendingInitialDocumentPageBuild {
    kind: PendingInitialDocumentPageBuildKind,
}

enum PendingInitialDocumentPageBuildKind {
    Build {
        key: crate::conn::state::InitialDocumentBuildKey,
        pending: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = anyhow::Result<crate::conn::state::BuiltInitialDocument>,
                    > + Send,
            >,
        >,
    },
    Join {
        waiter: InitialDocumentPageBuildWaiter,
    },
}

pub(crate) enum CompletedInitialDocumentPageBuild {
    Built(Box<crate::conn::state::BuiltInitialDocument>),
    Joined,
}

#[derive(Debug)]
pub(crate) struct FailedInitialDocumentPageBuild {
    key: Option<crate::conn::state::InitialDocumentBuildKey>,
    message: String,
}

impl std::fmt::Display for FailedInitialDocumentPageBuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl PendingInitialDocumentPageBuild {
    pub async fn wait(
        self,
    ) -> Result<CompletedInitialDocumentPageBuild, FailedInitialDocumentPageBuild> {
        match self.kind {
            PendingInitialDocumentPageBuildKind::Build { key, pending } => pending
                .await
                .map(|built| CompletedInitialDocumentPageBuild::Built(Box::new(built)))
                .map_err(|error| FailedInitialDocumentPageBuild {
                    key: Some(key),
                    message: format!("initial document page build failed: {error}"),
                }),
            PendingInitialDocumentPageBuildKind::Join { waiter } => waiter
                .wait()
                .await
                .map(|()| CompletedInitialDocumentPageBuild::Joined)
                .map_err(|message| FailedInitialDocumentPageBuild { key: None, message }),
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
    prepared_page: Option<PreparedNavigationResponse>,
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
                    .map(PreparedNavigationResponse::renderer_devtools_agent_token),
            )
            .finish_non_exhaustive()
    }
}

impl ResponseCommitReady {
    #[cfg(test)]
    pub(crate) fn final_url(&self) -> &Url {
        &self.final_url
    }

    #[cfg(test)]
    pub(crate) async fn materialize(
        mut self,
        policy: Option<PreparedDocumentPagePolicy>,
        mut inspection: moli_renderer_v8::RendererPreparedDocumentInspectionConfiguration,
    ) -> Result<LoadedNavigation, String> {
        let prepared_page = self
            .prepared_page
            .take()
            .expect("response commit-ready value must retain its prepared Page");
        // Admit the service-owned bootstrap update without waiting on it to
        // authorize the Browser operation. Both use this exact renderer owner.
        inspection.main_document_commit = self.main_document_commit.as_deref().cloned();
        let inspection_ack = prepared_page
            .inspection_configuration_endpoint()
            .start_configure(inspection);
        let built = prepared_page.materialize(policy).await;
        if let Err(error) = inspection_ack.await {
            tracing::warn!(%error, "prepared document inspection configuration failed");
        }
        self.finish_materialization(built).await
    }

    async fn finish_materialization<P>(
        mut self,
        built: anyhow::Result<moli_core::runtime::BuiltDocumentPage<P>>,
    ) -> Result<LoadedNavigation<P>, String> {
        let built = match built {
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
            #[cfg(test)]
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
            #[cfg(test)]
            main_document_commit: self.main_document_commit.take(),
            document_progress_transfer,
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
    prepared_page: PreparedNavigationResponse,
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
    ) -> NavigationLoadOutcome {
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
            timing_started: self.timing_started,
            main_document_commit: self.main_document_commit,
            network_error_page: None,
        };
        NavigationLoadOutcome::response_commit_ready(ready)
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
    load: AdmittedNavigationLoad,
    reply_boundary: RendererReplyBoundary,
    early_result: Option<BackgroundNavigationEarlyResult>,
    load_inputs: TargetNavigationLoadInputs,
    method: String,
    raw_url: String,
    body: Option<Vec<u8>>,
    request_headers: Vec<(String, String)>,
    body_progress_source: MainDocumentBodyProgressSource,
}

pub(crate) struct BackgroundStreamingResponseNavigationLoadJob {
    load: AdmittedNavigationLoad,
    load_inputs: TargetNavigationLoadInputs,
    requested_url: Url,
    request_method: String,
    request_headers: Vec<(String, String)>,
    response: StreamingRawResponse,
    network_observation_journal: NetworkObservationJournal,
    response_code: Option<u16>,
    response_headers_override: Vec<(String, String)>,
    body_progress_source: MainDocumentBodyProgressSource,
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
    token: NavigationId,
    state: NavigationDispatchState,
}

impl BackgroundNavigationBodyCompletionSink {
    pub(crate) fn new(
        sender: tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundNavigationCompletion,
        >,
        token: NavigationId,
        state: NavigationDispatchState,
    ) -> Self {
        Self {
            sender,
            token,
            state,
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
    ) -> (Result<NavigationLoadOutcome, String>, bool) {
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let timing_url = self.raw_url.clone();
        let mut load = self.load;
        let mut early_result = self.early_result.take();
        let mut early_result_sent = false;
        let navigation = async {
            if let Some(navigation) = load_inline_html_navigation_with_load_async(
                &mut load,
                &self.load_inputs,
                &self.method,
                &self.raw_url,
                self.request_headers.clone(),
                self.reply_boundary,
            )
            .await
            {
                early_result_sent =
                    Self::emit_early_result_for_successful_document(&mut early_result, &navigation);
                return navigation;
            }

            if let Some(navigation) = load_data_url_navigation_with_load_async(
                &mut load,
                &self.load_inputs,
                &self.method,
                &self.raw_url,
                self.request_headers.clone(),
                self.reply_boundary,
            )
            .await
            {
                early_result_sent =
                    Self::emit_early_result_for_successful_document(&mut early_result, &navigation);
                return navigation;
            }

            load.validate_request(&self.raw_url)
                .map_err(|error| error.to_string())?;

            let requested_url = Url::parse(&self.raw_url).map_err(|error| {
                format!("failed to parse request url `{}`: {error}", self.raw_url)
            })?;
            let navigation_response = load
                .fetch_navigation(
                    &self.method,
                    &self.raw_url,
                    self.body,
                    self.request_headers.clone(),
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
                        return prepare_network_error_page_navigation_with_load_async(
                            &mut load,
                            &self.load_inputs,
                            unreachable_url,
                            self.method,
                            self.request_headers,
                            failure.network_error_text().to_owned(),
                            self.reply_boundary,
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
            let navigation = build_navigation_from_streaming_raw_response_with_load_async(
                &mut load,
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
                self.reply_boundary,
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
        (navigation, early_result_sent)
    }
}

impl BackgroundStreamingResponseNavigationLoadJob {
    pub(crate) async fn run(
        self,
        body_completion_sink: Option<BackgroundNavigationBodyCompletionSink>,
    ) -> Result<NavigationLoadOutcome, String> {
        let mut load = self.load;
        build_navigation_from_streaming_raw_response_with_load_async(
            &mut load,
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
        .await
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

async fn load_inline_html_navigation_with_load_async(
    load: &mut AdmittedNavigationLoad,
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
            prepare_navigation_from_captured_raw_response_with_load_async(
                load,
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

async fn load_data_url_navigation_with_load_async(
    load: &mut AdmittedNavigationLoad,
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
            prepare_navigation_from_captured_raw_response_with_load_async(
                load,
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

async fn build_navigation_from_streaming_raw_response_with_load_async(
    load: &mut AdmittedNavigationLoad,
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
                return prepare_browser_owned_error_page_navigation_with_load_async(
                    load,
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
        let main_document_commit = load_inputs
            .main_document_commit_for_final_url(&final_url, None)
            .map(Arc::new);
        let prepared_page = load
            .prepare_document_response_async(
                requested_url.clone(),
                final_url.clone(),
                redirected,
                redirect_chain.len(),
                response_status,
                response_headers.clone(),
                moli_core::runtime::ExternalRawDocumentBodyStream::from_bytes(
                    response_text.into_bytes(),
                ),
                PageVmInitStage::DomContentLoaded,
                RendererReplyBoundary::Stage,
                resource_source,
                None,
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
                timing_started: timing_enabled.then_some(timing_started),
                main_document_commit,
                network_error_page: None,
            },
        ));
    }

    let (body_tx, body_rx) = mpsc::channel(EXTERNAL_RAW_BODY_CHANNEL_CAPACITY);
    let (completion_tx, completion_rx) = oneshot::channel();
    let raw_body = moli_core::runtime::ExternalRawDocumentBodyStream::new(body_rx, completion_rx);
    let main_document_commit = load_inputs
        .main_document_commit_for_final_url(&final_url, None)
        .map(Arc::new);
    let prepared_future = load.prepare_document_response_async(
        requested_url.clone(),
        final_url.clone(),
        response.redirected,
        redirect_chain.len(),
        response_status,
        response_headers.clone(),
        raw_body,
        PageVmInitStage::DomContentLoaded,
        reply_boundary,
        resource_source,
        reserved_service_worker_client,
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
            timing_started: timing_enabled.then_some(timing_started),
            main_document_commit,
            network_error_page: None,
        },
    ))
}

impl CdpConnection {
    fn navigation_admission_identity(
        &self,
        navigation: &NavigationDispatchState,
    ) -> Result<(String, String, NavigationId), String> {
        let (context_id, target_id) = self
            .resolved_page_owner_identity_for_owner(&navigation.owner)
            .ok_or("navigation WebContents unavailable")?;
        let token = self
            .browser_context_by_id(&context_id)
            .and_then(|context| {
                context.pending_navigation_id_for_loader(&target_id, &navigation.loader_id)
            })
            .ok_or("stale navigation document candidate")?;
        Ok((context_id, target_id, token))
    }

    fn admit_navigation_load(
        &mut self,
        navigation: &NavigationDispatchState,
    ) -> Result<AdmittedNavigationLoad, String> {
        let (context_id, target_id, token) = self.navigation_admission_identity(navigation)?;
        #[cfg(test)]
        self.ensure_page_navigation_engine_for_target(&context_id, &target_id)
            .ok_or("navigation WebContents engine unavailable")?;
        let defaults = self.document_fetch_defaults();
        let context = self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.id == context_id)
            .ok_or("navigation BrowserContext unavailable")?;
        let load = context.start_navigation_load_for_target(
            &target_id,
            token,
            navigation.request_load_policy,
            defaults,
            &self.permission_defaults,
        )?;
        // Projection binds the native reservation before prepare can publish.
        // It does not allocate or select a Browser Document identity.
        context.project_navigation_load_for_target(&target_id, &load)?;
        self.bind_renderer_page_output_owner(
            load.renderer_page(),
            TargetPageResidenceIdentity::new(context_id, Some(target_id), load.document_id()),
        );
        Ok(load)
    }

    fn navigation_load_inputs_for_navigation(
        &self,
        navigation: &NavigationDispatchState,
    ) -> TargetNavigationLoadInputs {
        apply_navigation_request_load_policy(
            self.navigation_load_inputs_for_owner(&navigation.owner),
            navigation.request_load_policy,
        )
        .with_main_document_commit_seed(RendererMainDocumentCommitSeed::from_navigation(navigation))
    }

    pub(super) fn document_fetch_defaults(&self) -> FetchConfig {
        let mut config = FetchConfig::default();
        config.set_browser_identity(
            self.global_browser_identity_override
                .clone()
                .unwrap_or_else(|| self.base_browser_identity.clone()),
        );
        config.set_http_proxy(self.base_http_proxy.clone());
        config.set_http_no_proxy(self.base_http_no_proxy.clone());
        config.set_tls_verify_host(self.base_tls_verify_host);
        config
    }

    #[cfg(test)]
    pub(crate) fn capture_document_policy_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        final_url: &Url,
    ) -> Result<Option<PreparedDocumentPagePolicy>, String> {
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            // Standalone construction has no installed WebContents policy to
            // refresh. Keep the native policy supplied when it was prepared.
            return Ok(None);
        };
        self.ensure_page_navigation_engine_for_target(&context_id, &target_id)
            .ok_or("navigation WebContents engine unavailable")?;
        let defaults = self.document_fetch_defaults();
        self.browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.id == context_id)
            .ok_or("navigation BrowserContext unavailable")?
            .capture_document_policy_for_target(
                &target_id,
                final_url,
                defaults,
                &self.permission_defaults,
            )
            .map(Some)
    }

    pub(crate) fn start_response_document_materialization_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        navigation: NavigationId,
        mut response: ResponseCommitReady,
    ) -> Result<
        impl std::future::Future<Output = Result<LoadedNavigation<PreparedDocumentNavigation>, String>>
        + use<>,
        String,
    > {
        let (context_id, target_id) = self
            .resolved_page_owner_identity_for_owner(owner)
            .ok_or("navigation WebContents unavailable")?;
        let commit = response
            .main_document_commit
            .as_ref()
            .ok_or("loaded navigation is missing its frozen main Document commit identity")?;
        let destination = DocumentNavigationDestination {
            url: response
                .network_error_page
                .as_ref()
                .map(|error| error.unreachable_url().clone())
                .unwrap_or_else(|| response.final_url.clone()),
            security_origin: commit.security_origin.clone(),
            secure_context_type: commit.secure_context_type.clone(),
        };
        let page = response
            .prepared_page
            .take()
            .expect("response must retain its prepared Document");
        let endpoint = page.inspection_configuration_endpoint();
        let defaults = self.document_fetch_defaults();
        let materialization = self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.id == context_id)
            .ok_or("navigation BrowserContext unavailable")?
            .start_document_materialization_for_target(
                &target_id,
                navigation,
                page,
                destination,
                defaults,
                &self.permission_defaults,
            )
            .map_err(|error| match error.as_str() {
                "stale navigation document candidate"
                | "canceled navigation document candidate" => {
                    "renderer channel navigation was superseded by a newer navigation".to_owned()
                }
                _ => error,
            })?;
        // Native admission precedes any renderer/runtime policy mutation.
        // The owned operation is tied to this reservation and cannot be retargeted.
        let mut inspection = self.prepared_document_inspection_for_owner(owner);
        inspection.main_document_commit = response.main_document_commit.as_deref().cloned();
        let inspection_ack = endpoint.start_configure(inspection);
        Ok(async move {
            let built = materialization.materialize().await;
            if let Err(error) = inspection_ack.await {
                tracing::warn!(%error, "prepared document inspection configuration failed");
            }
            response.finish_materialization(built).await
        })
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
        let mut load = self.admit_navigation_load(navigation)?;
        let main_document_commit = load_inputs
            .main_document_commit_for_final_url(&final_url, None)
            .map(Arc::new);
        let prepared_page = load
            .prepare_document_response_async(
                requested_url.clone(),
                final_url.clone(),
                response.redirected,
                redirect_chain.len(),
                response_status,
                response_headers.clone(),
                raw_body,
                PageVmInitStage::DomContentLoaded,
                RendererReplyBoundary::DocumentCommit,
                CommittedDocumentResourceSource::Synthetic,
                None,
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
            timing_started,
            main_document_commit,
        }))
    }

    pub(crate) fn start_initial_document_page_ensure_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<PendingInitialDocumentPageBuild>, String> {
        if self.runtime_session_owner_slot_for_owner(owner).is_err() {
            return Ok(None);
        }
        if self.has_loaded_page_for_owner(owner) {
            return Ok(None);
        }
        if !self.runtime_session_owner_target_is_initial_about_blank_for_owner(owner) {
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
        if self.has_pending_document_navigation_for_owner(owner) {
            return Ok(None);
        }

        self.start_initial_empty_document_page_build_for_owner(owner)
    }

    pub(crate) fn runtime_session_owner_target_is_initial_about_blank(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_target_is_initial_about_blank_for_owner(&owner)
    }

    fn runtime_session_owner_target_is_initial_about_blank_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        if let Some(is_on_initial_empty_document) =
            self.runtime_session_owner_record_is_on_initial_empty_document_for_owner(owner)
        {
            return is_on_initial_empty_document;
        }
        self.runtime_session_owner_target_url_for_owner(owner)
            .as_deref()
            .and_then(|raw_url| Url::parse(raw_url).ok())
            .as_ref()
            .is_some_and(moli_url::is_about_blank)
    }

    /// Returns whether the materialized initial `about:blank` still needs to
    /// be replaced by the target URL.
    ///
    /// This is a structural lifecycle query.  It deliberately does not look
    /// at `waitForDebuggerOnStart`: the explicit debugger-resume path uses it
    /// after the paused session has been released.
    pub(crate) fn runtime_session_owner_needs_initial_document_navigation_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        if !self.runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(owner) {
            return false;
        }
        if self
            .runtime_session_owner_initial_empty_document_has_pending_cross_document_navigation_for_owner(owner)
        {
            return false;
        }
        true
    }

    /// Returns whether an ordinary Page/Target command may opportunistically
    /// start the initial target-URL navigation.
    ///
    /// A target created with `waitForDebuggerOnStart` must remain on its
    /// initial document until `Runtime.runIfWaitingForDebugger` has published
    /// its terminal response.  Keeping that admission rule here prevents
    /// commands such as `Page.enable` and `Page.createIsolatedWorld` from
    /// racing each other into replacing the paused renderer attachment.
    pub(crate) fn runtime_session_owner_can_start_initial_document_navigation(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_can_start_initial_document_navigation_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_can_start_initial_document_navigation_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        !self.owner_target_has_waiting_for_debugger_session(owner)
            && self.runtime_session_owner_needs_initial_document_navigation_for_owner(owner)
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_replacement_url(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        if !self.runtime_session_owner_target_is_initial_about_blank_for_owner(owner) {
            return false;
        }
        let Some(target_url) = self.runtime_session_owner_target_url_for_owner(owner) else {
            return false;
        };
        let Some(initial_url) =
            self.runtime_session_owner_record_initial_empty_document_url_for_owner(owner)
        else {
            return false;
        };
        target_url != initial_url
    }

    fn start_initial_empty_document_page_build_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<PendingInitialDocumentPageBuild>, String> {
        use crate::conn::state::InitialDocumentAdmission;
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            return Ok(None);
        };
        let defaults = self.document_fetch_defaults();
        let admission = self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.id == context_id)
            .ok_or("TargetNotLoaded")?
            .start_initial_document_for_target(&target_id, defaults, &self.permission_defaults)?;
        let kind = match admission {
            InitialDocumentAdmission::Present => return Ok(None),
            InitialDocumentAdmission::Join(waiter) => {
                PendingInitialDocumentPageBuildKind::Join { waiter }
            }
            InitialDocumentAdmission::Build(mut build) => {
                let key = build.key();
                self.browser_context_by_id_mut(&context_id)
                    .expect("resolved BrowserContext")
                    .project_initial_document_build(&target_id, key);
                // Even preparation opens an output stream. Bind the native
                // reservation before any renderer command can be enqueued.
                self.bind_renderer_page_output_owner(
                    key.renderer(),
                    TargetPageResidenceIdentity::new(
                        context_id.clone(),
                        Some(target_id),
                        key.document(),
                    ),
                );
                if let Err(error) = build.start_preparation() {
                    self.browser_context_by_id_mut(&context_id)
                        .expect("resolved BrowserContext")
                        .retire_initial_document_projection(key);
                    return Err(error.to_string());
                }
                let inspection_ack = build
                    .inspection_endpoint()
                    .start_configure(self.prepared_document_inspection_for_owner(owner));
                PendingInitialDocumentPageBuildKind::Build {
                    key,
                    pending: Box::pin(async move {
                        if let Err(error) = inspection_ack.await {
                            tracing::warn!(%error, "initial document inspection configuration failed");
                        }
                        build.materialize().await
                    }),
                }
            }
        };
        Ok(Some(PendingInitialDocumentPageBuild { kind }))
    }

    pub(crate) fn reset_failed_initial_document_page_build_for_owner(
        &mut self,
        failed: FailedInitialDocumentPageBuild,
    ) -> String {
        if let Some(key) = failed.key {
            for context in self
                .browser_context
                .iter_mut()
                .chain(self.inactive_browser_contexts.iter_mut())
            {
                context.retire_initial_document_projection(key);
            }
        }
        failed.message
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
        let CompletedInitialDocumentPageBuild::Built(built) = completed else {
            return Ok(LoadedPageCreationDiagnosticsParts::default());
        };
        let key = built.key();
        let context = self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.owns_web_contents(key.web_contents()));
        let committed = match context {
            Some(context) => context.commit_initial_document(*built),
            None => Err(built),
        };
        match committed {
            Ok(diagnostics) => Ok(loaded_page_creation_diagnostics_parts(diagnostics)),
            Err(stale) => {
                for context in self
                    .browser_context
                    .iter_mut()
                    .chain(self.inactive_browser_contexts.iter_mut())
                {
                    context.retire_initial_document_projection(key);
                }
                stale.retire().await;
                Ok(LoadedPageCreationDiagnosticsParts::default())
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn load_navigation_via_runtime_async(
        &mut self,
        raw_url: &str,
    ) -> Result<LoadedNavigation, String> {
        let owner = CommandOwnerScope::capture(self, None);
        let load_inputs = self.navigation_load_inputs_for_owner(&owner);
        self.load_navigation_via_runtime_with_load_inputs_async(&owner, raw_url, load_inputs)
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
        let owner = CommandOwnerScope::capture(self, session_id);
        let load_inputs = self.navigation_fixture_load_inputs_for_session_owner(session_id)?;
        self.load_navigation_via_runtime_with_load_inputs_async(&owner, raw_url, load_inputs)
            .await
    }

    #[cfg(test)]
    async fn load_navigation_via_runtime_with_load_inputs_async(
        &mut self,
        owner: &CommandOwnerScope,
        raw_url: &str,
        load_inputs: TargetNavigationLoadInputs,
    ) -> Result<LoadedNavigation, String> {
        let request_headers = load_inputs.extra_http_headers.clone();
        let navigation = self
            .load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
                owner,
                load_inputs,
                "GET",
                raw_url,
                None,
                request_headers,
                MainDocumentBodyProgressSource::default(),
            )
            .await?;
        self.commit_navigation_load_outcome_for_owner_async(owner, navigation)
            .await
    }

    #[cfg(test)]
    async fn commit_navigation_load_outcome_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        navigation: NavigationLoadOutcome,
    ) -> Result<LoadedNavigation, String> {
        match navigation {
            NavigationLoadOutcome::ResponseCommitReady(navigation) => {
                let navigation = *navigation;
                let policy =
                    self.capture_document_policy_for_owner(owner, navigation.final_url())?;
                let inspection = self.prepared_document_inspection_for_owner(owner);
                navigation.materialize(policy, inspection).await
            }
            NavigationLoadOutcome::Download(_) => {
                Err("navigation resolved to a download".to_owned())
            }
            NavigationLoadOutcome::NetworkFailure(error_text) => Err(error_text),
        }
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
        let owner = CommandOwnerScope::capture(self, session_id);
        let load_inputs = apply_navigation_request_load_policy(
            self.navigation_load_inputs_for_owner(&owner),
            request_load_policy,
        );
        self.load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
            &owner,
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
        let load = self.admit_navigation_load(navigation)?;
        let job = BackgroundNavigationLoadJob {
            load,
            reply_boundary: RendererReplyBoundary::Stage,
            early_result: None,
            load_inputs: self.navigation_load_inputs_for_navigation(navigation),
            method: navigation.request_method.clone(),
            raw_url: navigation.requested_url.to_string(),
            body: navigation.clone_request_body_bytes(),
            request_headers: navigation.request_headers.clone(),
            body_progress_source,
        };
        job.run(None).await.0
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    async fn load_navigation_request_via_runtime_with_network_events_and_load_inputs_async(
        &mut self,
        owner: &CommandOwnerScope,
        load_inputs: TargetNavigationLoadInputs,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let load = self.navigation_load_fixture(owner, &load_inputs)?;
        BackgroundNavigationLoadJob {
            load,
            reply_boundary: RendererReplyBoundary::Stage,
            early_result: None,
            load_inputs,
            method: method.to_owned(),
            raw_url: raw_url.to_owned(),
            body,
            request_headers,
            body_progress_source,
        }
        .run(None)
        .await
        .0
    }

    pub(crate) fn navigation_load_job_for_navigation(
        &mut self,
        token: &NavigationId,
        navigation: &NavigationDispatchState,
        body_progress_source: MainDocumentBodyProgressSource,
        early_result: Option<BackgroundNavigationEarlyResult>,
    ) -> Option<BackgroundNavigationLoadJob> {
        if self.navigation_admission_identity(navigation).ok()?.2 != *token {
            return None;
        }
        let load = self.admit_navigation_load(navigation).ok()?;
        Some(BackgroundNavigationLoadJob {
            load,
            reply_boundary: RendererReplyBoundary::DocumentCommit,
            early_result,
            load_inputs: self.navigation_load_inputs_for_navigation(navigation),
            method: navigation.request_method.clone(),
            raw_url: navigation.requested_url.to_string(),
            body: navigation.clone_request_body_bytes(),
            request_headers: navigation.request_headers.clone(),
            body_progress_source,
        })
    }

    pub(crate) fn background_navigation_load_job_for_navigation(
        &mut self,
        token: &NavigationId,
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
    ) -> Result<BackgroundStreamingResponseNavigationLoadJob, String> {
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        let load = self.admit_navigation_load(navigation)?;
        Ok(BackgroundStreamingResponseNavigationLoadJob {
            load,
            load_inputs,
            requested_url: navigation.requested_url.clone(),
            request_method: navigation.request_method.clone(),
            request_headers: navigation.request_headers.clone(),
            response,
            network_observation_journal,
            response_code,
            response_headers_override,
            body_progress_source,
        })
    }

    #[cfg(test)]
    fn navigation_load_fixture(
        &mut self,
        owner: &CommandOwnerScope,
        inputs: &TargetNavigationLoadInputs,
    ) -> Result<AdmittedNavigationLoad, String> {
        let engine = self.navigation_engine_handle_for_load_inputs(inputs);
        let policy = if inputs.browser_navigation_kind == BrowserNavigationRequestKind::Reload {
            NavigationRequestLoadPolicy::Reload
        } else if inputs.infer_navigation_referrer {
            NavigationRequestLoadPolicy::DocumentInitiated
        } else {
            NavigationRequestLoadPolicy::BrowserInitiated
        };
        let load = AdmittedNavigationLoad::for_fixture(
            engine,
            inputs.resource_storage_handles().into_navigation_storage(),
            inputs.page_storage_handles().into_navigation_storage(),
            inputs.navigation_initiator_url.clone(),
            policy,
            inputs.network_offline,
            inputs.blocked_url_patterns.clone(),
        );
        if inputs.browser_context_id.is_some()
            && self.resolved_page_owner_identity_for_owner(owner).is_some()
        {
            let projected = self
                .reserve_target_page_residence_identity_for_owner(owner, load.renderer_page())
                .ok_or("navigation fixture owner unavailable")?;
            self.bind_renderer_page_output_owner(load.renderer_page(), projected);
        }
        Ok(load)
    }

    /// Returns a task-local handle to the NavigationEngine retained by the
    /// target owner. `NavigationEngine::clone` shares the target's Page policy
    /// and renderer owner, so a background load never has to return or replace
    /// the resident engine when it completes.
    #[cfg(test)]
    pub(super) fn navigation_engine_handle_for_load_inputs(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> NavigationEngine {
        let engine = self
            .configured_navigation_engine_for_load_inputs_mut(load_inputs)
            .expect("navigation load target must retain its resident NavigationEngine")
            .clone();
        // The handle may publish lifecycle or resource activity before the
        // DCL-bound navigation result is committed into a target slot.
        self.apply_scheduler_senders_to_navigation_engine(&engine);
        engine
    }

    #[cfg(test)]
    pub(crate) async fn load_page_via_runtime_async(
        &mut self,
        raw_url: &str,
    ) -> Result<Page, String> {
        let navigation = self.load_navigation_via_runtime_async(raw_url).await?;
        Ok(navigation.page)
    }

    /// Builds a loaded document from an already-buffered text response.
    ///
    /// This path is for synthetic or in-memory document sources, including
    /// initial document page build and test/setup helpers. It still uses the
    /// phase-one HTML parser; it is not the old NativeDom static builder and
    /// should not be used for real network document streaming.
    #[cfg(test)]
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
        let owner = CommandOwnerScope::capture(self, session_id);
        let load_inputs = self.navigation_fixture_load_inputs_for_session_owner(session_id)?;
        let initial_request_cookie_report =
            load_inputs.request_cookie_report_for_navigation(&requested_url, &request_method, true);
        let mut load = self.navigation_load_fixture(&owner, &load_inputs)?;
        let navigation = self
            .build_navigation_from_buffered_body_source_with_load_inputs_async(
                &mut load,
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
        self.commit_navigation_load_outcome_for_owner_async(&owner, navigation)
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
        let mut load = self.admit_navigation_load(navigation)?;
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_buffered_body_source_with_load_inputs_async(
            &mut load,
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
        load: &mut AdmittedNavigationLoad,
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
        self.build_navigation_from_captured_raw_response_with_load_inputs_async(
            load,
            load_inputs,
            requested_url,
            request_method,
            request_headers,
            head,
            response_body,
            network_observation_journal,
            body_progress_source,
        )
        .await
    }

    #[cfg(test)]
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
            .navigation_engine_for_load_inputs_mut(load_inputs)
            .ok_or_else(|| "navigation Page engine unavailable".to_owned())?
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
        apply_fixture_permission_overrides(&mut page, &load_inputs.permission_overrides).await?;
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
            network_error_page: None,
        })
    }

    pub(crate) async fn fetch_navigation_auth_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        auth: SubresourceAuthCredentials,
    ) -> Result<NetworkFetchResult<RawResponse>, String> {
        let load = self.admit_navigation_load(navigation)?;
        load.validate_request(navigation.requested_url.as_str())
            .map_err(|error| error.to_string())?;
        load.fetch_intercepted_auth_response(
            &navigation.request_method,
            navigation.requested_url.as_str(),
            navigation.clone_request_body_bytes(),
            navigation.request_headers.clone(),
            auth,
        )
        .await
        .map_err(|error| {
            format!(
                "failed to fetch page `{}`: {error}",
                navigation.requested_url
            )
        })
    }

    pub(crate) async fn fetch_navigation_streaming_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<NetworkFetchResult<StreamingRawResponse>, String> {
        let load = self.admit_navigation_load(navigation)?;
        load.validate_request(navigation.requested_url.as_str())
            .map_err(|error| error.to_string())?;
        load.fetch_intercepted_response(
            &navigation.request_method,
            navigation.requested_url.as_str(),
            navigation.clone_request_body_bytes(),
            navigation.request_headers.clone(),
            auth,
        )
        .await
        .map_err(|error| {
            format!(
                "failed to fetch page `{}`: {error}",
                navigation.requested_url
            )
        })
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
            .navigation_engine_for_load_inputs_mut(&load_inputs)
            .ok_or_else(|| "navigation Page engine unavailable".to_owned())?
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
        apply_fixture_permission_overrides(&mut page, &load_inputs.permission_overrides).await?;
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
            network_error_page: None,
        })
    }

    /// Builds navigation from raw bytes that are already fully buffered.
    ///
    /// This keeps buffered/synthetic cases explicit. It is not the main network
    /// document path; true network responses should use the streaming raw
    /// builders so parser work can start before body EOF.
    pub(crate) async fn build_navigation_from_buffered_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        response: NetworkFetchResult<RawResponse>,
    ) -> Result<NavigationLoadOutcome, String> {
        let mut load = self.admit_navigation_load(navigation)?;
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_buffered_raw_response_with_load_inputs_async(
            &mut load,
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
        load: &mut AdmittedNavigationLoad,
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
            load,
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
        let mut load = self.admit_navigation_load(navigation)?;
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_captured_raw_response_with_load_inputs_async(
            &mut load,
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
        load: &mut AdmittedNavigationLoad,
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

        prepare_navigation_from_captured_raw_response_with_load_async(
            load,
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

    pub(crate) async fn build_navigation_from_streaming_raw_response_for_navigation_async(
        &mut self,
        navigation: &NavigationDispatchState,
        response: NetworkFetchResult<StreamingRawResponse>,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<NavigationLoadOutcome, String> {
        let mut load = self.admit_navigation_load(navigation)?;
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_streaming_raw_response_with_load_inputs_async(
            &mut load,
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
        let mut load = self.admit_navigation_load(navigation)?;
        let load_inputs = self.navigation_load_inputs_for_navigation(navigation);
        self.build_navigation_from_streaming_raw_response_with_load_inputs_async(
            &mut load,
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
        load: &mut AdmittedNavigationLoad,
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
        build_navigation_from_streaming_raw_response_with_load_async(
            load,
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
        .await
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

async fn prepare_navigation_from_captured_raw_response_with_load_async(
    load: &mut AdmittedNavigationLoad,
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
        return prepare_browser_owned_error_page_navigation_with_load_async(
            load,
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
    prepare_captured_document_response_with_load_async(
        load,
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
async fn prepare_captured_document_response_with_load_async(
    load: &mut AdmittedNavigationLoad,
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
    let main_document_commit = load_inputs
        .main_document_commit_for_final_url(&final_url, network_error_page.as_ref())
        .map(Arc::new);
    let prepared_future = load.prepare_document_response_async(
        requested_url.clone(),
        final_url.clone(),
        redirected,
        redirect_chain.len(),
        response_status,
        response_headers.clone(),
        raw_body,
        PageVmInitStage::DomContentLoaded,
        reply_boundary,
        CommittedDocumentResourceSource::Synthetic,
        None,
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
        timing_started: None,
        main_document_commit,
        network_error_page,
    })
}

#[cfg(test)]
async fn apply_fixture_permission_overrides(
    page: &mut moli_core::page::Page,
    permissions: &[moli_core::page::PermissionOverrideRegistration],
) -> Result<(), String> {
    page.set_permission_overrides_async(permissions)
        .await
        .map_err(|error| format!("failed to apply page permission overrides: {error}"))
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
