use moli_page_types::DocumentNodeSnapshot;

use crate::dom::native::{DomHost, DomMutationEffects, DomMutationRecordKind, NativeNodeId};

type DomHandle = NativeNodeId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DevToolsDomMutationFact {
    Attribute {
        target: DomHandle,
        name: String,
        value: Option<String>,
    },
    CharacterData {
        target: DomHandle,
        is_text: bool,
        old_value: String,
        value: String,
        parent: Option<DomHandle>,
        previous_sibling_without_whitespace: Option<DomHandle>,
        final_parent_child_count_without_whitespace: Option<usize>,
        snapshot: Box<DocumentNodeSnapshot>,
    },
    ChildList(DevToolsDomChildListMutationFact),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevToolsDomChildListMutationFact {
    pub(crate) target: DomHandle,
    pub(crate) added_nodes: Vec<DevToolsDomInsertedNodeFact>,
    pub(crate) removed_subtrees: Vec<DevToolsDomRemovedSubtreeFact>,
    pub(crate) previous_sibling: Option<DomHandle>,
    pub(crate) previous_sibling_without_whitespace: Option<DomHandle>,
    pub(crate) final_child_count: usize,
    pub(crate) final_child_count_without_whitespace: usize,
    prepublished_removals: Vec<DevToolsDomPrepublishedRemoval>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevToolsDomPrepublishedRemoval {
    inspector_session_id: Option<String>,
    parent: DomHandle,
    removed_root: DomHandle,
}

impl DevToolsDomPrepublishedRemoval {
    pub(crate) fn new(
        inspector_session_id: Option<String>,
        parent: DomHandle,
        removed_root: DomHandle,
    ) -> Self {
        Self {
            inspector_session_id,
            parent,
            removed_root,
        }
    }
}

impl DevToolsDomChildListMutationFact {
    pub(crate) fn removal_was_prepublished_for_session(
        &self,
        inspector_session_id: Option<&str>,
        removed_root: DomHandle,
    ) -> bool {
        self.prepublished_removals.iter().any(|removal| {
            removal.inspector_session_id.as_deref() == inspector_session_id
                && removal.removed_root == removed_root
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevToolsDomInsertedNodeFact {
    pub(crate) snapshot: DocumentNodeSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevToolsDomRemovedSubtreeFact {
    pub(crate) handles: Vec<DomHandle>,
    pub(crate) root_is_whitespace_text: bool,
}

pub(crate) fn capture_devtools_dom_mutation_facts(
    dom_host: &DomHost,
    effects: &DomMutationEffects,
) -> Vec<DevToolsDomMutationFact> {
    if !dom_host.devtools_mutation_records_enabled() {
        return Vec::new();
    }

    effects
        .observer_records()
        .records()
        .iter()
        .filter_map(|record| match record.kind() {
            DomMutationRecordKind::Attributes(mutation) => {
                if mutation.old_value() == mutation.new_value() {
                    return None;
                }
                Some(DevToolsDomMutationFact::Attribute {
                    target: mutation.target(),
                    name: dom_host
                        .normalized_attribute_name(mutation.target(), mutation.local_name())
                        .unwrap_or_else(|| mutation.local_name().to_owned()),
                    value: mutation.new_value().map(str::to_owned),
                })
            }
            DomMutationRecordKind::CharacterData { old_value } => {
                let value = dom_host.node(record.target())?.node_value()?.to_owned();
                debug_assert!(
                    old_value.is_some(),
                    "DevTools character-data mutation records must retain old_value; missing \
                     old_value can silently drop Inspector DOM whitespace visibility transitions"
                );
                let parent = dom_host.parent_node(record.target());
                let snapshot = crate::runtime::page_dom::live_document_node_snapshot(
                    dom_host,
                    record.target(),
                    0,
                    parent,
                    false,
                )?;
                Some(DevToolsDomMutationFact::CharacterData {
                    target: record.target(),
                    is_text: dom_host.node(record.target()).is_some_and(|node| {
                        matches!(node.kind(), crate::dom::native::NodeData::Text(_))
                    }),
                    old_value: old_value.clone().unwrap_or_else(|| value.clone()),
                    value,
                    parent,
                    previous_sibling_without_whitespace: previous_non_whitespace_sibling(
                        dom_host,
                        dom_host.previous_sibling(record.target()),
                    ),
                    final_parent_child_count_without_whitespace: parent.map(|parent| {
                        dom_host
                            .child_handles(parent)
                            .filter(|&handle| {
                                !crate::runtime::page_dom::inspector_whitespace_text_node(
                                    dom_host, handle,
                                )
                            })
                            .count()
                    }),
                    snapshot: Box::new(snapshot),
                })
            }
            DomMutationRecordKind::ChildList(mutation) => {
                let added_nodes = mutation
                    .added_nodes()
                    .iter()
                    .filter_map(|&handle| {
                        crate::runtime::page_dom::live_document_node_snapshot(
                            dom_host,
                            handle,
                            0,
                            Some(mutation.target()),
                            false,
                        )
                        .map(|snapshot| DevToolsDomInsertedNodeFact { snapshot })
                    })
                    .collect();
                let removed_subtrees = mutation
                    .removed_nodes()
                    .iter()
                    .map(|&handle| DevToolsDomRemovedSubtreeFact {
                        handles: collect_light_tree_subtree_handles(dom_host, handle),
                        root_is_whitespace_text:
                            crate::runtime::page_dom::inspector_whitespace_text_node(
                                dom_host, handle,
                            ),
                    })
                    .collect();
                let previous_sibling_without_whitespace =
                    previous_non_whitespace_sibling(dom_host, mutation.previous_sibling());
                Some(DevToolsDomMutationFact::ChildList(
                    DevToolsDomChildListMutationFact {
                        target: mutation.target(),
                        added_nodes,
                        removed_subtrees,
                        previous_sibling: mutation.previous_sibling(),
                        previous_sibling_without_whitespace,
                        final_child_count: dom_host.child_handles(mutation.target()).count(),
                        final_child_count_without_whitespace: dom_host
                            .child_handles(mutation.target())
                            .filter(|&handle| {
                                !crate::runtime::page_dom::inspector_whitespace_text_node(
                                    dom_host, handle,
                                )
                            })
                            .count(),
                        prepublished_removals: Vec::new(),
                    },
                ))
            }
        })
        .collect()
}

pub(crate) fn attach_prepublished_removals(
    facts: &mut [DevToolsDomMutationFact],
    removals: impl IntoIterator<Item = DevToolsDomPrepublishedRemoval>,
) {
    for removal in removals {
        let Some(mutation) = facts.iter_mut().find_map(|fact| match fact {
            DevToolsDomMutationFact::ChildList(mutation)
                if mutation.target == removal.parent
                    && mutation
                        .removed_subtrees
                        .iter()
                        .any(|subtree| subtree.handles.first() == Some(&removal.removed_root)) =>
            {
                Some(mutation)
            }
            _ => None,
        }) else {
            continue;
        };
        if !mutation.prepublished_removals.contains(&removal) {
            mutation.prepublished_removals.push(removal);
        }
    }
}

fn previous_non_whitespace_sibling(
    dom_host: &DomHost,
    mut sibling: Option<DomHandle>,
) -> Option<DomHandle> {
    while let Some(handle) = sibling {
        if !crate::runtime::page_dom::inspector_whitespace_text_node(dom_host, handle) {
            return Some(handle);
        }
        sibling = dom_host.previous_sibling(handle);
    }
    None
}

fn collect_light_tree_subtree_handles(dom_host: &DomHost, root: DomHandle) -> Vec<DomHandle> {
    let mut handles = Vec::new();
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        handles.push(handle);
        stack.extend(dom_host.child_handles(handle));
        if let Some(shadow_root) = dom_host.shadow_root_handle(handle) {
            stack.push(shadow_root);
        }
    }
    handles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepublished_removal_is_scoped_to_its_inspector_session() {
        let parent = DomHandle::new(1);
        let removed_root = DomHandle::new(2);
        let mut facts = vec![DevToolsDomMutationFact::ChildList(
            DevToolsDomChildListMutationFact {
                target: parent,
                added_nodes: Vec::new(),
                removed_subtrees: vec![DevToolsDomRemovedSubtreeFact {
                    handles: vec![removed_root],
                    root_is_whitespace_text: false,
                }],
                previous_sibling: None,
                previous_sibling_without_whitespace: None,
                final_child_count: 0,
                final_child_count_without_whitespace: 0,
                prepublished_removals: Vec::new(),
            },
        )];

        attach_prepublished_removals(
            &mut facts,
            [DevToolsDomPrepublishedRemoval::new(
                Some("owner".to_owned()),
                parent,
                removed_root,
            )],
        );

        let DevToolsDomMutationFact::ChildList(mutation) = &facts[0] else {
            panic!("fixture must remain a child-list mutation");
        };
        assert!(mutation.removal_was_prepublished_for_session(Some("owner"), removed_root));
        assert!(!mutation.removal_was_prepublished_for_session(Some("peer"), removed_root));
        assert!(!mutation.removal_was_prepublished_for_session(None, removed_root));
    }
}
