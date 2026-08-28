use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
};

use super::super::{
    document_runtime::DomHandle,
    reflector::{DomPtr, ReflectorId, ReflectorRegistry},
};
use super::element::{control_label_handles, form_control_elements};
use super::{JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum BridgeHandle {
    Window,
    Node(DomHandle),
    ClassList(DomHandle, DomTokenListKind),
    Dataset(DomHandle),
    Style(DomHandle),
    ComputedStyle(DomHandle, ComputedStyleDescriptor),
}

impl BridgeHandle {
    pub(super) fn dom_handle(&self) -> Option<DomHandle> {
        match self {
            Self::Window => None,
            Self::Node(handle)
            | Self::ClassList(handle, _)
            | Self::Dataset(handle)
            | Self::Style(handle)
            | Self::ComputedStyle(handle, _) => Some(*handle),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ComputedStyleDescriptor {
    pub(crate) pseudo: ComputedStylePseudoKey,
    pub(crate) target: ComputedStyleTargetKey,
}

impl ComputedStyleDescriptor {
    pub(crate) fn new(pseudo: ComputedStylePseudoKey, target: ComputedStyleTargetKey) -> Self {
        Self { pseudo, target }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ComputedStylePseudoKey {
    Originating,
    ForcedEmpty,
    Before,
    After,
    Backdrop,
    Checkmark,
    FirstLetter,
    Selection,
    FileSelectorButton,
    GrammarError,
    Marker,
    Picker,
    PickerIcon,
    Placeholder,
    Highlight(String),
    SpellingError,
    ViewTransition,
    ViewTransitionGroup(String),
    ViewTransitionImagePair(String),
    ViewTransitionOld(String),
    ViewTransitionNew(String),
}

impl ComputedStylePseudoKey {
    pub(crate) fn from_stylo_pseudo(pseudo_element: &str) -> Option<Self> {
        match pseudo_element {
            "before" => Some(Self::Before),
            "after" => Some(Self::After),
            "backdrop" => Some(Self::Backdrop),
            "checkmark" => Some(Self::Checkmark),
            "first-letter" => Some(Self::FirstLetter),
            "selection" => Some(Self::Selection),
            "file-selector-button" => Some(Self::FileSelectorButton),
            "grammar-error" => Some(Self::GrammarError),
            "marker" => Some(Self::Marker),
            "picker(select)" => Some(Self::Picker),
            "picker-icon" => Some(Self::PickerIcon),
            "placeholder" => Some(Self::Placeholder),
            "spelling-error" => Some(Self::SpellingError),
            "view-transition" => Some(Self::ViewTransition),
            _ => {
                if let Some(name) = functional_pseudo_name(pseudo_element, "highlight") {
                    return Some(Self::Highlight(name.to_owned()));
                }
                if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-group")
                {
                    return Some(Self::ViewTransitionGroup(name.to_owned()));
                }
                if let Some(name) =
                    functional_pseudo_name(pseudo_element, "view-transition-image-pair")
                {
                    return Some(Self::ViewTransitionImagePair(name.to_owned()));
                }
                if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-old") {
                    return Some(Self::ViewTransitionOld(name.to_owned()));
                }
                if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-new") {
                    return Some(Self::ViewTransitionNew(name.to_owned()));
                }
                None
            }
        }
    }
}

fn functional_pseudo_name<'a>(pseudo_element: &'a str, function_name: &str) -> Option<&'a str> {
    pseudo_element
        .strip_prefix(function_name)?
        .strip_prefix('(')?
        .strip_suffix(')')
        .filter(|name| !name.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ComputedStyleTargetKey {
    Dynamic,
    ChildFrame(DomHandle),
    DetachedIframe(DomHandle),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum DomTokenListKind {
    Class,
    Part,
    Rel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum CollectionKind {
    NodeList,
    HtmlCollection,
    FormControlsCollection,
    OptionsCollection,
    RadioNodeList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LiveCollectionQueryKind {
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
    Labels,
    TableRows,
    TableBodies,
    TableSectionRows,
    TableRowCells,
}

impl LiveCollectionQueryKind {
    pub(super) fn as_dom_host_kind(self) -> &'static str {
        match self {
            Self::ChildNodes => "childNodes",
            Self::Children => "children",
            Self::FormControls => "formControls",
            Self::Options => "options",
            Self::SelectedOptions => "selectedOptions",
            Self::TagName => "tagName",
            Self::TagNameNs => "tagNameNs",
            Self::ClassName => "className",
            Self::Name => "name",
            Self::FormControlsByName => "formControlsByName",
            Self::Forms => "forms",
            Self::Images => "images",
            Self::Scripts => "scripts",
            Self::Links => "links",
            Self::Anchors => "anchors",
            Self::Labels => "labels",
            Self::TableRows => "tableRows",
            Self::TableBodies => "tableBodies",
            Self::TableSectionRows => "tableSectionRows",
            Self::TableRowCells => "tableRowCells",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct LiveCollectionDescriptor {
    pub(super) collection_kind: CollectionKind,
    pub(super) query_kind: LiveCollectionQueryKind,
    pub(super) root: DomHandle,
    pub(super) query: Option<String>,
    pub(super) include_root: bool,
    pub(super) tag_name_html_document: Option<bool>,
    pub(super) resolution_cache: LiveCollectionResolutionCache,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LiveCollectionResolutionCache(
    Rc<RefCell<Option<LiveCollectionResolutionCacheEntry>>>,
);

#[derive(Debug)]
struct LiveCollectionResolutionCacheEntry {
    query_version: u64,
    handles: Rc<[DomHandle]>,
}

impl PartialEq for LiveCollectionDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.collection_kind == other.collection_kind
            && self.query_kind == other.query_kind
            && self.root == other.root
            && self.query == other.query
            && self.include_root == other.include_root
            && self.tag_name_html_document == other.tag_name_html_document
    }
}

impl Eq for LiveCollectionDescriptor {}

impl Hash for LiveCollectionDescriptor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.collection_kind.hash(state);
        self.query_kind.hash(state);
        self.root.hash(state);
        self.query.hash(state);
        self.include_root.hash(state);
        self.tag_name_html_document.hash(state);
    }
}

impl LiveCollectionDescriptor {
    pub(super) fn resolve(&self, host: &JsContextHost) -> Rc<[DomHandle]> {
        let query_version = host.dom_host().query_version();
        if let Some(handles) = self
            .resolution_cache
            .0
            .borrow()
            .as_ref()
            .filter(|entry| entry.query_version == query_version)
            .map(|entry| entry.handles.clone())
        {
            return handles;
        }
        let handles = if self.query_kind == LiveCollectionQueryKind::TagName {
            let query = self.query.as_deref().unwrap_or("*");
            match self.tag_name_html_document {
                // HTML tag-name collections are often resolved repeatedly through
                // bridge wrappers. Cache the DOM-order handle list by query_version
                // so repeated reads do not rescan the document.
                Some(is_html_document) => host
                    .dom_host()
                    .cached_elements_by_tag_name_in_html_document(
                        self.root,
                        query,
                        self.include_root,
                        is_html_document,
                    ),
                None => host
                    .dom_host()
                    .elements_by_tag_name(self.root, query, self.include_root),
            }
        } else if self.query_kind == LiveCollectionQueryKind::Labels {
            control_label_handles(host, self.root)
        } else if self.query_kind == LiveCollectionQueryKind::FormControls {
            form_control_elements(host, self.root)
        } else if self.query_kind == LiveCollectionQueryKind::FormControlsByName {
            let query = self.query.as_deref().unwrap_or_default();
            form_control_elements(host, self.root)
                .into_iter()
                .filter(|handle| {
                    host.dom_host()
                        .node(*handle)
                        .and_then(crate::dom::native::Node::as_element)
                        .is_some_and(|element| element.matches_named_item_key(query))
                })
                .collect()
        } else {
            host.dom_host()
                .resolve_live_collection(
                    self.root,
                    self.query_kind.as_dom_host_kind(),
                    self.query.as_deref(),
                    self.include_root,
                )
                .unwrap_or_default()
        };
        let handles = Rc::<[DomHandle]>::from(handles);
        *self.resolution_cache.0.borrow_mut() = Some(LiveCollectionResolutionCacheEntry {
            query_version,
            handles: handles.clone(),
        });
        handles
    }
}

#[derive(Debug)]
struct LiveCollectionStore {
    next_id: u32,
    descriptors: HashMap<u32, LiveCollectionDescriptor>,
}

impl Default for LiveCollectionStore {
    fn default() -> Self {
        Self {
            next_id: 1,
            descriptors: HashMap::new(),
        }
    }
}

impl LiveCollectionStore {
    fn register(&mut self, descriptor: LiveCollectionDescriptor) -> u32 {
        let collection_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("live collection id overflow");
        let replaced = self.descriptors.insert(collection_id, descriptor);
        assert!(
            replaced.is_none(),
            "live collection ids must never be reused"
        );
        collection_id
    }

    fn descriptor(&self, collection_id: u32) -> Option<&LiveCollectionDescriptor> {
        self.descriptors.get(&collection_id)
    }
}

#[derive(Debug)]
struct StaticHandleCollectionStore {
    next_id: u32,
    handles: HashMap<u32, Vec<DomHandle>>,
}

impl Default for StaticHandleCollectionStore {
    fn default() -> Self {
        Self {
            next_id: 1,
            handles: HashMap::new(),
        }
    }
}

impl StaticHandleCollectionStore {
    fn register(&mut self, handles: Vec<DomHandle>) -> u32 {
        let collection_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("static handle collection id overflow");
        let replaced = self.handles.insert(collection_id, handles);
        assert!(
            replaced.is_none(),
            "static handle collection ids must never be reused"
        );
        collection_id
    }

    fn len(&self, collection_id: u32) -> Option<usize> {
        self.handles.get(&collection_id).map(Vec::len)
    }

    fn handle_at(&self, collection_id: u32, index: usize) -> Option<DomHandle> {
        self.handles
            .get(&collection_id)
            .and_then(|handles| handles.get(index))
            .copied()
    }
}

#[derive(Debug, Default)]
struct BridgeContextWrapperCache {
    wrappers: HashMap<ReflectorId, BridgeCachedWrapper>,
    live_collection_wrappers: HashMap<LiveCollectionDescriptor, BridgeCachedWrapper>,
}

#[derive(Debug)]
struct SharedDefaultWorldWrapperCache;

#[derive(Debug)]
struct BridgeCachedWrapper {
    wrapper: BridgeWrapperHandle,
    owner_realm: Option<RuntimeObservableContextToken>,
}

#[derive(Debug)]
enum BridgeWrapperHandle {
    Strong(v8::Global<v8::Object>),
    Weak(v8::Weak<v8::Object>),
}

impl BridgeWrapperHandle {
    fn new(scope: &mut v8::PinScope<'_, '_>, wrapper: v8::Local<'_, v8::Object>) -> Self {
        if crate::util::page_context_is_detached_document(scope.get_current_context()) {
            Self::Weak(v8::Weak::new(scope, wrapper))
        } else {
            Self::Strong(v8::Global::new(scope, wrapper))
        }
    }

    fn to_local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
        match self {
            Self::Strong(wrapper) => Some(v8::Local::new(scope, wrapper)),
            Self::Weak(wrapper) => wrapper.to_local(scope),
        }
    }

    fn weaken(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        let Self::Strong(wrapper) = self else {
            return;
        };
        let wrapper = v8::Local::new(scope, &*wrapper);
        *self = Self::Weak(v8::Weak::new(scope, wrapper));
    }

    #[cfg(test)]
    fn is_strong(&self) -> bool {
        matches!(self, Self::Strong(_))
    }
}

impl BridgeCachedWrapper {
    fn new(scope: &mut v8::PinScope<'_, '_>, wrapper: v8::Local<'_, v8::Object>) -> Self {
        Self {
            wrapper: BridgeWrapperHandle::new(scope, wrapper),
            // A child node can be first exposed through its parent before the
            // real child Context exists. Treat the ambient realm as a
            // provisional owner until the authoritative child realm consumes
            // the wrapper.
            owner_realm: scope
                .get_current_context()
                .get_slot::<RuntimeObservableContextToken>()
                .as_deref()
                .copied(),
        }
    }
}

#[derive(Debug)]
struct BridgeContextWindowWrapper {
    wrapper: RefCell<BridgeWrapperHandle>,
}

fn context_window_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    scope
        .get_current_context()
        .get_slot::<BridgeContextWindowWrapper>()
        .and_then(|entry| entry.wrapper.borrow().to_local(scope))
}

fn set_context_window_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
) {
    let _ = scope
        .get_current_context()
        .set_slot(Rc::new(BridgeContextWindowWrapper {
            wrapper: RefCell::new(BridgeWrapperHandle::new(scope, wrapper)),
        }));
}

fn context_wrapper_cache(
    scope: &mut v8::PinScope<'_, '_>,
) -> Rc<RefCell<BridgeContextWrapperCache>> {
    let context = scope.get_current_context();
    if let Some(cache) = context.get_slot::<RefCell<BridgeContextWrapperCache>>() {
        return cache;
    }
    let cache = Rc::new(RefCell::new(BridgeContextWrapperCache::default()));
    let _ = context.set_slot(cache.clone());
    cache
}

#[cfg(test)]
pub(crate) struct BridgeContextWrapperCacheRetainForTest {
    cache: Rc<RefCell<BridgeContextWrapperCache>>,
}

#[cfg(test)]
impl BridgeContextWrapperCacheRetainForTest {
    pub(crate) fn wrapper_entry_count(&self) -> usize {
        self.cache.borrow().wrappers.len()
    }

    pub(crate) fn strong_wrapper_entry_count(&self) -> usize {
        self.cache
            .borrow()
            .wrappers
            .values()
            .filter(|entry| entry.wrapper.is_strong())
            .count()
    }

    pub(crate) fn wrapper_entry_count_for_realm(
        &self,
        realm_token: RuntimeObservableContextToken,
    ) -> usize {
        self.cache
            .borrow()
            .wrappers
            .values()
            .filter(|entry| entry.owner_realm == Some(realm_token))
            .count()
    }

    pub(crate) fn strong_wrapper_entry_count_for_realm(
        &self,
        realm_token: RuntimeObservableContextToken,
    ) -> usize {
        self.cache
            .borrow()
            .wrappers
            .values()
            .filter(|entry| entry.owner_realm == Some(realm_token) && entry.wrapper.is_strong())
            .count()
    }

    pub(crate) fn strong_live_collection_entry_count_for_realm(
        &self,
        realm_token: RuntimeObservableContextToken,
    ) -> usize {
        self.cache
            .borrow()
            .live_collection_wrappers
            .values()
            .filter(|entry| entry.owner_realm == Some(realm_token) && entry.wrapper.is_strong())
            .count()
    }

    pub(crate) fn wrapper_owner_realm_for_object(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        object: v8::Local<'_, v8::Object>,
    ) -> Option<RuntimeObservableContextToken> {
        let cache = self.cache.borrow();
        cache
            .wrappers
            .values()
            .chain(cache.live_collection_wrappers.values())
            .find_map(|entry| {
                entry
                    .wrapper
                    .to_local(scope)
                    .is_some_and(|wrapper| wrapper.strict_equals(object.into()))
                    .then_some(entry.owner_realm)
            })
            .flatten()
    }
}

#[cfg(test)]
pub(crate) fn retain_context_wrapper_cache_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> BridgeContextWrapperCacheRetainForTest {
    BridgeContextWrapperCacheRetainForTest {
        cache: context_wrapper_cache(scope),
    }
}

pub(crate) fn clear_context_embedder_state_for_teardown(
    scope: &mut v8::PinScope<'_, '_>,
    include_shared_default_world: bool,
) {
    let context = scope.get_current_context();
    let realm_token = context
        .get_slot::<RuntimeObservableContextToken>()
        .as_deref()
        .copied();
    if let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(context)
        && let Some(realm_token) = realm_token
    {
        let host = unsafe { &mut *host_ptr };
        host.retire_pending_network_body_v8_handles_for_context_token(realm_token);
        host.retire_resource_timing_buffers_for_context_token(realm_token);
        host.retire_event_callbacks_for_context_token(realm_token);
        host.detach_abort_signals_for_context_teardown(scope, realm_token);
    }
    crate::util::detach_document_page_context(context);
    crate::context_bootstrap::clear_agent_microtask_checkpoint_tasks_for_context_teardown(
        scope, context,
    );
    crate::context_bootstrap::clear_indexed_db_runtime_state_table_for_context_teardown(context);
    let uses_shared_default_world_cache = context
        .get_slot::<SharedDefaultWorldWrapperCache>()
        .is_some();
    let weaken_all_cache_entries = include_shared_default_world || !uses_shared_default_world_cache;
    if let Some(cache) = context.get_slot::<RefCell<BridgeContextWrapperCache>>() {
        let mut cache_state = cache.borrow_mut();
        for entry in cache_state.wrappers.values_mut() {
            if weaken_all_cache_entries || entry.owner_realm == realm_token {
                entry.wrapper.weaken(scope);
            }
        }
        for entry in cache_state.live_collection_wrappers.values_mut() {
            if weaken_all_cache_entries || entry.owner_realm == realm_token {
                entry.wrapper.weaken(scope);
            }
        }
        drop(cache_state);
        crate::util::retain_context_v8_handle_state_for_safe_release(context, cache);
    }

    if let Some(window_wrapper) = context.get_slot::<BridgeContextWindowWrapper>() {
        window_wrapper.wrapper.borrow_mut().weaken(scope);
        crate::util::retain_context_v8_handle_state_for_safe_release(context, window_wrapper);
    }
    if let Some(registry) = crate::context_bootstrap::exposed_interfaces::weaken_intrinsic_interface_registry_for_context_teardown(scope) {
        crate::util::retain_context_v8_handle_state_for_safe_release(context, registry);
    }
    if let Some(prototype) =
        crate::context_bootstrap::weaken_dom_exception_prototype_for_context_teardown(scope)
    {
        crate::util::retain_context_v8_handle_state_for_safe_release(context, prototype);
    }
}

#[derive(Debug, Default)]
pub(super) struct BridgeIdentityStore {
    reflectors: ReflectorRegistry<BridgeHandle>,
    live_collections: LiveCollectionStore,
    static_handle_collections: StaticHandleCollectionStore,
    default_world_wrapper_cache: Rc<RefCell<BridgeContextWrapperCache>>,
}

impl BridgeIdentityStore {
    pub(super) fn install_default_world_wrapper_cache(&self, context: v8::Local<'_, v8::Context>) {
        let _ = context.set_slot(self.default_world_wrapper_cache.clone());
        let _ = context.set_slot(Rc::new(SharedDefaultWorldWrapperCache));
    }

    fn reflector_id_for(&mut self, handle: BridgeHandle) -> ReflectorId {
        self.reflectors.root(DomPtr::new(handle)).reflector_id()
    }

    pub(super) fn reflector_id(&mut self, handle: BridgeHandle) -> ReflectorId {
        self.reflector_id_for(handle)
    }

    pub(super) fn existing_reflector_id(&self, handle: BridgeHandle) -> Option<ReflectorId> {
        self.reflectors
            .existing(handle)
            .map(|reflector| reflector.id())
    }

    pub(super) fn bridge_handle(&self, reflector_id: ReflectorId) -> Option<BridgeHandle> {
        self.reflectors.key_for_id(reflector_id)
    }

    pub(super) fn cached_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        reflector_id: ReflectorId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if matches!(self.bridge_handle(reflector_id), Some(BridgeHandle::Window)) {
            return context_window_wrapper(scope);
        }
        context_wrapper_cache(scope)
            .borrow()
            .wrappers
            .get(&reflector_id)
            .and_then(|entry| entry.wrapper.to_local(scope))
    }

    pub(super) fn cache_wrapper(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        reflector_id: ReflectorId,
        wrapper: v8::Local<'_, v8::Object>,
    ) {
        if matches!(self.bridge_handle(reflector_id), Some(BridgeHandle::Window)) {
            set_context_window_wrapper(scope, wrapper);
            return;
        }
        context_wrapper_cache(scope)
            .borrow_mut()
            .wrappers
            .insert(reflector_id, BridgeCachedWrapper::new(scope, wrapper));
    }

    fn authoritative_child_realm_token_for_dom_handle(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        node_handle: DomHandle,
    ) -> Option<(DomHandle, RuntimeObservableContextToken)> {
        // Isolated worlds own a private cache whose ambient realm token is
        // already exact and whose entries are all weakened together.
        scope
            .get_current_context()
            .get_slot::<SharedDefaultWorldWrapperCache>()
            .as_deref()?;
        let host = unsafe { &*host_ptr };
        let document_handle = host.dom_host().owner_document_handle(node_handle)?;
        let child_handle = host.child_browsing_context_host_for_document_handle(document_handle)?;
        if host.child_browsing_context_document_handle(child_handle) != Some(document_handle) {
            return None;
        }
        let identity = host.current_registered_window_execution_context_identity(
            OwnerDispatchScope::Child(child_handle),
        )?;
        Some((child_handle, identity.realm_token()))
    }

    pub(super) fn bind_cached_wrapper_owner_realm(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: &BridgeHandle,
        reflector_id: ReflectorId,
    ) {
        let Some(node_handle) = handle.dom_handle() else {
            return;
        };
        let Some((_, realm_token)) =
            Self::authoritative_child_realm_token_for_dom_handle(scope, host_ptr, node_handle)
        else {
            return;
        };
        let cache = context_wrapper_cache(scope);
        let mut cache = cache.borrow_mut();
        let Some(entry) = cache.wrappers.get_mut(&reflector_id) else {
            return;
        };
        entry.owner_realm = Some(realm_token);
    }

    pub(super) fn register_live_collection(&mut self, descriptor: LiveCollectionDescriptor) -> u32 {
        self.live_collections.register(descriptor)
    }

    pub(super) fn live_collection_descriptor(
        &self,
        collection_id: u32,
    ) -> Option<&LiveCollectionDescriptor> {
        self.live_collections.descriptor(collection_id)
    }

    pub(super) fn cached_live_collection_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        descriptor: &LiveCollectionDescriptor,
    ) -> Option<v8::Local<'s, v8::Object>> {
        context_wrapper_cache(scope)
            .borrow()
            .live_collection_wrappers
            .get(descriptor)
            .and_then(|entry| entry.wrapper.to_local(scope))
    }

    pub(super) fn cache_live_collection_wrapper(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        descriptor: LiveCollectionDescriptor,
        wrapper: v8::Local<'_, v8::Object>,
    ) {
        context_wrapper_cache(scope)
            .borrow_mut()
            .live_collection_wrappers
            .insert(descriptor, BridgeCachedWrapper::new(scope, wrapper));
    }

    pub(super) fn bind_cached_live_collection_owner_realm(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        descriptor: &LiveCollectionDescriptor,
    ) -> Option<DomHandle> {
        let (child_handle, realm_token) =
            Self::authoritative_child_realm_token_for_dom_handle(scope, host_ptr, descriptor.root)?;
        let cache = context_wrapper_cache(scope);
        let mut cache = cache.borrow_mut();
        let entry = cache.live_collection_wrappers.get_mut(descriptor)?;
        entry.owner_realm = Some(realm_token);
        Some(child_handle)
    }

    pub(super) fn register_static_handle_collection(&mut self, handles: Vec<DomHandle>) -> u32 {
        self.static_handle_collections.register(handles)
    }

    pub(super) fn static_handle_collection_len(&self, collection_id: u32) -> Option<usize> {
        self.static_handle_collections.len(collection_id)
    }

    pub(super) fn static_handle_collection_handle_at(
        &self,
        collection_id: u32,
        index: usize,
    ) -> Option<DomHandle> {
        self.static_handle_collections
            .handle_at(collection_id, index)
    }
}
