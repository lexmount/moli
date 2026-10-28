use std::{cell::RefCell, rc::Rc};

use super::{BridgeIdentityStore, JsContextHost, ReflectorId};
use crate::{document_runtime::DomHandle, util};

const ANCHOR_SLOT: &str = "__moliNativeNodeOwnership";
const OWNER_SLOT: &str = "__moliNativeNodeCurrentOwner";
const HANDLE_SLOT: &str = "__moliNativeNodeCurrentHandle";
const DOCUMENT_SLOT: &str = "__moliNativeNodeOwnerDocument";
const REGISTRIES_SLOT: &str = "__moliNativeNodeRegistryReferences";

pub(in crate::native_bridge) fn resolve_object_node_ownership(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let object = v8::Local::new(scope, object);
    let anchor = util::get_private_object(scope, object, ANCHOR_SLOT)?;
    let handle = util::get_private_value(scope, anchor, HANDLE_SLOT)?;
    let handle = v8::Local::<v8::BigInt>::try_from(handle).ok()?;
    let (handle, lossless) = handle.u64_value();
    let handle = lossless.then(|| usize::try_from(handle).ok()).flatten()?;
    let owner = util::get_private_object(scope, anchor, OWNER_SLOT).unwrap_or(anchor);
    let owner = owner.get_creation_context(scope)?;
    Some((
        util::context_host_ptr_from_context_slot(owner)?,
        DomHandle::new(handle),
    ))
}

/// Native location shared by every retained view of a node. The Rust record
/// has only weak V8 edges. Wrappers and collections trace the anchor, which in
/// turn traces the current owning Document realm through a private property.
/// Moving a node updates this one location without changing its JS identity,
/// prototype, or the host-local IDs stored in an older collection snapshot.
#[derive(Debug)]
pub(in crate::native_bridge) struct NodeOwnership {
    location: RefCell<NodeLocation>,
}

#[derive(Debug)]
struct NodeLocation {
    handle: DomHandle,
    owner: v8::Weak<v8::Context>,
    anchor: v8::Weak<v8::Object>,
}

impl NodeOwnership {
    pub(super) fn existing_anchor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.location.borrow().anchor.to_local(scope)
    }

    pub(super) fn trace_document(&self, scope: &mut v8::PinScope<'_, '_>, document: &Self) {
        let anchor = self
            .anchor(scope)
            .expect("live node has an ownership anchor");
        let document = document
            .anchor(scope)
            .expect("live Document has an ownership anchor");
        util::set_private_value(scope, anchor, DOCUMENT_SLOT, document.into());
    }

    pub(super) fn trace_registries(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        values: &[v8::Local<'_, v8::Value>],
    ) {
        let Some(anchor) = self.existing_anchor(scope) else {
            return;
        };
        let value = if values.is_empty() {
            v8::undefined(scope).into()
        } else {
            v8::Array::new_with_elements(scope, values).into()
        };
        util::set_private_value(scope, anchor, REGISTRIES_SLOT, value);
    }
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        owner: v8::Local<'_, v8::Context>,
        handle: DomHandle,
    ) -> Self {
        let scope = &mut v8::ContextScope::new(scope, owner);
        let anchor = util::new_null_prototype_object(scope);
        let value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
        util::set_private_value(scope, anchor, HANDLE_SLOT, value.into());
        Self {
            location: RefCell::new(NodeLocation {
                handle,
                owner: v8::Weak::new(scope, owner),
                anchor: v8::Weak::new(scope, anchor),
            }),
        }
    }

    pub(in crate::native_bridge) fn resolve(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<(*mut JsContextHost, DomHandle)> {
        let location = self.location.borrow();
        let owner = location.owner.to_local(scope)?;
        let host = util::context_host_ptr_from_context_slot(owner)?;
        Some((host, location.handle))
    }

    pub(in crate::native_bridge) fn anchor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let mut location = self.location.borrow_mut();
        if let Some(anchor) = location.anchor.to_local(scope) {
            return Some(anchor);
        }
        let owner = location.owner.to_local(scope)?;
        let scope = &mut v8::ContextScope::new(scope, owner);
        let anchor = util::new_null_prototype_object(scope);
        let value = v8::BigInt::new_from_u64(scope, location.handle.index() as u64);
        util::set_private_value(scope, anchor, HANDLE_SLOT, value.into());
        location.anchor = v8::Weak::new(scope, anchor);
        let handle = location.handle;
        drop(location);
        if let Some(host) = util::context_host_ptr_from_context_slot(owner) {
            unsafe { &*host }
                .custom_element_registry_references_version
                .set(None);
            if unsafe { &*host }
                .custom_element_registry_references_enabled
                .get()
            {
                let document = unsafe { &*host }
                    .dom_host()
                    .owner_document_handle(handle)
                    .filter(|document| *document != handle)
                    .and_then(|document| {
                        unsafe { &*host }
                            .bridge
                            .identity
                            .node_ownership
                            .get(&document)
                            .cloned()
                    });
                if let Some(document) = document.and_then(|document| document.anchor(scope)) {
                    util::set_private_value(scope, anchor, DOCUMENT_SLOT, document.into());
                }
                let values =
                    super::registry_references::registry_values_for_subtree(scope, host, handle);
                if !values.is_empty() {
                    let values = v8::Array::new_with_elements(scope, &values);
                    util::set_private_value(scope, anchor, REGISTRIES_SLOT, values.into());
                }
            }
        }
        Some(anchor)
    }

    pub(in crate::native_bridge) fn bind(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        object: v8::Local<'_, v8::Object>,
    ) {
        let anchor = self
            .anchor(scope)
            .expect("live native node has an owner realm");
        util::set_private_value(scope, object, ANCHOR_SLOT, anchor.into());
    }

    pub(in crate::native_bridge) fn retarget(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: v8::Local<'_, v8::Context>,
        handle: DomHandle,
    ) {
        // A retained anchor can have been created in an earlier Page. Give it
        // a V8-traced edge to the new owner before replacing the weak location.
        let mut location = self.location.borrow_mut();
        if let Some(anchor) = location.anchor.to_local(scope) {
            // The new native tree determines registry and Document edges.
            // Do not retain the previous Document through a transferred view.
            if util::get_private_object(scope, anchor, DOCUMENT_SLOT).is_some()
                || util::get_private_object(scope, anchor, REGISTRIES_SLOT).is_some()
            {
                util::set_private_value(scope, anchor, DOCUMENT_SLOT, v8::undefined(scope).into());
                util::set_private_value(
                    scope,
                    anchor,
                    REGISTRIES_SLOT,
                    v8::undefined(scope).into(),
                );
            }
            let scope = &mut v8::ContextScope::new(scope, owner);
            let owner_anchor = util::new_null_prototype_object(scope);
            util::set_private_value(scope, anchor, OWNER_SLOT, owner_anchor.into());
            let value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
            util::set_private_value(scope, anchor, HANDLE_SLOT, value.into());
        }
        location.handle = handle;
        location.owner = v8::Weak::new(scope, owner);
    }
}

impl BridgeIdentityStore {
    pub(in crate::native_bridge) fn transfer_node_ownership_from(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        source: &mut Self,
        source_host: *mut JsContextHost,
        owner: v8::Local<'_, v8::Context>,
        handles: &std::collections::HashMap<DomHandle, DomHandle>,
    ) {
        // Capture existing canonical objects before retargeting. Their internal
        // reflector ID still belongs to the original host; its shared location
        // resolves to the new owner. No public getter or setter participates.
        // Adoption can run in a different world from every retained wrapper.
        // Move each world's canonical entries into that same world's target
        // partition, including wrappers first created by another native Page.
        let mut worlds = vec![None];
        worlds.extend(
            source
                .isolated_wrapper_worlds
                .borrow()
                .iter()
                .filter_map(|world| world.upgrade())
                .map(Some),
        );
        let mut captured = Vec::new();
        for world in worlds {
            let cache = match &world {
                None => source.default_world_wrapper_cache.clone(),
                Some(world) => {
                    let Some(cache) = world.hosts.borrow().get(&source.cache_host_id).cloned()
                    else {
                        continue;
                    };
                    cache
                }
            };
            let cache = cache.borrow();
            let wrappers = source
                .reflector_handles
                .iter()
                .enumerate()
                .filter_map(|(index, descriptor)| {
                    let old = descriptor.node_handle()?;
                    let new = *handles.get(&old)?;
                    let wrapper = cache.wrapper(scope, ReflectorId::from_index(index))?;
                    Some((
                        descriptor.clone().with_node_handle(new),
                        v8::Global::new(scope, wrapper),
                    ))
                })
                .collect::<Vec<_>>();
            let mut collections = Vec::new();
            for (descriptor, entry) in &cache.live_collection_wrappers {
                if let Some(&root) = handles.get(&descriptor.root)
                    && let Some(wrapper) = entry.wrapper.to_local(scope)
                {
                    let mut descriptor = descriptor.clone();
                    *descriptor.resolution_cache.0.borrow_mut() = None;
                    descriptor.root = root;
                    collections.push((descriptor, v8::Global::new(scope, wrapper)));
                }
            }
            for (descriptor, entry) in &cache.retired_live_collection_wrappers {
                if let Some(&root) = handles.get(&descriptor.root)
                    && let Some(wrapper) = entry.to_local(scope)
                {
                    let mut descriptor = descriptor.clone();
                    *descriptor.resolution_cache.0.borrow_mut() = None;
                    descriptor.root = root;
                    collections.push((descriptor, v8::Global::new(scope, wrapper)));
                }
            }
            captured.push((world, wrappers, collections));
        }
        for (&old, &new) in handles {
            let ownership = source.node_ownership(scope, source_host, old);
            ownership.retarget(scope, owner, new);
            assert!(
                self.node_ownership.insert(new, ownership).is_none(),
                "transferred native IDs are never reused"
            );
        }
        for (world, wrappers, collections) in captured {
            let cache = match world {
                None => self.default_world_wrapper_cache.clone(),
                Some(world) => self.isolated_wrapper_cache(&world),
            };
            let mut cache = cache.borrow_mut();
            for (descriptor, wrapper) in wrappers {
                let id = self.reflector_id(&descriptor);
                cache.cache_wrapper(scope, id, v8::Local::new(scope, &wrapper));
            }
            for (descriptor, wrapper) in collections {
                cache.cache_live_collection_wrapper(
                    scope,
                    descriptor,
                    v8::Local::new(scope, &wrapper),
                );
            }
        }
        // A retained descriptor not materialized in this world must also lose
        // its source-host query cache, whose version can collide with the target.
        for descriptor in source.live_collections.descriptors.values() {
            if handles.contains_key(&descriptor.root) {
                *descriptor.resolution_cache.0.borrow_mut() = None;
            }
        }
        unsafe { &*source_host }
            .custom_element_registry_references_version
            .set(None);
        source.synchronize_registry_references(scope, source_host);
    }

    pub(in crate::native_bridge) fn node_ownership(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut JsContextHost,
        handle: DomHandle,
    ) -> Rc<NodeOwnership> {
        if let Some(ownership) = self.node_ownership.get(&handle) {
            return ownership.clone();
        }
        let owner = self
            .node_owner_context(scope, host)
            .expect("native node owner has a Document realm");
        self.node_owner_realm = Some(v8::Weak::new(scope, owner));
        let ownership = Rc::new(NodeOwnership::new(scope, owner, handle));
        self.node_ownership.insert(handle, ownership.clone());
        if unsafe { &*host }
            .custom_element_registry_references_enabled
            .get()
        {
            if let Some(document) = unsafe { &*host }.dom_host().owner_document_handle(handle)
                && document != handle
            {
                let document = self.node_ownership(scope, host, document);
                ownership.trace_document(scope, &document);
            }
            let values =
                super::registry_references::registry_values_for_subtree(scope, host, handle);
            ownership.trace_registries(scope, &values);
        }
        ownership
    }

    pub(in crate::native_bridge) fn node_owner_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: *mut JsContextHost,
    ) -> Option<v8::Local<'s, v8::Context>> {
        // Native ownership belongs to the Page, independent of which isolated
        // world first materializes a wrapper. Keep only a weak realm reference
        // for retained nodes after the Page clears its active default context.
        unsafe { &*host }
            .page_default_context(scope)
            .or_else(|| {
                self.node_owner_realm
                    .as_ref()
                    .and_then(|owner| owner.to_local(scope))
            })
            .or_else(|| {
                let current = scope.get_current_context();
                (util::context_host_ptr_from_context_slot(current) == Some(host)).then_some(current)
            })
    }

    pub(in crate::native_bridge) fn resolve_node_ownership(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<(*mut JsContextHost, DomHandle)> {
        match self.node_ownership.get(&handle) {
            Some(ownership) => ownership.resolve(scope),
            None => Some((host, handle)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_node_traces_its_current_owner_and_releases_previous_and_final_owners() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let (ownership, wrapper, first_owner, current_owner) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let original = v8::Context::new(scope, Default::default());
            let first = v8::Context::new(scope, Default::default());
            let current = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, original);
            let wrapper = util::new_null_prototype_object(scope);
            let ownership = NodeOwnership::new(scope, original, DomHandle::new(5));
            ownership.bind(scope, wrapper);
            ownership.retarget(scope, first, DomHandle::new(105));
            ownership.retarget(scope, current, DomHandle::new(205));
            assert_eq!(wrapper.get_creation_context(scope), Some(original));
            let anchor = util::get_private_object(scope, wrapper, ANCHOR_SLOT).unwrap();
            let handle = util::get_private_value(scope, anchor, HANDLE_SLOT).unwrap();
            assert_eq!(
                v8::Local::<v8::BigInt>::try_from(handle)
                    .unwrap()
                    .u64_value(),
                (205, true)
            );
            (
                ownership,
                v8::Global::new(scope, wrapper),
                v8::Weak::new(scope, first),
                v8::Weak::new(scope, current),
            )
        };
        isolate.low_memory_notification();
        assert!(
            first_owner.is_empty(),
            "retargeting must release the intermediate Document realm"
        );
        assert!(
            !current_owner.is_empty(),
            "a retained wrapper traces its current native owner"
        );
        drop(wrapper);
        isolate.low_memory_notification();
        assert!(
            current_owner.is_empty(),
            "Rust ownership metadata must not root a released Document realm"
        );
        drop(ownership);
    }
}
