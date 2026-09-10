use std::str::FromStr;
use url::Url;

use super::{
    CapturedBody, CdpConnection, ClaimedNavigationRequest, CommandOwnerScope, DocumentFetchCommand,
    DocumentFetchCommandOutcome, NavigationDispatchState,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use moli_cookie_jar::StoredCookieQueryReport;
use moli_core::browser::web_contents::PausedDocumentTransfer;
use moli_core::page::{
    PendingSubresourceContinueOutcome, SubresourceAuthCredentials, SubresourceNetworkRequestHandle,
    SubresourceResourceType,
};
use moli_core::runtime::DetachedParserScriptFetchContinuation;
use moli_fetch::url_pattern_matches;

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
    use crate::devtools_runtime::DevToolsNetworkResourceType;
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
    pub(crate) navigation_permit: super::state::NavigationInterceptionPermit,
    pub navigation: NavigationDispatchState,
    pub(crate) request_cookie_report: Option<StoredCookieQueryReport>,
    pub intercept_response: bool,
    pub response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    pub auth_required_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
}

impl PendingFetchNavigation {
    #[cfg(test)]
    pub(crate) fn test_navigation_permit() -> super::state::NavigationInterceptionPermit {
        super::state::NavigationInterceptionPermit::for_correlation_test()
    }
}

/// A DevTools request-stage projection claimed together with the Browser work
/// addressed by its permit. `request` is absent only after the exact navigation
/// was already superseded or otherwise retired.
#[derive(Debug)]
pub(crate) struct ClaimedFetchNavigation {
    pub(crate) pending: PendingFetchNavigation,
    pub(crate) request: Option<ClaimedNavigationRequest>,
}

impl ClaimedFetchNavigation {
    pub(crate) fn new(
        pending: PendingFetchNavigation,
        request: Option<ClaimedNavigationRequest>,
    ) -> Self {
        Self { pending, request }
    }

    pub(crate) fn apply_overrides(
        &mut self,
        requested_url: Option<Url>,
        method: Option<String>,
        body: Option<String>,
        headers: Option<Vec<(String, String)>>,
    ) {
        if let Some(request) = self.request.as_mut() {
            request.apply_overrides(
                requested_url.clone(),
                method.clone(),
                body.clone(),
                headers.clone(),
            );
        }
        if let Some(requested_url) = requested_url {
            self.pending.navigation.requested_url = requested_url;
        }
        if let Some(method) = method {
            self.pending.navigation.request_method = method;
        }
        if let Some(body) = body {
            self.pending.navigation.set_request_body_text(body);
        }
        if let Some(headers) = headers {
            self.pending.navigation.request_headers = headers;
        }
    }

    pub(crate) fn into_parts(self) -> (PendingFetchNavigation, Option<ClaimedNavigationRequest>) {
        (self.pending, self.request)
    }
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
    pub navigation: NavigationDispatchState,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub(crate) auth_permit: super::state::NavigationInterceptionPermit,
    pub challenge: FetchAuthChallenge,
    pub intercept_response: bool,
    pub response_stage_url_match_policy: ResponseStageUrlMatchPolicy,
    pub auth_stage_chain: Option<Box<PendingSubresourceFetchAuthStageChain>>,
}

impl PendingFetchAuthNavigation {
    #[cfg(test)]
    pub(crate) fn test_auth_permit() -> super::state::NavigationInterceptionPermit {
        super::state::NavigationInterceptionPermit::for_correlation_test()
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

/// DevTools correlation for a Browser-owned response-stage pause.
///
/// The response body, prepared Document and cancellation authority live in
/// the pending WebContents navigation addressed by `permit`.
#[derive(Debug)]
pub struct PendingFetchResponseNavigation {
    pub navigation: NavigationDispatchState,
    pub(crate) permit: super::state::NavigationInterceptionPermit,
    active_body_stream_handle: Option<String>,
}

impl PendingFetchResponseNavigation {
    pub(crate) fn new(
        navigation: NavigationDispatchState,
        permit: super::state::NavigationInterceptionPermit,
    ) -> Self {
        Self {
            navigation,
            permit,
            active_body_stream_handle: None,
        }
    }

    pub(crate) fn owner_session_id(&self) -> Option<&str> {
        self.navigation.session_id.as_deref()
    }

    pub(crate) fn active_body_stream_handle(&self) -> Option<&str> {
        self.active_body_stream_handle.as_deref()
    }

    pub(crate) fn set_active_body_stream_handle(&mut self, handle: Option<String>) {
        self.active_body_stream_handle = handle;
    }
}

/// One terminal DevTools decision claimed together with its Browser work.
///
/// `transfer` is absent when the exact navigation retired or a body reader
/// holds its exclusive claim. The exact Core permit distinguishes a live
/// decision from retirement without selecting a current Target/Document.
#[derive(Debug)]
pub(crate) struct ClaimedFetchResponseNavigation {
    request_id: String,
    pending: PendingFetchResponseNavigation,
    transfer: Option<PausedDocumentTransfer>,
}

impl ClaimedFetchResponseNavigation {
    pub(crate) fn has_active_body_stream(&self) -> bool {
        self.transfer
            .as_ref()
            .is_some_and(|transfer| transfer.body_stream_offset().is_some())
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PendingFetchResponseNavigation,
        Option<PausedDocumentTransfer>,
    ) {
        (self.pending, self.transfer)
    }
    pub(crate) fn new(
        request_id: String,
        pending: PendingFetchResponseNavigation,
        transfer: Option<PausedDocumentTransfer>,
    ) -> Self {
        Self {
            request_id,
            pending,
            transfer,
        }
    }

    pub(crate) fn into_restore_parts(
        self,
    ) -> Option<(
        String,
        PendingFetchResponseNavigation,
        PausedDocumentTransfer,
    )> {
        Some((self.request_id, self.pending, self.transfer?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingFetchResponseBodyStreamRead {
    NotFound,
    Read { bytes: Vec<u8>, eof: bool },
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum PendingFetchResponseBodyStreamReadStart {
    NotFound,
    OffsetNotSupported,
    Pending(Box<PendingFetchResponseBodyStreamReadDispatch>),
}

#[derive(Debug)]
pub(crate) struct PendingFetchResponseBodyStreamReadDispatch {
    request_id: String,
    handle: String,
    transfer: PausedDocumentTransfer,
    size: Option<usize>,
}

#[derive(Debug)]
pub(crate) struct CompletedFetchResponseBodyStreamReadDispatch {
    request_id: String,
    handle: String,
    completed:
        Result<(Vec<u8>, bool, PausedDocumentTransfer), Box<(PausedDocumentTransfer, String)>>,
}

impl PendingFetchResponseBodyStreamReadDispatch {
    pub(crate) fn new(
        request_id: String,
        handle: String,
        transfer: PausedDocumentTransfer,
        size: Option<usize>,
    ) -> Self {
        Self {
            request_id,
            handle,
            transfer,
            size,
        }
    }

    pub(crate) async fn wait(self) -> CompletedFetchResponseBodyStreamReadDispatch {
        let Self {
            request_id,
            handle,
            transfer,
            size,
        } = self;
        CompletedFetchResponseBodyStreamReadDispatch {
            request_id,
            handle,
            completed: transfer
                .read_body_stream_async(size)
                .await
                .map_err(Box::new),
        }
    }
}

impl CompletedFetchResponseBodyStreamReadDispatch {
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn handle(&self) -> &str {
        &self.handle
    }

    pub(crate) fn into_completed(
        self,
    ) -> Result<(Vec<u8>, bool, PausedDocumentTransfer), Box<(PausedDocumentTransfer, String)>>
    {
        self.completed
    }
}

/// Installed requests use an exact Document owner; parser continuations carry
/// their own single-use authority before the Document is installed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PendingSubresourceFetchResidence {
    InstalledPage(super::state::TargetPageResidenceIdentity),
    DetachedParserScript(DetachedParserScriptFetchContinuation),
    Worker {
        observer: super::state::TargetPageResidenceIdentity,
        pause: moli_core::browser::WorkerFetchPause,
    },
}

impl PendingSubresourceFetchResidence {
    pub(crate) fn observer(&self) -> Option<&super::state::TargetPageResidenceIdentity> {
        match self {
            Self::InstalledPage(owner)
            | Self::Worker {
                observer: owner, ..
            } => Some(owner),
            Self::DetachedParserScript(_) => None,
        }
    }

    pub(crate) fn worker(&self) -> Option<&moli_core::browser::WorkerFetchPause> {
        match self {
            Self::Worker { pause, .. } => Some(pause),
            _ => None,
        }
    }
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
    pub(crate) fn continuation_key(&self) -> SubresourceFetchKey {
        self.residence
            .worker()
            .map_or(SubresourceFetchKey::Document(self.internal_id), |pause| {
                SubresourceFetchKey::Worker(pause.pause.handle())
            })
    }

    pub(crate) fn observer_page_owner(&self) -> Option<&super::state::TargetPageResidenceIdentity> {
        self.residence.observer()
    }

    pub(crate) fn detached_parser_script_fetch_continuation(
        &self,
    ) -> Option<&DetachedParserScriptFetchContinuation> {
        match &self.residence {
            PendingSubresourceFetchResidence::InstalledPage(_)
            | PendingSubresourceFetchResidence::Worker { .. } => None,
            PendingSubresourceFetchResidence::DetachedParserScript(continuation) => {
                Some(continuation)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SubresourceFetchKey {
    Document(u64),
    Worker(SubresourceNetworkRequestHandle),
}

impl From<u64> for SubresourceFetchKey {
    fn from(id: u64) -> Self {
        Self::Document(id)
    }
}

impl PendingSubresourceFetchRequest {
    pub fn apply_request_stage_continue_modifications(
        &mut self,
        url: Option<Url>,
        method: Option<String>,
        body: Option<String>,
        headers: Option<Vec<(String, String)>>,
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
            chain.headers = headers;
            chain.request_cookie_report = None;
        }
    }

    pub fn accumulated_request_stage_continue_modifications(
        &self,
    ) -> (
        Option<Url>,
        Option<String>,
        Option<Option<String>>,
        Option<Vec<(String, String)>>,
    ) {
        let Some(chain) = self.request_stage_chain.as_ref() else {
            return (None, None, None, None);
        };
        (
            Some(chain.url.clone()),
            Some(chain.method.clone()),
            Some(chain.body.clone()),
            Some(chain.headers.clone()),
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
    pub url: Url,
    pub method: String,
    pub headers: Vec<(String, String)>,
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
    pub url: Url,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
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
    pub url: Url,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
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
        response_headers: Vec<(String, String)>,
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
    pub(crate) fn start_worker_fetch_decision(
        &self,
        pause: &moli_core::browser::WorkerFetchPause,
        command: DocumentFetchCommand,
    ) -> Result<
        (
            moli_core::page::PendingWorkerFetchDecision,
            DocumentFetchCommandOutcome,
        ),
        String,
    > {
        use moli_core::page::WorkerFetchDecision as Decision;
        let continued = matches!(
            &command,
            DocumentFetchCommand::ContinueRequest { .. }
                | DocumentFetchCommand::ContinueAuth { .. }
                | DocumentFetchCommand::CancelAuth { .. }
        );
        let decision = match command {
            DocumentFetchCommand::ContinueRequest {
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
                ..
            } => Decision::ContinueRequest {
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            },
            DocumentFetchCommand::ContinueAuth { auth, .. } => Decision::ProvideAuth(auth),
            DocumentFetchCommand::CancelAuth { .. } => Decision::CancelAuth,
            DocumentFetchCommand::ContinueResponse {
                response_code,
                response_headers,
                ..
            } => Decision::ContinueResponse {
                response_code,
                response_headers,
            },
            DocumentFetchCommand::FailAuth { error_text, .. }
            | DocumentFetchCommand::FailRequest { error_text, .. }
            | DocumentFetchCommand::FailResponse { error_text, .. } => Decision::Fail(error_text),
            DocumentFetchCommand::FulfillRequest {
                response_code,
                response_headers,
                response_body,
                ..
            }
            | DocumentFetchCommand::FulfillResponse {
                response_code,
                response_headers,
                response_body,
                ..
            } => Decision::Fulfill {
                response_code,
                response_headers,
                response_body,
            },
            _ => return Err("Worker Fetch decision cannot control a WebSocket".into()),
        };
        let context = self
            .browser
            .context_handle(pause.document.web_contents().context())?;
        let pending = context.start_worker_fetch_decision(pause.clone(), decision)?;
        let outcome = if continued {
            DocumentFetchCommandOutcome::Continued(PendingSubresourceContinueOutcome::Started)
        } else {
            DocumentFetchCommandOutcome::Complete
        };
        Ok((pending, outcome))
    }

    pub(crate) async fn execute_subresource_fetch_command(
        &mut self,
        owner: &CommandOwnerScope,
        residence: &PendingSubresourceFetchResidence,
        command: DocumentFetchCommand,
    ) -> Result<
        (
            DocumentFetchCommandOutcome,
            Option<moli_core::RendererOutputFence>,
        ),
        String,
    > {
        if let Some(pause) = residence.worker() {
            let (pending, outcome) = self.start_worker_fetch_decision(pause, command)?;
            pending.wait().await?;
            return Ok((outcome, None));
        }
        self.execute_document_fetch_command_for_owner(owner, command)
            .await
    }

    async fn execute_document_fetch_command_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        command: DocumentFetchCommand,
    ) -> Result<
        (
            DocumentFetchCommandOutcome,
            Option<moli_core::RendererOutputFence>,
        ),
        String,
    > {
        let document = self.loaded_browser_document_for_owner(owner)?;
        let pending = self.start_document_fetch_command(document, command)?;
        let completed = pending.wait().await;
        let predecessor = completed.renderer_output_predecessor();
        let outcome = self.finish_document_fetch_command(completed)?;
        Ok((outcome, predecessor))
    }

    pub async fn continue_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
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
        headers: Option<Vec<(String, String)>>,
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
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome, String> {
        self.execute_document_fetch_command_for_owner(
            owner,
            DocumentFetchCommand::ContinueRequest {
                internal_id,
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            },
        )
        .await?
        .0
        .into_continue_outcome()
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
        self.execute_document_fetch_command_for_owner(
            owner,
            DocumentFetchCommand::ContinueAuth { internal_id, auth },
        )
        .await?
        .0
        .into_continue_outcome()
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
        self.execute_document_fetch_command_for_owner(
            owner,
            DocumentFetchCommand::FailAuth {
                internal_id,
                error_text,
            },
        )
        .await
        .map(|(_, predecessor)| predecessor)
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
        self.execute_document_fetch_command_for_owner(
            owner,
            DocumentFetchCommand::FailRequest {
                internal_id,
                error_text,
            },
        )
        .await
        .map(|(_, predecessor)| predecessor)
    }

    pub async fn fulfill_pending_subresource_fetch_async(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
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
        response_headers: Vec<(String, String)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.execute_document_fetch_command_for_owner(
            &owner,
            DocumentFetchCommand::FulfillRequest {
                internal_id,
                response_code,
                response_headers,
                response_body,
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn continue_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
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
        response_headers: Option<Vec<(String, String)>>,
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
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        self.execute_document_fetch_command_for_owner(
            owner,
            DocumentFetchCommand::ContinueResponse {
                internal_id,
                response_code,
                response_headers,
            },
        )
        .await
        .map(|_| ())
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
        self.execute_document_fetch_command_for_owner(
            owner,
            DocumentFetchCommand::FailResponse {
                internal_id,
                error_text,
            },
        )
        .await
        .map(|(_, predecessor)| predecessor)
    }

    pub async fn fulfill_pending_subresource_response_async(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
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
        response_headers: Vec<(String, String)>,
        response_body: moli_core::page::RendererSyntheticResponseBody,
    ) -> Result<(), String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.execute_document_fetch_command_for_owner(
            &owner,
            DocumentFetchCommand::FulfillResponse {
                internal_id,
                response_code,
                response_headers,
                response_body,
            },
        )
        .await
        .map(|_| ())
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
        let owner = CommandOwnerScope::capture(self, session_id);
        self.execute_document_fetch_command_for_owner(
            &owner,
            DocumentFetchCommand::DispatchWebSocketText { socket_id, data },
        )
        .await
        .map(|_| ())
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
        let owner = CommandOwnerScope::capture(self, session_id);
        self.execute_document_fetch_command_for_owner(
            &owner,
            DocumentFetchCommand::DispatchWebSocketBinary { socket_id, data },
        )
        .await
        .map(|_| ())
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
        let owner = CommandOwnerScope::capture(self, session_id);
        self.execute_document_fetch_command_for_owner(
            &owner,
            DocumentFetchCommand::CloseWebSocket {
                socket_id,
                code,
                reason,
            },
        )
        .await
        .map(|_| ())
    }
}
