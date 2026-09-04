use std::sync::Arc;

use indexmap::IndexSet;
use moli_selector::{
    StyloElementDependencySnapshot as StyleElementDependencySnapshot,
    stylo_removed_element_dependency_snapshots as removed_element_dependency_snapshots,
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, DomMutationEffects, Node},
};

const REMOVED_SUBTREE_SNAPSHOT_NODE_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum StyleMutationEffect {
    Attribute {
        element: DomHandle,
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
        removed_element_snapshots: Vec<StyleElementDependencySnapshot>,
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    },
}

impl StyleMutationEffect {
    #[cfg(test)]
    pub(crate) fn attribute_for_element_ns(
        host: &DomHost,
        element: DomHandle,
        namespace: Option<&str>,
        name: &str,
        old_value: Option<String>,
        new_value: Option<String>,
    ) -> Self {
        Self::Attribute {
            element,
            name: if namespace.is_some() {
                name.to_owned()
            } else {
                normalized_style_attribute_name(host, element, name)
            },
            old_value,
            new_value,
        }
    }

    pub(crate) fn from_dom_mutation_effects(
        host: &DomHost,
        effects: &DomMutationEffects,
    ) -> Vec<Self> {
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
                removed_element_snapshots: removed_element_dependency_snapshots_for_mutation(
                    host,
                    mutation.removed_nodes(),
                ),
                previous_sibling: mutation.previous_sibling(),
                next_sibling: mutation.next_sibling(),
            });
        }
        for mutation in effects.style().attribute_mutations() {
            let normalized_name = if mutation.namespace().is_some() {
                mutation.local_name().to_owned()
            } else {
                normalized_style_attribute_name(host, mutation.target(), mutation.local_name())
            };
            style_effects.insert(Self::Attribute {
                element: mutation.target(),
                name: normalized_name,
                old_value: mutation.old_value().map(str::to_owned),
                new_value: mutation.new_value().map(str::to_owned),
            });
        }
        style_effects.into_iter().collect()
    }

    pub(super) fn attribute_dependency_change(&self) -> Option<StyleAttributeDependencyChange<'_>> {
        let Self::Attribute {
            name,
            old_value,
            new_value,
            ..
        } = self
        else {
            return None;
        };
        Some(StyleAttributeDependencyChange::new(
            name,
            old_value.as_deref(),
            new_value.as_deref(),
        ))
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

/// Independent consequences of mutating a content attribute.
///
/// A compact flag set keeps combinations explicit without growing an enum
/// variant for every attribute that participates in multiple subsystems.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StyleAttributeImpact(u8);

impl StyleAttributeImpact {
    const LAYOUT_METRIC: Self = Self(1 << 0);
    const COMPUTED_STYLE: Self = Self(1 << 1);
    const DESCENDANT_COMPUTED_STYLE: Self = Self(1 << 2);
    const STYLESHEET_LINKAGE: Self = Self(1 << 3);

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub(crate) fn for_attribute_name(name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "style" | "class" | "id" => Self::COMPUTED_STYLE,
            "width" | "height" => Self::LAYOUT_METRIC.union(Self::COMPUTED_STYLE),
            "hidden" | "cols" | "rows" | "size" | "value" | "border" | "slot" | "align" => {
                Self::LAYOUT_METRIC
            }
            "cellpadding" => Self::DESCENDANT_COMPUTED_STYLE,
            "href" | "rel" | "media" | "blocking" | "disabled" => Self::STYLESHEET_LINKAGE,
            // Input type selects runtime behavior and whether width/height
            // participate in the presentation-hint cascade.
            "type" => Self::LAYOUT_METRIC
                .union(Self::COMPUTED_STYLE)
                .union(Self::STYLESHEET_LINKAGE),
            _ if moli_selector::is_svg_presentation_attribute_name(&name) => {
                Self::LAYOUT_METRIC.union(Self::COMPUTED_STYLE)
            }
            _ => Self::default(),
        }
    }

    pub(crate) fn affects_layout_metric(self) -> bool {
        self.intersects(
            Self::LAYOUT_METRIC
                .union(Self::COMPUTED_STYLE)
                .union(Self::DESCENDANT_COMPUTED_STYLE),
        )
    }

    pub(crate) fn has_non_css_runtime_side_effect(self) -> bool {
        self.intersects(
            Self::LAYOUT_METRIC
                .union(Self::DESCENDANT_COMPUTED_STYLE)
                .union(Self::STYLESHEET_LINKAGE),
        )
    }

    #[cfg(test)]
    pub(crate) fn changes_computed_style(self) -> bool {
        self.intersects(Self::COMPUTED_STYLE.union(Self::DESCENDANT_COMPUTED_STYLE))
    }

    #[cfg(test)]
    pub(crate) fn changes_stylesheet_linkage(self) -> bool {
        self.intersects(Self::STYLESHEET_LINKAGE)
    }

    #[cfg(test)]
    pub(crate) fn is_none(self) -> bool {
        self == Self::default()
    }
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

pub(super) fn detached_style_subtree_roots_for_mutations(
    effects: &[StyleMutationEffect],
) -> IndexSet<DomHandle> {
    let mut roots = IndexSet::new();
    for effect in effects {
        match effect {
            StyleMutationEffect::DisconnectedSubtrees {
                roots: disconnected_roots,
            } => {
                roots.extend(disconnected_roots.iter().copied());
            }
            StyleMutationEffect::ChildList { removed_nodes, .. } => {
                roots.extend(removed_nodes.iter().copied());
            }
            StyleMutationEffect::Attribute { .. }
            | StyleMutationEffect::ConnectedSubtrees { .. }
            | StyleMutationEffect::SlotAssignment { .. }
            | StyleMutationEffect::CharacterData { .. } => {}
        }
    }
    roots
}

fn removed_element_dependency_snapshots_for_mutation(
    host: &DomHost,
    removed_nodes: &[DomHandle],
) -> Vec<StyleElementDependencySnapshot> {
    if removed_nodes
        .iter()
        .map(|&root| style_subtree_element_count(host, root))
        .sum::<usize>()
        > REMOVED_SUBTREE_SNAPSHOT_NODE_LIMIT
    {
        return Vec::new();
    }
    removed_element_dependency_snapshots(host, removed_nodes)
}

fn style_subtree_element_count(host: &DomHost, root: DomHandle) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        let Some(node) = host.node(handle) else {
            continue;
        };
        if node.as_element().is_some() {
            count += 1;
        }
        let mut child = node.first_child();
        while let Some(current) = child {
            stack.push(current);
            child = host.next_sibling(current);
        }
    }
    count
}

pub(super) fn style_mutation_effects_are_all_attributes(effects: &[StyleMutationEffect]) -> bool {
    !effects.is_empty()
        && effects
            .iter()
            .all(|effect| matches!(effect, StyleMutationEffect::Attribute { .. }))
}

pub(super) fn style_mutation_effects_are_child_list_structural(
    effects: &[StyleMutationEffect],
) -> bool {
    !effects.is_empty()
        && effects.iter().all(|effect| {
            matches!(
                effect,
                StyleMutationEffect::ChildList { .. }
                    | StyleMutationEffect::ConnectedSubtrees { .. }
                    | StyleMutationEffect::SlotAssignment { .. }
            )
        })
        && effects
            .iter()
            .any(|effect| matches!(effect, StyleMutationEffect::ChildList { .. }))
}

pub(super) fn style_mutation_effects_are_all_character_data(
    effects: &[StyleMutationEffect],
) -> bool {
    !effects.is_empty()
        && effects
            .iter()
            .all(|effect| matches!(effect, StyleMutationEffect::CharacterData { .. }))
}

pub(super) fn style_mutation_effects_are_all_slot_assignments(
    effects: &[StyleMutationEffect],
) -> bool {
    !effects.is_empty()
        && effects
            .iter()
            .all(|effect| matches!(effect, StyleMutationEffect::SlotAssignment { .. }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StyleAttributeDependencyChange<'a> {
    pub(super) attribute_name: &'a str,
    pub(super) removed_class_tokens: Vec<String>,
    pub(super) added_class_tokens: Vec<String>,
    pub(super) removed_id: Option<String>,
    pub(super) added_id: Option<String>,
}

impl<'a> StyleAttributeDependencyChange<'a> {
    fn new(attribute_name: &'a str, old_value: Option<&str>, new_value: Option<&str>) -> Self {
        let (removed_class_tokens, added_class_tokens) = if attribute_name == "class" {
            changed_ascii_whitespace_tokens(old_value, new_value)
        } else {
            (Vec::new(), Vec::new())
        };
        let (removed_id, added_id) = if attribute_name == "id" {
            changed_identifier(old_value, new_value)
        } else {
            (None, None)
        };
        Self {
            attribute_name,
            removed_class_tokens,
            added_class_tokens,
            removed_id,
            added_id,
        }
    }
}

fn changed_ascii_whitespace_tokens(
    old_value: Option<&str>,
    new_value: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let old_tokens = ascii_whitespace_token_set(old_value.unwrap_or_default());
    let new_tokens = ascii_whitespace_token_set(new_value.unwrap_or_default());
    let removed = old_tokens
        .iter()
        .filter(|token| !new_tokens.contains(*token))
        .cloned()
        .collect();
    let added = new_tokens
        .iter()
        .filter(|token| !old_tokens.contains(*token))
        .cloned()
        .collect();
    (removed, added)
}

fn ascii_whitespace_token_set(value: &str) -> IndexSet<String> {
    value
        .split_ascii_whitespace()
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn changed_identifier(
    old_value: Option<&str>,
    new_value: Option<&str>,
) -> (Option<String>, Option<String>) {
    if old_value == new_value {
        return (None, None);
    }
    (
        old_value
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        new_value
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    )
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
        let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
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
}
