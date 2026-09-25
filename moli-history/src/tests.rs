use std::rc::Rc;

use moli_session_history::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};

use crate::*;

fn entry(id: &str) -> HistoryEntryRef {
    HistoryEntry {
        url: format!("https://example.test/#{id}"),
        id: id.to_owned(),
        key: NavigationHistoryEntryKey::allocate(),
        document: NavigationHistoryDocumentId::allocate(),
        index: 0,
        referrer_policy: None,
        history_state: None,
        navigation_state: None,
        scroll_restoration: ScrollRestoration::Auto,
        scroll_offset: None,
    }
    .into_ref()
}

#[test]
fn pushing_after_a_traversal_discards_only_forward_entries() {
    let first = entry("first");
    let second = entry("second");
    let third = entry("third");
    let mut history = WindowHistory::new(vec![first.clone(), second.clone(), third.clone()], 2);
    history.set_current_index(0);
    let replacement = entry("replacement");
    let removed = history.push(replacement.clone());
    assert_eq!(history.current_index(), 1);
    assert_eq!(history.entries().len(), 2);
    assert!(Rc::ptr_eq(&history.entries()[0], &first));
    assert!(Rc::ptr_eq(history.current_entry().unwrap(), &replacement));
    assert!(Rc::ptr_eq(&removed[0], &second));
    assert!(Rc::ptr_eq(&removed[1], &third));
    // A retained entry remains readable after leaving its former history.
    drop(history);
    assert_eq!(removed[0].borrow().url, "https://example.test/#second");
}

#[test]
fn replacement_preserves_forward_history_and_retained_old_entry() {
    let first = entry("first");
    let next = entry("next");
    let mut history = WindowHistory::new(vec![first.clone(), next.clone()], 0);
    let replacement = entry("replacement");
    let old = history.replace(replacement.clone()).unwrap();
    assert!(Rc::ptr_eq(&old, &first));
    assert_eq!(history.current_index(), 0);
    assert!(Rc::ptr_eq(&history.entries()[1], &next));
    assert!(Rc::ptr_eq(history.current_entry().unwrap(), &replacement));
}

#[test]
fn scroll_restoration_is_inherited_then_restored_per_entry() {
    let mut history = WindowHistory::new(vec![entry("first")], 0);
    history.set_scroll_restoration(ScrollRestoration::Manual);
    history.push(entry("second"));
    assert_eq!(history.scroll_restoration(), ScrollRestoration::Manual);
    history.set_scroll_restoration(ScrollRestoration::Auto);
    history.set_current_index(0);
    assert_eq!(history.scroll_restoration(), ScrollRestoration::Manual);
    history.set_current_index(1);
    assert_eq!(history.scroll_restoration(), ScrollRestoration::Auto);
}

#[test]
fn revisiting_a_serialized_snapshot_invalidates_the_bindings_cache() {
    let snapshot = SerializedScriptValue::new(vec![1, 2, 3], ());
    let mut history = WindowHistory::new(vec![entry("first")], 0);
    history.set_state(Some(snapshot.clone()));
    let first_revision = history.revision();
    history.push(entry("second"));
    history.set_current_index(0);
    assert!(history.revision() > first_revision);
    assert_eq!(history.state(), Some(snapshot.clone()));
    drop(history);
    assert_eq!(snapshot.bytes(), &[1, 2, 3]);
}

#[test]
fn pruning_other_entries_preserves_current_state_identity() {
    let current = entry("current");
    let mut history =
        WindowHistory::new(vec![entry("previous"), current.clone(), entry("next")], 1);
    let revision = history.revision();
    history.restore_entries(vec![current.clone()]);
    history.set_current_index(0);
    assert_eq!(history.current_index(), 0);
    assert_eq!(history.revision(), revision);
    assert!(Rc::ptr_eq(history.current_entry().unwrap(), &current));
}
