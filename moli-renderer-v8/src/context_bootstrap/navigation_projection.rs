use super::history_runtime::native;
use super::navigation_window::{runtime_window_owner, runtime_window_uses_top_level_history_model};
use crate::util::serialize_v8_iter_array;
use moli_history::HistoryEntryRef;
use std::rc::Rc;

pub(super) fn build_visible_navigation_entries_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entries: &[HistoryEntryRef],
    current_entry: Option<v8::Local<'s, v8::Object>>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Array> {
    let visible_entries = visible_navigation_entries(scope, entries, current_entry);
    let wrappers: Vec<_> = visible_entries
        .into_iter()
        .map(|entry| {
            let wrapper = native::entry_wrapper(scope, owner, entry);
            native::entry_in_realm(scope, wrapper, context)
        })
        .collect();
    let scope = &mut v8::ContextScope::new(scope, context);
    serialize_v8_iter_array(scope, wrappers).unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(super) fn visible_navigation_entries_len<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[HistoryEntryRef],
    current_entry: Option<v8::Local<'s, v8::Object>>,
) -> u32 {
    visible_navigation_entries(scope, entries, current_entry).len() as u32
}

pub(super) fn visible_navigation_index_for_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[HistoryEntryRef],
    current_entry: Option<v8::Local<'s, v8::Object>>,
    target_entry: &HistoryEntryRef,
) -> Option<u32> {
    visible_navigation_entries(scope, entries, current_entry)
        .iter()
        .position(|entry| Rc::ptr_eq(entry, target_entry))
        .map(|index| index as u32)
}

fn visible_navigation_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: &[HistoryEntryRef],
    current_entry: Option<v8::Local<'s, v8::Object>>,
) -> Vec<HistoryEntryRef> {
    let Some(current_wrapper) = current_entry else {
        return entries.to_vec();
    };
    let Some(current_entry) = native::entry(scope, current_wrapper) else {
        return entries.to_vec();
    };
    let Some(current_index) = raw_index_for_entry(entries, &current_entry) else {
        return entries.to_vec();
    };
    if url::Url::parse(&current_entry.borrow().url).is_err() {
        return vec![current_entry];
    }
    let owner = runtime_window_owner(scope, current_wrapper);
    let top_level = runtime_window_uses_top_level_history_model(scope, owner);
    let hidden = |entry: &HistoryEntryRef| {
        top_level
            && !Rc::ptr_eq(entry, &current_entry)
            && entry.borrow().url.split('#').next() == Some("about:blank")
    };
    let same_origin = |entry: &HistoryEntryRef| {
        let current = current_entry.borrow();
        let candidate = entry.borrow();
        current.document == candidate.document
            || (current.document_origin != "null"
                && current.document_origin == candidate.document_origin)
    };
    let mut start = current_index;
    while start > 0 {
        let candidate = &entries[start - 1];
        if !hidden(candidate) && !same_origin(candidate) {
            break;
        }
        start -= 1;
    }
    let mut end = current_index;
    while end + 1 < entries.len() {
        let candidate = &entries[end + 1];
        if !hidden(candidate) && !same_origin(candidate) {
            break;
        }
        end += 1;
    }
    let mut visible: Vec<HistoryEntryRef> = Vec::new();
    for (offset, entry) in entries[start..=end].iter().enumerate() {
        let entry = if start + offset == current_index {
            &current_entry
        } else {
            entry
        };
        if hidden(entry) {
            continue;
        }
        let index = entry.borrow().index;
        if let Some(position) = visible
            .iter()
            .position(|candidate| candidate.borrow().index == index)
        {
            visible[position] = entry.clone();
        } else {
            visible.push(entry.clone());
        }
    }
    visible
}

fn raw_index_for_entry(
    entries: &[HistoryEntryRef],
    target_entry: &HistoryEntryRef,
) -> Option<usize> {
    entries
        .iter()
        .position(|entry| Rc::ptr_eq(entry, target_entry))
        .or_else(|| {
            let target = target_entry.borrow();
            entries
                .iter()
                .position(|entry| entry.borrow().key == target.key)
        })
}
