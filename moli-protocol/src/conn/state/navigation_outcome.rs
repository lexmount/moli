use moli_core::page::{
    Page, RendererMainDocumentCommit, RendererPageCreationArtifacts,
    RendererPendingDownloadActivation, RendererRuntimeRealmInfo,
};
use moli_core::runtime::NavigationEngine;
use moli_fetch::StreamingRawResponse;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use url::Url;

use crate::conn::ResponseCommitReady;
use crate::devtools_runtime::DevToolsProtocol;
use crate::domains::network::{
    CompletedDocumentProgressTransfer, CompletedDownloadProgressTransfer,
};

use super::browser_context::BrowserContext;

pub(crate) const NETWORK_ERROR_PAGE_URL: &str = "chrome-error://chromewebdata/";

#[derive(Clone, Debug)]
pub(crate) struct NetworkErrorPageNavigation {
    error_text: String,
    unreachable_url: Url,
}

impl NetworkErrorPageNavigation {
    pub(crate) fn new(error_text: String, unreachable_url: Url) -> Self {
        Self {
            error_text,
            unreachable_url,
        }
    }

    pub(crate) fn error_text(&self) -> &str {
        &self.error_text
    }

    pub(crate) fn unreachable_url(&self) -> &Url {
        &self.unreachable_url
    }
}

/// Security state inherited only by URLs such as `about:blank`.
///
/// Ordinary final URLs compute their own origin after redirects. Keeping this
/// source-Document fact separate avoids duplicating the navigation's frame,
/// loader, and timestamp inside `NavigationDispatchState`.
#[derive(Clone, Debug)]
pub(crate) struct NavigationSourceDocumentSecurityContext {
    security_origin: String,
    secure_context_type: String,
}

impl NavigationSourceDocumentSecurityContext {
    pub(crate) fn new(security_origin: String, secure_context_type: String) -> Self {
        Self {
            security_origin,
            secure_context_type,
        }
    }
}

impl Default for NavigationSourceDocumentSecurityContext {
    fn default() -> Self {
        Self::new("null".to_owned(), "Secure".to_owned())
    }
}

/// Main-frame protocol identity frozen when a cross-document navigation starts.
///
/// The final URL is intentionally resolved later, after redirects. Everything
/// else belongs to the navigation transaction and must not be rediscovered
/// from mutable target state after Fetch interception or background loading.
#[derive(Clone, Debug)]
pub(crate) struct RendererMainDocumentCommitSeed {
    frame_id: String,
    loader_id: String,
    timestamp: f64,
    inherited_security_origin: String,
    inherited_secure_context_type: String,
}

impl RendererMainDocumentCommitSeed {
    pub(crate) fn from_navigation(navigation: &NavigationDispatchState) -> Self {
        Self {
            frame_id: navigation.frame_id.clone(),
            loader_id: navigation.loader_id.clone(),
            timestamp: navigation.timestamp,
            inherited_security_origin: navigation.source_document_security.security_origin.clone(),
            inherited_secure_context_type: navigation
                .source_document_security
                .secure_context_type
                .clone(),
        }
    }

    /// Freezes the same renderer commit identity for a test navigation that
    /// starts from an already-installed target without creating a synthetic
    /// `NavigationDispatchState` or mutating the target's request counters.
    #[cfg(test)]
    pub(crate) fn from_navigation_fixture(
        frame_id: String,
        loader_id: String,
        timestamp: f64,
    ) -> Self {
        let source_document_security = NavigationSourceDocumentSecurityContext::default();
        Self {
            frame_id,
            loader_id,
            timestamp,
            inherited_security_origin: source_document_security.security_origin,
            inherited_secure_context_type: source_document_security.secure_context_type,
        }
    }

    pub(crate) fn resolve(
        &self,
        final_url: &Url,
        network_error_page: Option<&NetworkErrorPageNavigation>,
    ) -> RendererMainDocumentCommit {
        let inherits_initial_origin = moli_url::is_about_blank(final_url);
        let security_origin = if network_error_page.is_some() {
            "://".to_owned()
        } else if inherits_initial_origin {
            self.inherited_security_origin.clone()
        } else {
            moli_url::origin_ascii_serialization(final_url)
        };
        let secure_context_type = if network_error_page.is_some() {
            "InsecureScheme".to_owned()
        } else if inherits_initial_origin {
            self.inherited_secure_context_type.clone()
        } else if moli_url::is_potentially_trustworthy_url(final_url) {
            "Secure".to_owned()
        } else {
            "InsecureScheme".to_owned()
        };
        RendererMainDocumentCommit {
            frame_id: self.frame_id.clone(),
            loader_id: self.loader_id.clone(),
            url: final_url.as_str().to_owned(),
            unreachable_url: network_error_page
                .map(|error_page| error_page.unreachable_url().as_str().to_owned()),
            security_origin,
            secure_context_type,
            timestamp: self.timestamp,
        }
    }
}

#[derive(Debug)]
pub(crate) enum CompletedDownloadBody {
    Buffered(Vec<u8>),
    Streaming(Box<StreamingRawResponse>),
}

#[derive(Debug)]
pub(crate) struct CompletedDownloadBodyArtifact {
    body: CompletedDownloadBody,
    response_headers: Vec<(String, String)>,
}

impl CompletedDownloadBodyArtifact {
    pub(crate) fn from_body(
        body: CompletedDownloadBody,
        response_headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            body,
            response_headers,
        }
    }

    pub(crate) fn into_parts(self) -> (CompletedDownloadBody, Vec<(String, String)>) {
        (self.body, self.response_headers)
    }
}

#[derive(Debug)]
pub struct LoadedNavigation {
    pub page: Page,
    pub pending_download: Option<RendererPendingDownloadActivation>,
    pub page_creation_artifacts: RendererPageCreationArtifacts,
    pub requested_url: Url,
    pub final_url: Url,
    pub request_method: String,
    pub request_headers: Vec<(String, String)>,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    pub response_from_cache: bool,
    pub initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    pub renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    pub(crate) main_document_commit: Option<Arc<RendererMainDocumentCommit>>,
    pub(crate) document_progress_transfer: CompletedDocumentProgressTransfer,
    pub(crate) navigation_engine: Option<NavigationEngine>,
    pub(crate) network_error_page: Option<NetworkErrorPageNavigation>,
}

impl LoadedNavigation {
    pub(crate) fn with_navigation_engine(mut self, engine: NavigationEngine) -> Self {
        self.navigation_engine = Some(engine);
        self
    }

    #[cfg(test)]
    pub(crate) fn response_body(&self) -> String {
        self.document_progress_transfer
            .body()
            .materialize_lossy_string()
            .expect("test navigation body should be readable")
    }

    #[cfg(test)]
    pub(crate) fn completed_body_network_events(
        &self,
    ) -> &crate::domains::network::CompletedMainDocumentNetworkEvents {
        self.document_progress_transfer
            .completed_body_network_events()
            .expect("navigation uses streaming body network events")
    }
}

#[derive(Debug)]
pub struct DownloadNavigation {
    pub final_url: Url,
    pub(crate) progress_transfer: CompletedDownloadProgressTransfer,
}

#[derive(Debug)]
pub enum NavigationLoadOutcome {
    ResponseCommitReady(Box<ResponseCommitReady>),
    Loaded(Box<LoadedNavigation>),
    Download(Box<DownloadNavigation>),
    NetworkFailure(String),
}

impl NavigationLoadOutcome {
    pub(crate) fn response_commit_ready(navigation: ResponseCommitReady) -> Self {
        Self::ResponseCommitReady(Box::new(navigation))
    }

    pub(crate) fn loaded(navigation: LoadedNavigation) -> Self {
        Self::Loaded(Box::new(navigation))
    }

    pub(crate) fn download(navigation: DownloadNavigation) -> Self {
        Self::Download(Box::new(navigation))
    }

    pub(crate) fn network_failure(error_text: String) -> Self {
        Self::NetworkFailure(error_text)
    }

    pub(crate) fn with_navigation_engine(self, engine: NavigationEngine) -> Self {
        match self {
            Self::ResponseCommitReady(navigation) => {
                Self::response_commit_ready(navigation.with_navigation_engine(engine))
            }
            Self::Loaded(navigation) => Self::loaded(navigation.with_navigation_engine(engine)),
            Self::Download(navigation) => Self::Download(navigation),
            Self::NetworkFailure(error_text) => Self::NetworkFailure(error_text),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum NavigationResultProjection {
    Cdp(Value),
    WebDriverClassic(Value),
    WebDriverBidi(Value),
}

impl NavigationResultProjection {
    pub(crate) fn new(protocol: DevToolsProtocol, payload: Value) -> Self {
        match protocol {
            DevToolsProtocol::Cdp => Self::Cdp(payload),
            DevToolsProtocol::WebDriverClassic => Self::WebDriverClassic(payload),
            DevToolsProtocol::WebDriverBidi => Self::WebDriverBidi(payload),
        }
    }

    pub(crate) fn protocol(&self) -> DevToolsProtocol {
        match self {
            Self::Cdp(_) => DevToolsProtocol::Cdp,
            Self::WebDriverClassic(_) => DevToolsProtocol::WebDriverClassic,
            Self::WebDriverBidi(_) => DevToolsProtocol::WebDriverBidi,
        }
    }

    pub(crate) fn payload(&self) -> &Value {
        match self {
            Self::Cdp(payload) | Self::WebDriverClassic(payload) | Self::WebDriverBidi(payload) => {
                payload
            }
        }
    }

    pub(crate) fn payload_mut(&mut self) -> &mut Value {
        match self {
            Self::Cdp(payload) | Self::WebDriverClassic(payload) | Self::WebDriverBidi(payload) => {
                payload
            }
        }
    }

    pub(crate) fn into_payload(self) -> Value {
        match self {
            Self::Cdp(payload) | Self::WebDriverClassic(payload) | Self::WebDriverBidi(payload) => {
                payload
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct NavigationDispatchState {
    pub navigate_id: Option<u64>,
    pub navigate_session_id: Option<String>,
    pub(crate) result_projection: NavigationResultProjection,
    pub frame_id: String,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub loader_id: String,
    pub request_announced: bool,
    pub requested_url: Url,
    pub request_method: String,
    /// Text projection used by CDP request events and Fetch interception.
    pub request_body: Option<String>,
    /// Authoritative bytes used for transport. This differs from the text
    /// projection for multipart form data containing binary file payloads.
    pub request_body_bytes: Option<Vec<u8>>,
    pub request_headers: Vec<(String, String)>,
    pub request_load_policy: NavigationRequestLoadPolicy,
    pub timestamp: f64,
    pub(crate) source_document_security: NavigationSourceDocumentSecurityContext,
}

impl NavigationDispatchState {
    pub(crate) fn clone_request_body_bytes(&self) -> Option<Vec<u8>> {
        self.request_body_bytes
            .clone()
            .or_else(|| self.request_body.clone().map(String::into_bytes))
    }

    pub(crate) fn set_request_body_text(&mut self, body: String) {
        self.request_body_bytes = Some(body.as_bytes().to_vec());
        self.request_body = Some(body);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NavigationRequestLoadPolicy {
    #[default]
    DocumentInitiated,
    BrowserInitiated,
    Reload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo<'a> {
    pub target_id: &'a str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub title: String,
    pub url: &'a str,
    pub attached: bool,
    pub can_access_opener: bool,
    pub browser_context_id: &'a str,
}

impl<'a> TargetInfo<'a> {
    pub fn from_bc(bc: &'a BrowserContext, target_id: &'a str, attached: bool) -> Self {
        Self {
            target_id,
            kind: "page",
            title: bc
                .active_page_target()
                .owner_state
                .committed_document_title()
                .map(str::to_owned)
                .or_else(|| {
                    bc.active_page_target()
                        .runtime_slot
                        .loaded_page()
                        .map(|page| page.document_title())
                })
                .unwrap_or_default(),
            url: bc.target_url(),
            attached,
            can_access_opener: false,
            browser_context_id: &bc.id,
        }
    }
}
