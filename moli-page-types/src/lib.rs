//! Shared page and protocol value types for Moli.
//!
//! This crate holds renderer-neutral data structures such as navigation
//! responses, script execution reports, subresource records, and frame
//! snapshots that are shared by the facade, renderer, and CDP crates.

mod inspector_identity;
mod inspector_state;
mod layout;
mod navigation_history;
mod renderer_transport_memory;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use http_auth::parser::ChallengeParser;
use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use moli_dom::NodeId;
use moli_fetch::{
    NegotiatedHttpVersion, NetworkRequestExtraInfo, NetworkResponseExtraInfo, RedirectInfo,
    RequestAuth, RequestAuthScheme, RequestAuthTarget, Response, ResponseBody, ResponseHead,
};
use moli_web_mime::is_json_module_mime;

const SUBRESOURCE_RESPONSE_BODY_MEMORY_LIMIT: usize = 1024 * 1024;
static NEXT_SUBRESOURCE_RESPONSE_BODY_SPOOL_ID: AtomicU64 = AtomicU64::new(1);

pub use inspector_identity::{
    DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId, RendererCallId,
    RendererCallIdOutOfRange, RendererDevToolsAgentToken, RendererDevToolsCommandId,
    RendererInspectorResponseDelivery,
};
pub use layout::LayoutPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentNodeAttributeSnapshot {
    pub local_name: String,
    pub value: String,
}

/// Snapshot-local DOM storage id used inside one captured document payload.
pub type DocumentSnapshotNodeId = NodeId;

/// Stable renderer identity for a node that exists only in the inspector
/// projection and therefore has no independent DOM storage handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentNodeInspectorIdentity {
    MarkerPseudoElement,
    UserAgentShadowTreeNode {
        tree_kind: u16,
        ordinal: u16,
        state: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentNodeAssociatedSnapshot {
    TemplateContent(DocumentNodeSnapshot),
    ContentDocument(DocumentNodeSnapshot),
}

impl DocumentNodeAssociatedSnapshot {
    pub fn node(&self) -> &DocumentNodeSnapshot {
        match self {
            Self::TemplateContent(node) | Self::ContentDocument(node) => node,
        }
    }

    pub fn node_mut(&mut self) -> &mut DocumentNodeSnapshot {
        match self {
            Self::TemplateContent(node) | Self::ContentDocument(node) => node,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentNodeSnapshot {
    pub node_id: DocumentSnapshotNodeId,
    pub parent_id: Option<DocumentSnapshotNodeId>,
    pub inspector_identity: Option<DocumentNodeInspectorIdentity>,
    pub inspector_parent_identity: Option<DocumentNodeInspectorIdentity>,
    pub frontend_node_id: Option<u32>,
    pub parent_frontend_node_id: Option<u32>,
    pub backend_node_id: Option<u32>,
    pub frame_id: Option<String>,
    pub node_type: u8,
    pub node_name: String,
    pub local_name: String,
    pub node_value: String,
    pub child_count: usize,
    pub document_url: String,
    pub base_url: String,
    pub namespace_uri: Option<String>,
    pub attributes: Vec<DocumentNodeAttributeSnapshot>,
    pub document_type_name: Option<String>,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    pub is_element: bool,
    pub has_geometry: bool,
    pub shadow_root_type: Option<String>,
    pub shadow_roots: Vec<DocumentNodeSnapshot>,
    /// CDP pseudo-element type for an inspector-only node such as `marker`.
    /// The originating element remains the snapshot identity owner.
    pub pseudo_type: Option<String>,
    pub pseudo_elements: Vec<DocumentNodeSnapshot>,
    /// A non-child node associated with this DOM node. An HTML template owns a
    /// shallow template-content fragment; an iframe/frame owner can expose its
    /// current content document. Those host categories are mutually exclusive.
    pub associated: Option<Box<DocumentNodeAssociatedSnapshot>>,
    pub children: Vec<DocumentNodeSnapshot>,
}

impl DocumentNodeSnapshot {
    pub fn associated_node(&self) -> Option<&Self> {
        self.associated
            .as_deref()
            .map(|associated| associated.node())
    }

    pub fn associated_node_mut(&mut self) -> Option<&mut Self> {
        self.associated
            .as_deref_mut()
            .map(|associated| associated.node_mut())
    }

    pub fn template_content(&self) -> Option<&Self> {
        match self.associated.as_deref() {
            Some(DocumentNodeAssociatedSnapshot::TemplateContent(content)) => Some(content),
            _ => None,
        }
    }

    pub fn content_document(&self) -> Option<&Self> {
        match self.associated.as_deref() {
            Some(DocumentNodeAssociatedSnapshot::ContentDocument(document)) => Some(document),
            _ => None,
        }
    }
}

pub const RENDERER_BACKEND_NODE_ID_START: u32 = 2_000_000_000;
// Chromium's V8 inspector caps protocol value construction at
// kMaxProtocolDepth = 1000. Blink's DOM deep serialization keeps DOM node
// expansion as a separate maxNodeDepth concern. Keep the same split here:
// protocol/BiDi depth parameters retain their protocol meaning, while these
// implementation caps stop Rust output builders from recursing without bound.
pub const MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH: usize = 1000;
pub const MAX_DOM_OUTPUT_TREE_DEPTH: usize = MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH;
pub const MAX_JSON_OUTPUT_TREE_DEPTH: usize = MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH;

pub const fn is_renderer_backend_node_id(id: u32) -> bool {
    id >= RENDERER_BACKEND_NODE_ID_START
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentNodeObjectSnapshot {
    pub frame_id: Option<String>,
    pub owner_node_id: Option<DocumentSnapshotNodeId>,
    pub node_path: Option<Vec<usize>>,
    pub snapshot: DocumentNodeSnapshot,
}

pub use inspector_state::{
    RendererDomDebuggerDomBreakpointType, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerXhrBreakpoint, RendererInspectorProtocolConfiguration,
    RendererInspectorProtocolConfigurationCommand, RendererInspectorSessionRestoreSnapshot,
    V8InspectorSessionAttach, V8InspectorSessionState,
    renderer_inspector_protocol_configuration_command_from_message,
    renderer_inspector_protocol_configuration_command_from_method,
};
pub use navigation_history::{
    NavigationActivationSeed, NavigationHistoryDocumentId, NavigationHistoryEntryId,
    NavigationHistoryEntryKey, NavigationHistoryEntrySeed, NavigationHistoryMutation,
    NavigationHistorySerializedEntry, NavigationTraversalSeedCandidate, SameDocumentHistoryUpdate,
    apply_child_browsing_context_javascript_url_navigation_to_entry_seed,
    apply_child_browsing_context_navigation_to_entry_seed,
    child_browsing_context_single_entry_seed, cross_document_navigation_seed,
    initial_navigation_history_seed, reload_navigation_seed,
    replace_child_browsing_context_navigation_in_entry_seed, traversal_navigation_seed_candidate,
};

/// A rectangle relative to a DOM node that should be exposed by a
/// scroll-into-view operation.
///
/// CDP's `DOM.scrollIntoViewIfNeeded` accepts this optional payload. Keeping
/// it in the shared page types lets the protocol pass the request to the
/// renderer without erasing it into JSON or reinterpreting it in the browser
/// process.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DomScrollIntoViewRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl DomScrollIntoViewRect {
    /// Creates a relative scroll rectangle when every component is a finite
    /// protocol number.
    ///
    /// Keeping the fields private makes this invariant shared by every command
    /// frontend and prevents non-finite coordinates from reaching renderer
    /// geometry.
    pub fn try_new(x: f64, y: f64, width: f64, height: f64) -> Option<Self> {
        [x, y, width, height]
            .into_iter()
            .all(f64::is_finite)
            .then_some(Self {
                x,
                y,
                width,
                height,
            })
    }

    pub fn x(self) -> f64 {
        self.x
    }

    pub fn y(self) -> f64 {
        self.y
    }

    pub fn width(self) -> f64 {
        self.width
    }

    pub fn height(self) -> f64 {
        self.height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationRedirect {
    pub from_url: Url,
    pub to_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub network_extra_info_available: bool,
    pub request_extra_info: Option<NetworkRequestExtraInfo>,
    pub response_extra_info: Option<NetworkResponseExtraInfo>,
    pub redirect_has_extra_info: bool,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
}

impl From<RedirectInfo> for NavigationRedirect {
    fn from(value: RedirectInfo) -> Self {
        Self {
            from_url: value.from_url,
            to_url: value.to_url,
            status: value.status,
            headers: value.headers,
            network_extra_info_available: value.network_extra_info_available,
            request_extra_info: value.request_extra_info,
            response_extra_info: value.response_extra_info,
            redirect_has_extra_info: value.redirect_has_extra_info,
            request_cookie_report: value.request_cookie_report,
            cookie_set_reports: value.cookie_set_reports,
            from_cache: value.from_cache,
            negotiated_http_version: value.negotiated_http_version,
        }
    }
}

impl From<NavigationRedirect> for RedirectInfo {
    fn from(value: NavigationRedirect) -> Self {
        Self {
            from_url: value.from_url,
            to_url: value.to_url,
            status: value.status,
            headers: value.headers,
            network_extra_info_available: value.network_extra_info_available,
            request_extra_info: value.request_extra_info,
            response_extra_info: value.response_extra_info,
            redirect_has_extra_info: value.redirect_has_extra_info,
            request_cookie_report: value.request_cookie_report,
            cookie_set_reports: value.cookie_set_reports,
            from_cache: value.from_cache,
            negotiated_http_version: value.negotiated_http_version,
        }
    }
}

#[derive(Debug)]
pub struct NavigationResponse {
    pub final_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    body: ResponseBody,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<NavigationRedirect>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_headers: Option<Vec<(String, String)>>,
}

impl Clone for NavigationResponse {
    fn clone(&self) -> Self {
        Self {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            body: self
                .body
                .clone_materialized()
                .expect("NavigationResponse body should remain materialized"),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
            network_request_headers: self.network_request_headers.clone(),
        }
    }
}

impl NavigationResponse {
    pub fn body_text(&self) -> &str {
        self.body
            .as_materialized_text()
            .expect("NavigationResponse body should remain materialized text")
    }

    pub fn body_bytes(&self) -> &[u8] {
        self.body
            .as_materialized_bytes()
            .expect("NavigationResponse body should remain materialized")
    }

    /// Clones the complete materialized byte payload for compatibility callers
    /// whose public contract requires an owned full-body buffer.
    pub fn clone_body_bytes(&self) -> Vec<u8> {
        self.body_bytes().to_vec()
    }

    pub fn materialized_body(&self) -> ResponseBody {
        self.body
            .clone_materialized()
            .expect("NavigationResponse body should remain materialized")
    }

    pub fn head(&self) -> ResponseHead {
        ResponseHead {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self
                .redirect_chain
                .clone()
                .into_iter()
                .map(Into::into)
                .collect(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        }
    }

    pub fn from_head_and_body(head: ResponseHead, body: String, body_bytes: Vec<u8>) -> Self {
        Self {
            final_url: head.final_url,
            status: head.status,
            headers: head.headers,
            body: ResponseBody::materialized_text(body, body_bytes),
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain.into_iter().map(Into::into).collect(),
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
            network_request_headers: None,
        }
    }

    /// Headers configured on the HTTP transfer that produced this response.
    /// `None` distinguishes local, cached, and synthetic responses from a
    /// response observed at the network transport boundary.
    pub fn with_network_request_headers(
        mut self,
        network_request_headers: Option<Vec<(String, String)>>,
    ) -> Self {
        self.network_request_headers = network_request_headers;
        self
    }

    pub fn network_request_headers(&self) -> Option<&[(String, String)]> {
        self.network_request_headers.as_deref()
    }

    pub fn from_head_and_materialized_body(head: ResponseHead, body: ResponseBody) -> Self {
        let (body, body_bytes) = body
            .try_into_lossy_materialized_text()
            .expect("NavigationResponse body should remain materialized text");
        Self::from_head_and_body(head, body, body_bytes)
    }

    pub fn from_head_and_text_body(head: ResponseHead, body: String) -> Self {
        let body_bytes = body.as_bytes().to_vec();
        Self::from_head_and_body(head, body, body_bytes)
    }

    pub fn from_text_body(
        final_url: Url,
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    ) -> Self {
        Self::from_head_and_text_body(
            ResponseHead {
                final_url,
                status,
                headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            body,
        )
    }

    pub fn with_status_headers_from(
        source: &Self,
        status: u16,
        headers: Vec<(String, String)>,
    ) -> Self {
        let mut head = source.head();
        head.status = status;
        head.headers = headers;
        Self::from_head_and_materialized_body(head, source.materialized_body())
            .with_network_request_headers(source.network_request_headers.clone())
    }

    pub fn into_parts(self) -> (ResponseHead, String, Vec<u8>) {
        let head = ResponseHead {
            final_url: self.final_url,
            status: self.status,
            headers: self.headers,
            request_cookie_report: self.request_cookie_report,
            cookie_set_reports: self.cookie_set_reports,
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.into_iter().map(Into::into).collect(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        };
        let (body, body_bytes) = self
            .body
            .try_into_lossy_materialized_text()
            .expect("NavigationResponse body should remain materialized text");
        (head, body, body_bytes)
    }

    /// Consumes a materialized navigation response when the next renderer step
    /// only accepts text and intentionally discards the exact byte payload.
    pub fn into_text_parts(self) -> (ResponseHead, String) {
        let (head, body, _) = self.into_parts();
        (head, body)
    }

    pub fn into_body(self) -> (ResponseHead, ResponseBody) {
        let head = ResponseHead {
            final_url: self.final_url,
            status: self.status,
            headers: self.headers,
            request_cookie_report: self.request_cookie_report,
            cookie_set_reports: self.cookie_set_reports,
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.into_iter().map(Into::into).collect(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        };
        (head, self.body)
    }
}

impl From<Response> for NavigationResponse {
    fn from(value: Response) -> Self {
        let network_request_headers = value
            .network_request_extra_info()
            .map(|extra_info| extra_info.headers.clone());
        let (head, body) = value.into_body();
        Self::from_head_and_materialized_body(head, body)
            .with_network_request_headers(network_request_headers)
    }
}

impl From<NavigationResponse> for Response {
    fn from(value: NavigationResponse) -> Self {
        let network_request_headers = value.network_request_headers.clone();
        let request_cookie_report = value.request_cookie_report.clone().unwrap_or_default();
        let (head, body) = value.into_body();
        Self::from_head_and_materialized_body(head, body)
            .expect("NavigationResponse body should remain materialized at Response boundary")
            .with_network_request_extra_info(network_request_headers.map(|headers| {
                NetworkRequestExtraInfo {
                    headers,
                    cookie_report: request_cookie_report,
                }
            }))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScriptGlobalsSnapshotState {
    #[default]
    Uncaptured,
    Fresh,
    Dirty,
}

#[derive(Debug, Clone, Default)]
pub struct ScriptExecutionReport {
    pub runs: Vec<ScriptRun>,
    globals: Arc<BTreeMap<String, JsValueSnapshot>>,
    globals_snapshot_state: ScriptGlobalsSnapshotState,
    observable_output_items: Vec<ScriptObservableOutputItem>,
    console_messages: Vec<String>,
    lifecycle_errors: Vec<String>,
    inspector_issues: Vec<InspectorIssueSnapshot>,
    network_output_items: Vec<ScriptNetworkOutputItem>,
    subresource_network_records: Vec<SubresourceNetworkRecord>,
    staged_subresource_lifecycle: Option<Box<StagedSubresourceReportState>>,
    websocket_network_events: Vec<WebSocketNetworkEvent>,
    websocket_lifecycle_events: Vec<WebSocketLifecycleEvent>,
}

#[derive(Debug, Clone, Default)]
struct StagedSubresourceReportState {
    requests: BTreeMap<u64, SubresourceRequestStarted>,
    responses: BTreeMap<u64, SubresourceResponseStarted>,
    bodies: BTreeMap<u64, SubresourceBodyFinished>,
    completed_handles: BTreeSet<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptNetworkOutput {
    items: Vec<ScriptNetworkOutputItem>,
    subresource_network_records: Vec<SubresourceNetworkRecord>,
    websocket_network_events: Vec<WebSocketNetworkEvent>,
    websocket_lifecycle_events: Vec<WebSocketLifecycleEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptNetworkOutputItem {
    SubresourceNetworkRecord(Box<SubresourceNetworkRecord>),
    SubresourceRequestStarted(Box<SubresourceRequestStarted>),
    SubresourceResponseStarted(Box<SubresourceResponseStarted>),
    SubresourceDataReceived(SubresourceDataReceived),
    SubresourceEventSourceMessageReceived(Box<SubresourceEventSourceMessageReceived>),
    SubresourceBodyFinished(Box<SubresourceBodyFinished>),
    WebSocketNetworkEvent(WebSocketNetworkEvent),
    WebSocketLifecycleEvent(WebSocketLifecycleEvent),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptObservableOutput {
    items: Vec<ScriptObservableOutputItem>,
    console_messages: Vec<String>,
    lifecycle_errors: Vec<String>,
    inspector_issues: Vec<InspectorIssueSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptObservableOutputItem {
    ConsoleMessage(String),
    LifecycleError(String),
    InspectorIssue(Box<InspectorIssueSnapshot>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorIssueSnapshot {
    QuirksMode(QuirksModeIssueSnapshot),
    ContentSecurityPolicy(ContentSecurityPolicyIssueSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuirksModeIssueSnapshot {
    is_limited_quirks_mode: bool,
    document_node_id: u32,
    url: String,
}

impl QuirksModeIssueSnapshot {
    pub fn new(is_limited_quirks_mode: bool, document_node_id: u32, url: String) -> Self {
        Self {
            is_limited_quirks_mode,
            document_node_id,
            url,
        }
    }

    pub fn is_limited_quirks_mode(&self) -> bool {
        self.is_limited_quirks_mode
    }

    pub fn document_node_id(&self) -> u32 {
        self.document_node_id
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSecurityPolicyViolationType {
    Eval,
    WasmEval,
    Inline,
    TrustedTypesPolicy,
    TrustedTypesSink,
    Url,
    SubresourceIntegrity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorSourceCodeLocationSnapshot {
    url: String,
    line_number: u32,
    column_number: u32,
}

impl InspectorSourceCodeLocationSnapshot {
    pub fn new(url: String, line_number: u32, column_number: u32) -> Self {
        Self {
            url,
            line_number,
            column_number,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn line_number(&self) -> u32 {
        self.line_number
    }

    pub fn column_number(&self) -> u32 {
        self.column_number
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSecurityPolicyIssueSnapshot {
    is_report_only: bool,
    violated_directive: String,
    violation_type: ContentSecurityPolicyViolationType,
    blocked_url: Option<String>,
    source_code_location: Option<InspectorSourceCodeLocationSnapshot>,
    violating_node_id: Option<u32>,
}

impl ContentSecurityPolicyIssueSnapshot {
    pub fn new(
        is_report_only: bool,
        violated_directive: String,
        violation_type: ContentSecurityPolicyViolationType,
    ) -> Self {
        Self {
            is_report_only,
            violated_directive,
            violation_type,
            blocked_url: None,
            source_code_location: None,
            violating_node_id: None,
        }
    }

    pub fn with_blocked_url(mut self, blocked_url: Option<String>) -> Self {
        self.blocked_url = blocked_url;
        self
    }

    pub fn with_source_code_location(
        mut self,
        source_code_location: Option<InspectorSourceCodeLocationSnapshot>,
    ) -> Self {
        self.source_code_location = source_code_location;
        self
    }

    pub fn with_violating_node_id(mut self, violating_node_id: Option<u32>) -> Self {
        self.violating_node_id = violating_node_id;
        self
    }

    pub fn is_report_only(&self) -> bool {
        self.is_report_only
    }

    pub fn violated_directive(&self) -> &str {
        &self.violated_directive
    }

    pub fn violation_type(&self) -> ContentSecurityPolicyViolationType {
        self.violation_type
    }

    pub fn blocked_url(&self) -> Option<&str> {
        self.blocked_url.as_deref()
    }

    pub fn source_code_location(&self) -> Option<&InspectorSourceCodeLocationSnapshot> {
        self.source_code_location.as_ref()
    }

    pub fn violating_node_id(&self) -> Option<u32> {
        self.violating_node_id
    }
}

impl ScriptNetworkOutput {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn from_items(items: impl IntoIterator<Item = ScriptNetworkOutputItem>) -> Self {
        let mut output = Self::default();
        for item in items {
            output.push_item(item);
        }
        output
    }

    pub fn into_items(self) -> impl Iterator<Item = ScriptNetworkOutputItem> {
        self.items.into_iter()
    }

    pub fn push_item(&mut self, item: ScriptNetworkOutputItem) {
        self.items.push(item.clone());
        match item {
            ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                self.subresource_network_records.push(*record);
            }
            ScriptNetworkOutputItem::SubresourceRequestStarted(_)
            | ScriptNetworkOutputItem::SubresourceResponseStarted(_)
            | ScriptNetworkOutputItem::SubresourceDataReceived(_)
            | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
            | ScriptNetworkOutputItem::SubresourceBodyFinished(_) => {}
            ScriptNetworkOutputItem::WebSocketNetworkEvent(event) => {
                self.websocket_network_events.push(event);
            }
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(event) => {
                self.websocket_lifecycle_events.push(event);
            }
        }
    }
}

impl ScriptObservableOutput {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn from_items(items: impl IntoIterator<Item = ScriptObservableOutputItem>) -> Self {
        let mut output = Self::default();
        for item in items {
            output.push_item(item);
        }
        output
    }

    pub fn into_items(self) -> impl Iterator<Item = ScriptObservableOutputItem> {
        self.items.into_iter()
    }

    pub fn push_item(&mut self, item: ScriptObservableOutputItem) {
        self.items.push(item.clone());
        match item {
            ScriptObservableOutputItem::ConsoleMessage(message) => {
                self.console_messages.push(message);
            }
            ScriptObservableOutputItem::LifecycleError(error) => {
                self.lifecycle_errors.push(error);
            }
            ScriptObservableOutputItem::InspectorIssue(issue) => {
                self.inspector_issues.push(*issue);
            }
        }
    }
}

impl ScriptExecutionReport {
    pub fn runs(&self) -> &[ScriptRun] {
        &self.runs
    }

    pub fn globals(&self) -> &BTreeMap<String, JsValueSnapshot> {
        &self.globals
    }

    pub fn global(&self, name: &str) -> Option<&JsValueSnapshot> {
        self.globals.get(name)
    }

    /// Whether [`Self::globals`] reflects the current JavaScript realm.
    ///
    /// [`ScriptGlobalsSnapshotState::Uncaptured`] means the realm was created
    /// without the opt-in diagnostics needed to distinguish browser-provided
    /// globals from page-created globals. Protocol turns deliberately avoid
    /// refreshing an enabled snapshot. While this is
    /// [`ScriptGlobalsSnapshotState::Dirty`], `globals()` and `global()` expose
    /// the last complete snapshot for backward compatibility; callers that
    /// require current values must first request an asynchronous full
    /// page-state refresh.
    pub fn globals_snapshot_state(&self) -> ScriptGlobalsSnapshotState {
        self.globals_snapshot_state
    }

    pub fn globals_are_fresh(&self) -> bool {
        self.globals_snapshot_state == ScriptGlobalsSnapshotState::Fresh
    }

    pub fn fresh_globals(&self) -> Option<&BTreeMap<String, JsValueSnapshot>> {
        self.globals_are_fresh().then_some(self.globals())
    }

    pub fn mark_globals_snapshot_dirty(&mut self) -> bool {
        match self.globals_snapshot_state {
            ScriptGlobalsSnapshotState::Fresh => {
                self.globals_snapshot_state = ScriptGlobalsSnapshotState::Dirty;
                true
            }
            ScriptGlobalsSnapshotState::Uncaptured | ScriptGlobalsSnapshotState::Dirty => false,
        }
    }

    pub fn replace_globals_snapshot(&mut self, globals: BTreeMap<String, JsValueSnapshot>) -> bool {
        if self.globals_snapshot_state == ScriptGlobalsSnapshotState::Fresh
            && self.globals.as_ref() == &globals
        {
            return false;
        }
        self.globals = Arc::new(globals);
        self.globals_snapshot_state = ScriptGlobalsSnapshotState::Fresh;
        true
    }

    pub fn console_messages(&self) -> &[String] {
        &self.console_messages
    }

    pub fn lifecycle_errors(&self) -> &[String] {
        &self.lifecycle_errors
    }

    pub fn inspector_issues(&self) -> &[InspectorIssueSnapshot] {
        &self.inspector_issues
    }

    pub fn subresource_network_records(&self) -> &[SubresourceNetworkRecord] {
        &self.subresource_network_records
    }

    pub fn websocket_network_events(&self) -> &[WebSocketNetworkEvent] {
        &self.websocket_network_events
    }

    pub fn websocket_lifecycle_events(&self) -> &[WebSocketLifecycleEvent] {
        &self.websocket_lifecycle_events
    }

    pub fn network_output_items(&self) -> &[ScriptNetworkOutputItem] {
        &self.network_output_items
    }

    pub fn observable_output_items(&self) -> &[ScriptObservableOutputItem] {
        &self.observable_output_items
    }

    pub fn extend_observable_output(&mut self, output: ScriptObservableOutput) {
        for item in output.into_items() {
            self.push_observable_output_item(item);
        }
    }

    pub fn extend_network_output(&mut self, output: ScriptNetworkOutput) {
        for item in output.into_items() {
            self.push_network_output_item(item);
        }
    }

    fn push_observable_output_item(&mut self, item: ScriptObservableOutputItem) {
        self.observable_output_items.push(item.clone());
        match item {
            ScriptObservableOutputItem::ConsoleMessage(message) => {
                self.console_messages.push(message);
            }
            ScriptObservableOutputItem::LifecycleError(error) => {
                self.lifecycle_errors.push(error);
            }
            ScriptObservableOutputItem::InspectorIssue(issue) => {
                self.inspector_issues.push(*issue);
            }
        }
    }

    fn push_network_output_item(&mut self, item: ScriptNetworkOutputItem) {
        self.network_output_items.push(item.clone());
        match item {
            ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                if let Some(handle) = record.request_handle() {
                    let handle = handle.get();
                    let state = self.staged_subresource_report_state_mut();
                    state.requests.remove(&handle);
                    state.responses.remove(&handle);
                    state.bodies.remove(&handle);
                    if !state.completed_handles.insert(handle) {
                        return;
                    }
                }
                self.subresource_network_records.push(*record);
            }
            ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                let handle = request.handle().get();
                let state = self.staged_subresource_report_state_mut();
                if state.completed_handles.contains(&handle) {
                    return;
                }
                state.requests.insert(handle, *request);
                self.try_materialize_staged_subresource_record(handle);
            }
            ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
                let handle = response.handle().get();
                let state = self.staged_subresource_report_state_mut();
                if state.completed_handles.contains(&handle) {
                    return;
                }
                state.responses.insert(handle, *response);
                self.try_materialize_staged_subresource_record(handle);
            }
            ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
                let handle = body.handle().get();
                let state = self.staged_subresource_report_state_mut();
                if state.completed_handles.contains(&handle) {
                    return;
                }
                state.bodies.insert(handle, *body);
                self.try_materialize_staged_subresource_record(handle);
            }
            ScriptNetworkOutputItem::SubresourceDataReceived(_)
            | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_) => {}
            ScriptNetworkOutputItem::WebSocketNetworkEvent(event) => {
                self.websocket_network_events.push(event);
            }
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(event) => {
                self.websocket_lifecycle_events.push(event);
            }
        }
    }

    fn staged_subresource_report_state_mut(&mut self) -> &mut StagedSubresourceReportState {
        self.staged_subresource_lifecycle
            .get_or_insert_with(|| Box::new(StagedSubresourceReportState::default()))
    }

    fn try_materialize_staged_subresource_record(&mut self, handle: u64) {
        let record = {
            let Some(state) = self.staged_subresource_lifecycle.as_ref() else {
                return;
            };
            let Some(request) = state.requests.get(&handle) else {
                return;
            };
            let Some(body) = state.bodies.get(&handle) else {
                return;
            };
            let response = state.responses.get(&handle);
            let Some(record) =
                SubresourceNetworkRecord::from_staged_lifecycle(request, response, body)
            else {
                return;
            };
            record
        };

        let state = self
            .staged_subresource_lifecycle
            .as_mut()
            .expect("staged subresource state should remain allocated");
        state.requests.remove(&handle);
        state.responses.remove(&handle);
        state.bodies.remove(&handle);
        if state.completed_handles.insert(handle) {
            self.subresource_network_records.push(record);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiPreloadChannelHandoff {
    pub handoff_id: String,
    pub token: String,
    pub channel: String,
    pub ownership: Option<String>,
    pub serialization_options: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentStartScript {
    pub registry_key: Option<String>,
    pub source: String,
    pub world_name: Option<String>,
    pub has_bidi_channel_argument: bool,
    pub bidi_channel_handoffs: Vec<BidiPreloadChannelHandoff>,
}

impl DocumentStartScript {
    pub fn with_registry_key(&self, registry_key: impl Into<String>) -> Self {
        let mut script = self.clone();
        script.registry_key = Some(registry_key.into());
        script
    }
}

#[derive(Debug, Clone)]
pub struct SubresourceNetworkRecord {
    frame_id: Option<String>,
    document_url: Url,
    url: Url,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    websocket_socket_id: Option<u64>,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    request_body_bytes: Option<Vec<u8>>,
    resource_type: SubresourceResourceType,
    request_initiator_type: SubresourceRequestInitiatorType,
    request_cookie_report: Option<StoredCookieQueryReport>,
    outcome: SubresourceNetworkOutcome,
    cookie_set_reports: Vec<StoredCookieSetReport>,
    from_cache: bool,
    network_request_headers: Option<Vec<(String, String)>>,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubresourceNetworkRequestHandle(u64);

impl SubresourceNetworkRequestHandle {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceRequestStarted {
    handle: SubresourceNetworkRequestHandle,
    frame_id: Option<String>,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<String>,
    request_body_bytes: Option<Vec<u8>>,
    keepalive: bool,
    resource_type: SubresourceResourceType,
    request_initiator_type: SubresourceRequestInitiatorType,
    request_cookie_report: Option<StoredCookieQueryReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceResponseStarted {
    handle: SubresourceNetworkRequestHandle,
    redirect_chain: Vec<NavigationRedirect>,
    final_url: Url,
    status: u16,
    status_text: Option<String>,
    response_headers: Vec<(String, String)>,
    cookie_set_reports: Vec<StoredCookieSetReport>,
    from_cache: bool,
    network_request_headers: Option<Vec<(String, String)>>,
    negotiated_http_version: Option<NegotiatedHttpVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceDataReceived {
    handle: SubresourceNetworkRequestHandle,
    data_length: usize,
    encoded_data_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceEventSourceMessageReceived {
    handle: SubresourceNetworkRequestHandle,
    event_name: String,
    event_id: String,
    data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceBodyFinished {
    handle: SubresourceNetworkRequestHandle,
    result: SubresourceBodyFinishedResult,
    data_was_streamed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubresourceBodyFinishedResult {
    Ready(SubresourceResponseBody),
    Failed(String),
    FailedWithPartialBody {
        error_text: String,
        partial_body: SubresourceResponseBody,
    },
}

impl PartialEq for SubresourceNetworkRecord {
    fn eq(&self, other: &Self) -> bool {
        self.frame_id == other.frame_id
            && self.document_url == other.document_url
            && self.url == other.url
            && self.websocket_socket_id == other.websocket_socket_id
            && self.method == other.method
            && self.request_headers == other.request_headers
            && self.request_body == other.request_body
            && self.request_body_bytes == other.request_body_bytes
            && self.resource_type == other.resource_type
            && self.request_initiator_type == other.request_initiator_type
            && self.request_cookie_report == other.request_cookie_report
            && self.outcome == other.outcome
            && self.cookie_set_reports == other.cookie_set_reports
            && self.from_cache == other.from_cache
            && self.network_request_headers == other.network_request_headers
            && self.negotiated_http_version == other.negotiated_http_version
    }
}

impl Eq for SubresourceNetworkRecord {}

impl SubresourceRequestStarted {
    pub fn new(
        handle: SubresourceNetworkRequestHandle,
        frame_id: Option<String>,
        document_url: Url,
        url: Url,
        method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        resource_type: SubresourceResourceType,
        request_initiator_type: SubresourceRequestInitiatorType,
        request_cookie_report: Option<StoredCookieQueryReport>,
    ) -> Self {
        let request_body_bytes = request_body_text_bytes(&request_body);
        Self {
            handle,
            frame_id,
            document_url,
            url,
            method,
            request_headers,
            request_body,
            request_body_bytes,
            keepalive: false,
            resource_type,
            request_initiator_type,
            request_cookie_report,
        }
    }

    pub fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.handle
    }

    pub fn with_request_body_bytes(mut self, request_body_bytes: Option<Vec<u8>>) -> Self {
        self.request_body_bytes =
            request_body_bytes.or_else(|| request_body_text_bytes(&self.request_body));
        self
    }

    pub fn with_keepalive(mut self, keepalive: bool) -> Self {
        self.keepalive = keepalive;
        self
    }

    pub fn frame_id(&self) -> Option<&str> {
        self.frame_id.as_deref()
    }

    pub fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    pub fn request_body(&self) -> Option<&str> {
        self.request_body.as_deref()
    }

    pub fn request_body_bytes(&self) -> Option<&[u8]> {
        self.request_body_bytes.as_deref()
    }

    pub fn keepalive(&self) -> bool {
        self.keepalive
    }

    pub fn resource_type(&self) -> SubresourceResourceType {
        self.resource_type
    }

    pub fn request_initiator_type(&self) -> SubresourceRequestInitiatorType {
        self.request_initiator_type
    }

    pub fn request_cookie_report(&self) -> Option<&StoredCookieQueryReport> {
        self.request_cookie_report.as_ref()
    }
}

impl SubresourceResponseStarted {
    pub fn new(
        handle: SubresourceNetworkRequestHandle,
        redirect_chain: Vec<NavigationRedirect>,
        final_url: Url,
        status: u16,
        response_headers: Vec<(String, String)>,
        cookie_set_reports: Vec<StoredCookieSetReport>,
    ) -> Self {
        Self {
            handle,
            redirect_chain,
            final_url,
            status,
            status_text: None,
            response_headers,
            cookie_set_reports,
            from_cache: false,
            network_request_headers: None,
            negotiated_http_version: None,
        }
    }

    pub fn with_status_text(mut self, status_text: Option<String>) -> Self {
        self.status_text = status_text;
        self
    }

    pub fn with_from_cache(mut self, from_cache: bool) -> Self {
        self.from_cache = from_cache;
        self
    }

    pub fn with_network_request_headers(
        mut self,
        network_request_headers: Option<Vec<(String, String)>>,
    ) -> Self {
        self.network_request_headers = network_request_headers;
        self
    }

    pub fn with_negotiated_http_version(
        mut self,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
    ) -> Self {
        self.negotiated_http_version = negotiated_http_version;
        self
    }

    pub fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.handle
    }

    pub fn redirect_chain(&self) -> &[NavigationRedirect] {
        &self.redirect_chain
    }

    pub fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn status_text(&self) -> Option<&str> {
        self.status_text.as_deref()
    }

    pub fn response_headers(&self) -> &[(String, String)] {
        &self.response_headers
    }

    pub fn cookie_set_reports(&self) -> &[StoredCookieSetReport] {
        &self.cookie_set_reports
    }

    pub fn from_cache(&self) -> bool {
        self.from_cache
    }

    pub fn network_request_headers(&self) -> Option<&[(String, String)]> {
        self.network_request_headers.as_deref()
    }

    pub fn negotiated_http_version(&self) -> Option<NegotiatedHttpVersion> {
        self.negotiated_http_version
    }
}

impl SubresourceDataReceived {
    pub fn new(
        handle: SubresourceNetworkRequestHandle,
        data_length: usize,
        encoded_data_length: usize,
    ) -> Self {
        Self {
            handle,
            data_length,
            encoded_data_length,
        }
    }

    pub fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.handle
    }

    pub fn data_length(&self) -> usize {
        self.data_length
    }

    pub fn encoded_data_length(&self) -> usize {
        self.encoded_data_length
    }
}

impl SubresourceEventSourceMessageReceived {
    pub fn new(
        handle: SubresourceNetworkRequestHandle,
        event_name: String,
        event_id: String,
        data: String,
    ) -> Self {
        Self {
            handle,
            event_name,
            event_id,
            data,
        }
    }

    pub fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.handle
    }

    pub fn event_name(&self) -> &str {
        &self.event_name
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub fn data(&self) -> &str {
        &self.data
    }
}

impl SubresourceBodyFinished {
    pub fn ready(handle: SubresourceNetworkRequestHandle, body: SubresourceResponseBody) -> Self {
        Self {
            handle,
            result: SubresourceBodyFinishedResult::Ready(body),
            data_was_streamed: false,
        }
    }

    pub fn ready_after_streaming(
        handle: SubresourceNetworkRequestHandle,
        body: SubresourceResponseBody,
    ) -> Self {
        Self {
            handle,
            result: SubresourceBodyFinishedResult::Ready(body),
            data_was_streamed: true,
        }
    }

    pub fn failed(handle: SubresourceNetworkRequestHandle, error_text: String) -> Self {
        Self {
            handle,
            result: SubresourceBodyFinishedResult::Failed(error_text),
            data_was_streamed: false,
        }
    }

    pub fn failed_with_partial_body(
        handle: SubresourceNetworkRequestHandle,
        error_text: String,
        partial_body: SubresourceResponseBody,
    ) -> Self {
        Self {
            handle,
            result: SubresourceBodyFinishedResult::FailedWithPartialBody {
                error_text,
                partial_body,
            },
            data_was_streamed: false,
        }
    }

    pub fn handle(&self) -> SubresourceNetworkRequestHandle {
        self.handle
    }

    pub fn result(&self) -> &SubresourceBodyFinishedResult {
        &self.result
    }

    pub fn data_was_streamed(&self) -> bool {
        self.data_was_streamed
    }
}

#[derive(Debug, Clone)]
pub struct SubresourceResponseBody {
    inner: Arc<SubresourceResponseBodyInner>,
}

#[derive(Debug)]
enum SubresourceResponseBodyInner {
    Memory {
        text: String,
        bytes: Vec<u8>,
    },
    File {
        path: PathBuf,
        len: usize,
        text_cache: Mutex<Option<String>>,
    },
}

impl Drop for SubresourceResponseBodyInner {
    fn drop(&mut self) {
        if let Self::File { path, .. } = self {
            let _ = fs::remove_file(path);
        }
    }
}

impl PartialEq for SubresourceResponseBody {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            return true;
        }
        match (self.inner.as_ref(), other.inner.as_ref()) {
            (
                SubresourceResponseBodyInner::Memory {
                    text: left_text,
                    bytes: left_bytes,
                },
                SubresourceResponseBodyInner::Memory {
                    text: right_text,
                    bytes: right_bytes,
                },
            ) => left_text == right_text && left_bytes == right_bytes,
            _ => false,
        }
    }
}

impl Eq for SubresourceResponseBody {}

#[derive(Debug)]
pub struct SubresourceResponseBodyWriter {
    memory_limit: usize,
    len: usize,
    memory: Vec<u8>,
    file: Option<File>,
    path: Option<PathBuf>,
    spool_failed: bool,
}

impl Default for SubresourceResponseBodyWriter {
    fn default() -> Self {
        Self::new(SUBRESOURCE_RESPONSE_BODY_MEMORY_LIMIT)
    }
}

impl SubresourceResponseBodyWriter {
    pub fn new(memory_limit: usize) -> Self {
        Self {
            memory_limit,
            len: 0,
            memory: Vec::new(),
            file: None,
            path: None,
            spool_failed: false,
        }
    }

    pub fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if self.file.is_some() {
            if let Some(file) = self.file.as_mut()
                && file.write_all(bytes).is_ok()
            {
                self.len = self.len.saturating_add(bytes.len());
                return;
            }
            self.abandon_file_to_memory();
            self.spool_failed = true;
        }
        if self.spool_failed || self.len.saturating_add(bytes.len()) <= self.memory_limit {
            self.memory.extend_from_slice(bytes);
            self.len = self.len.saturating_add(bytes.len());
            return;
        }
        if self.ensure_file().is_ok()
            && let Some(file) = self.file.as_mut()
            && file.write_all(bytes).is_ok()
        {
            self.len = self.len.saturating_add(bytes.len());
            return;
        }
        // Spooling is an optimization for CDP bookkeeping. If private temp-file
        // creation fails, keep the protocol-correct body in memory rather than
        // dropping captured bytes.
        self.spool_failed = true;
        self.abandon_file_to_memory();
        self.memory.extend_from_slice(bytes);
        self.len = self.len.saturating_add(bytes.len());
    }

    pub fn finish(mut self) -> SubresourceResponseBody {
        if let Some(mut file) = self.file.take() {
            if file.flush().is_ok() {
                let path = self
                    .path
                    .take()
                    .expect("subresource body spool path should be set");
                return SubresourceResponseBody {
                    inner: Arc::new(SubresourceResponseBodyInner::File {
                        path,
                        len: self.len,
                        text_cache: Mutex::new(None),
                    }),
                };
            }
            self.file = Some(file);
            self.abandon_file_to_memory();
        }
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
        SubresourceResponseBody::from_text_and_bytes(
            String::from_utf8_lossy(&self.memory).into_owned(),
            std::mem::take(&mut self.memory),
        )
    }

    fn ensure_file(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let path = unique_subresource_response_body_spool_path()?;
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        configure_secure_subresource_response_body_spool_file_options(&mut options);
        let mut file = options.open(&path)?;
        if !self.memory.is_empty() {
            if let Err(error) = file.write_all(&self.memory) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(error);
            }
            self.memory.clear();
        }
        self.path = Some(path);
        self.file = Some(file);
        Ok(())
    }

    fn abandon_file_to_memory(&mut self) {
        let _ = self.file.take();
        if let Some(path) = self.path.take() {
            if let Ok(mut bytes) = fs::read(&path) {
                bytes.extend_from_slice(&self.memory);
                self.memory = bytes;
            }
            let _ = fs::remove_file(path);
        }
        self.len = self.memory.len();
    }

    #[cfg(feature = "test-support")]
    pub fn replace_spool_path_for_test(&mut self, replacement: PathBuf) -> Option<PathBuf> {
        self.path.replace(replacement)
    }
}

impl Drop for SubresourceResponseBodyWriter {
    fn drop(&mut self) {
        let _ = self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl SubresourceResponseBody {
    pub fn from_text(text: String) -> Self {
        let bytes = text.as_bytes().to_vec();
        Self::from_text_and_bytes(text, bytes)
    }

    pub fn from_text_and_bytes(text: String, bytes: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(SubresourceResponseBodyInner::Memory { text, bytes }),
        }
    }

    pub fn from_materialized_body(body: ResponseBody) -> Self {
        let (text, bytes) = body
            .try_into_lossy_materialized_text()
            .expect("SubresourceResponseBody should be built from a materialized body");
        Self::from_text_and_bytes(text, bytes)
    }

    /// Builds the neutral subresource body carrier from a materialized fetch
    /// response without exposing a loose `(String, Vec<u8>)` pair to callers.
    pub fn from_fetch_response(response: &Response) -> Self {
        Self::from_materialized_body(response.materialized_body())
    }

    /// Builds the neutral subresource body carrier from a materialized
    /// navigation response at the explicit compatibility boundary.
    pub fn from_navigation_response(response: &NavigationResponse) -> Self {
        Self::from_materialized_body(response.materialized_body())
    }

    /// Builds a materialized navigation response at a compatibility boundary
    /// that still needs both the lossy text view and exact bytes.
    pub fn to_navigation_response(&self, head: ResponseHead) -> NavigationResponse {
        self.diagnostic_navigation_response(head)
    }

    /// Best-effort navigation response for diagnostics and legacy tests.
    /// Production protocol paths should use `try_to_navigation_response`.
    pub fn diagnostic_navigation_response(&self, head: ResponseHead) -> NavigationResponse {
        NavigationResponse::from_head_and_materialized_body(
            head,
            self.diagnostic_materialized_body(),
        )
    }

    /// Fallible variant of `to_navigation_response` for callers that can
    /// surface file-backed body source errors instead of treating them as an
    /// empty body.
    pub fn try_to_navigation_response(&self, head: ResponseHead) -> io::Result<NavigationResponse> {
        self.try_materialized_body()
            .map(|body| NavigationResponse::from_head_and_materialized_body(head, body))
    }

    /// Builds a materialized response body at an explicit compatibility
    /// boundary. File-backed bodies are read once so callers that need both the
    /// text view and exact bytes do not duplicate spool I/O.
    pub fn materialized_body(&self) -> ResponseBody {
        self.diagnostic_materialized_body()
    }

    /// Best-effort materialized body for diagnostics and legacy tests.
    /// Production protocol paths should use `try_materialized_body`.
    pub fn diagnostic_materialized_body(&self) -> ResponseBody {
        self.try_materialized_body()
            .unwrap_or_else(|_| ResponseBody::materialized_text(String::new(), Vec::new()))
    }

    /// Fallible materialization for production paths that need to distinguish
    /// source read failure from a legitimate empty body.
    pub fn try_materialized_body(&self) -> io::Result<ResponseBody> {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { text, bytes } => {
                Ok(ResponseBody::materialized_text(text.clone(), bytes.clone()))
            }
            SubresourceResponseBodyInner::File { text_cache, .. } => {
                let bytes = self.materialize_bytes()?;
                let mut cache = text_cache.lock();
                let text = cache
                    .get_or_insert_with(|| String::from_utf8_lossy(&bytes).into_owned())
                    .clone();
                Ok(ResponseBody::materialized_text(text, bytes))
            }
        }
    }

    pub fn text(&self) -> Cow<'_, str> {
        self.diagnostic_text()
    }

    /// Best-effort text view for diagnostics and legacy tests. Production
    /// protocol paths should use `try_text` so source read failures remain
    /// visible instead of becoming an empty string.
    pub fn diagnostic_text(&self) -> Cow<'_, str> {
        self.try_text()
            .unwrap_or_else(|_| Cow::Owned(String::new()))
    }

    pub fn try_text(&self) -> io::Result<Cow<'_, str>> {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { text, .. } => Ok(Cow::Borrowed(text)),
            SubresourceResponseBodyInner::File { text_cache, .. } => {
                let mut cache = text_cache.lock();
                if cache.is_none() {
                    let bytes = self.materialize_bytes()?;
                    *cache = Some(String::from_utf8_lossy(&bytes).into_owned());
                }
                Ok(Cow::Owned(cache.as_deref().unwrap_or_default().to_owned()))
            }
        }
    }

    pub fn bytes(&self) -> Cow<'_, [u8]> {
        self.diagnostic_bytes()
    }

    /// Best-effort byte view for diagnostics and legacy tests. Production
    /// protocol paths should use `try_bytes`.
    pub fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        self.try_bytes().unwrap_or_else(|_| Cow::Owned(Vec::new()))
    }

    pub fn try_bytes(&self) -> io::Result<Cow<'_, [u8]>> {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { bytes, .. } => Ok(Cow::Borrowed(bytes)),
            SubresourceResponseBodyInner::File { .. } => self.materialize_bytes().map(Cow::Owned),
        }
    }

    /// Explicit byte-level equality for callers that intentionally accept
    /// source I/O while comparing bodies.
    pub fn try_byte_eq(&self, other: &Self) -> io::Result<bool> {
        if self.len() != other.len() {
            return Ok(false);
        }
        let left = self.try_bytes()?;
        let right = other.try_bytes()?;
        Ok(left == right)
    }

    /// Best-effort byte-level equality for diagnostics and legacy tests.
    pub fn diagnostic_byte_eq(&self, other: &Self) -> bool {
        self.try_byte_eq(other).unwrap_or(false)
    }

    pub fn clone_body_bytes(&self) -> Vec<u8> {
        self.diagnostic_clone_body_bytes()
    }

    /// Best-effort owned byte clone for diagnostics and legacy tests.
    /// Production protocol paths should use `materialize_bytes`.
    pub fn diagnostic_clone_body_bytes(&self) -> Vec<u8> {
        self.materialize_bytes().unwrap_or_default()
    }

    pub fn clone_body_bytes_from(&self, offset: usize) -> Vec<u8> {
        self.diagnostic_clone_body_bytes_from(offset)
    }

    /// Best-effort owned byte clone from an offset for diagnostics and legacy
    /// tests. Production protocol paths should use `materialize_bytes_from`.
    pub fn diagnostic_clone_body_bytes_from(&self, offset: usize) -> Vec<u8> {
        self.materialize_bytes_from(offset).unwrap_or_default()
    }

    pub fn materialize_bytes(&self) -> io::Result<Vec<u8>> {
        self.materialize_bytes_from(0)
    }

    pub fn materialize_bytes_from(&self, offset: usize) -> io::Result<Vec<u8>> {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { bytes, .. } => {
                Ok(bytes.get(offset..).map(<[u8]>::to_vec).unwrap_or_default())
            }
            SubresourceResponseBodyInner::File { path, len, .. } => {
                if offset >= *len {
                    return Ok(Vec::new());
                }
                let mut file = File::open(path)?;
                file.seek(io::SeekFrom::Start(offset as u64))?;
                let mut bytes = Vec::with_capacity(len.saturating_sub(offset));
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            }
        }
    }

    pub fn read_chunk(&self, offset: usize, max_len: usize) -> io::Result<Vec<u8>> {
        if max_len == 0 {
            return Ok(Vec::new());
        }
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { bytes, .. } => {
                let Some(remaining) = bytes.get(offset..) else {
                    return Ok(Vec::new());
                };
                let len = remaining.len().min(max_len);
                Ok(remaining[..len].to_vec())
            }
            SubresourceResponseBodyInner::File { path, len, .. } => {
                if offset >= *len {
                    return Ok(Vec::new());
                }
                let mut file = File::open(path)?;
                file.seek(io::SeekFrom::Start(offset as u64))?;
                let mut chunk = vec![0; max_len.min(len.saturating_sub(offset))];
                let read = file.read(&mut chunk)?;
                chunk.truncate(read);
                Ok(chunk)
            }
        }
    }

    pub fn write_bytes_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { bytes, .. } => writer.write_all(bytes),
            SubresourceResponseBodyInner::File { path, .. } => {
                let mut file = File::open(path)?;
                let mut buffer = [0; 64 * 1024];
                loop {
                    let read = file.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    writer.write_all(&buffer[..read])?;
                }
                Ok(())
            }
        }
    }

    pub fn len(&self) -> usize {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory { bytes, .. } => bytes.len(),
            SubresourceResponseBodyInner::File { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn unique_subresource_response_body_spool_path() -> io::Result<PathBuf> {
    let root = std::env::temp_dir().join("moli-subresource-body-spool");
    create_secure_subresource_response_body_spool_root(&root)?;
    let id = NEXT_SUBRESOURCE_RESPONSE_BODY_SPOOL_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(root.join(format!(
        "subresource-body-{}-{id}-{nanos}.bin",
        std::process::id()
    )))
}

#[cfg(unix)]
fn create_secure_subresource_response_body_spool_root(root: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_secure_subresource_response_body_spool_root(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)
}

#[cfg(unix)]
fn configure_secure_subresource_response_body_spool_file_options(options: &mut OpenOptions) {
    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_secure_subresource_response_body_spool_file_options(_options: &mut OpenOptions) {}

#[derive(Debug, Clone, Default)]
pub struct SubresourceResponseWaitCriteria {
    pub url_contains: Option<String>,
    pub url_regex: Option<Regex>,
    pub body_contains: Option<String>,
    pub body_regex: Option<Regex>,
    pub json_path_equals: Option<SubresourceJsonPathEquals>,
    pub json_path_regex: Option<SubresourceJsonPathRegex>,
}

impl SubresourceResponseWaitCriteria {
    pub fn is_empty(&self) -> bool {
        self.url_contains.is_none() && self.url_regex.is_none() && !self.requires_response_body()
    }

    /// Compatibility matcher for diagnostic surfaces that prefer best-effort
    /// matching over surfacing a response-body source read error.
    pub fn matches(&self, record: &SubresourceNetworkRecord) -> bool {
        self.diagnostic_matches(record)
    }

    /// Best-effort matcher for diagnostics such as trace summaries. Unreadable
    /// response bodies are treated as no match so diagnostics can still render.
    pub fn diagnostic_matches(&self, record: &SubresourceNetworkRecord) -> bool {
        self.try_matches(record).unwrap_or(false)
    }

    /// Fallible matcher for production wait paths. A body-backed criterion
    /// should not silently treat an unreadable response body as empty text.
    pub fn try_matches(&self, record: &SubresourceNetworkRecord) -> io::Result<bool> {
        if let Some(needle) = self.url_contains.as_deref()
            && !record.url().as_str().contains(needle)
            && !matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Success { final_url, .. }
                    if final_url.as_str().contains(needle)
            )
        {
            return Ok(false);
        }

        if let Some(regex) = self.url_regex.as_ref()
            && !regex.is_match(record.url().as_str())
            && !matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Success { final_url, .. }
                    if regex.is_match(final_url.as_str())
            )
        {
            return Ok(false);
        }

        let SubresourceNetworkOutcome::Success {
            response_headers,
            response_body,
            ..
        } = record.outcome()
        else {
            return Ok(!self.requires_response_body());
        };

        let response_body_text = if self.requires_response_body() {
            Some(response_body.try_text()?)
        } else {
            None
        };

        if let Some(needle) = self.body_contains.as_deref()
            && !response_body_text
                .as_deref()
                .unwrap_or_default()
                .contains(needle)
        {
            return Ok(false);
        }

        if let Some(regex) = self.body_regex.as_ref()
            && !regex.is_match(response_body_text.as_deref().unwrap_or_default())
        {
            return Ok(false);
        }

        if let Some(expectation) = self.json_path_equals.as_ref()
            && !json_path_equals(
                response_headers,
                response_body_text.as_deref().unwrap_or_default(),
                expectation,
            )
        {
            return Ok(false);
        }

        if let Some(expectation) = self.json_path_regex.as_ref()
            && !json_path_matches_regex(
                response_headers,
                response_body_text.as_deref().unwrap_or_default(),
                expectation,
            )
        {
            return Ok(false);
        }

        Ok(true)
    }

    fn requires_response_body(&self) -> bool {
        self.body_contains.is_some()
            || self.body_regex.is_some()
            || self.json_path_equals.is_some()
            || self.json_path_regex.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceJsonPathEquals {
    pub path: Vec<String>,
    pub expected: String,
}

#[derive(Debug, Clone)]
pub struct SubresourceJsonPathRegex {
    pub path: Vec<String>,
    pub regex: Regex,
}

fn json_path_equals(
    headers: &[(String, String)],
    body: &str,
    expectation: &SubresourceJsonPathEquals,
) -> bool {
    json_path_satisfies(headers, body, &expectation.path, |value| {
        json_value_equals_string(value, &expectation.expected)
    })
}

fn json_path_matches_regex(
    headers: &[(String, String)],
    body: &str,
    expectation: &SubresourceJsonPathRegex,
) -> bool {
    json_path_satisfies(headers, body, &expectation.path, |value| {
        json_value_matches_regex(value, &expectation.regex)
    })
}

fn json_path_satisfies(
    headers: &[(String, String)],
    body: &str,
    path: &[String],
    predicate: impl FnOnce(&Value) -> bool,
) -> bool {
    let Some(content_type) = header_value(headers, "content-type") else {
        return false;
    };
    if !is_json_module_mime(content_type) {
        return false;
    }

    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let Some(value) = json_path_value(&parsed, path) else {
        return false;
    };
    predicate(value)
}

fn json_path_value<'a>(mut value: &'a Value, path: &[String]) -> Option<&'a Value> {
    for segment in path {
        value = match value {
            Value::Object(object) => object.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(value)
}

fn json_value_equals_string(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(actual) => actual == expected,
        Value::Bool(actual) => actual.to_string() == expected,
        Value::Number(actual) => actual.to_string() == expected,
        Value::Null => expected == "null",
        Value::Array(_) | Value::Object(_) => *value == expected,
    }
}

fn json_value_matches_regex(value: &Value, regex: &Regex) -> bool {
    match value {
        Value::String(actual) => regex.is_match(actual),
        Value::Bool(actual) => regex.is_match(if *actual { "true" } else { "false" }),
        Value::Number(actual) => regex.is_match(&actual.to_string()),
        Value::Null => regex.is_match("null"),
        Value::Array(_) | Value::Object(_) => false,
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

impl SubresourceNetworkRecord {
    fn from_staged_lifecycle(
        request: &SubresourceRequestStarted,
        response: Option<&SubresourceResponseStarted>,
        body: &SubresourceBodyFinished,
    ) -> Option<Self> {
        if request.handle() != body.handle()
            || response.is_some_and(|response| response.handle() != request.handle())
        {
            return None;
        }

        match body.result() {
            SubresourceBodyFinishedResult::Ready(response_body) => {
                let response = response?;
                let mut record = Self::success_with_body(
                    request.frame_id.clone(),
                    request.document_url.clone(),
                    request.url.clone(),
                    request.method.clone(),
                    request.request_headers.clone(),
                    request.request_body.clone(),
                    request.resource_type,
                    request.request_cookie_report.clone(),
                    response.redirect_chain.clone(),
                    response.final_url.clone(),
                    response.status,
                    response.response_headers.clone(),
                    response_body.clone(),
                    response.cookie_set_reports.clone(),
                )
                .with_request_handle(request.handle())
                .with_request_body_bytes(request.request_body_bytes.clone())
                .with_request_initiator_type(request.request_initiator_type)
                .with_from_cache(response.from_cache)
                .with_network_request_headers(response.network_request_headers.clone())
                .with_negotiated_http_version(response.negotiated_http_version);
                if let Some(status_text) = response.status_text.clone() {
                    record = record.with_response_status_text(status_text);
                }
                Some(record)
            }
            SubresourceBodyFinishedResult::Failed(error_text)
            | SubresourceBodyFinishedResult::FailedWithPartialBody { error_text, .. } => {
                Some(Self {
                    frame_id: request.frame_id.clone(),
                    document_url: request.document_url.clone(),
                    url: request.url.clone(),
                    request_handle: Some(request.handle()),
                    websocket_socket_id: None,
                    method: request.method.clone(),
                    request_headers: request.request_headers.clone(),
                    request_body: request.request_body.clone(),
                    request_body_bytes: request.request_body_bytes.clone(),
                    resource_type: request.resource_type,
                    request_initiator_type: request.request_initiator_type,
                    request_cookie_report: request.request_cookie_report.clone(),
                    outcome: SubresourceNetworkOutcome::Failure {
                        error_text: error_text.clone(),
                    },
                    cookie_set_reports: response
                        .map(|response| response.cookie_set_reports.clone())
                        .unwrap_or_default(),
                    from_cache: response.is_some_and(|response| response.from_cache),
                    network_request_headers: response
                        .and_then(|response| response.network_request_headers.clone()),
                    negotiated_http_version: response
                        .and_then(|response| response.negotiated_http_version),
                })
            }
        }
    }

    pub fn success(
        frame_id: Option<String>,
        document_url: Url,
        url: Url,
        method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        resource_type: SubresourceResourceType,
        request_cookie_report: Option<StoredCookieQueryReport>,
        redirect_chain: Vec<NavigationRedirect>,
        final_url: Url,
        status: u16,
        response_headers: Vec<(String, String)>,
        response_body: String,
        cookie_set_reports: Vec<StoredCookieSetReport>,
    ) -> Self {
        Self::success_with_body(
            frame_id,
            document_url,
            url,
            method,
            request_headers,
            request_body,
            resource_type,
            request_cookie_report,
            redirect_chain,
            final_url,
            status,
            response_headers,
            SubresourceResponseBody::from_text(response_body),
            cookie_set_reports,
        )
    }

    pub fn success_with_body(
        frame_id: Option<String>,
        document_url: Url,
        url: Url,
        method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        resource_type: SubresourceResourceType,
        request_cookie_report: Option<StoredCookieQueryReport>,
        redirect_chain: Vec<NavigationRedirect>,
        final_url: Url,
        status: u16,
        response_headers: Vec<(String, String)>,
        response_body: SubresourceResponseBody,
        cookie_set_reports: Vec<StoredCookieSetReport>,
    ) -> Self {
        let request_body_bytes = request_body_text_bytes(&request_body);
        Self {
            frame_id,
            document_url,
            url,
            request_handle: None,
            websocket_socket_id: None,
            method,
            request_headers,
            request_body,
            request_body_bytes,
            resource_type,
            request_initiator_type: SubresourceRequestInitiatorType::Script,
            request_cookie_report,
            outcome: SubresourceNetworkOutcome::Success {
                redirect_chain,
                final_url,
                status,
                status_text: None,
                response_headers,
                response_body,
            },
            cookie_set_reports,
            from_cache: false,
            network_request_headers: None,
            negotiated_http_version: None,
        }
    }

    pub fn failure(
        frame_id: Option<String>,
        document_url: Url,
        url: Url,
        method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        resource_type: SubresourceResourceType,
        error_text: String,
    ) -> Self {
        let request_body_bytes = request_body_text_bytes(&request_body);
        Self {
            frame_id,
            document_url,
            url,
            request_handle: None,
            websocket_socket_id: None,
            method,
            request_headers,
            request_body,
            request_body_bytes,
            resource_type,
            request_initiator_type: SubresourceRequestInitiatorType::Script,
            request_cookie_report: None,
            outcome: SubresourceNetworkOutcome::Failure { error_text },
            cookie_set_reports: Vec::new(),
            from_cache: false,
            network_request_headers: None,
            negotiated_http_version: None,
        }
    }

    pub fn with_websocket_socket_id(mut self, socket_id: u64) -> Self {
        self.websocket_socket_id = Some(socket_id);
        self
    }

    pub fn with_request_handle(mut self, handle: SubresourceNetworkRequestHandle) -> Self {
        self.request_handle = Some(handle);
        self
    }

    pub fn with_request_body_bytes(mut self, request_body_bytes: Option<Vec<u8>>) -> Self {
        self.request_body_bytes =
            request_body_bytes.or_else(|| request_body_text_bytes(&self.request_body));
        self
    }

    pub fn with_request_initiator_type(
        mut self,
        request_initiator_type: SubresourceRequestInitiatorType,
    ) -> Self {
        self.request_initiator_type = request_initiator_type;
        self
    }

    pub fn with_response_status_text(mut self, status_text: impl Into<String>) -> Self {
        if let SubresourceNetworkOutcome::Success {
            status_text: current,
            ..
        } = &mut self.outcome
        {
            *current = Some(status_text.into());
        }
        self
    }

    pub fn with_from_cache(mut self, from_cache: bool) -> Self {
        self.from_cache = from_cache;
        self
    }

    pub fn with_network_request_headers(
        mut self,
        network_request_headers: Option<Vec<(String, String)>>,
    ) -> Self {
        self.network_request_headers = network_request_headers;
        self
    }

    pub fn with_negotiated_http_version(
        mut self,
        negotiated_http_version: Option<NegotiatedHttpVersion>,
    ) -> Self {
        self.negotiated_http_version = negotiated_http_version;
        self
    }

    pub fn request_handle(&self) -> Option<SubresourceNetworkRequestHandle> {
        self.request_handle
    }

    pub fn websocket_socket_id(&self) -> Option<u64> {
        self.websocket_socket_id
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn frame_id(&self) -> Option<&str> {
        self.frame_id.as_deref()
    }

    pub fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    pub fn request_body(&self) -> Option<&str> {
        self.request_body.as_deref()
    }

    pub fn request_body_bytes(&self) -> Option<&[u8]> {
        self.request_body_bytes.as_deref()
    }

    pub fn resource_type(&self) -> SubresourceResourceType {
        self.resource_type
    }

    pub fn request_initiator_type(&self) -> SubresourceRequestInitiatorType {
        self.request_initiator_type
    }

    pub fn request_cookie_report(&self) -> Option<&StoredCookieQueryReport> {
        self.request_cookie_report.as_ref()
    }

    pub fn outcome(&self) -> &SubresourceNetworkOutcome {
        &self.outcome
    }

    pub fn try_response_body_byte_eq(&self, other: &Self) -> io::Result<bool> {
        match (&self.outcome, &other.outcome) {
            (
                SubresourceNetworkOutcome::Success {
                    response_body: left,
                    ..
                },
                SubresourceNetworkOutcome::Success {
                    response_body: right,
                    ..
                },
            ) => left.try_byte_eq(right),
            _ => Ok(false),
        }
    }

    pub fn diagnostic_response_body_byte_eq(&self, other: &Self) -> bool {
        self.try_response_body_byte_eq(other).unwrap_or(false)
    }

    pub fn cookie_set_reports(&self) -> &[StoredCookieSetReport] {
        &self.cookie_set_reports
    }

    pub fn from_cache(&self) -> bool {
        self.from_cache
    }

    pub fn network_request_headers(&self) -> Option<&[(String, String)]> {
        self.network_request_headers.as_deref()
    }

    pub fn negotiated_http_version(&self) -> Option<NegotiatedHttpVersion> {
        self.negotiated_http_version
    }
}

fn request_body_text_bytes(request_body: &Option<String>) -> Option<Vec<u8>> {
    request_body.as_ref().map(|body| body.as_bytes().to_vec())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceFetchInfo {
    pub internal_id: u64,
    pub network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub frame_id: Option<String>,
    pub document_url: Url,
    pub url: Url,
    pub websocket_socket_id: Option<u64>,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub request_body_bytes: Option<Vec<u8>>,
    pub resource_type: SubresourceResourceType,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceResponseInfo {
    pub internal_id: u64,
    pub url: Url,
    pub final_url: Url,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub resource_type: SubresourceResourceType,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub network_request_headers: Option<Vec<(String, String)>>,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    /// Exact response bytes plus the lossy compatibility text view needed while
    /// a response-stage Fetch pause is held.
    pub response_body: SubresourceResponseBody,
    pub from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubresourceAuthInfo {
    pub internal_id: u64,
    pub url: Url,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub resource_type: SubresourceResourceType,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub network_request_headers: Option<Vec<(String, String)>>,
    pub challenge: SubresourceAuthChallenge,
    pub intercept_response: bool,
    pub response_final_url: Url,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    pub response_body: SubresourceResponseBody,
    pub response_from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceAuthChallenge {
    pub source: String,
    pub scheme: String,
    pub realm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubresourceAuthTarget {
    Server,
    Proxy,
    ProxyHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubresourceAuthScheme {
    Basic,
    Digest,
    Negotiate,
    Ntlm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubresourceAuthCredentials {
    pub target: SubresourceAuthTarget,
    pub scheme: SubresourceAuthScheme,
    pub username: String,
    pub password: String,
}

pub fn extract_subresource_auth_challenge(
    headers: &[(String, String)],
) -> Option<SubresourceAuthChallenge> {
    let mut first_challenge = None;
    for (source, value) in headers.iter().filter_map(|(name, value)| {
        if name.eq_ignore_ascii_case("www-authenticate") {
            Some(("Server", value.as_str()))
        } else if name.eq_ignore_ascii_case("proxy-authenticate") {
            Some(("Proxy", value.as_str()))
        } else {
            None
        }
    }) {
        for candidate in parse_auth_challenge_candidates(value) {
            let challenge = SubresourceAuthChallenge {
                source: source.to_owned(),
                scheme: candidate.scheme,
                realm: candidate.realm,
            };
            if first_challenge.is_none() {
                first_challenge = Some(challenge.clone());
            }
            if subresource_auth_scheme_from_name(&challenge.scheme).is_some() {
                return Some(challenge);
            }
        }
    }
    first_challenge
}

pub fn subresource_auth_credentials_for_challenge(
    challenge: &SubresourceAuthChallenge,
    username: &str,
    password: &str,
) -> Option<SubresourceAuthCredentials> {
    let target = if challenge.source.eq_ignore_ascii_case("proxy") {
        if challenge.scheme.is_empty() || challenge.scheme.eq_ignore_ascii_case("basic") {
            SubresourceAuthTarget::ProxyHeader
        } else {
            SubresourceAuthTarget::Proxy
        }
    } else {
        SubresourceAuthTarget::Server
    };
    let scheme = subresource_auth_scheme_from_name(&challenge.scheme)?;
    Some(SubresourceAuthCredentials {
        target,
        scheme,
        username: username.to_owned(),
        password: password.to_owned(),
    })
}

fn subresource_auth_scheme_from_name(name: &str) -> Option<SubresourceAuthScheme> {
    match name.to_ascii_lowercase().as_str() {
        "" | "basic" => Some(SubresourceAuthScheme::Basic),
        "digest" => Some(SubresourceAuthScheme::Digest),
        "negotiate" => Some(SubresourceAuthScheme::Negotiate),
        "ntlm" => Some(SubresourceAuthScheme::Ntlm),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ParsedAuthChallenge {
    scheme: String,
    realm: String,
}

fn parse_auth_challenge_candidates(value: &str) -> Vec<ParsedAuthChallenge> {
    ChallengeParser::new(value)
        .filter_map(Result::ok)
        .map(|challenge| ParsedAuthChallenge {
            scheme: challenge.scheme.to_ascii_lowercase(),
            realm: challenge
                .params
                .iter()
                .find_map(|(name, value)| {
                    name.eq_ignore_ascii_case("realm")
                        .then(|| value.to_unescaped())
                })
                .unwrap_or_default(),
        })
        .collect()
}

impl From<SubresourceAuthCredentials> for RequestAuth {
    fn from(value: SubresourceAuthCredentials) -> Self {
        Self {
            target: match value.target {
                SubresourceAuthTarget::Server => RequestAuthTarget::Server,
                SubresourceAuthTarget::Proxy => RequestAuthTarget::Proxy,
                SubresourceAuthTarget::ProxyHeader => RequestAuthTarget::ProxyHeader,
            },
            scheme: match value.scheme {
                SubresourceAuthScheme::Basic => RequestAuthScheme::Basic,
                SubresourceAuthScheme::Digest => RequestAuthScheme::Digest,
                SubresourceAuthScheme::Negotiate => RequestAuthScheme::Negotiate,
                SubresourceAuthScheme::Ntlm => RequestAuthScheme::Ntlm,
            },
            username: value.username,
            password: value.password,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingSubresourceContinueOutcome {
    Started,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingSubresourceContinueEvent {
    Completed { internal_id: u64 },
    ResponsePaused(PendingSubresourceResponseInfo),
    AuthRequired(PendingSubresourceAuthInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubresourceResourceType {
    Script,
    Stylesheet,
    Image,
    Font,
    Audio,
    Video,
    Media,
    TextTrack,
    Fetch,
    EventSource,
    Xhr,
    Ping,
    CspReport,
    Dictionary,
    Manifest,
    WebSocket,
}

bitflags::bitflags! {
    /// Browser-level opt-ins for resource families that the lightweight
    /// default does not fetch from the network.
    ///
    /// This mask deliberately contains only optional render/media resources.
    /// Script, stylesheet, Fetch/XHR, and other behaviorally required requests
    /// are outside this policy and remain enabled.
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
    pub struct OptionalResourceFetchMask: u8 {
        const NONE = 0;
        const IMAGE = 1 << 0;
        const FONT = 1 << 1;
        const AUDIO = 1 << 2;
        const VIDEO = 1 << 3;
        const MEDIA = 1 << 4;
        const TEXT_TRACK = 1 << 5;
        const ALL = Self::IMAGE.bits()
            | Self::FONT.bits()
            | Self::AUDIO.bits()
            | Self::VIDEO.bits()
            | Self::MEDIA.bits()
            | Self::TEXT_TRACK.bits();
    }
}

impl OptionalResourceFetchMask {
    pub const fn for_resource_type(resource_type: SubresourceResourceType) -> Option<Self> {
        match resource_type {
            SubresourceResourceType::Image => Some(Self::IMAGE),
            SubresourceResourceType::Font => Some(Self::FONT),
            SubresourceResourceType::Audio => Some(Self::AUDIO),
            SubresourceResourceType::Video => Some(Self::VIDEO),
            SubresourceResourceType::Media => Some(Self::MEDIA),
            SubresourceResourceType::TextTrack => Some(Self::TEXT_TRACK),
            SubresourceResourceType::Script
            | SubresourceResourceType::Stylesheet
            | SubresourceResourceType::Fetch
            | SubresourceResourceType::EventSource
            | SubresourceResourceType::Xhr
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::Dictionary
            | SubresourceResourceType::Manifest
            | SubresourceResourceType::WebSocket => None,
        }
    }

    pub const fn allows(self, resource_type: SubresourceResourceType) -> bool {
        match Self::for_resource_type(resource_type) {
            Some(resource) => self.contains(resource),
            None => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubresourceRequestInitiatorType {
    Other,
    Parser,
    Script,
    Css,
}

impl SubresourceRequestInitiatorType {
    pub fn as_cdp_initiator_type(self) -> &'static str {
        match self {
            Self::Parser | Self::Css => "parser",
            Self::Script => "script",
            Self::Other => "other",
        }
    }

    pub fn as_bidi_request_initiator_type(self) -> Option<&'static str> {
        match self {
            Self::Script => Some("script"),
            Self::Css => Some("css"),
            Self::Other | Self::Parser => None,
        }
    }
}

impl SubresourceResourceType {
    pub fn as_cdp_type(self) -> &'static str {
        match self {
            Self::Script => "Script",
            Self::Stylesheet => "Stylesheet",
            Self::Image => "Image",
            Self::Font => "Font",
            Self::Audio | Self::Video => "Media",
            Self::Media => "Media",
            Self::TextTrack => "TextTrack",
            Self::Fetch => "Fetch",
            Self::EventSource => "EventSource",
            Self::Xhr => "XHR",
            Self::Ping => "Ping",
            Self::CspReport => "CSPViolationReport",
            Self::Dictionary => "Other",
            Self::Manifest => "Manifest",
            Self::WebSocket => "WebSocket",
        }
    }

    /// Returns the resource type exposed by the CDP Fetch interception domain.
    ///
    /// Chromium's loader interception receives Fetch, EventSource, and XHR as
    /// Blink's shared `kXhr` resource type. The Network domain still reports
    /// their higher-level types through `as_cdp_type`.
    pub fn as_cdp_fetch_interception_type(self) -> &'static str {
        match self {
            Self::Fetch | Self::EventSource | Self::Xhr => "XHR",
            _ => self.as_cdp_type(),
        }
    }

    pub fn has_same_cdp_fetch_interception_type(self, other: Self) -> bool {
        self.as_cdp_fetch_interception_type() == other.as_cdp_fetch_interception_type()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubresourceNetworkOutcome {
    Success {
        redirect_chain: Vec<NavigationRedirect>,
        final_url: Url,
        status: u16,
        status_text: Option<String>,
        response_headers: Vec<(String, String)>,
        response_body: SubresourceResponseBody,
    },
    Failure {
        error_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketNetworkEvent {
    socket_id: u64,
    document_url: Url,
    url: Url,
    direction: WebSocketFrameDirection,
    opcode: WebSocketFrameOpcode,
    payload_length: usize,
}

impl WebSocketNetworkEvent {
    pub fn new(
        socket_id: u64,
        document_url: Url,
        url: Url,
        direction: WebSocketFrameDirection,
        opcode: WebSocketFrameOpcode,
        payload_length: usize,
    ) -> Self {
        Self {
            socket_id,
            document_url,
            url,
            direction,
            opcode,
            payload_length,
        }
    }

    pub fn socket_id(&self) -> u64 {
        self.socket_id
    }

    pub fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn direction(&self) -> WebSocketFrameDirection {
        self.direction
    }

    pub fn opcode(&self) -> WebSocketFrameOpcode {
        self.opcode
    }

    pub fn payload_length(&self) -> usize {
        self.payload_length
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketLifecycleEvent {
    socket_id: u64,
    document_url: Url,
    url: Url,
    kind: WebSocketLifecycleKind,
    error_text: Option<String>,
    close_code: Option<u16>,
    close_reason: Option<String>,
    was_clean: Option<bool>,
}

impl WebSocketLifecycleEvent {
    pub fn open(socket_id: u64, document_url: Url, url: Url) -> Self {
        Self {
            socket_id,
            document_url,
            url,
            kind: WebSocketLifecycleKind::Open,
            error_text: None,
            close_code: None,
            close_reason: None,
            was_clean: None,
        }
    }

    pub fn error(socket_id: u64, document_url: Url, url: Url, error_text: String) -> Self {
        Self {
            socket_id,
            document_url,
            url,
            kind: WebSocketLifecycleKind::Error,
            error_text: Some(error_text),
            close_code: None,
            close_reason: None,
            was_clean: None,
        }
    }

    pub fn closing(socket_id: u64, document_url: Url, url: Url) -> Self {
        Self {
            socket_id,
            document_url,
            url,
            kind: WebSocketLifecycleKind::Closing,
            error_text: None,
            close_code: None,
            close_reason: None,
            was_clean: None,
        }
    }

    pub fn close(
        socket_id: u64,
        document_url: Url,
        url: Url,
        code: u16,
        reason: String,
        was_clean: bool,
    ) -> Self {
        Self {
            socket_id,
            document_url,
            url,
            kind: WebSocketLifecycleKind::Close,
            error_text: None,
            close_code: Some(code),
            close_reason: Some(reason),
            was_clean: Some(was_clean),
        }
    }

    pub fn socket_id(&self) -> u64 {
        self.socket_id
    }

    pub fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn kind(&self) -> WebSocketLifecycleKind {
        self.kind
    }

    pub fn error_text(&self) -> Option<&str> {
        self.error_text.as_deref()
    }

    pub fn close_code(&self) -> Option<u16> {
        self.close_code
    }

    pub fn close_reason(&self) -> Option<&str> {
        self.close_reason.as_deref()
    }

    pub fn was_clean(&self) -> Option<bool> {
        self.was_clean
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketLifecycleKind {
    Open,
    Error,
    Closing,
    Close,
}

impl WebSocketLifecycleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Error => "error",
            Self::Closing => "closing",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketFrameDirection {
    Sent,
    Received,
}

impl WebSocketFrameDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Received => "received",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketFrameOpcode {
    Text,
    Binary,
}

impl WebSocketFrameOpcode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Binary => "binary",
        }
    }
}

/// Identifies the renderer realm that invoked one Runtime binding.
///
/// The realm generation is the renderer's opaque runtime-observable context
/// token, not a protocol execution-context id. It is only exact together with
/// the installed Page generation carried by the protocol prepared batch.
/// Keeping it on the call prevents a delayed consumer from reinterpreting the
/// call through whichever realm currently reuses the public context id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeBindingCallSourceIdentity {
    local_window_id: u64,
    realm_generation: u64,
}

impl RuntimeBindingCallSourceIdentity {
    pub const fn new(local_window_id: u64, realm_generation: u64) -> Self {
        Self {
            local_window_id,
            realm_generation,
        }
    }

    pub const fn local_window_id(self) -> u64 {
        self.local_window_id
    }

    pub const fn realm_generation(self) -> u64 {
        self.realm_generation
    }
}

/// One Runtime binding invocation frozen at the renderer callback boundary.
///
/// `execution_context_id` is the public compatibility id emitted to CDP.
/// `source` is the opaque renderer identity that proves which realm produced
/// it; consumers must not recompute either value from current Page state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRuntimeBindingCall {
    pub source: RuntimeBindingCallSourceIdentity,
    pub name: String,
    pub payload: String,
    pub execution_context_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBindingRegistration {
    pub name: String,
    pub execution_context_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeContextRestoreEvent {
    Created(RuntimeExecutionContextRestoreEvent),
    Destroyed(RuntimeExecutionContextRestoreEvent),
    Cleared(RuntimeExecutionContextsClearedRestoreEvent),
}

impl RuntimeContextRestoreEvent {
    pub fn from_v8_inspector_message(message: Value) -> Option<Self> {
        match message["method"].as_str()? {
            "Runtime.executionContextCreated" => Some(Self::Created(
                RuntimeExecutionContextRestoreEvent::created_from_v8_inspector_params(
                    message["params"].clone(),
                ),
            )),
            "Runtime.executionContextDestroyed" => Some(Self::Destroyed(
                RuntimeExecutionContextRestoreEvent::destroyed_from_v8_inspector_params(
                    message["params"].clone(),
                ),
            )),
            "Runtime.executionContextsCleared" => Some(Self::Cleared(
                RuntimeExecutionContextsClearedRestoreEvent {},
            )),
            _ => None,
        }
    }

    pub fn execution_context_created_id(&self) -> Option<i64> {
        match self {
            Self::Created(event) => event.context_id,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeExecutionContextRestoreEvent {
    pub context_id: Option<i64>,
    pub realm_id: Option<String>,
    pub frame_id: Option<String>,
    pub origin: Option<String>,
    pub name: Option<String>,
    pub is_default: Option<bool>,
    pub context_type: Option<String>,
    pub grant_universal_access: Option<bool>,
}

impl RuntimeExecutionContextRestoreEvent {
    fn created_from_v8_inspector_params(params: Value) -> Self {
        let context = &params["context"];
        let aux_data = &context["auxData"];
        Self {
            context_id: context["id"].as_i64(),
            realm_id: context["uniqueId"].as_str().map(str::to_owned),
            frame_id: aux_data["frameId"].as_str().map(str::to_owned),
            origin: context["origin"].as_str().map(str::to_owned),
            name: context["name"].as_str().map(str::to_owned),
            is_default: aux_data["isDefault"].as_bool(),
            context_type: aux_data["type"].as_str().map(str::to_owned),
            grant_universal_access: aux_data["grantUniversalAccess"].as_bool(),
        }
    }

    fn destroyed_from_v8_inspector_params(params: Value) -> Self {
        Self {
            context_id: params["executionContextId"].as_i64(),
            realm_id: params["executionContextUniqueId"]
                .as_str()
                .map(str::to_owned),
            frame_id: None,
            origin: None,
            name: None,
            is_default: None,
            context_type: None,
            grant_universal_access: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeExecutionContextsClearedRestoreEvent {}

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionOverrideRegistration {
    pub permission: Value,
    pub setting: String,
    pub origin: Option<String>,
    pub embedded_origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIsolatedWorldDefinition {
    pub name: String,
    pub grant_universal_access: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmulatedMediaOverrides {
    pub media: Option<String>,
    pub color_scheme: Option<String>,
    pub reduced_motion: Option<String>,
    pub forced_colors: Option<String>,
    pub contrast: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmulatedIdleOverride {
    pub is_user_active: bool,
    pub is_screen_unlocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportSurface {
    pub inner_width: u32,
    pub inner_height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub device_pixel_ratio: f64,
    pub screen_width: u32,
    pub screen_height: u32,
    pub screen_avail_width: u32,
    pub screen_avail_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChildFrameTreeSnapshot {
    pub frame_id: String,
    #[serde(default)]
    pub loader_id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub owner_element_id: Option<String>,
    pub url: String,
    #[serde(default)]
    pub storage_key: String,
    #[serde(default)]
    pub security_origin_inherited: bool,
    #[serde(default)]
    pub security_origin_opaque: bool,
    #[serde(default)]
    pub child_frames: Vec<ChildFrameTreeSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChildFrameAttachmentSnapshot {
    pub frame_id: String,
    #[serde(default)]
    pub parent_frame_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChildFrameDocumentOpenedSnapshot {
    pub frame_id: String,
    #[serde(default)]
    pub parent_frame_id: Option<String>,
    #[serde(default)]
    pub loader_id: Option<String>,
    pub name: Option<String>,
    pub url: String,
    #[serde(default)]
    pub security_origin_inherited: bool,
    #[serde(default)]
    pub security_origin_opaque: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChildFrameDetachmentSnapshot {
    pub frame_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ChildFrameTreeEventSnapshot {
    Attached(ChildFrameAttachmentSnapshot),
    Detached(ChildFrameDetachmentSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChildFrameNavigationSnapshot {
    pub frame_id: String,
    #[serde(default)]
    pub parent_frame_id: Option<String>,
    #[serde(default)]
    pub loader_id: Option<String>,
    pub name: Option<String>,
    pub url: String,
    #[serde(default)]
    pub document_open_replacement: bool,
    #[serde(default)]
    pub security_origin_inherited: bool,
    #[serde(default)]
    pub security_origin_opaque: bool,
    #[serde(default)]
    pub document_network: Option<ChildFrameDocumentNetworkSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChildFrameDocumentNetworkSnapshot {
    pub request_url: String,
    pub request_method: String,
    #[serde(default)]
    pub request_headers: Vec<(String, String)>,
    pub final_url: String,
    pub status: u16,
    #[serde(default)]
    pub response_headers: Vec<(String, String)>,
    #[serde(default)]
    pub encoded_data_length: usize,
    /// Exact in-process response body source for protocol consumers.
    ///
    /// Renderer/protocol transport shares this carrier without copying the
    /// complete payload. Serialized snapshots retain their historical wire
    /// shape and therefore deserialize without a body source.
    #[serde(skip)]
    pub response_body: Option<SubresourceResponseBody>,
    #[serde(default)]
    pub from_cache: bool,
}

/// A completed child main-resource request whose Network facts remain
/// observable even though its navigation no longer owns the current child
/// Document.
///
/// Keeping this separate from `ChildFrameNavigationSnapshot` prevents a stale
/// response from synthesizing a navigation commit or lifecycle terminal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChildFrameDocumentNetworkActivitySnapshot {
    pub frame_id: String,
    #[serde(default)]
    pub parent_frame_id: Option<String>,
    pub loader_id: String,
    pub snapshot: ChildFrameDocumentNetworkSnapshot,
}

#[derive(Debug, Clone)]
pub struct ScriptRun {
    node_id: NodeId,
    kind: ScriptKind,
    mode: ScriptMode,
    source_kind: ScriptSourceKind,
    url: Url,
    outcome: ScriptRunOutcome,
}

impl ScriptRun {
    pub fn executed(
        node_id: NodeId,
        kind: ScriptKind,
        mode: ScriptMode,
        source_kind: ScriptSourceKind,
        url: Url,
    ) -> Self {
        Self {
            node_id,
            kind,
            mode,
            source_kind,
            url,
            outcome: ScriptRunOutcome::Executed,
        }
    }

    pub fn skipped(
        node_id: NodeId,
        kind: ScriptKind,
        mode: ScriptMode,
        source_kind: ScriptSourceKind,
        url: Url,
        reason: ScriptSkipReason,
    ) -> Self {
        Self {
            node_id,
            kind,
            mode,
            source_kind,
            url,
            outcome: ScriptRunOutcome::Skipped(reason),
        }
    }

    pub fn failed(
        node_id: NodeId,
        kind: ScriptKind,
        mode: ScriptMode,
        source_kind: ScriptSourceKind,
        url: Url,
        message: String,
    ) -> Self {
        Self {
            node_id,
            kind,
            mode,
            source_kind,
            url,
            outcome: ScriptRunOutcome::Failed(message),
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn kind(&self) -> ScriptKind {
        self.kind
    }

    pub fn mode(&self) -> ScriptMode {
        self.mode
    }

    pub fn source_kind(&self) -> ScriptSourceKind {
        self.source_kind
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn outcome(&self) -> &ScriptRunOutcome {
        &self.outcome
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsValueSnapshot {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Unsupported(String),
}

impl JsValueSnapshot {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) | Self::Unsupported(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScriptRunOutcome {
    Executed,
    Skipped(ScriptSkipReason),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Classic,
    Module,
    ImportMap,
    DataBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptMode {
    Normal,
    Defer,
    ModuleDefer,
    InOrder,
    ImportMapInOrder,
    ModuleInOrder,
    Async,
}

impl ScriptMode {
    pub fn is_defer_like(self) -> bool {
        matches!(self, Self::Defer | Self::ModuleDefer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptSourceKind {
    Inline,
    External,
}

impl ScriptSourceKind {
    pub fn from_script_src(src: &Option<String>) -> Self {
        if src.is_some() {
            Self::External
        } else {
            Self::Inline
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSkipReason {
    NotInMainDocument,
    AlreadyStarted,
    ScriptExecutionDisabled,
    EmptyInlineScript,
    NoModule,
    ModuleNotSupported,
    ImportMapNotSupported,
    UnsupportedType(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_url(path: &str) -> Url {
        Url::parse(&format!("https://example.test{path}")).expect("test URL should parse")
    }

    #[test]
    fn script_execution_report_exposes_globals_snapshot_freshness() {
        let mut report = ScriptExecutionReport::default();
        assert_eq!(
            report.globals_snapshot_state(),
            ScriptGlobalsSnapshotState::Uncaptured
        );
        assert_eq!(report.fresh_globals(), None);
        assert!(!report.mark_globals_snapshot_dirty());

        let mut globals = BTreeMap::new();
        globals.insert("answer".to_owned(), JsValueSnapshot::Number(42.0));
        assert!(report.replace_globals_snapshot(globals));
        assert_eq!(
            report
                .fresh_globals()
                .and_then(|values| values.get("answer")),
            Some(&JsValueSnapshot::Number(42.0))
        );

        let cloned = report.clone();
        assert!(Arc::ptr_eq(&report.globals, &cloned.globals));

        assert!(report.mark_globals_snapshot_dirty());
        assert!(!report.mark_globals_snapshot_dirty());
        assert_eq!(
            report.globals_snapshot_state(),
            ScriptGlobalsSnapshotState::Dirty
        );
        assert!(report.fresh_globals().is_none());
        assert_eq!(
            report.global("answer"),
            Some(&JsValueSnapshot::Number(42.0)),
            "the compatibility accessor must retain the last complete snapshot"
        );

        assert!(report.replace_globals_snapshot(BTreeMap::new()));
        assert!(report.globals_are_fresh());
        assert!(report.globals().is_empty());
    }

    #[test]
    fn scroll_into_view_rect_only_accepts_finite_protocol_numbers() {
        assert_eq!(
            DomScrollIntoViewRect::try_new(1.0, 2.0, -3.0, 4.0),
            Some(DomScrollIntoViewRect {
                x: 1.0,
                y: 2.0,
                width: -3.0,
                height: 4.0,
            })
        );

        for components in [
            [f64::NAN, 0.0, 0.0, 0.0],
            [0.0, f64::INFINITY, 0.0, 0.0],
            [0.0, 0.0, f64::NEG_INFINITY, 0.0],
            [0.0, 0.0, 0.0, f64::NAN],
        ] {
            assert!(
                DomScrollIntoViewRect::try_new(
                    components[0],
                    components[1],
                    components[2],
                    components[3],
                )
                .is_none()
            );
        }
    }

    #[test]
    fn optional_resource_fetch_mask_assigns_one_stable_bit_per_resource_family() {
        let resources = [
            (
                OptionalResourceFetchMask::IMAGE,
                1 << 0,
                SubresourceResourceType::Image,
            ),
            (
                OptionalResourceFetchMask::FONT,
                1 << 1,
                SubresourceResourceType::Font,
            ),
            (
                OptionalResourceFetchMask::AUDIO,
                1 << 2,
                SubresourceResourceType::Audio,
            ),
            (
                OptionalResourceFetchMask::VIDEO,
                1 << 3,
                SubresourceResourceType::Video,
            ),
            (
                OptionalResourceFetchMask::MEDIA,
                1 << 4,
                SubresourceResourceType::Media,
            ),
            (
                OptionalResourceFetchMask::TEXT_TRACK,
                1 << 5,
                SubresourceResourceType::TextTrack,
            ),
        ];

        assert_eq!(
            OptionalResourceFetchMask::default(),
            OptionalResourceFetchMask::NONE
        );
        assert!(OptionalResourceFetchMask::NONE.is_empty());
        assert_eq!(OptionalResourceFetchMask::ALL.bits(), 0b00_111111);

        for (index, (mask, expected_bit, resource_type)) in resources.iter().enumerate() {
            assert_eq!(mask.bits(), *expected_bit);
            assert_eq!(
                OptionalResourceFetchMask::for_resource_type(*resource_type),
                Some(*mask)
            );
            for (other_index, (other, _, _)) in resources.iter().enumerate() {
                assert_eq!(
                    mask.contains(*other),
                    index == other_index,
                    "{resource_type:?} unexpectedly shared a bit"
                );
            }
        }
    }

    #[test]
    fn cdp_fetch_interception_groups_fetch_event_source_and_xhr() {
        let fetch_like = [
            SubresourceResourceType::Fetch,
            SubresourceResourceType::EventSource,
            SubresourceResourceType::Xhr,
        ];

        for resource_type in fetch_like {
            assert_eq!(resource_type.as_cdp_fetch_interception_type(), "XHR");
            for other in fetch_like {
                assert!(resource_type.has_same_cdp_fetch_interception_type(other));
            }
            assert!(
                !resource_type
                    .has_same_cdp_fetch_interception_type(SubresourceResourceType::Script)
            );
        }

        assert_eq!(SubresourceResourceType::Fetch.as_cdp_type(), "Fetch");
        assert_eq!(
            SubresourceResourceType::EventSource.as_cdp_type(),
            "EventSource"
        );
        assert_eq!(SubresourceResourceType::Xhr.as_cdp_type(), "XHR");
    }

    #[test]
    fn optional_resource_fetch_mask_round_trips_valid_bits_and_rejects_unknown_bits() {
        for bits in 0..=OptionalResourceFetchMask::ALL.bits() {
            assert_eq!(
                OptionalResourceFetchMask::from_bits(bits).map(|mask| mask.bits()),
                Some(bits)
            );
        }
        for bits in [
            1 << 6,
            1 << 7,
            OptionalResourceFetchMask::ALL.bits() | (1 << 6),
            u8::MAX,
        ] {
            assert_eq!(OptionalResourceFetchMask::from_bits(bits), None);
        }
    }

    #[test]
    fn optional_resource_fetch_mask_allows_only_enabled_optional_families() {
        let optional = [
            (
                SubresourceResourceType::Image,
                OptionalResourceFetchMask::IMAGE,
            ),
            (
                SubresourceResourceType::Font,
                OptionalResourceFetchMask::FONT,
            ),
            (
                SubresourceResourceType::Audio,
                OptionalResourceFetchMask::AUDIO,
            ),
            (
                SubresourceResourceType::Video,
                OptionalResourceFetchMask::VIDEO,
            ),
            (
                SubresourceResourceType::Media,
                OptionalResourceFetchMask::MEDIA,
            ),
            (
                SubresourceResourceType::TextTrack,
                OptionalResourceFetchMask::TEXT_TRACK,
            ),
        ];
        let required = [
            SubresourceResourceType::Script,
            SubresourceResourceType::Stylesheet,
            SubresourceResourceType::Fetch,
            SubresourceResourceType::EventSource,
            SubresourceResourceType::Xhr,
            SubresourceResourceType::Ping,
            SubresourceResourceType::CspReport,
            SubresourceResourceType::Dictionary,
            SubresourceResourceType::WebSocket,
        ];

        for resource_type in required {
            assert!(OptionalResourceFetchMask::NONE.allows(resource_type));
        }
        for (resource_type, bit) in optional {
            assert!(!OptionalResourceFetchMask::NONE.allows(resource_type));
            assert!(bit.allows(resource_type));
            assert!(OptionalResourceFetchMask::ALL.allows(resource_type));
        }
    }

    #[test]
    fn optional_resource_fetch_mask_combines_and_removes_independent_bits() {
        let mut mask = OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::AUDIO;
        mask |= OptionalResourceFetchMask::TEXT_TRACK;
        assert_eq!(
            mask.bits(),
            OptionalResourceFetchMask::IMAGE.bits()
                | OptionalResourceFetchMask::AUDIO.bits()
                | OptionalResourceFetchMask::TEXT_TRACK.bits()
        );

        mask.set(OptionalResourceFetchMask::AUDIO, false);
        mask.set(OptionalResourceFetchMask::FONT, true);
        assert!(mask.contains(OptionalResourceFetchMask::IMAGE));
        assert!(mask.contains(OptionalResourceFetchMask::FONT));
        assert!(!mask.contains(OptionalResourceFetchMask::AUDIO));
        assert!(mask.contains(OptionalResourceFetchMask::TEXT_TRACK));
        assert_eq!(
            mask & OptionalResourceFetchMask::ALL,
            mask,
            "intersection with ALL should preserve every enabled bit"
        );
    }

    #[test]
    fn script_execution_report_extends_network_output_as_one_batch() {
        let document_url = test_url("/");
        let record = SubresourceNetworkRecord::failure(
            Some("FRAME".to_owned()),
            document_url.clone(),
            test_url("/api"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "network failed".to_owned(),
        );
        let websocket_event = WebSocketNetworkEvent::new(
            7,
            document_url.clone(),
            test_url("/socket"),
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Text,
            5,
        );
        let lifecycle_event =
            WebSocketLifecycleEvent::open(7, document_url.clone(), test_url("/socket"));
        let output = ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event.clone()),
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event.clone()),
        ]);
        assert!(!output.is_empty());

        let mut report = ScriptExecutionReport::default();
        report.extend_network_output(output);

        assert_eq!(
            report.subresource_network_records(),
            std::slice::from_ref(&record)
        );
        assert_eq!(
            report.websocket_network_events(),
            std::slice::from_ref(&websocket_event)
        );
        assert_eq!(
            report.websocket_lifecycle_events(),
            std::slice::from_ref(&lifecycle_event)
        );
        assert_eq!(
            report.network_output_items(),
            &[
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event.clone()),
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event.clone()),
            ],
        );
    }

    #[test]
    fn staged_subresource_network_output_materializes_settled_report_record() {
        let handle = SubresourceNetworkRequestHandle::new(42);
        let document_url = test_url("/");
        let request = SubresourceRequestStarted::new(
            handle,
            Some("FRAME".to_owned()),
            document_url,
            test_url("/image.png"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Image,
            SubresourceRequestInitiatorType::Parser,
            None,
        );
        let response = SubresourceResponseStarted::new(
            handle,
            Vec::new(),
            test_url("/image.png"),
            200,
            vec![("content-type".to_owned(), "image/png".to_owned())],
            Vec::new(),
        );
        let body = SubresourceBodyFinished::ready(
            handle,
            SubresourceResponseBody::from_text_and_bytes(String::new(), vec![1, 2, 3]),
        );
        let mut report = ScriptExecutionReport::default();
        report.extend_network_output(ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request.clone())),
        ]));
        report.extend_network_output(ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response.clone())),
        ]));
        assert!(report.subresource_network_records().is_empty());
        report.extend_network_output(ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(body.clone())),
        ]));

        let [record] = report.subresource_network_records() else {
            panic!("settled staged lifecycle should materialize one report record")
        };
        assert_eq!(record.request_handle(), Some(handle));
        assert_eq!(
            record.request_initiator_type(),
            SubresourceRequestInitiatorType::Parser
        );
        let SubresourceNetworkOutcome::Success {
            status,
            response_body,
            ..
        } = record.outcome()
        else {
            panic!("settled staged response should remain successful")
        };
        assert_eq!(*status, 200);
        assert_eq!(response_body.try_bytes().unwrap().as_ref(), &[1, 2, 3]);
        assert_eq!(
            report.network_output_items(),
            &[
                ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
                ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
                ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(body)),
            ],
        );
    }

    #[test]
    fn script_network_output_items_preserve_explicit_append_order() {
        let document_url = test_url("/");
        let record = SubresourceNetworkRecord::failure(
            Some("FRAME".to_owned()),
            document_url.clone(),
            test_url("/api"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "network failed".to_owned(),
        );
        let websocket_event = WebSocketNetworkEvent::new(
            7,
            document_url.clone(),
            test_url("/socket"),
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Text,
            5,
        );
        let lifecycle_event =
            WebSocketLifecycleEvent::open(7, document_url.clone(), test_url("/socket"));
        let output = ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event.clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event.clone()),
        ]);

        let items: Vec<_> = output.into_items().collect();

        assert_eq!(
            items,
            vec![
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event),
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event),
            ],
            "script network output iteration should preserve explicit producer append order"
        );
    }

    #[test]
    fn script_network_output_from_items_materializes_grouped_report_shape() {
        let document_url = test_url("/");
        let record = SubresourceNetworkRecord::failure(
            Some("FRAME".to_owned()),
            document_url.clone(),
            test_url("/api"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "network failed".to_owned(),
        );
        let websocket_event = WebSocketNetworkEvent::new(
            7,
            document_url.clone(),
            test_url("/socket"),
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Text,
            5,
        );
        let lifecycle_event =
            WebSocketLifecycleEvent::open(7, document_url.clone(), test_url("/socket"));

        let output = ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event.clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event.clone()),
        ]);

        assert_eq!(
            output.subresource_network_records.as_slice(),
            std::slice::from_ref(&record)
        );
        assert_eq!(
            output.websocket_network_events.as_slice(),
            std::slice::from_ref(&websocket_event)
        );
        assert_eq!(
            output.websocket_lifecycle_events.as_slice(),
            std::slice::from_ref(&lifecycle_event)
        );
        assert_eq!(
            output.items.as_slice(),
            &[
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event.clone()),
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event.clone()),
            ],
            "producer item order should survive materialization into grouped views"
        );
    }

    #[test]
    fn script_execution_report_preserves_network_output_item_order_from_producer_batch() {
        let document_url = test_url("/");
        let record = SubresourceNetworkRecord::failure(
            Some("FRAME".to_owned()),
            document_url.clone(),
            test_url("/api"),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            "network failed".to_owned(),
        );
        let websocket_event = WebSocketNetworkEvent::new(
            7,
            document_url.clone(),
            test_url("/socket"),
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Text,
            5,
        );
        let lifecycle_event =
            WebSocketLifecycleEvent::open(7, document_url.clone(), test_url("/socket"));
        let output = ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event.clone()),
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record.clone())),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event.clone()),
        ]);

        let mut report = ScriptExecutionReport::default();
        report.extend_network_output(output);

        assert_eq!(
            report.network_output_items(),
            &[
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(lifecycle_event),
                ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(record)),
                ScriptNetworkOutputItem::WebSocketNetworkEvent(websocket_event),
            ],
            "report should keep producer item order for later core/CDP output queues"
        );
    }

    #[test]
    fn script_observable_output_items_preserve_batch_append_order() {
        let output = ScriptObservableOutput::from_items([
            ScriptObservableOutputItem::LifecycleError("warn-a".to_owned()),
            ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
            ScriptObservableOutputItem::LifecycleError("warn-b".to_owned()),
        ]);

        let mut report = ScriptExecutionReport::default();
        report.extend_observable_output(output);

        assert_eq!(report.console_messages(), &["console-a".to_owned()]);
        assert_eq!(
            report.lifecycle_errors(),
            &["warn-a".to_owned(), "warn-b".to_owned()]
        );
        assert_eq!(
            report.observable_output_items(),
            &[
                ScriptObservableOutputItem::LifecycleError("warn-a".to_owned()),
                ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
                ScriptObservableOutputItem::LifecycleError("warn-b".to_owned()),
            ],
            "report should keep observable producer item order for later core/CDP queues"
        );
    }

    #[test]
    fn subresource_response_body_writer_keeps_small_body_in_memory() {
        let mut writer = SubresourceResponseBodyWriter::new(16);
        writer.append(b"hello");
        writer.append(b" world");
        let body = writer.finish();

        assert_eq!(body.len(), 11);
        assert_eq!(body.diagnostic_text(), "hello world");
        assert_eq!(body.clone_body_bytes(), b"hello world");
    }

    #[test]
    fn subresource_response_body_writer_spools_large_body_and_materializes_on_demand() {
        let mut writer = SubresourceResponseBodyWriter::new(4);
        writer.append(b"hello");
        writer.append(b" world");
        let body = writer.finish();

        assert_eq!(body.len(), 11);
        assert_eq!(body.diagnostic_text(), "hello world");

        let mut copied = Vec::new();
        body.write_bytes_to(&mut copied)
            .expect("spooled body should stream-copy into writer");
        assert_eq!(copied, b"hello world");
    }

    #[test]
    fn subresource_response_body_reads_memory_and_spooled_chunks_by_offset() {
        let memory = SubresourceResponseBody::from_text_and_bytes(
            "hello world".to_owned(),
            b"hello world".to_vec(),
        );
        assert_eq!(memory.read_chunk(6, 3).unwrap(), b"wor");
        assert_eq!(memory.materialize_bytes_from(6).unwrap(), b"world");
        assert_eq!(memory.diagnostic_clone_body_bytes_from(6), b"world");

        let mut writer = SubresourceResponseBodyWriter::new(4);
        writer.append(b"hello");
        writer.append(b" world");
        let spooled = writer.finish();
        assert_eq!(spooled.read_chunk(6, 3).unwrap(), b"wor");
        assert_eq!(spooled.materialize_bytes_from(6).unwrap(), b"world");
        assert_eq!(spooled.diagnostic_clone_body_bytes_from(6), b"world");
    }

    #[test]
    fn subresource_response_body_fallible_materialize_reports_missing_spool_file() {
        let missing_path = std::env::temp_dir().join(format!(
            "moli-missing-subresource-body-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&missing_path);
        let body = SubresourceResponseBody {
            inner: Arc::new(SubresourceResponseBodyInner::File {
                path: missing_path,
                len: 5,
                text_cache: Mutex::new(None),
            }),
        };

        assert!(body.materialize_bytes().is_err());
        assert!(body.materialize_bytes_from(2).is_err());
        assert!(body.try_materialized_body().is_err());
        assert!(body.try_text().is_err());
        assert!(body.try_bytes().is_err());
        assert!(
            body.try_to_navigation_response(ResponseHead {
                final_url: Url::parse("https://example.test/missing").unwrap(),
                status: 200,
                headers: Vec::new(),
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            })
            .is_err()
        );
        assert_eq!(body.diagnostic_clone_body_bytes(), Vec::<u8>::new());
        assert_eq!(
            body.diagnostic_materialized_body().as_materialized_bytes(),
            Some(&[][..])
        );
        assert_eq!(body.diagnostic_text(), "");
        assert_eq!(body.diagnostic_bytes().as_ref(), &[] as &[u8]);

        let record = SubresourceNetworkRecord::success_with_body(
            None,
            Url::parse("https://example.test/page").unwrap(),
            Url::parse("https://example.test/api").unwrap(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            Url::parse("https://example.test/api").unwrap(),
            200,
            vec![("content-type".to_owned(), "text/plain".to_owned())],
            body,
            Vec::new(),
        );
        let criteria = SubresourceResponseWaitCriteria {
            body_regex: Some(Regex::new("missing").unwrap()),
            ..SubresourceResponseWaitCriteria::default()
        };
        assert!(
            criteria.try_matches(&record).is_err(),
            "production wait criteria should surface body source read errors"
        );
        assert!(
            !criteria.diagnostic_matches(&record),
            "diagnostic compatibility matcher should degrade unreadable body to no match"
        );
    }

    #[test]
    fn url_wait_criteria_matches_original_and_final_urls_as_regex() {
        let record = SubresourceNetworkRecord::success(
            None,
            Url::parse("https://example.test/page").unwrap(),
            Url::parse("https://api.example.test/v1/orders/42").unwrap(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            Url::parse("https://api.example.test/v2/orders/42").unwrap(),
            200,
            Vec::new(),
            String::new(),
            Vec::new(),
        );

        for regex in [r"/v1/orders/\d+$", r"/v2/orders/\d+$"] {
            let criteria = SubresourceResponseWaitCriteria {
                url_regex: Some(Regex::new(regex).unwrap()),
                ..SubresourceResponseWaitCriteria::default()
            };
            assert!(criteria.try_matches(&record).is_ok_and(|matches| matches));
        }

        let non_matching = SubresourceResponseWaitCriteria {
            url_regex: Some(Regex::new(r"/v3/orders/\d+$").unwrap()),
            ..SubresourceResponseWaitCriteria::default()
        };
        assert!(
            non_matching
                .try_matches(&record)
                .is_ok_and(|matches| !matches)
        );
    }

    #[test]
    fn body_wait_criteria_supports_literal_and_regex_matching() {
        let record = SubresourceNetworkRecord::success_with_body(
            None,
            Url::parse("https://example.test/page").unwrap(),
            Url::parse("https://example.test/api").unwrap(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            Url::parse("https://example.test/api").unwrap(),
            200,
            vec![("content-type".to_owned(), "text/plain".to_owned())],
            SubresourceResponseBody::from_text_and_bytes(
                "order #42 ready".to_owned(),
                b"order #42 ready".to_vec(),
            ),
            Vec::new(),
        );

        let literal = SubresourceResponseWaitCriteria {
            body_contains: Some("order #42 ready".to_owned()),
            ..SubresourceResponseWaitCriteria::default()
        };
        assert!(literal.try_matches(&record).is_ok_and(|matches| matches));

        let regex_syntax_is_literal = SubresourceResponseWaitCriteria {
            body_contains: Some(r"order #\d+ ready".to_owned()),
            ..SubresourceResponseWaitCriteria::default()
        };
        assert!(
            regex_syntax_is_literal
                .try_matches(&record)
                .is_ok_and(|matches| !matches)
        );

        let criteria = SubresourceResponseWaitCriteria {
            body_regex: Some(Regex::new(r"order #\d+ ready").unwrap()),
            ..SubresourceResponseWaitCriteria::default()
        };
        assert!(criteria.try_matches(&record).is_ok_and(|matches| matches));

        let non_matching = SubresourceResponseWaitCriteria {
            body_regex: Some(Regex::new(r"^order #\d{3} ready$").unwrap()),
            ..SubresourceResponseWaitCriteria::default()
        };
        assert!(
            non_matching
                .try_matches(&record)
                .is_ok_and(|matches| !matches)
        );
    }

    #[test]
    fn json_path_wait_criteria_supports_equals_and_regex_with_shared_mime_parser() {
        let expectation = SubresourceJsonPathEquals {
            path: vec!["ok".to_owned()],
            expected: "true".to_owned(),
        };
        assert!(json_path_equals(
            &[(
                "content-type".to_owned(),
                "application/manifest+json;charset=utf-8".to_owned(),
            )],
            r#"{"ok":true}"#,
            &expectation,
        ));
        assert!(json_path_equals(
            &[(
                "content-type".to_owned(),
                "application/json; charset = utf-8".to_owned(),
            )],
            r#"{"ok":true}"#,
            &expectation,
        ));
        assert!(!json_path_equals(
            &[("content-type".to_owned(), "text/json".to_owned())],
            r#"{"ok":true}"#,
            &expectation,
        ));

        let regex_expectation = SubresourceJsonPathRegex {
            path: vec!["data".to_owned(), "url".to_owned()],
            regex: Regex::new(r"^/item/\d+$").unwrap(),
        };
        assert!(json_path_matches_regex(
            &[(
                "content-type".to_owned(),
                "application/json; charset=utf-8".to_owned(),
            )],
            r#"{"data":{"url":"/item/42"}}"#,
            &regex_expectation,
        ));
        assert!(!json_path_matches_regex(
            &[("content-type".to_owned(), "application/json".to_owned())],
            r#"{"data":{"url":"/orders/42"}}"#,
            &regex_expectation,
        ));
    }

    #[test]
    fn subresource_response_body_partial_eq_does_not_treat_unreadable_sources_as_empty() {
        let missing_path_a = std::env::temp_dir().join(format!(
            "moli-missing-subresource-body-eq-a-{}",
            std::process::id()
        ));
        let missing_path_b = std::env::temp_dir().join(format!(
            "moli-missing-subresource-body-eq-b-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&missing_path_a);
        let _ = fs::remove_file(&missing_path_b);
        let body_a = SubresourceResponseBody {
            inner: Arc::new(SubresourceResponseBodyInner::File {
                path: missing_path_a,
                len: 5,
                text_cache: Mutex::new(None),
            }),
        };
        let body_b = SubresourceResponseBody {
            inner: Arc::new(SubresourceResponseBodyInner::File {
                path: missing_path_b,
                len: 5,
                text_cache: Mutex::new(None),
            }),
        };

        assert_eq!(body_a, body_a.clone());
        assert_ne!(body_a, body_b);
        assert!(!body_a.diagnostic_byte_eq(&body_b));
    }

    #[test]
    fn subresource_response_body_partial_eq_keeps_file_backed_equality_io_free() {
        let mut left_writer = SubresourceResponseBodyWriter::new(2);
        left_writer.append(b"same");
        let left = left_writer.finish();

        let mut right_writer = SubresourceResponseBodyWriter::new(2);
        right_writer.append(b"same");
        let right = right_writer.finish();

        assert_ne!(
            left, right,
            "ordinary equality should not read separate file-backed sources"
        );
        assert!(
            left.try_byte_eq(&right)
                .expect("explicit byte equality should read spooled bodies"),
            "callers that need content equality must opt in to source I/O"
        );
    }

    #[test]
    fn subresource_network_record_response_body_byte_equality_is_explicit() {
        let mut left_writer = SubresourceResponseBodyWriter::new(2);
        left_writer.append(b"same");
        let left_body = left_writer.finish();

        let mut right_writer = SubresourceResponseBodyWriter::new(2);
        right_writer.append(b"same");
        let right_body = right_writer.finish();

        let left_record = SubresourceNetworkRecord::success_with_body(
            None,
            Url::parse("https://example.test/page").unwrap(),
            Url::parse("https://example.test/api").unwrap(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            Url::parse("https://example.test/api").unwrap(),
            200,
            Vec::new(),
            left_body,
            Vec::new(),
        );
        let right_record = SubresourceNetworkRecord::success_with_body(
            None,
            Url::parse("https://example.test/page").unwrap(),
            Url::parse("https://example.test/api").unwrap(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            Url::parse("https://example.test/api").unwrap(),
            200,
            Vec::new(),
            right_body,
            Vec::new(),
        );

        assert_ne!(
            left_record, right_record,
            "ordinary record equality should not read independent body sources"
        );
        assert!(
            left_record
                .try_response_body_byte_eq(&right_record)
                .expect("explicit record body equality should read spooled bodies"),
            "callers that need record body content equality must opt in"
        );
    }

    #[test]
    fn subresource_response_body_to_navigation_response_materializes_spooled_body_once() {
        let mut writer = SubresourceResponseBodyWriter::new(2);
        writer.append(b"hi ");
        writer.append(&[0xff, b'!']);
        let body = writer.finish();
        let navigation = body.diagnostic_navigation_response(ResponseHead {
            final_url: Url::parse("https://example.test/data").unwrap(),
            status: 200,
            headers: vec![(
                "content-type".to_owned(),
                "application/octet-stream".to_owned(),
            )],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        });

        assert_eq!(navigation.body_bytes(), &[b'h', b'i', b' ', 0xff, b'!']);
        assert_eq!(navigation.body_text(), "hi \u{fffd}!");
    }

    #[test]
    fn auth_challenge_parser_prefers_supported_scheme_from_later_header() {
        let challenge = extract_subresource_auth_challenge(&[
            (
                "www-authenticate".to_owned(),
                "Bearer realm=\"token-area\"".to_owned(),
            ),
            (
                "www-authenticate".to_owned(),
                "Basic realm=\"basic-area\"".to_owned(),
            ),
        ])
        .expect("auth challenge");

        assert_eq!(challenge.source, "Server");
        assert_eq!(challenge.scheme, "basic");
        assert_eq!(challenge.realm, "basic-area");
    }

    #[test]
    fn auth_challenge_parser_prefers_supported_scheme_from_combined_header() {
        let challenge = extract_subresource_auth_challenge(&[(
            "www-authenticate".to_owned(),
            "Bearer realm=\"token-area\", Basic realm=\"basic-area\"".to_owned(),
        )])
        .expect("auth challenge");

        assert_eq!(challenge.source, "Server");
        assert_eq!(challenge.scheme, "basic");
        assert_eq!(challenge.realm, "basic-area");
    }

    #[test]
    fn auth_challenge_parser_preserves_quoted_commas_and_escaped_quotes() {
        let challenge = extract_subresource_auth_challenge(&[(
            "www-authenticate".to_owned(),
            r#"Digest realm="area, with \"quote\"", nonce="deadbeef", qop="auth""#.to_owned(),
        )])
        .expect("auth challenge");

        assert_eq!(challenge.source, "Server");
        assert_eq!(challenge.scheme, "digest");
        assert_eq!(challenge.realm, "area, with \"quote\"");
    }

    #[test]
    fn auth_challenge_parser_recognizes_proxy_and_falls_back_to_unsupported() {
        let challenge = extract_subresource_auth_challenge(&[(
            "proxy-authenticate".to_owned(),
            "Bearer realm=\"proxy-token\"".to_owned(),
        )])
        .expect("proxy auth challenge");

        assert_eq!(challenge.source, "Proxy");
        assert_eq!(challenge.scheme, "bearer");
        assert_eq!(challenge.realm, "proxy-token");
    }

    #[test]
    fn auth_credentials_map_proxy_basic_to_proxy_header_and_digest_to_proxy_auth() {
        let basic = subresource_auth_credentials_for_challenge(
            &SubresourceAuthChallenge {
                source: "Proxy".to_owned(),
                scheme: "Basic".to_owned(),
                realm: "proxy".to_owned(),
            },
            "user",
            "pass",
        )
        .expect("basic credentials");
        assert_eq!(basic.target, SubresourceAuthTarget::ProxyHeader);
        assert_eq!(basic.scheme, SubresourceAuthScheme::Basic);

        let digest = subresource_auth_credentials_for_challenge(
            &SubresourceAuthChallenge {
                source: "Proxy".to_owned(),
                scheme: "digest".to_owned(),
                realm: "proxy".to_owned(),
            },
            "user",
            "pass",
        )
        .expect("digest credentials");
        assert_eq!(digest.target, SubresourceAuthTarget::Proxy);
        assert_eq!(digest.scheme, SubresourceAuthScheme::Digest);
    }
}
