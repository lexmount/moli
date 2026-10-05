//! Project native attribute changes into every exposed number list, including
//! isolated worlds. Weak registrations do not root elements or retired realms.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_dom::native::DomAttributeMutation;

use super::{SvgListKind, builders};
use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, node_runtime_and_handle_from_object_or_detached};

type OwnerKey = (usize, DomHandle);
type Store = Rc<RefCell<NumberLists>>;

#[derive(Clone)]
struct Entry {
    id: u64,
    animated: Rc<v8::Weak<v8::Object>>,
}

#[derive(Default)]
struct NumberLists {
    next_id: u64,
    owners: HashMap<OwnerKey, HashMap<String, Vec<Entry>>>,
}

impl NumberLists {
    fn remove(&mut self, owner: OwnerKey, attribute: &str, id: u64) {
        let Some(attributes) = self.owners.get_mut(&owner) else {
            return;
        };
        let Some(entries) = attributes.get_mut(attribute) else {
            return;
        };
        entries.retain(|entry| entry.id != id);
        if entries.is_empty() {
            attributes.remove(attribute);
        }
        if attributes.is_empty() {
            self.owners.remove(&owner);
        }
    }
}

pub(super) fn register<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    animated: v8::Local<'s, v8::Object>,
) {
    let Ok((host, handle)) = node_runtime_and_handle_from_object_or_detached(scope, owner) else {
        return;
    };
    let owner_key = (host as usize, handle);
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = {
        let mut state = store.borrow_mut();
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("SVG number-list identity exhausted");
        state.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let attribute_name = attribute.to_owned();
    // Never hold a RefCell borrow across V8 operations or weak finalizers.
    let animated = v8::Weak::with_finalizer(
        scope,
        animated,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().remove(owner_key, &attribute_name, id);
            }
        }),
    );
    store
        .borrow_mut()
        .owners
        .entry(owner_key)
        .or_default()
        .entry(attribute.to_owned())
        .or_default()
        .push(Entry {
            id,
            animated: Rc::new(animated),
        });
}

pub(crate) struct NumberListAttributeProjection {
    value: Option<String>,
    entries: Vec<Entry>,
}

pub(crate) fn collect_number_list_attribute_projections(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut JsContextHost,
    mutations: &[DomAttributeMutation],
) -> Vec<NumberListAttributeProjection> {
    let Some(store) = scope.get_slot::<Store>() else {
        return Vec::new();
    };
    let state = store.borrow();
    let mut projections = Vec::new();
    for mutation in mutations {
        if mutation.namespace().is_some() {
            continue;
        }
        let Some(entries) = state
            .owners
            .get(&(host as usize, mutation.target()))
            .and_then(|attributes| attributes.get(mutation.local_name()))
        else {
            continue;
        };
        projections.push(NumberListAttributeProjection {
            value: mutation.new_value().map(str::to_owned),
            entries: entries.clone(),
        });
    }
    projections
}

pub(crate) fn apply_number_list_attribute_projections(
    scope: &mut v8::PinScope<'_, '_>,
    projections: Vec<NumberListAttributeProjection>,
) {
    for projection in projections {
        for entry in projection.entries {
            let Some(animated) = entry.animated.to_local(scope) else {
                continue;
            };
            for member in ["baseVal", "animVal"] {
                if let Some(list) = builders::svg_animated_value_list_member(
                    scope,
                    animated,
                    member,
                    SvgListKind::Number,
                ) {
                    // Use each mutation's value, preserving intermediate detach
                    // transitions even when a native batch restores the old text.
                    builders::sync_svg_value_list_from_attribute_value(
                        scope,
                        list,
                        projection.value.as_deref(),
                        SvgListKind::Number,
                    );
                }
            }
        }
    }
}
