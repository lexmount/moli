use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use moli_module_script_tree as module_tree;
use url::Url;

use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{
        FrameDocumentModuleFetchClientStart, FrameDocumentTaskOwner, FrameRealmId,
    },
    native_bridge::{
        OwnerDispatchScope, WindowExecutionContextIdentity, WindowExecutionContextOwner,
    },
    planning::ScriptFetchMetadata,
};

use super::{
    ModuleAttributesKey, ModuleEntryId, ModuleGraphFetchedSource, ModuleImportPhase, ModuleKind,
    ModuleLoadError, ModuleMapKey, NativeDynamicModuleImportReady, NativeModuleGraphFetchRequest,
    NativeModuleGraphJob, NativeModuleGraphJobAdvance, NativeModuleTreeDocumentOwnerAdapter,
};

#[cfg(test)]
use crate::native_bridge::{RuntimeObservableContextToken, WindowExecutionContextAccessPolicy};

pub(crate) struct PendingDynamicModuleImport {
    context: v8::Global<v8::Context>,
    resolver: v8::Global<v8::PromiseResolver>,
    owner: DynamicModuleImportOwner,
    specifier: String,
    base_url: Url,
    resolved_url: Option<Url>,
    attributes: ModuleAttributesKey,
    phase: ModuleImportPhase,
    fetch_metadata: ScriptFetchMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DynamicModuleImportDocumentSnapshot {
    Main(FrameDocumentTaskOwner),
    Child {
        child_handle: DomHandle,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    },
}

/// Captures both the document fetch snapshot and the ScriptState-like owner of
/// a dynamic import promise. `document.open()` replaces the former while
/// preserving the latter; navigation retires the execution context itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DynamicModuleImportOwner {
    document: DynamicModuleImportDocumentSnapshot,
    execution_context: WindowExecutionContextIdentity,
}

pub(crate) struct PendingDynamicModuleEvaluationReaction {
    request: PendingDynamicModuleImport,
    target: DynamicModuleEvaluationTarget,
}

pub(crate) struct DynamicModuleEvaluationTarget {
    root_entry: ModuleEntryId,
    module: v8::Global<v8::Module>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DynamicModuleExecutionContextRetirement {
    pending_import_count: usize,
    pending_tree_count: usize,
    inflight_fetch_count: usize,
    joined_fetch_count: usize,
    pending_reaction_count: usize,
}

#[derive(Default)]
pub(crate) struct DynamicModuleResolver {
    pending_imports: VecDeque<NativeModuleGraphJob>,
    inflight_fetches: HashMap<u64, DynamicModulePendingTreeId>,
    joined_fetches: HashMap<module_tree::SingleModuleClientToken, DynamicModulePendingTreeId>,
    pending_trees: HashMap<DynamicModulePendingTreeId, DynamicModulePendingTree>,
    pending_reactions: HashMap<u64, PendingDynamicModuleEvaluationReaction>,
    next_reaction_id: u64,
    next_pending_tree_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DynamicModulePendingTreeId(u64);

pub(crate) struct DynamicModuleInflightFetch {
    resume: DynamicModuleFetchResume,
    request: NativeModuleGraphFetchRequest,
    owner_module_fetch_start: Option<FrameDocumentModuleFetchClientStart>,
}

struct DynamicModulePendingTree {
    owner: DynamicModuleImportOwner,
    load_ids: Vec<u64>,
    joined_clients: Vec<module_tree::SingleModuleClientToken>,
    requests: HashMap<u64, NativeModuleGraphFetchRequest>,
    owner_module_fetch_starts: HashMap<u64, FrameDocumentModuleFetchClientStart>,
    job: Option<NativeModuleGraphJob>,
}

pub(crate) struct DynamicModuleJoinedFetch {
    resume: DynamicModuleFetchResume,
    client: module_tree::SingleModuleClientToken,
}

pub(in crate::module_runtime) struct DynamicModuleFetchResume {
    tree_id: DynamicModulePendingTreeId,
    active_joined_client: Option<module_tree::SingleModuleClientToken>,
    job: NativeModuleGraphJob,
}

pub(crate) struct DynamicModuleFetchContinuation {
    resume: DynamicModuleFetchResume,
    advance: NativeModuleGraphJobAdvance,
}

pub(crate) struct DynamicModuleFetchFailure {
    resume: DynamicModuleFetchResume,
    error: ModuleLoadError,
}

#[derive(Clone)]
pub(crate) struct DynamicModuleScheduledFetch {
    load_id: u64,
    request: NativeModuleGraphFetchRequest,
    owner_module_fetch_start: Option<FrameDocumentModuleFetchClientStart>,
}

pub(crate) enum DynamicModuleFetchFinish {
    Advanced(DynamicModuleFetchContinuation),
    Failed(DynamicModuleFetchFailure),
}

pub(crate) enum DynamicModuleFetchOwnerAdvance {
    Waiting {
        scheduled_fetches: Vec<DynamicModuleScheduledFetch>,
    },
    Ready(Box<NativeDynamicModuleImportReady>),
    RestoredAfterUnexpectedComplete,
}

impl DynamicModuleScheduledFetch {
    pub(crate) fn new(
        load_id: u64,
        request: NativeModuleGraphFetchRequest,
        owner_module_fetch_start: Option<FrameDocumentModuleFetchClientStart>,
    ) -> Self {
        Self {
            load_id,
            request,
            owner_module_fetch_start,
        }
    }

    pub(crate) fn load_id(&self) -> u64 {
        self.load_id
    }

    pub(crate) fn owner_module_fetch_start(&self) -> Option<&FrameDocumentModuleFetchClientStart> {
        self.owner_module_fetch_start.as_ref()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        u64,
        NativeModuleGraphFetchRequest,
        Option<FrameDocumentModuleFetchClientStart>,
    ) {
        (self.load_id, self.request, self.owner_module_fetch_start)
    }
}

impl PendingDynamicModuleImport {
    pub(crate) fn new(
        context: v8::Global<v8::Context>,
        resolver: v8::Global<v8::PromiseResolver>,
        owner: DynamicModuleImportOwner,
        specifier: impl Into<String>,
        base_url: Url,
        attributes: ModuleAttributesKey,
        phase: ModuleImportPhase,
    ) -> Self {
        Self {
            context,
            resolver,
            owner,
            specifier: specifier.into(),
            base_url,
            resolved_url: None,
            attributes,
            phase,
            fetch_metadata: ScriptFetchMetadata::default(),
        }
    }

    pub(crate) fn with_referrer_fetch_metadata(mut self, metadata: ScriptFetchMetadata) -> Self {
        self.fetch_metadata = metadata;
        self
    }

    pub(crate) fn with_resolved_url(mut self, resolved_url: Url) -> Self {
        self.resolved_url = Some(resolved_url);
        self
    }

    pub(crate) fn context(&self) -> &v8::Global<v8::Context> {
        &self.context
    }

    pub(crate) fn resolver(&self) -> &v8::Global<v8::PromiseResolver> {
        &self.resolver
    }

    pub(crate) fn owner(&self) -> DynamicModuleImportOwner {
        self.owner
    }

    pub(crate) fn specifier(&self) -> &str {
        &self.specifier
    }

    pub(crate) fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub(crate) fn resolved_url(&self) -> Option<&Url> {
        self.resolved_url.as_ref()
    }

    pub(crate) fn attributes(&self) -> &ModuleAttributesKey {
        &self.attributes
    }

    pub(crate) fn phase(&self) -> ModuleImportPhase {
        self.phase
    }

    pub(crate) fn fetch_metadata(&self) -> &ScriptFetchMetadata {
        &self.fetch_metadata
    }

    pub(crate) fn child_browsing_context_handle(&self) -> Option<DomHandle> {
        self.owner.child_handle()
    }
}

impl DynamicModuleImportOwner {
    pub(crate) fn main(
        task_owner: FrameDocumentTaskOwner,
        execution_context: WindowExecutionContextIdentity,
    ) -> Self {
        debug_assert_eq!(
            execution_context.owner(),
            WindowExecutionContextOwner::Frame(task_owner.local_window_id)
        );
        debug_assert_eq!(execution_context.dispatch_scope(), OwnerDispatchScope::Top);
        Self {
            document: DynamicModuleImportDocumentSnapshot::Main(task_owner),
            execution_context,
        }
    }

    pub(crate) fn child(
        child_handle: DomHandle,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        execution_context: WindowExecutionContextIdentity,
    ) -> Self {
        debug_assert_eq!(
            execution_context.owner(),
            WindowExecutionContextOwner::Frame(task_owner.local_window_id)
        );
        debug_assert_eq!(
            execution_context.dispatch_scope(),
            OwnerDispatchScope::Child(child_handle)
        );
        Self {
            document: DynamicModuleImportDocumentSnapshot::Child {
                child_handle,
                task_owner,
                realm_id,
            },
            execution_context,
        }
    }

    #[cfg(test)]
    pub(crate) fn main_for_test() -> Self {
        Self::main_for_test_parts(1, 2, 3)
    }

    #[cfg(test)]
    pub(crate) fn main_for_test_parts(
        scheduler_lane_id: u64,
        local_window_id: u64,
        document_id: u64,
    ) -> Self {
        use crate::frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId};

        let task_owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(scheduler_lane_id),
            LocalWindowId(local_window_id),
            DocumentId(document_id),
        );
        Self::main(
            task_owner,
            WindowExecutionContextIdentity::new(
                WindowExecutionContextOwner::Frame(task_owner.local_window_id),
                OwnerDispatchScope::Top,
                RuntimeObservableContextToken::from_raw(local_window_id),
                WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            ),
        )
    }

    pub(crate) fn child_handle(self) -> Option<DomHandle> {
        match self.document {
            DynamicModuleImportDocumentSnapshot::Main(_) => None,
            DynamicModuleImportDocumentSnapshot::Child { child_handle, .. } => Some(child_handle),
        }
    }

    pub(crate) fn child_parts(self) -> Option<(DomHandle, FrameDocumentTaskOwner, FrameRealmId)> {
        match self.document {
            DynamicModuleImportDocumentSnapshot::Main(_) => None,
            DynamicModuleImportDocumentSnapshot::Child {
                child_handle,
                task_owner,
                realm_id,
            } => Some((child_handle, task_owner, realm_id)),
        }
    }

    pub(crate) fn task_owner(self) -> FrameDocumentTaskOwner {
        match self.document {
            DynamicModuleImportDocumentSnapshot::Main(owner)
            | DynamicModuleImportDocumentSnapshot::Child {
                task_owner: owner, ..
            } => owner,
        }
    }

    pub(crate) fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }

    pub(crate) fn execution_context_owner(self) -> WindowExecutionContextOwner {
        self.execution_context.owner()
    }
}

impl DynamicModuleExecutionContextRetirement {
    pub(crate) fn retired_anything(self) -> bool {
        self != Self::default()
    }

    pub(crate) fn pending_import_count(self) -> usize {
        self.pending_import_count
    }

    pub(crate) fn pending_tree_count(self) -> usize {
        self.pending_tree_count
    }

    pub(crate) fn inflight_fetch_count(self) -> usize {
        self.inflight_fetch_count
    }

    pub(crate) fn joined_fetch_count(self) -> usize {
        self.joined_fetch_count
    }

    pub(crate) fn pending_reaction_count(self) -> usize {
        self.pending_reaction_count
    }
}

impl fmt::Debug for PendingDynamicModuleImport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingDynamicModuleImport")
            .field("owner", &self.owner)
            .field("specifier", &self.specifier)
            .field("base_url", &self.base_url)
            .field("attributes", &self.attributes)
            .field("phase", &self.phase)
            .field("fetch_metadata", &self.fetch_metadata)
            .finish_non_exhaustive()
    }
}

impl PendingDynamicModuleEvaluationReaction {
    pub(crate) fn new(
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
    ) -> Self {
        Self { request, target }
    }

    pub(crate) fn into_parts(self) -> (PendingDynamicModuleImport, DynamicModuleEvaluationTarget) {
        (self.request, self.target)
    }

    pub(crate) fn owner(&self) -> DynamicModuleImportOwner {
        self.request.owner()
    }
}

impl DynamicModuleEvaluationTarget {
    pub(crate) fn new(root_entry: ModuleEntryId, module: v8::Global<v8::Module>) -> Self {
        Self { root_entry, module }
    }

    pub(crate) fn root_entry(&self) -> ModuleEntryId {
        self.root_entry
    }

    pub(crate) fn module(&self) -> &v8::Global<v8::Module> {
        &self.module
    }
}

impl fmt::Debug for PendingDynamicModuleEvaluationReaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingDynamicModuleEvaluationReaction")
            .field("request", &self.request)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for DynamicModuleEvaluationTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicModuleEvaluationTarget")
            .field("root_entry", &self.root_entry)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for DynamicModuleResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicModuleResolver")
            .field("pending_import_count", &self.pending_imports.len())
            .field("inflight_fetch_count", &self.inflight_fetches.len())
            .field("joined_fetch_count", &self.joined_fetches.len())
            .field("pending_tree_count", &self.pending_trees.len())
            .field("pending_reaction_count", &self.pending_reactions.len())
            .finish()
    }
}

impl DynamicModuleInflightFetch {
    pub(crate) fn owner(&self) -> DynamicModuleImportOwner {
        self.resume
            .job
            .dynamic_import_request()
            .expect("dynamic module fetch resume must retain its import request")
            .owner()
    }

    pub(crate) fn import_base_url(&self) -> &Url {
        self.resume
            .job
            .dynamic_import_request()
            .expect("dynamic module fetch resume must retain its import request")
            .base_url()
    }

    pub(crate) fn owner_module_fetch_start(&self) -> Option<&FrameDocumentModuleFetchClientStart> {
        self.owner_module_fetch_start.as_ref()
    }

    pub(crate) fn finish_for_owner(
        mut self,
        vm: &mut crate::script_vm::ScriptVm,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> DynamicModuleFetchFinish {
        match self
            .resume
            .job
            .finish_dynamic_import_fetch_for_request(vm, &self.request, source)
        {
            Ok(advance) => DynamicModuleFetchFinish::Advanced(DynamicModuleFetchContinuation::new(
                self.resume,
                advance,
            )),
            Err(error) => {
                DynamicModuleFetchFinish::Failed(DynamicModuleFetchFailure::new(self.resume, error))
            }
        }
    }

    pub(crate) fn finish_with_owner_adapter<O>(
        mut self,
        owner: &mut O,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> DynamicModuleFetchFinish
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        match self
            .resume
            .job
            .finish_dynamic_import_fetch_for_request_with_owner(owner, &self.request, source)
        {
            Ok(advance) => DynamicModuleFetchFinish::Advanced(DynamicModuleFetchContinuation::new(
                self.resume,
                advance,
            )),
            Err(error) => {
                DynamicModuleFetchFinish::Failed(DynamicModuleFetchFailure::new(self.resume, error))
            }
        }
    }

    pub(crate) fn into_failure(self, error: ModuleLoadError) -> DynamicModuleFetchFinish {
        DynamicModuleFetchFinish::Failed(DynamicModuleFetchFailure::new(self.resume, error))
    }

    pub(crate) fn tree_client(&self) -> Option<module_tree::SingleModuleClientToken> {
        self.request.tree_client()
    }

    pub(crate) fn fetch_metadata(&self) -> &super::ModuleFetchMetadata {
        self.request.fetch_metadata()
    }

    #[cfg(test)]
    pub(crate) fn request_for_test(&self) -> &NativeModuleGraphFetchRequest {
        &self.request
    }

    #[cfg(test)]
    pub(crate) fn finish_with_advance_for_test(
        self,
        advance: NativeModuleGraphJobAdvance,
    ) -> DynamicModuleFetchContinuation {
        DynamicModuleFetchContinuation::new(self.resume, advance)
    }

    #[cfg(test)]
    fn into_raw(self) -> (DynamicModuleFetchResume, NativeModuleGraphFetchRequest) {
        (self.resume, self.request)
    }
}

impl DynamicModuleJoinedFetch {
    pub(crate) fn owner(&self) -> DynamicModuleImportOwner {
        self.resume
            .job
            .dynamic_import_request()
            .expect("joined dynamic module fetch must retain its import request")
            .owner()
    }

    pub(crate) fn import_base_url(&self) -> &Url {
        self.resume
            .job
            .dynamic_import_request()
            .expect("joined dynamic module fetch resume must retain its import request")
            .base_url()
    }

    pub(crate) fn client(&self) -> module_tree::SingleModuleClientToken {
        self.client
    }

    pub(crate) fn finish_for_owner(
        mut self,
        vm: &mut crate::script_vm::ScriptVm,
        key: &ModuleMapKey,
    ) -> DynamicModuleFetchFinish {
        match self.resume.job.finish_joined_module_map_fetch(
            vm,
            chromium_module_key(key),
            self.client,
        ) {
            Ok(advance) => DynamicModuleFetchFinish::Advanced(DynamicModuleFetchContinuation::new(
                self.resume,
                advance,
            )),
            Err(error) => {
                DynamicModuleFetchFinish::Failed(DynamicModuleFetchFailure::new(self.resume, error))
            }
        }
    }

    pub(crate) fn finish_with_owner_adapter<O>(
        mut self,
        owner: &mut O,
        key: &ModuleMapKey,
    ) -> DynamicModuleFetchFinish
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        match self.resume.job.finish_joined_module_map_fetch_with_owner(
            owner,
            chromium_module_key(key),
            self.client,
        ) {
            Ok(advance) => DynamicModuleFetchFinish::Advanced(DynamicModuleFetchContinuation::new(
                self.resume,
                advance,
            )),
            Err(error) => {
                DynamicModuleFetchFinish::Failed(DynamicModuleFetchFailure::new(self.resume, error))
            }
        }
    }

    pub(crate) fn into_failure(self, error: ModuleLoadError) -> DynamicModuleFetchFinish {
        DynamicModuleFetchFinish::Failed(DynamicModuleFetchFailure::new(self.resume, error))
    }

    #[cfg(test)]
    pub(crate) fn into_failure_for_test(self, error: ModuleLoadError) -> DynamicModuleFetchFailure {
        DynamicModuleFetchFailure::new(self.resume, error)
    }

    #[cfg(test)]
    fn into_raw(
        self,
    ) -> (
        DynamicModuleFetchResume,
        module_tree::SingleModuleClientToken,
    ) {
        (self.resume, self.client)
    }
}

impl DynamicModuleFetchContinuation {
    fn new(resume: DynamicModuleFetchResume, advance: NativeModuleGraphJobAdvance) -> Self {
        Self { resume, advance }
    }

    pub(in crate::module_runtime) fn into_parts(
        self,
    ) -> (DynamicModuleFetchResume, NativeModuleGraphJobAdvance) {
        (self.resume, self.advance)
    }

    pub(crate) fn job(&self) -> &NativeModuleGraphJob {
        &self.resume.job
    }

    pub(crate) fn pending_fetch_requests(&self) -> Option<&[NativeModuleGraphFetchRequest]> {
        match &self.advance {
            NativeModuleGraphJobAdvance::NeedFetches(requests) => Some(requests),
            NativeModuleGraphJobAdvance::WaitingForFetches
            | NativeModuleGraphJobAdvance::Complete(_) => None,
        }
    }
}

impl DynamicModuleFetchFailure {
    fn new(resume: DynamicModuleFetchResume, error: ModuleLoadError) -> Self {
        Self { resume, error }
    }

    #[cfg(test)]
    pub(crate) fn for_test(request: PendingDynamicModuleImport, error: ModuleLoadError) -> Self {
        Self::new(
            DynamicModuleFetchResume {
                tree_id: DynamicModulePendingTreeId(0),
                active_joined_client: None,
                job: NativeModuleGraphJob::dynamic_import(request),
            },
            error,
        )
    }

    pub(in crate::module_runtime) fn into_parts(
        self,
    ) -> (DynamicModuleFetchResume, ModuleLoadError) {
        (self.resume, self.error)
    }
}

impl DynamicModuleFetchResume {
    pub(in crate::module_runtime) fn take_pending_joined_clients(
        &mut self,
    ) -> Vec<module_tree::SingleModuleClientToken> {
        self.job.take_pending_joined_clients()
    }

    pub(in crate::module_runtime) fn into_job(self) -> NativeModuleGraphJob {
        self.job
    }
}

fn chromium_module_key(key: &ModuleMapKey) -> module_tree::ModuleMapKey {
    module_tree::ModuleMapKey::new(
        key.url().clone(),
        match key.kind() {
            ModuleKind::JavaScript => module_tree::ModuleKind::JavaScript,
            ModuleKind::Json => module_tree::ModuleKind::Json,
            ModuleKind::Css => module_tree::ModuleKind::Css,
            ModuleKind::Text => module_tree::ModuleKind::Text,
            ModuleKind::WebAssembly => module_tree::ModuleKind::WebAssembly,
        },
        module_tree::ModuleAttributesKey::from_pairs(key.attributes().pairs().to_vec()),
    )
}

impl DynamicModuleResolver {
    pub(crate) fn retire_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> DynamicModuleExecutionContextRetirement {
        let pending_import_count_before = self.pending_imports.len();
        self.pending_imports.retain(|job| {
            job.dynamic_import_request()
                .is_none_or(|request| request.owner().execution_context_owner() != owner)
        });

        let retired_tree_ids: HashSet<_> = self
            .pending_trees
            .iter()
            .filter_map(|(tree_id, tree)| {
                (tree.owner.execution_context_owner() == owner).then_some(*tree_id)
            })
            .collect();
        for tree_id in &retired_tree_ids {
            self.pending_trees.remove(tree_id);
        }

        let inflight_fetch_count_before = self.inflight_fetches.len();
        self.inflight_fetches
            .retain(|_, tree_id| !retired_tree_ids.contains(tree_id));
        let joined_fetch_count_before = self.joined_fetches.len();
        self.joined_fetches
            .retain(|_, tree_id| !retired_tree_ids.contains(tree_id));

        let pending_reaction_count_before = self.pending_reactions.len();
        self.pending_reactions
            .retain(|_, reaction| reaction.owner().execution_context_owner() != owner);

        DynamicModuleExecutionContextRetirement {
            pending_import_count: pending_import_count_before - self.pending_imports.len(),
            pending_tree_count: retired_tree_ids.len(),
            inflight_fetch_count: inflight_fetch_count_before - self.inflight_fetches.len(),
            joined_fetch_count: joined_fetch_count_before - self.joined_fetches.len(),
            pending_reaction_count: pending_reaction_count_before - self.pending_reactions.len(),
        }
    }

    pub(crate) fn queue_import(&mut self, request: PendingDynamicModuleImport) {
        self.pending_imports
            .push_back(NativeModuleGraphJob::dynamic_import(request));
    }

    pub(crate) fn take_next_import(&mut self) -> Option<NativeModuleGraphJob> {
        self.pending_imports.pop_front()
    }

    pub(crate) fn resume_import_front(&mut self, job: NativeModuleGraphJob) {
        self.pending_imports.push_front(job);
    }

    pub(crate) fn reserve_evaluation_reaction(
        &mut self,
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
    ) -> u64 {
        let reaction_id = self.next_reaction_id;
        self.next_reaction_id = self.next_reaction_id.wrapping_add(1);
        self.pending_reactions.insert(
            reaction_id,
            PendingDynamicModuleEvaluationReaction::new(request, target),
        );
        reaction_id
    }

    pub(crate) fn evaluation_reaction_owner(
        &self,
        reaction_id: u64,
    ) -> Option<DynamicModuleImportOwner> {
        self.pending_reactions
            .get(&reaction_id)
            .map(PendingDynamicModuleEvaluationReaction::owner)
    }

    pub(crate) fn take_evaluation_reaction(
        &mut self,
        reaction_id: u64,
        expected_owner: DynamicModuleImportOwner,
    ) -> Option<PendingDynamicModuleEvaluationReaction> {
        if self.evaluation_reaction_owner(reaction_id) != Some(expected_owner) {
            return None;
        }
        self.pending_reactions.remove(&reaction_id)
    }

    pub(crate) fn suspend_fetches(
        &mut self,
        fetches: Vec<(u64, NativeModuleGraphFetchRequest)>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        job: NativeModuleGraphJob,
        owner_module_fetch_starts: Vec<(u64, FrameDocumentModuleFetchClientStart)>,
    ) {
        if fetches.is_empty() && joined_clients.is_empty() {
            self.resume_import_front(job);
            return;
        }
        let owner = job
            .dynamic_import_request()
            .expect("dynamic module pending tree must retain its import request")
            .owner();
        let tree_id = DynamicModulePendingTreeId(self.next_pending_tree_id);
        self.next_pending_tree_id = self.next_pending_tree_id.wrapping_add(1);
        let mut load_ids = Vec::with_capacity(fetches.len());
        let mut requests = HashMap::with_capacity(fetches.len());
        for (load_id, request) in fetches {
            self.inflight_fetches.insert(load_id, tree_id);
            load_ids.push(load_id);
            requests.insert(load_id, request);
        }
        for client in &joined_clients {
            self.joined_fetches.insert(*client, tree_id);
        }
        let owner_module_fetch_starts = owner_module_fetch_starts.into_iter().collect();
        self.pending_trees.insert(
            tree_id,
            DynamicModulePendingTree {
                owner,
                load_ids,
                joined_clients,
                requests,
                owner_module_fetch_starts,
                job: Some(job),
            },
        );
    }

    pub(crate) fn take_inflight_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<DynamicModuleInflightFetch> {
        let tree_id = self.inflight_fetches.remove(&load_id)?;
        let (request, owner_module_fetch_start, job) = {
            let pending_tree = self.pending_trees.get_mut(&tree_id)?;
            pending_tree
                .load_ids
                .retain(|pending_load_id| *pending_load_id != load_id);
            let request = pending_tree.requests.remove(&load_id)?;
            let owner_module_fetch_start = pending_tree.owner_module_fetch_starts.remove(&load_id);
            let job = pending_tree.job.take()?;
            (request, owner_module_fetch_start, job)
        };
        if self.pending_tree_waits_are_empty(tree_id) {
            self.pending_trees.remove(&tree_id);
        }
        Some(DynamicModuleInflightFetch {
            resume: DynamicModuleFetchResume {
                tree_id,
                active_joined_client: None,
                job,
            },
            request,
            owner_module_fetch_start,
        })
    }

    /// Returns the exact import owner captured by the pending tree that owns
    /// `load_id` without claiming or advancing the fetch.
    ///
    /// Network scheduling uses this projection immediately after suspension to
    /// stamp the stable Page task. Keeping the lookup in the resolver avoids a
    /// second `load_id -> owner` registry in the transport layer.
    pub(crate) fn inflight_fetch_owner(&self, load_id: u64) -> Option<DynamicModuleImportOwner> {
        let tree_id = self.inflight_fetches.get(&load_id)?;
        self.pending_trees.get(tree_id).map(|tree| tree.owner)
    }

    pub(crate) fn take_joined_fetch(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> Option<DynamicModuleJoinedFetch> {
        let tree_id = self.joined_fetches.remove(&client)?;
        let job = {
            let pending_tree = self.pending_trees.get_mut(&tree_id)?;
            pending_tree
                .joined_clients
                .retain(|pending_client| *pending_client != client);
            pending_tree.job.take()?
        };
        if self.pending_tree_waits_are_empty(tree_id) {
            self.pending_trees.remove(&tree_id);
        }
        Some(DynamicModuleJoinedFetch {
            resume: DynamicModuleFetchResume {
                tree_id,
                active_joined_client: Some(client),
                job,
            },
            client,
        })
    }

    pub(crate) fn restore_inflight_fetch_as_joined_owner_module_client(
        &mut self,
        inflight: DynamicModuleInflightFetch,
    ) -> Option<module_tree::SingleModuleClientToken> {
        let DynamicModuleInflightFetch {
            resume, request, ..
        } = inflight;
        let client = request.tree_client()?;
        let tree_id = resume.tree_id;
        let job = resume.job;
        self.joined_fetches.insert(client, tree_id);
        if let Some(pending_tree) = self.pending_trees.get_mut(&tree_id) {
            pending_tree.joined_clients.push(client);
            debug_assert!(pending_tree.job.is_none());
            pending_tree.job = Some(job);
        } else {
            let owner = job
                .dynamic_import_request()
                .expect("restored dynamic module tree must retain its import request")
                .owner();
            self.pending_trees.insert(
                tree_id,
                DynamicModulePendingTree {
                    owner,
                    load_ids: Vec::new(),
                    joined_clients: vec![client],
                    requests: HashMap::new(),
                    owner_module_fetch_starts: HashMap::new(),
                    job: Some(job),
                },
            );
        }
        Some(client)
    }

    pub(in crate::module_runtime) fn restore_fetch_resume(
        &mut self,
        resume: DynamicModuleFetchResume,
    ) {
        let tree_id = resume.tree_id;
        let job = resume.job;
        let Some(pending_tree) = self.pending_trees.get_mut(&tree_id) else {
            self.resume_import_front(job);
            return;
        };
        debug_assert!(pending_tree.job.is_none());
        pending_tree.job = Some(job);
    }

    pub(in crate::module_runtime) fn extend_pending_tree(
        &mut self,
        resume: DynamicModuleFetchResume,
        fetches: Vec<(u64, NativeModuleGraphFetchRequest)>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        owner_module_fetch_starts: Vec<(u64, FrameDocumentModuleFetchClientStart)>,
    ) {
        let tree_id = resume.tree_id;
        let job = resume.job;
        let Some(pending_tree) = self.pending_trees.get_mut(&tree_id) else {
            self.suspend_fetches(fetches, joined_clients, job, owner_module_fetch_starts);
            return;
        };
        for (load_id, request) in fetches {
            self.inflight_fetches.insert(load_id, tree_id);
            pending_tree.load_ids.push(load_id);
            pending_tree.requests.insert(load_id, request);
        }
        for (load_id, owner_start) in owner_module_fetch_starts {
            pending_tree
                .owner_module_fetch_starts
                .insert(load_id, owner_start);
        }
        for client in joined_clients {
            self.joined_fetches.insert(client, tree_id);
            pending_tree.joined_clients.push(client);
        }
        debug_assert!(pending_tree.job.is_none());
        pending_tree.job = Some(job);
    }

    pub(in crate::module_runtime) fn clear_fetch_resume(
        &mut self,
        resume: DynamicModuleFetchResume,
    ) -> (
        Vec<module_tree::SingleModuleClientToken>,
        NativeModuleGraphJob,
    ) {
        let DynamicModuleFetchResume {
            tree_id,
            active_joined_client,
            job,
            ..
        } = resume;
        let Some(mut pending_tree) = self.pending_trees.remove(&tree_id) else {
            return (active_joined_client.into_iter().collect(), job);
        };
        for load_id in pending_tree.load_ids {
            self.inflight_fetches.remove(&load_id);
            pending_tree.owner_module_fetch_starts.remove(&load_id);
        }
        for client in &pending_tree.joined_clients {
            self.joined_fetches.remove(client);
        }
        let mut joined_clients = pending_tree.joined_clients;
        if let Some(active_joined_client) = active_joined_client {
            joined_clients.push(active_joined_client);
        }
        (joined_clients, job)
    }

    pub(in crate::module_runtime) fn fetch_resume_has_pending_waits(
        &self,
        resume: &DynamicModuleFetchResume,
    ) -> bool {
        self.pending_trees
            .get(&resume.tree_id)
            .map(|pending_tree| {
                !pending_tree.load_ids.is_empty() || !pending_tree.joined_clients.is_empty()
            })
            .unwrap_or(false)
    }

    fn pending_tree_waits_are_empty(&self, tree_id: DynamicModulePendingTreeId) -> bool {
        self.pending_trees
            .get(&tree_id)
            .map(|pending_tree| {
                pending_tree.load_ids.is_empty() && pending_tree.joined_clients.is_empty()
            })
            .unwrap_or(true)
    }

    pub(crate) fn has_pending_import(&self) -> bool {
        !self.pending_imports.is_empty()
            || !self.inflight_fetches.is_empty()
            || !self.joined_fetches.is_empty()
    }

    pub(crate) fn has_ready_import(&self) -> bool {
        !self.pending_imports.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_fetch(&self) -> bool {
        !self.inflight_fetches.is_empty() || !self.joined_fetches.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId};
    use crate::module_runtime::{ModuleFetchMetadata, ModuleKind};

    fn dynamic_import_request() -> PendingDynamicModuleImport {
        dynamic_import_request_for_owner(DynamicModuleImportOwner::main_for_test())
    }

    fn dynamic_import_request_for_owner(
        owner: DynamicModuleImportOwner,
    ) -> PendingDynamicModuleImport {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            owner,
            "./dep.mjs",
            Url::parse("https://app.example.test/page").expect("base URL should parse"),
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        )
    }

    fn test_main_owner(local_window_id: u64, document_id: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(1),
            LocalWindowId(local_window_id),
            DocumentId(document_id),
        )
    }

    fn test_dynamic_import_owner(
        local_window_id: u64,
        document_id: u64,
    ) -> DynamicModuleImportOwner {
        let task_owner = test_main_owner(local_window_id, document_id);
        DynamicModuleImportOwner::main(
            task_owner,
            WindowExecutionContextIdentity::new(
                WindowExecutionContextOwner::Frame(task_owner.local_window_id),
                OwnerDispatchScope::Top,
                RuntimeObservableContextToken::from_raw(local_window_id),
                WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            ),
        )
    }

    fn dynamic_fetch_request(path: &str) -> NativeModuleGraphFetchRequest {
        let url = Url::parse(&format!("https://app.example.test/{path}"))
            .expect("fetch URL should parse");
        NativeModuleGraphFetchRequest::new_for_test(
            url.clone(),
            Url::parse("https://app.example.test/page").expect("initiator URL should parse"),
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
        )
    }

    fn joined_fetch_request(sequence: u64) -> module_tree::ModuleFetchRequest {
        let url = Url::parse(&format!("https://app.example.test/joined-{sequence}.mjs"))
            .expect("joined fetch URL should parse");
        module_tree::ModuleFetchRequest {
            key: module_tree::ModuleMapKey::javascript(url.clone()),
            tree_id: module_tree::ModuleTreeId(7),
            client: module_tree::SingleModuleClientToken {
                tree_id: module_tree::ModuleTreeId(7),
                sequence,
            },
            specifier: None,
            source_url: url.clone(),
            base_url: url,
            initiator_url: Url::parse("https://app.example.test/page")
                .expect("initiator URL should parse"),
            referrer: module_tree::ModuleReferrer::client(),
            position: module_tree::TextPosition::default(),
            parent: None,
            kind: module_tree::ModuleKind::JavaScript,
            attributes: module_tree::ModuleAttributesKey::empty(),
            phase: module_tree::ModuleImportPhase::Evaluation,
            graph_level: module_tree::ModuleGraphLevel::Dependent,
            fetch_metadata: module_tree::ModuleFetchMetadata::default(),
            render_blocking: module_tree::RenderBlockingBehavior::NonBlocking,
            requester: module_tree::ModuleFetchRequester::DynamicImport,
            ordering: module_tree::ModuleFetchOrdering::Runtime,
            custom_fetch_type: module_tree::ModuleScriptCustomFetchType::None,
        }
    }

    #[test]
    fn execution_context_retirement_removes_queued_and_suspended_graph_state() {
        let retired_owner = test_dynamic_import_owner(2, 3);
        let retained_same_window_owner = test_dynamic_import_owner(2, 4);
        let retained_owner = test_dynamic_import_owner(5, 6);
        let mut resolver = DynamicModuleResolver::default();

        resolver.queue_import(dynamic_import_request_for_owner(retired_owner));
        let suspended_job = resolver
            .take_next_import()
            .expect("retired owner should provide the suspended graph job");
        let joined_client = joined_fetch_request(90).client;
        resolver.suspend_fetches(
            vec![(11, dynamic_fetch_request("retired.mjs"))],
            vec![joined_client],
            suspended_job,
            Vec::new(),
        );
        resolver.queue_import(dynamic_import_request_for_owner(retired_owner));
        resolver.queue_import(dynamic_import_request_for_owner(retained_same_window_owner));
        resolver.queue_import(dynamic_import_request_for_owner(retained_owner));

        let retirement =
            resolver.retire_execution_context_owner(retired_owner.execution_context_owner());

        assert_eq!(retirement.pending_import_count(), 2);
        assert_eq!(retirement.pending_tree_count(), 1);
        assert_eq!(retirement.inflight_fetch_count(), 1);
        assert_eq!(retirement.joined_fetch_count(), 1);
        assert_eq!(retirement.pending_reaction_count(), 0);
        assert!(retirement.retired_anything());
        assert!(resolver.take_inflight_fetch(11).is_none());
        assert!(resolver.take_joined_fetch(joined_client).is_none());
        let retained_job = resolver
            .take_next_import()
            .expect("replacement owner graph job must remain queued");
        assert_eq!(
            retained_job
                .dynamic_import_request()
                .expect("retained graph job must keep its request")
                .owner(),
            retained_owner
        );
        assert!(resolver.take_next_import().is_none());
    }

    #[test]
    fn inflight_dynamic_import_fetch_is_pending_but_not_ready() {
        let mut resolver = DynamicModuleResolver::default();

        resolver.queue_import(dynamic_import_request());
        assert!(resolver.has_pending_import());
        assert!(resolver.has_ready_import());
        assert!(!resolver.has_inflight_fetch());

        let job = resolver
            .take_next_import()
            .expect("queued dynamic import should be ready");
        resolver.suspend_fetches(
            vec![(11, dynamic_fetch_request("dep.mjs"))],
            Vec::new(),
            job,
            Vec::new(),
        );

        assert!(resolver.has_pending_import());
        assert!(!resolver.has_ready_import());
        assert!(resolver.has_inflight_fetch());

        let inflight = resolver
            .take_inflight_fetch(11)
            .expect("inflight dynamic import should be resumable");
        let (resume, _request) = inflight.into_raw();
        resolver.resume_import_front(resume.into_job());

        assert!(resolver.has_pending_import());
        assert!(resolver.has_ready_import());
        assert!(!resolver.has_inflight_fetch());
    }

    #[test]
    fn dynamic_import_pending_tree_restores_job_until_last_load_finishes() {
        let mut resolver = DynamicModuleResolver::default();
        resolver.queue_import(dynamic_import_request());
        let job = resolver
            .take_next_import()
            .expect("queued dynamic import should be ready");

        resolver.suspend_fetches(
            vec![
                (11, dynamic_fetch_request("a.mjs")),
                (12, dynamic_fetch_request("b.mjs")),
            ],
            Vec::new(),
            job,
            Vec::new(),
        );

        let first = resolver
            .take_inflight_fetch(11)
            .expect("first fetch should take the shared job");
        let (first_resume, _request) = first.into_raw();
        assert!(resolver.fetch_resume_has_pending_waits(&first_resume));
        assert!(resolver.has_inflight_fetch());
        resolver.restore_fetch_resume(first_resume);
        assert!(!resolver.has_ready_import());

        let second = resolver
            .take_inflight_fetch(12)
            .expect("second fetch should recover the restored job");
        let (second_resume, _request) = second.into_raw();
        assert!(!resolver.fetch_resume_has_pending_waits(&second_resume));
        resolver.resume_import_front(second_resume.into_job());

        assert!(resolver.has_ready_import());
        assert!(!resolver.has_inflight_fetch());
    }

    #[test]
    fn joined_dynamic_import_fetch_uses_single_module_client_token() {
        let mut resolver = DynamicModuleResolver::default();
        resolver.queue_import(dynamic_import_request());
        let job = resolver
            .take_next_import()
            .expect("queued dynamic import should be ready");
        let queued_request = joined_fetch_request(90);
        let client = queued_request.client;
        resolver.suspend_fetches(Vec::new(), vec![client], job, Vec::new());

        let joined = resolver
            .take_joined_fetch(client)
            .expect("joined fetch should recover its dynamic import job");
        let (joined_resume, joined_client) = joined.into_raw();

        assert_eq!(joined_client, client);
        assert!(!resolver.fetch_resume_has_pending_waits(&joined_resume));
        assert!(!resolver.has_inflight_fetch());
    }

    #[test]
    fn clear_pending_tree_returns_joined_clients_and_drops_waits() {
        let mut resolver = DynamicModuleResolver::default();
        resolver.queue_import(dynamic_import_request());
        let job = resolver
            .take_next_import()
            .expect("queued dynamic import should be ready");
        let first_join = joined_fetch_request(91);
        let first_client = first_join.client;
        let second_join = joined_fetch_request(92);
        let second_client = second_join.client;
        resolver.suspend_fetches(
            vec![(17, dynamic_fetch_request("clear-group.mjs"))],
            vec![first_client, second_client],
            job,
            Vec::new(),
        );

        let inflight = resolver
            .take_inflight_fetch(17)
            .expect("inflight fetch should recover its dynamic import resume");
        let (resume, _request) = inflight.into_raw();
        let (removed, _job) = resolver.clear_fetch_resume(resume);

        assert_eq!(removed, vec![first_client, second_client]);
        assert!(!resolver.has_inflight_fetch());
        assert!(
            resolver.take_inflight_fetch(17).is_none(),
            "clearing a group should remove its pending load ids"
        );
        assert!(
            resolver
                .take_joined_fetch(joined_fetch_request(91).client)
                .is_none(),
            "clearing a group should remove its joined fetch tokens"
        );
    }
}
