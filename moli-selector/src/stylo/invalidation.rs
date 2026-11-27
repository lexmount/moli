use dom::ElementState;
use indexmap::IndexMap;
use style::{
    LocalName, Namespace, Prefix,
    dom::{TElement, TNode},
    invalidation::element::restyle_hints::RestyleHint,
    selector_parser::{Snapshot, SnapshotMap},
    servo::attr::{AttrIdentifier, AttrValue},
    values::AtomIdent,
};

use crate::dom::{NodeId, native::DomHost};

use super::style_traversal::StyleDomHostBinding;

#[derive(Debug, Default)]
pub struct StyleMutationSnapshots {
    elements: IndexMap<NodeId, ElementSnapshot>,
}

#[derive(Debug, Default)]
struct ElementSnapshot {
    old_state: Option<ElementState>,
    old_custom_states: Option<Vec<String>>,
    old_attributes: IndexMap<(Option<String>, String), Option<String>>,
}

impl StyleMutationSnapshots {
    pub fn record_attribute(
        &mut self,
        element: NodeId,
        namespace: Option<&str>,
        local_name: &str,
        previous: Option<String>,
    ) {
        self.elements
            .entry(element)
            .or_default()
            .old_attributes
            .entry((namespace.map(str::to_owned), local_name.to_owned()))
            .or_insert(previous);
    }

    pub fn record_state(&mut self, element: NodeId, old_state: ElementState) {
        self.elements
            .entry(element)
            .or_default()
            .old_state
            .get_or_insert(old_state);
    }

    pub fn record_custom_states(&mut self, element: NodeId, previous: Vec<String>) {
        self.elements
            .entry(element)
            .or_default()
            .old_custom_states
            .get_or_insert(previous);
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn handles(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.elements.keys().copied()
    }

    pub fn merge(&mut self, incoming: Self) {
        for (handle, incoming) in incoming.elements {
            let target = self.elements.entry(handle).or_default();
            target.old_state = target.old_state.or(incoming.old_state);
            if target.old_custom_states.is_none() {
                target.old_custom_states = incoming.old_custom_states;
            }
            for (attr, value) in incoming.old_attributes {
                target.old_attributes.entry(attr).or_insert(value);
            }
        }
    }
}

impl StyleDomHostBinding<'_> {
    pub fn computed_element_state(&self, host: &DomHost, handle: NodeId) -> Option<ElementState> {
        Some(self.element(host, handle)?.as_query().computed_state())
    }

    pub fn with_mutation_snapshots(
        &self,
        host: &DomHost,
        input: &StyleMutationSnapshots,
        invalidate: impl FnOnce(&SnapshotMap, &[NodeId]),
    ) -> Vec<(NodeId, RestyleHint)> {
        let mut snapshots = SnapshotMap::new();
        let mut handles = Vec::with_capacity(input.elements.len());
        for (&handle, input) in &input.elements {
            if let Some(element) = self.element(host, handle)
                && let Some(snapshot) =
                    input.to_stylo(host, handle, || element.as_query().computed_state())
            {
                snapshots.insert(element.as_node().opaque(), snapshot);
                handles.push(handle);
            }
        }
        self.collect_native_invalidation_targets(|| {
            self.with_snapshot_handles(handles.iter().copied(), || {
                invalidate(&snapshots, &handles)
            });
        })
    }
}

impl ElementSnapshot {
    fn to_stylo(
        &self,
        host: &DomHost,
        handle: NodeId,
        current_state: impl FnOnce() -> ElementState,
    ) -> Option<Snapshot> {
        let element = host.node(handle)?.as_element()?;
        let mut snapshot = Snapshot {
            state: self.old_state,
            custom_states: self.old_custom_states.as_ref().map(|states| {
                states
                    .iter()
                    .map(|state| AtomIdent::from(state.as_str()))
                    .collect()
            }),
            attrs: None,
            changed_attrs: Default::default(),
            class_changed: false,
            id_changed: false,
            other_attributes_changed: false,
        };
        if self.old_attributes.is_empty() {
            return Some(snapshot);
        }
        let mut attrs = element
            .attributes()
            .iter()
            .map(|attr| {
                let id = AttrIdentifier {
                    local_name: LocalName::from(attr.local_name()),
                    name: LocalName::from(attr.name().as_str()),
                    namespace: Namespace::from(attr.namespace()),
                    prefix: attr.prefix().map(Prefix::from),
                };
                let value = snapshot_attribute_value(&id, attr.value().to_owned());
                (id, value)
            })
            .collect::<Vec<_>>();
        for ((namespace, local_name), previous) in &self.old_attributes {
            let namespace = Namespace::from(namespace.as_deref().unwrap_or_default());
            let local_name = LocalName::from(local_name.as_str());
            let index = attrs
                .iter()
                .position(|(id, _)| id.namespace == namespace && id.local_name == local_name);
            if namespace.0.is_empty() {
                snapshot.class_changed |= &*local_name.0 == "class";
                snapshot.id_changed |= &*local_name.0 == "id";
            }
            snapshot.other_attributes_changed |=
                !namespace.0.is_empty() || !matches!(&*local_name.0, "id" | "class");
            snapshot.changed_attrs.push(local_name.clone());

            match (index, previous) {
                (Some(index), Some(previous)) => {
                    let (id, value) = &mut attrs[index];
                    *value = snapshot_attribute_value(id, previous.clone());
                }
                (Some(index), None) => {
                    attrs.remove(index);
                }
                (None, Some(previous)) => {
                    let id = AttrIdentifier {
                        name: local_name.clone(),
                        local_name,
                        namespace,
                        prefix: None,
                    };
                    let value = snapshot_attribute_value(&id, previous.clone());
                    attrs.push((id, value));
                }
                (None, None) => {}
            }
        }
        if self.old_attributes.keys().any(|(_, name)| name == "href") {
            let href = |namespace: &str| {
                attrs
                    .iter()
                    .find(|(id, _)| &*id.local_name.0 == "href" && &*id.namespace.0 == namespace)
            };
            let was_link = element.hyperlink_href_from(href).is_some()
                || (element.is_html_element("link") && href("").is_some());
            let mut state = snapshot.state.unwrap_or_else(current_state);
            state.remove(ElementState::VISITED_OR_UNVISITED);
            state.set(ElementState::UNVISITED, was_link);
            snapshot.state = Some(state);
        }
        snapshot.attrs = Some(attrs);
        Some(snapshot)
    }
}

fn snapshot_attribute_value(id: &AttrIdentifier, value: String) -> AttrValue {
    if id.namespace.0.is_empty() {
        match &*id.local_name.0 {
            "class" | "part" => return AttrValue::from_serialized_tokenlist(value),
            "id" => return AttrValue::from_atomic(value),
            "exportparts" => return AttrValue::from_shadow_parts(value),
            _ => {}
        }
    }
    AttrValue::from(value)
}

pub fn stylo_focus_change_handles(
    host: &DomHost,
    previous: Option<NodeId>,
    next: Option<NodeId>,
) -> impl Iterator<Item = NodeId> {
    let mut handles = indexmap::IndexSet::new();
    for root in [previous, next].into_iter().flatten() {
        let mut current = Some(root);
        while let Some(handle) = current {
            if host
                .node(handle)
                .is_some_and(|node| node.as_element().is_some())
            {
                handles.insert(handle);
            }
            current = style_parent_crossing_shadow_root(host, handle);
        }
    }
    handles.into_iter()
}

pub fn stylo_focus_state_matches_handle(host: &DomHost, focused: NodeId, handle: NodeId) -> bool {
    if focused == handle {
        return true;
    }
    let mut current = focused;
    while let Some(shadow_root) = host.containing_shadow_root(current) {
        let Some(shadow_host) = host.shadow_root_host(shadow_root) else {
            return false;
        };
        if shadow_host == handle {
            return true;
        }
        current = shadow_host;
    }
    false
}

/// Return whether the focused element is inside `handle` for Stylo's
/// `:focus-within` state, crossing shadow-root boundaries.
pub fn stylo_focus_within_state_matches_handle(
    host: &DomHost,
    focused: NodeId,
    handle: NodeId,
) -> bool {
    let mut current = Some(focused);
    while let Some(candidate) = current {
        if candidate == handle {
            return true;
        }
        current = style_parent_crossing_shadow_root(host, candidate);
    }
    false
}

fn style_parent_crossing_shadow_root(host: &DomHost, handle: NodeId) -> Option<NodeId> {
    host.parent_node(handle).or_else(|| {
        if host.is_shadow_root(handle) {
            host.shadow_root_host(handle)
        } else {
            None
        }
    })
}

pub fn stylo_shadow_root_host_participates_in_style_scope(host: &DomHost, root: NodeId) -> bool {
    host.shadow_root_host(root)
        .is_some_and(|shadow_host| stylo_shadow_host_participates_in_style_scope(host, shadow_host))
}

fn stylo_shadow_host_participates_in_style_scope(host: &DomHost, shadow_host: NodeId) -> bool {
    // A stylesheet inside a shadow tree can style that tree when the host is in
    // the active document. Detached/child documents are also independent style
    // scopes, even though their nodes are not connected to the active document.
    host.is_connected(shadow_host)
        || host
            .owner_document_handle(shadow_host)
            .is_some_and(|document| document != host.document_handle())
}
