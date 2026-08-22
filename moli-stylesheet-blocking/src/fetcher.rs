use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use moli_dom::NodeId;
use moli_fetch::{RequestCredentialsMode, RequestMode};
use moli_page_types::NavigationResponse;
use url::Url;

use crate::types::{
    StylesheetBlockingStatus, StylesheetImportGraphFetchResult, StylesheetImportNetworkResult,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StylesheetFetchOptions(Arc<StylesheetFetchOptionsData>);

#[derive(Debug, Default, PartialEq, Eq, Hash)]
struct StylesheetFetchOptionsData {
    cross_origin: Option<String>,
    referrer_policy: Option<String>,
    integrity: Option<String>,
    nonce: Option<String>,
    charset: Option<String>,
    fetch_priority: Option<String>,
}

impl Default for StylesheetFetchOptions {
    fn default() -> Self {
        Self(Arc::new(StylesheetFetchOptionsData::default()))
    }
}

impl StylesheetFetchOptions {
    pub fn from_link_attributes(
        cross_origin: Option<&str>,
        referrer_policy: Option<&str>,
        integrity: Option<&str>,
        nonce: Option<&str>,
        charset: Option<&str>,
        fetch_priority: Option<&str>,
    ) -> Self {
        Self(Arc::new(StylesheetFetchOptionsData {
            cross_origin: normalize_cross_origin(cross_origin),
            referrer_policy: normalize_token(referrer_policy),
            integrity: normalize_preserved_value(integrity),
            nonce: normalize_preserved_value(nonce),
            charset: normalize_token(charset),
            fetch_priority: normalize_token(fetch_priority),
        }))
    }

    pub fn cross_origin(&self) -> Option<&str> {
        self.0.cross_origin.as_deref()
    }

    pub fn referrer_policy(&self) -> Option<&str> {
        self.0.referrer_policy.as_deref()
    }

    pub fn integrity(&self) -> Option<&str> {
        self.0.integrity.as_deref()
    }

    pub fn nonce(&self) -> Option<&str> {
        self.0.nonce.as_deref()
    }

    pub fn charset(&self) -> Option<&str> {
        self.0.charset.as_deref()
    }

    pub fn fetch_priority(&self) -> Option<&str> {
        self.0.fetch_priority.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn request_mode_and_credentials(&self) -> (RequestMode, RequestCredentialsMode) {
        match self.cross_origin() {
            None => (RequestMode::NoCors, RequestCredentialsMode::Include),
            Some("use-credentials") => (RequestMode::Cors, RequestCredentialsMode::Include),
            Some(_) => (RequestMode::Cors, RequestCredentialsMode::SameOrigin),
        }
    }

    pub fn resource_key(&self, request_url: Url) -> StylesheetResourceKey {
        StylesheetResourceKey::new(request_url, self)
    }
}

fn normalize_cross_origin(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.trim().eq_ignore_ascii_case("use-credentials") {
        Some("use-credentials".to_owned())
    } else {
        Some("anonymous".to_owned())
    }
}

fn normalize_token(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}

fn normalize_preserved_value(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StylesheetResourceKey {
    request_url_without_fragment: Url,
    request_mode: RequestMode,
    credentials_mode: RequestCredentialsMode,
    referrer_policy: Option<String>,
    integrity: Option<String>,
    charset: Option<String>,
}

impl StylesheetResourceKey {
    pub fn new(mut request_url: Url, options: &StylesheetFetchOptions) -> Self {
        request_url.set_fragment(None);
        let (request_mode, credentials_mode) = options.request_mode_and_credentials();
        Self {
            request_url_without_fragment: request_url,
            request_mode,
            credentials_mode,
            referrer_policy: options.referrer_policy().map(str::to_owned),
            integrity: options.integrity().map(str::to_owned),
            charset: options.charset().map(str::to_owned),
        }
    }

    pub fn request_url(&self) -> &Url {
        &self.request_url_without_fragment
    }

    pub fn request_mode(&self) -> RequestMode {
        self.request_mode
    }

    pub fn credentials_mode(&self) -> RequestCredentialsMode {
        self.credentials_mode
    }
}

#[derive(Debug, Clone)]
pub enum StylesheetPhysicalOutcome {
    Response(Arc<NavigationResponse>),
    NetworkError(Arc<str>),
}

impl StylesheetPhysicalOutcome {
    pub fn as_result(&self) -> Result<NavigationResponse, String> {
        match self {
            Self::Response(response) => Ok(response.as_ref().clone()),
            Self::NetworkError(error) => Err(error.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum StylesheetUsability {
    Ready,
    Failed { reason: Arc<str> },
}

impl StylesheetUsability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone)]
pub struct StylesheetFetchTerminal {
    physical: StylesheetPhysicalOutcome,
    usability: StylesheetUsability,
    origin_clean: Option<bool>,
}

impl StylesheetFetchTerminal {
    pub fn response(
        response: NavigationResponse,
        usability: StylesheetUsability,
        origin_clean: bool,
    ) -> Self {
        Self {
            physical: StylesheetPhysicalOutcome::Response(Arc::new(response)),
            usability,
            origin_clean: Some(origin_clean),
        }
    }

    pub fn ready(response: NavigationResponse, origin_clean: bool) -> Self {
        Self::response(response, StylesheetUsability::Ready, origin_clean)
    }

    pub fn unusable_response(
        response: NavigationResponse,
        origin_clean: bool,
        reason: impl Into<Arc<str>>,
    ) -> Self {
        Self::response(
            response,
            StylesheetUsability::Failed {
                reason: reason.into(),
            },
            origin_clean,
        )
    }

    pub fn network_error(reason: impl Into<Arc<str>>) -> Self {
        let reason = reason.into();
        Self {
            physical: StylesheetPhysicalOutcome::NetworkError(Arc::clone(&reason)),
            usability: StylesheetUsability::Failed { reason },
            origin_clean: None,
        }
    }

    pub fn physical(&self) -> &StylesheetPhysicalOutcome {
        &self.physical
    }

    pub fn usability(&self) -> &StylesheetUsability {
        &self.usability
    }

    pub fn is_ready(&self) -> bool {
        self.usability.is_ready()
    }

    pub fn origin_clean(&self) -> Option<bool> {
        self.origin_clean
    }

    pub fn ready_response(&self) -> Option<&NavigationResponse> {
        if !self.is_ready() {
            return None;
        }
        match &self.physical {
            StylesheetPhysicalOutcome::Response(response) => Some(response),
            StylesheetPhysicalOutcome::NetworkError(_) => None,
        }
    }
}

pub trait StylesheetFetcher: Clone + Send + 'static {
    fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>);

    fn fetch_stylesheet_resource(
        &self,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>>;

    /// Fetches every network resource needed by one stylesheet import graph.
    ///
    /// The default keeps this crate parser-independent and fetches only the
    /// URLs discovered by the caller. A browser renderer can override the
    /// method to parse successful responses and continue through nested
    /// imports while retaining this state machine's exact blocker identity.
    fn fetch_stylesheet_import_graph(
        &self,
        document_url: Url,
        urls: Vec<Url>,
    ) -> Pin<Box<dyn Future<Output = StylesheetImportGraphFetchResult> + Send + 'static>> {
        let fetcher = self.clone();
        Box::pin(async move {
            let pending = urls.into_iter().map(|url| {
                let fetcher = fetcher.clone();
                let fetch_document_url = document_url.clone();
                async move {
                    let start_unix_millis = moli_time::unix_epoch_millis();
                    let terminal = fetcher
                        .fetch_stylesheet_resource(
                            fetch_document_url,
                            url.clone(),
                            StylesheetFetchOptions::default(),
                        )
                        .await;
                    StylesheetImportNetworkResult::new(url, start_unix_millis, terminal)
                }
            });
            let network_results = futures_util::future::join_all(pending).await;
            let successful = network_results
                .iter()
                .all(|result| result.terminal().is_ready());
            StylesheetImportGraphFetchResult::new(successful, network_results)
        })
    }
}

/// Identity of one shared stylesheet resource fetch.
///
/// Clones refer to the same fetch object. Ownership decisions must use
/// [`StylesheetFetch::ptr_eq`] or [`StylesheetFetch::identity`], never a URL,
/// counter, or document generation.
#[derive(Clone)]
pub struct StylesheetFetch(Arc<StylesheetFetchRequest>);

/// Immutable lookup identity for an exact [`StylesheetFetch`].
///
/// The value is derived from the allocation address. A lookup table using it
/// must retain a `StylesheetFetch` clone in the corresponding value for as long
/// as the key remains present, preventing allocation-address reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StylesheetFetchIdentity(usize);

#[derive(Debug)]
struct StylesheetFetchRequest {
    document_url: Url,
    key: StylesheetResourceKey,
    options: StylesheetFetchOptions,
    start_unix_millis: f64,
    terminal: OnceLock<Arc<StylesheetFetchTerminal>>,
    physical_observation_emitted: AtomicBool,
}

impl StylesheetFetch {
    pub(crate) fn new(
        document_url: Url,
        request_url: Url,
        options: StylesheetFetchOptions,
        start_unix_millis: f64,
    ) -> Self {
        let key = options.resource_key(request_url);
        Self(Arc::new(StylesheetFetchRequest {
            document_url,
            key,
            options,
            start_unix_millis,
            terminal: OnceLock::new(),
            physical_observation_emitted: AtomicBool::new(false),
        }))
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn identity(&self) -> StylesheetFetchIdentity {
        StylesheetFetchIdentity(Arc::as_ptr(&self.0) as usize)
    }

    pub fn document_url(&self) -> &Url {
        &self.0.document_url
    }

    pub(crate) fn request_url(&self) -> &Url {
        self.0.key.request_url()
    }

    pub fn key(&self) -> &StylesheetResourceKey {
        &self.0.key
    }

    pub fn options(&self) -> &StylesheetFetchOptions {
        &self.0.options
    }

    pub(crate) fn start_unix_millis(&self) -> f64 {
        self.0.start_unix_millis
    }

    pub(crate) fn status(&self) -> StylesheetBlockingStatus {
        match self.terminal() {
            None => StylesheetBlockingStatus::Pending,
            Some(terminal) if terminal.is_ready() => StylesheetBlockingStatus::Ready,
            Some(_) => StylesheetBlockingStatus::Failed,
        }
    }

    pub fn terminal(&self) -> Option<Arc<StylesheetFetchTerminal>> {
        self.0.terminal.get().cloned()
    }

    pub(crate) fn finish(&self, terminal: Arc<StylesheetFetchTerminal>) -> bool {
        self.0.terminal.set(terminal).is_ok()
    }

    pub(crate) fn claim_physical_observation(&self) -> bool {
        !self
            .0
            .physical_observation_emitted
            .swap(true, Ordering::AcqRel)
    }
}

impl std::fmt::Debug for StylesheetFetch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StylesheetFetch")
            .field("document_url", &self.document_url())
            .field("key", &self.key())
            .field("options", &self.options())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct StylesheetFetchNetworkResult {
    pub fetch: Option<StylesheetFetch>,
    pub blocking_operation: Option<crate::StylesheetBlockingOperation>,
    pub document_url: Url,
    pub request_url: Url,
    pub owner_node_ids: Vec<NodeId>,
    pub start_unix_millis: f64,
    pub terminal: Arc<StylesheetFetchTerminal>,
}
