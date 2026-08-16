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
    ConnectedSubtree {
        root: DomHandle,
    },
    DisconnectedSubtree {
        root: DomHandle,
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
        for &root in effects.tree().connected_roots() {
            style_effects.insert(Self::ConnectedSubtree { root });
        }
        for &root in effects.scripts().connected_roots() {
            style_effects.insert(Self::ConnectedSubtree { root });
        }
        for &root in effects.tree().disconnected_roots() {
            style_effects.insert(Self::DisconnectedSubtree { root });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StyleAttributeImpact {
    None,
    LayoutMetric,
    ComputedStyle,
    DescendantComputedStyle,
    StylesheetLinkage,
    LayoutMetricAndStylesheetLinkage,
}

impl StyleAttributeImpact {
    pub(crate) fn for_attribute_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "style" | "class" | "id" => Self::ComputedStyle,
            "hidden" | "width" | "height" | "cols" | "rows" | "size" | "value" | "border"
            | "slot" | "align" => Self::LayoutMetric,
            "cellpadding" => Self::DescendantComputedStyle,
            "href" | "rel" | "media" | "blocking" | "disabled" => Self::StylesheetLinkage,
            "type" => Self::LayoutMetricAndStylesheetLinkage,
            _ => Self::None,
        }
    }

    pub(crate) fn affects_layout_metric(self) -> bool {
        matches!(
            self,
            Self::LayoutMetric
                | Self::ComputedStyle
                | Self::DescendantComputedStyle
                | Self::LayoutMetricAndStylesheetLinkage
        )
    }

    #[cfg(test)]
    pub(crate) fn changes_computed_style(self) -> bool {
        matches!(self, Self::ComputedStyle | Self::DescendantComputedStyle)
    }

    #[cfg(test)]
    pub(crate) fn changes_stylesheet_linkage(self) -> bool {
        matches!(
            self,
            Self::StylesheetLinkage | Self::LayoutMetricAndStylesheetLinkage
        )
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
            StyleMutationEffect::DisconnectedSubtree { root } => {
                roots.insert(*root);
            }
            StyleMutationEffect::ChildList { removed_nodes, .. } => {
                roots.extend(removed_nodes.iter().copied());
            }
            StyleMutationEffect::Attribute { .. }
            | StyleMutationEffect::ConnectedSubtree { .. }
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
                    | StyleMutationEffect::ConnectedSubtree { .. }
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
