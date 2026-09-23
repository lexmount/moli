//! Stylesheet blocking state shared by parser and renderer scheduling code.
//!
//! This crate tracks discovered blocking stylesheets and completion wakeups
//! using a small fetcher trait. The renderer supplies the actual network loader,
//! keeping this state machine independent from V8 and page runtime ownership.
//! Parser and renderer code depend on this crate's read contract and discovery
//! types. Parser-specific DOM adapters stay in `moli-parser`; this crate
//! does not depend on parser or renderer ownership.

mod discovery;
mod fetcher;
mod state;
mod types;

pub use discovery::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheet,
    DocumentOwnedBlockingStylesheetCandidate, DocumentOwnedBlockingStylesheetDiscoveryInput,
    StylesheetBlockingReadView, StylesheetElementRead, StylesheetLinkDisposition,
    StylesheetPreloadLinkRequest, collect_blocking_stylesheet_nodes_before,
    collect_document_owned_blocking_stylesheet_candidates,
    collect_document_owned_blocking_stylesheet_discovery_inputs_before_in_view,
    collect_document_owned_blocking_stylesheet_nodes_before,
    collect_document_owned_blocking_stylesheets,
    collect_document_owned_blocking_stylesheets_before,
    collect_document_owned_blocking_stylesheets_before_in_view, connected_preload_like_link_url,
    document_node_precedes, document_owned_blocking_stylesheet_candidate_for_node,
    link_rel_includes_token, preload_like_link_loads_stylesheet, stylesheet_link_disposition,
    stylesheet_preload_link_request,
};
pub use fetcher::{
    PreparedStylesheetFetch, StylesheetFetch, StylesheetFetchIdentity,
    StylesheetFetchNetworkResult, StylesheetFetchOptions, StylesheetFetchTerminal,
    StylesheetFetcher, StylesheetPhysicalOutcome, StylesheetResourceKey, StylesheetUsability,
};
pub use state::StylesheetBlockingState;
pub use types::{
    StylesheetBlockingOperation, StylesheetBlockingStatus, StylesheetCompletion,
    StylesheetImportGraphFetchResult, StylesheetImportNetworkResult,
};

/// Captured publication route for one stylesheet request or import graph.
pub type StylesheetCompletionPublisher =
    std::sync::Arc<dyn Fn(StylesheetCompletion) + Send + Sync + 'static>;
