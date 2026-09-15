use std::collections::{BTreeMap, HashMap, HashSet};

use moli_module_script_tree as module_tree;

use crate::{
    document_runtime::DomHandle,
    document_script_scheduler::{
        DocumentScriptReadyActionDispatchRoute, DocumentScriptReadyActionRoute,
    },
    document_task_lane::DocumentRealmTask,
    dom::NodeId,
    frame_owner_model::{
        ChildDocumentModuleScriptLoadDelay, FrameDocumentOwner, FrameDocumentTaskOwner,
        FrameRealmId,
    },
    module_runtime::{ModuleEntryId, ModuleGraphHandle, ModuleLoadError, ModuleMapKey},
    parser_module_pending::ParserPendingModuleScriptState,
    planning::PreparedScript,
};

#[derive(Debug)]
pub(crate) enum ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure> {
    Ready(Box<ModuleScriptGraphReadyWork<Target>>),
    Failed(ParserModuleGraphFailure),
}

#[derive(Debug)]
pub(super) enum ParserModulePendingScriptWatchResult<Target, ParserModuleGraphFailure> {
    Missing,
    WaitingForTree,
    Ready(Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserOrderedModuleTerminalState {
    Missing,
    Ready,
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ParserPendingScriptKey {
    parser_position: usize,
    script_node_index: usize,
}

impl ParserPendingScriptKey {
    pub(crate) fn from_script(script: &PreparedScript) -> Self {
        Self {
            parser_position: script.position,
            script_node_index: script.node_id.index(),
        }
    }

    pub(crate) fn parser_position(self) -> usize {
        self.parser_position
    }

    pub(crate) fn script_node_id(self) -> NodeId {
        NodeId::new(self.script_node_index)
    }

    #[cfg(test)]
    pub(crate) fn from_parts_for_test(parser_position: usize, script_node_id: NodeId) -> Self {
        Self {
            parser_position,
            script_node_index: script_node_id.index(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ParserPendingScriptId<Owner> {
    owner: Owner,
    key: ParserPendingScriptKey,
}

impl<Owner: Copy> ParserPendingScriptId<Owner> {
    #[cfg(test)]
    pub(crate) fn new(owner: Owner, script: &PreparedScript) -> Self {
        Self::from_key(owner, ParserPendingScriptKey::from_script(script))
    }

    pub(crate) fn from_key(owner: Owner, key: ParserPendingScriptKey) -> Self {
        Self { owner, key }
    }

    pub(crate) fn owner(self) -> Owner {
        self.owner
    }

    pub(crate) fn key(self) -> ParserPendingScriptKey {
        self.key
    }

    pub(crate) fn parser_position(self) -> usize {
        self.key.parser_position()
    }

    pub(crate) fn script_node_id(self) -> NodeId {
        self.key.script_node_id()
    }
}

pub(crate) trait ParserPendingScriptRoute<Owner> {
    fn parser_pending_script_id(&self) -> ParserPendingScriptId<Owner>;
}

#[derive(Debug)]
pub(crate) enum DocumentModuleScriptReadyWork<GraphReady, GraphFailure, Evaluation> {
    GraphReady(GraphReady),
    GraphFailed(GraphFailure),
    EvaluationCompleted(Evaluation),
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentModuleGraphReadyPayload {
    pending_script_key: ParserPendingScriptKey,
    script_handle: DomHandle,
    request_key: ModuleMapKey,
    tree_id: module_tree::ModuleTreeId,
    load_delay_token: ChildDocumentModuleScriptLoadDelay,
}

pub(crate) type FrameDocumentModuleGraphReadyTarget =
    DocumentRealmTask<FrameDocumentTaskOwner, FrameRealmId, FrameDocumentModuleGraphReadyPayload>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentReadyActionRoute {
    document_owner: FrameDocumentOwner,
    child_handle: Option<DomHandle>,
    task_owner: FrameDocumentTaskOwner,
    realm_id: Option<FrameRealmId>,
    requires_realm: bool,
    script_handle: DomHandle,
}

impl FrameDocumentReadyActionRoute {
    fn new(
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script_handle: DomHandle,
    ) -> Self {
        Self {
            document_owner: task_owner.document_owner(),
            child_handle: None,
            task_owner,
            realm_id: Some(realm_id),
            requires_realm: true,
            script_handle,
        }
    }

    pub(crate) fn from_frame_document_parts(
        child_handle: Option<DomHandle>,
        task_owner: FrameDocumentTaskOwner,
        realm_id: Option<FrameRealmId>,
        requires_realm: bool,
        script_handle: DomHandle,
    ) -> Self {
        Self {
            document_owner: task_owner.document_owner(),
            child_handle,
            task_owner,
            realm_id,
            requires_realm,
            script_handle,
        }
    }

    pub(crate) fn document_owner(&self) -> FrameDocumentOwner {
        self.document_owner
    }

    pub(crate) fn child_handle(&self) -> Option<DomHandle> {
        self.child_handle
    }

    pub(crate) fn task_owner(&self) -> FrameDocumentTaskOwner {
        self.task_owner
    }

    pub(crate) fn optional_realm_id(&self) -> Option<FrameRealmId> {
        self.realm_id
    }

    pub(crate) fn requires_realm(&self) -> bool {
        self.requires_realm
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.script_handle
    }
}

impl FrameDocumentModuleGraphReadyTarget {
    pub(crate) fn from_graph_ready_parts(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: ParserPendingScriptId<FrameDocumentOwner>,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        tree_id: module_tree::ModuleTreeId,
        load_delay_token: impl Into<ChildDocumentModuleScriptLoadDelay>,
    ) -> Self {
        assert_eq!(pending_script_id.owner(), owner.document_owner());
        Self::new(
            owner,
            realm_id,
            FrameDocumentModuleGraphReadyPayload {
                pending_script_key: pending_script_id.key(),
                script_handle,
                request_key,
                tree_id,
                load_delay_token: load_delay_token.into(),
            },
        )
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.payload().script_handle
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        ParserPendingScriptId::from_key(
            self.owner().document_owner(),
            self.payload().pending_script_key,
        )
    }

    pub(crate) fn request_key(&self) -> &ModuleMapKey {
        &self.payload().request_key
    }

    pub(crate) fn tree_id(&self) -> module_tree::ModuleTreeId {
        self.payload().tree_id
    }

    pub(crate) fn load_delay_token(&self) -> ChildDocumentModuleScriptLoadDelay {
        self.payload().load_delay_token
    }
}

pub(crate) type DocumentModuleGraphReadyWork =
    ModuleScriptGraphReadyWork<FrameDocumentModuleGraphReadyTarget>;

impl DocumentScriptReadyActionRoute<FrameDocumentOwner> for DocumentModuleGraphReadyWork {
    fn payload_document_owner(&self) -> FrameDocumentOwner {
        self.owner().document_owner()
    }
}

impl ParserPendingScriptRoute<FrameDocumentOwner> for DocumentModuleGraphReadyWork {
    fn parser_pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        self.pending_script_id()
    }
}

impl DocumentScriptReadyActionDispatchRoute<FrameDocumentReadyActionRoute>
    for DocumentModuleGraphReadyWork
{
    fn dispatch_route(&self) -> FrameDocumentReadyActionRoute {
        FrameDocumentReadyActionRoute::new(self.owner(), self.realm_id(), self.script_handle())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentModuleGraphFailedPayload {
    pending_script_key: ParserPendingScriptKey,
    script_handle: DomHandle,
    request_key: ModuleMapKey,
    tree_id: Option<module_tree::ModuleTreeId>,
    load_delay_token: ChildDocumentModuleScriptLoadDelay,
}

pub(crate) type FrameDocumentModuleGraphFailedTarget =
    DocumentRealmTask<FrameDocumentTaskOwner, FrameRealmId, FrameDocumentModuleGraphFailedPayload>;

impl FrameDocumentModuleGraphFailedTarget {
    pub(crate) fn from_graph_failed_parts(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: ParserPendingScriptId<FrameDocumentOwner>,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        tree_id: Option<module_tree::ModuleTreeId>,
        load_delay_token: impl Into<ChildDocumentModuleScriptLoadDelay>,
    ) -> Self {
        assert_eq!(pending_script_id.owner(), owner.document_owner());
        Self::new(
            owner,
            realm_id,
            FrameDocumentModuleGraphFailedPayload {
                pending_script_key: pending_script_id.key(),
                script_handle,
                request_key,
                tree_id,
                load_delay_token: load_delay_token.into(),
            },
        )
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.payload().script_handle
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        ParserPendingScriptId::from_key(
            self.owner().document_owner(),
            self.payload().pending_script_key,
        )
    }

    pub(crate) fn request_key(&self) -> &ModuleMapKey {
        &self.payload().request_key
    }

    pub(crate) fn tree_id(&self) -> Option<module_tree::ModuleTreeId> {
        self.payload().tree_id
    }

    pub(crate) fn load_delay_token(&self) -> ChildDocumentModuleScriptLoadDelay {
        self.payload().load_delay_token
    }
}

pub(crate) type DocumentModuleGraphFailedWork =
    ModuleScriptGraphFailedWork<FrameDocumentModuleGraphFailedTarget>;

impl DocumentScriptReadyActionRoute<FrameDocumentOwner> for DocumentModuleGraphFailedWork {
    fn payload_document_owner(&self) -> FrameDocumentOwner {
        self.owner().document_owner()
    }
}

impl ParserPendingScriptRoute<FrameDocumentOwner> for DocumentModuleGraphFailedWork {
    fn parser_pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        self.pending_script_id()
    }
}

impl DocumentScriptReadyActionDispatchRoute<FrameDocumentReadyActionRoute>
    for DocumentModuleGraphFailedWork
{
    fn dispatch_route(&self) -> FrameDocumentReadyActionRoute {
        FrameDocumentReadyActionRoute::new(self.owner(), self.realm_id(), self.script_handle())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleScriptGraphFailedWork<Target = FrameDocumentModuleGraphFailedTarget> {
    target: Target,
    script: PreparedScript,
    error: ModuleLoadError,
}

impl ModuleScriptGraphFailedWork<FrameDocumentModuleGraphFailedTarget> {
    pub(crate) fn new(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: ParserPendingScriptId<FrameDocumentOwner>,
        script: PreparedScript,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        tree_id: Option<module_tree::ModuleTreeId>,
        load_delay_token: impl Into<ChildDocumentModuleScriptLoadDelay>,
        error: ModuleLoadError,
    ) -> Self {
        Self::with_target(
            FrameDocumentModuleGraphFailedTarget::from_graph_failed_parts(
                owner,
                realm_id,
                pending_script_id,
                script_handle,
                request_key,
                tree_id,
                load_delay_token,
            ),
            script,
            error,
        )
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.target.owner()
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        self.target.pending_script_id()
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.target.realm_id()
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.target.script_handle()
    }

    pub(crate) fn request_key(&self) -> &ModuleMapKey {
        self.target.request_key()
    }

    pub(crate) fn tree_id(&self) -> Option<module_tree::ModuleTreeId> {
        self.target.tree_id()
    }

    pub(crate) fn load_delay_token(&self) -> ChildDocumentModuleScriptLoadDelay {
        self.target.load_delay_token()
    }
}

impl<Target> ModuleScriptGraphFailedWork<Target> {
    pub(crate) fn with_target(
        target: Target,
        script: PreparedScript,
        error: ModuleLoadError,
    ) -> Self {
        Self {
            target,
            script,
            error,
        }
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        &self.script
    }

    pub(crate) fn error(&self) -> &ModuleLoadError {
        &self.error
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleScriptGraphReadyWork<Target = FrameDocumentModuleGraphReadyTarget> {
    target: Target,
    script: PreparedScript,
    graph: ModuleGraphHandle,
}

impl ModuleScriptGraphReadyWork<FrameDocumentModuleGraphReadyTarget> {
    pub(crate) fn new(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: ParserPendingScriptId<FrameDocumentOwner>,
        script: PreparedScript,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        tree_id: module_tree::ModuleTreeId,
        load_delay_token: impl Into<ChildDocumentModuleScriptLoadDelay>,
        graph: ModuleGraphHandle,
    ) -> Self {
        Self::with_target(
            FrameDocumentModuleGraphReadyTarget::from_graph_ready_parts(
                owner,
                realm_id,
                pending_script_id,
                script_handle,
                request_key,
                tree_id,
                load_delay_token,
            ),
            script,
            graph,
        )
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.target.owner()
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        self.target.pending_script_id()
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.target.realm_id()
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.target.script_handle()
    }

    pub(crate) fn request_key(&self) -> &ModuleMapKey {
        self.target.request_key()
    }

    pub(crate) fn tree_id(&self) -> module_tree::ModuleTreeId {
        self.target.tree_id()
    }

    pub(crate) fn load_delay_token(&self) -> ChildDocumentModuleScriptLoadDelay {
        self.target.load_delay_token()
    }
}

impl<Target> ModuleScriptGraphReadyWork<Target> {
    pub(crate) fn with_target(
        target: Target,
        script: PreparedScript,
        graph: ModuleGraphHandle,
    ) -> Self {
        Self {
            target,
            script,
            graph,
        }
    }

    pub(crate) fn into_parts(self) -> (Target, PreparedScript, ModuleGraphHandle) {
        (self.target, self.script, self.graph)
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        &self.script
    }

    pub(crate) fn entry_id(&self) -> ModuleEntryId {
        self.graph.root_entry
    }

    pub(crate) fn dependency_count(&self) -> usize {
        self.graph.entries.len().saturating_sub(1)
    }

    pub(crate) fn graph(&self) -> &ModuleGraphHandle {
        &self.graph
    }
}

#[derive(Debug)]
pub(super) struct ParserModuleScriptRunner<
    Target = FrameDocumentModuleGraphReadyTarget,
    ParserModuleGraphFailure = std::convert::Infallible,
> {
    key_by_node: HashMap<NodeId, ParserPendingScriptKey>,
    pending: BTreeMap<
        ParserPendingScriptKey,
        DocumentParserModulePendingScript<
            ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
        >,
    >,
}

#[derive(Debug)]
struct DocumentParserModulePendingScript<T> {
    node_id: NodeId,
    state: ParserPendingModuleScriptState<T>,
    is_async: bool,
    retained_by_parser_order: bool,
    blocking_stylesheet_signatures:
        HashSet<crate::stylesheet_blocking::DocumentBlockingStylesheetSignature>,
}

impl<T> DocumentParserModulePendingScript<T> {
    fn new(script: &PreparedScript) -> Self {
        Self {
            node_id: script.node_id,
            state: ParserPendingModuleScriptState::new(),
            is_async: script.mode == crate::types::ScriptMode::Async,
            retained_by_parser_order: false,
            blocking_stylesheet_signatures: HashSet::new(),
        }
    }
}

impl<Target, ParserModuleGraphFailure> Default
    for ParserModuleScriptRunner<Target, ParserModuleGraphFailure>
{
    fn default() -> Self {
        Self {
            key_by_node: HashMap::new(),
            pending: BTreeMap::new(),
        }
    }
}

impl<Target, ParserModuleGraphFailure> ParserModuleScriptRunner<Target, ParserModuleGraphFailure> {
    pub(super) fn register(&mut self, script: &PreparedScript) -> ParserPendingScriptKey {
        if self.key_by_node.contains_key(&script.node_id) {
            return self.key_by_node[&script.node_id];
        }
        let key = ParserPendingScriptKey::from_script(script);
        self.key_by_node.insert(script.node_id, key);
        self.pending
            .insert(key, DocumentParserModulePendingScript::new(script));
        key
    }

    pub(super) fn accept_parser_ordered(
        &mut self,
        script: &PreparedScript,
        blocking_stylesheet_signatures: HashSet<
            crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
        >,
    ) -> Option<ParserPendingScriptKey> {
        let expected_key = ParserPendingScriptKey::from_script(script);
        let key = self.register(script);
        if key != expected_key {
            return None;
        }
        let pending_script = self.pending.get_mut(&key)?;
        pending_script.retained_by_parser_order = true;
        pending_script.blocking_stylesheet_signatures = blocking_stylesheet_signatures;
        tracing::debug!(
            parser_position = key.parser_position(),
            script_node_id = ?key.script_node_id(),
            "accepted module PendingScript into parser order before graph start"
        );
        Some(key)
    }

    pub(super) fn prepare_parser_ordered_terminal(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> ParserOrderedModuleTerminalState {
        let Some(pending_script) = self.pending.get_mut(&key) else {
            return ParserOrderedModuleTerminalState::Missing;
        };
        if !pending_script.retained_by_parser_order {
            return ParserOrderedModuleTerminalState::Missing;
        }
        if pending_script.state.has_terminal() {
            return ParserOrderedModuleTerminalState::Ready;
        }
        pending_script.state.mark_watching_for_load();
        ParserOrderedModuleTerminalState::Waiting
    }

    pub(super) fn watch(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> ParserModulePendingScriptWatchResult<Target, ParserModuleGraphFailure> {
        if !self.mark_watching(key) {
            return ParserModulePendingScriptWatchResult::Missing;
        }

        if self.is_retained_by_parser_order(key) {
            return ParserModulePendingScriptWatchResult::WaitingForTree;
        }

        let ready = self.take_ready_terminals(key);
        if ready.is_empty() {
            ParserModulePendingScriptWatchResult::WaitingForTree
        } else {
            ParserModulePendingScriptWatchResult::Ready(ready)
        }
    }

    pub(super) fn notify_module_tree_load_finished(
        &mut self,
        key: ParserPendingScriptKey,
        work: ModuleScriptGraphReadyWork<Target>,
    ) -> Option<Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>>> {
        let terminal = ParserModuleGraphTerminalWork::Ready(Box::new(work));
        self.record_terminal(key, terminal)?;
        if self.is_retained_by_parser_order(key) {
            return Some(Vec::new());
        }
        Some(self.take_ready_terminals(key))
    }

    pub(super) fn notify_module_tree_load_failed(
        &mut self,
        key: ParserPendingScriptKey,
        failure: ParserModuleGraphFailure,
    ) -> Option<Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>>> {
        let terminal = ParserModuleGraphTerminalWork::Failed(failure);
        self.record_terminal(key, terminal)?;
        if self.is_retained_by_parser_order(key) {
            return Some(Vec::new());
        }
        Some(self.take_ready_terminals(key))
    }

    pub(super) fn blocking_stylesheet_signatures(
        &self,
        key: ParserPendingScriptKey,
    ) -> Option<&HashSet<crate::stylesheet_blocking::DocumentBlockingStylesheetSignature>> {
        Some(&self.pending.get(&key)?.blocking_stylesheet_signatures)
    }

    pub(super) fn is_retained_by_parser_order(&self, key: ParserPendingScriptKey) -> bool {
        self.pending
            .get(&key)
            .is_some_and(|pending| pending.retained_by_parser_order)
    }

    pub(super) fn mark_watching(&mut self, key: ParserPendingScriptKey) -> bool {
        let Some(pending_script) = self.pending.get_mut(&key) else {
            return false;
        };
        pending_script.state.mark_watching_for_load();
        true
    }

    pub(super) fn has_terminal(&self, key: ParserPendingScriptKey) -> bool {
        self.pending
            .get(&key)
            .is_some_and(|pending| pending.state.has_terminal())
    }

    pub(super) fn take_parser_ordered_ready_terminal(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> Option<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>> {
        if !self.is_retained_by_parser_order(key) {
            return None;
        }
        let terminal = self.pending.get_mut(&key)?.state.take_terminal()?;
        self.remove_pending(key);
        Some(terminal)
    }

    fn record_terminal(
        &mut self,
        key: ParserPendingScriptKey,
        terminal: ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
    ) -> Option<()> {
        self.pending
            .get_mut(&key)?
            .state
            .notify_module_tree_load_finished(terminal);
        Some(())
    }

    fn take_ready_terminals(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>> {
        // Async modules belong to the execute-as-soon-as-possible set, so only
        // their own readiness controls delivery, regardless of document order.
        if let Some(pending) = self.pending.get_mut(&key)
            && pending.is_async
        {
            let Some(terminal) = pending.state.take_ready_terminal() else {
                return Vec::new();
            };
            self.remove_pending(key);
            return vec![terminal];
        }
        let mut ready = Vec::new();
        while let Some(key) = self.next_pending_key_in_document_order() {
            let Some(pending) = self.pending.get(&key) else {
                break;
            };
            if !pending.state.has_ready_terminal() {
                break;
            }

            let mut pending = self.remove_pending(key);
            if let Some(terminal) = pending.state.take_ready_terminal() {
                ready.push(terminal);
            }
        }
        ready
    }

    fn next_pending_key_in_document_order(&self) -> Option<ParserPendingScriptKey> {
        self.pending.iter().find_map(|(key, pending)| {
            (!pending.retained_by_parser_order && !pending.is_async).then_some(*key)
        })
    }

    fn remove_pending(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> DocumentParserModulePendingScript<
        ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
    > {
        let pending = self
            .pending
            .remove(&key)
            .expect("pending parser module should still exist");
        self.key_by_node.remove(&pending.node_id);
        pending
    }

    #[cfg(test)]
    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub(super) fn has_lifecycle_blocking_pending_script(&self) -> bool {
        self.pending
            .values()
            .any(|pending| pending.retained_by_parser_order && pending.state.is_watching_for_load())
    }

    pub(super) fn contains(&self, key: ParserPendingScriptKey) -> bool {
        self.pending.contains_key(&key)
    }

    pub(super) fn discard(&mut self, key: ParserPendingScriptKey) -> bool {
        let Some(pending) = self.pending.remove(&key) else {
            return false;
        };
        self.key_by_node.remove(&pending.node_id);
        true
    }

    #[cfg(test)]
    pub(super) fn is_watching_for_test(&self, key: ParserPendingScriptKey) -> bool {
        self.pending
            .get(&key)
            .is_some_and(|pending| pending.state.is_watching_for_load())
    }
}
