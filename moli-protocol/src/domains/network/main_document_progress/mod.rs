mod emit;
mod gate;
#[cfg(test)]
mod tests;

use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use moli_core::RendererOutputFence;
use moli_core::page::{
    NavigationRedirect, Page, RendererMainDocumentCommit, RendererPageCreationArtifacts,
    RendererPendingDownloadActivation, RendererRuntimeRealmInfo, SubresourceRequestInitiatorType,
};
use moli_fetch::{
    NegotiatedHttpVersion, NetworkExchangeObservation, NetworkObservationJournal, RedirectInfo,
    StreamingRawResponse,
};
use std::sync::Arc;
use url::Url;

use crate::conn::{
    BackgroundEventSender, BackgroundProtocolEvent, CapturedBody, CdpConnection,
    CompletedDownloadBodyArtifact, DownloadNavigation, LoadedNavigation, NavigationDispatchState,
    NavigationLoadOutcome, ResponseCommitReady, TargetRuntimeSlot,
};

#[cfg(test)]
use gate::MainDocumentProgressDrain;
pub(crate) use gate::{MainDocumentProgressBackgroundEventBarrier, MainDocumentProgressGate};
use gate::{
    MainDocumentProgressEmission, MainDocumentProgressEventBatch, MainDocumentProgressOutputTarget,
    MainDocumentProgressPhase, MainDocumentProgressQueueHandle, MainDocumentProgressSource,
};

pub(crate) struct MaterializedLoadedDocumentProgress {
    pub(crate) page: Page,
    pub(crate) pending_download: Option<RendererPendingDownloadActivation>,
    pub(crate) page_creation_artifacts: RendererPageCreationArtifacts,
    pub(crate) final_url: Url,
    pub(crate) response_headers: Vec<(String, String)>,
    pub(crate) response_from_cache: bool,
    pub(crate) main_document_body: Option<CapturedBody>,
    pub(crate) initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    pub(crate) renderer_output_predecessor: Option<RendererOutputFence>,
    pub(crate) main_document_commit: Option<Arc<RendererMainDocumentCommit>>,
    pub(crate) progress_gate: MainDocumentProgressGate,
    pub(crate) network_error_page: Option<crate::conn::NetworkErrorPageNavigation>,
}

pub(crate) struct MaterializedDownloadDocumentProgress {
    pub(crate) final_url: Url,
    pub(crate) progress_gate: MainDocumentProgressGate,
    pub(crate) body_artifact: CompletedDownloadBodyArtifact,
}

pub(crate) struct MaterializedFailedDocumentProgress {
    pub(crate) error_text: String,
    pub(crate) document_policy: FailedNavigationDocumentPolicy,
    pub(crate) response_mode: FailedNavigationResponseMode,
    pub(crate) progress_gate: MainDocumentProgressGate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailedNavigationResponseMode {
    ProtocolError,
    CdpErrorTextResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailedNavigationDocumentPolicy {
    InvalidateCommittedDocument,
    PreserveCommittedDocument,
}

impl FailedNavigationDocumentPolicy {
    pub(crate) fn invalidates_committed_document(self) -> bool {
        matches!(self, Self::InvalidateCommittedDocument)
    }
}

pub(crate) enum MaterializedNavigationLoadOutcome {
    ResponseCommitReady(Box<ResponseCommitReady>),
    Loaded(Box<MaterializedLoadedDocumentProgress>),
    Download(MaterializedDownloadDocumentProgress),
    Failed(MaterializedFailedDocumentProgress),
}

#[cfg(test)]
pub(crate) fn empty_main_document_progress_gate_for_test() -> MainDocumentProgressGate {
    MainDocumentProgressGate::from_queue(MainDocumentProgressQueueHandle::new(
        MainDocumentProgressDrain::new(),
    ))
}

#[derive(Debug)]
pub(crate) struct CompletedDocumentProgressTransfer {
    body: CompletedDocumentProgressBody,
    network_progress: MainDocumentBodyNetworkProgress,
}

#[derive(Debug)]
enum CompletedDocumentProgressBody {
    Captured { body: CapturedBody, synthetic: bool },
    Pending,
}

impl CompletedDocumentProgressTransfer {
    pub(crate) fn new_captured(
        body: CapturedBody,
        synthetic: bool,
        network_progress: MainDocumentBodyNetworkProgress,
    ) -> Self {
        Self {
            body: CompletedDocumentProgressBody::Captured { body, synthetic },
            network_progress,
        }
    }

    pub(crate) fn new_pending_body(network_progress: MainDocumentBodyNetworkProgress) -> Self {
        Self {
            body: CompletedDocumentProgressBody::Pending,
            network_progress,
        }
    }

    fn captured_body(&self) -> Option<CapturedBody> {
        match &self.body {
            CompletedDocumentProgressBody::Captured { body, .. } => Some(body.clone()),
            CompletedDocumentProgressBody::Pending => None,
        }
    }

    fn into_progress_gate(
        self,
        conn: &mut CdpConnection,
        state: &NavigationDispatchState,
        final_url: &Url,
    ) -> MainDocumentProgressGate {
        let queue = completed_document_body_progress_queue(self, conn, state, final_url);
        MainDocumentProgressGate::from_queue(queue)
    }

    fn into_parts(
        self,
    ) -> (
        CompletedDocumentProgressBody,
        MainDocumentBodyNetworkProgress,
    ) {
        (self.body, self.network_progress)
    }

    #[cfg(test)]
    pub(crate) fn body(&self) -> &CapturedBody {
        match &self.body {
            CompletedDocumentProgressBody::Captured { body, .. } => body,
            CompletedDocumentProgressBody::Pending => {
                panic!("test navigation body is still pending")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn completed_body_network_events(
        &self,
    ) -> Option<&CompletedMainDocumentNetworkEvents> {
        self.network_progress.as_completed_body()
    }
}

#[derive(Debug)]
pub(crate) struct CompletedDownloadProgressTransfer {
    body: CompletedDownloadProgressBody,
    network_events: CompletedMainDocumentNetworkEvents,
}

#[derive(Debug)]
enum CompletedDownloadProgressBody {
    Buffered(Vec<u8>),
    Streaming(Box<StreamingRawResponse>),
}

impl CompletedDownloadProgressTransfer {
    pub(crate) fn new(body: Vec<u8>, network_events: CompletedMainDocumentNetworkEvents) -> Self {
        Self {
            body: CompletedDownloadProgressBody::Buffered(body),
            network_events,
        }
    }

    pub(crate) fn new_streaming(
        response: StreamingRawResponse,
        network_events: CompletedMainDocumentNetworkEvents,
    ) -> Self {
        Self {
            body: CompletedDownloadProgressBody::Streaming(Box::new(response)),
            network_events,
        }
    }

    fn into_progress_gate_and_artifact(
        self,
        conn: &CdpConnection,
        state: &NavigationDispatchState,
        final_url: &Url,
    ) -> (MainDocumentProgressGate, CompletedDownloadBodyArtifact) {
        let response_headers = self.network_events.response_headers.clone();
        let progress = CompletedDownloadBodyNetworkProgress {
            encoded_data_length: completed_download_body_len_hint(&self.body, &response_headers),
            network_events: self.network_events,
        };
        let queue = completed_download_body_progress_queue(progress, conn, state, final_url);
        let body = match self.body {
            CompletedDownloadProgressBody::Buffered(body) => {
                crate::conn::CompletedDownloadBody::Buffered(body)
            }
            CompletedDownloadProgressBody::Streaming(response) => {
                crate::conn::CompletedDownloadBody::Streaming(response)
            }
        };
        (
            MainDocumentProgressGate::from_queue(queue),
            CompletedDownloadBodyArtifact::from_body(body, response_headers),
        )
    }
}

fn completed_download_body_len_hint(
    body: &CompletedDownloadProgressBody,
    response_headers: &[(String, String)],
) -> usize {
    match body {
        CompletedDownloadProgressBody::Buffered(body) => body.len(),
        CompletedDownloadProgressBody::Streaming(_) => response_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or_default(),
    }
}

#[derive(Debug)]
struct CompletedDownloadBodyNetworkProgress {
    encoded_data_length: usize,
    network_events: CompletedMainDocumentNetworkEvents,
}

impl CompletedDownloadBodyNetworkProgress {
    fn len(&self) -> usize {
        self.encoded_data_length
    }

    fn into_network_events(self) -> CompletedMainDocumentNetworkEvents {
        self.network_events
    }
}

fn failed_navigation_progress_gate(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    error_text: &str,
) -> MainDocumentProgressGate {
    MainDocumentProgressGate::from_queue(MainDocumentProgressQueueHandle::from_source(
        MainDocumentProgressSource::failed_navigation(observed_navigation_failure_event(
            conn, state, error_text,
        )),
    ))
}

fn network_error_page_progress_gate(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    error_text: &str,
) -> MainDocumentProgressGate {
    let failure_event = observed_navigation_failure_event(conn, state, error_text);
    let finish_event = observed_navigation_finished_event(conn, state);
    MainDocumentProgressGate::from_queue(MainDocumentProgressQueueHandle::from_source(
        MainDocumentProgressSource::error_page(failure_event, finish_event),
    ))
}

fn materialize_failed_navigation_progress(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    error_text: String,
    document_policy: FailedNavigationDocumentPolicy,
    response_mode: FailedNavigationResponseMode,
) -> MaterializedFailedDocumentProgress {
    let progress_gate = failed_navigation_progress_gate(conn, state, &error_text);
    MaterializedFailedDocumentProgress {
        error_text,
        document_policy,
        response_mode,
        progress_gate,
    }
}

pub(crate) fn materialize_loaded_navigation_progress(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    navigation: LoadedNavigation,
) -> MaterializedLoadedDocumentProgress {
    let LoadedNavigation {
        page,
        pending_download,
        page_creation_artifacts,
        final_url,
        response_headers,
        response_from_cache,
        initial_runtime_realms,
        renderer_output_predecessor,
        main_document_commit,
        document_progress_transfer,
        network_error_page,
        ..
    } = navigation;
    let main_document_body = document_progress_transfer.captured_body();
    let progress_gate = match network_error_page.as_ref() {
        Some(error_page) => network_error_page_progress_gate(conn, state, error_page.error_text()),
        None => document_progress_transfer.into_progress_gate(conn, state, &final_url),
    };
    MaterializedLoadedDocumentProgress {
        page,
        pending_download,
        page_creation_artifacts,
        final_url,
        response_headers,
        response_from_cache,
        main_document_body,
        initial_runtime_realms,
        renderer_output_predecessor,
        main_document_commit,
        progress_gate,
        network_error_page,
    }
}

fn materialize_navigation_load_outcome(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    navigation: NavigationLoadOutcome,
) -> MaterializedNavigationLoadOutcome {
    match navigation {
        NavigationLoadOutcome::ResponseCommitReady(navigation) => {
            MaterializedNavigationLoadOutcome::ResponseCommitReady(navigation)
        }
        NavigationLoadOutcome::Loaded(navigation) => {
            MaterializedNavigationLoadOutcome::Loaded(Box::new(
                materialize_loaded_navigation_progress(conn, state, *navigation),
            ))
        }
        NavigationLoadOutcome::Download(navigation) => MaterializedNavigationLoadOutcome::Download(
            materialize_download_navigation_progress(conn, state, *navigation),
        ),
        NavigationLoadOutcome::NetworkFailure(error_text) => {
            MaterializedNavigationLoadOutcome::Failed(materialize_failed_navigation_progress(
                conn,
                state,
                error_text,
                FailedNavigationDocumentPolicy::InvalidateCommittedDocument,
                FailedNavigationResponseMode::CdpErrorTextResult,
            ))
        }
    }
}

pub(crate) fn materialize_navigation_load_result(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    navigation: Result<NavigationLoadOutcome, String>,
) -> MaterializedNavigationLoadOutcome {
    match navigation {
        Ok(navigation) => materialize_navigation_load_outcome(conn, state, navigation),
        Err(error_text) => {
            MaterializedNavigationLoadOutcome::Failed(materialize_failed_navigation_progress(
                conn,
                state,
                error_text,
                FailedNavigationDocumentPolicy::InvalidateCommittedDocument,
                FailedNavigationResponseMode::ProtocolError,
            ))
        }
    }
}

pub(crate) fn materialize_navigation_failure_preserving_committed_document(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    error_text: String,
) -> MaterializedNavigationLoadOutcome {
    MaterializedNavigationLoadOutcome::Failed(materialize_failed_navigation_progress(
        conn,
        state,
        error_text,
        FailedNavigationDocumentPolicy::PreserveCommittedDocument,
        FailedNavigationResponseMode::ProtocolError,
    ))
}

fn materialize_download_navigation_progress(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    navigation: DownloadNavigation,
) -> MaterializedDownloadDocumentProgress {
    let DownloadNavigation {
        final_url,
        progress_transfer,
    } = navigation;
    let (progress_gate, body_artifact) =
        progress_transfer.into_progress_gate_and_artifact(conn, state, &final_url);
    MaterializedDownloadDocumentProgress {
        final_url,
        progress_gate,
        body_artifact,
    }
}

fn completed_document_body_progress_queue(
    transfer: CompletedDocumentProgressTransfer,
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    final_url: &Url,
) -> MainDocumentProgressQueueHandle {
    let owner_session_id = state.session_id.as_deref();
    let session_ids = main_document_network_event_session_ids(conn, owner_session_id);
    let network_observed = main_document_network_observed(conn, owner_session_id);
    let (body, body_progress_source) = transfer.into_parts();
    let encoded_data_length = match body {
        CompletedDocumentProgressBody::Captured { body, synthetic } => {
            let encoded_data_length = body.len();
            let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
                owner_session_id,
                crate::devtools_runtime::DevToolsNetworkDataType::Response,
                encoded_data_length,
            );
            let collection_was_gated = conn.network_data_collection_is_gated_for_body(
                crate::devtools_runtime::DevToolsNetworkDataType::Response,
            );
            if network_observed
                && !synthetic
                && let Some(request_id) = state.request_id.clone()
            {
                conn.record_collected_network_data_body(
                    request_id,
                    crate::devtools_runtime::DevToolsNetworkDataType::Response,
                    body.clone(),
                    collector_ids.iter().cloned(),
                    collection_was_gated,
                );
            }
            let _ = conn
                .runtime_session_owner_slot_mut(owner_session_id)
                .ok()
                .and_then(|runtime_slot| {
                    record_main_document_response_body_for_network(
                        runtime_slot,
                        network_observed,
                        state.request_id.clone(),
                        &session_ids,
                        collector_ids,
                        collection_was_gated,
                        synthetic,
                        &body,
                    )
                });
            encoded_data_length
        }
        CompletedDocumentProgressBody::Pending => {
            record_pending_main_document_response_body(conn, state, &session_ids);
            0
        }
    };
    let event_request_id =
        completed_body_main_document_network_request_id(network_observed, state.request_id.clone());
    completed_or_streaming_document_progress_queue(
        conn,
        state,
        network_observed,
        event_request_id,
        body_progress_source.into_completed_body_events(),
        final_url,
        encoded_data_length,
    )
}

fn completed_download_body_progress_queue(
    progress: CompletedDownloadBodyNetworkProgress,
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    final_url: &Url,
) -> MainDocumentProgressQueueHandle {
    let network_observed = main_document_network_observed(conn, state.session_id.as_deref());
    let encoded_data_length = progress.len();
    completed_or_streaming_document_progress_queue(
        conn,
        state,
        network_observed,
        state.request_id.clone(),
        Some(progress.into_network_events()),
        final_url,
        encoded_data_length,
    )
}

fn completed_or_streaming_document_progress_queue(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    network_enabled: bool,
    request_id: Option<String>,
    events: Option<CompletedMainDocumentNetworkEvents>,
    final_url: &Url,
    encoded_data_length: usize,
) -> MainDocumentProgressQueueHandle {
    let Some(events) = events else {
        return MainDocumentProgressQueueHandle::from_source(
            MainDocumentProgressSource::streaming(),
        );
    };
    let context = CompletedMainDocumentProgressContext::new(
        main_document_network_event_session_ids(conn, state.session_id.as_deref()),
        completed_body_main_document_network_request_id(network_enabled, request_id),
        state.request_announced,
        state.requested_url.clone(),
        state.request_method.clone(),
        state.request_body.clone(),
        state.request_headers.clone(),
        state.loader_id.clone(),
        state.frame_id.clone(),
        state.timestamp,
    );
    MainDocumentProgressQueueHandle::from_source(MainDocumentProgressSource::completed_body(
        context.event_batches(&events, final_url, encoded_data_length),
    ))
}

#[derive(Clone)]
struct MainDocumentLiveNetworkProgressSource {
    sender: Option<BackgroundEventSender>,
    progress_queue: MainDocumentProgressQueueHandle,
    session_ids: Vec<Option<String>>,
    request_id: String,
    loader_id: String,
    frame_id: String,
    timestamp: f64,
    initial_request_headers: Vec<(String, String)>,
    initial_request_cookie_report: Option<StoredCookieQueryReport>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MainDocumentResponseVisibility {
    #[default]
    Immediate,
    AfterResponseStageContinue,
}

#[derive(Clone, Debug, Default)]
pub struct MainDocumentBodyProgressSource {
    live_source: Option<MainDocumentLiveNetworkProgressSource>,
    response_visibility: MainDocumentResponseVisibility,
}

impl MainDocumentBodyProgressSource {
    fn from_live_source(
        live_source: Option<MainDocumentLiveNetworkProgressSource>,
        response_visibility: MainDocumentResponseVisibility,
    ) -> Self {
        Self {
            live_source,
            response_visibility,
        }
    }

    pub(crate) fn emit_response_metadata(
        &self,
        final_request_method: &str,
        final_request_headers: &[(String, String)],
        final_request_cookie_report: Option<&StoredCookieQueryReport>,
        redirect_chain: &[RedirectInfo],
        final_url: &Url,
        response_status: u16,
        response_headers: &[(String, String)],
        response_cookie_reports: &[StoredCookieSetReport],
        network_observation_journal: &NetworkObservationJournal,
        network_extra_info_available: bool,
        response_from_cache: bool,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
    ) {
        let Some(live_source) = self.live_source.as_ref() else {
            return;
        };
        match self.response_visibility {
            MainDocumentResponseVisibility::Immediate => {
                live_source.emit_redirect_requests(
                    final_request_method,
                    None,
                    final_request_headers,
                    final_request_cookie_report,
                    redirect_chain,
                    network_observation_journal,
                    network_extra_info_available,
                );
                live_source.emit_response_received(
                    final_url,
                    response_status,
                    response_headers,
                    response_cookie_reports,
                    network_observation_journal,
                    redirect_chain.len(),
                    network_extra_info_available,
                    response_from_cache,
                    negotiated_http_version,
                );
            }
            MainDocumentResponseVisibility::AfterResponseStageContinue => {
                live_source.emit_response_received_without_extra_info(
                    final_url,
                    response_status,
                    response_headers,
                    network_extra_info_available,
                    response_from_cache,
                    negotiated_http_version,
                );
            }
        }
    }

    pub(crate) fn emit_response_without_extra_info_into_background_events(
        &self,
        out: &mut Vec<BackgroundProtocolEvent>,
        final_url: &Url,
        response_status: u16,
        response_headers: &[(String, String)],
        response_from_cache: bool,
    ) {
        let Some(live_source) = self.live_source.as_ref() else {
            return;
        };
        let mut output = MainDocumentProgressOutputTarget::background_events(out);
        live_source.send_progress_events_into_output(
            MainDocumentProgressPhase::ResponseReceived,
            vec![MainDocumentNavigationProgressEvent::ResponseReceived {
                target: live_source.progress_target(),
                final_url: final_url.clone(),
                status: response_status,
                headers: response_headers.to_vec(),
                cookie_set_reports: Vec::new(),
                extra_info_status: response_status,
                extra_info_headers: response_headers.to_vec(),
                network_extra_info_available: false,
                emit_extra_info: false,
                encoded_data_length: 0,
                from_cache: response_from_cache,
                negotiated_http_version: None,
                has_extra_info: false,
            }],
            &mut output,
        );
    }

    pub(crate) fn emit_failed_initial_request_extra_info(
        &self,
        network_observation_journal: &NetworkObservationJournal,
    ) {
        let Some(live_source) = self.live_source.as_ref() else {
            return;
        };
        let [exchange] = network_observation_journal.exchanges() else {
            return;
        };
        if exchange.response().is_some() {
            return;
        }
        let request = exchange.request();
        let cookie_report = request
            .cookie_report()
            .cloned()
            .or_else(|| live_source.initial_request_cookie_report.clone());
        live_source.send_progress_events(
            MainDocumentProgressPhase::RequestStarted,
            vec![request_extra_info_event(
                live_source.progress_target(),
                request.headers().to_vec(),
                cookie_report,
            )],
        );
    }

    pub(crate) fn emit_failed_request_progress(
        &self,
        final_request_method: &str,
        request_body: Option<&str>,
        final_request_headers: &[(String, String)],
        redirect_chain: &[RedirectInfo],
        network_observation_journal: &NetworkObservationJournal,
    ) {
        let Some(live_source) = self.live_source.as_ref() else {
            return;
        };
        let final_exchange = navigation_exchange_group(
            network_observation_journal,
            redirect_chain.len(),
            redirect_chain.len(),
        )
        .and_then(|group| group.last());
        let final_network_extra_info_available =
            final_exchange.is_some_and(|exchange| exchange.response().is_none());
        let final_request_cookie_report = final_exchange
            .and_then(|exchange| exchange.request().cookie_report())
            .cloned();
        let events = live_source.redirect_request_events(
            final_request_method,
            request_body,
            final_request_headers,
            final_request_cookie_report.as_ref(),
            redirect_chain,
            network_observation_journal,
            final_network_extra_info_available,
        );
        live_source.send_progress_events(MainDocumentProgressPhase::RequestStarted, events);
    }

    pub(crate) fn emit_response_extra_info_before_pause(
        &self,
        out: &mut Vec<BackgroundProtocolEvent>,
        final_request_method: &str,
        final_request_headers: &[(String, String)],
        final_request_cookie_report: Option<&StoredCookieQueryReport>,
        redirect_chain: &[RedirectInfo],
        response_status: u16,
        response_headers: &[(String, String)],
        response_cookie_reports: &[StoredCookieSetReport],
        network_observation_journal: &NetworkObservationJournal,
        network_extra_info_available: bool,
    ) {
        let Some(live_source) = self.live_source.as_ref() else {
            return;
        };
        if self.response_visibility != MainDocumentResponseVisibility::AfterResponseStageContinue {
            return;
        }

        let mut output = MainDocumentProgressOutputTarget::background_events(out);
        live_source.emit_redirect_requests_into_output(
            &mut output,
            final_request_method,
            None,
            final_request_headers,
            final_request_cookie_report,
            redirect_chain,
            network_observation_journal,
            network_extra_info_available,
        );
        if network_extra_info_available {
            let (response_status, response_headers) = observed_response_metadata(
                network_observation_journal,
                redirect_chain.len(),
                redirect_chain.len(),
                response_status,
                response_headers,
            );
            live_source.emit_response_received_extra_info_into_output(
                &mut output,
                response_status,
                &response_headers,
                response_cookie_reports,
            );
        }
    }

    pub(crate) fn emit_body_finished(&self, encoded_data_length: usize) {
        if let Some(live_source) = self.live_source.as_ref() {
            live_source.emit_body_finished(encoded_data_length);
        }
    }

    pub(crate) fn body_network_progress_for_completed_events(
        &self,
        completed_events: CompletedMainDocumentNetworkEvents,
    ) -> MainDocumentBodyNetworkProgress {
        if self
            .live_source
            .as_ref()
            .is_some_and(|live_source| live_source.sender.is_some())
        {
            MainDocumentBodyNetworkProgress::StreamingBody
        } else {
            let mut completed_events = completed_events;
            completed_events.response_stage_metadata_already_emitted = self.response_visibility
                == MainDocumentResponseVisibility::AfterResponseStageContinue;
            MainDocumentBodyNetworkProgress::CompletedBody(Box::new(completed_events))
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompletedMainDocumentNetworkEvents {
    pub(crate) request_method: String,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) final_request_cookie_report: Option<StoredCookieQueryReport>,
    pub(crate) response_status: u16,
    pub(crate) response_headers: Vec<(String, String)>,
    pub(crate) response_cookie_reports: Vec<StoredCookieSetReport>,
    pub(crate) redirect_chain: Vec<NavigationRedirect>,
    pub(crate) network_extra_info_available: bool,
    pub(crate) response_from_cache: bool,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_observation_journal: NetworkObservationJournal,
    response_stage_metadata_already_emitted: bool,
}

impl CompletedMainDocumentNetworkEvents {
    pub(crate) fn new(
        request_method: String,
        request_headers: Vec<(String, String)>,
        final_request_cookie_report: Option<StoredCookieQueryReport>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        response_cookie_reports: Vec<StoredCookieSetReport>,
        redirect_chain: Vec<NavigationRedirect>,
        network_extra_info_available: bool,
        response_from_cache: bool,
    ) -> Self {
        Self {
            request_method,
            request_headers,
            final_request_cookie_report,
            response_status,
            response_headers,
            response_cookie_reports,
            redirect_chain,
            network_extra_info_available,
            response_from_cache,
            negotiated_http_version: None,
            network_observation_journal: NetworkObservationJournal::default(),
            response_stage_metadata_already_emitted: false,
        }
    }

    pub(crate) fn with_negotiated_http_version(
        mut self,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
    ) -> Self {
        self.negotiated_http_version = negotiated_http_version;
        self
    }

    pub(crate) fn with_network_observation_journal(
        mut self,
        network_observation_journal: NetworkObservationJournal,
    ) -> Self {
        self.network_observation_journal = network_observation_journal;
        self
    }
}

#[derive(Debug)]
pub(crate) enum MainDocumentBodyNetworkProgress {
    CompletedBody(Box<CompletedMainDocumentNetworkEvents>),
    StreamingBody,
}

impl MainDocumentBodyNetworkProgress {
    pub(crate) fn into_completed_body_events(self) -> Option<CompletedMainDocumentNetworkEvents> {
        match self {
            Self::CompletedBody(events) => Some(*events),
            Self::StreamingBody => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_completed_body(&self) -> Option<&CompletedMainDocumentNetworkEvents> {
        match self {
            Self::CompletedBody(events) => Some(events),
            Self::StreamingBody => None,
        }
    }
}

impl MainDocumentLiveNetworkProgressSource {
    pub(crate) fn from_navigation_dispatch_state(
        sender: Option<BackgroundEventSender>,
        session_ids: Vec<Option<String>>,
        state: &NavigationDispatchState,
        initial_request_cookie_report: Option<&StoredCookieQueryReport>,
    ) -> Option<Self> {
        Some(Self {
            sender,
            progress_queue: MainDocumentProgressQueueHandle::from_source(
                MainDocumentProgressSource::streaming(),
            ),
            session_ids,
            request_id: state.request_id.clone()?,
            loader_id: state.loader_id.clone(),
            frame_id: state.frame_id.clone(),
            timestamp: state.timestamp,
            initial_request_headers: state.request_headers.clone(),
            initial_request_cookie_report: initial_request_cookie_report.cloned(),
        })
    }

    pub(crate) fn emit_initial_request_will_be_sent(
        &self,
        url: &Url,
        method: &str,
        request_body: Option<&str>,
        request_headers: &[(String, String)],
        cookie_access_report: Option<&StoredCookieQueryReport>,
    ) {
        let events = vec![MainDocumentNavigationProgressEvent::RequestWillBeSent {
            target: self.progress_target(),
            url: url.clone(),
            method: method.to_owned(),
            request_body: request_body.map(str::to_owned),
            request_headers: request_headers.to_vec(),
            request_initiator_type: SubresourceRequestInitiatorType::Other,
            redirect_response: Box::new(None),
            redirect_has_extra_info: false,
            cookie_access_report: cookie_access_report.cloned(),
        }];
        self.send_progress_events(MainDocumentProgressPhase::RequestStarted, events);
    }

    pub(crate) fn emit_redirect_requests(
        &self,
        final_request_method: &str,
        request_body: Option<&str>,
        final_request_headers: &[(String, String)],
        final_request_cookie_report: Option<&StoredCookieQueryReport>,
        redirect_chain: &[RedirectInfo],
        network_observation_journal: &NetworkObservationJournal,
        final_network_extra_info_available: bool,
    ) {
        let events = self.redirect_request_events(
            final_request_method,
            request_body,
            final_request_headers,
            final_request_cookie_report,
            redirect_chain,
            network_observation_journal,
            final_network_extra_info_available,
        );
        self.send_progress_events(MainDocumentProgressPhase::RequestStarted, events);
    }

    fn emit_redirect_requests_into_output(
        &self,
        output: &mut MainDocumentProgressOutputTarget<'_>,
        final_request_method: &str,
        request_body: Option<&str>,
        final_request_headers: &[(String, String)],
        final_request_cookie_report: Option<&StoredCookieQueryReport>,
        redirect_chain: &[RedirectInfo],
        network_observation_journal: &NetworkObservationJournal,
        final_network_extra_info_available: bool,
    ) {
        let events = self.redirect_request_events(
            final_request_method,
            request_body,
            final_request_headers,
            final_request_cookie_report,
            redirect_chain,
            network_observation_journal,
            final_network_extra_info_available,
        );
        self.send_progress_events_into_output(
            MainDocumentProgressPhase::RequestStarted,
            events,
            output,
        );
    }

    fn redirect_request_events(
        &self,
        final_request_method: &str,
        request_body: Option<&str>,
        final_request_headers: &[(String, String)],
        final_request_cookie_report: Option<&StoredCookieQueryReport>,
        redirect_chain: &[RedirectInfo],
        network_observation_journal: &NetworkObservationJournal,
        final_network_extra_info_available: bool,
    ) -> Vec<MainDocumentNavigationProgressEvent> {
        let mut events = Vec::new();
        let initial_network_extra_info_available = !network_observation_journal.is_empty()
            || redirect_chain
                .first()
                .is_some_and(|redirect| redirect.network_extra_info_available)
            || (redirect_chain.is_empty() && final_network_extra_info_available);
        if initial_network_extra_info_available {
            let initial_request_cookie_report = if redirect_chain.is_empty() {
                final_request_cookie_report
                    .cloned()
                    .or_else(|| self.initial_request_cookie_report.clone())
            } else {
                self.initial_request_cookie_report.clone()
            };
            events.push(request_extra_info_event(
                self.progress_target(),
                observed_request_headers(
                    network_observation_journal,
                    redirect_chain.len(),
                    0,
                    &self.initial_request_headers,
                ),
                initial_request_cookie_report,
            ));
        }
        for (index, redirect) in redirect_chain.iter().enumerate() {
            let target = self.progress_target();
            let observed_response_available =
                navigation_exchange_group(network_observation_journal, redirect_chain.len(), index)
                    .and_then(|group| group.last())
                    .and_then(NetworkExchangeObservation::response)
                    .is_some();
            if observed_response_available && !redirect.network_extra_info_available {
                let (status, headers) = observed_response_metadata(
                    network_observation_journal,
                    redirect_chain.len(),
                    index,
                    redirect.status,
                    &redirect.headers,
                );
                events.push(
                    MainDocumentNavigationProgressEvent::ResponseReceivedExtraInfo {
                        target: target.clone(),
                        headers,
                        status,
                        cookie_set_reports: redirect.cookie_set_reports.clone(),
                    },
                );
            }
            events.push(MainDocumentNavigationProgressEvent::RequestWillBeSent {
                target: target.clone(),
                url: redirect.to_url.clone(),
                method: final_request_method.to_owned(),
                request_body: request_body.map(str::to_owned),
                request_headers: final_request_headers.to_vec(),
                request_initiator_type: SubresourceRequestInitiatorType::Other,
                redirect_response: Box::new(Some(MainDocumentRedirectResponse {
                    url: redirect.from_url.clone(),
                    status: redirect.status,
                    status_text: critical_client_hint_internal_redirect_status_text(
                        &redirect.from_url,
                        &redirect.to_url,
                        redirect.status,
                        redirect.network_extra_info_available,
                        observed_response_available,
                    ),
                    headers: redirect.headers.clone(),
                    from_cache: redirect.from_cache,
                    negotiated_http_version: redirect.negotiated_http_version,
                })),
                redirect_has_extra_info: redirect.network_extra_info_available,
                cookie_access_report: redirect.request_cookie_report.clone(),
            });
            if observed_response_available && redirect.network_extra_info_available {
                let (status, headers) = observed_response_metadata(
                    network_observation_journal,
                    redirect_chain.len(),
                    index,
                    redirect.status,
                    &redirect.headers,
                );
                events.push(
                    MainDocumentNavigationProgressEvent::ResponseReceivedExtraInfo {
                        target: target.clone(),
                        headers,
                        status,
                        cookie_set_reports: redirect.cookie_set_reports.clone(),
                    },
                );
            }
            let request_network_extra_info_available = network_observation_journal
                .exchanges()
                .get(index + 1)
                .is_some()
                || redirect_chain
                    .get(index + 1)
                    .is_some_and(|next| next.network_extra_info_available)
                || (index + 1 == redirect_chain.len() && final_network_extra_info_available);
            if request_network_extra_info_available {
                let cookie_report = redirect.request_cookie_report.clone().or_else(|| {
                    (index + 1 == redirect_chain.len())
                        .then(|| final_request_cookie_report.cloned())
                        .flatten()
                });
                events.push(request_extra_info_event(
                    target,
                    observed_request_headers(
                        network_observation_journal,
                        redirect_chain.len(),
                        index + 1,
                        final_request_headers,
                    ),
                    cookie_report,
                ));
            }
        }
        events
    }

    pub(crate) fn emit_response_received(
        &self,
        final_url: &Url,
        response_status: u16,
        response_headers: &[(String, String)],
        response_cookie_reports: &[StoredCookieSetReport],
        network_observation_journal: &NetworkObservationJournal,
        redirect_count: usize,
        network_extra_info_available: bool,
        response_from_cache: bool,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
    ) {
        let (extra_info_status, extra_info_headers) = observed_response_metadata(
            network_observation_journal,
            redirect_count,
            redirect_count,
            response_status,
            response_headers,
        );
        self.emit_response_received_with_extra_info_policy(
            final_url,
            response_status,
            response_headers,
            response_cookie_reports,
            extra_info_status,
            extra_info_headers,
            network_extra_info_available,
            network_extra_info_available,
            response_from_cache,
            negotiated_http_version,
            network_extra_info_available && !response_from_cache,
        );
    }

    fn emit_response_received_without_extra_info(
        &self,
        final_url: &Url,
        response_status: u16,
        response_headers: &[(String, String)],
        network_extra_info_available: bool,
        response_from_cache: bool,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
    ) {
        self.emit_response_received_with_extra_info_policy(
            final_url,
            response_status,
            response_headers,
            &[],
            response_status,
            response_headers.to_vec(),
            network_extra_info_available,
            false,
            response_from_cache,
            negotiated_http_version,
            network_extra_info_available && !response_from_cache,
        );
    }

    fn emit_response_received_with_extra_info_policy(
        &self,
        final_url: &Url,
        response_status: u16,
        response_headers: &[(String, String)],
        response_cookie_reports: &[StoredCookieSetReport],
        extra_info_status: u16,
        extra_info_headers: Vec<(String, String)>,
        network_extra_info_available: bool,
        emit_extra_info: bool,
        response_from_cache: bool,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
        has_extra_info: bool,
    ) {
        self.send_progress_events(
            MainDocumentProgressPhase::ResponseReceived,
            vec![MainDocumentNavigationProgressEvent::ResponseReceived {
                target: self.progress_target(),
                final_url: final_url.clone(),
                status: response_status,
                headers: response_headers.to_vec(),
                cookie_set_reports: response_cookie_reports.to_vec(),
                extra_info_status,
                extra_info_headers,
                network_extra_info_available,
                emit_extra_info,
                encoded_data_length: 0,
                from_cache: response_from_cache,
                negotiated_http_version,
                has_extra_info,
            }],
        );
    }

    fn emit_response_received_extra_info_into_output(
        &self,
        output: &mut MainDocumentProgressOutputTarget<'_>,
        response_status: u16,
        response_headers: &[(String, String)],
        response_cookie_reports: &[StoredCookieSetReport],
    ) {
        self.send_progress_events_into_output(
            MainDocumentProgressPhase::RequestStarted,
            vec![
                MainDocumentNavigationProgressEvent::ResponseReceivedExtraInfo {
                    target: self.progress_target(),
                    headers: response_headers.to_vec(),
                    status: response_status,
                    cookie_set_reports: response_cookie_reports.to_vec(),
                },
            ],
            output,
        );
    }

    pub(crate) fn emit_body_finished(&self, encoded_data_length: usize) {
        self.send_progress_events(
            MainDocumentProgressPhase::BodyFinished,
            vec![MainDocumentNavigationProgressEvent::LoadingFinished {
                target: self.progress_target(),
                encoded_data_length,
            }],
        );
    }

    fn progress_target(&self) -> MainDocumentProgressEventTarget {
        MainDocumentProgressEventTarget {
            session_ids: self.session_ids.clone(),
            request_id: self.request_id.clone(),
            loader_id: self.loader_id.clone(),
            frame_id: self.frame_id.clone(),
            timestamp: self.timestamp,
        }
    }

    fn send_progress_events(
        &self,
        ready_until: MainDocumentProgressPhase,
        events: Vec<MainDocumentNavigationProgressEvent>,
    ) {
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        let mut output = MainDocumentProgressOutputTarget::background_sender(sender);
        self.send_progress_events_into_output(ready_until, events, &mut output);
    }

    fn send_progress_events_into_output(
        &self,
        ready_until: MainDocumentProgressPhase,
        events: Vec<MainDocumentNavigationProgressEvent>,
        output: &mut MainDocumentProgressOutputTarget<'_>,
    ) {
        let emission = MainDocumentProgressEmission::new(
            ready_until,
            MainDocumentProgressEventBatch::from_events(events),
        );
        self.progress_queue
            .append_ready_emission(ready_until, emission);
        self.progress_queue.drain_into_output_target(output);
    }
}

impl std::fmt::Debug for MainDocumentLiveNetworkProgressSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainDocumentLiveNetworkProgressSource")
            .field("session_ids", &self.session_ids)
            .field("request_id", &self.request_id)
            .field("loader_id", &self.loader_id)
            .field("frame_id", &self.frame_id)
            .field("timestamp", &self.timestamp)
            .finish_non_exhaustive()
    }
}

fn emit_main_document_initial_request_will_be_sent_for_sessions_into(
    output: &mut MainDocumentProgressOutputTarget<'_>,
    session_ids: &[Option<String>],
    state: &NavigationDispatchState,
    cookie_access_report: Option<&StoredCookieQueryReport>,
) -> bool {
    let Some(request_id) = state.request_id.clone() else {
        return false;
    };
    let target = MainDocumentProgressEventTarget {
        session_ids: session_ids.to_vec(),
        request_id,
        loader_id: state.loader_id.clone(),
        frame_id: state.frame_id.clone(),
        timestamp: state.timestamp,
    };
    output.emit_event(MainDocumentNavigationProgressEvent::RequestWillBeSent {
        target,
        url: state.requested_url.clone(),
        method: state.request_method.clone(),
        request_body: state.request_body.clone(),
        request_headers: state.request_headers.clone(),
        request_initiator_type: SubresourceRequestInitiatorType::Other,
        redirect_response: Box::new(None),
        redirect_has_extra_info: false,
        cookie_access_report: cookie_access_report.cloned(),
    });
    true
}

pub(crate) fn emit_fetch_navigation_initial_request_for_pause_background_events(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    state: &NavigationDispatchState,
    cookie_access_report: Option<&StoredCookieQueryReport>,
) -> bool {
    let mut session_ids =
        main_document_network_event_session_ids(conn, state.session_id.as_deref());
    if session_ids.is_empty() {
        session_ids.push(state.session_id.clone());
    }
    let mut output = MainDocumentProgressOutputTarget::background_events(out);
    emit_main_document_initial_request_will_be_sent_for_sessions_into(
        &mut output,
        &session_ids,
        state,
        cookie_access_report,
    )
}

pub(crate) fn emit_child_document_navigation_network_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
    request_id: &str,
    timestamp: f64,
    network: &moli_core::page::ChildFrameDocumentNetworkSnapshot,
) {
    let Ok(request_url) = Url::parse(&network.request_url) else {
        return;
    };
    let Ok(final_url) = Url::parse(&network.final_url) else {
        return;
    };
    let session_ids = conn.network_event_session_ids_for_session_owner(session_id);
    if session_ids.is_empty() {
        return;
    }
    let target = MainDocumentProgressEventTarget {
        session_ids: session_ids.clone(),
        request_id: request_id.to_owned(),
        loader_id: loader_id.to_owned(),
        frame_id: frame_id.to_owned(),
        timestamp,
    };
    let mut output = MainDocumentProgressOutputTarget::background_events(out);
    output.emit_event(MainDocumentNavigationProgressEvent::RequestWillBeSent {
        target: target.clone(),
        url: request_url,
        method: network.request_method.clone(),
        request_body: None,
        request_headers: network.request_headers.clone(),
        request_initiator_type: SubresourceRequestInitiatorType::Parser,
        redirect_response: Box::new(None),
        redirect_has_extra_info: false,
        cookie_access_report: None,
    });
    output.emit_event(MainDocumentNavigationProgressEvent::ResponseReceived {
        target: target.clone(),
        final_url,
        status: network.status,
        headers: network.response_headers.clone(),
        cookie_set_reports: Vec::new(),
        extra_info_status: network.status,
        extra_info_headers: network.response_headers.clone(),
        network_extra_info_available: false,
        emit_extra_info: false,
        encoded_data_length: 0,
        from_cache: network.from_cache,
        negotiated_http_version: None,
        has_extra_info: false,
    });
    record_child_document_response_body(
        conn,
        session_id,
        request_id,
        &session_ids,
        network.response_body.as_ref(),
    );
    output.emit_event(MainDocumentNavigationProgressEvent::LoadingFinished {
        target,
        encoded_data_length: network.encoded_data_length,
    });
}

fn record_child_document_response_body(
    conn: &mut CdpConnection,
    owner_session_id: Option<&str>,
    request_id: &str,
    session_ids: &[Option<String>],
    response_body: Option<&moli_core::page::SubresourceResponseBody>,
) {
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Response;
    let encoded_data_length = response_body.map_or(0, |body| body.len());
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        owner_session_id,
        data_type,
        encoded_data_length,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(data_type);
    let captured_body =
        response_body.map(crate::conn::CapturedBody::from_subresource_response_body);
    if let Some(captured_body) = captured_body.as_ref() {
        conn.record_collected_network_data_body(
            request_id.to_owned(),
            data_type,
            captured_body.clone(),
            collector_ids.iter().cloned(),
            collection_was_gated,
        );
    }
    let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(owner_session_id) else {
        return;
    };
    if let Some(captured_body) = captured_body {
        runtime_slot.record_captured_response_body_source_with_collector_scope(
            request_id.to_owned(),
            captured_body,
            session_ids.iter().cloned(),
            collector_ids,
            collection_was_gated,
        );
    } else {
        runtime_slot.record_failed_response_body_with_collector_scope(
            request_id.to_owned(),
            "child document response body unavailable".to_owned(),
            session_ids.iter().cloned(),
            collector_ids,
            collection_was_gated,
        );
    }
}

pub(crate) fn start_observed_main_document_navigation_progress_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    state: &NavigationDispatchState,
    cookie_access_report: Option<&StoredCookieQueryReport>,
) -> MainDocumentBodyProgressSource {
    let session_ids = main_document_network_event_session_ids(conn, state.session_id.as_deref());
    record_pending_main_document_response_body(conn, state, &session_ids);
    record_main_document_request_body(conn, state);
    if let Some(sender) = conn.background_event_sender()
        && let Some(live_source) =
            MainDocumentLiveNetworkProgressSource::from_navigation_dispatch_state(
                Some(sender),
                session_ids.clone(),
                state,
                cookie_access_report,
            )
    {
        live_source.emit_initial_request_will_be_sent(
            &state.requested_url,
            &state.request_method,
            state.request_body.as_deref(),
            &state.request_headers,
            cookie_access_report,
        );
        return MainDocumentBodyProgressSource::from_live_source(
            Some(live_source),
            MainDocumentResponseVisibility::Immediate,
        );
    }
    let mut output = MainDocumentProgressOutputTarget::background_events(out);
    emit_main_document_initial_request_will_be_sent_for_sessions_into(
        &mut output,
        &session_ids,
        state,
        cookie_access_report,
    );
    MainDocumentBodyProgressSource::default()
}

pub(crate) fn response_stage_main_document_navigation_network_progress(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    initial_request_cookie_report: Option<&StoredCookieQueryReport>,
) -> MainDocumentBodyProgressSource {
    if !main_document_network_observed(conn, state.session_id.as_deref()) {
        return MainDocumentBodyProgressSource::default();
    }
    let live_source = MainDocumentLiveNetworkProgressSource::from_navigation_dispatch_state(
        conn.background_event_sender(),
        main_document_network_event_session_ids(conn, state.session_id.as_deref()),
        state,
        initial_request_cookie_report,
    );
    MainDocumentBodyProgressSource::from_live_source(
        live_source,
        MainDocumentResponseVisibility::AfterResponseStageContinue,
    )
}

fn observed_navigation_failure_event(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
    error_text: &str,
) -> Option<MainDocumentNavigationProgressEvent> {
    if !conn.has_network_event_listeners_for_session_owner(state.session_id.as_deref()) {
        return None;
    }
    let request_id = state.request_id.clone()?;
    Some(MainDocumentNavigationProgressEvent::LoadingFailed {
        target: MainDocumentProgressEventTarget {
            session_ids: conn
                .network_event_session_ids_for_session_owner(state.session_id.as_deref()),
            request_id,
            loader_id: state.loader_id.clone(),
            frame_id: state.frame_id.clone(),
            timestamp: state.timestamp,
        },
        error_text: error_text.to_owned(),
    })
}

fn observed_navigation_finished_event(
    conn: &CdpConnection,
    state: &NavigationDispatchState,
) -> Option<MainDocumentNavigationProgressEvent> {
    if !conn.has_network_event_listeners_for_session_owner(state.session_id.as_deref()) {
        return None;
    }
    let request_id = state.request_id.clone()?;
    Some(MainDocumentNavigationProgressEvent::LoadingFinished {
        target: MainDocumentProgressEventTarget {
            session_ids: conn
                .network_event_session_ids_for_session_owner(state.session_id.as_deref()),
            request_id,
            loader_id: state.loader_id.clone(),
            frame_id: state.frame_id.clone(),
            timestamp: state.timestamp,
        },
        encoded_data_length: 0,
    })
}

fn main_document_network_event_session_ids(
    conn: &CdpConnection,
    trigger_session_id: Option<&str>,
) -> Vec<Option<String>> {
    conn.network_event_session_ids_for_session_owner(trigger_session_id)
}

struct CompletedMainDocumentProgressContext {
    session_ids: Vec<Option<String>>,
    request_id: Option<String>,
    request_announced: bool,
    requested_url: Url,
    request_method: String,
    request_body: Option<String>,
    request_headers: Vec<(String, String)>,
    loader_id: String,
    frame_id: String,
    timestamp: f64,
}

impl CompletedMainDocumentProgressContext {
    fn new(
        session_ids: Vec<Option<String>>,
        request_id: Option<String>,
        request_announced: bool,
        requested_url: Url,
        request_method: String,
        request_body: Option<String>,
        request_headers: Vec<(String, String)>,
        loader_id: String,
        frame_id: String,
        timestamp: f64,
    ) -> Self {
        Self {
            session_ids,
            request_id,
            request_announced,
            requested_url,
            request_method,
            request_body,
            request_headers,
            loader_id,
            frame_id,
            timestamp,
        }
    }

    fn event_batches(
        &self,
        events: &CompletedMainDocumentNetworkEvents,
        final_url: &Url,
        encoded_data_length: usize,
    ) -> MainDocumentNavigationProgressEventBatches {
        MainDocumentNavigationProgressEventBatches::new(
            self.request_and_redirect_progress_events(events),
            self.response_received_progress_events(events, final_url, encoded_data_length),
            self.loading_finished_progress_events(encoded_data_length),
        )
    }

    fn request_and_redirect_progress_events(
        &self,
        events: &CompletedMainDocumentNetworkEvents,
    ) -> Vec<MainDocumentNavigationProgressEvent> {
        if events.response_stage_metadata_already_emitted {
            return Vec::new();
        }
        let Some(target) = self.progress_target() else {
            return Vec::new();
        };
        let mut progress_events = Vec::new();
        let initial_request_cookie_report = events
            .redirect_chain
            .is_empty()
            .then(|| events.final_request_cookie_report.clone())
            .flatten();
        if !self.request_announced {
            progress_events.push(MainDocumentNavigationProgressEvent::RequestWillBeSent {
                target: target.clone(),
                url: self.requested_url.clone(),
                method: self.request_method.clone(),
                request_body: self.request_body.clone(),
                request_headers: self.request_headers.clone(),
                request_initiator_type: SubresourceRequestInitiatorType::Other,
                redirect_response: Box::new(None),
                redirect_has_extra_info: false,
                cookie_access_report: initial_request_cookie_report.clone(),
            });
        }
        let initial_network_extra_info_available = !events.network_observation_journal.is_empty()
            || events
                .redirect_chain
                .first()
                .is_some_and(|redirect| redirect.network_extra_info_available)
            || (events.redirect_chain.is_empty() && events.network_extra_info_available);
        if initial_network_extra_info_available {
            progress_events.push(request_extra_info_event(
                target.clone(),
                observed_request_headers(
                    &events.network_observation_journal,
                    events.redirect_chain.len(),
                    0,
                    &self.request_headers,
                ),
                initial_request_cookie_report,
            ));
        }
        for (index, redirect) in events.redirect_chain.iter().enumerate() {
            let observed_response_available = navigation_exchange_group(
                &events.network_observation_journal,
                events.redirect_chain.len(),
                index,
            )
            .and_then(|group| group.last())
            .and_then(NetworkExchangeObservation::response)
            .is_some();
            if observed_response_available && !redirect.network_extra_info_available {
                let (status, headers) = observed_response_metadata(
                    &events.network_observation_journal,
                    events.redirect_chain.len(),
                    index,
                    redirect.status,
                    &redirect.headers,
                );
                progress_events.push(
                    MainDocumentNavigationProgressEvent::ResponseReceivedExtraInfo {
                        target: target.clone(),
                        headers,
                        status,
                        cookie_set_reports: redirect.cookie_set_reports.clone(),
                    },
                );
            }
            progress_events.push(MainDocumentNavigationProgressEvent::RequestWillBeSent {
                target: target.clone(),
                url: redirect.to_url.clone(),
                method: events.request_method.clone(),
                request_body: self.request_body.clone(),
                request_headers: events.request_headers.clone(),
                request_initiator_type: SubresourceRequestInitiatorType::Other,
                redirect_response: Box::new(Some(MainDocumentRedirectResponse {
                    url: redirect.from_url.clone(),
                    status: redirect.status,
                    status_text: critical_client_hint_internal_redirect_status_text(
                        &redirect.from_url,
                        &redirect.to_url,
                        redirect.status,
                        redirect.network_extra_info_available,
                        observed_response_available,
                    ),
                    headers: redirect.headers.clone(),
                    from_cache: redirect.from_cache,
                    negotiated_http_version: redirect.negotiated_http_version,
                })),
                redirect_has_extra_info: redirect.network_extra_info_available,
                cookie_access_report: redirect.request_cookie_report.clone(),
            });
            if observed_response_available && redirect.network_extra_info_available {
                let (status, headers) = observed_response_metadata(
                    &events.network_observation_journal,
                    events.redirect_chain.len(),
                    index,
                    redirect.status,
                    &redirect.headers,
                );
                progress_events.push(
                    MainDocumentNavigationProgressEvent::ResponseReceivedExtraInfo {
                        target: target.clone(),
                        headers,
                        status,
                        cookie_set_reports: redirect.cookie_set_reports.clone(),
                    },
                );
            }
            let request_network_extra_info_available = events
                .network_observation_journal
                .exchanges()
                .get(index + 1)
                .is_some()
                || events
                    .redirect_chain
                    .get(index + 1)
                    .is_some_and(|next| next.network_extra_info_available)
                || (index + 1 == events.redirect_chain.len()
                    && events.network_extra_info_available);
            if request_network_extra_info_available {
                let cookie_report = redirect.request_cookie_report.clone().or_else(|| {
                    (index + 1 == events.redirect_chain.len())
                        .then(|| events.final_request_cookie_report.clone())
                        .flatten()
                });
                progress_events.push(request_extra_info_event(
                    target.clone(),
                    observed_request_headers(
                        &events.network_observation_journal,
                        events.redirect_chain.len(),
                        index + 1,
                        &events.request_headers,
                    ),
                    cookie_report,
                ));
            }
        }
        progress_events
    }

    fn response_received_progress_events(
        &self,
        events: &CompletedMainDocumentNetworkEvents,
        final_url: &Url,
        encoded_data_length: usize,
    ) -> Vec<MainDocumentNavigationProgressEvent> {
        let Some(target) = self.progress_target() else {
            return Vec::new();
        };
        let (extra_info_status, extra_info_headers) = observed_response_metadata(
            &events.network_observation_journal,
            events.redirect_chain.len(),
            events.redirect_chain.len(),
            events.response_status,
            &events.response_headers,
        );
        vec![MainDocumentNavigationProgressEvent::ResponseReceived {
            target,
            final_url: final_url.clone(),
            status: events.response_status,
            headers: events.response_headers.clone(),
            cookie_set_reports: events.response_cookie_reports.clone(),
            extra_info_status,
            extra_info_headers,
            network_extra_info_available: events.network_extra_info_available,
            emit_extra_info: events.network_extra_info_available
                && !events.response_stage_metadata_already_emitted,
            encoded_data_length,
            from_cache: events.response_from_cache,
            negotiated_http_version: events.negotiated_http_version,
            has_extra_info: events.network_extra_info_available && !events.response_from_cache,
        }]
    }

    fn loading_finished_progress_events(
        &self,
        encoded_data_length: usize,
    ) -> Vec<MainDocumentNavigationProgressEvent> {
        let Some(target) = self.progress_target() else {
            return Vec::new();
        };
        vec![MainDocumentNavigationProgressEvent::LoadingFinished {
            target,
            encoded_data_length,
        }]
    }

    fn progress_target(&self) -> Option<MainDocumentProgressEventTarget> {
        Some(MainDocumentProgressEventTarget {
            session_ids: self.session_ids.clone(),
            request_id: self.request_id.clone()?,
            loader_id: self.loader_id.clone(),
            frame_id: self.frame_id.clone(),
            timestamp: self.timestamp,
        })
    }
}

struct MainDocumentNavigationProgressEventBatches {
    request_started: Vec<MainDocumentNavigationProgressEvent>,
    response_received: Vec<MainDocumentNavigationProgressEvent>,
    body_finished: Vec<MainDocumentNavigationProgressEvent>,
}

#[derive(Clone)]
pub(crate) struct MainDocumentProgressEventTarget {
    pub(crate) session_ids: Vec<Option<String>>,
    pub(crate) request_id: String,
    pub(crate) loader_id: String,
    pub(crate) frame_id: String,
    pub(crate) timestamp: f64,
}

enum MainDocumentNavigationProgressEvent {
    RequestWillBeSent {
        target: MainDocumentProgressEventTarget,
        url: Url,
        method: String,
        request_body: Option<String>,
        request_headers: Vec<(String, String)>,
        request_initiator_type: SubresourceRequestInitiatorType,
        redirect_response: Box<Option<MainDocumentRedirectResponse>>,
        redirect_has_extra_info: bool,
        cookie_access_report: Option<StoredCookieQueryReport>,
    },
    RequestWillBeSentExtraInfo {
        target: MainDocumentProgressEventTarget,
        request_headers: Vec<(String, String)>,
        cookie_access_report: StoredCookieQueryReport,
    },
    ResponseReceivedExtraInfo {
        target: MainDocumentProgressEventTarget,
        headers: Vec<(String, String)>,
        status: u16,
        cookie_set_reports: Vec<StoredCookieSetReport>,
    },
    ResponseReceived {
        target: MainDocumentProgressEventTarget,
        final_url: Url,
        status: u16,
        headers: Vec<(String, String)>,
        cookie_set_reports: Vec<StoredCookieSetReport>,
        extra_info_status: u16,
        extra_info_headers: Vec<(String, String)>,
        network_extra_info_available: bool,
        emit_extra_info: bool,
        encoded_data_length: usize,
        from_cache: bool,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
        has_extra_info: bool,
    },
    LoadingFinished {
        target: MainDocumentProgressEventTarget,
        encoded_data_length: usize,
    },
    LoadingFailed {
        target: MainDocumentProgressEventTarget,
        error_text: String,
    },
}

pub(crate) struct MainDocumentRedirectResponse {
    pub(crate) url: Url,
    pub(crate) status: u16,
    pub(crate) status_text: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) from_cache: bool,
    pub(crate) negotiated_http_version: Option<NegotiatedHttpVersion>,
}

fn critical_client_hint_internal_redirect_status_text(
    from_url: &Url,
    to_url: &Url,
    status: u16,
    redirect_has_extra_info: bool,
    discarded_network_response_has_extra_info: bool,
) -> Option<String> {
    (status == 307
        && from_url == to_url
        && !redirect_has_extra_info
        && discarded_network_response_has_extra_info)
        .then(|| "Internal Redirect".to_owned())
}

fn request_extra_info_event(
    target: MainDocumentProgressEventTarget,
    request_headers: Vec<(String, String)>,
    cookie_access_report: Option<StoredCookieQueryReport>,
) -> MainDocumentNavigationProgressEvent {
    MainDocumentNavigationProgressEvent::RequestWillBeSentExtraInfo {
        target,
        request_headers,
        cookie_access_report: cookie_access_report.unwrap_or_default(),
    }
}

fn navigation_exchange_group(
    journal: &NetworkObservationJournal,
    redirect_count: usize,
    hop_index: usize,
) -> Option<&[NetworkExchangeObservation]> {
    // A truncated journal still proves that a transport exchange happened, but
    // no longer provides a trustworthy tail for redirect-hop correlation.
    if journal.truncated() {
        return None;
    }
    if hop_index > redirect_count {
        return None;
    }

    let exchanges = journal.exchanges();
    let mut group_start = 0;
    let mut current_hop = 0;
    for (index, exchange) in exchanges.iter().enumerate() {
        let ends_redirect_hop = exchange.response().is_some_and(|response| {
            matches!(response.status(), 301 | 302 | 303 | 307 | 308)
                || response.headers().iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("critical-ch") && !value.trim().is_empty()
                })
        });
        if !ends_redirect_hop || current_hop >= redirect_count {
            continue;
        }
        if current_hop == hop_index {
            return Some(&exchanges[group_start..=index]);
        }
        current_hop += 1;
        group_start = index.saturating_add(1);
    }
    (current_hop == hop_index && group_start < exchanges.len()).then_some(&exchanges[group_start..])
}

fn observed_request_headers(
    journal: &NetworkObservationJournal,
    redirect_count: usize,
    hop_index: usize,
    fallback: &[(String, String)],
) -> Vec<(String, String)> {
    navigation_exchange_group(journal, redirect_count, hop_index)
        .and_then(|group| group.first())
        .map(|exchange| exchange.request().headers().to_vec())
        .unwrap_or_else(|| fallback.to_vec())
}

fn observed_response_metadata(
    journal: &NetworkObservationJournal,
    redirect_count: usize,
    hop_index: usize,
    fallback_status: u16,
    fallback_headers: &[(String, String)],
) -> (u16, Vec<(String, String)>) {
    navigation_exchange_group(journal, redirect_count, hop_index)
        .and_then(|group| group.last())
        .and_then(NetworkExchangeObservation::response)
        .map(|response| (response.status(), response.headers().to_vec()))
        .unwrap_or_else(|| (fallback_status, fallback_headers.to_vec()))
}

impl MainDocumentNavigationProgressEventBatches {
    fn new(
        request_started: Vec<MainDocumentNavigationProgressEvent>,
        response_received: Vec<MainDocumentNavigationProgressEvent>,
        body_finished: Vec<MainDocumentNavigationProgressEvent>,
    ) -> Self {
        Self {
            request_started,
            response_received,
            body_finished,
        }
    }

    fn take_request_started(&mut self) -> Vec<MainDocumentNavigationProgressEvent> {
        std::mem::take(&mut self.request_started)
    }

    fn take_response_received(&mut self) -> Vec<MainDocumentNavigationProgressEvent> {
        std::mem::take(&mut self.response_received)
    }

    fn take_body_finished(&mut self) -> Vec<MainDocumentNavigationProgressEvent> {
        std::mem::take(&mut self.body_finished)
    }
}

impl MainDocumentNavigationProgressEvent {
    fn emit_into(self, output: &mut MainDocumentProgressOutputTarget<'_>) {
        match self {
            Self::RequestWillBeSent {
                target,
                url,
                method,
                request_body,
                request_headers,
                request_initiator_type,
                redirect_response,
                redirect_has_extra_info,
                cookie_access_report,
            } => {
                let redirect_response = redirect_response.as_ref().as_ref().map(|redirect| {
                    (
                        &redirect.url,
                        redirect.status,
                        redirect.status_text.as_deref(),
                        redirect.headers.as_slice(),
                        redirect.from_cache,
                        redirect.negotiated_http_version,
                    )
                });
                for session_id in &target.session_ids {
                    emit::emit_main_document_request_will_be_sent(
                        output,
                        session_id.as_deref(),
                        &target.request_id,
                        &target.frame_id,
                        &target.loader_id,
                        target.timestamp,
                        &url,
                        &method,
                        request_body.as_deref(),
                        &request_headers,
                        request_initiator_type,
                        redirect_response,
                        redirect_has_extra_info,
                        cookie_access_report.as_ref(),
                    );
                }
            }
            Self::RequestWillBeSentExtraInfo {
                target,
                request_headers,
                cookie_access_report,
            } => {
                for session_id in &target.session_ids {
                    emit::emit_request_will_be_sent_extra_info(
                        output,
                        session_id.as_deref(),
                        &target.request_id,
                        &request_headers,
                        &cookie_access_report,
                        target.timestamp,
                    );
                }
            }
            Self::ResponseReceivedExtraInfo {
                target,
                headers,
                status,
                cookie_set_reports,
            } => {
                for session_id in &target.session_ids {
                    emit::emit_response_received_extra_info(
                        output,
                        session_id.as_deref(),
                        &target.request_id,
                        &headers,
                        status,
                        &cookie_set_reports,
                    );
                }
            }
            Self::ResponseReceived {
                target,
                final_url,
                status,
                headers,
                cookie_set_reports,
                extra_info_status,
                extra_info_headers,
                network_extra_info_available,
                emit_extra_info,
                encoded_data_length,
                from_cache,
                negotiated_http_version,
                has_extra_info,
            } => {
                for session_id in &target.session_ids {
                    emit::emit_main_document_response_received(
                        output,
                        session_id.as_deref(),
                        &target.request_id,
                        &target.frame_id,
                        &target.loader_id,
                        target.timestamp,
                        &final_url,
                        status,
                        &headers,
                        &cookie_set_reports,
                        extra_info_status,
                        &extra_info_headers,
                        network_extra_info_available,
                        emit_extra_info,
                        encoded_data_length,
                        from_cache,
                        negotiated_http_version,
                        has_extra_info,
                    );
                }
            }
            Self::LoadingFinished {
                target,
                encoded_data_length,
            } => {
                for session_id in &target.session_ids {
                    emit::emit_body_finished(
                        output,
                        session_id.as_deref(),
                        &target.request_id,
                        &target.frame_id,
                        &target.loader_id,
                        target.timestamp,
                        encoded_data_length,
                    );
                }
            }
            Self::LoadingFailed { target, error_text } => {
                for session_id in &target.session_ids {
                    emit::emit_loading_failed(
                        output,
                        session_id.as_deref(),
                        &target.request_id,
                        &target.frame_id,
                        &target.loader_id,
                        target.timestamp,
                        &error_text,
                    );
                }
            }
        }
    }
}

fn completed_body_main_document_network_request_id(
    network_enabled: bool,
    request_id: Option<String>,
) -> Option<String> {
    if network_enabled { request_id } else { None }
}

fn record_main_document_response_body_for_network(
    runtime_slot: &mut TargetRuntimeSlot,
    network_enabled: bool,
    request_id: Option<String>,
    session_ids: &[Option<String>],
    collector_ids: Vec<String>,
    collection_was_gated: bool,
    synthetic: bool,
    response_body: &CapturedBody,
) -> Option<String> {
    if !network_enabled || synthetic {
        return None;
    }
    let request_id = request_id?;
    runtime_slot.record_captured_response_body_source_with_collector_scope(
        request_id.clone(),
        response_body.clone(),
        session_ids.iter().cloned(),
        collector_ids,
        collection_was_gated,
    );
    Some(request_id)
}

fn record_pending_main_document_response_body(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    session_ids: &[Option<String>],
) {
    if !main_document_network_observed(conn, state.session_id.as_deref()) {
        return;
    }
    let Some(request_id) = state.request_id.clone() else {
        return;
    };
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        state.session_id.as_deref(),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        0,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
    );
    if let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(state.session_id.as_deref()) {
        runtime_slot.record_pending_response_body_with_collector_scope(
            request_id,
            session_ids.iter().cloned(),
            collector_ids,
            collection_was_gated,
        );
    }
}

pub(crate) fn record_main_document_request_body(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
) {
    if !main_document_network_observed(conn, state.session_id.as_deref()) {
        return;
    }
    let Some(request_id) = state.request_id.clone() else {
        return;
    };
    let Some(transport_bytes) = state.clone_request_body_bytes() else {
        return;
    };
    let session_ids = main_document_network_event_session_ids(conn, state.session_id.as_deref());
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Request;
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        state.session_id.as_deref(),
        data_type,
        transport_bytes.len(),
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(data_type);
    // The BiDi network data collector keeps the raw transport bytes, including
    // multipart file payloads.
    conn.record_collected_network_data_body(
        request_id.clone(),
        data_type,
        crate::conn::CapturedBody::from_bytes(transport_bytes.clone()),
        collector_ids.iter().cloned(),
        collection_was_gated,
    );
    if let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(state.session_id.as_deref()) {
        // CDP getRequestPostData uses the text projection that request events
        // and Fetch interception expose, which omits multipart file contents.
        // Never surface raw transport bytes through the CDP captured store.
        let cdp_request_body = state
            .request_body
            .clone()
            .unwrap_or_else(|| String::from_utf8_lossy(&transport_bytes).into_owned());
        runtime_slot.record_captured_request_body_with_collector_scope(
            request_id,
            cdp_request_body.into_bytes(),
            session_ids.iter().cloned(),
            collector_ids,
            collection_was_gated,
        );
    }
}

pub(crate) fn record_failed_main_document_response_body(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    error_text: String,
) {
    if !main_document_network_observed(conn, state.session_id.as_deref()) {
        return;
    }
    let Some(request_id) = state.request_id.clone() else {
        return;
    };
    let session_ids = main_document_network_event_session_ids(conn, state.session_id.as_deref());
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        state.session_id.as_deref(),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        0,
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
    );
    if let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(state.session_id.as_deref()) {
        runtime_slot.record_failed_response_body_with_collector_scope(
            request_id,
            error_text,
            session_ids,
            collector_ids,
            collection_was_gated,
        );
    }
}

pub(crate) fn record_completed_main_document_response_body(
    conn: &mut CdpConnection,
    state: &NavigationDispatchState,
    synthetic: bool,
    response_body: &CapturedBody,
) {
    let network_enabled = main_document_network_observed(conn, state.session_id.as_deref());
    let session_ids = main_document_network_event_session_ids(conn, state.session_id.as_deref());
    let collector_ids = conn.network_data_collector_ids_for_session_owner_body(
        state.session_id.as_deref(),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        response_body.len(),
    );
    let collection_was_gated = conn.network_data_collection_is_gated_for_body(
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
    );
    if network_enabled
        && !synthetic
        && let Some(request_id) = state.request_id.clone()
    {
        conn.record_collected_network_data_body(
            request_id,
            crate::devtools_runtime::DevToolsNetworkDataType::Response,
            response_body.clone(),
            collector_ids.iter().cloned(),
            collection_was_gated,
        );
    }
    if let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut(state.session_id.as_deref()) {
        let _ = record_main_document_response_body_for_network(
            runtime_slot,
            network_enabled,
            state.request_id.clone(),
            &session_ids,
            collector_ids,
            collection_was_gated,
            synthetic,
            response_body,
        );
    }
}

fn main_document_network_observed(conn: &CdpConnection, session_id: Option<&str>) -> bool {
    conn.has_network_event_listeners_for_session_owner(session_id)
}
