use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use url::Url;

mod devtools_mutations;
mod document_write;
mod dom_facade;
mod events;
mod inspector_issues;
mod lifecycle;
mod main_parser_continuation;
mod meta_refresh;
mod mutation_commands;
mod parser_modulepreload;
mod query_facade;
mod runtime_core;
mod script_lifecycle;
mod script_scheduling;
mod security_policy;
#[path = "stylesheet_runtime/mod.rs"]
mod stylesheet_runtime;

#[cfg(test)]
pub(crate) use mutation_commands::ParserPostStepRuntimeWorkForTest;
use mutation_commands::{ParserPostStepRuntimeWork, TreeAdoptionPlan};

pub(super) use super::host::EventTargetHandle;
use super::{
    context_bootstrap, custom_elements,
    host::{
        HostDocumentState, HostEventTargetRegistry, HostScriptScheduler, HostTimeoutRunResult,
        HostTimeoutScheduler, SelectorDebugCounters, SelectorDebugSnapshot, create_host_event,
        dispatch_public_event, dispatch_public_event_with_original_target, event_target_value,
        host_event_defaults,
    },
    mutation_coordinator::{MutationCoordinator, RuntimeMutationOptions},
    native_bridge::{self, JsContextHost},
    page_task_queue::{PageTask, PageTaskQueue, PostParseLifecycleWork, PostParsePageOwnedWork},
    planning::PreparedScript,
    runtime::RendererBrowserContextRuntime,
    stylesheet_blocking::{
        DocumentBlockingStylesheetSignature, StylesheetBlockingState, StylesheetBlockingStatus,
    },
    util::{object_string_property, v8str},
};
#[cfg(test)]
use crate::dom::native::ShadowRootBindingSnapshot;
use crate::{
    dom::{
        NodeId,
        native::{
            DocumentReadyState, DomHost, DomMutationEffects, DomStylesheetOwnerChangeKind,
            NativeDom, NativeNodeId, Node,
        },
    },
    frame_owner_model::FrameDocumentTaskOwner,
    live_document_parser::{
        DocumentParserLifetime, DocumentParserRunState, DocumentParserSession,
        DocumentParserSessionControlHandle, ParserResumePermit, ParserSuspensionCause,
    },
    module_runtime::ModuleMapKey,
    network::ResourceRequestClient,
    parser::{HtmlParser, ParserInputContext, ParserInputSession},
    selector::{QueryEngine, SelectorError},
    service_worker_runtime::ServiceWorkerClientId,
    types::SubresourceResourceType,
};
pub(crate) use devtools_mutations::{
    DevToolsDomChildListMutationFact, DevToolsDomMutationFact, DevToolsDomPrepublishedRemoval,
};
pub(crate) use inspector_issues::PendingInspectorIssue;
pub(crate) use meta_refresh::MetaRefreshNavigation;
pub(crate) use parser_modulepreload::MainDocumentModulepreloadFetchOutcome;
pub(crate) use script_lifecycle::{
    DeferredPageTask, DeferredPageTaskLane, DeferredPageTaskState, DocumentScriptLifecycle,
    FollowupPageTaskDisposition, PendingMainParserDeferredScriptStart, RuntimeScriptWorkPauseKind,
    RuntimeScriptWorkState, SharedRuntimeScriptWorkState, parser_prepared_script_page_owned_work,
    parser_script_preparation_failure_page_owned_work,
};
pub(crate) use security_policy::{
    DocumentConnectPolicySnapshot, DocumentContentSecurityPolicyCheck,
    DocumentContentSecurityPolicyViolation, DocumentSubresourceCspKind,
    create_content_security_policy_violation_event, document_content_security_policy_error_message,
    response_content_security_policies_from_headers,
    response_content_security_report_only_policies_from_headers,
};
pub(crate) use stylesheet_runtime::attribute_reprocesses_connected_stylesheet;
#[cfg(test)]
use stylesheet_runtime::{ConnectedLinkReadinessFetchOptions, ConnectedLoadParameters};
pub(crate) use stylesheet_runtime::{
    ConnectedLoadCompletion, LiveStylesheetImportLoadCompletion,
    fetch_complete_stylesheet_import_graph,
};
use stylesheet_runtime::{
    ConnectedLoadOperation, QueuedConnectedStyleLoad, StylesheetLinkClientIndex,
    StylesheetOwnerCspDisposition, StylesheetOwnerRuntimeStates,
};
pub(crate) use stylesheet_runtime::{
    ConnectedStyleEventElementKind, ConnectedStyleLoadPrimeResult, InstallLinkedStylesheet,
    OwnerlessStylesheetAdmissionError, PreparedLinkedStylesheetResource, ReadyConnectedStyleLoad,
    StylesheetLinkClientTerminal,
};

pub(super) type DomHandle = NativeNodeId;
pub(crate) type ParserStreamHandle = crate::live_document_parser::DocumentParserStreamHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HtmlFragmentCustomElementUpgradeTiming {
    InReturnedFragment,
    AfterInsertion,
}

#[derive(Debug)]
pub(crate) enum DocumentProcessingAction {
    PostParsePageOwnedWork(Box<PostParsePageOwnedWork>),
    DispatchConnectedStyleLoad(ReadyConnectedStyleLoad),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentProcessingWakeSource {
    InjectedPageTask,
    TaskSourceLoadCompletion,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseTimeWakeSource {
    InjectedPageTask,
    TaskSourceLoadCompletion,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseTimeWakeObservation {
    ReadyNow,
    Arrived(ParseTimeWakeSource),
    TimedOutNoReady,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentProcessingWakeObservation {
    ReadyNow,
    Arrived(DocumentProcessingWakeSource),
    NoWake,
}

#[derive(Clone, Copy)]
pub(crate) struct PostParseOwnerReadiness {
    pub(crate) should_poll_document_processing: bool,
    pub(crate) blocks_page_task_pop: bool,
    pub(crate) has_pending_progress_source: bool,
}

#[derive(Debug)]
pub(crate) enum PostParseOwnerDriverStep {
    Ready(Box<DocumentProcessingAction>),
    NeedsContinuation,
    AwaitProgress,
    Idle,
}

#[derive(Debug)]
struct ParserConnectedScriptContext {
    insertion_controller: ParserInsertionController,
    _input_context: ParserInputContext,
}

#[derive(Debug)]
struct CurrentScriptContext {
    handle: Option<DomHandle>,
    parser_connected: Option<ParserConnectedScriptContext>,
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentScriptContextSpec {
    pub(crate) handle: Option<DomHandle>,
    pub(crate) parser_write_insertion_point_active: bool,
    pub(crate) parser_insertion_controller: Option<ParserInsertionController>,
}

#[derive(Clone)]
pub(crate) struct ParserInsertionController {
    input_session: ParserInputSession,
    parser_stream: ParserStreamHandle,
    parser_control: DocumentParserSessionControlHandle,
}

impl std::fmt::Debug for ParserInsertionController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserInsertionController")
            .field("input_session", &self.input_session)
            .field("session_id", &self.parser_control.session_id())
            .field("run_state", &self.parser_control.run_state())
            .finish()
    }
}

impl ParserInsertionController {
    #[cfg(test)]
    pub(crate) fn for_stream(stream: ParserStreamHandle) -> Self {
        let input_session = stream.borrow().script_input_session();
        Self {
            input_session,
            parser_stream: stream,
            parser_control: DocumentParserSessionControlHandle::new(),
        }
    }

    pub(crate) fn for_session(parser: &DocumentParserSession) -> Option<Self> {
        let parser_stream = parser.html_stream_handle()?;
        let input_session = parser_stream.borrow().script_input_session();
        Some(Self {
            input_session,
            parser_stream,
            parser_control: parser.control_handle(),
        })
    }

    pub(crate) fn input_session(&self) -> ParserInputSession {
        self.input_session.clone()
    }

    pub(crate) fn parser_stream(&self) -> ParserStreamHandle {
        self.parser_stream.clone()
    }

    pub(crate) fn run_state(&self) -> DocumentParserRunState {
        self.parser_control.run_state()
    }

    pub(crate) fn suspend(&self, cause: ParserSuspensionCause) -> ParserResumePermit {
        self.parser_control.suspend(cause)
    }

    pub(crate) fn resume(&self, permit: ParserResumePermit) -> bool {
        self.parser_control.resume(permit)
    }

    pub(crate) fn begin_pump(&self) -> crate::live_document_parser::DocumentParserPumpGuard {
        self.parser_control.begin_pump()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StylesheetLinkPromotionTrace {
    promotion_count: u64,
    inspected_link_states: u64,
    promoted_clients: u64,
    max_indexed_link_clients: usize,
    total_elapsed_us: u128,
}

struct StylesheetLifecycleState {
    pending_connected_loads: VecDeque<Arc<QueuedConnectedStyleLoad>>,
    /// Synthetic ready events used only by low-level ordering tests.
    ///
    /// Async stylesheet work never enters this queue; it always publishes a
    /// typed DOM-manipulation task through `task_producer`.
    #[cfg(test)]
    injected_ready_connected_loads: VecDeque<ReadyConnectedStyleLoad>,
    /// Owners already processed through a connected mutation before the
    /// one-time post-parse discovery scan.
    ///
    /// Their event tasks may finish before that scan now that DOM work has a
    /// stable Page source, so transient event residence cannot be used as the
    /// discovery deduplication fact.
    pre_initial_scan_processed_owners: HashSet<DomHandle>,
    ready_connected_load_network_results: VecDeque<ConnectedLoadNetworkResult>,
    ready_stylesheet_link_client_terminals: VecDeque<StylesheetLinkClientTerminal>,
    // One canonical entry owns each stylesheet-related owner's operation,
    // completion/event disposition, and link state.
    owner_states: StylesheetOwnerRuntimeStates,
    link_client_index: StylesheetLinkClientIndex,
    link_promotion_trace: StylesheetLinkPromotionTrace,
    fetches: StylesheetBlockingState,
    task_sender: crate::page_task_queue::RendererPageStylesheetTaskSender,
    task_producer: Option<crate::page_task_queue::RendererPageStylesheetTaskProducer>,
    service_worker_connected_link_context: Option<ServiceWorkerConnectedLinkContext>,
    #[cfg(test)]
    task_test_residence: Option<crate::page_task_queue::RendererPageStylesheetTaskTestResidence>,
}

impl StylesheetLifecycleState {
    fn new(task_sender: crate::page_task_queue::RendererPageStylesheetTaskSender) -> Self {
        Self {
            pending_connected_loads: VecDeque::new(),
            #[cfg(test)]
            injected_ready_connected_loads: VecDeque::new(),
            pre_initial_scan_processed_owners: HashSet::new(),
            ready_connected_load_network_results: VecDeque::new(),
            ready_stylesheet_link_client_terminals: VecDeque::new(),
            owner_states: StylesheetOwnerRuntimeStates::default(),
            link_client_index: StylesheetLinkClientIndex::default(),
            link_promotion_trace: StylesheetLinkPromotionTrace::default(),
            fetches: StylesheetBlockingState::default(),
            task_sender,
            task_producer: None,
            service_worker_connected_link_context: None,
            #[cfg(test)]
            task_test_residence: None,
        }
    }

    fn set_service_worker_connected_link_context(
        &mut self,
        browser_context_runtime: RendererBrowserContextRuntime,
        client_id: ServiceWorkerClientId,
    ) {
        self.service_worker_connected_link_context = Some(ServiceWorkerConnectedLinkContext {
            browser_context_runtime,
            client_id,
        });
    }
}

#[derive(Clone)]
struct ServiceWorkerConnectedLinkContext {
    browser_context_runtime: RendererBrowserContextRuntime,
    client_id: ServiceWorkerClientId,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ConnectedStyleImportRoot {
    pub(crate) owner: DomHandle,
    pub(crate) stylesheet_id: crate::live_stylesheet::StylesheetId,
    pub(crate) contents_revision: u64,
    pub(crate) import_generation: u64,
    pub(crate) root_resource_url: Option<Url>,
}

impl ConnectedStyleImportRoot {
    pub(crate) fn new(
        owner: DomHandle,
        stylesheet: &crate::live_stylesheet::LiveStylesheetRef,
        root_is_external_resource: bool,
    ) -> Self {
        Self {
            owner,
            stylesheet_id: stylesheet.id(),
            contents_revision: stylesheet.contents_revision(),
            import_generation: stylesheet.import_generation(),
            root_resource_url: root_is_external_resource.then(|| stylesheet.base_url().clone()),
        }
    }

    pub(crate) fn matches_stylesheet(
        &self,
        stylesheet: &crate::live_stylesheet::LiveStylesheetRef,
    ) -> bool {
        stylesheet.id() == self.stylesheet_id
            && stylesheet.contents_revision() == self.contents_revision
            && stylesheet.import_generation() == self.import_generation
    }
}

#[derive(Debug)]
pub(crate) struct ReadyBlockingStyleImportGraph {
    operation: Arc<ConnectedLoadOperation>,
    roots: Vec<ConnectedStyleImportRoot>,
    graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    successful: bool,
}

impl ReadyBlockingStyleImportGraph {
    fn new(
        operation: Arc<ConnectedLoadOperation>,
        roots: Vec<ConnectedStyleImportRoot>,
        graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
        successful: bool,
    ) -> Self {
        Self {
            operation,
            roots,
            graph,
            successful,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Arc<ConnectedLoadOperation>,
        Vec<ConnectedStyleImportRoot>,
        Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
        bool,
    ) {
        (self.operation, self.roots, self.graph, self.successful)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConnectedLoadNetworkResult {
    pub(crate) stylesheet_fetch: Option<crate::stylesheet_blocking::StylesheetFetch>,
    pub(crate) blocking_operation: Option<crate::stylesheet_blocking::StylesheetBlockingOperation>,
    pub(crate) source_operation: Option<Arc<ConnectedLoadOperation>>,
    pub(crate) import_roots: Vec<ConnectedStyleImportRoot>,
    pub(crate) document_url: Url,
    pub(crate) request_url: Url,
    pub(crate) source_owners: Vec<DomHandle>,
    pub(crate) resource_type: SubresourceResourceType,
    pub(crate) start_unix_millis: Option<f64>,
    pub(crate) origin_clean: bool,
    pub(crate) result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
}

#[derive(Debug)]
struct DocumentWriteExternalScriptStart {
    node: DomHandle,
    host_script_handle: String,
    script: PreparedScript,
}

#[derive(Debug)]
struct SuspendedDocumentWriteInsertion {
    document_handle: DomHandle,
    parser_insertion_controller: ParserInsertionController,
    resume_permit: ParserResumePermit,
    resume_permit_consumed: bool,
}

#[derive(Debug)]
enum SuspendedDocumentWriteContinuation {
    ResumeAfterCompleted {
        start: DocumentWriteExternalScriptStart,
        insertion: SuspendedDocumentWriteInsertion,
    },
    StartExternal {
        start: DocumentWriteExternalScriptStart,
        insertion: SuspendedDocumentWriteInsertion,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    },
}

#[derive(Debug)]
struct PendingDocumentWriteExternalScriptLoad {
    target: crate::types::DocumentWriteExternalScriptFetchTarget,
    start: DocumentWriteExternalScriptStart,
    insertion: SuspendedDocumentWriteInsertion,
    /// Stylesheets preceding this parser-inserted script.  Their fetches run
    /// in parallel with the script source, but their owner events and style
    /// installation must complete before the source can execute.
    blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    ready_completion: Option<crate::types::DocumentWriteExternalScriptLoadCompletion>,
    resume_after_completion: VecDeque<SuspendedDocumentWriteContinuation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentWriteExternalScriptLoadApplication {
    Applied,
    SupersededDuringApplication,
    RejectedStaleTarget,
}

#[derive(Debug)]
struct DocumentWriteScriptPreload {
    request: crate::runtime::BufferedScriptPreloadRequest,
    target: crate::types::DocumentWriteExternalScriptFetchTarget,
    ready_completion: Option<crate::types::DocumentWriteExternalScriptLoadCompletion>,
}

/// A parser-blocking script reached by a live `document.write`-style parser
/// while one of the stylesheets that preceded it is still unresolved.
///
/// The parser stream and its insertion point stay owned by the document
/// runtime.  A stylesheet-completion owner turn resumes this exact handoff;
/// CDP/document callers never wait on the network operation themselves.
#[derive(Debug)]
struct PendingDocumentWriteStylesheetBlockedScript {
    node: DomHandle,
    start_line: u64,
    start_column: u64,
    script: PreparedScript,
    blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    insertion: SuspendedDocumentWriteInsertion,
}

/// Parser-created blocking stylesheet boundary with no script attached to it.
/// Blink pauses token consumption at this boundary until the stylesheet owner
/// settles, even when the next token is an ordinary element.
#[derive(Debug)]
struct PendingDocumentWriteStylesheetParserPause {
    blocking_signatures: HashSet<DocumentBlockingStylesheetSignature>,
    insertion: SuspendedDocumentWriteInsertion,
}

#[derive(Debug)]
enum DocumentWriteScriptRunOutcome {
    Complete,
    Suspend(Box<DocumentWriteExternalScriptStart>),
}

impl std::fmt::Debug for StylesheetLifecycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("StylesheetLifecycleState");
        debug.field(
            "pending_connected_loads_len",
            &self.pending_connected_loads.len(),
        );
        #[cfg(test)]
        debug.field(
            "injected_ready_connected_loads_len",
            &self.injected_ready_connected_loads.len(),
        );
        debug
            .field("owner_states_len", &self.owner_states.len())
            .field(
                "indexed_stylesheet_link_clients",
                &self.link_client_index.len(),
            )
            .field("link_promotion_trace", &self.link_promotion_trace)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParserStepGenerationGuard {
    runtime_reset_generation: u64,
    depth: usize,
}

#[derive(Debug, Default)]
struct ParserReentryState {
    active_step_generation_guard: Option<ParserStepGenerationGuard>,
    custom_element_reaction_queue_active: bool,
    // Counts nested synchronous parser-blocking script execution scopes,
    // including load/error completion dispatched before parser resume. This
    // does not describe parser-inserted ownership, deferred-script execution,
    // parser pause depth, or document.write insertion-point availability.
    script_nesting_level: usize,
    pause_depth: usize,
    insertion_session_depth: usize,
    dynamic_markup_insertion_counters: HashMap<DomHandle, usize>,
}

pub(crate) struct ParserScriptNestingGuard {
    runtime: *mut DocumentRuntime,
}

pub(crate) struct ParserPauseGuard {
    runtime: *mut DocumentRuntime,
}

pub(crate) struct ParserInsertionSessionGuard {
    runtime: *mut DocumentRuntime,
}

impl Drop for ParserScriptNestingGuard {
    fn drop(&mut self) {
        // SAFETY: The guard is created from `DocumentRuntime::enter_parser_script_nesting`
        // and is only used on the renderer owner thread while the owning
        // `DocumentRuntime` remains alive. The raw pointer avoids holding a
        // broad mutable borrow across V8 script execution, which can re-enter
        // the runtime through DOM callbacks.
        unsafe { &mut *self.runtime }.exit_parser_script_nesting();
    }
}

impl Drop for ParserPauseGuard {
    fn drop(&mut self) {
        // SAFETY: The guard is created from `DocumentRuntime::enter_parser_pause`
        // and is scoped to the renderer owner thread. The raw pointer avoids
        // holding a broad mutable borrow while V8 executes user constructor code.
        unsafe { &mut *self.runtime }.exit_parser_pause();
    }
}

impl Drop for ParserInsertionSessionGuard {
    fn drop(&mut self) {
        // SAFETY: The guard is created from `DocumentRuntime::enter_parser_insertion_session`
        // and stays on the renderer owner thread. The raw pointer avoids
        // holding a broad mutable borrow across parser pumping and script handoff.
        unsafe { &mut *self.runtime }.exit_parser_insertion_session();
    }
}

struct StructuralMutationGuard {
    runtime: *mut DocumentRuntime,
}

impl Drop for StructuralMutationGuard {
    fn drop(&mut self) {
        // SAFETY: The guard is created from `DocumentRuntime::enter_structural_mutation`
        // and is scoped to the renderer owner thread. The raw pointer avoids
        // holding a broad mutable borrow while the runtime mutates its native
        // DOM storage.
        unsafe { &mut *self.runtime }.exit_structural_mutation();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocumentPolicyContainer {
    pub(crate) document_referrer: String,
    pub(crate) referrer_policy: Option<String>,
    pub(crate) cross_origin_embedder_policy:
        crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
    pub(crate) document_isolation_policy: crate::cross_origin_isolation::DocumentIsolationPolicy,
    pub(crate) cross_origin_isolated: bool,
    pub(crate) document_content_security_policies: Vec<String>,
    pub(crate) response_content_security_policies: Vec<String>,
    pub(crate) response_content_security_report_only_policies: Vec<String>,
    pub(crate) content_security_reporting_endpoints:
        crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    pub(crate) credentialless: bool,
    pub(crate) credentialless_storage_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    pub(crate) sandbox: DocumentSandboxPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DocumentSandboxPolicy {
    pub(crate) forces_opaque_origin: bool,
    pub(crate) allows_scripts: bool,
    pub(crate) allows_popups_to_escape: bool,
    pub(crate) sandboxes_document_domain: bool,
}

impl Default for DocumentSandboxPolicy {
    fn default() -> Self {
        Self {
            forces_opaque_origin: false,
            allows_scripts: true,
            allows_popups_to_escape: false,
            sandboxes_document_domain: false,
        }
    }
}

impl DocumentSandboxPolicy {
    pub(crate) fn from_response_content_security_policies(policies: &[String]) -> Self {
        Self {
            forces_opaque_origin:
                crate::content_security_policy::content_security_policy_forces_opaque_origin(
                    policies,
                ),
            allows_scripts:
                crate::content_security_policy::content_security_policy_sandbox_allows_scripts(
                    policies,
                ),
            allows_popups_to_escape:
                crate::content_security_policy::content_security_policy_sandbox_allows_popups_to_escape(
                    policies,
                ),
            sandboxes_document_domain:
                crate::content_security_policy::content_security_policy_sandboxes_document_domain(
                    policies,
                ),
        }
    }

    pub(crate) fn with_response_content_security_policy(mut self, response: Self) -> Self {
        if response.sandboxes_document_domain {
            self.forces_opaque_origin |= response.forces_opaque_origin;
            self.allows_scripts &= response.allows_scripts;
            self.allows_popups_to_escape = if self.sandboxes_document_domain {
                self.allows_popups_to_escape && response.allows_popups_to_escape
            } else {
                response.allows_popups_to_escape
            };
        }
        self.sandboxes_document_domain |= response.sandboxes_document_domain;
        self
    }
}

#[derive(Debug)]
pub(super) struct DocumentRuntime {
    dom_host: LiveRuntimeDomHost,
    parser_reentry: ParserReentryState,
    pending_parser_post_step_runtime_work: ParserPostStepRuntimeWork,
    /// Used when temporarily parking live shadow-root bindings across compat replacement paths.
    #[cfg(test)]
    parked_live_shadow_root_bindings: Option<Vec<ShadowRootBindingSnapshot>>,
    selector_engine: QueryEngine,
    selector_debug: SelectorDebugCounters,
    document: HostDocumentState,
    design_mode_documents: HashSet<DomHandle>,
    script_execution_control: crate::script_execution_control::RendererScriptExecutionControl,
    bypass_content_security_policy: bool,
    policy_container: DocumentPolicyContainer,
    delivered_meta_content_security_policies: RefCell<HashMap<(u64, DomHandle), Vec<String>>>,
    processed_meta_content_security_policy_handles: RefCell<HashSet<(u64, DomHandle, DomHandle)>>,
    document_character_set: String,
    resource_loader_binding: Option<DocumentResourceLoaderBinding>,
    script_context_stack: Vec<CurrentScriptContext>,
    root_document_parser: Option<DocumentParserSession>,
    post_parse_schedule_invalidated: bool,
    stylesheet_lifecycle: StylesheetLifecycleState,
    main_parser_continuation: main_parser_continuation::MainParserContinuationState,
    pending_stylesheet_source_css_projection_owners: Vec<DomHandle>,
    pending_connected_style_load_prime_result: ConnectedStyleLoadPrimeResult,
    initial_connected_style_loads_queued: bool,
    late_preload_stylesheet_handles: HashSet<DomHandle>,
    in_document_image_priority_boost_count: usize,
    parser_discovered_modulepreloads: HashSet<ModuleMapKey>,
    modulepreload_invalid_as_link_errors: HashSet<DomHandle>,
    style_source_document_sync_pending: bool,
    pending_devtools_dom_mutations: Vec<devtools_mutations::DevToolsDomMutationFact>,
    #[cfg(test)]
    pending_runtime_binding_calls: Vec<native_bridge::PendingRuntimeBindingCall>,
    pending_inspector_issues: Vec<inspector_issues::PendingInspectorIssue>,
    quirks_mode_issue_reported: bool,
    script_lifecycle: DocumentScriptLifecycle,
    parser_script_start_positions: HashMap<DomHandle, ParserScriptStartPosition>,
    timeouts: HostTimeoutScheduler,
    events: HostEventTargetRegistry,
    mutations: MutationCoordinator,
    meta_refresh_scheduler: meta_refresh::MetaRefreshScheduler,
    custom_element_reaction_depth: usize,
    structural_mutation_depth: usize,
    dom_content_loaded_dispatched: bool,
    runtime_reset_generation: u64,
    document_input_stream_opened: bool,
    next_document_write_external_script_load_id: u64,
    document_write_script_preload_scanner:
        Option<Box<crate::runtime::IncrementalBufferedScriptPreloadScanner>>,
    /// In-flight classic script loads started by the main-document scanner.
    /// The Document shares this resource residence with parser re-entry while
    /// scanner/tokenizer state remains owned by phase one.
    main_document_script_preloads: crate::runtime::DocumentScriptPreloadStore,
    document_write_script_preloads:
        HashMap<crate::runtime::BufferedScriptPreloadKey, DocumentWriteScriptPreload>,
    pending_document_write_external_script_load: Option<PendingDocumentWriteExternalScriptLoad>,
    pending_document_write_stylesheet_blocked_script:
        Option<PendingDocumentWriteStylesheetBlockedScript>,
    pending_document_write_stylesheet_parser_pause:
        Option<PendingDocumentWriteStylesheetParserPause>,
}

#[derive(Clone, Debug)]
struct DocumentResourceLoaderBinding {
    registry: crate::network::context::DocumentResourceLoaderRegistry,
    owner: crate::native_bridge::WindowDocumentOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParserScriptStartPosition {
    pub(crate) line: u64,
    pub(crate) column: u64,
}

/// Wraps the runtime-owned live `DomHost`.
///
/// The runtime always owns the `DomHost`; parser steps do not move it out and
/// do not put the runtime into a separate DOM-access mode. Parser callbacks
/// call renderer-owned sinks, and each sink short-borrows this same native DOM
/// object graph through `DocumentRuntime`.
///
/// The wrapper centralizes direct `DomHost` borrowing so the parser path can
/// avoid carrying a `DomHost` pointer or borrow token through helper layers.
#[derive(Debug)]
struct LiveRuntimeDomHost {
    dom_host: DomHost,
}

impl LiveRuntimeDomHost {
    fn from_dom_host(dom_host: DomHost) -> Self {
        Self { dom_host }
    }

    #[track_caller]
    fn borrow(&self) -> &DomHost {
        &self.dom_host
    }

    #[track_caller]
    fn borrow_mut(&mut self) -> &mut DomHost {
        &mut self.dom_host
    }

    fn into_dom_host(self) -> DomHost {
        self.dom_host
    }
}

impl DocumentRuntime {
    #[cfg(test)]
    /// Exposes the synchronous parser-blocking execution depth to contract
    /// tests; it is not a general measure of parser ownership or activity.
    pub(crate) fn parser_script_nesting_level(&self) -> usize {
        self.parser_reentry.script_nesting_level
    }

    #[cfg(test)]
    pub(crate) fn parser_pause_depth(&self) -> usize {
        self.parser_reentry.pause_depth
    }

    #[cfg(test)]
    pub(crate) fn is_parser_paused_for_reentry(&self) -> bool {
        self.parser_reentry.pause_depth > 0
    }

    #[cfg(test)]
    pub(crate) fn parser_insertion_session_depth(&self) -> usize {
        self.parser_reentry.insertion_session_depth
    }

    pub(crate) fn is_parser_insertion_session_active(&self) -> bool {
        self.parser_reentry.insertion_session_depth > 0
    }

    #[cfg(test)]
    pub(crate) fn active_parser_step_generation(&self) -> Option<u64> {
        self.parser_reentry
            .active_step_generation_guard
            .map(|guard| guard.runtime_reset_generation)
    }

    #[cfg(test)]
    pub(crate) fn active_parser_step_depth(&self) -> Option<usize> {
        self.parser_reentry
            .active_step_generation_guard
            .map(|guard| guard.depth)
    }

    pub(crate) fn should_checkpoint_before_parser_custom_element_constructor(&self) -> bool {
        self.parser_reentry.script_nesting_level == 0
    }

    pub(crate) fn enter_parser_script_nesting(&mut self) -> ParserScriptNestingGuard {
        self.parser_reentry.script_nesting_level = self
            .parser_reentry
            .script_nesting_level
            .checked_add(1)
            .expect("parser script nesting level overflow");
        ParserScriptNestingGuard {
            runtime: self as *mut DocumentRuntime,
        }
    }

    fn exit_parser_script_nesting(&mut self) {
        assert!(
            self.parser_reentry.script_nesting_level > 0,
            "parser script nesting guard exited without matching enter"
        );
        self.parser_reentry.script_nesting_level -= 1;
    }

    pub(crate) fn enter_parser_pause(&mut self) -> ParserPauseGuard {
        self.parser_reentry.pause_depth = self
            .parser_reentry
            .pause_depth
            .checked_add(1)
            .expect("parser pause depth overflow");
        ParserPauseGuard {
            runtime: self as *mut DocumentRuntime,
        }
    }

    fn exit_parser_pause(&mut self) {
        assert!(
            self.parser_reentry.pause_depth > 0,
            "parser pause guard exited without matching enter"
        );
        self.parser_reentry.pause_depth -= 1;
    }

    pub(crate) fn enter_parser_insertion_session(&mut self) -> ParserInsertionSessionGuard {
        self.parser_reentry.insertion_session_depth = self
            .parser_reentry
            .insertion_session_depth
            .checked_add(1)
            .expect("parser insertion session depth overflow");
        ParserInsertionSessionGuard {
            runtime: self as *mut DocumentRuntime,
        }
    }

    fn exit_parser_insertion_session(&mut self) {
        assert!(
            self.parser_reentry.insertion_session_depth > 0,
            "parser insertion session guard exited without matching enter"
        );
        self.parser_reentry.insertion_session_depth -= 1;
    }

    pub(crate) fn enter_throw_on_dynamic_markup_insertion(&mut self, document: DomHandle) {
        *self
            .parser_reentry
            .dynamic_markup_insertion_counters
            .entry(document)
            .or_insert(0) += 1;
    }

    pub(crate) fn exit_throw_on_dynamic_markup_insertion(&mut self, document: DomHandle) {
        let Some(counter) = self
            .parser_reentry
            .dynamic_markup_insertion_counters
            .get_mut(&document)
        else {
            return;
        };
        *counter = counter.saturating_sub(1);
        if *counter == 0 {
            self.parser_reentry
                .dynamic_markup_insertion_counters
                .remove(&document);
        }
    }

    pub(crate) fn has_throw_on_dynamic_markup_insertion_counter(
        &self,
        document: DomHandle,
    ) -> bool {
        self.parser_reentry
            .dynamic_markup_insertion_counters
            .get(&document)
            .is_some_and(|counter| *counter > 0)
    }

    /// Runs a synchronous parser step against the runtime-owned live DOM.
    ///
    /// This keeps the active generation/depth guard owned by `DocumentRuntime`
    /// and guarantees the guard is released when the step returns or unwinds.
    #[track_caller]
    pub(crate) fn with_dom_host_parse_step<R>(&mut self, step: impl FnOnce(&mut Self) -> R) -> R {
        struct FinishParserStepOnDrop<'a> {
            runtime: &'a mut DocumentRuntime,
        }

        impl Drop for FinishParserStepOnDrop<'_> {
            fn drop(&mut self) {
                self.runtime.finish_dom_host_parse_step();
            }
        }

        self.begin_dom_host_parse_step();
        let guard = FinishParserStepOnDrop { runtime: self };
        step(&mut *guard.runtime)
    }

    /// Starts one synchronous parser step against the runtime-owned live DOM.
    ///
    /// Parser sinks do not receive this generation identity. They call back into
    /// `DocumentRuntime`, and each callback short-borrows the current DomHost
    /// after this runtime-private active step guard rejects stale document
    /// replacement generation changes.
    #[track_caller]
    pub(crate) fn begin_dom_host_parse_step(&mut self) {
        let generation = self.runtime_reset_generation;
        if let Some(active) = self.parser_reentry.active_step_generation_guard.as_mut() {
            assert_eq!(
                active.runtime_reset_generation, generation,
                "nested parser steps must target the current document generation"
            );
            active.depth = active
                .depth
                .checked_add(1)
                .expect("parser step depth overflow");
            return;
        }
        self.parser_reentry.active_step_generation_guard = Some(ParserStepGenerationGuard {
            runtime_reset_generation: generation,
            depth: 1,
        });
    }

    /// Finishes one synchronous parser step and marks parser-discovered work dirty.
    #[track_caller]
    pub(crate) fn finish_dom_host_parse_step(&mut self) {
        let guard = self
            .parser_reentry
            .active_step_generation_guard
            .as_mut()
            .expect("parser step must be active before finishing");
        assert!(
            guard.depth > 0,
            "parser step must be active before finishing"
        );
        guard.depth -= 1;
        if guard.depth == 0 {
            self.parser_reentry.active_step_generation_guard.take();
        }
    }

    #[track_caller]
    fn assert_active_parser_dom_generation(&self) {
        let active = self
            .parser_reentry
            .active_step_generation_guard
            .expect("parser step must be active");
        assert_eq!(
            active.runtime_reset_generation, self.runtime_reset_generation,
            "parser step must target the current document generation"
        );
    }

    #[track_caller]
    fn dom_host_mut_for_active_parser_step(&mut self) -> &mut DomHost {
        self.assert_active_parser_dom_generation();
        self.dom_host.borrow_mut()
    }

    fn enter_structural_mutation(&mut self) -> StructuralMutationGuard {
        self.structural_mutation_depth = self
            .structural_mutation_depth
            .checked_add(1)
            .expect("structural mutation depth overflow");
        StructuralMutationGuard {
            runtime: self as *mut DocumentRuntime,
        }
    }

    fn exit_structural_mutation(&mut self) {
        assert!(
            self.structural_mutation_depth > 0,
            "structural mutation guard exited without matching enter"
        );
        self.structural_mutation_depth -= 1;
    }

    pub(crate) fn is_structural_mutation_active(&self) -> bool {
        self.structural_mutation_depth > 0
    }

    pub(crate) fn debug_assert_not_in_structural_mutation(&self, action: &str) {
        debug_assert!(
            !self.is_structural_mutation_active(),
            "{action} must run after structural mutation scope exits"
        );
    }

    #[track_caller]
    fn append_child_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> DomMutationEffects {
        let _guard = self.enter_structural_mutation();
        self.dom_host.append_child_effects(parent, child)
    }

    #[track_caller]
    fn insert_before_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> DomMutationEffects {
        let _guard = self.enter_structural_mutation();
        self.dom_host
            .insert_before_effects(parent, child, reference_child)
    }

    #[track_caller]
    fn remove_child_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> DomMutationEffects {
        let _guard = self.enter_structural_mutation();
        self.dom_host.remove_child_effects(parent, child)
    }

    #[track_caller]
    fn replace_child_with_self_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        old_child: DomHandle,
    ) -> DomMutationEffects {
        let _guard = self.enter_structural_mutation();
        self.dom_host
            .replace_child_with_self_effects(parent, old_child)
    }

    #[track_caller]
    fn parser_append_child_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> DomMutationEffects {
        self.assert_active_parser_dom_generation();
        self.append_child_effects_in_structural_scope(parent, child)
    }

    #[track_caller]
    fn parser_insert_before_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> DomMutationEffects {
        self.assert_active_parser_dom_generation();
        self.insert_before_effects_in_structural_scope(parent, child, reference_child)
    }

    #[track_caller]
    fn parser_remove_child_effects_in_structural_scope(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> DomMutationEffects {
        self.assert_active_parser_dom_generation();
        self.remove_child_effects_in_structural_scope(parent, child)
    }

    pub(crate) fn node_is_connected_for_web_api(&self, handle: DomHandle) -> bool {
        custom_elements::is_shadow_including_rooted_in_document(self.dom_host.borrow(), handle)
    }
}

impl std::ops::Deref for LiveRuntimeDomHost {
    type Target = DomHost;

    #[track_caller]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl std::ops::DerefMut for LiveRuntimeDomHost {
    #[track_caller]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.borrow_mut()
    }
}

impl super::planning::ParserPlanningReadView for LiveRuntimeDomHost {
    fn parser_script_read(
        &self,
        node_id: NativeNodeId,
    ) -> Option<super::planning::ParserScriptRead> {
        self.borrow().parser_script_read(node_id)
    }

    fn is_connected(&self, node_id: NativeNodeId) -> bool {
        self.borrow().is_connected(node_id)
    }

    fn script_handles(&self) -> Vec<NativeNodeId> {
        self.borrow().script_handles()
    }

    fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        self.borrow().document_order_script_handles()
    }

    fn document_order_position(&self, node_id: NativeNodeId) -> Option<usize> {
        self.borrow().document_order_position(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.borrow().final_url_clone()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        <DomHost as super::planning::ParserPlanningReadView>::document_base_url_clone(self.borrow())
    }
}

impl super::stylesheet_blocking::StylesheetBlockingReadView for LiveRuntimeDomHost {
    fn stylesheet_element(
        &self,
        node_id: NativeNodeId,
    ) -> Option<super::stylesheet_blocking::StylesheetElementRead> {
        <DomHost as super::stylesheet_blocking::StylesheetBlockingReadView>::stylesheet_element(
            self.borrow(),
            node_id,
        )
    }

    fn child_ids(&self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        <DomHost as super::stylesheet_blocking::StylesheetBlockingReadView>::child_ids(
            self.borrow(),
            node_id,
        )
    }

    fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.borrow().text_content(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        <DomHost as super::stylesheet_blocking::StylesheetBlockingReadView>::final_url_clone(
            self.borrow(),
        )
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        <DomHost as super::stylesheet_blocking::StylesheetBlockingReadView>::document_base_url_clone(
            self.borrow(),
        )
    }

    fn document_node_id(&self) -> NativeNodeId {
        self.borrow().document_node_id()
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<NodeId>,
    ) -> Vec<NativeNodeId> {
        <DomHost as super::stylesheet_blocking::StylesheetBlockingReadView>::document_order_stylesheet_candidate_ids_before(
            self.borrow(),
            target_node_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use url::Url;

    use crate::stylesheet_blocking::StylesheetFetchOptions;
    use crate::{
        dom::{
            NodeId,
            native::{Element, Node},
        },
        network::ResourceRequestClient,
        parser::HtmlParser,
        types::SubresourceResourceType,
        {
            document_runtime::DocumentProcessingWakeObservation,
            document_script_scheduler::{DocumentScriptExecutionLane, PageOwnedDocumentScriptWork},
            native_bridge::PendingRuntimeBindingCall,
            page_task_queue::{PageTask, PostParsePageOwnedWork},
        },
    };
    use moli_fetch::FetchConfig;

    use super::{
        ConnectedLinkReadinessFetchOptions, ConnectedLoadCompletion, ConnectedLoadNetworkResult,
        ConnectedLoadOperation, ConnectedLoadParameters, ConnectedStyleEventElementKind,
        DocumentRuntime, DomHandle, DomHost, LiveRuntimeDomHost, ParseTimeWakeObservation,
        ParseTimeWakeSource,
    };

    fn first_element_handle(document: &crate::dom::native::NativeDom, tag_name: &str) -> DomHandle {
        let mut stack = vec![document.document_node_id()];
        while let Some(handle) = stack.pop() {
            if document
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_element(tag_name))
            {
                return handle;
            }
            let mut children = document.child_nodes(handle).unwrap_or_default();
            children.reverse();
            stack.extend(children);
        }
        panic!("expected {tag_name} element in test document")
    }

    fn parse_document_with_blocking_stylesheet_inputs(
        final_url: Url,
        html: impl Into<String>,
    ) -> (
        crate::dom::native::NativeDom,
        Vec<crate::DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) {
        let (document, _, inputs) =
            crate::parse_html_test_fixture_with_parser_outputs(final_url, html.into());
        (document, inputs)
    }

    fn preload_load_parameters(url: Url) -> ConnectedLoadParameters {
        ConnectedLoadParameters::PreloadLikeLink {
            url,
            options: Arc::new(ConnectedLinkReadinessFetchOptions {
                resource_type: SubresourceResourceType::Stylesheet,
                request_resource_type: Some(moli_fetch::RequestResourceType::CssStyleSheet),
                script_fetch_metadata: None,
                request_mode: moli_fetch::RequestMode::NoCors,
                credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                fetch_priority_hint: None,
                link_preload: true,
                link_fetch_options: StylesheetFetchOptions::default(),
            }),
        }
    }

    #[test]
    fn live_runtime_dom_host_forwards_the_canonical_document_base_url() {
        let document = HtmlParser.parse(
            Url::parse("https://example.test/path/page.html").unwrap(),
            "<!doctype html><html><head><base href=\"/assets/\"></head></html>".to_owned(),
        );
        let host = LiveRuntimeDomHost::from_dom_host(DomHost::from_dom(document));
        let expected = Url::parse("https://example.test/assets/").unwrap();

        assert_eq!(
            <LiveRuntimeDomHost as crate::planning::ParserPlanningReadView>::document_base_url_clone(
                &host,
            ),
            Some(expected.clone())
        );
        assert_eq!(
            <LiveRuntimeDomHost as crate::stylesheet_blocking::StylesheetBlockingReadView>::document_base_url_clone(
                &host,
            ),
            Some(expected)
        );
    }

    #[test]
    fn dom_host_builds_from_parsed_document_and_updates_text_content() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main>seed</main></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let div_handle = host.create_element("div");

        assert!(host.set_attribute(div_handle, "data-kind", "native"));
        assert!(host.set_text_content(div_handle, "child"));
        assert!(host.append_child(body_handle, div_handle));
        assert_eq!(
            host.get_attribute(div_handle, "data-kind"),
            Some("native".to_owned())
        );
        assert_eq!(host.text_content(body_handle).as_deref(), Some("seedchild"));
    }

    #[test]
    fn live_runtime_dom_host_direct_borrow_does_not_block_runtime_access() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main>seed</main></body></html>".to_owned(),
        );
        let mut live = LiveRuntimeDomHost::from_dom_host(DomHost::from_dom(document));
        let document_handle = live.document_handle();

        let child = live.create_element("div");

        assert!(live.append_child(document_handle, child));
        assert_eq!(live.borrow_mut().document_handle(), document_handle);
    }

    #[test]
    fn parser_reentry_state_tracks_script_pause_insertion_and_dynamic_markup() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::from_document(document);
        let document_handle = runtime.document_handle();

        assert_eq!(runtime.parser_script_nesting_level(), 0);
        assert_eq!(runtime.parser_pause_depth(), 0);
        assert_eq!(runtime.parser_insertion_session_depth(), 0);
        assert!(!runtime.is_parser_paused_for_reentry());
        assert!(!runtime.is_parser_insertion_session_active());
        assert!(!runtime.has_throw_on_dynamic_markup_insertion_counter(document_handle));
        assert!(runtime.should_checkpoint_before_parser_custom_element_constructor());
        {
            let _outer = runtime.enter_parser_script_nesting();
            assert_eq!(runtime.parser_script_nesting_level(), 1);
            assert!(!runtime.should_checkpoint_before_parser_custom_element_constructor());
            {
                let _inner = runtime.enter_parser_script_nesting();
                assert_eq!(runtime.parser_script_nesting_level(), 2);
                assert!(!runtime.should_checkpoint_before_parser_custom_element_constructor());
            }
            assert_eq!(runtime.parser_script_nesting_level(), 1);
            assert!(!runtime.should_checkpoint_before_parser_custom_element_constructor());
        }
        assert_eq!(runtime.parser_script_nesting_level(), 0);
        assert!(runtime.should_checkpoint_before_parser_custom_element_constructor());

        {
            let _pause = runtime.enter_parser_pause();
            assert_eq!(runtime.parser_pause_depth(), 1);
            assert!(runtime.is_parser_paused_for_reentry());
            assert!(runtime.should_checkpoint_before_parser_custom_element_constructor());
            {
                let _nested_pause = runtime.enter_parser_pause();
                assert_eq!(runtime.parser_pause_depth(), 2);
                assert!(runtime.is_parser_paused_for_reentry());
            }
            assert_eq!(runtime.parser_pause_depth(), 1);
        }
        assert_eq!(runtime.parser_pause_depth(), 0);
        assert!(!runtime.is_parser_paused_for_reentry());

        {
            let _session = runtime.enter_parser_insertion_session();
            assert_eq!(runtime.parser_insertion_session_depth(), 1);
            assert!(runtime.is_parser_insertion_session_active());
            assert!(runtime.should_checkpoint_before_parser_custom_element_constructor());
            {
                let _nested_session = runtime.enter_parser_insertion_session();
                assert_eq!(runtime.parser_insertion_session_depth(), 2);
                assert!(runtime.is_parser_insertion_session_active());
            }
            assert_eq!(runtime.parser_insertion_session_depth(), 1);
        }
        assert_eq!(runtime.parser_insertion_session_depth(), 0);
        assert!(!runtime.is_parser_insertion_session_active());

        runtime.enter_throw_on_dynamic_markup_insertion(document_handle);
        assert!(runtime.has_throw_on_dynamic_markup_insertion_counter(document_handle));
        runtime.enter_throw_on_dynamic_markup_insertion(document_handle);
        assert!(runtime.has_throw_on_dynamic_markup_insertion_counter(document_handle));
        runtime.exit_throw_on_dynamic_markup_insertion(document_handle);
        assert!(runtime.has_throw_on_dynamic_markup_insertion_counter(document_handle));
        runtime.exit_throw_on_dynamic_markup_insertion(document_handle);
        assert!(!runtime.has_throw_on_dynamic_markup_insertion_counter(document_handle));
    }

    #[test]
    fn structural_mutation_guard_tracks_nested_raw_tree_splices() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::from_document(document);

        assert!(!runtime.is_structural_mutation_active());
        {
            let _outer = runtime.enter_structural_mutation();
            assert!(runtime.is_structural_mutation_active());
            {
                let _inner = runtime.enter_structural_mutation();
                assert!(runtime.is_structural_mutation_active());
            }
            assert!(runtime.is_structural_mutation_active());
        }
        assert!(!runtime.is_structural_mutation_active());

        let body = runtime
            .dom_host
            .document_body_handle()
            .expect("body handle");
        let child = runtime.dom_host_mut().create_element("div");
        let effects = runtime.append_child_effects_in_structural_scope(body, child);
        assert!(effects.did_change());
        assert!(!runtime.is_structural_mutation_active());
    }

    #[test]
    fn parser_runtime_dom_generation_allows_nested_steps_on_same_document_generation() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main>seed</main></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::from_document(document);

        runtime.begin_dom_host_parse_step();
        let outer_generation = runtime.active_parser_step_generation();
        runtime.begin_dom_host_parse_step();
        assert_eq!(runtime.active_parser_step_generation(), outer_generation);
        assert_eq!(runtime.active_parser_step_depth(), Some(2));

        runtime.finish_dom_host_parse_step();
        assert_eq!(runtime.active_parser_step_generation(), outer_generation);
        assert_eq!(runtime.active_parser_step_depth(), Some(1));
        runtime.finish_dom_host_parse_step();

        assert_eq!(runtime.active_parser_step_generation(), None);
        assert_eq!(runtime.active_parser_step_depth(), None);
    }

    #[test]
    #[should_panic(expected = "current document generation")]
    fn parser_runtime_dom_generation_rejects_document_generation_replacement() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main>seed</main></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::from_document(document);

        runtime.begin_dom_host_parse_step();
        runtime.open_document();
        runtime.parser_runtime_dom_node_exists(runtime.document_handle());
    }

    #[test]
    fn dom_host_updates_tree_navigation_and_contains_relations() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main id=one>one</main><main id=two>two</main></body></html>"
                .to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let children = host.child_nodes(body_handle).expect("body children");
        let first = children[0];
        let second = children[1];
        let inserted = host.create_element("aside");

        assert_eq!(
            host.node(body_handle).and_then(Node::owner_document),
            Some(host.document_handle())
        );
        assert_eq!(
            host.node(first).and_then(Node::parent_node),
            Some(body_handle)
        );
        assert_eq!(
            host.node(body_handle).and_then(Node::first_child),
            Some(first)
        );
        assert_eq!(
            host.node(body_handle).and_then(Node::last_child),
            Some(second)
        );
        assert_eq!(host.node(first).and_then(Node::next_sibling), Some(second));
        assert_eq!(host.node(second).and_then(Node::prev_sibling), Some(first));
        assert!(
            host.node(body_handle)
                .is_some_and(|node| node.contains(host.dom(), second))
        );
        assert!(
            host.node(first)
                .is_some_and(|node| node.contains(host.dom(), first))
        );

        assert!(host.insert_before(body_handle, inserted, Some(second)));
        assert_eq!(
            host.node(first).and_then(Node::next_sibling),
            Some(inserted)
        );
        assert_eq!(
            host.node(second).and_then(Node::prev_sibling),
            Some(inserted)
        );
        assert_eq!(
            host.child_nodes(body_handle)
                .expect("updated body children"),
            vec![first, inserted, second]
        );

        assert!(host.remove_child(body_handle, inserted));
        assert_eq!(
            host.child_nodes(body_handle)
                .expect("body children after removal"),
            vec![first, second]
        );
        assert_eq!(host.node(inserted).and_then(Node::parent_node), None);
        assert!(
            !host
                .node(inserted)
                .is_some_and(|node| node.contains(host.dom(), first))
        );
    }

    #[test]
    fn dom_host_reports_unified_mutation_effects() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let fragment = host.create_document_fragment();
        let script = host.create_element("script");

        let insert_script = host.append_child_effects(fragment, script);
        assert!(insert_script.did_change());
        assert!(insert_script.scripts().connected_roots().is_empty());
        assert_eq!(insert_script.scripts().updated_nodes(), &[script]);
        assert_eq!(insert_script.scripts().prepare_triggers().len(), 1);
        assert_eq!(
            insert_script.scripts().prepare_triggers()[0].handle(),
            script
        );
        assert!(insert_script.tree().disconnected_roots().is_empty());

        let connect_fragment = host.append_child_effects(body_handle, fragment);
        assert!(connect_fragment.did_change());
        assert!(connect_fragment.scripts().connected_roots().is_empty());
        assert_eq!(connect_fragment.scripts().updated_nodes(), &[script]);
        assert_eq!(connect_fragment.scripts().prepare_triggers().len(), 1);
        assert_eq!(
            connect_fragment.scripts().prepare_triggers()[0].handle(),
            script
        );
        assert!(connect_fragment.tree().disconnected_roots().is_empty());

        let update_script = host.set_attribute_effects(script, "src", "/dynamic.js");
        assert!(update_script.did_change());
        assert!(update_script.scripts().connected_roots().is_empty());
        assert_eq!(update_script.scripts().updated_nodes(), &[script]);
        assert_eq!(update_script.scripts().prepare_triggers().len(), 1);
        assert_eq!(
            update_script.scripts().prepare_triggers()[0].handle(),
            script
        );

        let disconnect_script = host.remove_child_effects(body_handle, script);
        assert!(disconnect_script.did_change());
        assert!(disconnect_script.scripts().connected_roots().is_empty());
        assert!(disconnect_script.scripts().updated_nodes().is_empty());
        assert!(disconnect_script.scripts().prepare_triggers().is_empty());
        assert_eq!(disconnect_script.tree().disconnected_roots(), &[script]);
    }

    #[test]
    fn dom_host_uses_connected_script_roots_only_for_non_script_subtrees() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let wrapper = host.create_element("div");
        let script = host.create_element("script");

        let insert_script = host.append_child_effects(wrapper, script);
        assert!(insert_script.scripts().connected_roots().is_empty());
        assert_eq!(insert_script.scripts().prepare_triggers().len(), 1);
        assert_eq!(
            insert_script.scripts().prepare_triggers()[0].handle(),
            script
        );

        let connect_wrapper = host.append_child_effects(body_handle, wrapper);
        assert_eq!(connect_wrapper.scripts().connected_roots(), &[wrapper]);
        assert!(connect_wrapper.scripts().prepare_triggers().is_empty());
    }

    #[test]
    fn dom_host_scans_connected_shadow_trees_for_scripts() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let shadow_host = host.create_element("div");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let script = host.create_element("script");

        let insert_script = host.append_child_effects(shadow_root, script);
        assert!(insert_script.did_change());
        assert_eq!(insert_script.scripts().prepare_triggers().len(), 1);
        assert_eq!(
            insert_script.scripts().prepare_triggers()[0].handle(),
            script
        );

        let connect_host = host.append_child_effects(body_handle, shadow_host);
        assert_eq!(connect_host.scripts().connected_roots(), &[shadow_host]);
        assert_eq!(host.connected_script_handles(shadow_host), vec![script]);
    }

    #[test]
    fn dom_host_flattens_nested_slot_fallback_without_losing_siblings() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let shadow_host = host.create_element("section");
        assert!(host.append_child(body_handle, shadow_host));

        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let outer_slot = host.create_element("slot");
        let inner_slot = host.create_element("slot");
        let fallback_text = host.create_text_node("hello");
        let fallback_span = host.create_element("span");

        assert!(host.append_child(shadow_root, outer_slot));
        assert!(host.append_child(outer_slot, inner_slot));
        assert!(host.append_child(inner_slot, fallback_text));
        assert!(host.append_child(outer_slot, fallback_span));

        assert_eq!(
            host.assigned_nodes_for_slot_with_options(outer_slot, false),
            Vec::new()
        );
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(outer_slot, true),
            vec![fallback_text, fallback_span]
        );
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(inner_slot, false),
            Vec::new()
        );
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(inner_slot, true),
            vec![fallback_text]
        );
    }

    #[test]
    fn dom_host_slot_mutation_effects_mark_old_and_new_slots() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let shadow_host = host.create_element("section");
        assert!(host.append_child(body_handle, shadow_host));

        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let slot_a = host.create_element("slot");
        let slot_b = host.create_element("slot");
        let child = host.create_element("div");

        assert!(host.set_attribute(slot_a, "name", "a"));
        assert!(host.set_attribute(slot_b, "name", "b"));
        assert!(host.append_child(shadow_root, slot_a));
        assert!(host.append_child(shadow_root, slot_b));
        assert!(host.set_attribute(child, "slot", "a"));

        let insert_effects = host.append_child_effects(shadow_host, child);
        assert_eq!(insert_effects.slots().changed_slots(), &[slot_a]);
        assert_eq!(host.assigned_slot_for_node(child), Some(slot_a));

        let retarget_effects = host.set_attribute_effects(child, "slot", "b");
        let retargeted: HashSet<_> = retarget_effects
            .slots()
            .changed_slots()
            .iter()
            .map(|handle| handle.index())
            .collect();
        assert_eq!(retargeted, HashSet::from([slot_a.index(), slot_b.index()]));
        assert_eq!(host.assigned_slot_for_node(child), Some(slot_b));

        let remove_effects = host.remove_child_effects(shadow_host, child);
        assert_eq!(remove_effects.slots().changed_slots(), &[slot_b]);
        assert_eq!(host.assigned_slot_for_node(child), None);
    }

    #[test]
    fn dom_host_move_effects_mark_shadow_slot_changes() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let shadow_host = host.create_element("section");
        assert!(host.append_child(body_handle, shadow_host));

        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let slot = host.create_element("slot");
        let slottable = host.create_element("p");
        assert!(host.set_attribute(slot, "name", "content"));
        assert!(host.set_attribute(slottable, "slot", "content"));
        assert!(host.append_child(shadow_root, slot));
        assert!(host.append_child(shadow_host, slottable));
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(slot, false),
            vec![slottable]
        );

        let move_out_effects = host.insert_before_effects(body_handle, slottable, None);
        assert_eq!(move_out_effects.slots().changed_slots(), &[slot]);
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(slot, false),
            Vec::new()
        );

        let move_in_effects = host.insert_before_effects(shadow_host, slottable, None);
        assert_eq!(move_in_effects.slots().changed_slots(), &[slot]);
        assert_eq!(
            host.assigned_nodes_for_slot_with_options(slot, false),
            vec![slottable]
        );

        let fallback = host.create_element("span");
        assert!(host.append_child(shadow_root, fallback));
        let fallback_in_effects = host.insert_before_effects(slot, fallback, None);
        assert_eq!(fallback_in_effects.slots().changed_slots(), &[slot]);

        let fallback_out_effects = host.insert_before_effects(shadow_root, fallback, None);
        assert_eq!(fallback_out_effects.slots().changed_slots(), &[slot]);

        let slot_out_effects = host.insert_before_effects(body_handle, slot, None);
        assert_eq!(slot_out_effects.slots().changed_slots(), &[slot]);

        let slot_in_effects = host.insert_before_effects(shadow_root, slot, None);
        assert_eq!(slot_in_effects.slots().changed_slots(), &[slot]);
    }

    #[test]
    fn document_runtime_replace_live_document_preserves_shadow_root_bindings() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body_handle = document.document_body_handle().expect("body handle");
        let shadow_host = runtime.dom_host_mut().create_element("section");
        assert!(
            runtime
                .dom_host_mut()
                .append_child(body_handle, shadow_host)
        );

        let shadow_root = runtime
            .dom_host_mut()
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let shadow_child = runtime.dom_host_mut().create_element("span");
        assert!(
            runtime
                .dom_host_mut()
                .append_child(shadow_root, shadow_child)
        );

        let parser_snapshot = runtime.snapshot_document();
        runtime.replace_live_document(&parser_snapshot);

        assert_eq!(
            runtime.dom_host().shadow_root_handle(shadow_host),
            Some(shadow_root)
        );
        assert_eq!(
            runtime
                .dom_host()
                .child_handles(shadow_root)
                .collect::<Vec<_>>(),
            vec![shadow_child]
        );
    }

    #[test]
    fn document_runtime_replace_live_document_preserves_script_already_started() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script src=\"/app.js\"></script></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let script = document
            .script_handles()
            .first()
            .copied()
            .expect("parser-created script handle");

        assert!(
            runtime
                .dom_host_mut()
                .set_script_already_started(script, true),
            "live runtime should allow marking parser-created script as already-started"
        );
        let replacement_document = document.clone();
        runtime.replace_live_document(&replacement_document);

        assert!(
            runtime
                .snapshot_document()
                .node(script)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.script_already_started()),
            "document replacement should preserve runtime-owned already-started state"
        );
    }

    #[test]
    fn document_runtime_can_mark_script_already_started_by_node_id() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script src=\"/app.js\"></script></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let script = document
            .script_handles()
            .first()
            .copied()
            .expect("parser-created script handle");
        let node_id = NodeId::new(script.index());

        assert!(
            runtime.mark_script_already_started_by_node_id(node_id),
            "marking by node id should resolve the live runtime script handle"
        );
        assert!(
            runtime
                .snapshot_document()
                .node(script)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.script_already_started()),
            "marking by node id should project into the live DOM snapshot"
        );
    }

    #[test]
    fn document_runtime_sets_parser_script_force_async_like_chrome() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script></script><script async></script></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let parser_scripts = document.script_handles();
        let parser_script = parser_scripts
            .first()
            .copied()
            .expect("parser-created script handle");
        let parser_async_script = parser_scripts
            .get(1)
            .copied()
            .expect("parser-created async script handle");
        let dynamic_script = runtime.dom_host_mut().create_element("script");

        assert!(
            !runtime
                .dom_host()
                .node(parser_script)
                .and_then(Node::as_element)
                .is_some_and(Element::script_async)
        );
        assert!(
            runtime
                .dom_host()
                .node(parser_async_script)
                .and_then(Node::as_element)
                .is_some_and(Element::script_async)
        );
        assert!(
            runtime
                .dom_host()
                .node(dynamic_script)
                .and_then(Node::as_element)
                .is_some_and(Element::script_async)
        );
    }

    #[test]
    fn document_runtime_snapshot_projects_live_dom_without_reparse() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body><main id=seed>seed</main></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body_handle = document.document_body_handle().expect("body handle");
        let script = runtime.dom_host_mut().create_element("script");
        let text = runtime.dom_host_mut().create_text_node("console.log('ok')");
        let section = runtime.dom_host_mut().create_element("section");

        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(section, "data-state", "live")
        );
        assert!(runtime.dom_host_mut().set_text_content(section, "fresh"));
        assert!(runtime.dom_host_mut().append_child(body_handle, section));
        assert!(runtime.dom_host_mut().append_child(script, text));
        assert!(runtime.dom_host_mut().append_child(body_handle, script));

        let snapshot = runtime.snapshot_document();
        let body_id = snapshot.body_node_id().expect("snapshot body");
        let body_html = snapshot.outer_html(body_id).expect("snapshot body html");

        assert!(snapshot.parse_errors().is_empty());
        assert!(body_html.contains("<section data-state=\"live\">fresh</section>"));
        assert!(body_html.contains("<script>console.log('ok')</script>"));
        assert_eq!(snapshot.script_node_ids().len(), 1);
    }

    #[test]
    fn parser_roundtrip_through_live_runtime_keeps_style_import_text_before_later_script() {
        let parser = HtmlParser;
        let mut stream = parser.start_document(Url::parse("https://example.com/").unwrap());
        let html = "<!doctype html><html><head><script>window.start = 1;</script><style>@import url('/slow.css');</style><script>window.afterStyle = 1;</script><script src='/blocking.js'></script></head><body><div id='late'>late</div></body></html>";

        let crate::parser::ParserPumpStep::Yield(crate::parser::ParserYield::Script(_)) =
            stream.pump_parser_step(html).result
        else {
            panic!("expected first script handoff");
        };
        let first_document = stream.take_parser_stream_dom_host().into_dom();
        let mut runtime = DocumentRuntime::new(&first_document);
        runtime.replace_live_document_with_document(first_document);
        stream.restore_parser_stream_dom_host(DomHost::from_dom(runtime.snapshot_document()));

        let crate::parser::ParserPumpStep::Yield(crate::parser::ParserYield::Script(_)) =
            stream.pump_parser_step("").result
        else {
            panic!("expected second script handoff");
        };
        let second_document = stream.take_parser_stream_dom_host().into_dom();
        runtime.replace_live_document_with_document(second_document);
        stream.restore_parser_stream_dom_host(DomHost::from_dom(runtime.snapshot_document()));

        let crate::parser::ParserPumpStep::Yield(crate::parser::ParserYield::Script(third_handoff)) =
            stream.pump_parser_step("").result
        else {
            panic!("expected third script handoff");
        };
        let third_handoff = third_handoff.node_id();
        let third_snapshot = stream.snapshot_parser_stream_document();
        let style_node_id = third_snapshot
            .nodes()
            .iter()
            .find(|node| node.is_html_element_named("style"))
            .map(|node| node.id())
            .expect("expected style element");

        assert_eq!(
            third_snapshot.text_content(style_node_id).as_deref(),
            Some("@import url('/slow.css');")
        );
        assert_eq!(
            third_snapshot.script_src(third_handoff),
            Some("/blocking.js")
        );
        assert!(
            third_snapshot
                .node(third_handoff)
                .is_some_and(|node| node.flags().parser_created()),
            "external script should remain parser-created after live roundtrip"
        );
        assert!(
            third_snapshot.document_body_handle().is_none(),
            "later body content should still be hidden at the external script handoff"
        );
    }

    #[test]
    fn parser_roundtrip_after_live_dom_mutation_keeps_later_body_hidden() {
        let parser = HtmlParser;
        let mut stream = parser.start_document(Url::parse("https://example.com/").unwrap());
        let html = "<!doctype html><html><head><script>window.start = 1;</script><script>window.after = 1;</script></head><body><div id='late'>late</div></body></html>";

        let crate::parser::ParserPumpStep::Yield(crate::parser::ParserYield::Script(_)) =
            stream.pump_parser_step(html).result
        else {
            panic!("expected first script handoff");
        };
        let first_document = stream.take_parser_stream_dom_host().into_dom();
        let mut runtime = DocumentRuntime::new(&first_document);
        runtime.replace_live_document_with_document(first_document);

        let head = runtime
            .dom_host()
            .document_head_handle()
            .expect("head should exist before runtime mutation");
        let link = runtime.dom_host_mut().create_element("link");
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(link, "rel", "stylesheet")
        );
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(link, "href", "/runtime.css")
        );
        assert!(runtime.dom_host_mut().append_child(head, link));

        stream.restore_parser_stream_dom_host(DomHost::from_dom(runtime.snapshot_document()));

        let crate::parser::ParserPumpStep::Yield(crate::parser::ParserYield::Script(
            second_handoff,
        )) = stream.pump_parser_step("").result
        else {
            panic!("expected second script handoff");
        };
        let second_handoff = second_handoff.node_id();
        let second_snapshot = stream.snapshot_parser_stream_document();
        assert_eq!(
            second_snapshot.script_text(second_handoff).as_deref(),
            Some("window.after = 1;")
        );
        assert!(
            second_snapshot.document_body_handle().is_none(),
            "later body content should remain hidden after live DOM mutation roundtrip"
        );
    }

    #[test]
    fn precedes_in_document_order_handles_disconnected_roots_without_underflow() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><div id=live></div></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let live = runtime
            .dom_host()
            .element_handle_by_id("live")
            .expect("live node in document");
        let fragment = runtime.dom_host_mut().create_document_fragment();
        let detached = runtime.dom_host_mut().create_element("div");

        assert!(runtime.dom_host_mut().append_child(fragment, detached));
        assert!(!runtime.precedes_in_document_order(live, detached));
        assert!(!runtime.precedes_in_document_order(detached, live));
    }

    #[test]
    fn dom_host_import_and_adopt_node_preserve_owner_document_and_parentage() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><div id=container><span id=seed>seed</span></div></body></html>"
                .to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let document_handle = document.document_node_id();
        let container = host.element_handle_by_id("container").expect("container");
        let seed = host.element_handle_by_id("seed").expect("seed");

        let imported = host
            .import_node(document_handle, container, true)
            .expect("imported clone");
        assert_ne!(imported, container);
        assert_eq!(
            host.node(imported).and_then(Node::owner_document),
            Some(document_handle)
        );
        assert_eq!(host.node(imported).and_then(Node::parent_node), None);
        let imported_children = host.child_nodes(imported).expect("imported children");
        assert_eq!(imported_children.len(), 1);
        assert_ne!(imported_children[0], seed);
        assert_eq!(
            host.node(imported_children[0])
                .and_then(Node::owner_document),
            Some(document_handle)
        );

        assert!(host.remove_child(container, seed));
        let adopted = host
            .adopt_node(document_handle, seed)
            .expect("adopted node");
        assert_eq!(adopted, seed);
        assert_eq!(host.node(seed).and_then(Node::parent_node), None);
        assert_eq!(
            host.node(seed).and_then(Node::owner_document),
            Some(document_handle)
        );
    }

    #[test]
    fn dom_host_rebuilds_id_lookup_after_id_attribute_mutation() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main id=one>one</main><main>two</main></body></html>"
                .to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let children = host.child_nodes(body_handle).expect("body children");
        let first = children[0];
        let second = children[1];

        assert_eq!(host.element_handle_by_id("one"), Some(first));

        assert!(host.set_attribute(second, "id", "two"));
        assert_eq!(host.element_handle_by_id("two"), Some(second));

        assert!(host.remove_attribute(first, "id"));
        assert_eq!(host.element_handle_by_id("one"), None);
    }

    #[test]
    fn dom_host_invalidates_live_collection_cache_on_tree_and_attribute_mutations() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><main></main></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let body_handle = document.document_body_handle().expect("body handle");
        let main = host
            .child_nodes(body_handle)
            .expect("body children")
            .into_iter()
            .next()
            .expect("seed child");

        assert_eq!(
            host.resolve_live_collection(body_handle, "children", None, false)
                .expect("children collection")
                .len(),
            1
        );

        let aside = host.create_element("aside");
        assert!(host.append_child(body_handle, aside));
        assert_eq!(
            host.resolve_live_collection(body_handle, "children", None, false)
                .expect("children collection after append")
                .len(),
            2
        );

        assert_eq!(
            host.resolve_live_collection(body_handle, "className", Some("active"), false)
                .expect("class collection before attr change")
                .len(),
            0
        );
        assert!(host.set_attribute(main, "class", "active"));
        assert_eq!(
            host.resolve_live_collection(body_handle, "className", Some("active"), false)
                .expect("class collection after attr change")
                .len(),
            1
        );
    }

    #[test]
    fn dom_host_normalize_merges_adjacent_text_nodes_recursively() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><div id=container></div></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let container = host.element_handle_by_id("container").expect("container");

        let first = host.create_text_node("a");
        let empty = host.create_text_node("");
        let second = host.create_text_node("b");
        let nested = host.create_element("section");
        let nested_first = host.create_text_node("x");
        let nested_empty = host.create_text_node("");
        let nested_second = host.create_text_node("y");

        assert!(host.append_child(container, first));
        assert!(host.append_child(container, empty));
        assert!(host.append_child(container, second));
        assert!(host.append_child(container, nested));
        assert!(host.append_child(nested, nested_first));
        assert!(host.append_child(nested, nested_empty));
        assert!(host.append_child(nested, nested_second));

        let effects = host.normalize_effects(container);
        assert!(effects.did_change());

        let children = host.child_nodes(container).expect("container children");
        assert_eq!(children.len(), 2);
        assert_eq!(host.text_content(children[0]).as_deref(), Some("ab"));
        let nested_children = host.child_nodes(children[1]).expect("nested children");
        assert_eq!(nested_children.len(), 1);
        assert_eq!(host.text_content(nested_children[0]).as_deref(), Some("xy"));
    }

    #[tokio::test]
    async fn stylesheet_typed_task_is_ready_for_document_processing() {
        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=\"data:text/css,body%7B%7D\"></head><body></body></html>"
                .to_owned(),
        );
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let link = first_element_handle(&document, "link");
        let stylesheet_url = Url::parse("data:text/css,body%7B%7D").unwrap();
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );
        runtime
            .stylesheet_lifecycle
            .fetches
            .enqueue_completion_for_testing(
                crate::dom::NodeId::new(link.index()),
                stylesheet_url,
                true,
            );

        assert_eq!(
            runtime
                .observe_document_processing_wake(&mut task_queue)
                .await,
            DocumentProcessingWakeObservation::ReadyNow,
            "a stylesheet completion published before observation must remain visible as typed Networking work"
        );
    }

    #[tokio::test]
    async fn injected_page_task_can_wake_parse_time_turn_without_timeout_fallback() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let sender = task_queue.parser_boundary_sender();

        tokio::spawn(async move {
            let _ = sender.send(PageTask::DispatchDomContentLoaded);
        });

        assert_eq!(
            runtime
                .wait_for_parse_time_turn_arrival(&mut task_queue)
                .await,
            ParseTimeWakeObservation::Arrived(ParseTimeWakeSource::InjectedPageTask)
        );
        task_queue.accept_ready_parse_time_wakes();
        assert!(matches!(
            task_queue.parse_time_pop_front(),
            Some(PageTask::DispatchDomContentLoaded)
        ));
    }

    #[tokio::test]
    async fn stylesheet_completion_with_link_owner_surfaces_ready_parse_time_work() {
        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=\"data:text/css,body%7B%7D\"></head><body></body></html>"
                .to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let head = document.document_head_handle().expect("head handle");
        let link = document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("stylesheet link");
        let link_node_id = crate::dom::NodeId::new(link.index());
        let stylesheet_url = Url::parse("data:text/css,body%7B%7D").unwrap();
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );
        runtime
            .stylesheet_lifecycle
            .fetches
            .enqueue_completion_for_testing(link_node_id, stylesheet_url, true);

        assert_eq!(
            runtime
                .wait_for_parse_time_turn_arrival(&mut task_queue)
                .await,
            ParseTimeWakeObservation::ReadyNow,
            "the resource wake is not a parse-time source, but draining it makes the link event ready"
        );
    }

    #[tokio::test]
    async fn draining_blocking_stylesheet_completion_persists_resolved_state() {
        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/app.css></head><body></body></html>"
                .to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );
        assert!(!runtime.has_all_blocking_stylesheets_resolved());

        let node_id = NodeId::new(first_element_handle(&document, "link").index());
        let stylesheet_url = Url::parse("https://example.com/app.css").unwrap();
        runtime
            .stylesheet_lifecycle
            .fetches
            .enqueue_completion_for_testing(node_id, stylesheet_url, true);

        runtime.drain_blocking_stylesheet_completions();

        assert!(
            runtime.has_all_blocking_stylesheets_resolved(),
            "drained blocking stylesheet completions should persist as resolved state"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn response_csp_blocks_ownerless_stylesheet_admission() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        runtime.set_response_content_security_policies(&["style-src 'none'".to_owned()]);

        assert!(
            runtime
                .preload_stylesheet(
                    Url::parse("https://example.com/speculative.css").unwrap(),
                    crate::stylesheet_blocking::StylesheetFetchOptions::default(),
                )
                .is_none(),
            "response policy admission must run before creating an ownerless resource"
        );
        assert!(runtime.take_ready_stylesheet_network_results().is_empty());
    }

    #[tokio::test]
    async fn typed_stylesheet_claim_preserves_blocking_resolution_state() {
        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/app.css></head><body></body></html>"
                .to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        let node_id = NodeId::new(first_element_handle(&document, "link").index());
        let stylesheet_url = Url::parse("https://example.com/app.css").unwrap();
        runtime
            .stylesheet_lifecycle
            .fetches
            .enqueue_completion_for_testing(node_id, stylesheet_url, true);

        assert!(
            runtime.wait_for_stylesheet_networking_task_for_test().await,
            "the exact blocking terminal must remain resident in the typed Networking source"
        );
        assert!(
            runtime.apply_next_stylesheet_networking_task_for_test(),
            "the test must claim the same typed task consumed by a live Page"
        );

        assert!(
            runtime.has_all_blocking_stylesheets_resolved(),
            "claiming the typed task must persist the canonical blocking resolution"
        );
    }

    #[test]
    fn stale_connected_stylesheet_network_result_releases_source_owner() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/current.css></head><body></body></html>"
                .to_owned(),
        );
        let link = first_element_handle(&document, "link");
        let stale_url = Url::parse("https://example.com/old.css").unwrap();
        let mut runtime = DocumentRuntime::new(&document);

        runtime
            .stylesheet_lifecycle
            .ready_connected_load_network_results
            .push_back(ConnectedLoadNetworkResult {
                stylesheet_fetch: None,
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document.final_url().unwrap().clone(),
                request_url: stale_url.clone(),
                source_owners: vec![link],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("stale connected stylesheet result".to_owned()),
            });

        let results = runtime.take_ready_stylesheet_network_results();

        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap();
        assert_eq!(result.request_url, stale_url);
        assert!(
            result.source_owners.is_empty(),
            "stale connected stylesheet results must not retain a source owner"
        );
    }

    #[tokio::test]
    async fn active_link_load_keeps_physical_observation_separate_from_client_terminal() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/current.css></head><body></body></html>"
                .to_owned(),
        );
        let link = first_element_handle(&document, "link");
        let current_url = Url::parse("https://example.com/current.css").unwrap();
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_document_lifecycle_processing();
        let active_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("stylesheet processing should create an owner-bound load");

        runtime
            .stylesheet_lifecycle
            .ready_connected_load_network_results
            .push_back(ConnectedLoadNetworkResult {
                stylesheet_fetch: Some(active_load.fetch().clone()),
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document.final_url().unwrap().clone(),
                request_url: current_url.clone(),
                source_owners: vec![link],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("current connected stylesheet result".to_owned()),
            });

        let results = runtime.take_ready_stylesheet_network_results();

        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap();
        assert_eq!(result.request_url, current_url);
        assert!(
            result.source_owners.is_empty(),
            "typed physical observations must not double as stylesheet client delivery"
        );
    }

    #[tokio::test]
    async fn removed_link_rejects_its_detached_load_network_source_owner() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/current.css></head><body></body></html>"
                .to_owned(),
        );
        let link = first_element_handle(&document, "link");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_document_lifecycle_processing();
        let detached_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("active stylesheet load");
        let parent = runtime
            .dom_host()
            .parent_node(link)
            .expect("link should have a parent");
        assert!(runtime.dom_host_mut().remove_child(parent, link));
        runtime.invalidate_style_related_state(std::ptr::null_mut(), link);

        runtime
            .stylesheet_lifecycle
            .ready_connected_load_network_results
            .push_back(ConnectedLoadNetworkResult {
                stylesheet_fetch: Some(detached_load.fetch().clone()),
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: detached_load.fetch().document_url().clone(),
                request_url: detached_load.request_url().clone(),
                source_owners: vec![link],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("detached load result".to_owned()),
            });

        let result = runtime
            .take_ready_stylesheet_network_results()
            .into_iter()
            .next()
            .expect("resource observation result");
        assert!(result.source_owners.is_empty());
    }

    #[tokio::test]
    async fn disabled_link_rejects_its_detached_load_network_source_owner() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/current.css></head><body></body></html>"
                .to_owned(),
        );
        let link = first_element_handle(&document, "link");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_document_lifecycle_processing();
        let detached_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("active stylesheet load");
        assert!(runtime.dom_host_mut().set_attribute(link, "disabled", ""));
        runtime.invalidate_style_related_state(std::ptr::null_mut(), link);
        runtime.queue_connected_style_loads(link);
        runtime.prime_document_lifecycle_processing();
        assert!(
            runtime
                .active_stylesheet_link_client_for_test(link)
                .is_none()
        );

        runtime
            .stylesheet_lifecycle
            .ready_connected_load_network_results
            .push_back(ConnectedLoadNetworkResult {
                stylesheet_fetch: Some(detached_load.fetch().clone()),
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: detached_load.fetch().document_url().clone(),
                request_url: detached_load.request_url().clone(),
                source_owners: vec![link],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("disabled load result".to_owned()),
            });

        let result = runtime
            .take_ready_stylesheet_network_results()
            .into_iter()
            .next()
            .expect("resource observation result");
        assert!(result.source_owners.is_empty());
    }

    #[tokio::test]
    async fn same_url_reprocess_rejects_the_detached_owner_bound_load_object() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/a.css></head><body></body></html>"
                .to_owned(),
        );
        let link = first_element_handle(&document, "link");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_document_lifecycle_processing();
        let first_a_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("first A processing load");

        runtime.invalidate_style_related_state(std::ptr::null_mut(), link);
        assert!(runtime.dom_host_mut().set_attribute(link, "href", "/b.css"));
        runtime.queue_connected_style_loads(link);
        runtime.prime_document_lifecycle_processing();
        let b_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("B processing load");

        runtime.invalidate_style_related_state(std::ptr::null_mut(), link);
        assert!(runtime.dom_host_mut().set_attribute(link, "href", "/a.css"));
        runtime.queue_connected_style_loads(link);
        runtime.prime_document_lifecycle_processing();
        let second_a_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("second A processing load");

        assert!(!super::stylesheet_runtime::StylesheetLinkClient::ptr_eq(
            &first_a_load,
            &b_load
        ));
        assert!(!super::stylesheet_runtime::StylesheetLinkClient::ptr_eq(
            &first_a_load,
            &second_a_load
        ));
        assert_eq!(first_a_load.request_url(), second_a_load.request_url());
        assert!(
            first_a_load.fetch().ptr_eq(second_a_load.fetch()),
            "a new owner processing client should reuse the compatible document resource"
        );
        assert!(
            !runtime.accept_stylesheet_link_client_completion_for_test(link, &first_a_load, true,),
            "an old A load object must not complete the new A processing"
        );
    }

    #[tokio::test]
    async fn linked_stylesheet_request_attribute_reprocess_captures_new_fetch_options() {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.com/page").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='http://127.0.0.1:9/a.css' crossorigin=anonymous integrity=sha256-first referrerpolicy=no-referrer fetchpriority=low></head><body></body></html>"
                .to_owned(),
        );
        let link = first_element_handle(&document, "link");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_document_lifecycle_processing();
        let first_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("first processing load");
        assert_eq!(
            first_load.fetch().options().cross_origin(),
            Some("anonymous")
        );
        assert_eq!(
            first_load.fetch().options().integrity(),
            Some("sha256-first")
        );

        runtime.invalidate_style_related_state(std::ptr::null_mut(), link);
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(link, "crossorigin", "use-credentials")
        );
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(link, "integrity", "sha256-second")
        );
        runtime.queue_connected_style_loads(link);
        runtime.prime_document_lifecycle_processing();
        let second_load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("reprocessed load");

        assert!(!super::stylesheet_runtime::StylesheetLinkClient::ptr_eq(
            &first_load,
            &second_load
        ));
        assert!(!first_load.fetch().ptr_eq(second_load.fetch()));
        assert_eq!(
            second_load.fetch().options().cross_origin(),
            Some("use-credentials")
        );
        assert_eq!(
            second_load.fetch().options().integrity(),
            Some("sha256-second")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn link_blocking_stylesheet_fetch_is_shared_across_document_lifecycle_state() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::{Duration, timeout},
        };

        let hits = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_hits = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer).await.unwrap();
            server_hits.fetch_add(1, Ordering::SeqCst);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: 20\r\nConnection: close\r\n\r\nbody { color: red; }",
                )
                .await
                .unwrap();
        });

        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse(&format!("http://{addr}/page")).unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'><script defer src='/app.js'></script></head><body></body></html>".to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        let node_id = NodeId::new(first_element_handle(&document, "link").index());
        let stylesheet_url = Url::parse(&format!("http://{addr}/app.css")).unwrap();

        timeout(Duration::from_secs(2), async {
            loop {
                runtime.drain_blocking_stylesheet_completions();
                let shared_resolved = runtime
                    .stylesheet_lifecycle
                    .fetches
                    .status(node_id, &stylesheet_url)
                    .is_some();
                if shared_resolved && runtime.has_all_blocking_stylesheets_resolved() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("stylesheet fetch should resolve");

        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "link stylesheet fetch should be shared between connected-style and document-owned blocker state"
        );

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn link_stylesheet_fetch_respects_nosniff_mime_blocking() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::{Duration, timeout},
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            const BODY: &str = "body { color: red; }";
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer).await.unwrap();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        BODY.len(),
                        BODY
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse(&format!("http://{addr}/page")).unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'><script defer src='/app.js'></script></head><body></body></html>".to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        let node_id = NodeId::new(first_element_handle(&document, "link").index());
        let stylesheet_url = Url::parse(&format!("http://{addr}/app.css")).unwrap();

        let status = timeout(Duration::from_secs(2), async {
            loop {
                runtime.drain_blocking_stylesheet_completions();
                if let Some(status) = runtime
                    .stylesheet_lifecycle
                    .fetches
                    .status(node_id, &stylesheet_url)
                {
                    break status;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("stylesheet fetch should resolve");

        runtime.drain_blocking_stylesheet_completions();
        assert!(
            !status,
            "nosniff text/html stylesheet should fail readiness"
        );
        assert!(
            runtime.has_all_blocking_stylesheets_resolved(),
            "failed stylesheet readiness must unblock script scheduling"
        );

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parser_created_style_import_network_results_are_drained() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::{Duration, timeout},
        };

        const STYLESHEET_BODY: &str = "body { color: blue; }";

        let hits = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_hits = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer).await.unwrap();
            server_hits.fetch_add(1, Ordering::SeqCst);
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        STYLESHEET_BODY.len(),
                        STYLESHEET_BODY
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let document_url = Url::parse(&format!("http://{addr}/page")).unwrap();
        let parser = HtmlParser;
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><style>@import url('/imported.css');</style><script src='/app.js'></script></head><body></body></html>".to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let script_node_id = NodeId::new(document.script_handles()[0].index());
        let inputs = moli_stylesheet_blocking::collect_document_owned_blocking_stylesheets_before(
            &document,
            script_node_id,
        )
        .into_iter()
        .map(|blocker| crate::DocumentOwnedBlockingStylesheetDiscoveryInput::from(&blocker))
        .collect::<Vec<_>>();
        assert_eq!(
            inputs.len(),
            1,
            "expected one parser-created style import blocker"
        );
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(inputs.iter());

        let results = timeout(Duration::from_secs(2), async {
            loop {
                let results = runtime.take_ready_stylesheet_network_results();
                if !results.is_empty() {
                    break results;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("parser-created style import should resolve");

        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap();
        let result_document_url = result.document_url;
        let request_url = result.request_url;
        let source_owners = result.source_owners;
        let result = result.result;
        assert_eq!(result_document_url, document_url);
        assert_eq!(
            request_url,
            Url::parse(&format!("http://{addr}/imported.css")).unwrap()
        );
        assert!(
            source_owners.is_empty(),
            "physical parser-blocking terminals must not carry opportunistic live-owner authority"
        );
        let response = result.expect("stylesheet import should succeed");
        assert_eq!(response.status, 200);
        assert_eq!(response.body_text(), STYLESHEET_BODY);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parser_created_style_import_respects_nosniff_mime_blocking() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::{Duration, timeout},
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            const BODY: &str = "body { color: blue; }";
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer).await.unwrap();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        BODY.len(),
                        BODY
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let document_url = Url::parse(&format!("http://{addr}/page")).unwrap();
        let parser = HtmlParser;
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><style>@import url('/imported.css');</style><script src='/app.js'></script></head><body></body></html>".to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let script_node_id = NodeId::new(document.script_handles()[0].index());
        let inputs = moli_stylesheet_blocking::collect_document_owned_blocking_stylesheets_before(
            &document,
            script_node_id,
        )
        .into_iter()
        .map(|blocker| crate::DocumentOwnedBlockingStylesheetDiscoveryInput::from(&blocker))
        .collect::<Vec<_>>();
        assert_eq!(
            inputs.len(),
            1,
            "expected one parser-created style import blocker"
        );
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(inputs.iter());

        let results = timeout(Duration::from_secs(2), async {
            loop {
                let results = runtime.take_ready_stylesheet_network_results();
                if !results.is_empty() {
                    break results;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("parser-created style import should resolve");

        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap();
        let blocking_operation = result
            .blocking_operation
            .expect("parser-created import should retain its blocking operation");
        let request_url = result.request_url;
        let source_owners = result.source_owners;
        let result = result.result;
        assert_eq!(
            request_url,
            Url::parse(&format!("http://{addr}/imported.css")).unwrap()
        );
        assert!(
            source_owners.is_empty(),
            "an unusable physical terminal must remain separate from its one-shot install authority"
        );
        let response = result.expect("the HTTP response remains a physical network success");
        assert_eq!(response.status, 200);
        assert_eq!(
            runtime
                .stylesheet_lifecycle
                .fetches
                .status_for_blocking_operation(&blocking_operation),
            Some(crate::stylesheet_blocking::StylesheetBlockingStatus::Failed),
            "nosniff text/html must fail stylesheet usability while retaining the HTTP response"
        );
        assert!(
            runtime.has_all_blocking_stylesheets_resolved(),
            "failed import readiness must unblock script scheduling"
        );

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connected_style_import_failure_marks_style_load_error() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::{Duration, timeout},
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer).await.unwrap();
            const BODY: &str = "missing";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        BODY.len(),
                        BODY
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let document_url = Url::parse(&format!("http://{addr}/page")).unwrap();
        let parser = HtmlParser;
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><style>@import url('/missing.css');</style></head><body></body></html>"
                .to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new(&document);
        let document_loader = crate::network::context::DocumentResourceLoader::new(
            loader.clone(),
            crate::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("stylesheet test runtime"),
            crate::network::context::DocumentFetchContext::new(
                crate::native_bridge::WindowDocumentOwner::Frame(
                    super::runtime_core::test_stylesheet_document_owner(),
                ),
                document_url.clone(),
                document_url.clone(),
                moli_url::origin_ascii_serialization(&document_url),
            ),
        );
        runtime.install_standalone_document_resource_loader(&document_loader);
        let style = document
            .document_head_handle()
            .and_then(|head| document.child_nodes(head))
            .and_then(|children| {
                children.into_iter().find(|handle| {
                    document
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))
                })
            })
            .expect("style handle");

        runtime.queue_initial_connected_style_loads();
        runtime.prime_document_lifecycle_processing();
        assert!(
            timeout(
                Duration::from_secs(2),
                runtime.wait_for_stylesheet_networking_task_for_test()
            )
            .await
            .expect("connected style import should resolve"),
            "the stylesheet Networking source must remain open"
        );
        assert!(
            runtime.apply_ready_stylesheet_networking_tasks_for_test(),
            "the completed import must publish typed Networking work"
        );
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("the import completion must publish its typed style event");

        assert_eq!(ready.owner(), style);
        assert!(!ready.successful());
        let results = runtime.take_ready_stylesheet_network_results();
        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap();
        assert_eq!(
            result.request_url,
            Url::parse(&format!("http://{addr}/missing.css")).unwrap()
        );
        let response = result
            .result
            .expect("HTTP failure must retain its physical response");
        assert_eq!(response.status, 404);
        assert_eq!(response.body_text(), "missing");

        server.abort();
    }

    #[test]
    fn drained_preload_like_network_result_still_drives_load_event_result() {
        let parser = HtmlParser;
        let document_url = Url::parse("https://example.com/page").unwrap();
        let request_url = Url::parse("https://example.com/chunk.js").unwrap();
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><link rel=preload as=script href=/chunk.js></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let link = document
            .document_head_handle()
            .and_then(|head| document.child_nodes(head))
            .and_then(|children| {
                children.into_iter().find(|handle| {
                    document
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("link"))
                })
            })
            .expect("link handle");
        let operation = ConnectedLoadOperation::new_with_load_event_binding(
            link,
            crate::document_runtime::ConnectedStyleEventElementKind::Link,
            preload_load_parameters(request_url.clone()),
            None,
            Some(
                crate::frame_owner_model::MainDocumentStyleLoadEventBinding::unowned_for_document_runtime_test(
                    link,
                ),
            ),
        );
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&operation));
        runtime.apply_connected_style_load_completion(ConnectedLoadCompletion {
            operation,
            successful: false,
            network_results: vec![ConnectedLoadNetworkResult {
                stylesheet_fetch: None,
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document_url.clone(),
                request_url: request_url.clone(),
                source_owners: vec![link],
                resource_type: SubresourceResourceType::Script,
                start_unix_millis: None,
                origin_clean: true,
                result: Err("preload failed".to_owned()),
            }],
        });

        runtime.prime_document_lifecycle_processing();
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("failed preload event task");
        assert_eq!(ready.owner(), link);
        assert!(!ready.successful());

        let results = runtime.take_ready_stylesheet_network_results();
        assert_eq!(results.len(), 1);
        assert!(
            runtime
                .stylesheet_lifecycle
                .ready_connected_load_network_results
                .is_empty()
        );
    }

    #[test]
    fn same_parameter_connected_completion_cannot_cross_operation_identity() {
        let parser = HtmlParser;
        let document_url = Url::parse("https://example.com/page").unwrap();
        let request_url = Url::parse("https://example.com/a.css").unwrap();
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><link rel=preload as=style href=/a.css></head><body></body></html>"
                .to_owned(),
        );
        let owner = first_element_handle(&document, "link");
        let inline_source = Arc::new(crate::style_engine::OwnerStyleSheetSource::new(
            owner,
            "@import url(a.css);".to_owned(),
            document_url.clone(),
        ));

        for parameters in [
            preload_load_parameters(request_url.clone()),
            ConnectedLoadParameters::StyleImports {
                source: super::stylesheet_runtime::ConnectedStyleImportSource::Inline(Arc::clone(
                    &inline_source,
                )),
                urls: vec![request_url.clone()],
                roots: Vec::new(),
            },
        ] {
            let mut runtime = DocumentRuntime::new(&document);
            let first_a = ConnectedLoadOperation::new_for_test(
                owner,
                ConnectedStyleEventElementKind::Link,
                parameters.clone(),
                None,
            );
            let middle_b = ConnectedLoadOperation::new_for_test(
                owner,
                ConnectedStyleEventElementKind::Link,
                preload_load_parameters(Url::parse("https://example.com/b.css").unwrap()),
                None,
            );
            let current_a = ConnectedLoadOperation::new_for_test(
                owner,
                ConnectedStyleEventElementKind::Link,
                parameters,
                None,
            );

            runtime
                .stylesheet_lifecycle
                .owner_states
                .install_pending_operation(Arc::clone(&first_a));
            runtime
                .stylesheet_lifecycle
                .owner_states
                .install_pending_operation(middle_b);
            runtime
                .stylesheet_lifecycle
                .owner_states
                .install_pending_operation(Arc::clone(&current_a));
            runtime.apply_connected_style_load_completion(ConnectedLoadCompletion {
                operation: first_a,
                successful: true,
                network_results: vec![ConnectedLoadNetworkResult {
                    stylesheet_fetch: None,
                    blocking_operation: None,
                    source_operation: None,
                    import_roots: Vec::new(),
                    document_url: document_url.clone(),
                    request_url: request_url.clone(),
                    source_owners: vec![owner],
                    resource_type: SubresourceResourceType::Stylesheet,
                    start_unix_millis: Some(1.0),
                    origin_clean: true,
                    result: Err("stale A completion".to_owned()),
                }],
            });

            assert!(
                runtime
                    .stylesheet_lifecycle
                    .owner_states
                    .pending_operation(owner)
                    .is_some_and(|pending| ConnectedLoadOperation::ptr_eq(pending, &current_a)),
                "the new A processing object must remain pending"
            );
            assert!(
                runtime
                    .stylesheet_lifecycle
                    .injected_ready_connected_loads
                    .is_empty()
            );
            let stale_result = runtime
                .stylesheet_lifecycle
                .ready_connected_load_network_results
                .pop_front()
                .expect("stale completion still records network observation");
            assert!(stale_result.source_owners.is_empty());
        }
    }

    #[test]
    fn accepted_style_import_result_installs_only_for_its_exact_processing_operation() {
        let parser = HtmlParser;
        let document_url = Url::parse("https://example.com/page").unwrap();
        let request_url = Url::parse("https://example.com/shared.css").unwrap();
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><style>@import url('/shared.css');</style></head><body></body></html>"
                .to_owned(),
        );
        let owner = first_element_handle(&document, "style");
        let source = Arc::new(crate::style_engine::OwnerStyleSheetSource::new(
            owner,
            "@import url('/shared.css');".to_owned(),
            document_url.clone(),
        ));
        let operation = ConnectedLoadOperation::new_for_test(
            owner,
            ConnectedStyleEventElementKind::Style,
            ConnectedLoadParameters::StyleImports {
                source: super::stylesheet_runtime::ConnectedStyleImportSource::Inline(source),
                urls: vec![request_url.clone()],
                roots: Vec::new(),
            },
            None,
        );
        let mut runtime = DocumentRuntime::new(&document);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&operation));
        runtime.apply_connected_style_load_completion(ConnectedLoadCompletion {
            operation,
            successful: true,
            network_results: vec![ConnectedLoadNetworkResult {
                stylesheet_fetch: None,
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document_url.clone(),
                request_url: request_url.clone(),
                source_owners: vec![owner],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("accepted source observation".to_owned()),
            }],
        });
        let result = runtime
            .take_ready_stylesheet_network_results()
            .into_iter()
            .next()
            .expect("accepted source result");

        assert_eq!(result.source_owners, vec![owner]);
    }

    #[test]
    fn accepted_style_import_result_loses_install_authority_after_same_url_aba() {
        let parser = HtmlParser;
        let document_url = Url::parse("https://example.com/page").unwrap();
        let request_url = Url::parse("https://example.com/shared.css").unwrap();
        let document = parser.parse(
            document_url.clone(),
            "<!doctype html><html><head><style>@import url('/shared.css');</style></head><body></body></html>"
                .to_owned(),
        );
        let owner = first_element_handle(&document, "style");
        let operation_for = |css_text: &str| {
            let source = Arc::new(crate::style_engine::OwnerStyleSheetSource::new(
                owner,
                css_text.to_owned(),
                document_url.clone(),
            ));
            ConnectedLoadOperation::new_for_test(
                owner,
                ConnectedStyleEventElementKind::Style,
                ConnectedLoadParameters::StyleImports {
                    source: super::stylesheet_runtime::ConnectedStyleImportSource::Inline(source),
                    urls: vec![request_url.clone()],
                    roots: Vec::new(),
                },
                None,
            )
        };
        let first_a = operation_for("@import url('/shared.css'); /* first A */");
        let mut runtime = DocumentRuntime::new(&document);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&first_a));
        runtime.apply_connected_style_load_completion(ConnectedLoadCompletion {
            operation: first_a,
            successful: true,
            network_results: vec![ConnectedLoadNetworkResult {
                stylesheet_fetch: None,
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document_url.clone(),
                request_url: request_url.clone(),
                source_owners: vec![owner],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("first A source observation".to_owned()),
            }],
        });

        runtime.invalidate_style_related_state(std::ptr::null_mut(), owner);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(operation_for(
                "@import url('/middle.css'); /* B */ @import url('/shared.css');",
            ));
        runtime.invalidate_style_related_state(std::ptr::null_mut(), owner);
        let current_a = operation_for("@import url('/shared.css'); /* current A */");
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&current_a));

        let stale = runtime
            .take_ready_stylesheet_network_results()
            .into_iter()
            .next()
            .expect("stale first A observation remains reportable");

        assert!(stale.source_owners.is_empty());
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .pending_operation(owner)
                .is_some_and(|pending| ConnectedLoadOperation::ptr_eq(pending, &current_a)),
            "the current A operation must retain its pending authority"
        );
    }

    #[tokio::test]
    async fn blocked_front_page_task_does_not_count_as_ready_document_processing_wake() {
        let (document, blocking_stylesheet_inputs) =
            parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href=/app.css></head><body><script defer src=/app.js></script></body></html>"
                .to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let script = document.script_handles()[0];

        let script_work = PageOwnedDocumentScriptWork::script(
            DocumentScriptExecutionLane::ClassicDefer,
            crate::planning::PreparedScript {
                position: 0,
                node_id: crate::dom::NodeId::new(script.index()),
                kind: crate::types::ScriptKind::Classic,
                mode: crate::types::ScriptMode::Defer,
                source_kind: crate::types::ScriptSourceKind::External,
                fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
                source: crate::planning::ScriptSource::External,
                initiator_url: Url::parse("https://example.com/app.js").unwrap(),
                base_url: Url::parse("https://example.com/app.js").unwrap(),
                url: Url::parse("https://example.com/app.js").unwrap(),
                host_script_handle: None,
            },
        );
        task_queue.extend_post_parse_work([
            PostParsePageOwnedWork::document_script_work_with_blocking_signatures(
                script_work,
                std::collections::HashSet::from([
                    crate::DocumentBlockingStylesheetSignature::Link {
                        url: Url::parse("https://example.com/app.css").unwrap(),
                        options: crate::stylesheet_blocking::StylesheetFetchOptions::default(),
                    },
                ]),
            ),
        ]);

        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        assert!(!runtime.has_ready_document_processing_wake(&mut task_queue));
    }

    #[tokio::test]
    async fn open_document_resets_document_owned_lifecycle_state() {
        let (document, blocking_stylesheet_inputs) = parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><link rel=stylesheet href=/app.css></body></html>"
                .to_owned(),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let handle = first_element_handle(&document, "link");
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        runtime.enqueue_pending_connected_style_load_for_test(handle);
        runtime
            .stylesheet_lifecycle
            .injected_ready_connected_loads
            .push_back(crate::document_runtime::ReadyConnectedStyleLoad::for_owner(
                handle,
                false,
                crate::document_runtime::ConnectedStyleEventElementKind::Link,
            ));
        let pending_operation = ConnectedLoadOperation::new_for_test(
            handle,
            ConnectedStyleEventElementKind::Link,
            preload_load_parameters(Url::parse("https://example.com/app.css").unwrap()),
            None,
        );
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&pending_operation));
        let node_id = crate::dom::NodeId::new(handle.index());
        let stylesheet_url = Url::parse("https://example.com/app.css").unwrap();
        runtime
            .stylesheet_lifecycle
            .fetches
            .enqueue_completion_for_testing(node_id, stylesheet_url.clone(), true);
        let _ = runtime
            .stylesheet_lifecycle
            .fetches
            .drain_ready_completions();
        runtime
            .stylesheet_lifecycle
            .ready_connected_load_network_results
            .push_back(ConnectedLoadNetworkResult {
                stylesheet_fetch: None,
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document.final_url().unwrap().clone(),
                request_url: stylesheet_url.clone(),
                source_owners: vec![handle],
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(1.0),
                origin_clean: true,
                result: Err("stale connected stylesheet result".to_owned()),
            });
        runtime.absorb_runtime_binding_calls(vec![PendingRuntimeBindingCall {
            source: crate::protocol_types::RuntimeBindingCallSourceIdentity::new(1, 1),
            name: "binding".to_owned(),
            payload: "{}".to_owned(),
            execution_context_id: 1,
        }]);

        runtime.open_document();

        assert!(
            runtime
                .stylesheet_lifecycle
                .pending_connected_loads
                .is_empty()
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .injected_ready_connected_loads
                .is_empty()
        );
        assert!(runtime.stylesheet_lifecycle.owner_states.is_empty());
        assert!(
            !runtime
                .stylesheet_lifecycle
                .fetches
                .has_any_pending_entries()
        );
        assert!(runtime.pop_ready_connected_style_load().is_none());
        assert!(
            runtime
                .stylesheet_lifecycle
                .ready_connected_load_network_results
                .is_empty()
        );
        let stale_results = runtime.take_ready_stylesheet_network_results();
        assert_eq!(
            stale_results.len(),
            1,
            "the retired typed terminal remains a resource observation"
        );
        assert_eq!(stale_results[0].request_url, stylesheet_url);
        assert!(
            stale_results[0].source_owners.is_empty(),
            "document.open() must revoke the retired terminal's stylesheet install authority"
        );
        assert_eq!(
            runtime.pending_runtime_binding_call_count(),
            1,
            "document.open() must retain a Runtime binding observation accepted before Document replacement"
        );
    }
}
