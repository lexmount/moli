use super::*;

#[test]
fn restored_contexts_require_readmission_only_when_the_transition_changes() {
    let mut history = JointSessionHistory::default();
    let root = SessionHistoryContextId::ROOT;
    let child = SessionHistoryContextId::allocate();
    history.attach(root, None, entry("top", "top"));
    history.attach(child, Some(root), entry("child-0", "child"));
    let first = history.current_step();
    history.push(child, entry("child-1", "child"));
    let second = history.current_step();
    let saved = history.context_entries(child);
    history.detach(child);
    let stale = history.plan_traversal(first).unwrap();

    history.restore_context(child, root, &saved);
    let restored = history.clone();
    assert_eq!(history.commit_traversal(&stale), None);
    assert_eq!(
        history, restored,
        "failed admission must preserve restored entries"
    );
    let plan = history.plan_traversal(first).unwrap();
    assert_eq!(history.commit_traversal(&plan), Some(-1));
    assert_eq!(history.entry(child), Some(&entry("child-0", "child")));

    let forward = history.plan_traversal(second).unwrap();
    let stable = SessionHistoryContextId::allocate();
    history.restore_context(
        stable,
        root,
        &[
            (first, entry("stable", "stable")),
            (second, entry("stable", "stable")),
        ],
    );
    assert_eq!(history.commit_traversal(&forward), Some(1));
    assert_eq!(history.entry(child), Some(&entry("child-1", "child")));
    assert_eq!(history.entry(stable), Some(&entry("stable", "stable")));
}

#[test]
fn traversal_plan_contains_the_entire_mixed_transition_in_either_context_order() {
    for cross_created_first in [true, false] {
        let mut history = JointSessionHistory::default();
        let root = SessionHistoryContextId::ROOT;
        let first = SessionHistoryContextId::allocate();
        let second = SessionHistoryContextId::allocate();
        let (cross, same) = if cross_created_first {
            (first, second)
        } else {
            (second, first)
        };
        history.attach(root, None, entry("top", "top"));
        history.attach(cross, Some(root), entry("a0", "a-document-0"));
        history.attach(same, Some(root), entry("b0", "b-document"));
        let target = history.current_step();
        history.push(cross, entry("a1", "a-document-1"));
        history.push(same, entry("b1", "b-document"));
        let before = history.clone();
        let plan = history.plan_traversal(target).unwrap();
        assert_eq!(history, before, "planning must not change the cursor");
        assert_eq!(plan.source_step(), before.current_step());
        assert_eq!(plan.target_step(), target);
        assert_eq!(plan.delta(), -2);
        assert_eq!(plan.changes().len(), 2);
        for (context, from, to) in [
            (
                cross,
                entry("a1", "a-document-1"),
                entry("a0", "a-document-0"),
            ),
            (same, entry("b1", "b-document"), entry("b0", "b-document")),
        ] {
            assert_eq!(
                plan.changes()
                    .iter()
                    .find(|change| change.context == context),
                Some(&SessionHistoryContextChange {
                    context,
                    from: Some(from),
                    to: Some(to)
                })
            );
        }
        assert_eq!(plan.target_entries().get(&root), Some(&entry("top", "top")));
        assert_eq!(history.commit_traversal(&plan), Some(-2));
        assert_eq!(history.entry(cross), Some(&entry("a0", "a-document-0")));
        assert_eq!(history.entry(same), Some(&entry("b0", "b-document")));
        assert_eq!(
            history.position(),
            SessionHistoryPosition::new(0, 3).unwrap()
        );
        assert_eq!(
            history.commit_traversal(&plan),
            None,
            "an accepted plan cannot be replayed"
        );
    }
}

#[test]
fn intervening_mutations_reject_the_whole_plan_without_modifying_history() {
    let root = SessionHistoryContextId::ROOT;
    let child = SessionHistoryContextId::allocate();
    let mut source = JointSessionHistory::default();
    source.attach(root, None, entry("top", "top"));
    source.attach(child, Some(root), entry("child-0", "child-0"));
    let target = source.current_step();
    source.push(child, entry("child-1", "child-1"));
    for mutation in ["push", "replace", "attach", "detach", "prune", "traverse"] {
        let mut history = source.clone();
        let plan = history.plan_traversal(target).unwrap();
        match mutation {
            "push" => history.push(root, entry("top-2", "top")),
            "replace" => history.replace(child, entry("replacement", "child")),
            "attach" => history.attach(
                SessionHistoryContextId::allocate(),
                Some(child),
                entry("new", "new"),
            ),
            "detach" => history.detach(child),
            "prune" => history.prune_all_but_current(),
            "traverse" => {
                history
                    .commit_traversal(&history.plan_traversal(target).unwrap())
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let after_mutation = history.clone();
        assert_eq!(history.commit_traversal(&plan), None, "{mutation}");
        assert_eq!(
            history, after_mutation,
            "a stale {mutation} plan must have no effects"
        );
    }
}

#[test]
fn returning_to_the_same_cursor_does_not_revive_an_old_plan() {
    let root = SessionHistoryContextId::ROOT;
    let mut history = JointSessionHistory::default();
    history.attach(root, None, entry("a", "top"));
    let a = history.current_step();
    history.push(root, entry("b", "top"));
    let b = history.current_step();
    history.push(root, entry("c", "top"));
    let c = history.current_step();
    let stale = history.plan_traversal(b).unwrap();
    for target in [a, c] {
        let plan = history.plan_traversal(target).unwrap();
        history.commit_traversal(&plan).unwrap();
    }
    assert_eq!(history.commit_traversal(&stale), None);
    assert_eq!(history.entry(root), Some(&entry("c", "top")));
}

#[test]
fn independently_mutated_snapshots_cannot_accept_each_others_plans() {
    let root = SessionHistoryContextId::ROOT;
    let mut first = JointSessionHistory::default();
    first.attach(root, None, entry("a", "top"));
    let target = first.current_step();
    first.push(root, entry("b", "top"));
    let mut second = first.clone();
    for history in [&mut first, &mut second] {
        history.replace(root, entry("b-replaced", "top"));
    }
    assert_eq!(first.position(), second.position());
    assert_eq!(first.entry(root), second.entry(root));
    let plan = first.plan_traversal(target).unwrap();
    assert_eq!(second.commit_traversal(&plan), None);
    assert_eq!(first.commit_traversal(&plan), Some(-1));
}

#[test]
fn replacement_keeps_the_step_identity_but_invalidates_its_plans() {
    let root = SessionHistoryContextId::ROOT;
    let child = SessionHistoryContextId::allocate();
    let mut history = JointSessionHistory::default();
    history.attach(root, None, entry("top", "top"));
    history.attach(child, Some(root), entry("child-0", "child"));
    let target = history.current_step();
    history.push(child, entry("child-1", "child"));
    let plan = history.plan_traversal(target).unwrap();
    history.replace(root, entry("top-replaced", "top"));
    assert!(history.entries_at(target).is_some());
    assert_eq!(history.commit_traversal(&plan), None);
    assert_eq!(plan.target_entries().get(&root), Some(&entry("top", "top")));
    assert_eq!(
        history
            .plan_traversal(target)
            .unwrap()
            .target_entries()
            .get(&root),
        Some(&entry("top-replaced", "top"))
    );
}

#[test]
fn parent_document_replacement_plans_descendant_removal_and_restoration() {
    let root = SessionHistoryContextId::ROOT;
    let child = SessionHistoryContextId::allocate();
    let grandchild = SessionHistoryContextId::allocate();
    let mut history = JointSessionHistory::default();
    history.attach(root, None, entry("top-0", "document-0"));
    history.attach(child, Some(root), entry("child", "child"));
    history.attach(grandchild, Some(child), entry("grandchild", "grandchild"));
    let previous = history.current_step();
    history.push(root, entry("top-1", "document-1"));
    let next = history.current_step();
    let back = history.plan_traversal(previous).unwrap();
    for context in [child, grandchild] {
        let change = back
            .changes()
            .iter()
            .find(|change| change.context == context)
            .unwrap();
        assert!(change.from.is_none());
        assert!(change.to.is_some());
    }
    history.commit_traversal(&back).unwrap();
    let forward = history.plan_traversal(next).unwrap();
    for context in [child, grandchild] {
        let change = forward
            .changes()
            .iter()
            .find(|change| change.context == context)
            .unwrap();
        assert!(change.from.is_some());
        assert!(change.to.is_none());
    }
}

#[test]
fn idempotent_projection_updates_do_not_invalidate_a_plan() {
    let root = SessionHistoryContextId::ROOT;
    let mut history = JointSessionHistory::default();
    history.attach(root, None, entry("a", "top"));
    let previous = history.current_step();
    history.push(root, entry("b", "top"));
    let plan = history.plan_traversal(previous).unwrap();
    history.attach(root, None, entry("b", "top"));
    history.replace(root, entry("b", "top"));
    history.detach(SessionHistoryContextId::allocate());
    assert_eq!(history.commit_traversal(&plan), Some(-1));
}

#[test]
fn unchanged_context_attachment_preserves_admission_and_survives_commit() {
    let root = SessionHistoryContextId::ROOT;
    let child = SessionHistoryContextId::allocate();
    let mut history = JointSessionHistory::default();
    history.attach(root, None, entry("a", "top"));
    let target = history.current_step();
    history.push(root, entry("b", "top"));
    let plan = history.plan_traversal(target).unwrap();
    history.attach(child, Some(root), entry("child", "child"));
    assert!(history.is_traversal_plan_current(&plan));
    assert_eq!(history.commit_traversal(&plan), Some(-1));
    assert_eq!(history.entry(child), Some(&entry("child", "child")));

    let forward = history
        .plan_traversal(history.step_by_delta(1).unwrap())
        .unwrap();
    history.detach(child);
    assert_eq!(history.commit_traversal(&forward), Some(1));
    assert!(history.entry(child).is_none());
}

fn entry(key: &str, document: &str) -> SessionHistoryEntry {
    SessionHistoryEntry {
        key: NavigationHistoryEntryKey::from_serialized(key.into()),
        document: NavigationHistoryDocumentId::from_serialized(document.into()),
    }
}

#[test]
fn branch_truncates_forward_steps_and_invalidates_queued_targets() {
    let mut history = JointSessionHistory::default();
    let root = SessionHistoryContextId::ROOT;
    history.attach(root, None, entry("a", "top"));
    for key in ["b", "c", "d"] {
        history.push(root, entry(key, "top"));
    }
    let stale = history.current_step();
    let b = history.step_by_delta(-2).unwrap();
    let plan = history.plan_traversal(b).unwrap();
    assert_eq!(history.commit_traversal(&plan), Some(-2));
    history.push(root, entry("e", "top"));
    assert_eq!(
        history.position(),
        SessionHistoryPosition::new(2, 3).unwrap()
    );
    assert!(history.plan_traversal(stale).is_none());
    assert_eq!(history.entry(root), Some(&entry("e", "top")));
}

#[test]
fn joint_steps_follow_commit_order_and_navigation_traversal_uses_nearest_step() {
    let mut history = JointSessionHistory::default();
    let root = SessionHistoryContextId::ROOT;
    let a = SessionHistoryContextId::allocate();
    let b = SessionHistoryContextId::allocate();
    history.attach(root, None, entry("top", "top"));
    history.attach(a, Some(root), entry("a0", "a"));
    history.attach(b, Some(root), entry("b0", "b"));
    history.push(a, entry("a1", "a"));
    history.push(b, entry("b1", "b"));
    history.push(a, entry("a2", "a"));
    let target = history.step_for_entry(a, &entry("a1", "a").key).unwrap();
    let plan = history.plan_traversal(target).unwrap();
    assert_eq!(history.commit_traversal(&plan), Some(-1));
    assert_eq!(history.entry(b), Some(&entry("b1", "b")));
    let plan = history
        .plan_traversal(history.step_by_delta(-1).unwrap())
        .unwrap();
    history.commit_traversal(&plan).unwrap();
    assert_eq!(history.entry(b), Some(&entry("b0", "b")));
    history.push(b, entry("b2", "b"));
    assert_eq!(
        history.position(),
        SessionHistoryPosition::new(2, 3).unwrap()
    );
    assert!(!history.contains_entry(a, &entry("a2", "a").key));
    assert!(!history.contains_entry(b, &entry("b1", "b").key));
}

#[test]
fn replace_preserves_steps_and_updates_shared_entry_references() {
    let mut history = JointSessionHistory::default();
    let root = SessionHistoryContextId::ROOT;
    let child = SessionHistoryContextId::allocate();
    history.attach(root, None, entry("top", "top"));
    history.attach(child, Some(root), entry("child0", "child"));
    history.push(child, entry("child1", "child"));
    let position = history.position();
    history.replace(root, entry("top-replaced", "top"));
    assert_eq!(history.position(), position);
    let plan = history
        .plan_traversal(history.step_by_delta(-1).unwrap())
        .unwrap();
    history.commit_traversal(&plan).unwrap();
    assert_eq!(history.entry(root), Some(&entry("top-replaced", "top")));
    history.replace(root, entry("new-document", "new-document"));
    assert!(history.entry(child).is_none());
    assert_eq!(history.position().length(), 2);
}

#[test]
fn browser_position_includes_opaque_steps_and_push_uses_the_cursor() {
    let mut history = JointSessionHistory::new(SessionHistoryPosition::new(1, 4).unwrap());
    let root = SessionHistoryContextId::ROOT;
    history.attach(root, None, entry("current", "document"));
    history.push(root, entry("next", "document"));
    assert_eq!(
        history.position(),
        SessionHistoryPosition::new(2, 3).unwrap()
    );
    assert_eq!(
        history
            .entries_at(history.step_by_delta(-2).unwrap())
            .unwrap()
            .len(),
        0
    );
    assert!(SessionHistoryPosition::new(0, 0).is_none());
    assert!(SessionHistoryPosition::new(3, 3).is_none());
}
