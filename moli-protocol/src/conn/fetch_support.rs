use anyhow::Context;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;

use super::body_spool::ensure_materialize_limit;
use super::{
    CapturedBody, CapturedBodyWriter, CdpConnection, CommandOwnerScope, DocumentNavigationToken,
    NavigationDispatchState, NavigationLoadOutcome,
};
use crate::automation::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use crate::domains::network::MainDocumentBodyProgressSource;
use moli_cookie_jar::StoredCookieQueryReport;
use moli_core::page::{
    PendingSubresourceContinueOutcome, SubresourceAuthCredentials, SubresourceNetworkRequestHandle,
    SubresourceResourceType,
};
use moli_core::runtime::DetachedParserScriptFetchContinuation;
use moli_fetch::{
    NetworkFetchResult, NetworkObservationJournal, RawResponse, ResponseHead, StreamingRawResponse,
    url_pattern_matches,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
pub enum FetchRequestStage {
    Request,
    Response,
}

impl FetchRequestStage {
    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchInterceptionPattern {
    pub url_pattern: String,
    pub resource_type_filter: Option<FetchResourceTypeFilter>,
    pub request_stage: FetchRequestStage,
}

impl FetchInterceptionPattern {
    pub fn matches_request(&self, resource_type: DevToolsNetworkResourceType, url: &Url) -> bool {
        self.resource_type_filter
            .is_none_or(|filter| filter.matches_resource_type(resource_type))
            && url_pattern_matches(&self.url_pattern, url.as_str())
    }
}

pub fn matching_fetch_pattern<'a>(
    patterns: &'a [FetchInterceptionPattern],
    resource_type: DevToolsNetworkResourceType,
    url: &Url,
) -> Option<&'a FetchInterceptionPattern> {
    patterns
        .iter()
        .find(|pattern| pattern.matches_request(resource_type, url))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
pub enum FetchResourceTypeFilter {
    Document,
    Script,
    Stylesheet,
    Image,
    Media,
    TextTrack,
    Fetch,
    EventSource,
    #[strum(serialize = "XHR")]
    Xhr,
    Ping,
    #[strum(serialize = "CSPViolationReport")]
    CspViolationReport,
    WebSocket,
    Other,
}

impl FetchResourceTypeFilter {
    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn matches_resource_type(self, resource_type: DevToolsNetworkResourceType) -> bool {
        match self {
            Self::Fetch | Self::EventSource | Self::Xhr => {
                matches!(
                    resource_type,
                    DevToolsNetworkResourceType::Fetch
                        | DevToolsNetworkResourceType::EventSource
                        | DevToolsNetworkResourceType::Xhr
                )
            }
            Self::Document => resource_type == DevToolsNetworkResourceType::Document,
            Self::Script => resource_type == DevToolsNetworkResourceType::Script,
            Self::Stylesheet => resource_type == DevToolsNetworkResourceType::Stylesheet,
            Self::Image => resource_type == DevToolsNetworkResourceType::Image,
            Self::Media => resource_type == DevToolsNetworkResourceType::Media,
            Self::TextTrack => resource_type == DevToolsNetworkResourceType::TextTrack,
            Self::Ping => resource_type == DevToolsNetworkResourceType::Ping,
            Self::CspViolationReport => {
                resource_type == DevToolsNetworkResourceType::CspViolationReport
            }
            Self::WebSocket => resource_type == DevToolsNetworkResourceType::WebSocket,
            Self::Other => resource_type == DevToolsNetworkResourceType::Other,
        }
    }

    pub fn subresource_type(self) -> Option<SubresourceResourceType> {
        match self {
            Self::Document | Self::Stylesheet | Self::Media | Self::TextTrack => None,
            Self::Script => Some(SubresourceResourceType::Script),
            Self::Image => Some(SubresourceResourceType::Image),
            Self::Fetch => Some(SubresourceResourceType::Fetch),
            Self::EventSource => Some(SubresourceResourceType::EventSource),
            Self::Xhr => Some(SubresourceResourceType::Xhr),
            Self::Ping => Some(SubresourceResourceType::Ping),
            Self::CspViolationReport => Some(SubresourceResourceType::CspReport),
            Self::WebSocket => Some(SubresourceResourceType::WebSocket),
            // CDP exposes several Blink resource types through the broad
            // "Other" token. Moli currently only produces this token for
            // compression-dictionary link fetches, so narrowing an `Other`
            // Fetch.enable pattern to Dictionary preserves the scheduler
            // request kind when the client continues the paused request. If
            // another internal resource starts reporting as "Other", this
            // filter must become a small set instead of a single type.
            Self::Other => Some(SubresourceResourceType::Dictionary),
        }
    }

    pub fn supports_fetch_enable(self) -> bool {
        matches!(
            self,
            Self::Document
                | Self::Script
                | Self::Image
                | Self::Fetch
                | Self::EventSource
                | Self::Xhr
                | Self::Ping
                | Self::CspViolationReport
                | Self::WebSocket
                | Self::Other
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FetchInterceptionPattern, FetchRequestStage, FetchResourceTypeFilter,
        fetch_subresource_interception_config_for_patterns, matching_fetch_pattern,
    };
    use crate::automation::DevToolsNetworkResourceType;
    use moli_core::page::SubresourceResourceType;
    use url::Url;

    #[test]
    fn fetch_request_stage_parses_cdp_tokens() {
        for (raw, expected) in [
            ("Request", FetchRequestStage::Request),
            ("Response", FetchRequestStage::Response),
        ] {
            let parsed =
                FetchRequestStage::parse(raw).expect("CDP Fetch requestStage token should parse");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.label(), raw);
        }
        assert!(FetchRequestStage::parse("request").is_none());
        assert!(FetchRequestStage::parse("Both").is_none());
    }

    #[test]
    fn fetch_resource_type_filter_parses_cdp_tokens() {
        for (raw, expected) in [
            ("Document", FetchResourceTypeFilter::Document),
            ("Script", FetchResourceTypeFilter::Script),
            ("Stylesheet", FetchResourceTypeFilter::Stylesheet),
            ("Image", FetchResourceTypeFilter::Image),
            ("Media", FetchResourceTypeFilter::Media),
            ("TextTrack", FetchResourceTypeFilter::TextTrack),
            ("Fetch", FetchResourceTypeFilter::Fetch),
            ("EventSource", FetchResourceTypeFilter::EventSource),
            ("XHR", FetchResourceTypeFilter::Xhr),
            ("Ping", FetchResourceTypeFilter::Ping),
            (
                "CSPViolationReport",
                FetchResourceTypeFilter::CspViolationReport,
            ),
            ("WebSocket", FetchResourceTypeFilter::WebSocket),
            ("Other", FetchResourceTypeFilter::Other),
        ] {
            let parsed = FetchResourceTypeFilter::parse(raw)
                .expect("CDP Fetch resourceType token should parse");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.label(), raw);
            assert!(
                parsed.matches_resource_type(
                    DevToolsNetworkResourceType::from_cdp_type(raw)
                        .expect("supported Fetch filter should be a CDP network resource type"),
                )
            );
        }
        assert!(FetchResourceTypeFilter::parse("xhr").is_none());
    }

    #[test]
    fn matching_fetch_pattern_filters_by_resource_type_and_url() {
        let patterns = vec![
            FetchInterceptionPattern {
                url_pattern: "*://example.test/script.js".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Script),
                request_stage: FetchRequestStage::Request,
            },
            FetchInterceptionPattern {
                url_pattern: "*://example.test/api".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
                request_stage: FetchRequestStage::Response,
            },
        ];
        let url = Url::parse("https://example.test/api").unwrap();

        let matched =
            matching_fetch_pattern(&patterns, DevToolsNetworkResourceType::Fetch, &url).unwrap();
        assert_eq!(matched.request_stage, FetchRequestStage::Response);
        assert!(
            matching_fetch_pattern(&patterns, DevToolsNetworkResourceType::Image, &url).is_none()
        );
        let script_url = Url::parse("https://example.test/script.js").unwrap();
        assert!(
            matching_fetch_pattern(&patterns, DevToolsNetworkResourceType::Script, &script_url,)
                .is_some()
        );
    }

    #[test]
    fn fetch_like_filters_match_chromiums_shared_xhr_interception_type() {
        for filter in [
            FetchResourceTypeFilter::Fetch,
            FetchResourceTypeFilter::EventSource,
            FetchResourceTypeFilter::Xhr,
        ] {
            for resource_type in [
                DevToolsNetworkResourceType::Fetch,
                DevToolsNetworkResourceType::EventSource,
                DevToolsNetworkResourceType::Xhr,
            ] {
                assert!(filter.matches_resource_type(resource_type), "{filter:?}");
            }
            assert!(
                !filter.matches_resource_type(DevToolsNetworkResourceType::Script),
                "{filter:?}"
            );
        }
    }

    #[test]
    fn matching_fetch_pattern_accepts_unfiltered_default_pattern() {
        let patterns = vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: None,
            request_stage: FetchRequestStage::Request,
        }];
        let url = Url::parse("https://example.test/style.css").unwrap();

        assert!(
            matching_fetch_pattern(&patterns, DevToolsNetworkResourceType::Stylesheet, &url)
                .is_some()
        );
    }

    #[test]
    fn fetch_resource_type_support_tracks_implemented_interception_paths() {
        assert!(FetchResourceTypeFilter::Script.supports_fetch_enable());
        assert_eq!(
            FetchResourceTypeFilter::Script.subresource_type(),
            Some(SubresourceResourceType::Script)
        );
        for filter in [
            FetchResourceTypeFilter::Stylesheet,
            FetchResourceTypeFilter::Media,
            FetchResourceTypeFilter::TextTrack,
        ] {
            assert!(!filter.supports_fetch_enable(), "{filter:?}");
            assert_eq!(filter.subresource_type(), None, "{filter:?}");
        }
        for filter in [
            FetchResourceTypeFilter::Document,
            FetchResourceTypeFilter::Image,
            FetchResourceTypeFilter::Fetch,
            FetchResourceTypeFilter::EventSource,
            FetchResourceTypeFilter::Xhr,
            FetchResourceTypeFilter::Ping,
            FetchResourceTypeFilter::CspViolationReport,
            FetchResourceTypeFilter::WebSocket,
            FetchResourceTypeFilter::Other,
        ] {
            assert!(filter.supports_fetch_enable(), "{filter:?}");
        }
    }

    #[test]
    fn csp_violation_report_filter_maps_to_csp_report_subresource_type() {
        let patterns = vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::CspViolationReport),
            request_stage: FetchRequestStage::Request,
        }];

        assert_eq!(
            fetch_subresource_interception_config_for_patterns(true, &patterns),
            (true, Some(SubresourceResourceType::CspReport))
        );
    }

    #[test]
    fn other_filter_maps_to_dictionary_subresource_type() {
        let patterns = vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Other),
            request_stage: FetchRequestStage::Request,
        }];

        assert_eq!(
            fetch_subresource_interception_config_for_patterns(true, &patterns),
            (true, Some(SubresourceResourceType::Dictionary))
        );
    }

    #[test]
    fn image_filter_maps_to_image_subresource_type() {
        let patterns = vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Image),
            request_stage: FetchRequestStage::Request,
        }];

        assert_eq!(
            fetch_subresource_interception_config_for_patterns(true, &patterns),
            (true, Some(SubresourceResourceType::Image))
        );
    }

    #[test]
    fn fetch_like_patterns_share_one_renderer_interception_type() {
        let patterns = [
            FetchInterceptionPattern {
                url_pattern: "*/fetch".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
                request_stage: FetchRequestStage::Request,
            },
            FetchInterceptionPattern {
                url_pattern: "*/xhr".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Xhr),
                request_stage: FetchRequestStage::Response,
            },
            FetchInterceptionPattern {
                url_pattern: "*/events".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::EventSource),
                request_stage: FetchRequestStage::Request,
            },
        ];

        assert_eq!(
            fetch_subresource_interception_config_for_patterns(true, &patterns),
            (true, Some(SubresourceResourceType::Fetch))
        );
    }
}

pub fn fetch_subresource_interception_config(
    fetch_enabled: bool,
    filter: Option<FetchResourceTypeFilter>,
) -> (bool, Option<SubresourceResourceType>) {
    if !fetch_enabled {
        return (false, None);
    }
    filter.map_or((true, None), |filter| {
        filter
            .subresource_type()
            .map_or((false, None), |resource_type| (true, Some(resource_type)))
    })
}

pub fn fetch_subresource_interception_config_for_patterns(
    fetch_enabled: bool,
    patterns: &[FetchInterceptionPattern],
) -> (bool, Option<SubresourceResourceType>) {
    if !fetch_enabled {
        return (false, None);
    }
    if patterns.is_empty() {
        return fetch_subresource_interception_config(fetch_enabled, None);
    }

    let mut renderer_resource_type = None;
    for pattern in patterns {
        let Some(filter) = pattern.resource_type_filter else {
            return (true, None);
        };
        let Some(resource_type) = filter.subresource_type() else {
            continue;
        };
        match renderer_resource_type {
            None => renderer_resource_type = Some(resource_type),
            Some(expected) if expected.has_same_cdp_fetch_interception_type(resource_type) => {}
            Some(_) => return (true, None),
        }
    }
    renderer_resource_type.map_or((false, None), |resource_type| (true, Some(resource_type)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseStageUrlMatchPolicy {
    AlreadyMatched,
    MatchFinalUrl,
}

impl ResponseStageUrlMatchPolicy {
    pub(crate) fn requires_final_url_match(self) -> bool {
        self == Self::MatchFinalUrl
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingSubresourceFetchOwnerKind {
    Fetch,
    NetworkOrBidi,
}

impl PendingSubresourceFetchOwnerKind {
    pub(crate) fn drains_on_fetch_disable(self) -> bool {
        matches!(self, Self::Fetch)
    }
}

#[derive(Debug, Clone)]
pub struct PendingFetchNavigation {
    pub fetch_request_id: String,
    pub interception_session_id: Option<String>,
    pub(crate) document_navigation_token: Option<DocumentNavigationToken>,
    pub navigation: NavigationDispatchState,
    pub(crate) request_cookie_report: Option<StoredCookieQueryReport>,
    pub intercept_response: bool,
    pub response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    pub auth_required_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

#[derive(Debug, Clone)]
pub struct PendingFetchAuthNavigation {
    pub owner_session_id: Option<String>,
    // Auth can chain across CDP and BiDi sessions. Keep the current auth action
    // identity separate from the original Fetch response-stage identity.
    pub action_session_id: Option<String>,
    pub interception_session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    // The public auth request id advances with each chained pause; the
    // response stage must retain the id announced by the original request.
    pub fetch_request_id: String,
    pub response_stage_request_id: String,
    pub(crate) document_navigation_token: Option<DocumentNavigationToken>,
    pub navigation: NavigationDispatchState,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub auth_response: Arc<NetworkFetchResult<RawResponse>>,
    pub challenge: FetchAuthChallenge,
    pub intercept_response: bool,
    pub response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    pub auth_stage_chain: Option<Box<PendingSubresourceFetchAuthStageChain>>,
}

impl PendingFetchAuthNavigation {
    #[cfg(test)]
    pub(crate) fn test_auth_response(url: Url) -> Arc<NetworkFetchResult<RawResponse>> {
        Arc::new(NetworkFetchResult::without_request_observation(
            RawResponse::from_head_and_body(
                ResponseHead {
                    final_url: url,
                    status: 401,
                    headers: vec![(
                        "WWW-Authenticate".to_owned(),
                        b"Basic realm=\"test\"".to_vec(),
                    )],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                b"auth required".to_vec(),
            ),
        ))
    }

    pub fn pop_next_auth_required_pause(&mut self) -> Option<PendingSubresourceFetchAuthStage> {
        let chain = self.auth_stage_chain.as_mut()?;
        let next = chain.remaining_sessions.first().cloned()?;
        chain.remaining_sessions.remove(0);
        Some(next)
    }

    pub fn auth_stage_pause_state(&self) -> Option<&PendingSubresourceFetchAuthStageChain> {
        self.auth_stage_chain.as_deref()
    }
}

/// Protocol keeps navigation ownership even while a task exclusively reads the body.
#[derive(Debug)]
pub struct PausedDocumentTransfer {
    fetch_request_id: String,
    document_navigation_token: Option<DocumentNavigationToken>,
    navigation: NavigationDispatchState,
    state: PausedDocumentTransferState,
}

#[derive(Debug)]
enum PausedDocumentTransferState {
    Available(PausedDocumentBody),
    Reading(InFlightDocumentBodyRead),
}

#[derive(Debug)]
enum PausedDocumentBody {
    Pending(Box<DocumentBodySource>),
    ActiveBodyStream {
        handle: String,
        stream: Box<ActiveDocumentBodyStreamState>,
    },
}

/// Dropping a request must also cancel a body that is temporarily owned by a task.
#[derive(Debug)]
struct InFlightDocumentBodyRead {
    id: u64,
    handle: Option<String>,
    cancellation: Option<moli_fetch::FetchCancelHandle>,
}

impl Drop for InFlightDocumentBodyRead {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }
}

#[derive(Debug)]
struct ActiveDocumentBodyStreamState {
    requested_url: Url,
    request_method: String,
    request_headers: moli_fetch::RequestHeaders,
    response: StreamingRawResponse,
    network_observation_journal: NetworkObservationJournal,
    body_progress_source: MainDocumentBodyProgressSource,
    captured_body: CapturedBodyWriter,
    unread_body: Vec<u8>,
    offset: usize,
    finished: bool,
}

#[derive(Debug, Default)]
pub struct PausedDocumentTransfers {
    transfers: HashMap<String, PausedDocumentTransfer>,
    body_stream_handles: HashMap<String, String>,
    next_read_id: u64,
}

/// Carries only response data, never a navigation or its terminal reply.
#[derive(Debug)]
pub(crate) struct PendingDocumentBodyRead {
    request_id: String,
    id: u64,
    body: PausedDocumentBody,
}

#[derive(Debug)]
pub(crate) struct CompletedDocumentBodyRead<T> {
    request_id: String,
    id: u64,
    body: PausedDocumentBody,
    result: anyhow::Result<T>,
}

#[cfg(test)]
impl<T> CompletedDocumentBodyRead<T> {
    pub(crate) fn result(&self) -> &anyhow::Result<T> {
        &self.result
    }
}

impl PausedDocumentTransfers {
    pub(crate) fn is_empty(&self) -> bool {
        self.transfers.is_empty() && self.body_stream_handles.is_empty()
    }

    pub(crate) fn contains_request(&self, request_id: &str) -> bool {
        self.transfers.contains_key(request_id)
    }

    #[cfg(test)]
    pub(crate) fn get(&self, request_id: &str) -> Option<&PausedDocumentTransfer> {
        self.transfers.get(request_id)
    }

    #[cfg(test)]
    pub(crate) fn active_body_stream_request_id(&self, handle: &str) -> Option<&str> {
        self.body_stream_handles.get(handle).map(String::as_str)
    }

    pub(crate) fn clear(&mut self) {
        self.transfers.clear();
        self.body_stream_handles.clear();
    }

    pub(crate) fn drain_pending_transfers(&mut self) -> Vec<PausedDocumentTransfer> {
        self.body_stream_handles.clear();
        std::mem::take(&mut self.transfers).into_values().collect()
    }

    pub(crate) fn drain_superseded(
        &mut self,
        current: &DocumentNavigationToken,
    ) -> Vec<PausedDocumentTransfer> {
        let request_ids = self
            .transfers
            .iter()
            .filter(|(_, transfer)| transfer.navigation_token() != Some(current))
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        request_ids
            .into_iter()
            .filter_map(|id| self.take(&id))
            .collect()
    }

    pub(crate) fn drain_pending_transfers_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<PausedDocumentTransfer> {
        let request_ids = self
            .transfers
            .iter()
            .filter(|(_, transfer)| {
                let owner_session_id = transfer.navigation.session_id.as_deref();
                session_id.is_none() || owner_session_id.is_none() || owner_session_id == session_id
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        request_ids
            .into_iter()
            .filter_map(|id| self.take(&id))
            .collect()
    }

    pub(crate) fn drop_active_body_streams(&mut self) -> Vec<String> {
        let mut dropped_request_ids = Vec::new();
        self.transfers.retain(|request_id, transfer| {
            let keep = !matches!(
                transfer.state,
                PausedDocumentTransferState::Available(PausedDocumentBody::ActiveBodyStream { .. })
            );
            if !keep {
                dropped_request_ids.push(request_id.clone());
            }
            keep
        });
        self.body_stream_handles
            .retain(|_, request_id| self.transfers.contains_key(request_id));
        dropped_request_ids
    }

    pub(crate) fn take(&mut self, request_id: &str) -> Option<PausedDocumentTransfer> {
        let transfer = self.transfers.remove(request_id)?;
        self.clear_body_stream_handle_owner(request_id);
        Some(transfer)
    }

    /// Continuing, opening, or replacing a response requires its body to be available.
    pub(crate) fn take_available(&mut self, request_id: &str) -> Option<PausedDocumentTransfer> {
        if matches!(
            self.transfers.get(request_id)?.state,
            PausedDocumentTransferState::Reading(_)
        ) {
            return None;
        }
        self.take(request_id)
    }

    pub(crate) fn register(&mut self, request_id: String, transfer: PausedDocumentTransfer) {
        self.clear_body_stream_handle_owner(&request_id);
        if let Some(handle) = transfer.active_body_stream_handle() {
            self.body_stream_handles
                .insert(handle.to_owned(), request_id.clone());
        }
        self.transfers.insert(request_id, transfer);
    }

    pub(crate) fn register_pending_navigation(
        &mut self,
        request_id: String,
        document_navigation_token: Option<DocumentNavigationToken>,
        navigation: NavigationDispatchState,
        body: DocumentBodySource,
    ) {
        self.register(
            request_id.clone(),
            PausedDocumentTransfer::pending(
                request_id,
                document_navigation_token,
                navigation,
                body,
            ),
        );
    }

    pub(crate) fn take_body_stream_by_handle(
        &mut self,
        handle: &str,
    ) -> Option<(String, PausedDocumentTransfer)> {
        let request_id = self.body_stream_handles.get(handle)?.clone();
        self.take_available(&request_id)
            .map(|transfer| (request_id, transfer))
    }

    pub(crate) fn start_body_read(&mut self, request_id: &str) -> Option<PendingDocumentBodyRead> {
        let transfer = self.transfers.get_mut(request_id)?;
        let PausedDocumentTransferState::Available(body) = &transfer.state else {
            return None;
        };
        let cancellation = match body {
            PausedDocumentBody::Pending(body) => match body.as_ref() {
                DocumentBodySource::StreamingRaw { response, .. } => {
                    Some(response.cancellation_handle())
                }
                _ => None,
            },
            PausedDocumentBody::ActiveBodyStream { stream, .. } => {
                Some(stream.response.cancellation_handle())
            }
        };
        let id = self.next_read_id;
        self.next_read_id = id.checked_add(1).expect("body read id exhausted");
        let reading = InFlightDocumentBodyRead {
            id,
            handle: transfer.active_body_stream_handle().map(str::to_owned),
            cancellation,
        };
        let PausedDocumentTransferState::Available(body) = std::mem::replace(
            &mut transfer.state,
            PausedDocumentTransferState::Reading(reading),
        ) else {
            unreachable!("the response body was checked before starting its read")
        };
        Some(PendingDocumentBodyRead {
            request_id: request_id.to_owned(),
            id,
            body,
        })
    }

    /// A completion can update only the exact read that still owns this record.
    /// Removal, cancellation, or replacement of the record is final.
    pub(crate) fn finish_body_read<T>(
        &mut self,
        completed: CompletedDocumentBodyRead<T>,
    ) -> Option<anyhow::Result<T>> {
        let CompletedDocumentBodyRead {
            request_id,
            id,
            body,
            result,
        } = completed;
        let transfer = self.transfers.get_mut(&request_id)?;
        let PausedDocumentTransferState::Reading(reading) = &mut transfer.state else {
            return None;
        };
        if reading.id != id {
            return None;
        }
        // The body is returning to its owner, not being cancelled.
        reading.cancellation = None;
        transfer.state = PausedDocumentTransferState::Available(body);
        let handle = transfer.active_body_stream_handle().map(str::to_owned);
        self.clear_body_stream_handle_owner(&request_id);
        if let Some(handle) = handle {
            self.body_stream_handles.insert(handle, request_id);
        }
        Some(result)
    }

    pub(crate) fn start_body_stream_read(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> PendingFetchResponseBodyStreamReadStart {
        let Some(request_id) = self.body_stream_handles.get(handle).cloned() else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        let Some(transfer) = self.transfers.get(&request_id) else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        let PausedDocumentTransferState::Available(PausedDocumentBody::ActiveBodyStream {
            stream,
            ..
        }) = &transfer.state
        else {
            return PendingFetchResponseBodyStreamReadStart::NotFound;
        };
        if offset.is_some_and(|offset| offset != stream.offset) {
            return PendingFetchResponseBodyStreamReadStart::OffsetNotSupported;
        }
        let read = self
            .start_body_read(&request_id)
            .expect("available stream body");
        PendingFetchResponseBodyStreamReadStart::Pending(Box::new(
            PendingFetchResponseBodyStreamReadDispatch {
                handle: handle.to_owned(),
                read,
                size,
            },
        ))
    }

    fn clear_body_stream_handle_owner(&mut self, request_id: &str) {
        self.body_stream_handles
            .retain(|_, owner| owner != request_id);
    }
}

#[derive(Debug)]
pub struct PendingFetchResponseOpenedBodyStream {
    pub handle: String,
    pub buffered_bytes: Option<Vec<u8>>,
    pub transfer: PausedDocumentTransfer,
}

#[derive(Debug)]
pub(crate) enum PendingFetchResponseBodyStreamRead {
    NotFound,
    Read { bytes: Vec<u8>, eof: bool },
    Failed(anyhow::Error),
}

#[derive(Debug)]
pub(crate) enum PendingFetchResponseBodyStreamReadStart {
    NotFound,
    OffsetNotSupported,
    Pending(Box<PendingFetchResponseBodyStreamReadDispatch>),
}

#[derive(Debug)]
pub(crate) struct PendingFetchResponseBodyStreamReadDispatch {
    handle: String,
    read: PendingDocumentBodyRead,
    size: Option<usize>,
}

#[derive(Debug)]
pub(crate) struct CompletedFetchResponseBodyStreamReadDispatch {
    pub(crate) handle: String,
    pub(crate) completed: CompletedDocumentBodyRead<(Vec<u8>, bool)>,
}

impl PendingFetchResponseBodyStreamReadDispatch {
    pub(crate) async fn wait(self) -> CompletedFetchResponseBodyStreamReadDispatch {
        CompletedFetchResponseBodyStreamReadDispatch {
            handle: self.handle,
            completed: self.read.read_stream(self.size).await,
        }
    }
}

impl PendingDocumentBodyRead {
    pub(crate) async fn materialize(
        self,
        limit: usize,
    ) -> CompletedDocumentBodyRead<Option<Vec<u8>>> {
        let (body, result) = match self.body {
            PausedDocumentBody::Pending(body) => {
                match body.materialize_body_limited_async(limit).await {
                    Ok((bytes, body)) => {
                        (PausedDocumentBody::Pending(Box::new(body)), Ok(Some(bytes)))
                    }
                    Err((error, body)) => (PausedDocumentBody::Pending(Box::new(body)), Err(error)),
                }
            }
            body => (body, Ok(None)),
        };
        CompletedDocumentBodyRead {
            request_id: self.request_id,
            id: self.id,
            body,
            result,
        }
    }

    async fn read_stream(self, size: Option<usize>) -> CompletedDocumentBodyRead<(Vec<u8>, bool)> {
        let mut body = self.body;
        let result = match &mut body {
            PausedDocumentBody::ActiveBodyStream { stream, .. } => {
                match stream.read_async(size).await {
                    Ok((bytes, true)) => match stream.finish_pending_body_source() {
                        Ok(source) => {
                            body = PausedDocumentBody::Pending(Box::new(source));
                            Ok((bytes, true))
                        }
                        Err(error) => Err(error),
                    },
                    result => result,
                }
            }
            PausedDocumentBody::Pending(_) => Err(anyhow::anyhow!("StreamHandleNotFound")),
        };
        CompletedDocumentBodyRead {
            request_id: self.request_id,
            id: self.id,
            body,
            result,
        }
    }
}

#[derive(Debug)]
pub(crate) enum OpenBodyStreamError {
    NotOpenable(Box<PausedDocumentTransfer>),
    Failed {
        transfer: Box<PausedDocumentTransfer>,
        error: anyhow::Error,
    },
}

pub(crate) struct PendingStreamingDocumentResponseNavigation {
    pub(crate) document_navigation_token: DocumentNavigationToken,
    pub(crate) navigation: NavigationDispatchState,
    pub(crate) response: StreamingRawResponse,
    pub(crate) network_observation_journal: NetworkObservationJournal,
    pub(crate) body_progress_source: MainDocumentBodyProgressSource,
}

impl PausedDocumentTransfer {
    pub(crate) fn navigation_token(&self) -> Option<&DocumentNavigationToken> {
        self.document_navigation_token.as_ref()
    }

    pub(crate) fn into_navigation(
        self,
    ) -> (Option<DocumentNavigationToken>, NavigationDispatchState) {
        // The available body or in-flight read registration cancels its transport on drop.
        (self.document_navigation_token, self.navigation)
    }

    pub(crate) fn pending(
        fetch_request_id: String,
        document_navigation_token: Option<DocumentNavigationToken>,
        navigation: NavigationDispatchState,
        body: DocumentBodySource,
    ) -> Self {
        Self {
            fetch_request_id,
            document_navigation_token,
            navigation,
            state: PausedDocumentTransferState::Available(PausedDocumentBody::Pending(Box::new(
                body,
            ))),
        }
    }

    pub(crate) fn fetch_request_id(&self) -> &str {
        &self.fetch_request_id
    }

    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        matches!(
            self.state,
            PausedDocumentTransferState::Available(PausedDocumentBody::Pending(_))
        )
    }

    fn active_body_stream_handle(&self) -> Option<&str> {
        match &self.state {
            PausedDocumentTransferState::Available(PausedDocumentBody::ActiveBodyStream {
                handle,
                ..
            }) => Some(handle),
            PausedDocumentTransferState::Reading(reading) => reading.handle.as_deref(),
            PausedDocumentTransferState::Available(PausedDocumentBody::Pending(_)) => None,
        }
    }

    pub(crate) fn open_body_stream(
        self,
        handle: String,
    ) -> Result<PendingFetchResponseOpenedBodyStream, OpenBodyStreamError> {
        let Self {
            fetch_request_id,
            document_navigation_token,
            navigation,
            state,
        } = self;
        let PausedDocumentTransferState::Available(PausedDocumentBody::Pending(body)) = state
        else {
            return Err(OpenBodyStreamError::NotOpenable(Box::new(Self {
                fetch_request_id,
                document_navigation_token,
                navigation,
                state,
            })));
        };
        let (body, buffered_bytes) = match *body {
            DocumentBodySource::StreamingRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
                body_progress_source,
            } => (
                PausedDocumentBody::ActiveBodyStream {
                    handle: handle.clone(),
                    stream: Box::new(ActiveDocumentBodyStreamState {
                        requested_url,
                        request_method,
                        request_headers,
                        response,
                        network_observation_journal,
                        body_progress_source,
                        captured_body: CapturedBodyWriter::default(),
                        unread_body: Vec::new(),
                        offset: 0,
                        finished: false,
                    }),
                },
                None,
            ),
            body => {
                let bytes = match &body {
                    DocumentBodySource::BufferedRaw { response, .. } => {
                        Ok(clone_buffered_raw_body_for_paused_reuse(response))
                    }
                    DocumentBodySource::CapturedRaw { body, .. } => body
                        .materialize_bytes()
                        .context("failed to materialize captured response body"),
                    DocumentBodySource::StreamingRaw { .. } => unreachable!(),
                };
                match bytes {
                    Ok(bytes) => (PausedDocumentBody::Pending(Box::new(body)), Some(bytes)),
                    Err(error) => {
                        return Err(OpenBodyStreamError::Failed {
                            transfer: Box::new(Self::pending(
                                fetch_request_id,
                                document_navigation_token,
                                navigation,
                                body,
                            )),
                            error,
                        });
                    }
                }
            }
        };
        Ok(PendingFetchResponseOpenedBodyStream {
            handle,
            buffered_bytes,
            transfer: Self {
                fetch_request_id,
                document_navigation_token,
                navigation,
                state: PausedDocumentTransferState::Available(body),
            },
        })
    }

    pub(crate) fn into_pending_streaming_document_response_navigation(
        self,
    ) -> Result<PendingStreamingDocumentResponseNavigation, Box<Self>> {
        let Self {
            fetch_request_id,
            document_navigation_token,
            navigation,
            state,
        } = self;
        let (document_navigation_token, body) = match (document_navigation_token, state) {
            (
                Some(token),
                PausedDocumentTransferState::Available(PausedDocumentBody::Pending(body)),
            ) if matches!(body.as_ref(), DocumentBodySource::StreamingRaw { .. }) => (token, body),
            (document_navigation_token, state) => {
                return Err(Box::new(Self {
                    fetch_request_id,
                    document_navigation_token,
                    navigation,
                    state,
                }));
            }
        };
        let DocumentBodySource::StreamingRaw {
            response,
            network_observation_journal,
            body_progress_source,
            ..
        } = *body
        else {
            unreachable!("the response was checked before transferring it")
        };
        Ok(PendingStreamingDocumentResponseNavigation {
            document_navigation_token,
            navigation,
            response,
            network_observation_journal,
            body_progress_source,
        })
    }

    pub(crate) async fn continue_response_async(
        self,
        conn: &mut CdpConnection,
        response_code: Option<u16>,
        response_headers: Vec<(String, Vec<u8>)>,
    ) -> Result<
        (
            Option<DocumentNavigationToken>,
            NavigationDispatchState,
            anyhow::Result<NavigationLoadOutcome>,
        ),
        Self,
    > {
        let Self {
            fetch_request_id,
            document_navigation_token,
            navigation,
            state,
        } = self;
        let PausedDocumentTransferState::Available(PausedDocumentBody::Pending(body)) = state
        else {
            return Err(Self {
                fetch_request_id,
                document_navigation_token,
                navigation,
                state,
            });
        };
        let result = body
            .continue_navigation_async(
                conn,
                document_navigation_token.as_ref(),
                &navigation,
                response_code,
                response_headers,
            )
            .await;
        Ok((document_navigation_token, navigation, result))
    }

    pub(crate) async fn fulfill_synthetic_async(
        self,
        conn: &mut CdpConnection,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        synthetic_body: CapturedBody,
    ) -> (
        Option<DocumentNavigationToken>,
        NavigationDispatchState,
        anyhow::Result<NavigationLoadOutcome>,
    ) {
        let (url, request_cookie_report, body_progress_source) = match &self.state {
            PausedDocumentTransferState::Available(PausedDocumentBody::Pending(body)) => (
                self.navigation.requested_url.clone(),
                body.request_cookie_report().cloned(),
                body.body_progress_source(),
            ),
            PausedDocumentTransferState::Available(PausedDocumentBody::ActiveBodyStream {
                stream,
                ..
            }) => (
                stream.response.final_url.clone(),
                stream.response.request_cookie_report.clone(),
                stream.body_progress_source.clone(),
            ),
            PausedDocumentTransferState::Reading(_) => {
                return self.fail(anyhow::anyhow!("ResponseBodyReadInProgress"));
            }
        };
        let result = conn
            .build_navigation_from_buffered_body_source_for_navigation_async(
                self.document_navigation_token.as_ref(),
                &self.navigation,
                url,
                response_code,
                response_headers,
                synthetic_body,
                request_cookie_report,
                NetworkObservationJournal::default(),
                body_progress_source,
            )
            .await;
        (self.document_navigation_token, self.navigation, result)
    }

    pub(crate) fn fail(
        self,
        error: anyhow::Error,
    ) -> (
        Option<DocumentNavigationToken>,
        NavigationDispatchState,
        anyhow::Result<NavigationLoadOutcome>,
    ) {
        let (token, navigation) = self.into_navigation();
        (token, navigation, Err(error))
    }
}

impl ActiveDocumentBodyStreamState {
    async fn read_async(&mut self, size: Option<usize>) -> anyhow::Result<(Vec<u8>, bool)> {
        read_active_body_stream_async(
            &mut self.response,
            &mut self.captured_body,
            &mut self.unread_body,
            &mut self.offset,
            &mut self.finished,
            size,
        )
        .await
    }

    fn finish_pending_body_source(&mut self) -> anyhow::Result<DocumentBodySource> {
        let head = self.response.head();
        let body = self
            .captured_body
            .finish_in_place()
            .context("failed to finish captured response body")?;
        Ok(DocumentBodySource::CapturedRaw {
            requested_url: self.requested_url.clone(),
            request_method: self.request_method.clone(),
            request_headers: self.request_headers.clone(),
            head,
            body,
            network_observation_journal: self.network_observation_journal.clone(),
            body_progress_source: self.body_progress_source.clone(),
        })
    }
}

#[derive(Debug)]
pub enum DocumentBodySource {
    BufferedRaw {
        requested_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        response: RawResponse,
        network_observation_journal: NetworkObservationJournal,
    },
    StreamingRaw {
        requested_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        response: StreamingRawResponse,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    },
    CapturedRaw {
        requested_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        head: ResponseHead,
        body: CapturedBody,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    },
}

async fn read_active_body_stream_async(
    response: &mut StreamingRawResponse,
    captured_body: &mut CapturedBodyWriter,
    unread_body: &mut Vec<u8>,
    offset: &mut usize,
    finished: &mut bool,
    size: Option<usize>,
) -> anyhow::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    match size {
        Some(limit) => {
            while bytes.len() < limit {
                if unread_body.is_empty() && !*finished {
                    read_next_active_body_stream_chunk_async(
                        response,
                        captured_body,
                        unread_body,
                        finished,
                    )
                    .await?;
                }
                if unread_body.is_empty() {
                    break;
                }
                let remaining = limit.saturating_sub(bytes.len());
                drain_unread_active_body_stream_bytes(unread_body, &mut bytes, remaining);
            }
        }
        None => {
            while !unread_body.is_empty() || !*finished {
                if unread_body.is_empty() {
                    read_next_active_body_stream_chunk_async(
                        response,
                        captured_body,
                        unread_body,
                        finished,
                    )
                    .await?;
                }
                let remaining = unread_body.len();
                drain_unread_active_body_stream_bytes(unread_body, &mut bytes, remaining);
            }
        }
    }
    *offset = offset.saturating_add(bytes.len());

    let eof = *finished && unread_body.is_empty();
    Ok((bytes, eof))
}

fn drain_unread_active_body_stream_bytes(
    unread_body: &mut Vec<u8>,
    bytes: &mut Vec<u8>,
    limit: usize,
) {
    let take = limit.min(unread_body.len());
    bytes.extend(unread_body.drain(..take));
}

async fn read_next_active_body_stream_chunk_async(
    response: &mut StreamingRawResponse,
    captured_body: &mut CapturedBodyWriter,
    unread_body: &mut Vec<u8>,
    finished: &mut bool,
) -> anyhow::Result<()> {
    if *finished {
        return Ok(());
    }
    if let Some(chunk) = response.next_chunk().await {
        captured_body
            .append(&chunk)
            .context("failed to capture response body stream")?;
        unread_body.extend(chunk);
        return Ok(());
    }
    response
        .finish()
        .await
        .context("failed to read page body from stream")?;
    *finished = true;
    Ok(())
}

fn clone_buffered_raw_body_for_paused_reuse(response: &RawResponse) -> Vec<u8> {
    // Fetch.getResponseBody and Fetch.takeResponseBodyAsStream can inspect a
    // buffered response-stage body while the paused request remains resumable.
    // Until paused state owns a shared/spooled body source, that requires an
    // explicit clone rather than consuming the RawResponse.
    response.clone_body_bytes()
}

impl DocumentBodySource {
    pub(crate) fn request_cookie_report(&self) -> Option<&StoredCookieQueryReport> {
        match self {
            Self::BufferedRaw { response, .. } => response.request_cookie_report.as_ref(),
            Self::StreamingRaw { response, .. } => response.request_cookie_report.as_ref(),
            Self::CapturedRaw { head, .. } => head.request_cookie_report.as_ref(),
        }
    }

    fn body_progress_source(&self) -> MainDocumentBodyProgressSource {
        match self {
            Self::StreamingRaw {
                body_progress_source,
                ..
            }
            | Self::CapturedRaw {
                body_progress_source,
                ..
            } => body_progress_source.clone(),
            Self::BufferedRaw { .. } => MainDocumentBodyProgressSource::default(),
        }
    }

    pub(crate) async fn continue_navigation_async(
        self,
        conn: &mut CdpConnection,
        token: Option<&DocumentNavigationToken>,
        navigation: &NavigationDispatchState,
        response_code: Option<u16>,
        response_headers: Vec<(String, Vec<u8>)>,
    ) -> anyhow::Result<NavigationLoadOutcome> {
        let has_response_override = response_code.is_some() || !response_headers.is_empty();
        match self {
            Self::BufferedRaw {
                response,
                network_observation_journal,
                ..
            } => {
                if !has_response_override {
                    conn.build_navigation_from_buffered_raw_response_for_navigation_async(
                        token,
                        navigation,
                        NetworkFetchResult::with_observation_journal(
                            response,
                            network_observation_journal,
                        ),
                    )
                    .await
                } else {
                    let (head, body) = response.into_body();
                    let status = head.status;
                    let headers = head.headers.clone();
                    let final_url = head.final_url.clone();
                    let request_cookie_report = head.request_cookie_report.clone();
                    let body = body
                        .try_into_materialized_bytes()
                        .expect("RawResponse body should remain materialized at the response override boundary");
                    let body = CapturedBody::from_bytes(body);
                    conn.build_navigation_from_buffered_body_source_for_navigation_async(
                        token,
                        navigation,
                        final_url,
                        response_code.unwrap_or(status),
                        if response_headers.is_empty() {
                            headers
                        } else {
                            response_headers
                        },
                        body,
                        request_cookie_report,
                        network_observation_journal,
                        MainDocumentBodyProgressSource::default(),
                    )
                    .await
                }
            }
            Self::StreamingRaw {
                response,
                network_observation_journal,
                body_progress_source,
                ..
            } => {
                conn.build_navigation_from_streaming_raw_response_with_response_override_for_navigation_async(
                    token,
                    navigation,
                    NetworkFetchResult::with_observation_journal(
                        response,
                        network_observation_journal,
                    ),
                    response_code,
                    response_headers,
                    body_progress_source,
                )
                .await
            }
            Self::CapturedRaw {
                head,
                body,
                network_observation_journal,
                body_progress_source,
                ..
            } => {
                if !has_response_override {
                    conn.build_navigation_from_captured_raw_response_for_navigation_async(
                        token,
                        navigation,
                        head,
                        body,
                        network_observation_journal,
                        body_progress_source,
                    )
                    .await
                } else {
                    let status = head.status;
                    let headers = head.headers.clone();
                    let final_url = head.final_url.clone();
                    let request_cookie_report = head.request_cookie_report.clone();
                    conn.build_navigation_from_buffered_body_source_for_navigation_async(
                        token,
                        navigation,
                        final_url,
                        response_code.unwrap_or(status),
                        if response_headers.is_empty() {
                            headers
                        } else {
                            response_headers
                        },
                        body,
                        request_cookie_report,
                        network_observation_journal,
                        body_progress_source,
                    )
                    .await
                }
            }
        }
    }

    pub(crate) async fn materialize_body_limited_async(
        self,
        limit: usize,
    ) -> Result<(Vec<u8>, Self), (anyhow::Error, Self)> {
        match self {
            Self::BufferedRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
            } => {
                if let Err(error) = ensure_materialize_limit(response.body_bytes().len(), limit) {
                    return Err((
                        error,
                        Self::BufferedRaw {
                            requested_url,
                            request_method,
                            request_headers,
                            response,
                            network_observation_journal,
                        },
                    ));
                }
                let bytes = clone_buffered_raw_body_for_paused_reuse(&response);
                Ok((
                    bytes,
                    Self::BufferedRaw {
                        requested_url,
                        request_method,
                        request_headers,
                        response,
                        network_observation_journal,
                    },
                ))
            }
            Self::StreamingRaw {
                requested_url,
                request_method,
                request_headers,
                response,
                network_observation_journal,
                body_progress_source,
            } => {
                let preserved_head = response.head();
                let (head, body) = match capture_streaming_raw_response(response).await {
                    Ok(captured) => captured,
                    Err(message) => {
                        return Err((
                            message,
                            Self::CapturedRaw {
                                requested_url,
                                request_method,
                                request_headers,
                                head: preserved_head,
                                body: CapturedBody::from_bytes(Vec::new()),
                                network_observation_journal,
                                body_progress_source,
                            },
                        ));
                    }
                };
                let result = body
                    .materialize_bytes_limited(limit)
                    .context("failed to materialize captured response body");
                let source = Self::CapturedRaw {
                    requested_url,
                    request_method,
                    request_headers,
                    head,
                    body,
                    network_observation_journal,
                    body_progress_source,
                };
                match result {
                    Ok(bytes) => Ok((bytes, source)),
                    Err(message) => Err((message, source)),
                }
            }
            Self::CapturedRaw {
                requested_url,
                request_method,
                request_headers,
                head,
                body,
                network_observation_journal,
                body_progress_source,
            } => {
                let result = body
                    .materialize_bytes_limited(limit)
                    .context("failed to materialize captured response body");
                let source = Self::CapturedRaw {
                    requested_url,
                    request_method,
                    request_headers,
                    head,
                    body,
                    network_observation_journal,
                    body_progress_source,
                };
                match result {
                    Ok(bytes) => Ok((bytes, source)),
                    Err(message) => Err((message, source)),
                }
            }
        }
    }
}

async fn capture_streaming_raw_response(
    mut response: StreamingRawResponse,
) -> anyhow::Result<(ResponseHead, CapturedBody)> {
    let head = response.head();
    let mut body = CapturedBodyWriter::default();
    while let Some(chunk) = response.next_chunk().await {
        body.append(&chunk)
            .context("failed to capture response body stream")?;
    }
    response
        .finish()
        .await
        .context("failed to read page body from stream")?;
    let body = body
        .finish()
        .context("failed to finish captured response body")?;
    Ok((head, body))
}

/// Residence that owns a request-stage subresource Fetch pause.
///
/// Runtime requests belong to an installed target-local Page. Parser-blocking
/// script fetches can pause while the destination Page is still being built;
/// their one-shot continuation capability is the authority and no installed Page
/// residence exists yet. Keeping these states disjoint prevents both a stale
/// Page request from reaching a replacement and an in-progress parser request
/// from being rejected merely because its Page has not been installed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PendingSubresourceFetchResidence {
    InstalledPage(super::state::TargetPageResidenceIdentity),
    DetachedParserScript(DetachedParserScriptFetchContinuation),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingSubresourceFetchRequest {
    pub(crate) residence: PendingSubresourceFetchResidence,
    pub owner_session_id: Option<String>,
    pub action_session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    pub internal_id: u64,
    pub network_request_id: String,
    pub network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub frame_id: String,
    pub document_url: Url,
    pub resource_type: SubresourceResourceType,
    pub websocket_socket_id: Option<u64>,
    pub request_stage_chain: Option<Box<PendingSubresourceFetchRequestStageChain>>,
}

impl PendingSubresourceFetchRequest {
    pub(crate) fn installed_page_owner(
        &self,
    ) -> Option<&super::state::TargetPageResidenceIdentity> {
        match &self.residence {
            PendingSubresourceFetchResidence::InstalledPage(owner) => Some(owner),
            PendingSubresourceFetchResidence::DetachedParserScript(_) => None,
        }
    }

    pub(crate) fn detached_parser_script_fetch_continuation(
        &self,
    ) -> Option<&DetachedParserScriptFetchContinuation> {
        match &self.residence {
            PendingSubresourceFetchResidence::InstalledPage(_) => None,
            PendingSubresourceFetchResidence::DetachedParserScript(continuation) => {
                Some(continuation)
            }
        }
    }
}

impl PendingSubresourceFetchRequest {
    pub fn apply_request_stage_continue_modifications(
        &mut self,
        url: Option<Url>,
        method: Option<String>,
        body: Option<String>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
    ) {
        let Some(chain) = self.request_stage_chain.as_mut() else {
            return;
        };
        if let Some(url) = url {
            chain.url = url;
        }
        if let Some(method) = method {
            chain.method = method;
        }
        if let Some(body) = body {
            chain.body = Some(body);
        }
        if let Some(headers) = headers {
            chain.headers = headers.headers().clone();
            chain.header_override = Some(headers);
            chain.request_cookie_report = None;
        }
    }

    pub fn accumulated_request_stage_continue_modifications(
        &self,
    ) -> (
        Option<Url>,
        Option<String>,
        Option<Option<String>>,
        Option<moli_fetch::RequestHeaderOverride>,
    ) {
        let Some(chain) = self.request_stage_chain.as_ref() else {
            return (None, None, None, None);
        };
        (
            Some(chain.url.clone()),
            Some(chain.method.clone()),
            Some(chain.body.clone()),
            chain.header_override.clone(),
        )
    }

    pub fn pop_next_request_stage_pause(&mut self) -> Option<PendingSubresourceFetchRequestStage> {
        let chain = self.request_stage_chain.as_mut()?;
        let next = chain.remaining_sessions.first().cloned()?;
        chain.remaining_sessions.remove(0);
        Some(next)
    }

    pub fn request_stage_pause_state(&self) -> Option<&PendingSubresourceFetchRequestStageChain> {
        self.request_stage_chain.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingSubresourceFetchRequestStageChain {
    pub header_override: Option<moli_fetch::RequestHeaderOverride>,
    pub url: Url,
    pub method: String,
    pub headers: moli_fetch::RequestHeaders,
    pub body: Option<String>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub remaining_sessions: Vec<PendingSubresourceFetchRequestStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceFetchRequestStage {
    pub session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    pub request_id: String,
    pub blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

#[derive(Debug, Clone)]
pub struct InFlightSubresourceFetchRequest {
    pub request_id: Option<String>,
    pub pending: PendingSubresourceFetchRequest,
    pub response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    pub response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

/// Exact protocol-side request state claimed together with one renderer
/// subresource continuation output.
///
/// A terminal completion can race with request-stage pause publication, so it
/// may settle either an already in-flight request or the still-pending pause.
/// Response/auth continuations are only valid for an in-flight request. The
/// fetch owner performs this distinction atomically; output routing must not
/// independently probe both registries by raw `internal_id`.
#[derive(Debug)]
pub(crate) enum ClaimedSubresourceContinueRequest {
    InFlight(InFlightSubresourceFetchRequest),
    PendingCompletion(PendingSubresourceFetchRequest),
}

#[derive(Debug, Clone)]
pub struct PendingSubresourceFetchAuthRequest {
    /// Page residence that owns the paused renderer authentication request.
    pub(crate) page_owner: super::state::TargetPageResidenceIdentity,
    pub owner_session_id: Option<String>,
    pub action_session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    pub internal_id: u64,
    pub network_request_id: String,
    pub network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub frame_id: String,
    pub document_url: Url,
    pub resource_type: SubresourceResourceType,
    pub websocket_socket_id: Option<u64>,
    pub url: Url,
    pub method: String,
    pub request_headers: moli_fetch::RequestHeaders,
    pub request_body: Option<String>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub challenge: FetchAuthChallenge,
    pub intercept_response: bool,
    pub auth_stage_chain: Option<Box<PendingSubresourceFetchAuthStageChain>>,
}

impl PendingSubresourceFetchAuthRequest {
    pub fn pop_next_auth_required_pause(&mut self) -> Option<PendingSubresourceFetchAuthStage> {
        let chain = self.auth_stage_chain.as_mut()?;
        let next = chain.remaining_sessions.first().cloned()?;
        chain.remaining_sessions.remove(0);
        Some(next)
    }

    pub fn auth_stage_pause_state(&self) -> Option<&PendingSubresourceFetchAuthStageChain> {
        self.auth_stage_chain.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceFetchAuthStageChain {
    pub remaining_sessions: Vec<PendingSubresourceFetchAuthStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceFetchAuthStage {
    pub session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    pub request_id: String,
    pub blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

#[derive(Debug, Clone)]
pub struct PendingSubresourceFetchResponseRequest {
    /// Page residence that owns the paused renderer response request.
    pub(crate) page_owner: super::state::TargetPageResidenceIdentity,
    pub owner_session_id: Option<String>,
    pub action_session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    pub internal_id: u64,
    pub network_request_id: String,
    pub network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub frame_id: String,
    pub document_url: Url,
    pub resource_type: SubresourceResourceType,
    pub websocket_socket_id: Option<u64>,
    pub url: Url,
    pub method: String,
    pub request_headers: moli_fetch::RequestHeaders,
    pub request_body: Option<String>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub response_status: u16,
    pub response_headers: Vec<(String, Vec<u8>)>,
    pub response_head_overridden: bool,
    pub response_body_taken_as_stream: bool,
    /// Exact paused response body for `Fetch.getResponseBody` and IO streams.
    pub response_body: CapturedBody,
    pub response_stage_chain: Option<Box<PendingSubresourceFetchResponseStageChain>>,
}

impl PendingSubresourceFetchResponseRequest {
    pub fn pop_next_response_stage_pause(
        &mut self,
    ) -> Option<PendingSubresourceFetchResponseStage> {
        let chain = self.response_stage_chain.as_mut()?;
        let next = chain.remaining_sessions.first().cloned()?;
        chain.remaining_sessions.remove(0);
        Some(next)
    }

    pub fn response_stage_pause_state(&self) -> Option<&PendingSubresourceFetchResponseStageChain> {
        self.response_stage_chain.as_deref()
    }

    pub fn apply_response_head_override(
        &mut self,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
    ) {
        self.response_status = response_status;
        self.response_headers = response_headers;
        self.response_head_overridden = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceFetchResponseStageChain {
    pub remaining_sessions: Vec<PendingSubresourceFetchResponseStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceFetchResponseStage {
    pub session_id: Option<String>,
    pub owner_kind: PendingSubresourceFetchOwnerKind,
    pub request_id: String,
    pub blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

#[derive(Debug, Clone)]
pub struct FetchAuthChallenge {
    pub origin: String,
    pub source: String,
    pub scheme: String,
    pub realm: String,
}

impl CdpConnection {
    pub async fn continue_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<moli_fetch::RequestHeaders>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        self.continue_pending_subresource_fetch_for_session_owner_async(
            None,
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )
        .await
    }

    pub async fn continue_pending_subresource_fetch_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<moli_fetch::RequestHeaders>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.continue_pending_subresource_fetch_for_owner_async(
            &owner,
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )
        .await
    }

    pub(crate) async fn continue_pending_subresource_fetch_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<moli_fetch::RequestHeaders>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.continue_pending_subresource_fetch_async(
            internal_id,
            url,
            method,
            body,
            headers.map(Into::into),
            intercept_response,
            handle_auth_requests,
        )
        .await
        .map_err(|error| format!("subresource fetch continue failed: {error}"))
    }

    pub async fn continue_pending_subresource_auth_async(
        &mut self,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        self.continue_pending_subresource_auth_for_session_owner_async(None, internal_id, auth)
            .await
    }

    pub async fn continue_pending_subresource_auth_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.continue_pending_subresource_auth_for_owner_async(&owner, internal_id, auth)
            .await
    }

    pub(crate) async fn continue_pending_subresource_auth_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        auth: SubresourceAuthCredentials,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.continue_pending_subresource_auth_async(internal_id, auth)
            .await
            .map_err(|error| format!("subresource auth continue failed: {error}"))
    }

    pub async fn fail_pending_subresource_auth_async(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        self.fail_pending_subresource_auth_for_session_owner_async(None, internal_id, error_text)
            .await
    }

    pub async fn fail_pending_subresource_auth_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.fail_pending_subresource_auth_for_owner_async(&owner, internal_id, error_text)
            .await
    }

    pub(crate) async fn fail_pending_subresource_auth_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fail_pending_subresource_auth_async(internal_id, error_text)
            .await
            .map_err(|error| format!("subresource auth fail failed: {error}"))
    }

    pub async fn fail_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        self.fail_pending_subresource_fetch_for_session_owner_async(None, internal_id, error_text)
            .await
    }

    pub async fn fail_pending_subresource_fetch_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.fail_pending_subresource_fetch_for_owner_async(&owner, internal_id, error_text)
            .await
    }

    pub(crate) async fn fail_pending_subresource_fetch_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fail_pending_subresource_fetch_async(internal_id, error_text)
            .await
            .map_err(|error| format!("subresource fetch fail failed: {error}"))
    }

    pub async fn fulfill_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        self.fulfill_pending_subresource_fetch_for_session_owner_async(
            None,
            internal_id,
            response_code,
            response_headers,
            response_body,
        )
        .await
    }

    pub async fn fulfill_pending_subresource_fetch_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        let page = self
            .runtime_session_owner_slot_mut(session_id)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fulfill_pending_subresource_fetch_async(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )
        .await
        .map_err(|error| format!("subresource fetch fulfill failed: {error}"))
    }

    pub async fn continue_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> Result<(), String> {
        self.continue_pending_subresource_response_for_session_owner_async(
            None,
            internal_id,
            response_code,
            response_headers,
        )
        .await
    }

    pub async fn continue_pending_subresource_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> Result<(), String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.continue_pending_subresource_response_for_owner_async(
            &owner,
            internal_id,
            response_code,
            response_headers,
        )
        .await
    }

    pub(crate) async fn continue_pending_subresource_response_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> Result<(), String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.continue_pending_subresource_response_async(
            internal_id,
            response_code,
            response_headers,
        )
        .await
        .map_err(|error| format!("subresource response continue failed: {error}"))
    }

    pub async fn fail_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        self.fail_pending_subresource_response_for_session_owner_async(
            None,
            internal_id,
            error_text,
        )
        .await
    }

    pub async fn fail_pending_subresource_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.fail_pending_subresource_response_for_owner_async(&owner, internal_id, error_text)
            .await
    }

    pub(crate) async fn fail_pending_subresource_response_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        internal_id: u64,
        error_text: String,
    ) -> Result<Option<moli_core::RendererOutputFence>, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fail_pending_subresource_response_async(internal_id, error_text)
            .await
            .map_err(|error| format!("subresource response fail failed: {error}"))
    }

    pub async fn fulfill_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        self.fulfill_pending_subresource_response_for_session_owner_async(
            None,
            internal_id,
            response_code,
            response_headers,
            response_body,
        )
        .await
    }

    pub async fn fulfill_pending_subresource_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        let page = self
            .runtime_session_owner_slot_mut(session_id)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.fulfill_pending_subresource_response_async(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )
        .await
        .map_err(|error| format!("subresource response fulfill failed: {error}"))
    }

    pub async fn receive_synthetic_websocket_text_async(
        &mut self,
        socket_id: u64,
        data: String,
    ) -> Result<(), String> {
        self.receive_synthetic_websocket_text_for_session_owner_async(None, socket_id, data)
            .await
    }

    pub async fn receive_synthetic_websocket_text_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        socket_id: u64,
        data: String,
    ) -> Result<(), String> {
        let page = self
            .runtime_session_owner_slot_mut(session_id)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.receive_synthetic_websocket_text_async(socket_id, data)
            .await
            .map_err(|error| format!("synthetic websocket text dispatch failed: {error}"))
    }

    pub async fn receive_synthetic_websocket_binary_async(
        &mut self,
        socket_id: u64,
        data: Vec<u8>,
    ) -> Result<(), String> {
        self.receive_synthetic_websocket_binary_for_session_owner_async(None, socket_id, data)
            .await
    }

    pub async fn receive_synthetic_websocket_binary_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        socket_id: u64,
        data: Vec<u8>,
    ) -> Result<(), String> {
        let page = self
            .runtime_session_owner_slot_mut(session_id)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.receive_synthetic_websocket_binary_async(socket_id, data)
            .await
            .map_err(|error| format!("synthetic websocket binary dispatch failed: {error}"))
    }

    pub async fn close_synthetic_websocket_from_server_async(
        &mut self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<(), String> {
        self.close_synthetic_websocket_from_server_for_session_owner_async(
            None, socket_id, code, reason,
        )
        .await
    }

    pub async fn close_synthetic_websocket_from_server_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<(), String> {
        let page = self
            .runtime_session_owner_slot_mut(session_id)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.close_synthetic_websocket_from_server_async(socket_id, code, reason)
            .await
            .map_err(|error| format!("synthetic websocket close dispatch failed: {error}"))
    }
}
