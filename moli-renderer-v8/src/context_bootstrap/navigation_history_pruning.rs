use super::history_runtime::native;
use super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entry_key_value,
    set_history_entries, set_history_index,
};
use super::navigation_events::dispatch_navigation_entry_dispose;
use super::navigation_window::window_history_for_holder;
use moli_history::HistoryEntryRef;

#[derive(Debug)]
pub(crate) struct NavigationHistoryPrunePlan {
    retained_entry_key: String,
    removed_entry_keys: Vec<String>,
}

pub(crate) fn plan_navigation_history_prune(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<NavigationHistoryPrunePlan> {
    let (_history, entries, current_index, current_entry) = navigation_history_prune_state(scope)?;
    let retained_entry_key = current_entry.borrow().key.as_str().to_owned();
    let removed_entry_keys = entries
        .iter()
        .enumerate()
        .rev()
        .filter(|(index, _)| *index != current_index as usize)
        .map(|(_, entry)| entry.borrow().key.as_str().to_owned())
        .collect();
    Some(NavigationHistoryPrunePlan {
        retained_entry_key,
        removed_entry_keys,
    })
}

pub(crate) fn apply_navigation_history_prune_plan(
    scope: &mut v8::PinScope<'_, '_>,
    plan: &NavigationHistoryPrunePlan,
) -> bool {
    let owner = scope.get_current_context().global(scope);
    let Some(history) = window_history_for_holder(scope, owner) else {
        return false;
    };
    let Some(entries) = history_entries(scope, history) else {
        return false;
    };
    let Some(current_entry_key) = navigation_current_entry(scope, owner)
        .and_then(|entry| navigation_entry_key_value(scope, entry))
    else {
        return false;
    };

    let mut retained_entries = Vec::with_capacity(entries.len());
    let mut removed_entries = Vec::with_capacity(plan.removed_entry_keys.len());
    for entry in entries {
        let key = entry.borrow().key.as_str().to_owned();
        if plan.removed_entry_keys.contains(&key) {
            removed_entries.push((key, native::entry_wrapper(scope, owner, entry)));
        } else {
            retained_entries.push((key, entry));
        }
    }
    if !retained_entries
        .iter()
        .any(|(key, _)| key == &plan.retained_entry_key)
    {
        return false;
    }
    let Some(current_index) = retained_entries
        .iter()
        .position(|(key, _)| key == &current_entry_key)
    else {
        return false;
    };
    let retained_entries = retained_entries
        .into_iter()
        .enumerate()
        .map(|(index, (_, entry))| {
            entry.borrow_mut().index = index as u32;
            entry
        })
        .collect();
    set_history_entries(scope, history, retained_entries);
    set_history_index(scope, history, current_index as u32);
    super::navigation_serialize::sync_child_navigation_entry_seed_from_owner(scope, owner);
    for removed_key in &plan.removed_entry_keys {
        if let Some((_, entry)) = removed_entries.iter().find(|(key, _)| key == removed_key) {
            dispatch_navigation_entry_dispose(scope, *entry);
        }
    }
    true
}

pub(crate) fn finalize_navigation_history_prune(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let owner = scope.get_current_context().global(scope);
    window_history_for_holder(scope, owner).is_some()
}

fn navigation_history_prune_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    Vec<HistoryEntryRef>,
    u32,
    HistoryEntryRef,
)> {
    let owner = scope.get_current_context().global(scope);
    let history = window_history_for_holder(scope, owner)?;
    let entries = history_entries(scope, history)?;
    let current_index = history_index(scope, history);
    let current_entry = entries.get(current_index as usize)?.clone();
    Some((history, entries, current_index, current_entry))
}
