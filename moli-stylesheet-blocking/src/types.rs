use std::sync::Arc;

use moli_dom::NodeId;
use url::Url;

use crate::discovery::DocumentBlockingStylesheetSignature;
use crate::fetcher::{StylesheetFetch, StylesheetFetchOptions, StylesheetFetchTerminal};

pub(crate) struct StylesheetFetchEntry {
    pub(crate) resource_cache_scope: u64,
    pub(crate) signature: StylesheetFetchSignature,
    pub(crate) fetch: StylesheetFetch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StylesheetFetchSignature {
    pub(crate) url: Url,
    pub(crate) options: StylesheetFetchOptions,
}

impl TryFrom<&DocumentBlockingStylesheetSignature> for StylesheetFetchSignature {
    type Error = ();

    fn try_from(signature: &DocumentBlockingStylesheetSignature) -> Result<Self, Self::Error> {
        match signature {
            DocumentBlockingStylesheetSignature::Link { url, options } => Ok(Self {
                url: url.clone(),
                options: options.clone(),
            }),
            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { .. } => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StylesheetBlockingStatus {
    Pending,
    Ready,
    Failed,
}

pub(crate) type StylesheetFetchStatus = StylesheetBlockingStatus;

#[derive(Debug)]
pub(crate) struct StylesheetFetchCompletion {
    pub(crate) fetch: StylesheetFetch,
    pub(crate) terminal: StylesheetFetchTerminal,
}

#[derive(Clone)]
pub struct StylesheetBlockingOperation(Arc<StylesheetBlockingOperationIdentity>);

#[derive(Debug)]
struct StylesheetBlockingOperationIdentity {
    node_id: NodeId,
    document_url: Url,
    signature: DocumentBlockingStylesheetSignature,
}

impl StylesheetBlockingOperation {
    pub(crate) fn new(
        node_id: NodeId,
        document_url: Url,
        signature: DocumentBlockingStylesheetSignature,
    ) -> Self {
        Self(Arc::new(StylesheetBlockingOperationIdentity {
            node_id,
            document_url,
            signature,
        }))
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn node_id(&self) -> NodeId {
        self.0.node_id
    }

    pub fn signature(&self) -> &DocumentBlockingStylesheetSignature {
        &self.0.signature
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.0.document_url
    }
}

impl std::fmt::Debug for StylesheetBlockingOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StylesheetBlockingOperation")
            .field("node_id", &self.node_id())
            .field("document_url", &self.document_url())
            .field("signature", &self.signature())
            .finish_non_exhaustive()
    }
}

pub(crate) struct StylesheetBlockingEntry {
    pub(crate) operation: StylesheetBlockingOperation,
    pub(crate) resource: StylesheetBlockingResource,
}

pub(crate) enum StylesheetBlockingResource {
    Link(StylesheetFetch),
    StyleImports {
        status: StylesheetFetchStatus,
        completed_graph: Option<Arc<StylesheetImportGraphFetchResult>>,
    },
}

#[derive(Debug, Clone)]
pub struct StylesheetImportGraphFetchResult {
    pub(crate) successful: bool,
    pub(crate) network_results: Vec<StylesheetImportNetworkResult>,
}

impl StylesheetImportGraphFetchResult {
    pub fn new(successful: bool, network_results: Vec<StylesheetImportNetworkResult>) -> Self {
        Self {
            successful,
            network_results,
        }
    }

    pub fn successful(&self) -> bool {
        self.successful
    }

    pub fn network_results(&self) -> &[StylesheetImportNetworkResult] {
        &self.network_results
    }

    pub fn into_parts(self) -> (bool, Vec<StylesheetImportNetworkResult>) {
        (self.successful, self.network_results)
    }
}

pub(crate) struct StylesheetImportCompletion {
    pub(crate) operation: StylesheetBlockingOperation,
    pub(crate) graph: StylesheetImportGraphFetchResult,
}

#[derive(Debug, Clone)]
pub struct StylesheetImportNetworkResult {
    pub(crate) request_url: Url,
    pub(crate) start_unix_millis: f64,
    pub(crate) terminal: StylesheetFetchTerminal,
    import_options: Option<crate::StylesheetFetchOptions>,
}

impl StylesheetImportNetworkResult {
    pub fn new(
        request_url: Url,
        start_unix_millis: f64,
        terminal: StylesheetFetchTerminal,
    ) -> Self {
        Self {
            request_url,
            start_unix_millis,
            terminal,
            import_options: None,
        }
    }

    pub fn request_url(&self) -> &Url {
        &self.request_url
    }

    pub fn start_unix_millis(&self) -> f64 {
        self.start_unix_millis
    }

    pub fn terminal(&self) -> &StylesheetFetchTerminal {
        &self.terminal
    }

    pub fn with_import_options(mut self, options: Option<crate::StylesheetFetchOptions>) -> Self {
        self.import_options = options;
        self
    }

    pub fn import_options(&self) -> Option<&crate::StylesheetFetchOptions> {
        self.import_options.as_ref()
    }

    pub fn into_parts(self) -> (Url, f64, StylesheetFetchTerminal) {
        (self.request_url, self.start_unix_millis, self.terminal)
    }
}

pub struct StylesheetCompletion(StylesheetCompletionPayload);

pub(crate) enum StylesheetCompletionPayload {
    Fetch(Box<StylesheetFetchCompletion>),
    StyleImports(StylesheetImportCompletion),
}

impl StylesheetCompletion {
    pub(crate) fn fetch(completion: StylesheetFetchCompletion) -> Self {
        Self(StylesheetCompletionPayload::Fetch(Box::new(completion)))
    }

    pub(crate) fn style_imports(completion: StylesheetImportCompletion) -> Self {
        Self(StylesheetCompletionPayload::StyleImports(completion))
    }

    pub(crate) fn into_payload(self) -> StylesheetCompletionPayload {
        self.0
    }
}

impl std::fmt::Debug for StylesheetCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StylesheetCompletion")
            .finish_non_exhaustive()
    }
}
