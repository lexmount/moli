use super::*;
use std::sync::Arc;

pub type DomHandle = NativeNodeId;

#[derive(Debug, Clone)]
pub struct HostElementSnapshot {
    pub text_content: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct NamedElementIndex {
    pub(super) handles_by_value: HashMap<Arc<str>, IndexSet<DomHandle>>,
    pub(super) value_by_handle: HashMap<DomHandle, Arc<str>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ShadowSlotNameIndex {
    pub(super) slots_in_tree_order: Vec<DomHandle>,
    pub(super) slots_by_name: HashMap<String, Vec<DomHandle>>,
}

#[derive(Debug, Clone)]
pub struct DomHost {
    pub(super) dom: NativeDom,
    pub(super) dom_version: Cell<u64>,
    pub(super) query_version: Cell<u64>,
    pub(super) shadow_root_binding_version: Cell<u64>,
    pub(super) connected_shadow_roots_version: Cell<u64>,
    /// Per-Document identity of the connected ShadowRoot TreeScope universe.
    ///
    /// The two counters above belong to the page-wide binding snapshot cache.
    /// Style consumers must not use those global counters: an iframe adding a
    /// ShadowRoot cannot invalidate the retained style world of its parent or
    /// a sibling frame.
    pub(super) document_tree_scope_versions: RefCell<HashMap<DomHandle, u64>>,
    pub(super) id_index: RefCell<Option<NamedElementIndex>>,
    pub(super) name_index: RefCell<Option<NamedElementIndex>>,
    pub(super) element_query_index: RefCell<ElementQueryIndex>,
    pub(super) live_collection_cache:
        RefCell<HashMap<LiveCollectionCacheKey, CachedLiveCollection>>,
    pub(super) shadow_roots_by_host: RefCell<HashMap<DomHandle, ShadowRootState>>,
    pub(super) shadow_hosts_by_root: RefCell<HashMap<DomHandle, DomHandle>>,
    pub(super) shadow_slot_name_indexes: RefCell<HashMap<DomHandle, ShadowSlotNameIndex>>,
    #[cfg(test)]
    pub(super) shadow_slot_name_index_build_count: Cell<u64>,
    pub(super) connected_shadow_roots_cache: RefCell<Option<CachedConnectedShadowRoots>>,
    pub(super) manual_slot_assignments: RefCell<HashMap<DomHandle, Vec<DomHandle>>>,
    pub(super) child_browsing_context_host_candidates: RefCell<Vec<DomHandle>>,
    pub(super) shadow_disabled_custom_element_definitions: RefCell<HashSet<String>>,
    pub(super) active_element: Cell<Option<DomHandle>>,
    pub(super) hovered_elements: RefCell<IndexSet<DomHandle>>,
    pub(super) mutation_observer_records_enabled: Cell<bool>,
    pub(super) devtools_mutation_records_enabled: Cell<bool>,
}

#[derive(Debug, Clone)]
pub(super) struct CachedConnectedShadowRoots {
    pub(super) shadow_root_binding_version: u64,
    pub(super) connected_shadow_roots_version: u64,
    pub(super) bindings: Vec<ConnectedShadowRootSnapshot>,
    pub(super) bindings_by_host: HashMap<DomHandle, ConnectedShadowRootSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowRootInit {
    mode: String,
    delegates_focus: bool,
    slot_assignment: String,
    clonable: bool,
    serializable: bool,
    reference_target: Option<String>,
    adopted_style_sheets: Option<String>,
    null_custom_element_registry: bool,
}

impl ShadowRootInit {
    pub fn new(mode: &str) -> Self {
        Self {
            mode: mode.to_owned(),
            delegates_focus: false,
            slot_assignment: "named".to_owned(),
            clonable: false,
            serializable: false,
            reference_target: None,
            adopted_style_sheets: None,
            null_custom_element_registry: false,
        }
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    pub fn delegates_focus(&self) -> bool {
        self.delegates_focus
    }

    pub fn set_delegates_focus(&mut self, delegates_focus: bool) {
        self.delegates_focus = delegates_focus;
    }

    pub fn slot_assignment(&self) -> &str {
        &self.slot_assignment
    }

    pub fn set_slot_assignment(&mut self, slot_assignment: &str) {
        self.slot_assignment = slot_assignment.to_owned();
    }

    pub fn clonable(&self) -> bool {
        self.clonable
    }

    pub fn set_clonable(&mut self, clonable: bool) {
        self.clonable = clonable;
    }

    pub fn serializable(&self) -> bool {
        self.serializable
    }

    pub fn set_serializable(&mut self, serializable: bool) {
        self.serializable = serializable;
    }

    pub fn reference_target(&self) -> Option<&str> {
        self.reference_target.as_deref()
    }

    pub fn set_reference_target(&mut self, reference_target: Option<String>) {
        self.reference_target = reference_target;
    }

    pub fn adopted_style_sheets(&self) -> Option<&str> {
        self.adopted_style_sheets.as_deref()
    }

    pub fn set_adopted_style_sheets(&mut self, adopted_style_sheets: Option<String>) {
        self.adopted_style_sheets = adopted_style_sheets;
    }

    pub fn null_custom_element_registry(&self) -> bool {
        self.null_custom_element_registry
    }

    pub fn set_null_custom_element_registry(&mut self, null_custom_element_registry: bool) {
        self.null_custom_element_registry = null_custom_element_registry;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedShadowRootSnapshot {
    pub host: DomHandle,
    pub root: DomHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowRootBindingSnapshot {
    pub host: DomHandle,
    pub root: DomHandle,
    pub init: ShadowRootInit,
    pub declarative: bool,
    pub available_to_element_internals: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShadowRootState {
    pub(super) handle: DomHandle,
    pub(super) init: ShadowRootInit,
    pub(super) declarative: bool,
    pub(super) available_to_element_internals: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MutationScope {
    LocalState,
    QueryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LiveCollectionCacheKind {
    ChildNodes,
    Children,
    FormControls,
    Options,
    SelectedOptions,
    TagName,
    TagNameNs,
    ClassName,
    Name,
    FormControlsByName,
    Forms,
    Images,
    Scripts,
    Links,
    Anchors,
    TableRows,
    TableBodies,
    TableSectionRows,
    TableRowCells,
}

impl LiveCollectionCacheKind {
    pub(super) fn parse(kind: &str) -> Option<Self> {
        match kind {
            "childNodes" => Some(Self::ChildNodes),
            "children" => Some(Self::Children),
            "formControls" => Some(Self::FormControls),
            "options" => Some(Self::Options),
            "selectedOptions" => Some(Self::SelectedOptions),
            "tagName" => Some(Self::TagName),
            "tagNameNs" => Some(Self::TagNameNs),
            "className" => Some(Self::ClassName),
            "name" => Some(Self::Name),
            "formControlsByName" => Some(Self::FormControlsByName),
            "forms" => Some(Self::Forms),
            "images" => Some(Self::Images),
            "scripts" => Some(Self::Scripts),
            "links" => Some(Self::Links),
            "anchors" => Some(Self::Anchors),
            "tableRows" => Some(Self::TableRows),
            "tableBodies" => Some(Self::TableBodies),
            "tableSectionRows" => Some(Self::TableSectionRows),
            "tableRowCells" => Some(Self::TableRowCells),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct LiveCollectionCacheKey {
    pub(super) root: DomHandle,
    pub(super) kind: LiveCollectionCacheKind,
    pub(super) query: Option<String>,
    pub(super) include_root: bool,
}

#[derive(Debug, Clone)]
pub(super) struct CachedLiveCollection {
    pub(super) version: u64,
    pub(super) handles: Vec<DomHandle>,
}
