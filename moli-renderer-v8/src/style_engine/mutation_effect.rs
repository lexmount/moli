use std::sync::Arc;

use indexmap::IndexSet;
use smallvec::SmallVec;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, DomMutationEffects, Node},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum StyleMutationEffect {
    Attribute {
        element: DomHandle,
        namespace: Option<Arc<str>>,
        name: String,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    ConnectedSubtrees {
        roots: Arc<[DomHandle]>,
    },
    DisconnectedSubtrees {
        roots: Arc<[DomHandle]>,
    },
    SlotAssignment {
        slot: DomHandle,
        previous_assigned_nodes: Option<Vec<DomHandle>>,
        assigned_nodes: Option<Vec<DomHandle>>,
    },
    CharacterData {
        node: DomHandle,
    },
    ChildList {
        parent: DomHandle,
        added_nodes: Vec<DomHandle>,
        removed_nodes: Vec<DomHandle>,
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    },
}

impl StyleMutationEffect {
    pub(super) fn detached_roots(&self) -> &[DomHandle] {
        match self {
            Self::DisconnectedSubtrees { roots } => roots,
            Self::ChildList { removed_nodes, .. } => removed_nodes,
            _ => &[],
        }
    }

    pub(crate) fn touches_connected_tree(&self, host: &DomHost) -> bool {
        match self {
            Self::Attribute { element: node, .. }
            | Self::CharacterData { node }
            | Self::ChildList { parent: node, .. }
            | Self::SlotAssignment { slot: node, .. } => host.is_connected(*node),
            Self::ConnectedSubtrees { roots } | Self::DisconnectedSubtrees { roots } => {
                roots.iter().any(|&root| host.is_connected(root))
            }
        }
    }

    pub(crate) fn affects_layout_metric(&self, host: &DomHost) -> bool {
        let Self::Attribute {
            element,
            namespace,
            name,
            ..
        } = self
        else {
            return true;
        };
        // Native geometry and commonly changed CSS inputs can affect cached
        // layout independently of the element's presentational hints.
        matches!(
            name.as_str(),
            "style"
                | "class"
                | "id"
                | "width"
                | "height"
                | "cols"
                | "rows"
                | "size"
                | "value"
                | "border"
                | "slot"
                | "type"
        ) || moli_selector::attribute_affects_presentational_hints(
            host,
            *element,
            namespace.as_deref(),
            name,
        )
    }

    pub(crate) fn from_dom_mutation_effects(effects: &DomMutationEffects) -> SmallVec<[Self; 1]> {
        // A single text update needs neither deduplication nor a heap buffer.
        let style = effects.style();
        if let [node] = style.character_data_mutations()
            && style.attribute_mutations().is_empty()
            && style.child_list_mutations().is_empty()
            && effects.tree().connected_roots().is_empty()
            && effects.tree().disconnected_roots().is_empty()
            && effects.slots().assignment_changes().is_empty()
            && effects.slots().changed_slots().is_empty()
        {
            return SmallVec::from_buf([Self::CharacterData { node: *node }]);
        }
        let mut style_effects = IndexSet::new();
        let connected_roots = effects.tree().connected_roots();
        #[cfg(debug_assertions)]
        {
            let connected_root_set = connected_roots
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>();
            debug_assert!(
                effects
                    .scripts()
                    .connected_roots()
                    .iter()
                    .all(|root| connected_root_set.contains(root)),
                "script connection roots must also be tree connection roots"
            );
        }
        if !connected_roots.is_empty() {
            style_effects.insert(Self::ConnectedSubtrees {
                roots: shared_tree_roots(
                    connected_roots,
                    effects
                        .style()
                        .child_list_mutations()
                        .iter()
                        .map(|mutation| (mutation.added_nodes(), mutation.shared_added_nodes())),
                ),
            });
        }
        let disconnected_roots = effects.tree().disconnected_roots();
        if !disconnected_roots.is_empty() {
            style_effects.insert(Self::DisconnectedSubtrees {
                roots: shared_tree_roots(
                    disconnected_roots,
                    effects
                        .style()
                        .child_list_mutations()
                        .iter()
                        .map(|mutation| {
                            (mutation.removed_nodes(), mutation.shared_removed_nodes())
                        }),
                ),
            });
        }
        let detailed_slot_assignment_slots = effects
            .slots()
            .assignment_changes()
            .iter()
            .map(|change| change.slot())
            .collect::<IndexSet<_>>();
        for change in effects.slots().assignment_changes() {
            style_effects.insert(Self::SlotAssignment {
                slot: change.slot(),
                previous_assigned_nodes: Some(change.previous_assigned_nodes().to_vec()),
                assigned_nodes: Some(change.assigned_nodes().to_vec()),
            });
        }
        for &slot in effects.slots().changed_slots() {
            if detailed_slot_assignment_slots.contains(&slot) {
                continue;
            }
            style_effects.insert(Self::SlotAssignment {
                slot,
                previous_assigned_nodes: None,
                assigned_nodes: None,
            });
        }
        for &node in effects.style().character_data_mutations() {
            style_effects.insert(Self::CharacterData { node });
        }
        for mutation in effects.style().child_list_mutations() {
            style_effects.insert(Self::ChildList {
                parent: mutation.target(),
                added_nodes: mutation.added_nodes().to_vec(),
                removed_nodes: mutation.removed_nodes().to_vec(),
                previous_sibling: mutation.previous_sibling(),
                next_sibling: mutation.next_sibling(),
            });
        }
        for mutation in effects.style().attribute_mutations() {
            style_effects.insert(Self::Attribute {
                element: mutation.target(),
                namespace: mutation.namespace().map(Arc::from),
                name: mutation.local_name().to_owned(),
                old_value: mutation.old_value().map(str::to_owned),
                new_value: mutation.new_value().map(str::to_owned),
            });
        }
        style_effects.into_iter().collect()
    }
}

fn shared_tree_roots<'a>(
    roots: &[DomHandle],
    child_list_roots: impl Iterator<Item = (&'a [DomHandle], Arc<[DomHandle]>)>,
) -> Arc<[DomHandle]> {
    child_list_roots
        .filter_map(|(candidate, shared)| (candidate == roots).then_some(shared))
        .next()
        .unwrap_or_else(|| Arc::from(roots))
}

pub(crate) fn normalized_style_attribute_name(
    host: &DomHost,
    handle: DomHandle,
    name: &str,
) -> String {
    host.node(handle)
        .and_then(Node::as_element)
        .map(|element| element.normalized_attribute_name(name))
        .unwrap_or_else(|| name.to_owned())
}

/// Drop exhausted ordinary fragment history after resolving the full source
/// scope and retiring removed roots. Later mutations can refill the fragment.
pub(super) fn retain_live_structural_mutation_effects(
    host: &DomHost,
    effects: &mut Vec<StyleMutationEffect>,
) {
    effects.retain(|effect| !is_empty_ordinary_fragment_child_list(host, effect));
}

fn is_empty_ordinary_fragment_child_list(host: &DomHost, effect: &StyleMutationEffect) -> bool {
    let StyleMutationEffect::ChildList { parent, .. } = effect else {
        return false;
    };
    host.node(*parent)
        .is_some_and(|node| node.is_document_fragment() && node.first_child().is_none())
        && !host.is_shadow_root(*parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeDom;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ))
    }

    #[test]
    fn fragment_connection_roots_share_one_batched_child_list_payload() {
        let mut host = test_host();
        let document = host.document_handle();
        let parent = host.create_element("main");
        let fragment = host.create_document_fragment();
        let first = host.create_element("div");
        let second = host.create_element("span");
        assert!(host.append_child(document, parent));
        assert!(host.append_child(fragment, first));
        assert!(host.append_child(fragment, second));

        let effects = host.append_child_effects(parent, fragment);
        let child_list = effects
            .style()
            .child_list_mutations()
            .iter()
            .find(|mutation| mutation.target() == parent)
            .expect("fragment insertion should record one child-list mutation");
        let shared_added_nodes = child_list.shared_added_nodes();
        let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
        let connected_batches = style_effects
            .iter()
            .filter_map(|effect| match effect {
                StyleMutationEffect::ConnectedSubtrees { roots } => Some(roots),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(connected_batches.len(), 1);
        assert_eq!(connected_batches[0].as_ref(), [first, second]);
        assert!(Arc::ptr_eq(connected_batches[0], &shared_added_nodes));
    }

    fn collect_native_effects(output: &mut Vec<StyleMutationEffect>, effects: &DomMutationEffects) {
        assert!(effects.did_change());
        output.extend(StyleMutationEffect::from_dom_mutation_effects(effects));
    }

    #[test]
    fn consumed_fragment_preparation_preserves_the_order_of_real_splices() {
        let mut host = test_host();
        let parent = host.create_element("div");
        assert!(host.append_child(host.document_handle(), parent));
        let mut effects = Vec::new();
        let mut fragments = Vec::new();
        // Match textContent + ParentNode.append's native commits, including
        // the staging insertion and fragment consumption as separate facts.
        for _ in 0..64 {
            let child = host.create_element("div");
            let text = host.set_text_content_effects(child, "ABCD");
            collect_native_effects(&mut effects, &text);
            let fragment = host.create_document_fragment();
            let staged = host.append_child_effects(fragment, child);
            collect_native_effects(&mut effects, &staged);
            let consumed = host.append_child_effects(parent, fragment);
            collect_native_effects(&mut effects, &consumed);
            fragments.push(fragment);
        }
        let mut normalized = effects.clone();
        retain_live_structural_mutation_effects(&host, &mut normalized);
        let real_splices = effects
            .iter()
            .filter(|effect| match effect {
                StyleMutationEffect::ChildList { parent, .. } => !fragments.contains(parent),
                _ => false,
            })
            .cloned()
            .collect::<Vec<_>>();
        let kept_splices = normalized
            .iter()
            .filter(|effect| matches!(effect, StyleMutationEffect::ChildList { .. }))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(real_splices.len(), 128);
        assert_eq!(kept_splices, real_splices);
    }

    #[test]
    fn element_shadow_and_refilled_fragment_histories_are_retained() {
        for parent_kind in ["detached-element", "shadow-root", "refilled-fragment"] {
            let mut host = test_host();
            let document = host.document_handle();
            let destination = host.create_element("main");
            assert!(host.append_child(document, destination));
            let old_parent = match parent_kind {
                "detached-element" => host.create_element("section"),
                "shadow-root" => {
                    let shadow_host = host.create_element("div");
                    assert!(host.append_child(document, shadow_host));
                    host.attach_shadow_root(shadow_host, "open").unwrap()
                }
                "refilled-fragment" => host.create_document_fragment(),
                _ => unreachable!(),
            };
            let moved = host.create_element("span");
            let mut effects = Vec::new();
            let staged = host.append_child_effects(old_parent, moved);
            collect_native_effects(&mut effects, &staged);
            let relocated = host.append_child_effects(destination, moved);
            collect_native_effects(&mut effects, &relocated);
            assert!(effects.iter().any(|effect| matches!(
                effect,
                StyleMutationEffect::ChildList { parent, removed_nodes, .. }
                    if *parent == old_parent && removed_nodes == &[moved]
            )));
            let mut normalized = effects.clone();
            if parent_kind == "refilled-fragment" {
                let remaining = host.create_element("span");
                assert!(host.append_child(old_parent, remaining));
            }
            let original_count = normalized.len();
            retain_live_structural_mutation_effects(&host, &mut normalized);
            assert_eq!(normalized.len(), original_count, "{parent_kind}");
        }
    }

    #[test]
    fn consumed_fragment_does_not_erase_an_existing_elements_removal() {
        let mut host = test_host();
        let document = host.document_handle();
        let old_parent = host.create_element("section");
        let destination = host.create_element("main");
        let moved = host.create_element("span");
        assert!(host.append_child(document, old_parent));
        assert!(host.append_child(document, destination));
        assert!(host.append_child(old_parent, moved));
        let fragment = host.create_document_fragment();
        let mut effects = Vec::new();
        let staged = host.append_child_effects(fragment, moved);
        collect_native_effects(&mut effects, &staged);
        let consumed = host.append_child_effects(destination, fragment);
        collect_native_effects(&mut effects, &consumed);
        let mut normalized = effects.clone();
        retain_live_structural_mutation_effects(&host, &mut normalized);
        assert!(normalized.iter().any(|effect| matches!(
            effect,
            StyleMutationEffect::ChildList { parent, removed_nodes, .. }
                if *parent == old_parent && removed_nodes == &[moved]
        )));
        assert!(normalized.iter().any(|effect| matches!(
            effect,
            StyleMutationEffect::ConnectedSubtrees { roots } if roots.contains(&moved)
        )));
        assert!(normalized.iter().any(|effect| matches!(
            effect,
            StyleMutationEffect::DisconnectedSubtrees { roots } if roots.contains(&moved)
        )));
    }
}
