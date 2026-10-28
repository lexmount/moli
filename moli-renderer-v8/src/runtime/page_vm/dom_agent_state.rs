use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

use super::backend_node_registry::{
    RendererBackendNodeKey, SharedRendererBackendNodeRegistry,
    new_shared_renderer_backend_node_registry,
};
use super::frontend_node_bindings::RendererFrontendNodeBindings;
use crate::document_runtime::DomHandle;
use crate::frame_owner_model::DocumentId;
use moli_page_types::DocumentNodeInspectorIdentity;

const MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION: usize = 1_024;

type NodeCreationStackTraceKey = (DocumentId, DomHandle);
type SharedNodeCreationStackTrace = Rc<crate::RendererDomNodeCreationStackTrace>;

#[derive(Debug, Clone)]
struct NodeCreationStackTraceStore {
    traces: HashMap<NodeCreationStackTraceKey, SharedNodeCreationStackTrace>,
    insertion_order: VecDeque<NodeCreationStackTraceKey>,
    capacity: usize,
}

impl Default for NodeCreationStackTraceStore {
    fn default() -> Self {
        Self::with_capacity(MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION)
    }
}

impl NodeCreationStackTraceStore {
    fn with_capacity(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "node creation stack trace capacity must be positive"
        );
        Self {
            // Most DOM sessions never enable this diagnostic. Keep the store
            // allocation lazy rather than reserving the full bound per session.
            traces: HashMap::new(),
            insertion_order: VecDeque::new(),
            capacity,
        }
    }

    fn insert(&mut self, key: NodeCreationStackTraceKey, trace: SharedNodeCreationStackTrace) {
        if let Some(existing) = self.traces.get_mut(&key) {
            *existing = trace;
            return;
        }
        while self.traces.len() >= self.capacity {
            let oldest = self
                .insertion_order
                .pop_front()
                .expect("bounded stack trace order must match its entries");
            let removed = self.traces.remove(&oldest);
            debug_assert!(removed.is_some());
        }
        self.insertion_order.push_back(key);
        self.traces.insert(key, trace);
    }

    fn get_cloned(
        &self,
        key: &NodeCreationStackTraceKey,
    ) -> Option<crate::RendererDomNodeCreationStackTrace> {
        self.traces.get(key).map(|trace| trace.as_ref().clone())
    }

    fn clear(&mut self) {
        self.traces.clear();
        self.insertion_order.clear();
    }
}

#[derive(Clone)]
pub(crate) struct RendererDomAgentState {
    inner: Rc<RefCell<RendererDomAgentStateInner>>,
}

struct RendererDomAgentStateInner {
    sessions: HashMap<Option<String>, RendererDomAgentSessionState>,
    backend_nodes: SharedRendererBackendNodeRegistry,
}

impl Default for RendererDomAgentState {
    fn default() -> Self {
        Self::new(new_shared_renderer_backend_node_registry())
    }
}

impl RendererDomAgentState {
    pub(crate) fn new(backend_nodes: SharedRendererBackendNodeRegistry) -> Self {
        Self {
            inner: Rc::new(RefCell::new(RendererDomAgentStateInner {
                sessions: HashMap::new(),
                backend_nodes,
            })),
        }
    }

    fn with_session_mut<T>(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        op: impl FnOnce(&mut RendererDomAgentSessionState) -> T,
    ) -> T {
        let key = inspector_session_id.map(str::to_owned);
        let mut inner = self.inner.borrow_mut();
        let state = inner.sessions.entry(key).or_default();
        state.reset_for_document(document_id);
        op(state)
    }

    pub(crate) fn discard_frontend_bindings(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.discard_frontend_bindings()
        });
    }

    pub(crate) fn discard_all_frontend_bindings(&self, document_id: Option<DocumentId>) {
        for state in self.inner.borrow_mut().sessions.values_mut() {
            state.discard_frontend_bindings_for_document(document_id);
        }
    }

    pub(crate) fn remove_session(&self, inspector_session_id: Option<&str>) {
        self.inner
            .borrow_mut()
            .sessions
            .remove(&inspector_session_id.map(str::to_owned));
    }

    pub(crate) fn reset_for_document_replacement(&self, document_id: DocumentId) {
        for state in self.inner.borrow_mut().sessions.values_mut() {
            state.reset_for_document(Some(document_id));
        }
    }

    pub(crate) fn has_frontend_bindings(&self) -> bool {
        self.inner
            .borrow()
            .sessions
            .values()
            .any(RendererDomAgentSessionState::has_frontend_bindings)
    }

    pub(crate) fn session_keys(&self) -> Vec<Option<String>> {
        let mut keys = self
            .inner
            .borrow()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub(crate) fn set_include_whitespace(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        include_whitespace: bool,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.include_whitespace = include_whitespace;
        });
    }

    pub(crate) fn includes_whitespace(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
    ) -> bool {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.include_whitespace
        })
    }

    pub(crate) fn mark_children_requested(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
        child_count: usize,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.mark_children_requested(backend_node_id, child_count)
        });
    }

    pub(crate) fn cache_child_count(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
        child_count: usize,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.cache_child_count(backend_node_id, child_count)
        });
    }

    pub(crate) fn cached_child_count(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> Option<usize> {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.cached_child_count(backend_node_id)
        })
    }

    pub(crate) fn children_requested(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> bool {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.children_requested(backend_node_id)
        })
    }

    pub(crate) fn mark_node_published(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.published_nodes.insert(backend_node_id);
        });
    }

    pub(crate) fn node_is_published(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> bool {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.published_nodes.contains(&backend_node_id)
        })
    }

    pub(crate) fn frontend_node_id_for_existing_backend_node_id(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> Option<u32> {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.frontend_node_id_for_existing_backend_node_id(backend_node_id)
        })
    }

    pub(crate) fn published_frontend_node_id_for_backend_node_id(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> Option<u32> {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            if !state.published_nodes.contains(&backend_node_id) {
                return None;
            }
            state.frontend_node_id_for_existing_backend_node_id(backend_node_id)
        })
    }

    pub(crate) fn remove_frontend_bindings_for_backend_node_ids(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_ids: impl IntoIterator<Item = u32>,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            for backend_node_id in backend_node_ids {
                state.remove_frontend_binding_for_backend_node_id(backend_node_id);
            }
        });
    }

    pub(crate) fn backend_node_id_for_node(
        &self,
        document_id: DocumentId,
        handle: DomHandle,
    ) -> u32 {
        let backend_nodes = self.inner.borrow().backend_nodes.clone();
        backend_nodes.borrow_mut().id_for_node(document_id, handle)
    }

    pub(crate) fn backend_node_id_for_inspector_node(
        &self,
        document_id: DocumentId,
        host: DomHandle,
        inspector_identity: DocumentNodeInspectorIdentity,
    ) -> u32 {
        let backend_nodes = self.inner.borrow().backend_nodes.clone();
        backend_nodes
            .borrow_mut()
            .id_for_inspector_node(document_id, host, inspector_identity)
    }

    pub(crate) fn backend_node_key_for_id(
        &self,
        backend_node_id: u32,
    ) -> Option<RendererBackendNodeKey> {
        let backend_nodes = self.inner.borrow().backend_nodes.clone();
        backend_nodes.borrow().key_for_id(backend_node_id)
    }

    pub(crate) fn retain_detached_backend_node_resolution(&self, backend_node_id: u32) -> bool {
        let backend_nodes = self.inner.borrow().backend_nodes.clone();
        backend_nodes
            .borrow_mut()
            .retain_detached_resolution(backend_node_id)
    }

    pub(crate) fn backend_node_resolves_while_detached(&self, backend_node_id: u32) -> bool {
        let backend_nodes = self.inner.borrow().backend_nodes.clone();
        backend_nodes
            .borrow()
            .resolves_while_detached(backend_node_id)
    }

    pub(crate) fn remove_stale_backend_node_id(
        &self,
        backend_node_id: u32,
        key: RendererBackendNodeKey,
    ) {
        let backend_nodes = self.inner.borrow().backend_nodes.clone();
        backend_nodes
            .borrow_mut()
            .remove_stale_id(backend_node_id, key);
    }

    pub(crate) fn frontend_node_id_for_backend_node_id(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> u32 {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.frontend_node_id_for_backend_node_id(backend_node_id)
        })
    }

    pub(crate) fn frontend_node_binding(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        frontend_node_id: u32,
    ) -> crate::RendererDomFrontendNodeBindingResolution {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.frontend_node_binding(frontend_node_id)
        })
    }

    pub(crate) fn has_frontend_node_id_for_backend_node_id(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> bool {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.has_frontend_node_id_for_backend_node_id(backend_node_id)
        })
    }

    pub(crate) fn register_bidi_node_binding(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        shared_id: String,
        backend_node_id: u32,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.register_bidi_node_binding(shared_id, backend_node_id)
        });
    }

    pub(super) fn bidi_node_binding(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        shared_id: &str,
    ) -> crate::RendererDomBidiNodeBindingResolution {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.bidi_node_binding(shared_id)
        })
    }

    pub(super) fn bidi_node_shared_id_for_backend_node_id(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_id: u32,
    ) -> crate::RendererDomBidiNodeSharedIdResolution {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.bidi_node_shared_id_for_backend_node_id(backend_node_id)
        })
    }

    pub(super) fn register_search_results(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        backend_node_ids: Vec<u32>,
    ) -> crate::RendererDomSearchRegistration {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.register_search_results(backend_node_ids)
        })
    }

    pub(super) fn search_results_slice(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        search_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> crate::RendererDomSearchResultsResolution {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.search_results_slice(search_id, from_index, to_index)
        })
    }

    pub(super) fn discard_search_results(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        search_id: &str,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.discard_search_results(search_id)
        });
    }

    pub(crate) fn set_node_stack_traces_enabled(
        &self,
        inspector_session_id: Option<&str>,
        document_id: Option<DocumentId>,
        enabled: bool,
    ) {
        self.with_session_mut(inspector_session_id, document_id, |state| {
            state.capture_node_stack_traces = enabled;
        });
    }

    pub(crate) fn has_node_stack_trace_capture_interest(&self) -> bool {
        self.inner
            .borrow()
            .sessions
            .values()
            .any(|state| state.capture_node_stack_traces)
    }

    pub(crate) fn record_node_creation_stack_trace(
        &self,
        document_id: DocumentId,
        handle: DomHandle,
        trace: crate::RendererDomNodeCreationStackTrace,
    ) {
        let trace = Rc::new(trace);
        for state in self.inner.borrow_mut().sessions.values_mut() {
            if state.capture_node_stack_traces {
                state
                    .node_creation_stack_traces
                    .insert((document_id, handle), Rc::clone(&trace));
            }
        }
    }

    pub(crate) fn node_creation_stack_trace(
        &self,
        inspector_session_id: Option<&str>,
        session_document_id: Option<DocumentId>,
        node_document_id: DocumentId,
        handle: DomHandle,
    ) -> Option<crate::RendererDomNodeCreationStackTrace> {
        self.with_session_mut(inspector_session_id, session_document_id, |state| {
            state
                .node_creation_stack_traces
                .get_cloned(&(node_document_id, handle))
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct RendererDomAgentSessionState {
    document_id: Option<DocumentId>,
    include_whitespace: bool,
    frontend_node_bindings: RendererFrontendNodeBindings,
    bidi_node_bindings: HashMap<String, u32>,
    bidi_node_shared_ids_by_backend_node_id: HashMap<u32, String>,
    // Search matches do not allocate frontend IDs. Only reading a range can
    // publish its nodes and expose their IDs to the client.
    search_results: HashMap<String, Vec<u32>>,
    // A binding may be allocated while preparing a snapshot. Only DOM tree
    // responses/notifications make it a valid path anchor for the client.
    published_nodes: HashSet<u32>,
    children_requested: HashSet<u32>,
    cached_child_counts: HashMap<u32, usize>,
    capture_node_stack_traces: bool,
    node_creation_stack_traces: NodeCreationStackTraceStore,
    next_search_id: u32,
}

impl RendererDomAgentSessionState {
    pub(super) fn reset_for_document(&mut self, document_id: Option<DocumentId>) {
        if self.document_id == document_id {
            return;
        }
        self.document_id = document_id;
        self.discard_frontend_bindings();
        self.node_creation_stack_traces.clear();
    }

    fn discard_frontend_bindings(&mut self) {
        self.frontend_node_bindings.clear();
        self.bidi_node_bindings.clear();
        self.bidi_node_shared_ids_by_backend_node_id.clear();
        self.search_results.clear();
        self.published_nodes.clear();
        self.children_requested.clear();
        self.cached_child_counts.clear();
    }

    fn discard_frontend_bindings_for_document(&mut self, document_id: Option<DocumentId>) {
        if self.document_id == document_id {
            self.discard_frontend_bindings();
        } else {
            // Moving the session to another Document also retires creation
            // traces owned by the previous Document.
            self.reset_for_document(document_id);
        }
    }

    fn has_frontend_bindings(&self) -> bool {
        !self.frontend_node_bindings.is_empty()
    }

    fn mark_children_requested(&mut self, backend_node_id: u32, child_count: usize) {
        self.children_requested.insert(backend_node_id);
        self.cache_child_count(backend_node_id, child_count);
    }

    fn cache_child_count(&mut self, backend_node_id: u32, child_count: usize) {
        self.cached_child_counts
            .insert(backend_node_id, child_count);
    }

    fn cached_child_count(&self, backend_node_id: u32) -> Option<usize> {
        self.cached_child_counts.get(&backend_node_id).copied()
    }

    fn children_requested(&self, backend_node_id: u32) -> bool {
        self.children_requested.contains(&backend_node_id)
    }

    fn frontend_node_id_for_existing_backend_node_id(&self, backend_node_id: u32) -> Option<u32> {
        self.frontend_node_bindings
            .frontend_node_id_for_backend_node_id(backend_node_id)
    }

    fn remove_frontend_binding_for_backend_node_id(&mut self, backend_node_id: u32) {
        self.frontend_node_bindings
            .remove_backend_node_id(backend_node_id);
        self.published_nodes.remove(&backend_node_id);
        self.children_requested.remove(&backend_node_id);
        self.cached_child_counts.remove(&backend_node_id);
    }

    pub(super) fn frontend_node_id_for_backend_node_id(&mut self, backend_node_id: u32) -> u32 {
        self.frontend_node_bindings
            .id_for_backend_node_id(backend_node_id)
    }

    pub(super) fn frontend_node_binding(
        &self,
        frontend_node_id: u32,
    ) -> crate::RendererDomFrontendNodeBindingResolution {
        self.frontend_node_bindings
            .backend_node_id_for_frontend_node_id(frontend_node_id)
            .map(crate::RendererDomFrontendNodeBindingResolution::BackendNodeId)
            .unwrap_or(crate::RendererDomFrontendNodeBindingResolution::NotFound)
    }

    pub(super) fn has_frontend_node_id_for_backend_node_id(&self, backend_node_id: u32) -> bool {
        self.frontend_node_bindings
            .has_backend_node_id(backend_node_id)
    }

    pub(super) fn register_bidi_node_binding(&mut self, shared_id: String, backend_node_id: u32) {
        self.bidi_node_shared_ids_by_backend_node_id
            .insert(backend_node_id, shared_id.clone());
        self.bidi_node_bindings.insert(shared_id, backend_node_id);
    }

    pub(super) fn bidi_node_binding(
        &self,
        shared_id: &str,
    ) -> crate::RendererDomBidiNodeBindingResolution {
        self.bidi_node_bindings
            .get(shared_id)
            .copied()
            .map(crate::RendererDomBidiNodeBindingResolution::BackendNodeId)
            .unwrap_or(crate::RendererDomBidiNodeBindingResolution::NotFound)
    }

    pub(super) fn bidi_node_shared_id_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> crate::RendererDomBidiNodeSharedIdResolution {
        self.bidi_node_shared_ids_by_backend_node_id
            .get(&backend_node_id)
            .cloned()
            .map(crate::RendererDomBidiNodeSharedIdResolution::SharedId)
            .unwrap_or(crate::RendererDomBidiNodeSharedIdResolution::NotFound)
    }

    pub(super) fn register_search_results(
        &mut self,
        backend_node_ids: Vec<u32>,
    ) -> crate::RendererDomSearchRegistration {
        let search_id = self.next_search_id.to_string();
        self.next_search_id += 1;
        let result_count = backend_node_ids.len() as u32;
        self.search_results
            .insert(search_id.clone(), backend_node_ids);
        crate::RendererDomSearchRegistration {
            search_id,
            result_count,
        }
    }

    pub(super) fn discard_search_results(&mut self, search_id: &str) {
        self.search_results.remove(search_id);
    }

    pub(super) fn search_results_slice(
        &self,
        search_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> crate::RendererDomSearchResultsResolution {
        let Some(results) = self.search_results.get(search_id) else {
            return crate::RendererDomSearchResultsResolution::SearchResultNotFound;
        };
        if from_index >= to_index {
            return crate::RendererDomSearchResultsResolution::BadIndices;
        }
        if from_index >= results.len() {
            return crate::RendererDomSearchResultsResolution::BadFromIndex;
        }
        if to_index > results.len() {
            return crate::RendererDomSearchResultsResolution::BadToIndex;
        }
        crate::RendererDomSearchResultsResolution::Found(
            results[from_index..to_index]
                .iter()
                .map(|&backend_node_id| crate::RendererDomSearchResultNode {
                    frontend_node_id: 0,
                    backend_node_id,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document_id(value: u64) -> DocumentId {
        DocumentId(value)
    }

    fn dom_handle(value: usize) -> DomHandle {
        DomHandle::new(value)
    }

    fn node_creation_stack_trace(
        function_name: impl Into<String>,
    ) -> crate::RendererDomNodeCreationStackTrace {
        crate::RendererDomNodeCreationStackTrace {
            call_frames: vec![crate::RendererDomNodeCreationStackFrame {
                function_name: function_name.into(),
                script_id: "1".to_owned(),
                url: "stack-store-test.js".to_owned(),
                line_number: 0,
                column_number: 0,
            }],
        }
    }

    fn shared_node_creation_stack_trace(
        function_name: impl Into<String>,
    ) -> SharedNodeCreationStackTrace {
        Rc::new(node_creation_stack_trace(function_name))
    }

    #[test]
    fn node_creation_stack_trace_store_evicts_oldest_without_duplicate_order_entries() {
        let mut store = NodeCreationStackTraceStore::with_capacity(2);
        let first = (document_id(1), dom_handle(1));
        let second = (document_id(1), dom_handle(2));
        let third = (document_id(1), dom_handle(3));

        store.insert(first, shared_node_creation_stack_trace("first"));
        store.insert(second, shared_node_creation_stack_trace("second"));
        store.insert(first, shared_node_creation_stack_trace("updated-first"));
        assert_eq!(store.traces.len(), 2);
        assert_eq!(store.insertion_order.len(), 2);
        assert_eq!(
            store.get_cloned(&first).unwrap().call_frames[0].function_name,
            "updated-first"
        );

        store.insert(third, shared_node_creation_stack_trace("third"));
        assert!(store.get_cloned(&first).is_none());
        assert!(store.get_cloned(&second).is_some());
        assert!(store.get_cloned(&third).is_some());
        assert_eq!(store.traces.len(), 2);
        assert_eq!(store.insertion_order.len(), 2);

        store.clear();
        assert!(store.traces.is_empty());
        assert!(store.insertion_order.is_empty());
    }

    #[test]
    fn agent_state_bounds_traces_per_session_and_shares_each_captured_payload() {
        let state = RendererDomAgentState::default();
        let document_id = document_id(1);
        for session_id in ["session-1", "session-2"] {
            state.set_node_stack_traces_enabled(Some(session_id), Some(document_id), true);
        }

        for index in 0..=MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION {
            state.record_node_creation_stack_trace(
                document_id,
                dom_handle(index),
                node_creation_stack_trace(index.to_string()),
            );
        }

        for session_id in ["session-1", "session-2"] {
            assert!(
                state
                    .node_creation_stack_trace(
                        Some(session_id),
                        Some(document_id),
                        document_id,
                        dom_handle(0),
                    )
                    .is_none(),
                "the oldest trace must be evicted for {session_id}"
            );
            assert_eq!(
                state
                    .node_creation_stack_trace(
                        Some(session_id),
                        Some(document_id),
                        document_id,
                        dom_handle(MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION),
                    )
                    .unwrap()
                    .call_frames[0]
                    .function_name,
                MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION.to_string()
            );
        }

        let inner = state.inner.borrow();
        let first_store = &inner
            .sessions
            .get(&Some("session-1".to_owned()))
            .unwrap()
            .node_creation_stack_traces;
        let second_store = &inner
            .sessions
            .get(&Some("session-2".to_owned()))
            .unwrap()
            .node_creation_stack_traces;
        assert_eq!(
            first_store.traces.len(),
            MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION
        );
        assert_eq!(
            second_store.traces.len(),
            MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION
        );
        let newest = (
            document_id,
            dom_handle(MAX_RETAINED_NODE_CREATION_STACK_TRACES_PER_SESSION),
        );
        assert!(Rc::ptr_eq(
            first_store.traces.get(&newest).unwrap(),
            second_store.traces.get(&newest).unwrap()
        ));
    }

    #[test]
    fn document_replacement_releases_old_stacks_before_new_nodes_are_captured() {
        let state = RendererDomAgentState::default();
        let first_document = document_id(1);
        let second_document = document_id(2);
        state.set_node_stack_traces_enabled(Some("session-1"), Some(first_document), true);
        state.record_node_creation_stack_trace(
            first_document,
            dom_handle(1),
            node_creation_stack_trace("captured"),
        );
        assert!(
            state
                .node_creation_stack_trace(
                    Some("session-1"),
                    Some(first_document),
                    first_document,
                    dom_handle(1),
                )
                .is_some()
        );

        state.reset_for_document_replacement(second_document);
        state.record_node_creation_stack_trace(
            second_document,
            dom_handle(2),
            node_creation_stack_trace("replacement"),
        );
        assert!(
            state
                .node_creation_stack_trace(
                    Some("session-1"),
                    Some(second_document),
                    first_document,
                    dom_handle(1),
                )
                .is_none()
        );
        assert_eq!(
            state
                .node_creation_stack_trace(
                    Some("session-1"),
                    Some(second_document),
                    second_document,
                    dom_handle(2),
                )
                .unwrap()
                .call_frames[0]
                .function_name,
            "replacement",
            "the first trace captured for the replacement document must survive its first lookup"
        );
        assert!(state.has_node_stack_trace_capture_interest());

        state.remove_session(Some("session-1"));
        assert!(!state.has_node_stack_trace_capture_interest());
        assert!(state.inner.borrow().sessions.is_empty());
    }

    #[test]
    fn agent_state_resets_session_bindings_on_document_change() {
        let state = RendererDomAgentState::default();
        let backend_node_id = state.backend_node_id_for_node(document_id(1), dom_handle(7));
        let frontend_node_id = state.frontend_node_id_for_backend_node_id(
            Some("session-1"),
            Some(document_id(1)),
            backend_node_id,
        );

        assert_eq!(frontend_node_id, 1);
        assert_eq!(
            state.frontend_node_binding(Some("session-1"), Some(document_id(1)), frontend_node_id),
            crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id)
        );
        assert_eq!(
            state.frontend_node_binding(Some("session-1"), Some(document_id(2)), frontend_node_id),
            crate::RendererDomFrontendNodeBindingResolution::NotFound
        );
        assert_eq!(
            state.backend_node_key_for_id(backend_node_id),
            Some(RendererBackendNodeKey {
                document_id: document_id(1),
                handle: dom_handle(7),
                inspector_identity: None,
            })
        );
    }

    #[test]
    fn agent_state_resets_search_and_bidi_bindings_on_document_change() {
        let state = RendererDomAgentState::default();
        let backend_node_id = state.backend_node_id_for_node(document_id(1), dom_handle(7));
        state.register_bidi_node_binding(
            Some("session-1"),
            Some(document_id(1)),
            "shared-1".to_owned(),
            backend_node_id,
        );
        let search = state.register_search_results(
            Some("session-1"),
            Some(document_id(1)),
            vec![backend_node_id],
        );

        assert_eq!(
            state.bidi_node_binding(Some("session-1"), Some(document_id(1)), "shared-1"),
            crate::RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id)
        );
        assert!(matches!(
            state.search_results_slice(
                Some("session-1"),
                Some(document_id(1)),
                &search.search_id,
                0,
                1,
            ),
            crate::RendererDomSearchResultsResolution::Found(nodes)
                if nodes.len() == 1 && nodes[0].backend_node_id == backend_node_id
        ));

        assert_eq!(
            state.bidi_node_binding(Some("session-1"), Some(document_id(2)), "shared-1"),
            crate::RendererDomBidiNodeBindingResolution::NotFound
        );
        assert_eq!(
            state.search_results_slice(
                Some("session-1"),
                Some(document_id(2)),
                &search.search_id,
                0,
                1,
            ),
            crate::RendererDomSearchResultsResolution::SearchResultNotFound
        );
    }

    #[test]
    fn search_results_do_not_allocate_frontend_bindings() {
        let state = RendererDomAgentState::default();
        let backend_node_id = state.backend_node_id_for_node(document_id(1), dom_handle(7));
        let search = state.register_search_results(
            Some("session-1"),
            Some(document_id(1)),
            vec![backend_node_id],
        );

        assert!(matches!(
            state.search_results_slice(
                Some("session-1"),
                Some(document_id(1)),
                &search.search_id,
                0,
                1,
            ),
            crate::RendererDomSearchResultsResolution::Found(nodes)
                if nodes.len() == 1
                    && nodes[0].frontend_node_id == 0
                    && nodes[0].backend_node_id == backend_node_id
        ));
        assert!(!state.has_frontend_node_id_for_backend_node_id(
            Some("session-1"),
            Some(document_id(1)),
            backend_node_id,
        ));
        assert!(
            !state.node_is_published(Some("session-1"), Some(document_id(1)), backend_node_id,)
        );
    }

    #[test]
    fn discarding_frontend_bindings_preserves_monotonic_ids() {
        let state = RendererDomAgentState::default();
        let first_backend = state.backend_node_id_for_node(document_id(1), dom_handle(7));
        let first_frontend = state.frontend_node_id_for_backend_node_id(
            Some("session-1"),
            Some(document_id(1)),
            first_backend,
        );

        state.discard_frontend_bindings(Some("session-1"), Some(document_id(1)));

        assert_eq!(
            state.frontend_node_binding(Some("session-1"), Some(document_id(1)), first_frontend,),
            crate::RendererDomFrontendNodeBindingResolution::NotFound
        );
        let rebound = state.frontend_node_id_for_backend_node_id(
            Some("session-1"),
            Some(document_id(1)),
            first_backend,
        );
        assert!(rebound > first_frontend);
    }

    #[test]
    fn discarding_all_frontend_bindings_preserves_current_and_resets_stale_sessions() {
        let state = RendererDomAgentState::default();
        let document = document_id(1);
        let stale_document = document_id(2);
        let backend_node_id = state.backend_node_id_for_node(document, dom_handle(7));
        let mut frontend_node_ids = Vec::new();
        for (session_id, include_whitespace) in [("session-default", false), ("session-all", true)]
        {
            state.set_include_whitespace(Some(session_id), Some(document), include_whitespace);
            state.set_node_stack_traces_enabled(Some(session_id), Some(document), true);
            frontend_node_ids.push((
                session_id,
                include_whitespace,
                state.frontend_node_id_for_backend_node_id(
                    Some(session_id),
                    Some(document),
                    backend_node_id,
                ),
            ));
        }
        state.record_node_creation_stack_trace(
            document,
            dom_handle(7),
            node_creation_stack_trace("retained-across-dcl"),
        );
        state.set_node_stack_traces_enabled(
            Some("session-stale-document"),
            Some(stale_document),
            true,
        );
        state.record_node_creation_stack_trace(
            stale_document,
            dom_handle(8),
            node_creation_stack_trace("retired-across-document-change"),
        );

        state.discard_all_frontend_bindings(Some(document));

        for (session_id, include_whitespace, old_frontend_node_id) in frontend_node_ids {
            assert_eq!(
                state
                    .frontend_node_binding(Some(session_id), Some(document), old_frontend_node_id,),
                crate::RendererDomFrontendNodeBindingResolution::NotFound
            );
            assert_eq!(
                state.includes_whitespace(Some(session_id), Some(document)),
                include_whitespace
            );
            assert_eq!(
                state
                    .node_creation_stack_trace(
                        Some(session_id),
                        Some(document),
                        document,
                        dom_handle(7),
                    )
                    .unwrap()
                    .call_frames[0]
                    .function_name,
                "retained-across-dcl"
            );
            let rebound = state.frontend_node_id_for_backend_node_id(
                Some(session_id),
                Some(document),
                backend_node_id,
            );
            assert!(rebound > old_frontend_node_id);
        }
        assert!(
            state
                .node_creation_stack_trace(
                    Some("session-stale-document"),
                    Some(document),
                    stale_document,
                    dom_handle(8),
                )
                .is_none(),
            "a session advanced from another Document must retire that Document's traces"
        );
        assert!(state.has_node_stack_trace_capture_interest());
    }
}
