use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use moli_dom::native::{DomAdoptionTarget, NativeDom};

use super::{BridgeIdentityStore, PageScriptHost, SharedWorldWrapperCache};
use crate::{document_runtime::DomHandle, util};

type WeakRealm = Rc<CreationRealm>;
const CREATION_REALMS_SLOT: &str = "__moliNativeNodeCreationRealms";

#[derive(Debug)]
struct CreationRealm {
    context: v8::Weak<v8::Context>,
    retired: Cell<bool>,
}

impl CreationRealm {
    fn to_local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Context>> {
        self.context.to_local(scope)
    }
}

#[derive(Debug)]
enum AdoptedNodeCreationRealm {
    Document(DomHandle),
    Transferred(WeakRealm),
}

/// Remember the creation document only for nodes that have changed documents.
/// The Document's weak V8 reference is independent of wrapper-cache and
/// dispatch retirement. Multiple adoptions preserve the first source.
#[derive(Debug, Default)]
pub(super) struct NodeCreationRealms {
    documents: HashMap<DomHandle, WeakRealm>,
    adopted: HashMap<DomHandle, AdoptedNodeCreationRealm>,
    adoptions_to_trace: HashSet<DomHandle>,
    tree_references: HashMap<DomHandle, Vec<WeakRealm>>,
    references_version: Option<u64>,
}

impl BridgeIdentityStore {
    pub(in crate::native_bridge) fn remember_document_creation_realm(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        dom: &NativeDom,
        document: DomHandle,
    ) {
        let context = scope.get_current_context();
        if !is_default_wrapper_world(context) {
            return;
        }
        let realms = self
            .node_creation_realms
            .get_or_insert_with(Default::default);
        if let std::collections::hash_map::Entry::Vacant(entry) = realms.documents.entry(document) {
            let realm = context.get_slot::<CreationRealm>().unwrap_or_else(|| {
                let realm = Rc::new(CreationRealm {
                    context: v8::Weak::new(scope, context),
                    retired: Cell::new(
                        context.get_slot::<super::RetiredWrapperContext>().is_some(),
                    ),
                });
                let _ = context.set_slot(realm.clone());
                realm
            });
            entry.insert(realm.clone());
            realms.references_version = None;
            if realm.retired.get() {
                realms.trace_retired_adoptions(dom, &realm);
            }
        }
    }

    pub(in crate::native_bridge) fn remember_node_creation_documents(
        &mut self,
        dom: &NativeDom,
        targets: &[DomAdoptionTarget],
    ) {
        for target in targets {
            if target.old_document != target.new_document {
                let realms = self
                    .node_creation_realms
                    .get_or_insert_with(Default::default);
                let source_document = match realms
                    .adopted
                    .entry(target.handle)
                    .or_insert(AdoptedNodeCreationRealm::Document(target.old_document))
                {
                    AdoptedNodeCreationRealm::Document(document) => Some(*document),
                    AdoptedNodeCreationRealm::Transferred(_) => None,
                };
                if source_document.is_none_or(|document| {
                    realms
                        .document_realm(dom, document)
                        .is_some_and(|realm| realm.retired.get())
                }) {
                    realms.adoptions_to_trace.insert(target.handle);
                    realms.references_version = None;
                }
            }
        }
    }

    pub(in crate::native_bridge) fn node_creation_realm<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &PageScriptHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Context>> {
        if !is_default_wrapper_world(scope.get_current_context()) {
            return None;
        }
        let realms = self.node_creation_realms.as_ref()?;
        let dom = host.dom_host().dom();
        let realm = match realms.adopted.get(&handle) {
            Some(adopted) => realms.adopted_realm(dom, adopted),
            None => realms.document_realm(dom, host.dom_host().owner_document_handle(handle)?),
        };
        realm?.to_local(scope)
    }

    pub(super) fn transfer_node_creation_realm(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        source: &mut Self,
        source_dom: &NativeDom,
        old: DomHandle,
        new: DomHandle,
        source_document: Option<DomHandle>,
    ) {
        let Some(realms) = source.node_creation_realms.as_mut() else {
            return;
        };
        realms.adoptions_to_trace.remove(&old);
        let realm = match realms.adopted.remove(&old) {
            Some(AdoptedNodeCreationRealm::Transferred(realm)) => Some(realm),
            Some(AdoptedNodeCreationRealm::Document(document)) => {
                realms.document_realm(source_dom, document).cloned()
            }
            None => source_document
                .and_then(|document| realms.document_realm(source_dom, document).cloned()),
        };
        if let Some(realm) = realm {
            // V8 allocations during retargeting can collect the old ownership
            // anchor before the destination has installed its references.
            let _keep_alive = realm.to_local(scope);
            let target = self
                .node_creation_realms
                .get_or_insert_with(Default::default);
            target
                .adopted
                .insert(new, AdoptedNodeCreationRealm::Transferred(realm));
            // A different Page owns this tree. A source teardown cannot update
            // its anchors, so install destination edges during transfer.
            target.adoptions_to_trace.insert(new);
            target.references_version = None;
        }
        realms.references_version = None;
    }

    pub(super) fn retire_node_creation_realm(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
    ) {
        let Some(realm) = scope.get_current_context().get_slot::<CreationRealm>() else {
            return;
        };
        realm.retired.set(true);
        if let Some(realms) = &mut self.node_creation_realms {
            realms.trace_retired_adoptions(unsafe { &*host_ptr }.dom_host().dom(), &realm);
        }
        self.synchronize_node_creation_references(scope, host_ptr);
    }

    /// Mirror native tree reachability into the V8 graph. A retained wrapper
    /// can reach nodes through parents and siblings as well as descendants.
    /// Cache one set of weak realm references per tree, then attach the V8
    /// edges to its live ownership anchors. Unlinking a tree releases the
    /// Document's references without rooting the disconnected tree in Rust.
    pub(in crate::native_bridge) fn synchronize_node_creation_references(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
    ) {
        let host = unsafe { &*host_ptr };
        let dom = host.dom_host().dom();
        let version = dom.tree_version();
        let Some(realms) = self.node_creation_realms.as_ref().filter(|realms| {
            realms.references_version != Some(version)
                && (!realms.adoptions_to_trace.is_empty() || !realms.tree_references.is_empty())
        }) else {
            return;
        };
        let Some(owner) = self.node_owner_context(scope, host_ptr) else {
            return;
        };
        let scope = &mut v8::ContextScope::new(scope, owner);
        let mut references = HashMap::<DomHandle, Vec<WeakRealm>>::new();
        let mut redundant = Vec::new();
        for &node in &realms.adoptions_to_trace {
            let Some(realm) = realms
                .adopted
                .get(&node)
                .and_then(|provenance| realms.adopted_realm(dom, provenance))
            else {
                continue;
            };
            let Some(creation) = realm.to_local(scope) else {
                continue;
            };
            // Once the current Document's realm is recorded, ordinary
            // wrapping already resolves to that realm. Drop those entries
            // before later mutations have to scan them again.
            let document_realm = host
                .dom_host()
                .owner_document_handle(node)
                .and_then(|document| realms.documents.get(&document))
                .and_then(|realm| realm.to_local(scope));
            if document_realm == Some(creation) {
                redundant.push(node);
            } else if creation != owner
                && let Some(root) = associated_tree_root(dom, node)
            {
                let tree = references.entry(root).or_default();
                if !tree.iter().any(|existing| Rc::ptr_eq(existing, realm)) {
                    tree.push(realm.clone());
                }
            }
        }
        if !references.is_empty() || !realms.tree_references.is_empty() {
            let values = references
                .iter()
                .map(|(&tree, realms)| (tree, realm_references(scope, realms)))
                .collect::<HashMap<_, _>>();
            for (&node, ownership) in &self.node_ownership {
                if let Some(anchor) = ownership.existing_anchor(scope) {
                    let value = associated_tree_root(dom, node)
                        .and_then(|root| values.get(&root).copied().flatten());
                    set_realm_references(scope, anchor, value);
                }
            }
        }
        let realms = self
            .node_creation_realms
            .as_mut()
            .expect("creation realms remain indexed");
        for node in redundant {
            realms.adopted.remove(&node);
            realms.adoptions_to_trace.remove(&node);
        }
        realms.references_version = Some(version);
        realms.tree_references = references;
    }

    pub(super) fn trace_node_creation_references(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host: &PageScriptHost,
        node: DomHandle,
        anchor: v8::Local<'_, v8::Object>,
    ) {
        let Some(realms) = &self.node_creation_realms else {
            return;
        };
        let dom = host.dom_host().dom();
        if realms.tree_references.is_empty()
            || realms.references_version != Some(dom.tree_version())
        {
            return;
        }
        if let Some(references) =
            associated_tree_root(dom, node).and_then(|root| realms.tree_references.get(&root))
            && let Some(owner) = anchor.get_creation_context(scope)
        {
            let scope = &mut v8::ContextScope::new(scope, owner);
            let value = realm_references(scope, references);
            set_realm_references(scope, anchor, value);
        }
    }
}

impl NodeCreationRealms {
    fn trace_retired_adoptions(&mut self, dom: &NativeDom, realm: &WeakRealm) {
        let previous = self.adoptions_to_trace.len();
        for (&node, provenance) in &self.adopted {
            if self
                .adopted_realm(dom, provenance)
                .is_some_and(|source| Rc::ptr_eq(source, realm))
            {
                self.adoptions_to_trace.insert(node);
            }
        }
        if self.adoptions_to_trace.len() != previous {
            self.references_version = None;
        }
    }

    fn adopted_realm<'a>(
        &'a self,
        dom: &NativeDom,
        provenance: &'a AdoptedNodeCreationRealm,
    ) -> Option<&'a WeakRealm> {
        match provenance {
            AdoptedNodeCreationRealm::Document(document) => self.document_realm(dom, *document),
            AdoptedNodeCreationRealm::Transferred(realm) => Some(realm),
        }
    }

    fn document_realm(&self, dom: &NativeDom, document: DomHandle) -> Option<&WeakRealm> {
        self.documents.get(&document).or_else(|| {
            let creator = dom
                .is_inert_template_document(document)
                .then(|| dom.inert_template_document_creator(document))
                .flatten()?;
            self.documents.get(&creator)
        })
    }
}

fn associated_tree_root(dom: &NativeDom, mut handle: DomHandle) -> Option<DomHandle> {
    loop {
        let node = dom.node(handle)?;
        let Some(parent) = node
            .parent_node()
            .or_else(|| node.as_document_fragment()?.host())
        else {
            return Some(handle);
        };
        handle = parent;
    }
}

fn realm_references<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    realms: &[WeakRealm],
) -> Option<v8::Local<'s, v8::Array>> {
    let mut anchors = Vec::with_capacity(realms.len());
    for realm in realms {
        if let Some(context) = realm.to_local(scope) {
            // The extras object belongs to the original context even if its
            // WindowProxy was detached and reused after navigation.
            anchors.push(context.get_extras_binding_object(scope).into());
        }
    }
    (!anchors.is_empty()).then(|| v8::Array::new_with_elements(scope, &anchors))
}

pub(super) fn set_realm_references(
    scope: &mut v8::PinScope<'_, '_>,
    anchor: v8::Local<'_, v8::Object>,
    value: Option<v8::Local<'_, v8::Array>>,
) {
    let anchor = v8::Local::new(scope, anchor);
    if value.is_some() || util::get_private_object(scope, anchor, CREATION_REALMS_SLOT).is_some() {
        let value = value.map_or_else(|| v8::undefined(scope).into(), Into::into);
        util::set_private_value(scope, anchor, CREATION_REALMS_SLOT, value);
    }
}

fn is_default_wrapper_world(context: v8::Local<'_, v8::Context>) -> bool {
    context.get_slot::<SharedWorldWrapperCache>().is_some()
        && super::super::isolated_window_world_for_context(context).is_none()
}
