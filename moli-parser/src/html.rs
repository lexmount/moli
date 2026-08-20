use std::{
    borrow::Cow,
    cell::{Cell, Ref, RefCell},
    collections::{HashSet, VecDeque},
    rc::Rc,
};

use html5ever::{
    Attribute, LocalName, Namespace, QualName,
    tendril::StrTendril,
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
};
use url::Url;

use crate::script_planning::{PreparedImportMap, PreparedScript};
use moli_dom::native::Node;
use moli_dom::native::{
    Attribute as NativeAttribute, DomHost, NativeDom, NativeDomNodes, NativeNodeId,
};
use moli_page_types::{ScriptMode, ScriptRun};
use moli_stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
};

use super::live_target::{
    ParserDomMutationConsumer, ParserDomReadConsumer, ParserElementCreationConsumer,
    ParserMutationEffectConsumer, ParserMutationEffectDelivery, ParserRuntimeDomSinks,
    ParserStreamHtmlTreeSinkTarget, new_live_document_root_html_tree_sink_stream,
    new_live_fragment_root_html_tree_sink_stream, new_parser_stream_html_tree_sink_stream,
    new_parser_stream_html_tree_sink_target,
};
use super::{
    ParserSourcePosition, html_chunks,
    session::{HtmlParserSession, html_parse_opts, html_parse_opts_with_scripting},
    stream::HtmlTreeSinkStream,
};

#[derive(Debug, Clone, Default)]
pub struct HtmlParser;

#[derive(Clone, Debug)]
pub struct ParserInputSession(Rc<RefCell<ParserInputState>>);

#[derive(Debug)]
pub struct ParserInputContext {
    session: ParserInputSession,
}

#[derive(Debug)]
pub struct ParserInputQueue(Rc<RefCell<ParserInputState>>);

#[derive(Debug, Default)]
struct ParserInputState {
    script_input_queue: VecDeque<String>,
    insertion_preload_queue: VecDeque<String>,
    pending_stack: Vec<String>,
    processed_insertion_meta_csp_count: usize,
}

pub struct DocumentStream {
    inner: HtmlTreeSinkStream,
    input: HtmlParserInputStream,
}

#[derive(Debug, Default)]
struct HtmlParserInputStream {
    end_segments: VecDeque<String>,
}

#[derive(Debug, Clone)]
pub struct ParserStreamDocumentSnapshot(NativeDom);

#[derive(Debug)]
pub struct ParserPumpOutcome {
    pub result: ParserPumpStep,
    pub discovered_async_prefetch_scripts: Vec<PreparedScript>,
    pub discovered_modulepreload_link_candidates: Vec<NativeNodeId>,
    pub discovered_blocking_stylesheet_inputs: Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
}

#[derive(Debug, Default)]
pub struct ParserFinishDiscoverySignals {
    pub parser_created_null_registry_elements: Vec<NativeNodeId>,
    pub discovered_modulepreload_link_candidates: Vec<NativeNodeId>,
    pub discovered_parser_meta_csp_candidates: Vec<NativeNodeId>,
    pub discovered_blocking_stylesheet_inputs: Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
}

#[derive(Debug, Clone)]
pub enum ParserScriptHandoff {
    BlockingClassic {
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        script: PreparedScript,
    },
    AsyncPostParse {
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        script: PreparedScript,
    },
    NonAsyncPostParse {
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        script: PreparedScript,
    },
    ImportMap {
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        import_map: PreparedImportMap,
    },
    NoExecution {
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        outcome: ParserScriptNoExecutionOutcome,
    },
    PreparationFailure {
        node_id: NativeNodeId,
        start_line: u64,
        start_column: u64,
        failure: ParserScriptPreparationFailure,
    },
}

impl ParserScriptHandoff {
    pub fn node_id(&self) -> NativeNodeId {
        match self {
            Self::BlockingClassic { node_id, .. }
            | Self::AsyncPostParse { node_id, .. }
            | Self::NonAsyncPostParse { node_id, .. }
            | Self::ImportMap { node_id, .. }
            | Self::NoExecution { node_id, .. }
            | Self::PreparationFailure { node_id, .. } => *node_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParserScriptNoExecutionOutcome {
    position: usize,
    mode: ScriptMode,
    run: Option<ScriptRun>,
    element_state_transition: ParserScriptElementStateTransition,
}

#[derive(Debug, Clone)]
pub struct ParserScriptPreparationFailure {
    position: usize,
    mode: ScriptMode,
    message: String,
    element_state_transition: ParserScriptElementStateTransition,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParserScriptElementStateTransition {
    #[default]
    None,
    ConsumeParserInserted {
        force_async: bool,
    },
    MarkAlreadyStarted,
}

impl ParserScriptNoExecutionOutcome {
    pub fn ignored(position: usize, mode: ScriptMode) -> Self {
        Self {
            position,
            mode,
            run: None,
            element_state_transition: ParserScriptElementStateTransition::None,
        }
    }

    pub fn skipped(position: usize, mode: ScriptMode, run: ScriptRun) -> Self {
        Self {
            position,
            mode,
            run: Some(run),
            element_state_transition: ParserScriptElementStateTransition::None,
        }
    }

    pub(crate) fn with_element_state_transition(
        mut self,
        transition: ParserScriptElementStateTransition,
    ) -> Self {
        self.element_state_transition = transition;
        self
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn mode(&self) -> ScriptMode {
        self.mode
    }

    pub fn run(&self) -> Option<&ScriptRun> {
        self.run.as_ref()
    }

    pub fn element_state_transition(&self) -> ParserScriptElementStateTransition {
        self.element_state_transition
    }

    pub fn into_parts(self) -> (usize, ScriptMode, Option<ScriptRun>) {
        (self.position, self.mode, self.run)
    }
}

impl ParserScriptPreparationFailure {
    pub fn new(position: usize, mode: ScriptMode, message: String) -> Self {
        Self {
            position,
            mode,
            message,
            element_state_transition: ParserScriptElementStateTransition::None,
        }
    }

    pub(crate) fn with_element_state_transition(
        mut self,
        transition: ParserScriptElementStateTransition,
    ) -> Self {
        self.element_state_transition = transition;
        self
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn mode(&self) -> ScriptMode {
        self.mode
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn element_state_transition(&self) -> ParserScriptElementStateTransition {
        self.element_state_transition
    }

    pub fn into_parts(self) -> (usize, ScriptMode, String) {
        (self.position, self.mode, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct ParserCustomElementConstructionHandoff {
    pub placeholder: NativeNodeId,
    pub local_name: String,
    pub namespace: String,
    pub prefix: Option<String>,
    pub attributes: Vec<NativeAttribute>,
    pub owner_document: NativeNodeId,
    pub parent_at_creation: Option<NativeNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParserBlockingStylesheetPause {
    pub node_id: NativeNodeId,
}

#[derive(Debug, Clone)]
pub enum ParserYield {
    Script(Box<ParserScriptHandoff>),
    CustomElementConstruction(Box<ParserCustomElementConstructionHandoff>),
    BlockingStylesheet(ParserBlockingStylesheetPause),
}

#[derive(Debug, Clone)]
pub enum ParserPumpStep {
    Yield(ParserYield),
    InputDrained,
}

#[derive(Debug, Clone)]
pub(super) struct ParseHandle {
    identity: ParseHandleIdentity,
    pub(super) element_name: Option<Rc<QualName>>,
    pub(super) parser_flags: ParserElementFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseHandleIdentity {
    DomNode(NativeNodeId),
    // Standalone fragment parsing, like `Element.innerHTML` staging, only has a
    // context element name. Chromium keeps a real `context_element` next to the
    // `DocumentFragment` target; our detached staging parser uses this
    // parser-only handle instead of pretending the document node is that
    // context. It may answer parser questions such as `elem_name`, but it must
    // never be used as a DOM mutation target.
    SyntheticFragmentContext,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ParserElementFlags {
    // html5ever passes this through ElementFlags and later asks the TreeSink for
    // it with the same parser handle while deciding whether MathML
    // annotation-xml children should switch back to HTML insertion rules.
    pub(super) mathml_annotation_xml_integration_point: bool,
}

impl ParserElementFlags {
    pub(super) fn from_html5ever(flags: &ElementFlags) -> Self {
        // Keep only parser-handle metadata here. `flags.template` creates
        // DOM-observable template contents and stays in the template handling
        // path instead of becoming parser-only state.
        Self {
            mathml_annotation_xml_integration_point: flags.mathml_annotation_xml_integration_point,
        }
    }
}

pub(super) struct DocumentSink {
    target: RefCell<ParserStreamHtmlTreeSinkTarget>,
    // html5ever exposes token lines but not columns. Once inserted input is
    // mixed into the tokenizer queue, it also cannot distinguish generated
    // lines from the original document tail, so location fidelity only
    // degrades and never recovers for this parser session.
    source_positions_known: Cell<bool>,
}

impl HtmlParser {
    pub fn parse(&self, final_url: Url, html: String) -> NativeDom {
        self.parse_dom_host(final_url, html).into_dom()
    }

    pub fn parse_dom_host(&self, final_url: Url, html: String) -> DomHost {
        let mut stream = self.start_document(final_url);
        for chunk in html_chunks(&html) {
            stream.feed(chunk);
        }
        stream.finish_dom_host()
    }

    pub fn parse_without_declarative_shadow_roots(
        &self,
        final_url: Url,
        html: String,
    ) -> NativeDom {
        let target =
            ParserStreamHtmlTreeSinkTarget::new_with_declarative_shadow_roots(final_url, false);
        let mut stream = HtmlTreeSinkStream::from_target(target);
        for chunk in html_chunks(&html) {
            stream.feed(chunk);
        }
        stream.finish()
    }

    pub fn start_document(&self, final_url: Url) -> DocumentStream {
        DocumentStream::new_parser_stream(final_url)
    }

    pub fn start_live_document_root(
        &self,
        final_url: Url,
        document_handle: NativeNodeId,
    ) -> DocumentStream {
        DocumentStream::new_live_document_root(final_url, document_handle)
    }

    pub fn parse_fragment(
        &self,
        final_url: Url,
        context_namespace: &str,
        context_local_name: &str,
        html: String,
    ) -> NativeDom {
        self.parse_fragment_with_declarative_shadow_roots(
            final_url,
            context_namespace,
            context_local_name,
            html,
            true,
            true,
        )
    }

    pub fn parse_fragment_without_declarative_shadow_roots(
        &self,
        final_url: Url,
        context_namespace: &str,
        context_local_name: &str,
        html: String,
    ) -> NativeDom {
        self.parse_fragment_without_declarative_shadow_roots_with_scripting(
            final_url,
            context_namespace,
            context_local_name,
            html,
            true,
        )
    }

    pub fn parse_fragment_without_declarative_shadow_roots_with_scripting(
        &self,
        final_url: Url,
        context_namespace: &str,
        context_local_name: &str,
        html: String,
        scripting_enabled: bool,
    ) -> NativeDom {
        self.parse_fragment_with_declarative_shadow_roots(
            final_url,
            context_namespace,
            context_local_name,
            html,
            false,
            scripting_enabled,
        )
    }

    fn parse_fragment_with_declarative_shadow_roots(
        &self,
        final_url: Url,
        context_namespace: &str,
        context_local_name: &str,
        html: String,
        allow_declarative_shadow_roots: bool,
        scripting_enabled: bool,
    ) -> NativeDom {
        let target = ParserStreamHtmlTreeSinkTarget::new_with_declarative_shadow_roots(
            final_url,
            allow_declarative_shadow_roots,
        );
        let context = QualName::new(
            None,
            Namespace::from(context_namespace),
            LocalName::from(context_local_name),
        );
        let context_handle = ParseHandle::new_synthetic_fragment_context(Rc::new(context));
        let sink = DocumentSink::new(target);
        let mut parser = HtmlParserSession::new_fragment(
            sink,
            html_parse_opts_with_scripting(scripting_enabled),
            context_handle,
            scripting_enabled,
        );

        for chunk in html_chunks(&html) {
            parser.process(StrTendril::from(chunk));
        }
        parser.finish().finish_document(html)
    }

    pub fn parse_fragment_dom_host(
        &self,
        final_url: Url,
        context_namespace: &str,
        context_local_name: &str,
        html: String,
    ) -> DomHost {
        let target = new_parser_stream_html_tree_sink_target(final_url);
        let context = QualName::new(
            None,
            Namespace::from(context_namespace),
            LocalName::from(context_local_name),
        );
        let context_handle = ParseHandle::new_synthetic_fragment_context(Rc::new(context));
        let sink = DocumentSink::new(target);
        let mut parser =
            HtmlParserSession::new_fragment(sink, html_parse_opts(), context_handle, true);

        for chunk in html_chunks(&html) {
            parser.process(StrTendril::from(chunk));
        }
        parser.finish().finish_dom_host()
    }
}

impl Default for ParserInputQueue {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(ParserInputState::default())))
    }
}

impl DocumentStream {
    fn new_parser_stream(final_url: Url) -> Self {
        Self {
            inner: new_parser_stream_html_tree_sink_stream(final_url),
            input: HtmlParserInputStream::default(),
        }
    }

    fn new_live_document_root(final_url: Url, document_handle: NativeNodeId) -> Self {
        Self {
            inner: new_live_document_root_html_tree_sink_stream(final_url, document_handle),
            input: HtmlParserInputStream::default(),
        }
    }

    fn new_live_fragment_root(
        final_url: Url,
        fragment_handle: NativeNodeId,
        context_handle: NativeNodeId,
        context_namespace: &str,
        context_local_name: &str,
        runtime_dom_sinks: ParserRuntimeDomSinks,
        allow_declarative_shadow_roots: bool,
    ) -> Self {
        Self {
            inner: new_live_fragment_root_html_tree_sink_stream(
                final_url,
                fragment_handle,
                context_handle,
                context_namespace,
                context_local_name,
                runtime_dom_sinks,
                allow_declarative_shadow_roots,
            ),
            input: HtmlParserInputStream::default(),
        }
    }

    pub fn is_parser_stream_backend_for_testing(&self) -> bool {
        true
    }

    pub fn new_parser_stream_for_testing(final_url: Url) -> Self {
        Self::new_parser_stream(final_url)
    }

    pub fn new_live_document_root_for_testing(
        final_url: Url,
        document_handle: NativeNodeId,
    ) -> Self {
        Self::new_live_document_root(final_url, document_handle)
    }

    pub fn new_live_fragment_root_for_testing<T>(
        final_url: Url,
        fragment_handle: NativeNodeId,
        context_handle: NativeNodeId,
        context_namespace: &str,
        context_local_name: &str,
        consumer: &mut T,
        allow_declarative_shadow_roots: bool,
    ) -> Self
    where
        T: ParserDomReadConsumer + ParserDomMutationConsumer + ParserMutationEffectConsumer,
    {
        // SAFETY: this constructor consumes the erased callbacks while `consumer`
        // is borrowed and clears them before returning the DocumentStream.
        let runtime_dom_sinks =
            unsafe { ParserRuntimeDomSinks::from_consumer_without_element_creation(consumer) };
        Self::new_live_fragment_root(
            final_url,
            fragment_handle,
            context_handle,
            context_namespace,
            context_local_name,
            runtime_dom_sinks,
            allow_declarative_shadow_roots,
        )
    }

    pub fn note_defined_autonomous_custom_element(&mut self, local_name: &str) {
        self.inner
            .note_defined_autonomous_custom_element(local_name);
    }

    pub fn drain_pending_custom_element_construction_handoffs(
        &mut self,
    ) -> Vec<ParserCustomElementConstructionHandoff> {
        self.inner
            .drain_pending_custom_element_construction_handoffs()
    }

    pub fn script_input_session(&self) -> ParserInputSession {
        self.inner.script_input_session()
    }

    pub fn take_next_script_input(&self) -> Option<String> {
        self.inner.take_next_script_input()
    }

    pub fn has_script_input(&self) -> bool {
        self.inner.has_script_input()
    }

    pub fn take_next_insertion_preload_input(&self) -> Option<String> {
        self.inner.take_next_insertion_preload_input()
    }

    pub fn take_processed_insertion_meta_csp_count(&self) -> usize {
        self.inner.take_processed_insertion_meta_csp_count()
    }

    pub fn feed(&mut self, chunk: &str) {
        self.inner.feed(chunk)
    }

    /// Append decoded document input to the parser-owned end segment chain.
    ///
    /// The tokenizer only receives a bounded prefix when the owner pumps the
    /// parser.  Appending while a script or stylesheet blocks parsing therefore
    /// cannot advance the DOM commit frontier.
    pub fn append_to_end(&mut self, chunk: String) {
        if !chunk.is_empty() {
            self.input.end_segments.push_back(chunk);
        }
    }

    /// Append input at the end of the currently active parser-inserted frame.
    ///
    /// This is used when a parser script has already inserted input and then
    /// continues writing while that input is blocked on a nested resource.
    /// The input must remain after the blocked frame's unconsumed tail rather
    /// than becoming a newer nested insertion.
    pub fn append_to_current_inserted_input(&mut self, chunk: &str) -> bool {
        self.inner.append_to_current_inserted_input(chunk)
    }

    pub fn has_pending_input(&self) -> bool {
        self.inner.has_script_input()
            || self.inner.has_buffered_input()
            || !self.input.end_segments.is_empty()
    }

    pub fn next_input_len(&self) -> usize {
        self.inner
            .next_script_input_len()
            .or_else(|| {
                self.inner
                    .has_buffered_input()
                    .then(|| self.inner.buffered_input_len())
            })
            .or_else(|| self.input.end_segments.front().map(String::len))
            .unwrap_or_default()
    }

    pub fn snapshot_pending_input(&self) -> String {
        let mut pending = self.inner.snapshot_script_input();
        pending.push_str(&self.inner.snapshot_buffered_input());
        for segment in &self.input.end_segments {
            pending.push_str(segment);
        }
        pending
    }

    pub fn queued_end_segment_count_for_testing(&self) -> usize {
        self.input.end_segments.len()
    }

    fn take_next_owned_input(&mut self, max_bytes: usize) -> (String, bool) {
        if let Some(input) = self.inner.take_next_script_input() {
            // One parser insertion is an atomic source segment. Splitting it
            // outside html5ever would let a remainder jump ahead of bytes the
            // tokenizer retained when it yielded on a script boundary.
            return (input, true);
        }

        if self.inner.has_buffered_input() {
            return (String::new(), false);
        }

        let Some(input) = self.input.end_segments.pop_front() else {
            return (String::new(), false);
        };
        let (prefix, remainder) = split_parser_input_prefix(input, max_bytes);
        if let Some(remainder) = remainder {
            self.input.end_segments.push_front(remainder);
        }
        (prefix, false)
    }

    pub fn pump_next_parser_step(&mut self, max_bytes: usize) -> ParserPumpOutcome {
        let (chunk, inserted_source) = self.take_next_owned_input(max_bytes);
        if inserted_source {
            self.inner.pump_parser_inserted_step(&chunk)
        } else {
            self.inner.pump_parser_step(&chunk)
        }
    }

    /// Feed parser input until either the current buffer is exhausted or the parser yields a
    /// concrete embedder control-flow boundary.
    ///
    /// This is the low-level surface the runtime needs for parser/script coordination:
    /// instead of slicing the original HTML string on `</script>` boundaries and hoping the next
    /// parser step will line up with script readiness, we drive html5ever one tokenizer run at a
    /// time and stop when the tree builder returns `TokenizerResult::Script(handle)` or when
    /// Moli-side parser state records another parser yield reason.
    ///
    /// Important constraints:
    /// - this method only exposes *when parser control should be yielded back to runtime*
    /// - it does not execute JS itself
    /// - any actual V8/isolate work still happens in the runtime coordination layer
    ///
    /// Returning `ParserPumpStep::Yield(reason)` means the parser has stopped before consuming
    /// following tokens. Script and custom-element yields transfer work to the runtime; a
    /// blocking-stylesheet yield only asks the runtime to retain and later resume this parser.
    /// No stylesheet ownership is transferred.
    ///
    /// Returning `ParserPumpStep::InputDrained` means:
    /// - the current input buffer has been consumed as far as html5ever can go for now
    /// - either there is no pending yield, or more bytes are needed before another boundary exists
    pub fn pump_parser_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        self.inner.pump_parser_step(chunk)
    }

    #[cfg(test)]
    pub(crate) fn pump_parser_inserted_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        self.inner.pump_parser_inserted_step(chunk)
    }

    pub fn pump_parser_step_with_runtime_dom_consumer<T>(
        &mut self,
        chunk: &str,
        consumer: &mut T,
    ) -> ParserPumpOutcome
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        // SAFETY: `consumer` stays exclusively borrowed for this call; the
        // parser-step Drop guard removes every erased callback before return.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumer(consumer) };
        self.pump_parser_step_with_runtime_dom_sinks(chunk, sinks)
    }

    pub fn pump_next_parser_step_with_runtime_dom_consumer<T>(
        &mut self,
        max_bytes: usize,
        consumer: &mut T,
    ) -> ParserPumpOutcome
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        // SAFETY: `consumer` stays exclusively borrowed for this call; the
        // parser-step Drop guard removes every erased callback before return.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumer(consumer) };
        self.inner.enter_runtime_dom_sinks_parse_step(sinks);
        let mut step = RuntimeDomSinksParserStep { stream: self };
        step.pump_next_parser_step(max_bytes)
    }

    pub fn pump_parser_step_with_runtime_dom_consumer_without_element_creation<T>(
        &mut self,
        chunk: &str,
        consumer: &mut T,
    ) -> ParserPumpOutcome
    where
        T: ParserDomReadConsumer + ParserDomMutationConsumer + ParserMutationEffectConsumer,
    {
        // SAFETY: `consumer` stays exclusively borrowed for this call; the
        // parser-step Drop guard removes every erased callback before return.
        let sinks =
            unsafe { ParserRuntimeDomSinks::from_consumer_without_element_creation(consumer) };
        self.pump_parser_step_with_runtime_dom_sinks(chunk, sinks)
    }

    pub fn pump_parser_step_with_runtime_dom_consumers<T, E>(
        &mut self,
        chunk: &str,
        consumer: &mut T,
        element_consumer: &mut E,
    ) -> ParserPumpOutcome
    where
        T: ParserDomReadConsumer + ParserDomMutationConsumer + ParserMutationEffectConsumer,
        E: ParserElementCreationConsumer,
    {
        // SAFETY: both consumers stay exclusively and independently borrowed
        // until the parser-step Drop guard removes every erased callback.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumers(consumer, element_consumer) };
        self.pump_parser_step_with_runtime_dom_sinks(chunk, sinks)
    }

    fn pump_parser_step_with_runtime_dom_sinks(
        &mut self,
        chunk: &str,
        sinks: ParserRuntimeDomSinks,
    ) -> ParserPumpOutcome {
        self.inner.enter_runtime_dom_sinks_parse_step(sinks);
        let mut step = RuntimeDomSinksParserStep { stream: self };
        step.pump_parser_step(chunk)
    }

    pub fn pump_parser_inserted_step_with_runtime_dom_consumer<T>(
        &mut self,
        chunk: &str,
        consumer: &mut T,
    ) -> ParserPumpOutcome
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        // SAFETY: `consumer` stays exclusively borrowed for this call; the
        // parser-step Drop guard removes every erased callback before return.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumer(consumer) };
        self.pump_parser_inserted_step_with_runtime_dom_sinks(chunk, sinks)
    }

    fn pump_parser_inserted_step_with_runtime_dom_sinks(
        &mut self,
        chunk: &str,
        sinks: ParserRuntimeDomSinks,
    ) -> ParserPumpOutcome {
        self.inner.enter_runtime_dom_sinks_parse_step(sinks);
        let mut step = RuntimeDomSinksParserStep { stream: self };
        step.pump_parser_inserted_step(chunk)
    }

    pub fn finish_with_runtime_dom_consumer<T>(
        self,
        consumer: &mut T,
    ) -> ParserFinishDiscoverySignals
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        // SAFETY: `consumer` stays exclusively borrowed for this call. A normal
        // finish consumes the parser and its bundle; the Drop guard clears the
        // bundle if finish unwinds before consumption.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumer(consumer) };
        self.finish_with_runtime_dom_sinks(sinks)
    }

    fn finish_with_runtime_dom_sinks(
        mut self,
        sinks: ParserRuntimeDomSinks,
    ) -> ParserFinishDiscoverySignals {
        self.inner.enter_runtime_dom_sinks_parse_step(sinks);
        let mut finish = RuntimeDomSinksParserFinish { stream: Some(self) };
        finish.finish()
    }

    pub fn with_stylesheet_blocking_read_view<R>(
        &self,
        f: impl FnOnce(&dyn moli_stylesheet_blocking::StylesheetBlockingReadView) -> R,
    ) -> R {
        self.inner.with_stylesheet_blocking_read_view(f)
    }

    pub fn snapshot_parser_stream_document(&self) -> ParserStreamDocumentSnapshot {
        ParserStreamDocumentSnapshot(self.inner.snapshot_parser_stream_document())
    }

    pub fn snapshot_parser_stream_dom_host(&self) -> DomHost {
        self.inner.snapshot_parser_stream_dom_host()
    }

    pub fn take_parser_stream_null_custom_element_registry_elements(
        &mut self,
    ) -> Vec<NativeNodeId> {
        self.inner
            .take_parser_stream_null_custom_element_registry_elements()
    }

    pub fn take_parser_stream_dom_host(&mut self) -> DomHost {
        self.inner.take_parser_stream_dom_host()
    }

    pub fn restore_parser_stream_dom_host(&mut self, dom_host: DomHost) {
        self.inner.restore_parser_stream_dom_host(dom_host);
    }

    pub fn with_parser_stream_dom_host_for_bootstrap<R>(
        &mut self,
        f: impl FnOnce(DomHost) -> std::result::Result<R, Box<(anyhow::Error, DomHost)>>,
    ) -> anyhow::Result<R> {
        let bootstrap_document = self.inner.take_parser_stream_dom_host();
        match f(bootstrap_document) {
            Ok(result) => Ok(result),
            Err(error) => {
                let (error, bootstrap_document) = *error;
                self.inner
                    .restore_parser_stream_dom_host(bootstrap_document);
                Err(error)
            }
        }
    }

    pub fn replace_parser_stream_document_from_snapshot(
        &mut self,
        document: ParserStreamDocumentSnapshot,
    ) {
        self.inner.replace_parser_stream_document(document.into())
    }

    pub fn drain_ready_parser_scripts(&mut self) -> Vec<NativeNodeId> {
        self.inner.drain_ready_parser_scripts()
    }

    pub fn drain_discovered_async_prefetch_candidates(&mut self) -> Vec<NativeNodeId> {
        self.inner.drain_discovered_async_prefetch_candidates()
    }

    pub fn drain_discovered_modulepreload_link_candidates(&mut self) -> Vec<NativeNodeId> {
        self.inner.drain_discovered_modulepreload_link_candidates()
    }

    pub fn drain_discovered_parser_meta_csp_candidates(&mut self) -> Vec<NativeNodeId> {
        self.inner.drain_discovered_parser_meta_csp_candidates()
    }

    pub fn mark_script_already_started(&mut self, node_id: NativeNodeId) {
        // The streaming parser and the live runtime intentionally share DOM snapshots during
        // parse-time execution. When runtime code claims ownership of a parser-discovered script
        // without executing it immediately (phase 2 `defer` / external `async`), we still need
        // the parser-side DOM to remember that claim. Otherwise `finish()` would hand back a
        // snapshot where the same script still looks fresh, and the later whole-document planner
        // would rediscover and execute it a second time.
        self.inner.mark_script_already_started(node_id)
    }

    pub fn finish(self) -> NativeDom {
        self.inner.finish()
    }

    pub fn finish_dom_host(self) -> DomHost {
        self.inner.finish_dom_host()
    }
}

struct RuntimeDomSinksParserStep<'a> {
    stream: &'a mut DocumentStream,
}

impl RuntimeDomSinksParserStep<'_> {
    fn pump_parser_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        self.stream.pump_parser_step(chunk)
    }

    fn pump_parser_inserted_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        self.stream.inner.pump_parser_inserted_step(chunk)
    }

    fn pump_next_parser_step(&mut self, max_bytes: usize) -> ParserPumpOutcome {
        self.stream.pump_next_parser_step(max_bytes)
    }
}

fn split_parser_input_prefix(input: String, max_bytes: usize) -> (String, Option<String>) {
    if max_bytes == 0 || input.len() <= max_bytes {
        return (input, None);
    }

    let mut split = input.len();
    for (index, character) in input.char_indices() {
        let end = index + character.len_utf8();
        if end > max_bytes {
            split = if index == 0 { end } else { index };
            break;
        }
    }
    let remainder = input[split..].to_owned();
    let prefix = input[..split].to_owned();
    (prefix, (!remainder.is_empty()).then_some(remainder))
}

impl Drop for RuntimeDomSinksParserStep<'_> {
    fn drop(&mut self) {
        self.stream.inner.clear_runtime_dom_sinks_after_parse_step();
    }
}

struct RuntimeDomSinksParserFinish {
    stream: Option<DocumentStream>,
}

impl RuntimeDomSinksParserFinish {
    fn finish(&mut self) -> ParserFinishDiscoverySignals {
        if let Some(stream) = self.stream.take() {
            stream.inner.finish_live_runtime_dom_sink_parser()
        } else {
            ParserFinishDiscoverySignals::default()
        }
    }
}

impl Drop for RuntimeDomSinksParserFinish {
    fn drop(&mut self) {
        if let Some(stream) = &mut self.stream {
            stream.inner.clear_runtime_dom_sinks_after_parse_step();
        }
    }
}

impl ParserStreamDocumentSnapshot {
    pub fn into_document(self) -> NativeDom {
        self.0
    }

    pub fn nodes(&self) -> NativeDomNodes<'_> {
        self.0.nodes()
    }

    pub fn final_url(&self) -> Option<&url::Url> {
        self.0.final_url()
    }

    pub fn document_base_url(&self) -> Option<url::Url> {
        self.0
            .document()
            .map(|document| document.base_url().clone())
    }

    pub fn parse_errors(&self) -> &[String] {
        self.0.parse_errors()
    }

    pub fn document_node_id(&self) -> NativeNodeId {
        self.0.document_node_id()
    }

    pub fn document_body_handle(&self) -> Option<NativeNodeId> {
        self.0.document_body_handle()
    }

    pub fn node(&self, node_id: NativeNodeId) -> Option<&Node> {
        self.0.node(node_id)
    }

    pub fn child_ids(&self, node_id: NativeNodeId) -> impl Iterator<Item = NativeNodeId> + '_ {
        self.0.child_ids(node_id)
    }

    pub fn stylesheet_candidate_handles_before_in_tree_scope(
        &self,
        tree_scope: NativeNodeId,
        stop_at: Option<NativeNodeId>,
    ) -> Vec<NativeNodeId> {
        self.0
            .stylesheet_candidate_handles_before_in_tree_scope(tree_scope, stop_at)
    }

    pub fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.0.text_content(node_id)
    }

    pub fn elements_by_tag_name(
        &self,
        root: NativeNodeId,
        tag_name: &str,
        include_root: bool,
    ) -> Vec<NativeNodeId> {
        self.0.elements_by_tag_name(root, tag_name, include_root)
    }

    pub fn script_handles(&self) -> Vec<NativeNodeId> {
        self.0.script_handles()
    }

    pub fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        self.0.document_order_script_handles()
    }

    pub fn script_src(&self, node_id: NativeNodeId) -> Option<&str> {
        self.0.script_src(node_id)
    }

    pub fn script_text(&self, node_id: NativeNodeId) -> Option<String> {
        self.0.script_text(node_id)
    }

    pub fn node_is_parser_created(&self, node_id: NativeNodeId) -> bool {
        self.0
            .node(node_id)
            .is_some_and(|node| node.flags().parser_created())
    }

    pub fn mark_script_already_started(&mut self, node_id: NativeNodeId) -> bool {
        self.0
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| element.set_script_already_started(true))
    }
}

impl From<NativeDom> for ParserStreamDocumentSnapshot {
    fn from(document: NativeDom) -> Self {
        Self(document)
    }
}

impl From<ParserStreamDocumentSnapshot> for NativeDom {
    fn from(snapshot: ParserStreamDocumentSnapshot) -> Self {
        snapshot.0
    }
}

impl ParserInputQueue {
    pub fn session(&self) -> ParserInputSession {
        ParserInputSession(self.0.clone())
    }

    pub fn take_next_script_input(&self) -> Option<String> {
        self.0
            .borrow_mut()
            .script_input_queue
            .pop_front()
            .filter(|html| !html.is_empty())
    }

    pub fn next_script_input_len(&self) -> Option<usize> {
        self.0.borrow().script_input_queue.front().map(String::len)
    }

    pub fn snapshot_script_input(&self) -> String {
        self.0
            .borrow()
            .script_input_queue
            .iter()
            .fold(String::new(), |mut input, segment| {
                input.push_str(segment);
                input
            })
    }

    pub fn has_script_input(&self) -> bool {
        !self.0.borrow().script_input_queue.is_empty()
    }

    pub fn take_next_insertion_preload_input(&self) -> Option<String> {
        self.0
            .borrow_mut()
            .insertion_preload_queue
            .pop_front()
            .filter(|html| !html.is_empty())
    }

    pub fn take_processed_insertion_meta_csp_count(&self) -> usize {
        std::mem::take(&mut self.0.borrow_mut().processed_insertion_meta_csp_count)
    }
}

impl ParserInputSession {
    pub fn enqueue_script_input_html(&self, html: String) {
        if html.is_empty() {
            return;
        }
        let mut state = self.0.borrow_mut();
        if let Some(tail) = state.script_input_queue.back_mut() {
            tail.push_str(&html);
        } else {
            state.script_input_queue.push_back(html);
        }
    }

    pub fn take_next_script_input_html(&self) -> Option<String> {
        self.0
            .borrow_mut()
            .script_input_queue
            .pop_front()
            .filter(|html| !html.is_empty())
    }

    pub fn enter_pending_context(&self) -> ParserInputContext {
        self.0.borrow_mut().pending_stack.push(String::new());
        ParserInputContext {
            session: self.clone(),
        }
    }

    pub fn enqueue_script_input_preload_html(&self, html: String) {
        if html.is_empty() {
            return;
        }
        let mut state = self.0.borrow_mut();
        if let Some(tail) = state.insertion_preload_queue.back_mut() {
            tail.push_str(&html);
        } else {
            state.insertion_preload_queue.push_back(html);
        }
    }

    pub fn note_processed_insertion_meta_csp(&self, count: usize) {
        let mut state = self.0.borrow_mut();
        state.processed_insertion_meta_csp_count = state
            .processed_insertion_meta_csp_count
            .saturating_add(count);
    }

    pub fn take_current_script_input_html(&self) -> String {
        self.0
            .borrow_mut()
            .pending_stack
            .last_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    pub fn set_current_script_input_html(&self, html: String) {
        let mut state = self.0.borrow_mut();
        let Some(current) = state.pending_stack.last_mut() else {
            return;
        };
        *current = html;
    }

    fn flush_and_pop_pending_context(&self) {
        let mut state = self.0.borrow_mut();
        let pending = state.pending_stack.pop().unwrap_or_default();
        if pending.is_empty() {
            return;
        }
        if let Some(tail) = state.script_input_queue.back_mut() {
            tail.push_str(&pending);
        } else {
            state.script_input_queue.push_back(pending);
        }
    }
}

impl ParserInputContext {
    pub fn session(&self) -> ParserInputSession {
        self.session.clone()
    }
}

impl Drop for ParserInputContext {
    fn drop(&mut self) {
        self.session.flush_and_pop_pending_context();
    }
}

impl ParseHandle {
    pub(super) fn new(node_id: NativeNodeId, element_name: Option<Rc<QualName>>) -> Self {
        Self {
            identity: ParseHandleIdentity::DomNode(node_id),
            element_name,
            parser_flags: ParserElementFlags::default(),
        }
    }

    pub(super) fn new_synthetic_fragment_context(element_name: Rc<QualName>) -> Self {
        Self {
            identity: ParseHandleIdentity::SyntheticFragmentContext,
            element_name: Some(element_name),
            parser_flags: ParserElementFlags::default(),
        }
    }

    pub(super) fn new_element(
        node_id: NativeNodeId,
        element_name: Rc<QualName>,
        parser_flags: ParserElementFlags,
    ) -> Self {
        Self {
            identity: ParseHandleIdentity::DomNode(node_id),
            element_name: Some(element_name),
            parser_flags,
        }
    }

    pub(super) fn dom_node_id(&self) -> Option<NativeNodeId> {
        match self.identity {
            ParseHandleIdentity::DomNode(node_id) => Some(node_id),
            ParseHandleIdentity::SyntheticFragmentContext => None,
        }
    }

    fn is_script_element(&self) -> bool {
        self.element_name.as_ref().is_some_and(|name| {
            name.local.as_ref() == "script"
                && matches!(
                    name.ns.as_ref(),
                    "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
                )
        })
    }

    pub(super) fn node_id(&self) -> NativeNodeId {
        self.dom_node_id()
            .expect("parser operation requires a real DOM node handle")
    }
}

impl PartialEq for ParseHandle {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for ParseHandle {}

impl DocumentSink {
    pub(super) fn mark_source_positions_unknown(&self) {
        self.source_positions_known.set(false);
        let unknown = ParserSourcePosition::UNKNOWN;
        self.target
            .borrow_mut()
            .set_current_position(unknown.line, unknown.column);
    }

    fn mutate_target(
        &self,
        mutation: impl FnOnce(&mut ParserStreamHtmlTreeSinkTarget) -> ParserMutationEffectDelivery,
    ) {
        let delivery = {
            let mut target = self.target.borrow_mut();
            mutation(&mut target)
        };
        delivery.consume();
    }

    pub(super) fn new(target: ParserStreamHtmlTreeSinkTarget) -> Self {
        Self {
            target: RefCell::new(target),
            source_positions_known: Cell::new(true),
        }
    }

    pub(super) fn snapshot_parser_stream_document(&self) -> NativeDom {
        self.target.borrow().snapshot_parser_stream_document()
    }

    pub(super) fn snapshot_parser_stream_dom_host(&self) -> DomHost {
        self.target.borrow().snapshot_parser_stream_dom_host()
    }

    pub(super) fn take_parser_stream_null_custom_element_registry_elements(
        &self,
    ) -> Vec<NativeNodeId> {
        self.target
            .borrow_mut()
            .take_parser_stream_null_custom_element_registry_elements()
    }

    pub(super) fn take_parser_stream_dom_host(&self) -> DomHost {
        self.target.borrow_mut().take_parser_stream_document()
    }

    pub(super) fn restore_parser_stream_dom_host(&self, dom_host: DomHost) {
        self.target
            .borrow_mut()
            .restore_parser_stream_dom_host(dom_host);
    }

    pub(super) fn enter_runtime_dom_sinks_parse_step(&self, sinks: ParserRuntimeDomSinks) {
        self.target
            .borrow_mut()
            .enter_runtime_dom_sinks_parse_step(sinks);
    }

    pub(super) fn clear_runtime_dom_sinks_after_parse_step(&self) {
        self.target
            .borrow_mut()
            .clear_runtime_dom_sinks_after_parse_step()
    }

    pub(super) fn borrow_target(&self) -> Ref<'_, ParserStreamHtmlTreeSinkTarget> {
        self.target.borrow()
    }

    pub(super) fn replace_parser_stream_document(&self, document: NativeDom) {
        self.target
            .borrow_mut()
            .replace_parser_stream_document(document);
    }

    pub(super) fn drain_ready_parser_scripts(&self) -> Vec<NativeNodeId> {
        self.target.borrow_mut().drain_ready_parser_scripts()
    }

    pub(super) fn drain_discovered_async_prefetch_candidates(&self) -> Vec<NativeNodeId> {
        self.target
            .borrow_mut()
            .drain_discovered_async_prefetch_candidates()
    }

    pub(super) fn drain_discovered_modulepreload_link_candidates(&self) -> Vec<NativeNodeId> {
        self.target
            .borrow_mut()
            .drain_discovered_modulepreload_link_candidates()
    }

    pub(super) fn drain_discovered_parser_meta_csp_candidates(&self) -> Vec<NativeNodeId> {
        self.target
            .borrow_mut()
            .drain_discovered_parser_meta_csp_candidates()
    }

    pub(super) fn note_defined_autonomous_custom_element(&self, local_name: &str) {
        self.target
            .borrow_mut()
            .note_defined_autonomous_custom_element(local_name);
    }

    pub(super) fn drain_pending_custom_element_construction_handoffs(
        &self,
    ) -> Vec<ParserCustomElementConstructionHandoff> {
        self.target
            .borrow_mut()
            .drain_pending_custom_element_construction_handoffs()
    }

    pub(super) fn has_pending_custom_element_construction_handoff(&self) -> bool {
        self.target
            .borrow()
            .has_pending_custom_element_construction_handoff()
    }

    pub(super) fn pending_custom_element_construction_handoff_placeholder(
        &self,
    ) -> Option<NativeNodeId> {
        self.target
            .borrow()
            .front_pending_custom_element_construction_handoff()
            .map(|handoff| handoff.placeholder)
    }

    pub(super) fn pop_pending_custom_element_construction_handoff(
        &self,
    ) -> Option<ParserCustomElementConstructionHandoff> {
        self.target
            .borrow_mut()
            .pop_pending_custom_element_construction_handoff()
    }

    pub(super) fn pending_blocking_stylesheet_pause(&self) -> Option<NativeNodeId> {
        self.target
            .borrow()
            .front_pending_blocking_stylesheet_pause()
    }

    pub(super) fn pop_pending_blocking_stylesheet_pause(&self) -> Option<NativeNodeId> {
        self.target
            .borrow_mut()
            .pop_pending_blocking_stylesheet_pause()
    }

    pub(super) fn begin_tree_builder_finish(&self) {
        self.target.borrow_mut().begin_tree_builder_finish();
    }

    pub(super) fn drain_discovered_blocking_stylesheet_inputs(
        &self,
    ) -> Vec<DocumentOwnedBlockingStylesheetDiscoveryInput> {
        self.target
            .borrow_mut()
            .drain_discovered_blocking_stylesheet_inputs()
    }

    pub(super) fn captured_blocking_stylesheet_signatures(
        &self,
    ) -> HashSet<DocumentBlockingStylesheetSignature> {
        self.target
            .borrow()
            .captured_blocking_stylesheet_signatures()
    }

    pub(super) fn note_foreign_end_tag_processed(&self, local_name: &str) -> Option<NativeNodeId> {
        self.target
            .borrow_mut()
            .note_foreign_end_tag_processed(local_name)
    }

    pub(super) fn note_self_closing_foreign_element_processed(
        &self,
        local_name: &str,
    ) -> Option<NativeNodeId> {
        self.target
            .borrow_mut()
            .note_self_closing_foreign_element_processed(local_name)
    }

    pub(super) fn mark_script_already_started(&self, node_id: NativeNodeId) {
        self.target
            .borrow_mut()
            .mark_script_already_started(node_id);
    }
}

impl TreeSink for DocumentSink {
    type Handle = ParseHandle;
    type Output = ParserStreamHtmlTreeSinkTarget;
    type ElemName<'a>
        = &'a QualName
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        self.target.into_inner()
    }

    fn parse_error(&self, err: Cow<'static, str>) {
        self.target.borrow_mut().push_parse_error(err.into_owned());
    }

    fn get_document(&self) -> Self::Handle {
        self.target.borrow().document_handle()
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.target.borrow_mut().set_html_quirks_mode(mode);
    }

    fn same_node(&self, left: &Self::Handle, right: &Self::Handle) -> bool {
        left == right
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        target
            .element_name
            .as_deref()
            .expect("html5ever requested the name of a non-element node")
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &Self::Handle) -> bool {
        target.parser_flags.mathml_annotation_xml_integration_point
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        self.target.borrow_mut().create_element(name, attrs, flags)
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.target.borrow_mut().create_comment(text.to_string())
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.target
            .borrow_mut()
            .create_processing_instruction(target.to_string(), data.to_string())
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        self.mutate_target(|target| target.append(parent.node_id(), child));
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, child: NodeOrText<Self::Handle>) {
        self.mutate_target(|target| target.append_before_sibling(sibling.node_id(), child));
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        self.mutate_target(|target| {
            target.append_based_on_parent_node(element.node_id(), prev_element.node_id(), child)
        });
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.mutate_target(|target| {
            target.append_doctype(
                name.to_string(),
                public_id.to_string(),
                system_id.to_string(),
            )
        });
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        target
            .dom_node_id()
            .and_then(|node_id| self.target.borrow_mut().template_contents_handle(node_id))
            .unwrap_or_else(|| target.clone())
    }

    fn attach_declarative_shadow(
        &self,
        location: &Self::Handle,
        template: &Self::Handle,
        attrs: &[Attribute],
    ) -> bool {
        let Some(location_id) = location.dom_node_id() else {
            return false;
        };
        let Some(template_id) = template.dom_node_id() else {
            return false;
        };
        self.target
            .borrow_mut()
            .attach_declarative_shadow(location_id, template_id, attrs)
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        if let Some(target_id) = target.dom_node_id() {
            self.target
                .borrow_mut()
                .add_attrs_if_missing(target_id, attrs);
        }
    }

    fn associate_with_form(
        &self,
        target: &Self::Handle,
        form: &Self::Handle,
        _nodes: (&Self::Handle, Option<&Self::Handle>),
    ) {
        let Some(target_id) = target.dom_node_id() else {
            return;
        };
        let Some(form_id) = form.dom_node_id() else {
            return;
        };
        self.target
            .borrow_mut()
            .associate_with_form(target_id, form_id);
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        if let Some(target_id) = target.dom_node_id() {
            self.mutate_target(|sink| sink.remove_from_parent(target_id));
        }
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let Some(node_id) = node.dom_node_id() else {
            return;
        };
        let Some(new_parent_id) = new_parent.dom_node_id() else {
            return;
        };
        self.mutate_target(|target| target.reparent_children(node_id, new_parent_id));
    }

    fn mark_script_already_started(&self, node: &Self::Handle) {
        // html5ever may mark a script "already started" as part of its tree-builder state
        // machine. Keep that flag in the backing DOM as well so any snapshot handed to later
        // runtime/planning code preserves the same script bookkeeping contract.
        if let Some(node_id) = node.dom_node_id() {
            self.target
                .borrow_mut()
                .mark_script_already_started(node_id);
        }
    }

    fn pop(&self, node: &Self::Handle) {
        // html5ever calls `pop()` when the element has been fully closed by the parser. For
        // classic parser-inserted scripts that is the earliest safe moment to hand execution back
        // to the runtime without risking partial inline source or a half-built element subtree.
        if let Some(node_id) = node.dom_node_id() {
            self.target
                .borrow_mut()
                .note_node_closed(node_id, node.is_script_element());
        }
    }

    fn set_current_line(&self, line_number: u64) {
        let mut target = self.target.borrow_mut();
        if self.source_positions_known.get() {
            let position = ParserSourcePosition::line_only(line_number);
            target.set_current_position(position.line, position.column);
        } else {
            let unknown = ParserSourcePosition::UNKNOWN;
            target.set_current_position(unknown.line, unknown.column);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::{HtmlParser, ParseHandle, ParserInputQueue};
    use html5ever::{LocalName, Namespace, QualName};
    use moli_dom::native::{NativeDom, NativeNodeId};
    use url::Url;

    const HTML_NS: &str = "http://www.w3.org/1999/xhtml";
    const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";

    fn parse_test_document(html: &str) -> NativeDom {
        HtmlParser.parse(
            Url::parse("https://example.test/").expect("test url"),
            html.to_owned(),
        )
    }

    fn first_element_by_ns(
        document: &NativeDom,
        namespace: &str,
        local_name: &str,
    ) -> NativeNodeId {
        document
            .elements_by_tag_name_ns(
                document.document_node_id(),
                Some(namespace),
                local_name,
                true,
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("expected {namespace} {local_name} element"))
    }

    #[test]
    fn standalone_fragment_context_handle_is_not_the_document_handle() {
        let document = ParseHandle::new(NativeNodeId::new(0), None);
        let context = Rc::new(QualName::new(
            None,
            Namespace::from(HTML_NS),
            LocalName::from("body"),
        ));
        let fragment_context = ParseHandle::new_synthetic_fragment_context(context);

        assert_ne!(fragment_context, document);
        assert_eq!(fragment_context.dom_node_id(), None);
        assert_eq!(document.dom_node_id(), Some(NativeNodeId::new(0)));
        assert_eq!(
            fragment_context
                .element_name
                .as_deref()
                .map(|name| name.local.as_ref()),
            Some("body")
        );
    }

    #[test]
    fn template_contents_do_not_publish_document_stylesheet_candidates() {
        let document = HtmlParser.parse_dom_host(
            Url::parse("https://template-styles.test/page.html").expect("test URL"),
            concat!(
                "<!doctype html>",
                "<template>",
                "<link rel='stylesheet' href='/inside.css'>",
                "<style>@import url('/inside-import.css');</style>",
                "</template>",
            )
            .to_owned(),
        );
        assert!(
            document
                .stylesheet_candidate_handles_for_tree_scope(document.document_node_id())
                .is_empty(),
            "inert template contents must not become Document stylesheet candidates"
        );
    }

    #[test]
    fn mathml_annotation_xml_text_html_is_html_integration_point() {
        let document = parse_test_document(concat!(
            "<!doctype html>",
            "<math><annotation-xml encoding='text/html'><div></div></annotation-xml></math>"
        ));
        let annotation = first_element_by_ns(&document, MATHML_NS, "annotation-xml");
        let div = first_element_by_ns(&document, HTML_NS, "div");

        assert_eq!(
            document.node(div).and_then(|node| node.parent_node_id()),
            Some(annotation),
            "HTML integration point children should remain under annotation-xml"
        );
    }

    #[test]
    fn mathml_annotation_xml_without_html_encoding_is_not_integration_point() {
        let document = parse_test_document(concat!(
            "<!doctype html>",
            "<div><math><annotation-xml><p></p></annotation-xml></math></div>"
        ));
        let annotation = first_element_by_ns(&document, MATHML_NS, "annotation-xml");
        let paragraph = first_element_by_ns(&document, HTML_NS, "p");

        assert_ne!(
            document
                .node(paragraph)
                .and_then(|node| node.parent_node_id()),
            Some(annotation),
            "plain annotation-xml must not opt into HTML integration point parsing"
        );
    }
    #[test]
    fn parser_input_session_keeps_nested_pending_buffers_on_a_stack() {
        let queue = ParserInputQueue::default();
        let session = queue.session();

        let outer = session.enter_pending_context();
        session.set_current_script_input_html("<scr".to_owned());

        let inner = session.enter_pending_context();
        session.set_current_script_input_html("<div>inner".to_owned());
        assert_eq!(session.take_current_script_input_html(), "<div>inner");
        session.set_current_script_input_html("<div>inner".to_owned());
        drop(inner);

        assert_eq!(
            queue.take_next_script_input().as_deref(),
            Some("<div>inner")
        );
        assert_eq!(outer.session().take_current_script_input_html(), "<scr");
        outer
            .session()
            .set_current_script_input_html("<script>outer</script>".to_owned());
        drop(outer);

        assert_eq!(
            queue.take_next_script_input().as_deref(),
            Some("<script>outer</script>")
        );
    }

    #[test]
    fn parser_input_session_keeps_insertion_preload_html_separate_from_script_input() {
        let queue = ParserInputQueue::default();
        let session = queue.session();

        session.enqueue_script_input_html("<script>script-input</script>".to_owned());
        session.enqueue_script_input_preload_html("<scr".to_owned());
        session.enqueue_script_input_preload_html("ipt src=\"/write.js\"></script>".to_owned());

        assert_eq!(
            queue.take_next_insertion_preload_input().as_deref(),
            Some("<script src=\"/write.js\"></script>")
        );
        assert_eq!(
            queue.take_next_script_input().as_deref(),
            Some("<script>script-input</script>")
        );
    }

    #[test]
    fn parser_input_session_transfers_insertion_meta_csp_acknowledgements_once() {
        let queue = ParserInputQueue::default();
        let session = queue.session();

        session.note_processed_insertion_meta_csp(1);
        session.note_processed_insertion_meta_csp(2);

        assert_eq!(queue.take_processed_insertion_meta_csp_count(), 3);
        assert_eq!(queue.take_processed_insertion_meta_csp_count(), 0);
    }
}
