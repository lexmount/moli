use crate::{
    DocumentOwnedBlockingStylesheetDiscoveryInput,
    dom::native::{DomHost, NativeNodeId},
    parser::{
        DocumentStream, HtmlParser, ParserBlockingStylesheetPause,
        ParserCustomElementConstructionHandoff, ParserDomMutationConsumer, ParserDomReadConsumer,
        ParserElementCreationConsumer, ParserMutationEffectConsumer, ParserPumpOutcome,
        ParserPumpStep, ParserScriptHandoff, ParserYield, PreparedScript, XmlDocumentStream,
    },
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use url::Url;

pub(crate) type DocumentParserStreamHandle = Rc<RefCell<DocumentStream>>;
type XmlDocumentParserStreamHandle = Rc<RefCell<XmlDocumentStream>>;

fn new_document_parser_stream_handle(stream: DocumentStream) -> DocumentParserStreamHandle {
    Rc::new(RefCell::new(stream))
}

pub(crate) trait LiveDocumentParserOwner:
    ParserDomReadConsumer
    + ParserDomMutationConsumer
    + ParserMutationEffectConsumer
    + ParserElementCreationConsumer
{
}

#[cfg(test)]
fn pump_live_document_parser_step(
    stream: &mut DocumentStream,
    chunk: &str,
    owner: &mut impl LiveDocumentParserOwner,
) -> ParserPumpOutcome {
    stream.pump_parser_step_with_runtime_dom_consumer(chunk, owner)
}

fn pump_next_live_document_parser_step(
    stream: &mut DocumentStream,
    max_bytes: usize,
    owner: &mut impl LiveDocumentParserOwner,
) -> ParserPumpOutcome {
    stream.pump_next_parser_step_with_runtime_dom_consumer(max_bytes, owner)
}

pub(crate) enum LiveDocumentParserStepOutcome {
    /// The current parser input boundary was reached without producing a
    /// synchronous parser lifecycle handoff. Parser-owned parent input may
    /// already have been restored and still require another step.
    InputBoundary,
    /// The tree builder needs author-defined custom element construction before
    /// parser insertion can continue in the same document parser.
    CustomElementConstructionHandoff(Box<ParserCustomElementConstructionHandoff>),
    /// A parser-created blocking stylesheet in the body pauses token
    /// consumption while the owner keeps the same parser and buffered input.
    BlockingStylesheetPause(ParserBlockingStylesheetPause),
    /// The tree builder reached a parser-connected script boundary. The owner
    /// decides whether this executes immediately or blocks on source/resources.
    ScriptHandoff(Box<ParserScriptHandoff>),
}

struct LiveDocumentParserStepAdvance {
    outcome: LiveDocumentParserStepOutcome,
    discovery_signals: LiveDocumentParserDiscoverySignals,
}

#[derive(Debug, Default)]
pub(crate) struct LiveDocumentParserDiscoverySignals {
    pub(crate) async_prefetch_scripts: Vec<PreparedScript>,
    pub(crate) modulepreload_link_candidates: Vec<NativeNodeId>,
    pub(crate) parser_meta_csp_candidates: Vec<NativeNodeId>,
    pub(crate) blocking_stylesheet_inputs: Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
}

pub(crate) struct DocumentParserFinishSignals {
    pub(crate) parser_created_null_registry_elements: Vec<NativeNodeId>,
    pub(crate) discovery_signals: LiveDocumentParserDiscoverySignals,
}

impl LiveDocumentParserDiscoverySignals {
    pub(crate) fn extend(&mut self, other: Self) {
        self.async_prefetch_scripts
            .extend(other.async_prefetch_scripts);
        self.modulepreload_link_candidates
            .extend(other.modulepreload_link_candidates);
        self.parser_meta_csp_candidates
            .extend(other.parser_meta_csp_candidates);
        self.blocking_stylesheet_inputs
            .extend(other.blocking_stylesheet_inputs);
    }
}

impl LiveDocumentParserStepAdvance {
    #[cfg(test)]
    fn split(
        self,
    ) -> (
        LiveDocumentParserStepOutcome,
        LiveDocumentParserDiscoverySignals,
    ) {
        (self.outcome, self.discovery_signals)
    }
}

#[cfg(test)]
fn advance_live_document_parser_step<Driver>(
    stream: &mut DocumentStream,
    parser_step: &str,
    driver: &mut Driver,
) -> LiveDocumentParserStepAdvance
where
    Driver: LiveDocumentParserOwner,
{
    let outcome = pump_live_document_parser_step(stream, parser_step, driver);
    let parser_meta_csp_candidates = stream.drain_discovered_parser_meta_csp_candidates();
    live_document_parser_advance_from_outcome(outcome, parser_meta_csp_candidates)
}

fn advance_next_live_document_parser_step<Driver>(
    stream: &mut DocumentStream,
    max_bytes: usize,
    driver: &mut Driver,
) -> LiveDocumentParserStepAdvance
where
    Driver: LiveDocumentParserOwner,
{
    let outcome = pump_next_live_document_parser_step(stream, max_bytes, driver);
    let parser_meta_csp_candidates = stream.drain_discovered_parser_meta_csp_candidates();
    live_document_parser_advance_from_outcome(outcome, parser_meta_csp_candidates)
}

fn live_document_parser_advance_from_outcome(
    outcome: ParserPumpOutcome,
    discovered_parser_meta_csp_candidates: Vec<NativeNodeId>,
) -> LiveDocumentParserStepAdvance {
    let ParserPumpOutcome {
        result,
        discovered_async_prefetch_scripts,
        discovered_modulepreload_link_candidates,
        discovered_blocking_stylesheet_inputs,
    } = outcome;
    let discovery_signals = LiveDocumentParserDiscoverySignals {
        async_prefetch_scripts: discovered_async_prefetch_scripts,
        modulepreload_link_candidates: discovered_modulepreload_link_candidates,
        parser_meta_csp_candidates: discovered_parser_meta_csp_candidates,
        blocking_stylesheet_inputs: discovered_blocking_stylesheet_inputs,
    };
    let outcome = match result {
        ParserPumpStep::InputDrained => LiveDocumentParserStepOutcome::InputBoundary,
        ParserPumpStep::Yield(ParserYield::CustomElementConstruction(handoff)) => {
            LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(handoff)
        }
        ParserPumpStep::Yield(ParserYield::BlockingStylesheet(pause)) => {
            LiveDocumentParserStepOutcome::BlockingStylesheetPause(pause)
        }
        ParserPumpStep::Yield(ParserYield::Script(handoff)) => {
            LiveDocumentParserStepOutcome::ScriptHandoff(handoff)
        }
    };
    LiveDocumentParserStepAdvance {
        outcome,
        discovery_signals,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentParserLifetime {
    /// The parser consumes a finite navigation or `srcdoc` input.
    Finite,
    /// `document.open()` created a parser that remains open across writes.
    Open,
    /// `document.close()` requested EOF; finish after the active blocker releases.
    Closing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentParserCloseDisposition {
    /// The parser is ready to consume any queued input and reach EOF now.
    DrainNow,
    /// An active parser operation or blocker owns progress; closing resumes
    /// through that operation's existing completion path.
    DeferredUntilReady,
}

static NEXT_DOCUMENT_PARSER_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ParserSessionId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ParserSuspensionId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParserSuspensionCause {
    ParserClassicSource { script: NativeNodeId },
    ParserClassicStylesheets { script: NativeNodeId },
    ParserCreatedStylesheet { owner: NativeNodeId },
    DocumentWriteExternalScript { script: NativeNodeId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParserSuspension {
    id: ParserSuspensionId,
    cause: ParserSuspensionCause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParserResumePermit {
    session_id: ParserSessionId,
    suspension_id: ParserSuspensionId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParserStopReason {
    DocumentReplacement,
    MainResourceLoadFailure,
    OwnerDropped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentParserRunState {
    Ready,
    Pumping {
        epoch: u64,
    },
    Suspended {
        id: ParserSuspensionId,
        cause: ParserSuspensionCause,
    },
    Finishing,
    Finished,
    Stopped(ParserStopReason),
}

impl From<ParserSuspension> for DocumentParserRunState {
    fn from(suspension: ParserSuspension) -> Self {
        Self::Suspended {
            id: suspension.id,
            cause: suspension.cause,
        }
    }
}

impl DocumentParserRunState {
    fn suspension(self) -> Option<ParserSuspension> {
        match self {
            Self::Suspended { id, cause } => Some(ParserSuspension { id, cause }),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct DocumentParserSessionControl {
    session_id: ParserSessionId,
    next_suspension_id: u64,
    next_pump_epoch: u64,
    run_state: DocumentParserRunState,
}

#[derive(Clone, Debug)]
pub(crate) struct DocumentParserSessionControlHandle(Rc<RefCell<DocumentParserSessionControl>>);

impl DocumentParserSessionControlHandle {
    pub(crate) fn new() -> Self {
        let session_id =
            ParserSessionId(NEXT_DOCUMENT_PARSER_SESSION_ID.fetch_add(1, Ordering::Relaxed));
        Self(Rc::new(RefCell::new(DocumentParserSessionControl {
            session_id,
            next_suspension_id: 1,
            next_pump_epoch: 1,
            run_state: DocumentParserRunState::Ready,
        })))
    }

    pub(crate) fn session_id(&self) -> ParserSessionId {
        self.0.borrow().session_id
    }

    pub(crate) fn run_state(&self) -> DocumentParserRunState {
        self.0.borrow().run_state
    }

    pub(crate) fn suspend(&self, cause: ParserSuspensionCause) -> ParserResumePermit {
        let mut control = self.0.borrow_mut();
        assert_eq!(
            control.run_state,
            DocumentParserRunState::Ready,
            "only a ready live parser session can enter a persistent suspension"
        );
        let suspension_id = ParserSuspensionId(control.next_suspension_id);
        control.next_suspension_id = control.next_suspension_id.wrapping_add(1).max(1);
        let suspension = ParserSuspension {
            id: suspension_id,
            cause,
        };
        control.run_state = suspension.into();
        ParserResumePermit {
            session_id: control.session_id,
            suspension_id,
        }
    }

    pub(crate) fn current_resume_permit(&self) -> Option<ParserResumePermit> {
        let control = self.0.borrow();
        let suspension = control.run_state.suspension()?;
        Some(ParserResumePermit {
            session_id: control.session_id,
            suspension_id: suspension.id,
        })
    }

    pub(crate) fn resume(&self, permit: ParserResumePermit) -> bool {
        let mut control = self.0.borrow_mut();
        if permit.session_id != control.session_id {
            return false;
        }
        let Some(suspension) = control.run_state.suspension() else {
            return false;
        };
        if suspension.id != permit.suspension_id {
            return false;
        }
        control.run_state = DocumentParserRunState::Ready;
        true
    }

    pub(crate) fn begin_pump(&self) -> DocumentParserPumpGuard {
        let mut control = self.0.borrow_mut();
        assert_eq!(
            control.run_state,
            DocumentParserRunState::Ready,
            "a live parser may only pump from the ready state"
        );
        let epoch = control.next_pump_epoch;
        control.next_pump_epoch = control.next_pump_epoch.wrapping_add(1).max(1);
        control.run_state = DocumentParserRunState::Pumping { epoch };
        drop(control);
        DocumentParserPumpGuard {
            control: self.clone(),
            epoch,
        }
    }

    fn begin_finish(&self) {
        let mut control = self.0.borrow_mut();
        assert_eq!(
            control.run_state,
            DocumentParserRunState::Ready,
            "a suspended or stopped live parser cannot finish"
        );
        control.run_state = DocumentParserRunState::Finishing;
    }

    fn finish(&self) {
        let mut control = self.0.borrow_mut();
        assert_eq!(
            control.run_state,
            DocumentParserRunState::Finishing,
            "parser finish must complete the active finishing transition"
        );
        control.run_state = DocumentParserRunState::Finished;
    }

    pub(crate) fn stop(&self, reason: ParserStopReason) {
        let mut control = self.0.borrow_mut();
        if !matches!(
            control.run_state,
            DocumentParserRunState::Finished | DocumentParserRunState::Stopped(_)
        ) {
            control.run_state = DocumentParserRunState::Stopped(reason);
        }
    }
}

pub(crate) struct DocumentParserPumpGuard {
    control: DocumentParserSessionControlHandle,
    epoch: u64,
}

impl Drop for DocumentParserPumpGuard {
    fn drop(&mut self) {
        let mut control = self.control.0.borrow_mut();
        if control.run_state == (DocumentParserRunState::Pumping { epoch: self.epoch }) {
            control.run_state = DocumentParserRunState::Ready;
        }
    }
}

/// The canonical runtime owner of one live document parser.
///
/// Root and child documents store this same type. A `ParserInsertionController`
/// is only a temporary reentrant capability derived from its stream handle; it
/// does not own parser lifetime or suspension state.
enum ExecutableDocumentParserBackend {
    Html(DocumentParserStreamHandle),
    Xml(XmlDocumentParserStreamHandle),
}

impl ExecutableDocumentParserBackend {
    fn name(&self) -> &'static str {
        match self {
            Self::Html(_) => "html",
            Self::Xml(_) => "xml",
        }
    }
}

pub(crate) struct DocumentParserSession {
    backend: Option<ExecutableDocumentParserBackend>,
    discovery_signals: LiveDocumentParserDiscoverySignals,
    lifetime: DocumentParserLifetime,
    control: DocumentParserSessionControlHandle,
}

impl std::fmt::Debug for DocumentParserSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DocumentParserSession")
            .field(
                "backend",
                &self
                    .backend
                    .as_ref()
                    .map(ExecutableDocumentParserBackend::name),
            )
            .field("lifetime", &self.lifetime)
            .field("session_id", &self.control.session_id())
            .field("run_state", &self.control.run_state())
            .finish_non_exhaustive()
    }
}

struct DocumentParserDriver;

impl DocumentParserDriver {
    #[cfg(test)]
    fn advance_step(
        stream: &mut DocumentStream,
        parser_step: &str,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> LiveDocumentParserStepAdvance {
        advance_live_document_parser_step(stream, parser_step, owner)
    }

    fn advance_next_step(
        stream: &mut DocumentStream,
        max_bytes: usize,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> LiveDocumentParserStepAdvance {
        advance_next_live_document_parser_step(stream, max_bytes, owner)
    }

    fn note_defined_autonomous_custom_elements(
        stream: &mut DocumentStream,
        names: impl IntoIterator<Item = String>,
    ) {
        for name in names {
            stream.note_defined_autonomous_custom_element(&name);
        }
    }

    fn take_next_insertion_preload_input(stream: &DocumentStream) -> Option<String> {
        stream.take_next_insertion_preload_input()
    }

    fn take_processed_insertion_meta_csp_count(stream: &DocumentStream) -> usize {
        stream.take_processed_insertion_meta_csp_count()
    }

    fn has_script_input(stream: &DocumentStream) -> bool {
        stream.has_script_input()
    }

    #[cfg(test)]
    fn take_null_custom_element_registry_elements(
        stream: &mut DocumentStream,
    ) -> Vec<NativeNodeId> {
        stream.take_parser_stream_null_custom_element_registry_elements()
    }

    fn finish(
        stream: DocumentStream,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> DocumentParserFinishSignals {
        finish_live_document_parser(stream, owner)
    }
}

impl DocumentParserSession {
    pub(crate) fn start_main_document(document_url: Url) -> Self {
        Self::new_html(
            HtmlParser.start_document(document_url),
            DocumentParserLifetime::Finite,
        )
    }

    pub(crate) fn start_main_xml_document(document_url: Url) -> Self {
        Self::new_xml(
            XmlDocumentStream::new_top_level_document(document_url),
            DocumentParserLifetime::Finite,
        )
    }

    pub(crate) fn start_finite_live_document(
        document_url: Url,
        document_handle: NativeNodeId,
    ) -> Self {
        Self::new_html(
            HtmlParser.start_live_document_root(document_url, document_handle),
            DocumentParserLifetime::Finite,
        )
    }

    pub(crate) fn start_finite_live_xml_document(
        document_url: Url,
        document_handle: NativeNodeId,
    ) -> Self {
        Self::new_xml(
            XmlDocumentStream::new_live_document_root(document_url, document_handle),
            DocumentParserLifetime::Finite,
        )
    }

    pub(crate) fn start_open_live_document(
        document_url: Url,
        document_handle: NativeNodeId,
    ) -> Self {
        Self::new_html(
            HtmlParser.start_live_document_root(document_url, document_handle),
            DocumentParserLifetime::Open,
        )
    }

    fn new_html(stream: DocumentStream, lifetime: DocumentParserLifetime) -> Self {
        Self {
            backend: Some(ExecutableDocumentParserBackend::Html(
                new_document_parser_stream_handle(stream),
            )),
            discovery_signals: LiveDocumentParserDiscoverySignals::default(),
            lifetime,
            control: DocumentParserSessionControlHandle::new(),
        }
    }

    fn new_xml(stream: XmlDocumentStream, lifetime: DocumentParserLifetime) -> Self {
        Self {
            backend: Some(ExecutableDocumentParserBackend::Xml(Rc::new(RefCell::new(
                stream,
            )))),
            discovery_signals: LiveDocumentParserDiscoverySignals::default(),
            lifetime,
            control: DocumentParserSessionControlHandle::new(),
        }
    }

    fn backend(&self) -> &ExecutableDocumentParserBackend {
        self.backend
            .as_ref()
            .expect("a finished parser session no longer owns a backend")
    }

    pub(crate) fn run_state(&self) -> DocumentParserRunState {
        self.control.run_state()
    }

    pub(crate) fn control_handle(&self) -> DocumentParserSessionControlHandle {
        self.control.clone()
    }

    pub(crate) fn suspend(&mut self, cause: ParserSuspensionCause) -> ParserResumePermit {
        self.control.suspend(cause)
    }

    pub(crate) fn current_resume_permit(&self) -> Option<ParserResumePermit> {
        self.control.current_resume_permit()
    }

    pub(crate) fn resume(&mut self, permit: ParserResumePermit) -> bool {
        self.control.resume(permit)
    }

    pub(crate) fn stop(&mut self, reason: ParserStopReason) {
        self.control.stop(reason);
    }

    pub(crate) fn stream_handle(&self) -> DocumentParserStreamHandle {
        self.html_stream_handle()
            .expect("HTML parser stream requested from an XML document parser session")
    }

    pub(crate) fn html_stream_handle(&self) -> Option<DocumentParserStreamHandle> {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => Some(stream.clone()),
            ExecutableDocumentParserBackend::Xml(_) => None,
        }
    }

    pub(crate) fn lifetime(&self) -> DocumentParserLifetime {
        self.lifetime
    }

    pub(crate) fn request_close(&mut self) -> DocumentParserCloseDisposition {
        self.lifetime = DocumentParserLifetime::Closing;
        if self.run_state() == DocumentParserRunState::Ready {
            DocumentParserCloseDisposition::DrainNow
        } else {
            DocumentParserCloseDisposition::DeferredUntilReady
        }
    }

    pub(crate) fn finishes_on_empty_input(&self) -> bool {
        matches!(
            self.lifetime,
            DocumentParserLifetime::Finite | DocumentParserLifetime::Closing
        )
    }

    pub(crate) fn is_suspended(&self) -> bool {
        matches!(self.run_state(), DocumentParserRunState::Suspended { .. })
    }

    pub(crate) fn suspension_cause(&self) -> Option<ParserSuspensionCause> {
        self.run_state().suspension().map(|pause| pause.cause)
    }

    pub(crate) fn has_exclusive_stream_handle(&self) -> bool {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => Rc::strong_count(stream) == 1,
            ExecutableDocumentParserBackend::Xml(stream) => Rc::strong_count(stream) == 1,
        }
    }

    pub(crate) fn note_defined_autonomous_custom_elements(
        &mut self,
        names: impl IntoIterator<Item = String>,
    ) {
        if let ExecutableDocumentParserBackend::Html(stream) = self.backend() {
            DocumentParserDriver::note_defined_autonomous_custom_elements(
                &mut stream.borrow_mut(),
                names,
            );
        }
    }

    pub(crate) fn queue_arrived_chunk(&mut self, source: String) {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                stream.borrow_mut().append_to_end(source);
            }
            ExecutableDocumentParserBackend::Xml(stream) => {
                stream.borrow_mut().append_to_end(source);
            }
        }
    }

    pub(crate) fn append_to_current_inserted_input(&mut self, source: &str) -> bool {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                stream.borrow_mut().append_to_current_inserted_input(source)
            }
            ExecutableDocumentParserBackend::Xml(_) => false,
        }
    }

    pub(crate) fn declare_eof(&mut self) {
        if let ExecutableDocumentParserBackend::Xml(stream) = self.backend() {
            stream.borrow_mut().declare_eof();
        }
    }

    pub(crate) fn set_xml_document_content_type(&mut self, content_type: String) {
        match self.backend() {
            ExecutableDocumentParserBackend::Xml(stream) => {
                stream.borrow_mut().set_document_content_type(content_type);
            }
            ExecutableDocumentParserBackend::Html(_) => {
                panic!("XML content type applied to an HTML parser session")
            }
        }
    }

    pub(crate) fn has_script_input(&self) -> bool {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                DocumentParserDriver::has_script_input(&stream.borrow())
            }
            ExecutableDocumentParserBackend::Xml(_) => false,
        }
    }

    pub(crate) fn input_is_empty(&self) -> bool {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => !stream.borrow().has_pending_input(),
            ExecutableDocumentParserBackend::Xml(stream) => !stream.borrow().has_pending_input(),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.input_is_empty()
    }

    pub(crate) fn current_chunk_len(&self) -> usize {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => stream.borrow().next_input_len(),
            ExecutableDocumentParserBackend::Xml(stream) => stream.borrow().next_input_len(),
        }
    }

    pub(crate) fn take_next_insertion_preload_input(&self) -> Option<String> {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                DocumentParserDriver::take_next_insertion_preload_input(&stream.borrow())
            }
            ExecutableDocumentParserBackend::Xml(_) => None,
        }
    }

    pub(crate) fn take_processed_insertion_meta_csp_count(&self) -> usize {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                DocumentParserDriver::take_processed_insertion_meta_csp_count(&stream.borrow())
            }
            ExecutableDocumentParserBackend::Xml(_) => 0,
        }
    }

    pub(crate) fn snapshot_pending_input(&self) -> String {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                stream.borrow().snapshot_pending_input()
            }
            ExecutableDocumentParserBackend::Xml(stream) => {
                stream.borrow().snapshot_pending_input()
            }
        }
    }

    pub(crate) fn advance_queued_or_resume_step(
        &mut self,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> LiveDocumentParserStepOutcome {
        self.advance_next_step(0, owner)
    }

    pub(crate) fn advance_next_step(
        &mut self,
        max_bytes: usize,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> LiveDocumentParserStepOutcome {
        let _pump_guard = self.control.begin_pump();
        let advance = match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                DocumentParserDriver::advance_next_step(&mut stream.borrow_mut(), max_bytes, owner)
            }
            ExecutableDocumentParserBackend::Xml(stream) => {
                let mut stream = stream.borrow_mut();
                let outcome =
                    stream.pump_next_parser_step_with_runtime_dom_consumer(max_bytes, owner);
                let parser_meta_csp_candidates =
                    stream.drain_discovered_parser_meta_csp_candidates();
                live_document_parser_advance_from_outcome(outcome, parser_meta_csp_candidates)
            }
        };
        self.discovery_signals.extend(advance.discovery_signals);
        advance.outcome
    }

    #[cfg(test)]
    pub(crate) fn advance_step_and_take_null_custom_element_registry_elements(
        &mut self,
        parser_step: &str,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> (LiveDocumentParserStepOutcome, Vec<NativeNodeId>) {
        let _pump_guard = self.control.begin_pump();
        let (advance, null_custom_element_registry_elements) = match self.backend() {
            ExecutableDocumentParserBackend::Html(_) => self.with_reentrant_stream_step(|stream| {
                let advance = DocumentParserDriver::advance_step(stream, parser_step, owner);
                let null_custom_element_registry_elements =
                    DocumentParserDriver::take_null_custom_element_registry_elements(stream);
                (advance, null_custom_element_registry_elements)
            }),
            ExecutableDocumentParserBackend::Xml(_) => {
                panic!("XML document parsers do not support HTML insertion input")
            }
        };
        let (outcome, discovery_signals) = advance.split();
        self.discovery_signals.extend(discovery_signals);
        (outcome, null_custom_element_registry_elements)
    }

    pub(crate) fn take_discovery_signals(&mut self) -> LiveDocumentParserDiscoverySignals {
        std::mem::take(&mut self.discovery_signals)
    }

    pub(crate) fn with_parser_stream_dom_host_for_bootstrap<R>(
        &mut self,
        f: impl FnOnce(DomHost) -> std::result::Result<R, Box<(anyhow::Error, DomHost)>>,
    ) -> anyhow::Result<R> {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => stream
                .borrow_mut()
                .with_parser_stream_dom_host_for_bootstrap(f),
            ExecutableDocumentParserBackend::Xml(stream) => stream
                .borrow_mut()
                .with_parser_stream_dom_host_for_bootstrap(f),
        }
    }

    pub(crate) fn take_parser_stream_null_custom_element_registry_elements(
        &mut self,
    ) -> Vec<NativeNodeId> {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => stream
                .borrow_mut()
                .take_parser_stream_null_custom_element_registry_elements(),
            ExecutableDocumentParserBackend::Xml(stream) => stream
                .borrow_mut()
                .take_parser_stream_null_custom_element_registry_elements(),
        }
    }

    pub(crate) fn with_stylesheet_blocking_read_view<R>(
        &self,
        f: impl FnOnce(&dyn crate::StylesheetBlockingReadView) -> R,
    ) -> R {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                stream.borrow().with_stylesheet_blocking_read_view(f)
            }
            ExecutableDocumentParserBackend::Xml(stream) => {
                stream.borrow().with_stylesheet_blocking_read_view(f)
            }
        }
    }

    pub(crate) fn finish(
        &mut self,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> DocumentParserFinishSignals {
        assert!(
            self.input_is_empty(),
            "a live document parser may only finish after all parser-owned input is drained"
        );
        self.control.begin_finish();
        let mut discovery_signals = std::mem::take(&mut self.discovery_signals);
        let backend = self
            .backend
            .take()
            .expect("one parser session can only finish once");
        let mut finish_signals = match backend {
            ExecutableDocumentParserBackend::Html(stream) => {
                DocumentParserDriver::finish(unwrap_exclusive_parser_stream(stream), owner)
            }
            ExecutableDocumentParserBackend::Xml(stream) => {
                finish_live_xml_document_parser(unwrap_exclusive_xml_parser_stream(stream), owner)
            }
        };
        discovery_signals.extend(finish_signals.discovery_signals);
        finish_signals.discovery_signals = discovery_signals;
        self.control.finish();
        finish_signals
    }

    #[cfg(test)]
    fn with_reentrant_stream_step<R>(&self, op: impl FnOnce(&mut DocumentStream) -> R) -> R {
        let stream = self
            .html_stream_handle()
            .expect("reentrant insertion step requires an HTML parser stream");
        let stream_ptr = stream.as_ref().as_ptr();
        // SAFETY: The Rc keeps the DocumentStream allocation alive for this
        // synchronous parser step, and phase-one parser turns run on the
        // renderer owner thread. We intentionally avoid a RefCell guard here
        // because TreeSink structural mutations synchronously deliver effects
        // to a runtime mutation owner. Holding RefMut<DocumentStream> across
        // that boundary would make parser-created custom element construction
        // fail before the DOM-specific reentry rules can run.
        //
        // While this operation is active, callbacks must not reenter the same
        // parser stream. Parser-connected scripts still yield through
        // ParserScriptHandoff and are not run by the parser-tree-sink mutation
        // owner. Custom element construction must enter the dynamic markup
        // insertion guard before invoking page JS, so document.write/open/close
        // throw before they can borrow this stream.
        unsafe { op(&mut *stream_ptr) }
    }

    #[cfg(test)]
    pub(crate) fn queued_chunk_count_for_testing(&self) -> usize {
        match self.backend() {
            ExecutableDocumentParserBackend::Html(stream) => {
                stream.borrow().queued_end_segment_count_for_testing()
            }
            ExecutableDocumentParserBackend::Xml(_) => 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn current_chunk_is_non_empty_for_testing(&self) -> bool {
        self.current_chunk_len() > 0
    }
}

impl Drop for DocumentParserSession {
    fn drop(&mut self) {
        self.control.stop(ParserStopReason::OwnerDropped);
    }
}

fn finish_live_document_parser(
    stream: DocumentStream,
    owner: &mut impl LiveDocumentParserOwner,
) -> DocumentParserFinishSignals {
    let crate::parser::ParserFinishDiscoverySignals {
        parser_created_null_registry_elements,
        discovered_modulepreload_link_candidates,
        discovered_parser_meta_csp_candidates,
        discovered_blocking_stylesheet_inputs,
    } = stream.finish_with_runtime_dom_consumer(owner);
    DocumentParserFinishSignals {
        parser_created_null_registry_elements,
        discovery_signals: LiveDocumentParserDiscoverySignals {
            modulepreload_link_candidates: discovered_modulepreload_link_candidates,
            parser_meta_csp_candidates: discovered_parser_meta_csp_candidates,
            blocking_stylesheet_inputs: discovered_blocking_stylesheet_inputs,
            ..LiveDocumentParserDiscoverySignals::default()
        },
    }
}

fn finish_live_xml_document_parser(
    stream: XmlDocumentStream,
    owner: &mut impl LiveDocumentParserOwner,
) -> DocumentParserFinishSignals {
    let crate::parser::ParserFinishDiscoverySignals {
        parser_created_null_registry_elements,
        discovered_modulepreload_link_candidates,
        discovered_parser_meta_csp_candidates,
        discovered_blocking_stylesheet_inputs,
    } = stream.finish_with_runtime_dom_consumer(owner);
    DocumentParserFinishSignals {
        parser_created_null_registry_elements,
        discovery_signals: LiveDocumentParserDiscoverySignals {
            modulepreload_link_candidates: discovered_modulepreload_link_candidates,
            parser_meta_csp_candidates: discovered_parser_meta_csp_candidates,
            blocking_stylesheet_inputs: discovered_blocking_stylesheet_inputs,
            ..LiveDocumentParserDiscoverySignals::default()
        },
    }
}

fn unwrap_exclusive_parser_stream(stream: DocumentParserStreamHandle) -> DocumentStream {
    Rc::try_unwrap(stream)
        .unwrap_or_else(|_| {
            panic!("live document parser session must not retain cloned stream handles at finish")
        })
        .into_inner()
}

fn unwrap_exclusive_xml_parser_stream(stream: XmlDocumentParserStreamHandle) -> XmlDocumentStream {
    Rc::try_unwrap(stream)
        .unwrap_or_else(|_| {
            panic!("live XML parser session must not retain cloned stream handles at finish")
        })
        .into_inner()
}

#[cfg(test)]
mod session_state_tests {
    use super::*;

    fn session() -> DocumentParserSession {
        DocumentParserSession::start_finite_live_document(
            Url::parse("https://parser-session.test/").expect("test URL"),
            NativeNodeId::new(1),
        )
    }

    #[test]
    fn parser_resume_permit_is_exact_and_one_shot() {
        let mut parser = session();
        let permit = parser.suspend(ParserSuspensionCause::ParserClassicSource {
            script: NativeNodeId::new(8),
        });

        assert_eq!(permit.session_id, parser.control.session_id());
        assert!(parser.resume(permit));
        assert!(
            !parser.resume(permit),
            "a copied permit cannot resume the same suspension twice"
        );
    }

    #[test]
    fn parser_resume_rejects_wrong_session_and_suspension() {
        let mut parser = session();
        let first = parser.suspend(ParserSuspensionCause::ParserCreatedStylesheet {
            owner: NativeNodeId::new(5),
        });

        let mut other = session();
        assert!(!other.resume(first));

        assert!(parser.resume(first));
        let second = parser.suspend(ParserSuspensionCause::DocumentWriteExternalScript {
            script: NativeNodeId::new(6),
        });
        assert!(!parser.resume(first));
        assert!(parser.resume(second));
    }

    #[test]
    fn close_defers_without_consuming_the_active_parser_suspension() {
        let mut parser = DocumentParserSession::start_open_live_document(
            Url::parse("https://parser-session.test/").expect("test URL"),
            NativeNodeId::new(1),
        );
        let permit = parser.suspend(ParserSuspensionCause::ParserClassicSource {
            script: NativeNodeId::new(8),
        });
        let suspended_state = parser.run_state();

        assert_eq!(
            parser.request_close(),
            DocumentParserCloseDisposition::DeferredUntilReady
        );
        assert_eq!(parser.lifetime(), DocumentParserLifetime::Closing);
        assert_eq!(
            parser.run_state(),
            suspended_state,
            "document.close() must not bypass the active parser blocker"
        );
        assert_eq!(parser.current_resume_permit(), Some(permit));

        assert!(parser.resume(permit));
        assert_eq!(
            parser.request_close(),
            DocumentParserCloseDisposition::DrainNow,
            "the delayed close can drain after the exact blocker releases"
        );
    }

    #[test]
    fn parser_pump_and_drop_transitions_are_observable_by_derived_capabilities() {
        let parser = session();
        let control = parser.control_handle();
        {
            let _pump = control.begin_pump();
            assert!(matches!(
                control.run_state(),
                DocumentParserRunState::Pumping { .. }
            ));
        }
        assert_eq!(control.run_state(), DocumentParserRunState::Ready);

        drop(parser);
        assert_eq!(
            control.run_state(),
            DocumentParserRunState::Stopped(ParserStopReason::OwnerDropped)
        );
    }
}
