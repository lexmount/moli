use std::collections::HashMap;

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleTreeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleFetchId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleTreeClientToken {
    pub tree_id: ModuleTreeId,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SingleModuleClientToken {
    pub tree_id: ModuleTreeId,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleTreeConfig {
    pub tree_id: ModuleTreeId,
    pub client_sequence: u64,
    pub owner: ModuleTreeOwner,
    pub custom_fetch_type: ModuleScriptCustomFetchType,
}

impl Default for ModuleTreeConfig {
    fn default() -> Self {
        Self {
            tree_id: ModuleTreeId(0),
            client_sequence: 0,
            owner: ModuleTreeOwner::parser_pending_script(),
            custom_fetch_type: ModuleScriptCustomFetchType::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleTreeOwner {
    pub kind: ModuleTreeOwnerKind,
    pub requester: ModuleFetchRequester,
    pub ordering: ModuleFetchOrdering,
}

impl ModuleTreeOwner {
    pub fn parser_pending_script() -> Self {
        Self {
            kind: ModuleTreeOwnerKind::ParserPendingScript,
            requester: ModuleFetchRequester::ParserPendingScript,
            ordering: ModuleFetchOrdering::DclCritical,
        }
    }

    pub fn runtime_module_script() -> Self {
        Self {
            kind: ModuleTreeOwnerKind::RuntimeModuleScript,
            requester: ModuleFetchRequester::RuntimeModuleScript,
            ordering: ModuleFetchOrdering::Runtime,
        }
    }

    pub fn dynamic_import() -> Self {
        Self {
            kind: ModuleTreeOwnerKind::DynamicImport,
            requester: ModuleFetchRequester::DynamicImport,
            ordering: ModuleFetchOrdering::Runtime,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleTreeOwnerKind {
    ParserPendingScript,
    RuntimeModuleScript,
    DynamicImport,
    WorkerModuleScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleTreeState {
    Initial,
    FetchingRoot,
    FetchingDependencies,
    Linking,
    Finished,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleTreeTerminalState {
    Complete(ModuleGraphHandle),
    Failed(ModuleLoadError),
    Aborted(ModuleTreeAbortReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleRootInput {
    External(ModuleExternalRootInput),
    Inline(ModuleInlineRootInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleExternalRootInput {
    pub source_url: Url,
    pub base_url: Url,
    pub initiator_url: Url,
    pub attributes: ModuleAttributesKey,
    pub phase: ModuleImportPhase,
    pub kind_hint: Option<ModuleKind>,
    pub fetch_metadata: ModuleFetchMetadata,
    pub referrer: ModuleReferrer,
    pub position: TextPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInlineRootInput {
    pub root_key: ModuleMapKey,
    pub root_entry: ModuleEntryId,
    pub source_url: Url,
    pub base_url: Url,
    pub phase: ModuleImportPhase,
    pub fetch_metadata: ModuleFetchMetadata,
    pub referrer: ModuleReferrer,
    pub position: TextPosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleMapKey {
    pub url: Url,
    pub kind: ModuleKind,
    pub attributes: ModuleAttributesKey,
}

impl ModuleMapKey {
    pub fn new(url: Url, kind: ModuleKind, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            kind,
            attributes,
        }
    }

    pub fn javascript(url: Url) -> Self {
        Self::new(url, ModuleKind::JavaScript, ModuleAttributesKey::empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ModuleAttributesKey {
    pub attributes: Vec<(String, String)>,
}

impl ModuleAttributesKey {
    pub fn empty() -> Self {
        Self {
            attributes: Vec::new(),
        }
    }

    pub fn from_pairs(mut attributes: Vec<(String, String)>) -> Self {
        attributes.sort();
        attributes.dedup();
        Self { attributes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleEntryId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    JavaScript,
    Json,
    Css,
    WebAssembly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleImportPhase {
    Evaluation,
    Source,
}

impl ModuleImportPhase {
    pub fn strongest(self, other: Self) -> Self {
        if self == Self::Evaluation || other == Self::Evaluation {
            Self::Evaluation
        } else {
            Self::Source
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleGraphLevel {
    TopLevel,
    Dependent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFetchRequest {
    pub key: ModuleMapKey,
    pub tree_id: ModuleTreeId,
    pub client: SingleModuleClientToken,
    pub specifier: Option<String>,
    pub source_url: Url,
    pub base_url: Url,
    pub initiator_url: Url,
    pub referrer: ModuleReferrer,
    pub position: TextPosition,
    pub parent: Option<ParentModuleRef>,
    pub kind: ModuleKind,
    pub attributes: ModuleAttributesKey,
    pub phase: ModuleImportPhase,
    pub graph_level: ModuleGraphLevel,
    pub fetch_metadata: ModuleFetchMetadata,
    pub render_blocking: RenderBlockingBehavior,
    pub requester: ModuleFetchRequester,
    pub ordering: ModuleFetchOrdering,
    pub custom_fetch_type: ModuleScriptCustomFetchType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentModuleRef {
    pub key: ModuleMapKey,
    pub entry: ModuleEntryId,
    pub base_url: Url,
    pub effective_fetch_metadata: ModuleFetchMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFetchMetadata {
    pub credentials_mode: CredentialsMode,
    pub referrer_policy: ReferrerPolicy,
    pub integrity: Option<String>,
    pub nonce: Option<String>,
    pub charset: Option<String>,
    pub fetch_priority: FetchPriorityHint,
    pub scheduler_priority: Option<ScriptFetchSchedulerPriority>,
    pub request_context: ModuleRequestContext,
    pub destination: ModuleRequestDestination,
    /// HTML script fetch options parser metadata. CSP `strict-dynamic`
    /// distinguishes parser-inserted requests from script-created requests,
    /// and descendant module fetch options preserve this value.
    pub parser_inserted: bool,
}

impl ModuleFetchMetadata {
    pub fn descendant(&self) -> Self {
        let mut metadata = self.clone();
        metadata.integrity = None;
        metadata.charset = None;
        metadata.fetch_priority = FetchPriorityHint::Auto;
        metadata.scheduler_priority = None;
        metadata
    }
}

impl Default for ModuleFetchMetadata {
    fn default() -> Self {
        Self {
            credentials_mode: CredentialsMode::SameOrigin,
            referrer_policy: ReferrerPolicy::EmptyString,
            integrity: None,
            nonce: None,
            charset: None,
            fetch_priority: FetchPriorityHint::Auto,
            scheduler_priority: None,
            request_context: ModuleRequestContext::Script,
            destination: ModuleRequestDestination::Script,
            parser_inserted: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CredentialsMode {
    Omit,
    SameOrigin,
    Include,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferrerPolicy {
    EmptyString,
    NoReferrer,
    NoReferrerWhenDowngrade,
    Origin,
    OriginWhenCrossOrigin,
    SameOrigin,
    StrictOrigin,
    StrictOriginWhenCrossOrigin,
    UnsafeUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchPriorityHint {
    Auto,
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptFetchSchedulerPriority {
    VeryHigh,
    High,
    Normal,
    Low,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleRequestContext {
    Script,
    Worker,
    SharedWorker,
    ServiceWorker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleRequestDestination {
    Script,
    Worker,
    SharedWorker,
    ServiceWorker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderBlockingBehavior {
    Blocking,
    NonBlocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleFetchRequester {
    ParserPendingScript,
    RuntimeModuleScript,
    DynamicImport,
    ModulePreload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleFetchOrdering {
    DclCritical,
    Runtime,
    BackgroundPreload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleScriptCustomFetchType {
    None,
    WorkerConstructor,
    WorkletAddModule,
    InstalledServiceWorker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleReferrer {
    pub url: Option<Url>,
    pub referrer_string: Option<String>,
}

impl ModuleReferrer {
    pub fn client() -> Self {
        Self {
            url: None,
            referrer_string: Some("client".to_owned()),
        }
    }

    pub fn from_url(url: Url) -> Self {
        Self {
            referrer_string: Some(url.as_str().to_owned()),
            url: Some(url),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFetchResult {
    pub key: ModuleMapKey,
    pub client: SingleModuleClientToken,
    pub requested_phase: ModuleImportPhase,
    pub outcome: ModuleFetchOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleFetchOutcome {
    Fetched(Box<FetchedModuleSource>),
    Ready(Box<ReadyModule>),
    Failed(ModuleLoadError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyModule {
    pub entry: ModuleEntryId,
    pub key: ModuleMapKey,
    pub base_url: Url,
    pub effective_fetch_metadata: ModuleFetchMetadata,
}

impl ReadyModule {
    pub fn new(
        entry: ModuleEntryId,
        key: ModuleMapKey,
        base_url: Url,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> Self {
        Self {
            entry,
            key,
            base_url,
            effective_fetch_metadata,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleGraphHandle {
    pub root_entry: ModuleEntryId,
    pub entries: Vec<ModuleEntryId>,
    pub entry_phases: HashMap<ModuleEntryId, ModuleImportPhase>,
    pub dependency_edges: Vec<ModuleDependencyEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDependencyEdge {
    pub parent_key: ModuleMapKey,
    pub parent_entry: ModuleEntryId,
    pub child_key: ModuleMapKey,
    pub specifier: String,
    pub attributes: ModuleAttributesKey,
    pub phase: ModuleImportPhase,
    pub position: TextPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRequestRecord {
    pub specifier: String,
    pub attributes: ModuleAttributesKey,
    pub phase: ModuleImportPhase,
    pub position: TextPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModuleRequest {
    pub key: ModuleMapKey,
    pub source_url: Url,
    pub base_url: Url,
    pub kind: ModuleKind,
    pub attributes: ModuleAttributesKey,
    pub phase: ModuleImportPhase,
    pub integrity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledModuleSnapshot {
    pub entry: ModuleEntryId,
    pub key: ModuleMapKey,
    pub base_url: Url,
    pub effective_fetch_metadata: ModuleFetchMetadata,
    pub requested_modules: Vec<ModuleRequestRecord>,
    pub phase: ModuleImportPhase,
    pub has_parse_error: bool,
    pub parse_error: Option<ModuleLoadError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDependencySnapshot {
    pub entry: ModuleEntryId,
    pub key: ModuleMapKey,
    pub base_url: Url,
    pub effective_fetch_metadata: ModuleFetchMetadata,
    pub requested_modules: Vec<ModuleRequestRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedModuleSource {
    pub request_key: ModuleMapKey,
    pub key: ModuleMapKey,
    pub source_url: Url,
    pub base_url: Url,
    pub source: ModuleSource,
    pub effective_fetch_metadata: ModuleFetchMetadata,
}

impl FetchedModuleSource {
    pub fn new(
        request_key: ModuleMapKey,
        key: ModuleMapKey,
        source_url: Url,
        base_url: Url,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> Self {
        Self {
            request_key,
            key,
            source_url,
            base_url,
            source,
            effective_fetch_metadata,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleSource {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleLoadError {
    pub stage: ModuleLoadStage,
    pub key: Option<Box<ModuleMapKey>>,
    pub message: String,
    pub error_constructor: Option<ModuleErrorConstructorKind>,
}

impl ModuleLoadError {
    pub fn new(stage: ModuleLoadStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            key: None,
            message: message.into(),
            error_constructor: None,
        }
    }

    pub fn with_key(mut self, key: ModuleMapKey) -> Self {
        self.key = Some(Box::new(key));
        self
    }

    pub fn with_error_constructor(mut self, constructor: ModuleErrorConstructorKind) -> Self {
        self.error_constructor = Some(constructor);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleErrorConstructorKind {
    SyntaxError,
    TypeError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleLoadStage {
    Resolve,
    Fetch,
    Decode,
    TypeCheck,
    Compile,
    DependencyDiscovery,
    Link,
    Instantiate,
    Evaluate,
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleTreeAbortReason {
    ContextDestroyed,
    NavigationGenerationChanged,
    ExplicitCancel,
    OwnerDropped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleScriptTreePoll {
    Pending,
    NeedFetches(Vec<ModuleFetchRequest>),
    WaitingForSingleModuleClients(ModulePendingClientWait),
    Complete(ModuleGraphHandle),
    Failed(ModuleLoadError),
    Aborted(ModuleTreeAbortReason),
    IgnoredStaleCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePendingClientWait {
    pub client_count: usize,
}

impl ModulePendingClientWait {
    pub fn has_clients(&self) -> bool {
        self.client_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SingleModuleFetchDisposition {
    StartedNetworkFetch { fetch_id: ModuleFetchId },
    JoinedExistingFetch,
    Completed(ModuleFetchOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleScriptTreeDrive {
    Pending(ModuleScriptTreeIdle),
    NeedFetches(ModuleScriptTreeFetches),
    WaitingForSingleModuleClients(ModuleScriptTreeWait),
    Complete(ModuleGraphHandle),
    Failed(ModuleLoadError),
    Aborted(ModuleTreeAbortReason),
    IgnoredStaleCompletion(ModuleScriptTreeIdle),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleScriptTreeIdle {
    pub joined_fetches: Vec<ModuleFetchRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleScriptTreeFetches {
    pub fetches: Vec<ModuleFetchRequest>,
    pub joined_fetches: Vec<ModuleFetchRequest>,
}

impl ModuleScriptTreeFetches {
    pub fn len(&self) -> usize {
        self.fetches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fetches.is_empty()
    }

    pub fn into_parts(self) -> (Vec<ModuleFetchRequest>, Vec<ModuleFetchRequest>) {
        (self.fetches, self.joined_fetches)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleScriptTreeWait {
    pub client_count: usize,
    pub joined_fetches: Vec<ModuleFetchRequest>,
}

impl ModuleScriptTreeWait {
    pub fn has_clients(&self) -> bool {
        self.client_count > 0
    }
}
