//! HTML/XML parsing and parser-neutral handoff types for Moli.
//!
//! This crate owns document construction, parser streaming, and parse-time
//! discovery outputs without depending on the renderer implementation.

mod html;
mod html_input;
mod live_target;
mod script_planning;
mod session;
mod stream;
mod stylesheet_blocking;
mod xml;
mod xml_stream;
mod xml_tree_viewer;

use std::collections::{HashMap, HashSet, VecDeque};

use html5ever::tree_builder::QuirksMode;
use moli_dom::native::NativeNodeId;
use moli_stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
};

pub use html::{
    DocumentStream, HtmlParser, ParserBlockingStylesheetPause,
    ParserCustomElementConstructionHandoff, ParserFinishDiscoverySignals, ParserInputContext,
    ParserInputQueue, ParserInputSession, ParserPumpOutcome, ParserPumpStep,
    ParserScriptElementStateTransition, ParserScriptHandoff, ParserScriptNoExecutionOutcome,
    ParserScriptPreparationFailure, ParserStreamDocumentSnapshot, ParserYield,
};
pub use live_target::{
    ParserDomMutation, ParserDomMutationConsumer, ParserDomReadConsumer,
    ParserElementCreationConsumer, ParserElementCreationRequest, ParserMutationEffectConsumer,
};
pub use script_planning::{
    ParserPlanningReadView, ParserScriptRead, PrepareScriptOutcome, PreparedImportMap,
    PreparedImportMapSource, PreparedScript, PreparedScriptRuntimeGeneration, ScriptFetchMetadata,
    ScriptFilterSkipReason, ScriptSource, build_prepared_import_map, build_prepared_script,
    classify_parser_script,
};
pub use xml::XmlParser;
pub use xml_stream::XmlDocumentStream;

const STREAM_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParserSourcePosition {
    line: u64,
    column: u64,
}

impl ParserSourcePosition {
    const UNKNOWN: Self = Self { line: 0, column: 0 };

    const fn line_only(line: u64) -> Self {
        Self { line, column: 0 }
    }
}

impl Default for ParserSourcePosition {
    fn default() -> Self {
        Self::line_only(1)
    }
}

#[derive(Debug)]
struct HtmlTreeSinkState {
    html_quirks_mode: QuirksMode,
    ready_parser_scripts: VecDeque<NativeNodeId>,
    discovered_async_prefetch_candidates: VecDeque<NativeNodeId>,
    discovered_modulepreload_link_candidates: VecDeque<NativeNodeId>,
    parser_meta_csp_candidates: HashSet<NativeNodeId>,
    discovered_parser_meta_csp_candidates: VecDeque<NativeNodeId>,
    defined_autonomous_custom_elements: HashSet<String>,
    pending_custom_element_construction_handoffs:
        VecDeque<html::ParserCustomElementConstructionHandoff>,
    pending_blocking_stylesheet_pause: Option<NativeNodeId>,
    discovered_blocking_stylesheet_inputs: VecDeque<DocumentOwnedBlockingStylesheetDiscoveryInput>,
    captured_blocking_stylesheet_nodes: HashSet<NativeNodeId>,
    captured_blocking_stylesheet_signatures: HashSet<DocumentBlockingStylesheetSignature>,
    finishing_tree_builder: bool,
    current_position: ParserSourcePosition,
    script_start_positions: HashMap<NativeNodeId, ParserSourcePosition>,
}

impl Default for HtmlTreeSinkState {
    fn default() -> Self {
        Self {
            html_quirks_mode: QuirksMode::NoQuirks,
            ready_parser_scripts: VecDeque::new(),
            discovered_async_prefetch_candidates: VecDeque::new(),
            discovered_modulepreload_link_candidates: VecDeque::new(),
            parser_meta_csp_candidates: HashSet::new(),
            discovered_parser_meta_csp_candidates: VecDeque::new(),
            defined_autonomous_custom_elements: HashSet::new(),
            pending_custom_element_construction_handoffs: VecDeque::new(),
            pending_blocking_stylesheet_pause: None,
            discovered_blocking_stylesheet_inputs: VecDeque::new(),
            captured_blocking_stylesheet_nodes: HashSet::new(),
            captured_blocking_stylesheet_signatures: HashSet::new(),
            finishing_tree_builder: false,
            current_position: ParserSourcePosition::default(),
            script_start_positions: HashMap::new(),
        }
    }
}

pub(crate) fn html_chunks(html: &str) -> impl Iterator<Item = &str> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < html.len() {
        let mut end = (start + STREAM_CHUNK_BYTES).min(html.len());
        while end > start && !html.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = html.len();
        }
        chunks.push(&html[start..end]);
        start = end;
    }

    chunks.into_iter()
}
