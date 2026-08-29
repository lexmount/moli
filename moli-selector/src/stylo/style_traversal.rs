use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    ptr::NonNull,
    vec,
};

use app_units::Au;
use dom::ElementState;
use euclid::default::Size2D;
use selectors::{
    Element as SelectorsElement, OpaqueElement,
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    bloom::BloomFilter,
    matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode},
};
use style::{
    Atom, LocalName, Namespace,
    context::{QuirksMode, SharedStyleContext},
    data::{ElementDataMut, ElementDataRef, ElementDataWrapper},
    dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot},
    invalidation::{element::restyle_hints::RestyleHint, stylesheets::StylesheetInvalidationSet},
    properties::{PropertyDeclarationBlock, longhands::display::computed_value::T as Display},
    selector_parser::{AttrValue as SelectorAttrValue, Lang, PseudoElement, SelectorImpl},
    servo_arc::{Arc, ArcBorrow},
    shared_lock::{Locked, SharedRwLock},
    stylist::CascadeData,
    values::AtomIdent,
};

use crate::{
    dom::{
        NodeId,
        native::{DomHost, Element, Node},
    },
    stylo::{atoms::QueryAtomCache, query::QueryElement},
};

/// Document-bucketed Stylo adapter for real style traversal.
///
/// Query APIs use the same DOM wrappers with no style data attached. Style
/// calculation must instead attach Stylo's per-element `ElementDataWrapper` to
/// the lifetime of the owning Moli document, which is what this adapter
/// owns.
#[derive(Debug)]
pub struct StyloDomStyleAdapter {
    state: Box<StyleDomState>,
}

/// Document-bucketed storage for Stylo `ElementDataWrapper`.
///
/// Servo stores this data on DOM elements. Moli's extracted DOM is kept
/// layout-free, so the style engine owns a side table keyed by DOM node id.
/// Returned Stylo borrows rely on the same unsafe contract as `TElement`: callers
/// must not clear an element's data while Stylo holds a borrow to it.
#[derive(Default)]
pub struct StyloElementDataStore {
    handle_documents: RefCell<HashMap<NodeId, NodeId>>,
    documents: RefCell<HashMap<NodeId, StyloElementDocumentDataStore>>,
    #[cfg(test)]
    targeted_owner_membership_checks: Cell<usize>,
    #[cfg(test)]
    fallback_document_scans: Cell<usize>,
}

#[derive(Default)]
struct StyloElementDocumentDataStore {
    data: HashMap<NodeId, Box<ElementDataWrapper>>,
    inline_styles: HashMap<NodeId, Arc<Locked<PropertyDeclarationBlock>>>,
    selector_flags: HashMap<NodeId, ElementSelectorFlags>,
    /// Stylo's equivalent of Blink's `ChildNeedsStyleInvalidation` breadcrumb.
    ///
    /// Stylesheet invalidation already calls `TElement::set_dirty_descendants`
    /// while unwinding from a matched descendant. Keeping that bit in the
    /// document side table lets Moli follow only dirty branches later instead
    /// of rediscovering them by scanning every published `ElementData` entry.
    dirty_descendants: HashSet<NodeId>,
}

#[derive(Default)]
struct ShadowCascadeDataStore {
    root_documents: RefCell<HashMap<NodeId, NodeId>>,
    documents: RefCell<HashMap<NodeId, HashMap<NodeId, Arc<CascadeData>>>>,
}

pub type StyloDocument<'a> = StyleDocument<'a>;
pub type StyloDomHostBinding<'a> = StyleDomHostBinding<'a>;
pub type StyloElement<'a> = StyleElement<'a>;
pub type StyloNode<'a> = StyleNode<'a>;
pub type StyloShadowRoot<'a> = StyleShadowRoot<'a>;

#[derive(Debug)]
struct StyleDomState {
    host: Cell<*const DomHost>,
    shared_lock: SharedRwLock,
    element_data: StyloElementDataStore,
    atom_cache: QueryAtomCache,
    node_data: RefCell<HashMap<NodeId, Box<StyleNodeData>>>,
    slotted_node_data: RefCell<HashMap<NodeId, Box<[StyleNode<'static>]>>>,
    shadow_cascade_data: ShadowCascadeDataStore,
    snapshot_handles: RefCell<HashSet<NodeId>>,
}

#[derive(Clone, Copy)]
pub struct StyleNode<'a>(NonNull<StyleNodeData>, PhantomData<&'a ()>);

#[derive(Clone, Copy)]
pub struct StyleElement<'a>(NonNull<StyleNodeData>, PhantomData<&'a ()>);

#[derive(Clone, Copy)]
pub struct StyleDocument<'a>(NonNull<StyleNodeData>, PhantomData<&'a ()>);

#[derive(Clone, Copy)]
pub struct StyleShadowRoot<'a>(NonNull<StyleNodeData>, PhantomData<&'a ()>);

#[derive(Debug)]
struct StyleNodeData {
    state: *const StyleDomState,
    handle: NodeId,
    document: NodeId,
}

pub struct StyleDomHostBinding<'a> {
    state: &'a StyleDomState,
}

struct SnapshotHandlesReset<'a> {
    state: &'a StyleDomState,
    previous: Option<HashSet<NodeId>>,
}

impl StyloDomStyleAdapter {
    pub fn new() -> Self {
        Self {
            state: Box::new(StyleDomState {
                host: Cell::new(std::ptr::null()),
                shared_lock: SharedRwLock::new(),
                element_data: StyloElementDataStore::default(),
                atom_cache: QueryAtomCache::default(),
                node_data: RefCell::default(),
                slotted_node_data: RefCell::default(),
                shadow_cascade_data: ShadowCascadeDataStore::default(),
                snapshot_handles: RefCell::default(),
            }),
        }
    }

    pub fn shared_lock(&self) -> &SharedRwLock {
        &self.state.shared_lock
    }

    pub fn with_bound_host<R>(
        &self,
        host: &DomHost,
        f: impl for<'scope> FnOnce(&'scope StyleDomHostBinding<'scope>) -> R,
    ) -> R {
        self.bind_host(host);
        let binding = StyleDomHostBinding { state: &self.state };
        f(&binding)
    }

    // Adapter-level state operations are used by invalidation and cache
    // preparation paths that do not produce lifetime-bound Stylo DOM wrappers.
    pub fn clear_element_data(&self, handle: NodeId) {
        self.state.clear_element_data(handle);
    }

    pub fn clear_element_selector_flags(&self, handle: NodeId) {
        self.state.clear_element_selector_flags(handle);
    }

    /// Retains the element's last published computed values while marking
    /// them unavailable for the next style observation.
    pub fn mark_element_style_dirty(&self, handle: NodeId) -> bool {
        self.state.mark_element_style_dirty(handle)
    }

    pub fn clear_element_data_for_document(&self, document: NodeId) {
        self.state.clear_element_data_for_document(document);
    }

    /// Returns whether the canonical Stylo side table still contains any
    /// resolved element style for this Document.
    pub fn has_element_styles_for_document(&self, document: NodeId) -> bool {
        self.state.has_element_styles_for_document(document)
    }

    pub fn set_inline_style_attribute(
        &self,
        handle: NodeId,
        declarations: Arc<Locked<PropertyDeclarationBlock>>,
    ) {
        self.state.set_inline_style_attribute(handle, declarations);
    }

    pub fn clear_inline_style_attribute(&self, handle: NodeId) {
        self.state.clear_inline_style_attribute(handle);
    }

    pub fn clear_inline_style_attributes_for_document(&self, document: NodeId) {
        self.state
            .clear_inline_style_attributes_for_document(document);
    }

    pub fn process_stylesheet_invalidations(
        &self,
        host: &DomHost,
        roots: &[NodeId],
        invalidations: &StylesheetInvalidationSet,
    ) -> Vec<NodeId> {
        self.with_bound_host(host, |binding| {
            let mut changed_roots = Vec::new();
            for root in roots.iter().filter_map(|root| binding.element(host, *root)) {
                // Every root belongs to an independent Document/ShadowRoot
                // scope and must be processed even after an earlier match.
                if invalidations.process_style(root, None) {
                    changed_roots.push(root);
                }
            }
            collect_dirty_style_roots(changed_roots)
        })
    }

    /// Collects the top-most dirty elements by following Stylo's retained
    /// dirty-descendant breadcrumbs from the supplied scope roots.
    ///
    /// This is deliberately proportional to dirty branches. It replaces the
    /// old document-wide scan over every published `ElementData` entry.
    pub fn collect_dirty_style_roots(&self, host: &DomHost, roots: &[NodeId]) -> Vec<NodeId> {
        self.with_bound_host(host, |binding| {
            collect_dirty_style_roots(roots.iter().filter_map(|root| binding.element(host, *root)))
        })
    }

    pub fn element_selector_flag_handles_for_document(&self, document: NodeId) -> Vec<NodeId> {
        self.state
            .element_selector_flag_handles_for_document(document)
    }

    pub fn clear_shadow_cascade_data_for_roots(&self, roots: impl IntoIterator<Item = NodeId>) {
        self.state.clear_shadow_cascade_data_for_roots(roots);
    }

    pub fn clear_shadow_cascade_data_for_document(&self, document: NodeId) {
        self.state.clear_shadow_cascade_data_for_document(document);
    }

    pub fn shadow_cascade_roots_for_document(&self, document: NodeId) -> Vec<NodeId> {
        self.state.shadow_cascade_roots_for_document(document)
    }

    pub fn set_shadow_cascade_data(&self, root: NodeId, cascade_data: Arc<CascadeData>) {
        self.state.set_shadow_cascade_data(root, cascade_data);
    }

    #[doc(hidden)]
    pub fn set_shadow_cascade_data_for_document_for_test(
        &self,
        document: NodeId,
        root: NodeId,
        cascade_data: Arc<CascadeData>,
    ) {
        self.state
            .set_shadow_cascade_data_for_document_for_test(document, root, cascade_data);
    }

    #[doc(hidden)]
    pub fn has_shadow_cascade_data_for_test(&self, root: NodeId) -> bool {
        self.state.has_shadow_cascade_data(root)
    }

    pub fn has_element_data(&self, handle: NodeId) -> bool {
        self.state.has_element_data(handle)
    }

    #[doc(hidden)]
    pub fn shadow_cascade_document_count_for_test(&self) -> usize {
        self.state.shadow_cascade_document_count_for_test()
    }

    fn bind_host(&self, host: &DomHost) {
        self.state.host.set(host as *const DomHost);
        self.state.clear_node_data();
    }
}

fn collect_dirty_style_roots<'a>(roots: impl IntoIterator<Item = StyleElement<'a>>) -> Vec<NodeId> {
    fn collect_from_element(
        element: StyleElement<'_>,
        roots: &mut Vec<NodeId>,
        seen: &mut HashSet<NodeId>,
        covered_by_dirty_ancestor: bool,
    ) {
        let handle = element.handle();
        if !seen.insert(handle) {
            return;
        }

        let self_is_dirty = element
            .borrow_data()
            .is_some_and(|data| !data.hint.is_empty());
        let descendants_are_dirty = element.has_dirty_descendants();
        if self_is_dirty {
            // Moli resolves demanded elements one at a time. Treating a dirty
            // element as a lazy subtree root conservatively carries inherited
            // changes to any descendant resolved after it.
            if !covered_by_dirty_ancestor {
                roots.push(handle);
            }
        }

        if !descendants_are_dirty {
            return;
        }
        for child in element.traversal_children() {
            if let Some(child) = child.as_element() {
                collect_from_element(
                    child,
                    roots,
                    seen,
                    covered_by_dirty_ancestor || self_is_dirty,
                );
            }
        }
        unsafe { element.unset_dirty_descendants() };
    }

    let mut dirty_roots = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        collect_from_element(root, &mut dirty_roots, &mut seen, false);
    }
    dirty_roots
}

impl<'binding> StyleDomHostBinding<'binding> {
    pub fn shared_lock(&self) -> &SharedRwLock {
        &self.state.shared_lock
    }

    pub fn document<'scope>(&'scope self, host: &DomHost) -> StyleDocument<'scope> {
        self.ensure_bound_host(host);
        let document = host.document_handle();
        StyleDocument(self.state.node_data(document, document), PhantomData)
    }

    pub fn node<'scope>(&'scope self, host: &DomHost, handle: NodeId) -> Option<StyleNode<'scope>> {
        self.ensure_bound_host(host);
        let document = host.owner_document_handle(handle)?;
        host.node(handle)
            .map(|_| StyleNode(self.state.node_data(handle, document), PhantomData))
    }

    pub fn element<'scope>(
        &'scope self,
        host: &DomHost,
        handle: NodeId,
    ) -> Option<StyleElement<'scope>> {
        self.ensure_bound_host(host);
        let document = host.owner_document_handle(handle)?;
        host.node(handle)?.as_element()?;
        Some(StyleElement(
            self.state.node_data(handle, document),
            PhantomData,
        ))
    }

    pub fn set_shadow_cascade_data(&self, root: NodeId, cascade_data: Arc<CascadeData>) {
        self.state.set_shadow_cascade_data(root, cascade_data);
    }

    pub fn has_element_data(&self, handle: NodeId) -> bool {
        self.state.has_element_data(handle)
    }

    pub fn element_data_count(&self) -> usize {
        self.state.element_data_count()
    }

    #[doc(hidden)]
    pub fn element_side_table_document_count_for_test(&self) -> usize {
        self.state.element_side_table_document_count_for_test()
    }

    pub(in crate::stylo) fn with_snapshot_handles<R>(
        &self,
        handles: impl IntoIterator<Item = NodeId>,
        f: impl FnOnce() -> R,
    ) -> R {
        self.state.with_snapshot_handles(handles, f)
    }

    fn ensure_bound_host(&self, host: &DomHost) {
        assert_eq!(
            self.state.host.get(),
            host as *const DomHost,
            "style adapter host access must use the currently bound DomHost"
        );
    }
}

impl Drop for StyleDomHostBinding<'_> {
    fn drop(&mut self) {
        self.state.clear_node_data();
        self.state.clear_shadow_cascade_data_after_binding();
        self.state.host.set(std::ptr::null());
    }
}

impl StyloElementDataStore {
    pub(super) fn note_owner_document_for_host(
        &self,
        host: &DomHost,
        handle: NodeId,
    ) -> Option<NodeId> {
        let document = host.owner_document_handle(handle)?;
        self.note_owner_document(handle, document);
        Some(document)
    }

    fn note_owner_document(&self, handle: NodeId, document: NodeId) {
        let previous = self.handle_documents.borrow_mut().insert(handle, document);
        let Some(previous) = previous.filter(|previous| *previous != document) else {
            return;
        };

        let mut documents = self.documents.borrow_mut();
        let mut moved_data = None;
        let mut moved_inline_style = None;
        let mut moved_selector_flags = None;
        let mut moved_dirty_descendants = false;
        let mut previous_is_empty = false;
        if let Some(previous_bucket) = documents.get_mut(&previous) {
            moved_data = previous_bucket.data.remove(&handle);
            moved_inline_style = previous_bucket.inline_styles.remove(&handle);
            moved_selector_flags = previous_bucket.selector_flags.remove(&handle);
            moved_dirty_descendants = previous_bucket.dirty_descendants.remove(&handle);
            previous_is_empty = previous_bucket.is_empty();
        }
        if previous_is_empty {
            documents.remove(&previous);
        }

        if moved_data.is_none()
            && moved_inline_style.is_none()
            && moved_selector_flags.is_none()
            && !moved_dirty_descendants
        {
            return;
        }

        let bucket = documents.entry(document).or_default();
        if let Some(data) = moved_data {
            bucket.data.insert(handle, data);
        }
        if let Some(inline_style) = moved_inline_style {
            bucket.inline_styles.insert(handle, inline_style);
        }
        if let Some(selector_flags) = moved_selector_flags {
            bucket.selector_flags.insert(handle, selector_flags);
        }
        if moved_dirty_descendants {
            bucket.dirty_descendants.insert(handle);
        }
    }

    pub(super) fn ensure_for_host(&self, host: &DomHost, handle: NodeId) -> ElementDataMut<'_> {
        let document = host
            .owner_document_handle(handle)
            .expect("style element data requires an owner document");
        self.ensure_for_document(document, handle)
    }

    pub(super) fn ensure_for_document(
        &self,
        document: NodeId,
        handle: NodeId,
    ) -> ElementDataMut<'_> {
        self.note_owner_document(handle, document);
        let data = {
            let mut documents = self.documents.borrow_mut();
            let entry = documents
                .entry(document)
                .or_default()
                .data
                .entry(handle)
                .or_insert_with(|| Box::new(ElementDataWrapper::default()));
            &**entry as *const ElementDataWrapper
        };
        // SAFETY: ElementDataWrapper is heap-allocated behind a Box, so its
        // address remains stable across HashMap rehashes. The TElement data API
        // is already unsafe: callers must not clear this handle's data while a
        // Stylo borrow is alive.
        unsafe { (&*data).borrow_mut() }
    }

    pub(super) fn clear(&self, handle: NodeId) {
        self.remove_from_documents(handle, |bucket| {
            bucket.data.remove(&handle);
        });
    }

    pub(super) fn clear_for_host(&self, host: &DomHost, handle: NodeId) {
        self.note_owner_document_for_host(host, handle);
        self.clear(handle);
    }

    pub(super) fn clear_for_document(&self, document: NodeId, handle: NodeId) {
        self.remove_from_document(document, handle, |bucket| {
            bucket.data.remove(&handle);
        });
    }

    pub(super) fn clear_selector_flags(&self, handle: NodeId) {
        self.remove_from_documents(handle, |bucket| {
            bucket.selector_flags.remove(&handle);
        });
    }

    #[cfg(test)]
    fn clear_style_values(&self, handle: NodeId) {
        self.remove_from_documents(handle, |bucket| {
            bucket.data.remove(&handle);
            bucket.inline_styles.remove(&handle);
        });
    }

    fn mark_style_dirty(&self, handle: NodeId) -> bool {
        let data = {
            let documents = self.documents.borrow();
            self.document_for_handle(handle)
                .and_then(|document| documents.get(&document))
                .and_then(|bucket| bucket.data.get(&handle))
                .map(|data| &**data as *const ElementDataWrapper)
                .or_else(|| {
                    documents
                        .values()
                        .find_map(|bucket| bucket.data.get(&handle))
                        .map(|data| &**data as *const ElementDataWrapper)
                })
        };
        let Some(data) = data else {
            return false;
        };
        // SAFETY: ElementDataWrapper is heap-allocated behind a Box and the
        // adapter's side-table contract forbids removing this entry while the
        // returned Stylo borrow is alive.
        unsafe { &*data }
            .borrow_mut()
            .hint
            .insert(RestyleHint::RESTYLE_SELF | RestyleHint::RESTYLE_PSEUDOS);
        true
    }

    fn clear_data_for_document(&self, document: NodeId) {
        let orphaned_handles = {
            let mut documents = self.documents.borrow_mut();
            let Some(bucket) = documents.get_mut(&document) else {
                return;
            };
            let affected_handles = bucket
                .data
                .keys()
                .chain(bucket.selector_flags.keys())
                .chain(bucket.dirty_descendants.iter())
                .copied()
                .collect::<HashSet<_>>();
            bucket.data.clear();
            bucket.selector_flags.clear();
            bucket.dirty_descendants.clear();
            let orphaned_handles = affected_handles
                .into_iter()
                .filter(|handle| {
                    !self.bucket_contains_handle_for_owner_maintenance(bucket, *handle)
                })
                .collect::<Vec<_>>();
            if bucket.is_empty() {
                documents.remove(&document);
            }
            orphaned_handles
        };
        self.remove_owner_mappings_for_document(document, orphaned_handles);
    }

    pub(super) fn has(&self, handle: NodeId) -> bool {
        let documents = self.documents.borrow();
        self.document_for_handle(handle)
            .and_then(|document| documents.get(&document))
            .is_some_and(|bucket| bucket.data.contains_key(&handle))
            || documents
                .values()
                .any(|bucket| bucket.data.contains_key(&handle))
    }

    pub(super) fn has_for_host(&self, host: &DomHost, handle: NodeId) -> bool {
        self.note_owner_document_for_host(host, handle);
        self.has(handle)
    }

    pub(super) fn has_for_document(&self, document: NodeId, handle: NodeId) -> bool {
        self.documents
            .borrow()
            .get(&document)
            .is_some_and(|bucket| bucket.data.contains_key(&handle))
    }

    fn has_style_values_for_document(&self, document: NodeId) -> bool {
        self.documents
            .borrow()
            .get(&document)
            .is_some_and(|bucket| bucket.data.values().any(|data| data.borrow().has_styles()))
    }

    fn len(&self) -> usize {
        self.documents
            .borrow()
            .values()
            .map(|bucket| bucket.data.len())
            .sum()
    }

    fn document_count_for_test(&self) -> usize {
        self.documents
            .borrow()
            .values()
            .filter(|bucket| !bucket.is_empty())
            .count()
    }

    pub(super) fn borrow(&self, handle: NodeId) -> Option<ElementDataRef<'_>> {
        let data = {
            let documents = self.documents.borrow();
            self.find_data(&documents, handle)
                .map(|data| data as *const ElementDataWrapper)
        }?;
        // SAFETY: See ensure(); returning a borrow after releasing the map
        // borrow relies on the TElement caller not clearing this element data
        // while Stylo holds the returned ElementDataRef.
        Some(unsafe { (&*data).borrow() })
    }

    pub(super) fn borrow_for_host(
        &self,
        host: &DomHost,
        handle: NodeId,
    ) -> Option<ElementDataRef<'_>> {
        self.note_owner_document_for_host(host, handle);
        self.borrow(handle)
    }

    pub(super) fn borrow_for_document(
        &self,
        document: NodeId,
        handle: NodeId,
    ) -> Option<ElementDataRef<'_>> {
        let data = {
            let documents = self.documents.borrow();
            documents
                .get(&document)
                .and_then(|bucket| bucket.data.get(&handle))
                .map(|data| &**data as *const ElementDataWrapper)
        }?;
        // SAFETY: See ensure_for_document(); returning a borrow after releasing
        // the map borrow relies on the TElement caller not clearing this element
        // data while Stylo holds the returned ElementDataRef.
        Some(unsafe { (&*data).borrow() })
    }

    pub(super) fn mutate(&self, handle: NodeId) -> Option<ElementDataMut<'_>> {
        let data = {
            let documents = self.documents.borrow();
            self.find_data(&documents, handle)
                .map(|data| data as *const ElementDataWrapper)
        }?;
        // SAFETY: See ensure(); returning a mutable borrow after releasing the
        // map borrow relies on the TElement caller not clearing this element
        // data while Stylo holds the returned ElementDataMut.
        Some(unsafe { (&*data).borrow_mut() })
    }

    pub(super) fn mutate_for_host(
        &self,
        host: &DomHost,
        handle: NodeId,
    ) -> Option<ElementDataMut<'_>> {
        self.note_owner_document_for_host(host, handle);
        self.mutate(handle)
    }

    pub(super) fn mutate_for_document(
        &self,
        document: NodeId,
        handle: NodeId,
    ) -> Option<ElementDataMut<'_>> {
        let data = {
            let documents = self.documents.borrow();
            documents
                .get(&document)
                .and_then(|bucket| bucket.data.get(&handle))
                .map(|data| &**data as *const ElementDataWrapper)
        }?;
        // SAFETY: See ensure_for_document(); returning a mutable borrow after
        // releasing the map borrow relies on the TElement caller not clearing
        // this element data while Stylo holds the returned ElementDataMut.
        Some(unsafe { (&*data).borrow_mut() })
    }

    fn set_inline_style_for_host(
        &self,
        host: &DomHost,
        handle: NodeId,
        declarations: Arc<Locked<PropertyDeclarationBlock>>,
    ) {
        let document = self
            .note_owner_document_for_host(host, handle)
            .expect("inline style side table requires an owner document");
        self.documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .inline_styles
            .insert(handle, declarations);
    }

    fn set_inline_style_for_indexed_handle(
        &self,
        handle: NodeId,
        declarations: Arc<Locked<PropertyDeclarationBlock>>,
    ) {
        let document = self
            .document_for_handle(handle)
            .expect("inline style side table requires an indexed owner document");
        self.documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .inline_styles
            .insert(handle, declarations);
    }

    fn clear_inline_styles_for_document(&self, document: NodeId) {
        let orphaned_handles = {
            let mut documents = self.documents.borrow_mut();
            let Some(bucket) = documents.get_mut(&document) else {
                return;
            };
            let affected_handles = bucket.inline_styles.keys().copied().collect::<Vec<_>>();
            bucket.inline_styles.clear();
            let orphaned_handles = affected_handles
                .into_iter()
                .filter(|handle| {
                    !self.bucket_contains_handle_for_owner_maintenance(bucket, *handle)
                })
                .collect::<Vec<_>>();
            if bucket.is_empty() {
                documents.remove(&document);
            }
            orphaned_handles
        };
        self.remove_owner_mappings_for_document(document, orphaned_handles);
    }

    fn selector_flag_handles_for_document(&self, document: NodeId) -> Vec<NodeId> {
        self.documents
            .borrow()
            .get(&document)
            .map(|bucket| bucket.selector_flags.keys().copied().collect())
            .unwrap_or_default()
    }

    fn has_dirty_descendants_for_document(&self, document: NodeId, handle: NodeId) -> bool {
        self.documents
            .borrow()
            .get(&document)
            .is_some_and(|bucket| bucket.dirty_descendants.contains(&handle))
    }

    fn set_dirty_descendants_for_document(&self, document: NodeId, handle: NodeId) {
        self.note_owner_document(handle, document);
        self.documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .dirty_descendants
            .insert(handle);
    }

    fn unset_dirty_descendants_for_document(&self, document: NodeId, handle: NodeId) {
        self.remove_from_document(document, handle, |bucket| {
            bucket.dirty_descendants.remove(&handle);
        });
    }

    fn clear_inline_style(&self, handle: NodeId) {
        self.remove_from_documents(handle, |bucket| {
            bucket.inline_styles.remove(&handle);
        });
    }

    fn apply_selector_flags_for_document(
        &self,
        host: &DomHost,
        document: NodeId,
        handle: NodeId,
        flags: ElementSelectorFlags,
    ) {
        let self_flags = flags.for_self();
        if !self_flags.is_empty() {
            self.note_owner_document(handle, document);
            self.documents
                .borrow_mut()
                .entry(document)
                .or_default()
                .selector_flags
                .entry(handle)
                .or_insert_with(ElementSelectorFlags::empty)
                .insert(self_flags);
        }

        let parent_flags = flags.for_parent();
        if parent_flags.is_empty() {
            return;
        }
        let Some(parent) = host.node(handle).and_then(Node::parent_node) else {
            return;
        };
        if host.node(parent).and_then(Node::as_element).is_none() && !host.is_shadow_root(parent) {
            return;
        }
        self.note_owner_document(parent, document);
        self.documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .selector_flags
            .entry(parent)
            .or_insert_with(ElementSelectorFlags::empty)
            .insert(parent_flags);
    }

    fn has_selector_flags_for_document(
        &self,
        document: NodeId,
        handle: NodeId,
        flags: ElementSelectorFlags,
    ) -> bool {
        self.documents
            .borrow()
            .get(&document)
            .and_then(|bucket| bucket.selector_flags.get(&handle))
            .is_some_and(|stored| stored.contains(flags))
    }

    fn relative_selector_search_direction_for_document(
        &self,
        document: NodeId,
        handle: NodeId,
    ) -> ElementSelectorFlags {
        self.documents
            .borrow()
            .get(&document)
            .and_then(|bucket| bucket.selector_flags.get(&handle))
            .copied()
            .unwrap_or_else(ElementSelectorFlags::empty)
            & ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING
    }

    pub(super) fn borrow_inline_style_for_host(
        &self,
        host: &DomHost,
        handle: NodeId,
    ) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.note_owner_document_for_host(host, handle);
        self.borrow_inline_style(handle)
    }

    pub(super) fn borrow_inline_style_for_document(
        &self,
        document: NodeId,
        handle: NodeId,
    ) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        let declarations = {
            let documents = self.documents.borrow();
            documents
                .get(&document)
                .and_then(|bucket| bucket.inline_styles.get(&handle))
                .map(|declarations| &**declarations as *const Locked<PropertyDeclarationBlock>)
        }?;
        // SAFETY: The pointed-to allocation is owned by a servo_arc::Arc. The
        // TElement contract already requires callers not to clear style data
        // while Stylo holds a borrowed declaration block.
        Some(unsafe { ArcBorrow::from_ref(&*declarations) })
    }

    pub(super) fn borrow_inline_style(
        &self,
        handle: NodeId,
    ) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        let declarations = {
            let documents = self.documents.borrow();
            self.find_inline_style(&documents, handle)
                .map(|declarations| &**declarations as *const Locked<PropertyDeclarationBlock>)
        }?;
        // SAFETY: The pointed-to allocation is owned by a servo_arc::Arc. The
        // TElement contract already requires callers not to clear style data
        // while Stylo holds a borrowed declaration block.
        Some(unsafe { ArcBorrow::from_ref(&*declarations) })
    }

    fn document_for_handle(&self, handle: NodeId) -> Option<NodeId> {
        self.handle_documents.borrow().get(&handle).copied()
    }

    fn remove_from_documents(
        &self,
        handle: NodeId,
        mut remove: impl FnMut(&mut StyloElementDocumentDataStore),
    ) {
        if let Some(document) = self.document_for_handle(handle) {
            self.remove_from_document(document, handle, remove);
            return;
        }

        // All side-table writes install an owner entry. Keep this recovery
        // path for stores created by older callers or partially initialized
        // state, but bound its work by document buckets rather than by every
        // indexed handle. If another payload survives, repair the missing
        // owner so subsequent mutations return to the O(1) path.
        let remaining_document = {
            let mut documents = self.documents.borrow_mut();
            let mut empty_documents = Vec::new();
            let mut remaining_document = None;
            for (document, bucket) in documents.iter_mut() {
                self.record_fallback_document_scan();
                remove(bucket);
                if bucket.contains_handle(handle) {
                    debug_assert!(
                        remaining_document.is_none(),
                        "one style handle must not have payloads in multiple document buckets"
                    );
                    remaining_document.get_or_insert(*document);
                }
                if bucket.is_empty() {
                    empty_documents.push(*document);
                }
            }
            for document in empty_documents {
                documents.remove(&document);
            }
            remaining_document
        };
        if let Some(document) = remaining_document {
            self.handle_documents.borrow_mut().insert(handle, document);
        }
    }

    fn remove_from_document(
        &self,
        document: NodeId,
        handle: NodeId,
        mut remove: impl FnMut(&mut StyloElementDocumentDataStore),
    ) {
        let handle_still_present = {
            let mut documents = self.documents.borrow_mut();
            let handle_still_present = if let Some(bucket) = documents.get_mut(&document) {
                remove(bucket);
                self.bucket_contains_handle_for_owner_maintenance(bucket, handle)
            } else {
                self.record_targeted_owner_membership_check();
                false
            };
            if documents
                .get(&document)
                .is_some_and(StyloElementDocumentDataStore::is_empty)
            {
                documents.remove(&document);
            }
            handle_still_present
        };
        if !handle_still_present {
            self.remove_owner_mapping_if_document_matches(document, handle);
        }
    }

    fn remove_owner_mapping_if_document_matches(&self, document: NodeId, handle: NodeId) {
        let mut handle_documents = self.handle_documents.borrow_mut();
        if handle_documents.get(&handle).copied() == Some(document) {
            handle_documents.remove(&handle);
        }
    }

    fn remove_owner_mappings_for_document(
        &self,
        document: NodeId,
        handles: impl IntoIterator<Item = NodeId>,
    ) {
        let mut handle_documents = self.handle_documents.borrow_mut();
        for handle in handles {
            if handle_documents.get(&handle).copied() == Some(document) {
                handle_documents.remove(&handle);
            }
        }
    }

    fn bucket_contains_handle_for_owner_maintenance(
        &self,
        bucket: &StyloElementDocumentDataStore,
        handle: NodeId,
    ) -> bool {
        self.record_targeted_owner_membership_check();
        bucket.contains_handle(handle)
    }

    #[cfg(test)]
    fn record_targeted_owner_membership_check(&self) {
        self.targeted_owner_membership_checks.set(
            self.targeted_owner_membership_checks
                .get()
                .saturating_add(1),
        );
    }

    #[cfg(not(test))]
    fn record_targeted_owner_membership_check(&self) {}

    #[cfg(test)]
    fn record_fallback_document_scan(&self) {
        self.fallback_document_scans
            .set(self.fallback_document_scans.get().saturating_add(1));
    }

    #[cfg(not(test))]
    fn record_fallback_document_scan(&self) {}

    fn find_data<'a>(
        &self,
        documents: &'a HashMap<NodeId, StyloElementDocumentDataStore>,
        handle: NodeId,
    ) -> Option<&'a ElementDataWrapper> {
        if let Some(document) = self.document_for_handle(handle)
            && let Some(data) = documents
                .get(&document)
                .and_then(|bucket| bucket.data.get(&handle))
        {
            return Some(data);
        }
        documents
            .values()
            .find_map(|bucket| bucket.data.get(&handle).map(|data| &**data))
    }

    fn find_inline_style<'a>(
        &self,
        documents: &'a HashMap<NodeId, StyloElementDocumentDataStore>,
        handle: NodeId,
    ) -> Option<&'a Arc<Locked<PropertyDeclarationBlock>>> {
        if let Some(document) = self.document_for_handle(handle)
            && let Some(declarations) = documents
                .get(&document)
                .and_then(|bucket| bucket.inline_styles.get(&handle))
        {
            return Some(declarations);
        }
        documents
            .values()
            .find_map(|bucket| bucket.inline_styles.get(&handle))
    }
}

impl StyloElementDocumentDataStore {
    fn is_empty(&self) -> bool {
        self.data.is_empty()
            && self.inline_styles.is_empty()
            && self.selector_flags.is_empty()
            && self.dirty_descendants.is_empty()
    }

    fn contains_handle(&self, handle: NodeId) -> bool {
        self.data.contains_key(&handle)
            || self.inline_styles.contains_key(&handle)
            || self.selector_flags.contains_key(&handle)
            || self.dirty_descendants.contains(&handle)
    }
}

impl fmt::Debug for StyloElementDataStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let documents = self.documents.borrow();
        f.debug_struct("StyloElementDataStore")
            .field("document_count", &documents.len())
            .field(
                "data_len",
                &documents
                    .values()
                    .map(|bucket| bucket.data.len())
                    .sum::<usize>(),
            )
            .field(
                "inline_styles_len",
                &documents
                    .values()
                    .map(|bucket| bucket.inline_styles.len())
                    .sum::<usize>(),
            )
            .field(
                "selector_flags_len",
                &documents
                    .values()
                    .map(|bucket| bucket.selector_flags.len())
                    .sum::<usize>(),
            )
            .field(
                "dirty_descendants_len",
                &documents
                    .values()
                    .map(|bucket| bucket.dirty_descendants.len())
                    .sum::<usize>(),
            )
            .finish()
    }
}

impl ShadowCascadeDataStore {
    fn set_for_host(&self, host: &DomHost, root: NodeId, cascade_data: Arc<CascadeData>) {
        let document = host
            .owner_document_handle(root)
            .expect("shadow cascade side table requires an owner document");
        self.set_for_document(document, root, cascade_data);
    }

    fn set_for_document(&self, document: NodeId, root: NodeId, cascade_data: Arc<CascadeData>) {
        self.note_owner_document(root, document);
        self.documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .insert(root, cascade_data);
    }

    fn set_for_indexed_root(&self, root: NodeId, cascade_data: Arc<CascadeData>) {
        let document = self
            .root_documents
            .borrow()
            .get(&root)
            .copied()
            .expect("shadow cascade side table requires an indexed owner document");
        self.documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .insert(root, cascade_data);
    }

    fn note_owner_document(&self, root: NodeId, document: NodeId) {
        let previous = self.root_documents.borrow_mut().insert(root, document);
        let Some(previous) = previous.filter(|previous| *previous != document) else {
            return;
        };
        let mut documents = self.documents.borrow_mut();
        let moved = documents
            .get_mut(&previous)
            .and_then(|bucket| bucket.remove(&root));
        if documents.get(&previous).is_some_and(HashMap::is_empty) {
            documents.remove(&previous);
        }
        if let Some(cascade_data) = moved {
            documents
                .entry(document)
                .or_default()
                .insert(root, cascade_data);
        }
    }

    fn clear_all(&self) {
        self.root_documents.borrow_mut().clear();
        self.documents.borrow_mut().clear();
    }

    fn clear_document(&self, document: NodeId) {
        self.documents.borrow_mut().remove(&document);
        self.root_documents
            .borrow_mut()
            .retain(|_, owner_document| *owner_document != document);
    }

    fn clear_roots(&self, roots: impl IntoIterator<Item = NodeId>) {
        let mut documents = self.documents.borrow_mut();
        for root in roots {
            if let Some(document) = self.root_documents.borrow().get(&root).copied() {
                if let Some(bucket) = documents.get_mut(&document) {
                    bucket.remove(&root);
                }
            } else {
                for bucket in documents.values_mut() {
                    bucket.remove(&root);
                }
            }
            self.root_documents.borrow_mut().remove(&root);
        }
        documents.retain(|_, bucket| !bucket.is_empty());
    }

    fn roots_for_document(&self, document: NodeId) -> Vec<NodeId> {
        self.documents
            .borrow()
            .get(&document)
            .map(|bucket| bucket.keys().copied().collect())
            .unwrap_or_default()
    }

    fn has(&self, root: NodeId) -> bool {
        let documents = self.documents.borrow();
        self.find(&documents, root).is_some()
    }

    fn get(&self, root: NodeId) -> Option<&CascadeData> {
        let cascade_data = {
            let documents = self.documents.borrow();
            self.find(&documents, root)
                .map(|cascade_data| &**cascade_data as *const CascadeData)
        }?;
        // SAFETY: The CascadeData is owned by a servo_arc::Arc stored in the
        // shadow cascade side table. The side table must not be cleared while
        // Stylo holds the returned reference.
        Some(unsafe { &*cascade_data })
    }

    fn document_count_for_test(&self) -> usize {
        self.documents
            .borrow()
            .values()
            .filter(|bucket| !bucket.is_empty())
            .count()
    }

    fn find<'a>(
        &self,
        documents: &'a HashMap<NodeId, HashMap<NodeId, Arc<CascadeData>>>,
        root: NodeId,
    ) -> Option<&'a Arc<CascadeData>> {
        if let Some(document) = self.root_documents.borrow().get(&root).copied()
            && let Some(cascade_data) = documents
                .get(&document)
                .and_then(|bucket| bucket.get(&root))
        {
            return Some(cascade_data);
        }
        documents.values().find_map(|bucket| bucket.get(&root))
    }
}

impl fmt::Debug for ShadowCascadeDataStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let documents = self.documents.borrow();
        f.debug_struct("ShadowCascadeDataStore")
            .field("document_count", &documents.len())
            .field(
                "cascade_data_len",
                &documents.values().map(HashMap::len).sum::<usize>(),
            )
            .finish()
    }
}

impl StyleDomState {
    fn host(&self) -> &DomHost {
        let host = self.host.get();
        assert!(!host.is_null(), "style adapter host should be bound");
        // SAFETY: The renderer binds the adapter to the DomHost for the
        // duration of one synchronous Stylo style resolution.
        unsafe { &*host }
    }

    fn try_host(&self) -> Option<&DomHost> {
        let host = self.host.get();
        if host.is_null() {
            return None;
        }
        // SAFETY: See host().
        Some(unsafe { &*host })
    }

    fn note_owner_document_if_bound(&self, handle: NodeId) {
        if let Some(host) = self.try_host() {
            self.element_data.note_owner_document_for_host(host, handle);
        }
    }

    fn node_data(&self, handle: NodeId, document: NodeId) -> NonNull<StyleNodeData> {
        let data = {
            let mut node_data = self.node_data.borrow_mut();
            let entry = node_data.entry(handle).or_insert_with(|| {
                Box::new(StyleNodeData {
                    state: self as *const StyleDomState,
                    handle,
                    document,
                })
            });
            if entry.document != document {
                entry.document = document;
            }
            &mut **entry as *mut StyleNodeData
        };
        // SAFETY: StyleNodeData is heap-allocated behind a Box, so its address
        // remains stable across HashMap rehashes. `bind_host()` clears this
        // arena before a new synchronous Stylo resolution starts; callers must
        // not retain Stylo wrappers across that boundary.
        NonNull::new(data).expect("boxed style node data pointers are never null")
    }

    fn clear_node_data(&self) {
        self.slotted_node_data.borrow_mut().clear();
        self.node_data.borrow_mut().clear();
    }

    fn slotted_nodes_for_slot<'a>(&'a self, slot: NodeId, document: NodeId) -> &'a [StyleNode<'a>] {
        let nodes = {
            let mut slotted_node_data = self.slotted_node_data.borrow_mut();
            let entry = slotted_node_data.entry(slot).or_insert_with(|| {
                self.host()
                    .assigned_nodes_for_slot_with_options(slot, false)
                    .into_iter()
                    .map(|handle| {
                        StyleNode::<'static>(self.node_data(handle, document), PhantomData)
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            });
            &**entry as *const [StyleNode<'static>]
        };
        // SAFETY: The cached nodes contain pointers into `node_data`, and both
        // tables are cleared together when the adapter is rebound after the
        // synchronous Stylo traversal. `StyleNode<'a>` only carries the traversal
        // lifetime in PhantomData, so narrowing the cached slice to `&self` is
        // equivalent to constructing the wrappers directly for this traversal.
        unsafe { &*nodes }
    }

    fn clear_element_data(&self, handle: NodeId) {
        self.note_owner_document_if_bound(handle);
        self.element_data.clear(handle);
    }

    fn clear_element_selector_flags(&self, handle: NodeId) {
        self.note_owner_document_if_bound(handle);
        self.element_data.clear_selector_flags(handle);
    }

    fn mark_element_style_dirty(&self, handle: NodeId) -> bool {
        self.note_owner_document_if_bound(handle);
        self.element_data.mark_style_dirty(handle)
    }

    fn clear_element_data_for_document(&self, document: NodeId) {
        self.element_data.clear_data_for_document(document);
    }

    fn has_element_styles_for_document(&self, document: NodeId) -> bool {
        self.element_data.has_style_values_for_document(document)
    }

    fn set_inline_style_attribute(
        &self,
        handle: NodeId,
        declarations: Arc<Locked<PropertyDeclarationBlock>>,
    ) {
        if let Some(host) = self.try_host() {
            self.element_data
                .set_inline_style_for_host(host, handle, declarations);
        } else {
            self.element_data
                .set_inline_style_for_indexed_handle(handle, declarations);
        }
    }

    fn clear_inline_style_attribute(&self, handle: NodeId) {
        self.note_owner_document_if_bound(handle);
        self.element_data.clear_inline_style(handle);
    }

    fn clear_inline_style_attributes_for_document(&self, document: NodeId) {
        self.element_data.clear_inline_styles_for_document(document);
    }

    fn element_selector_flag_handles_for_document(&self, document: NodeId) -> Vec<NodeId> {
        self.element_data
            .selector_flag_handles_for_document(document)
    }

    fn has_element_data(&self, handle: NodeId) -> bool {
        self.element_data.has(handle)
    }

    fn element_data_count(&self) -> usize {
        self.element_data.len()
    }

    fn set_shadow_cascade_data(&self, root: NodeId, cascade_data: Arc<CascadeData>) {
        if let Some(host) = self.try_host() {
            self.shadow_cascade_data
                .set_for_host(host, root, cascade_data);
        } else {
            self.shadow_cascade_data
                .set_for_indexed_root(root, cascade_data);
        }
    }

    fn set_shadow_cascade_data_for_document_for_test(
        &self,
        document: NodeId,
        root: NodeId,
        cascade_data: Arc<CascadeData>,
    ) {
        self.shadow_cascade_data
            .set_for_document(document, root, cascade_data);
    }

    fn clear_shadow_cascade_data_after_binding(&self) {
        self.shadow_cascade_data.clear_all();
    }

    fn clear_shadow_cascade_data_for_roots(&self, roots: impl IntoIterator<Item = NodeId>) {
        self.shadow_cascade_data.clear_roots(roots);
    }

    fn clear_shadow_cascade_data_for_document(&self, document: NodeId) {
        self.shadow_cascade_data.clear_document(document);
    }

    fn shadow_cascade_roots_for_document(&self, document: NodeId) -> Vec<NodeId> {
        self.shadow_cascade_data.roots_for_document(document)
    }

    fn has_shadow_cascade_data(&self, root: NodeId) -> bool {
        self.shadow_cascade_data.has(root)
    }

    fn shadow_cascade_data(&self, root: NodeId) -> Option<&CascadeData> {
        self.shadow_cascade_data.get(root)
    }

    fn element_side_table_document_count_for_test(&self) -> usize {
        self.element_data.document_count_for_test()
    }

    fn shadow_cascade_document_count_for_test(&self) -> usize {
        self.shadow_cascade_data.document_count_for_test()
    }

    fn with_snapshot_handles<R>(
        &self,
        handles: impl IntoIterator<Item = NodeId>,
        f: impl FnOnce() -> R,
    ) -> R {
        let previous = self
            .snapshot_handles
            .replace(handles.into_iter().collect::<HashSet<_>>());
        let mut reset = SnapshotHandlesReset {
            state: self,
            previous: Some(previous),
        };
        let result = f();
        reset.restore();
        result
    }

    fn has_snapshot(&self, handle: NodeId) -> bool {
        self.snapshot_handles.borrow().contains(&handle)
    }
}

impl SnapshotHandlesReset<'_> {
    fn restore(&mut self) {
        if let Some(previous) = self.previous.take() {
            self.state.snapshot_handles.replace(previous);
        }
    }
}

impl Drop for SnapshotHandlesReset<'_> {
    fn drop(&mut self) {
        self.restore();
    }
}

impl<'a> StyleNode<'a> {
    fn data(self) -> &'a StyleNodeData {
        // SAFETY: Style node data is owned by the style adapter's per-resolution
        // arena and remains stable until the adapter is rebound for the next
        // synchronous style resolution.
        unsafe { &*self.0.as_ptr() }
    }

    fn style_state(self) -> &'a StyleDomState {
        // SAFETY: StyleDomState is owned by StyloDomStyleAdapter and outlives a
        // synchronous style resolution.
        unsafe { &*self.data().state }
    }

    fn host(self) -> &'a DomHost {
        self.style_state().host()
    }

    fn handle(self) -> NodeId {
        self.data().handle
    }

    fn document(self) -> NodeId {
        self.data().document
    }

    fn node(self) -> Option<&'a Node> {
        self.host().node(self.handle())
    }

    pub fn as_document(&self) -> Option<StyleDocument<'a>> {
        self.node()?.as_document()?;
        Some(StyleDocument(self.0, PhantomData))
    }
}

impl<'a> StyleElement<'a> {
    fn data(self) -> &'a StyleNodeData {
        // SAFETY: See StyleNode::data.
        unsafe { &*self.0.as_ptr() }
    }

    fn style_state(self) -> &'a StyleDomState {
        // SAFETY: See StyleNode::state.
        unsafe { &*self.data().state }
    }

    fn host(self) -> &'a DomHost {
        self.style_state().host()
    }

    pub(in crate::stylo) fn handle(self) -> NodeId {
        self.data().handle
    }

    fn document(self) -> NodeId {
        self.data().document
    }

    fn node(self) -> &'a Node {
        self.host()
            .node(self.handle())
            .expect("style element node should exist")
    }

    fn element(self) -> &'a Element {
        self.node()
            .as_element()
            .expect("style element should wrap an element node")
    }

    pub(in crate::stylo) fn as_query(self) -> QueryElement<'a> {
        QueryElement::new(
            self.host(),
            self.handle(),
            &self.style_state().shared_lock,
            Some(&self.style_state().element_data),
            &self.style_state().atom_cache,
        )
    }

    fn synthesize_presentational_hints<V>(&self, hints: &mut V)
    where
        V: selectors::sink::Push<style::applicable_declarations::ApplicableDeclarationBlock>,
    {
        self.as_query().synthesize_presentational_hints(hints);
    }

    fn from_handle(state: &'a StyleDomState, handle: NodeId) -> Option<Self> {
        let document = state.host().owner_document_handle(handle)?;
        state.host().node(handle)?.as_element()?;
        Some(Self(state.node_data(handle, document), PhantomData))
    }

    fn from_handle_in_document(
        state: &'a StyleDomState,
        document: NodeId,
        handle: NodeId,
    ) -> Option<Self> {
        state.host().node(handle)?.as_element()?;
        Some(Self(state.node_data(handle, document), PhantomData))
    }
}

impl<'a> StyleDocument<'a> {
    fn data(self) -> &'a StyleNodeData {
        // SAFETY: See StyleNode::data.
        unsafe { &*self.0.as_ptr() }
    }

    fn style_state(self) -> &'a StyleDomState {
        // SAFETY: See StyleNode::state.
        unsafe { &*self.data().state }
    }

    fn host(self) -> &'a DomHost {
        self.style_state().host()
    }

    fn handle(self) -> NodeId {
        self.data().handle
    }

    fn document(self) -> NodeId {
        self.data().document
    }
}

impl<'a> StyleShadowRoot<'a> {
    fn data(self) -> &'a StyleNodeData {
        // SAFETY: See StyleNode::data.
        unsafe { &*self.0.as_ptr() }
    }

    fn style_state(self) -> &'a StyleDomState {
        // SAFETY: See StyleNode::state.
        unsafe { &*self.data().state }
    }

    fn host_dom(self) -> &'a DomHost {
        self.style_state().host()
    }

    fn handle(self) -> NodeId {
        self.data().handle
    }

    fn document(self) -> NodeId {
        self.data().document
    }
}

impl fmt::Debug for StyleNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StyleNode({})", self.data().handle.index())
    }
}

impl fmt::Debug for StyleElement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StyleElement({})", self.data().handle.index())
    }
}

impl fmt::Debug for StyleDocument<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StyleDocument")
    }
}

impl fmt::Debug for StyleShadowRoot<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StyleShadowRoot({})", self.data().handle.index())
    }
}

impl PartialEq for StyleNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.data().state == other.data().state && self.data().handle == other.data().handle
    }
}

impl Eq for StyleNode<'_> {}

impl PartialEq for StyleElement<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.data().state == other.data().state && self.data().handle == other.data().handle
    }
}

impl Eq for StyleElement<'_> {}

impl Hash for StyleElement<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.data().state as usize).hash(state);
        self.data().handle.hash(state);
    }
}

impl PartialEq for StyleDocument<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.data().state == other.data().state
    }
}

impl Eq for StyleDocument<'_> {}

impl PartialEq for StyleShadowRoot<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.data().state == other.data().state && self.data().handle == other.data().handle
    }
}

impl Eq for StyleShadowRoot<'_> {}

impl NodeInfo for StyleNode<'_> {
    fn is_element(&self) -> bool {
        self.node().is_some_and(Node::is_element)
    }

    fn is_text_node(&self) -> bool {
        self.node().is_some_and(Node::is_text)
    }
}

impl<'a> TDocument for StyleDocument<'a> {
    type ConcreteNode = StyleNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        StyleNode(
            self.style_state().node_data(self.handle(), self.document()),
            PhantomData,
        )
    }

    fn is_html_document(&self) -> bool {
        self.host()
            .document_element_handle()
            .and_then(|handle| self.host().node(handle))
            .and_then(Node::as_element)
            .is_some_and(|element| element.namespace() == "http://www.w3.org/1999/xhtml")
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.host()
            .node(self.handle())
            .and_then(Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(QuirksMode::NoQuirks)
    }

    fn shared_lock(&self) -> &SharedRwLock {
        &self.style_state().shared_lock
    }
}

impl<'a> TNode for StyleNode<'a> {
    type ConcreteElement = StyleElement<'a>;
    type ConcreteDocument = StyleDocument<'a>;
    type ConcreteShadowRoot = StyleShadowRoot<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.node().and_then(Node::parent_node).map(|handle| {
            StyleNode(
                self.style_state().node_data(handle, self.document()),
                PhantomData,
            )
        })
    }

    fn first_child(&self) -> Option<Self> {
        self.node().and_then(Node::first_child).map(|handle| {
            StyleNode(
                self.style_state().node_data(handle, self.document()),
                PhantomData,
            )
        })
    }

    fn last_child(&self) -> Option<Self> {
        self.node().and_then(Node::last_child).map(|handle| {
            StyleNode(
                self.style_state().node_data(handle, self.document()),
                PhantomData,
            )
        })
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.node().and_then(Node::prev_sibling).map(|handle| {
            StyleNode(
                self.style_state().node_data(handle, self.document()),
                PhantomData,
            )
        })
    }

    fn next_sibling(&self) -> Option<Self> {
        self.node().and_then(Node::next_sibling).map(|handle| {
            StyleNode(
                self.style_state().node_data(handle, self.document()),
                PhantomData,
            )
        })
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        let document = self.document();
        StyleDocument(
            self.style_state().node_data(document, document),
            PhantomData,
        )
    }

    fn is_in_document(&self) -> bool {
        self.node()
            .is_some_and(|node| node.flags().in_document_tree())
    }

    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        let parent = self.parent_node()?;
        if let Some(element) = parent.as_element() {
            return Some(element);
        }
        parent.as_shadow_root().map(|root| root.host())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.handle().index())
    }

    fn debug_id(self) -> usize {
        self.handle().index()
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        self.node()?.as_element()?;
        Some(StyleElement(self.0, PhantomData))
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        self.node()?.as_document()?;
        Some(StyleDocument(self.0, PhantomData))
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        self.host()
            .is_shadow_root(self.handle())
            .then_some(StyleShadowRoot(self.0, PhantomData))
    }
}

impl<'a> TShadowRoot for StyleShadowRoot<'a> {
    type ConcreteNode = StyleNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        StyleNode(self.0, PhantomData)
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        let host = self
            .host_dom()
            .shadow_root_host(self.handle())
            .expect("shadow root should have a host");
        StyleElement(
            self.style_state().node_data(host, self.document()),
            PhantomData,
        )
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        self.style_state().shadow_cascade_data(self.handle())
    }
}

impl<'a> TElement for StyleElement<'a> {
    type ConcreteNode = StyleNode<'a>;
    type TraversalChildrenIterator = vec::IntoIter<StyleNode<'a>>;

    fn get_attr(&self, attr: &LocalName, namespace: &Namespace) -> Option<String> {
        self.as_query().get_attr(attr, namespace)
    }

    fn as_node(&self) -> Self::ConcreteNode {
        StyleNode(self.0, PhantomData)
    }

    fn traversal_children(&self) -> LayoutIterator<Self::TraversalChildrenIterator> {
        let children = self
            .host()
            .child_handles(self.handle())
            .map(|handle| {
                StyleNode(
                    self.style_state().node_data(handle, self.document()),
                    PhantomData,
                )
            })
            .collect::<Vec<_>>();
        LayoutIterator(children.into_iter())
    }

    fn is_html_element(&self) -> bool {
        self.element().namespace() == "http://www.w3.org/1999/xhtml"
    }

    fn is_mathml_element(&self) -> bool {
        self.element().namespace() == "http://www.w3.org/1998/Math/MathML"
    }

    fn is_svg_element(&self) -> bool {
        self.element().namespace() == "http://www.w3.org/2000/svg"
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.style_state()
            .element_data
            .borrow_inline_style_for_document(self.document(), self.handle())
    }

    fn slotted_nodes(&self) -> &[Self::ConcreteNode] {
        if !self.is_html_slot_element()
            || self.host().containing_shadow_root(self.handle()).is_none()
        {
            return &[];
        }
        self.style_state()
            .slotted_nodes_for_slot(self.handle(), self.document())
    }

    fn animation_rule(
        &self,
        _: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn transition_rule(
        &self,
        _: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn state(&self) -> ElementState {
        self.as_query().computed_state()
    }

    fn has_part_attr(&self) -> bool {
        self.element().has_attribute("part")
    }

    fn exports_any_part(&self) -> bool {
        self.element().has_attribute("exportparts")
    }

    fn id(&self) -> Option<&Atom> {
        self.element()
            .id()
            .map(|id| self.style_state().atom_cache.atom(id))
    }

    fn each_class<F>(&self, callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        self.as_query().each_class(callback);
    }

    fn each_part<F>(&self, callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        self.as_query().each_part(callback);
    }

    fn each_exported_part<F>(&self, name: &AtomIdent, callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        self.as_query().each_exported_part(name, callback);
    }

    fn each_custom_state<F>(&self, callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        self.as_query().each_custom_state(callback);
    }

    fn each_attr_name<F>(&self, callback: F)
    where
        F: FnMut(&LocalName),
    {
        self.as_query().each_attr_name(callback);
    }

    fn has_dirty_descendants(&self) -> bool {
        self.style_state()
            .element_data
            .has_dirty_descendants_for_document(self.document(), self.handle())
    }

    fn has_snapshot(&self) -> bool {
        self.style_state().has_snapshot(self.handle())
    }

    fn handled_snapshot(&self) -> bool {
        true
    }

    unsafe fn set_handled_snapshot(&self) {}

    unsafe fn set_dirty_descendants(&self) {
        self.style_state()
            .element_data
            .set_dirty_descendants_for_document(self.document(), self.handle());
    }

    unsafe fn unset_dirty_descendants(&self) {
        self.style_state()
            .element_data
            .unset_dirty_descendants_for_document(self.document(), self.handle());
    }

    fn store_children_to_process(&self, _n: isize) {}

    fn did_process_child(&self) -> isize {
        0
    }

    unsafe fn ensure_data(&self) -> ElementDataMut<'_> {
        self.style_state()
            .element_data
            .ensure_for_document(self.document(), self.handle())
    }

    unsafe fn clear_data(&self) {
        self.style_state()
            .element_data
            .clear_for_document(self.document(), self.handle());
    }

    fn has_data(&self) -> bool {
        self.style_state()
            .element_data
            .has_for_document(self.document(), self.handle())
    }

    fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.style_state()
            .element_data
            .borrow_for_document(self.document(), self.handle())
    }

    fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        self.style_state()
            .element_data
            .mutate_for_document(self.document(), self.handle())
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        false
    }

    fn has_animations(&self, _: &SharedStyleContext) -> bool {
        false
    }

    fn has_css_animations(&self, _: &SharedStyleContext, _: Option<PseudoElement>) -> bool {
        false
    }

    fn has_css_transitions(&self, _: &SharedStyleContext, _: Option<PseudoElement>) -> bool {
        false
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        self.host().shadow_root_handle(self.handle()).map(|handle| {
            StyleShadowRoot(
                self.style_state().node_data(handle, self.document()),
                PhantomData,
            )
        })
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        self.host()
            .containing_shadow_root(self.handle())
            .map(|handle| {
                StyleShadowRoot(
                    self.style_state().node_data(handle, self.document()),
                    PhantomData,
                )
            })
    }

    fn lang_attr(&self) -> Option<SelectorAttrValue> {
        self.as_query().lang_attr()
    }

    fn match_element_lang(
        &self,
        override_lang: Option<Option<SelectorAttrValue>>,
        value: &Lang,
    ) -> bool {
        self.as_query().match_element_lang(override_lang, value)
    }

    fn is_html_document_body_element(&self) -> bool {
        self.host().document_body_handle() == Some(self.handle())
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _: VisitedHandlingMode,
        hints: &mut V,
    ) where
        V: selectors::sink::Push<style::applicable_declarations::ApplicableDeclarationBlock>,
    {
        self.synthesize_presentational_hints(hints);
    }

    fn local_name(&self) -> &<SelectorImpl as selectors::parser::SelectorImpl>::BorrowedLocalName {
        self.style_state()
            .atom_cache
            .local_name(self.element().local_name())
    }

    fn namespace(
        &self,
    ) -> &<SelectorImpl as selectors::parser::SelectorImpl>::BorrowedNamespaceUrl {
        self.style_state()
            .atom_cache
            .namespace(self.element().namespace())
    }

    fn query_container_size(&self, _: &Display) -> Size2D<Option<Au>> {
        inline_container_size(self.host(), self.handle())
    }

    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        self.style_state()
            .element_data
            .has_selector_flags_for_document(self.document(), self.handle(), flags)
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        self.style_state()
            .element_data
            .relative_selector_search_direction_for_document(self.document(), self.handle())
    }
}

fn inline_container_size(host: &DomHost, handle: NodeId) -> Size2D<Option<Au>> {
    let Some(style) = host.get_attribute(handle, "style") else {
        return Size2D::new(None, None);
    };
    let mut width = None;
    let mut height = None;
    for declaration in moli_css_parse::parse_declaration_list(
        &style,
        moli_css_parse::DeclarationParseOptions {
            canonicalize_property_name: true,
            unescape_value_semicolons: false,
            preserve_empty_values: false,
        },
    ) {
        let parsed = moli_css_parse::parse_px_length(
            &declaration.value,
            moli_css_parse::UnitlessLength::ZeroOnly,
        )
        .map(|px| Au::from_f32_px(px.max(0.0) as f32));
        match declaration.name.as_str() {
            "width" => width = parsed,
            "height" => height = parsed,
            _ => {}
        }
    }
    Size2D::new(width, height)
}

impl SelectorsElement for StyleElement<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        self.as_query().opaque()
    }

    fn parent_element(&self) -> Option<Self> {
        let parent = self.node().parent_node()?;
        Self::from_handle(self.style_state(), parent)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        self.node()
            .parent_node()
            .is_some_and(|parent| self.host().is_shadow_root(parent))
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        let root = self.host().containing_shadow_root(self.handle())?;
        let host = self.host().shadow_root_host(root)?;
        Self::from_handle(self.style_state(), host)
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.node()
            .previous_element_sibling(self.host().dom())
            .and_then(|handle| Self::from_handle(self.style_state(), handle))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.node()
            .next_element_sibling(self.host().dom())
            .and_then(|handle| Self::from_handle(self.style_state(), handle))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node()
            .first_element_child(self.host().dom())
            .and_then(|handle| Self::from_handle(self.style_state(), handle))
    }

    fn first_element_child_for_featureless_host_has(&self) -> Option<Self> {
        let shadow_root = self.host().shadow_root_handle(self.handle())?;
        self.host()
            .node(shadow_root)?
            .first_element_child(self.host().dom())
            .and_then(|handle| Self::from_handle(self.style_state(), handle))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.as_query().is_html_element_in_html_document()
    }

    fn has_local_name(
        &self,
        local_name: &<Self::Impl as selectors::parser::SelectorImpl>::BorrowedLocalName,
    ) -> bool {
        self.element().local_name() == local_name.as_ref()
    }

    fn has_namespace(
        &self,
        ns: &<Self::Impl as selectors::parser::SelectorImpl>::BorrowedNamespaceUrl,
    ) -> bool {
        self.element().namespace() == ns.as_ref()
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.element().local_name() == other.element().local_name()
            && self.element().namespace() == other.element().namespace()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&<Self::Impl as selectors::parser::SelectorImpl>::NamespaceUrl>,
        local_name: &<Self::Impl as selectors::parser::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<
            &<Self::Impl as selectors::parser::SelectorImpl>::AttrValue,
        >,
    ) -> bool {
        self.as_query().attr_matches(ns, local_name, operation)
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &<Self::Impl as selectors::parser::SelectorImpl>::NonTSPseudoClass,
        context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        self.as_query().match_non_ts_pseudo_class(pc, context)
    }

    fn match_pseudo_element(
        &self,
        pseudo_element: &<Self::Impl as selectors::parser::SelectorImpl>::PseudoElement,
        context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        self.as_query()
            .match_pseudo_element(pseudo_element, context)
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        self.style_state()
            .element_data
            .apply_selector_flags_for_document(self.host(), self.document(), self.handle(), flags);
    }

    fn is_link(&self) -> bool {
        self.as_query().is_link()
    }

    fn is_html_slot_element(&self) -> bool {
        self.element().is_html_element("slot")
    }

    fn assigned_slot(&self) -> Option<Self> {
        self.host()
            .assigned_slot_for_node(self.handle())
            .and_then(|handle| {
                Self::from_handle_in_document(self.style_state(), self.document(), handle)
            })
    }

    fn has_id(
        &self,
        id: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.as_query().has_id(id, case_sensitivity)
    }

    fn has_class(
        &self,
        name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.as_query().has_class(name, case_sensitivity)
    }

    fn has_custom_state(
        &self,
        state: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
    ) -> bool {
        self.as_query().has_custom_state(state)
    }

    fn imported_part(
        &self,
        name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
    ) -> Option<<Self::Impl as selectors::parser::SelectorImpl>::Identifier> {
        self.as_query().imported_part(name)
    }

    fn is_part(&self, name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier) -> bool {
        self.as_query().is_part(name)
    }

    fn is_empty(&self) -> bool {
        self.as_query().is_empty()
    }

    fn is_root(&self) -> bool {
        self.node()
            .parent_node()
            .and_then(|parent| self.host().node(parent))
            .is_some_and(Node::is_document)
    }

    fn add_element_unique_hashes(&self, filter: &mut BloomFilter) -> bool {
        self.as_query().add_element_unique_hashes(filter)
    }
}

#[cfg(test)]
mod element_data_store_tests {
    use super::*;

    fn empty_inline_style() -> Arc<Locked<PropertyDeclarationBlock>> {
        let lock = SharedRwLock::read_only();
        Arc::new(lock.wrap(PropertyDeclarationBlock::new()))
    }

    #[test]
    fn targeted_element_data_cleanup_is_linear_in_removed_handles() {
        const HANDLE_COUNT: usize = 4_096;
        let store = StyloElementDataStore::default();
        let document = NodeId::new(0);
        let handles = (1..=HANDLE_COUNT).map(NodeId::new).collect::<Vec<_>>();

        for &handle in &handles {
            drop(store.ensure_for_document(document, handle));
        }
        assert_eq!(store.len(), HANDLE_COUNT);
        assert_eq!(store.handle_documents.borrow().len(), HANDLE_COUNT);

        store.targeted_owner_membership_checks.set(0);
        store.fallback_document_scans.set(0);
        for &handle in &handles {
            store.clear_for_document(document, handle);
        }

        assert_eq!(store.targeted_owner_membership_checks.get(), HANDLE_COUNT);
        assert_eq!(store.fallback_document_scans.get(), 0);
        assert_eq!(store.len(), 0);
        assert!(store.handle_documents.borrow().is_empty());
        assert!(store.documents.borrow().is_empty());
    }

    #[test]
    fn owner_mapping_survives_until_every_handle_payload_is_removed() {
        let store = StyloElementDataStore::default();
        let document = NodeId::new(0);
        let handle = NodeId::new(1);

        drop(store.ensure_for_document(document, handle));
        store.set_inline_style_for_indexed_handle(handle, empty_inline_style());
        store
            .documents
            .borrow_mut()
            .get_mut(&document)
            .unwrap()
            .selector_flags
            .insert(handle, ElementSelectorFlags::HAS_SLOW_SELECTOR);

        store.clear_for_document(document, handle);
        assert_eq!(store.document_for_handle(handle), Some(document));
        assert!(store.borrow_inline_style(handle).is_some());

        store.clear_inline_style(handle);
        assert_eq!(store.document_for_handle(handle), Some(document));
        assert!(
            store
                .documents
                .borrow()
                .get(&document)
                .unwrap()
                .selector_flags
                .contains_key(&handle)
        );

        store.clear_selector_flags(handle);
        assert_eq!(store.document_for_handle(handle), None);
        assert!(store.documents.borrow().is_empty());
    }

    #[test]
    fn moving_a_handle_preserves_new_owner_during_old_document_cleanup() {
        let store = StyloElementDataStore::default();
        let old_document = NodeId::new(0);
        let new_document = NodeId::new(1);
        let handle = NodeId::new(2);

        drop(store.ensure_for_document(old_document, handle));
        store.set_inline_style_for_indexed_handle(handle, empty_inline_style());
        store
            .documents
            .borrow_mut()
            .get_mut(&old_document)
            .unwrap()
            .selector_flags
            .insert(handle, ElementSelectorFlags::HAS_SLOW_SELECTOR);

        store.note_owner_document(handle, new_document);
        assert_eq!(store.document_for_handle(handle), Some(new_document));
        assert!(!store.has_for_document(old_document, handle));
        assert!(store.has_for_document(new_document, handle));
        assert!(!store.documents.borrow().contains_key(&old_document));

        store.clear_for_document(old_document, handle);
        assert_eq!(store.document_for_handle(handle), Some(new_document));
        assert!(store.has_for_document(new_document, handle));

        store.clear_style_values(handle);
        assert_eq!(store.document_for_handle(handle), Some(new_document));
        store.clear_selector_flags(handle);
        assert_eq!(store.document_for_handle(handle), None);
        assert!(store.documents.borrow().is_empty());
    }

    #[test]
    fn dirty_descendant_breadcrumb_moves_and_retires_with_its_owner() {
        let store = StyloElementDataStore::default();
        let old_document = NodeId::new(0);
        let new_document = NodeId::new(1);
        let handle = NodeId::new(2);

        store.set_dirty_descendants_for_document(old_document, handle);
        assert!(store.has_dirty_descendants_for_document(old_document, handle));
        assert_eq!(store.document_for_handle(handle), Some(old_document));

        store.note_owner_document(handle, new_document);
        assert!(!store.has_dirty_descendants_for_document(old_document, handle));
        assert!(store.has_dirty_descendants_for_document(new_document, handle));
        assert_eq!(store.document_for_handle(handle), Some(new_document));

        store.unset_dirty_descendants_for_document(new_document, handle);
        assert_eq!(store.document_for_handle(handle), None);
        assert!(store.documents.borrow().is_empty());
    }

    #[test]
    fn document_bulk_cleanup_updates_only_affected_owner_entries() {
        let store = StyloElementDataStore::default();
        let document = NodeId::new(0);
        let data_and_inline = NodeId::new(1);
        let data_only = NodeId::new(2);
        let inline_only = NodeId::new(3);
        let flags_only = NodeId::new(4);

        drop(store.ensure_for_document(document, data_and_inline));
        drop(store.ensure_for_document(document, data_only));
        store.note_owner_document(inline_only, document);
        store.note_owner_document(flags_only, document);
        store.set_inline_style_for_indexed_handle(data_and_inline, empty_inline_style());
        store.set_inline_style_for_indexed_handle(inline_only, empty_inline_style());
        store
            .documents
            .borrow_mut()
            .get_mut(&document)
            .unwrap()
            .selector_flags
            .insert(flags_only, ElementSelectorFlags::HAS_SLOW_SELECTOR);

        store.targeted_owner_membership_checks.set(0);
        store.clear_data_for_document(document);
        assert_eq!(store.targeted_owner_membership_checks.get(), 3);
        assert_eq!(store.document_for_handle(data_and_inline), Some(document));
        assert_eq!(store.document_for_handle(data_only), None);
        assert_eq!(store.document_for_handle(inline_only), Some(document));
        assert_eq!(store.document_for_handle(flags_only), None);

        store.clear_inline_styles_for_document(document);
        assert_eq!(store.targeted_owner_membership_checks.get(), 5);
        assert!(store.handle_documents.borrow().is_empty());
        assert!(store.documents.borrow().is_empty());
    }

    #[test]
    fn missing_owner_fallback_repairs_a_surviving_payload_owner() {
        let store = StyloElementDataStore::default();
        let document = NodeId::new(0);
        let handle = NodeId::new(1);
        store
            .documents
            .borrow_mut()
            .entry(document)
            .or_default()
            .selector_flags
            .insert(handle, ElementSelectorFlags::HAS_SLOW_SELECTOR);

        store.clear(handle);
        assert_eq!(store.fallback_document_scans.get(), 1);
        assert_eq!(store.document_for_handle(handle), Some(document));

        store.clear_selector_flags(handle);
        assert_eq!(store.targeted_owner_membership_checks.get(), 1);
        assert!(store.handle_documents.borrow().is_empty());
        assert!(store.documents.borrow().is_empty());
    }
}
