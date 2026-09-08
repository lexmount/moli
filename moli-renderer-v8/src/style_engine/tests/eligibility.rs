use super::super::eligibility::child_list_effects_have_current_topology;
use super::*;
use crate::dom::native::Node;

fn sibling_fixture() -> (DomHost, DomHandle, [DomHandle; 3]) {
    let mut host = test_host();
    let parent = host.create_element("div");
    assert!(host.append_child(host.document_handle(), parent));
    let children = std::array::from_fn(|_| {
        let child = host.create_element("span");
        assert!(host.append_child(parent, child));
        child
    });
    (host, parent, children)
}

#[test]
fn stable_single_insertions_and_removals_keep_retained_topology() {
    for index in 0..3 {
        let (mut host, parent, children) = sibling_fixture();
        let removed = host.remove_child_effects(parent, children[index]);
        let removed = StyleMutationEffect::from_dom_mutation_effects(&host, &removed);
        assert!(child_list_effects_have_current_topology(&host, &removed));

        let inserted =
            host.insert_before_effects(parent, children[index], children.get(index + 1).copied());
        let inserted = StyleMutationEffect::from_dom_mutation_effects(&host, &inserted);
        assert!(child_list_effects_have_current_topology(&host, &inserted));
        assert!(
            !child_list_effects_have_current_topology(&host, &removed),
            "reinserting the removed node invalidates the earlier snapshot"
        );
    }
}

#[test]
fn consecutive_insertions_keep_retained_topology() {
    let (mut host, parent, [_, b, _]) = sibling_fixture();
    let mut effects = Vec::new();
    for reference in [None, None, Some(b), Some(b)] {
        let added = host.create_element("span");
        let inserted = host.insert_before_effects(parent, added, reference);
        effects.extend(StyleMutationEffect::from_dom_mutation_effects(
            &host, &inserted,
        ));
    }
    assert!(child_list_effects_have_current_topology(&host, &effects));
}

#[test]
fn removal_snapshot_rejects_old_neighbor_moved_to_another_parent() {
    let (mut host, parent, [a, b, _]) = sibling_fixture();
    let removed = host.remove_child_effects(parent, a);
    let removed = StyleMutationEffect::from_dom_mutation_effects(&host, &removed);
    let other = host.create_element("div");
    assert!(host.append_child(host.document_handle(), other));
    assert!(host.append_child(other, b));

    assert_eq!(host.node(a).and_then(Node::parent_node), None);
    assert!(!child_list_effects_have_current_topology(&host, &removed));
}

#[test]
fn removal_snapshot_rejects_old_neighbor_reordered_within_parent() {
    let (mut host, parent, [a, b, _]) = sibling_fixture();
    let removed = host.remove_child_effects(parent, b);
    let removed = StyleMutationEffect::from_dom_mutation_effects(&host, &removed);
    assert!(host.append_child(parent, a));

    assert_eq!(host.node(b).and_then(Node::parent_node), None);
    assert_eq!(host.node(a).and_then(Node::parent_node), Some(parent));
    assert!(!child_list_effects_have_current_topology(&host, &removed));
}

#[test]
fn removal_snapshot_rejects_reattached_descendant_of_detached_root() {
    let (mut host, parent, [_, b, c]) = sibling_fixture();
    let descendant = host.create_element("span");
    assert!(host.append_child(b, descendant));
    let removed = host.remove_child_effects(parent, b);
    let removed = StyleMutationEffect::from_dom_mutation_effects(&host, &removed);
    assert!(child_list_effects_have_current_topology(&host, &removed));
    assert!(host.append_child(c, descendant));

    assert_eq!(host.node(b).and_then(Node::parent_node), None);
    assert!(!child_list_effects_have_current_topology(&host, &removed));
}

#[test]
fn insertion_snapshot_rejects_changed_range_at_consumption_time() {
    let (mut host, parent, [a, _, _]) = sibling_fixture();
    let added = host.create_element("span");
    let inserted = host.insert_before_effects(parent, added, Some(a));
    let inserted = StyleMutationEffect::from_dom_mutation_effects(&host, &inserted);
    assert!(child_list_effects_have_current_topology(&host, &inserted));
    assert!(host.append_child(parent, added));

    // Check the old record alone, without relying on the later mutation being
    // in the same viewport/media group.
    assert!(!child_list_effects_have_current_topology(&host, &inserted));
}
