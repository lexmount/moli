use super::navigation_entry::{
    history_length_number, navigation_entry_initial_index, navigation_entry_key_value,
    navigation_entry_url_value, set_history_length,
};
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, runtime_top_window_owner,
    runtime_window_is_global, runtime_window_owner, runtime_window_uses_top_level_history_model,
    window_history_for_holder,
};
use crate::util::{context_host_ptr_from_global_bridge, serialize_v8_iter_array};

pub(super) fn build_visible_navigation_entries_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    current_entry: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Array> {
    let visible_entries = visible_navigation_entries(scope, entries, current_entry);
    serialize_v8_iter_array(scope, visible_entries).unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(super) fn visible_navigation_entries_len<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    current_entry: Option<v8::Local<'s, v8::Object>>,
) -> u32 {
    visible_navigation_entries(scope, entries, current_entry).len() as u32
}

pub(super) fn visible_navigation_index_for_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    current_entry: Option<v8::Local<'s, v8::Object>>,
    target_entry: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    visible_navigation_entries(scope, entries, current_entry)
        .into_iter()
        .position(|entry| entry.strict_equals(target_entry.into()))
        .map(|index| index as u32)
}

fn visible_navigation_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    current_entry: Option<v8::Local<'s, v8::Object>>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(current_entry) = current_entry else {
        return all_entries(scope, entries);
    };
    let Some(current_index) = raw_index_for_entry(scope, entries, current_entry) else {
        return all_entries(scope, entries);
    };
    if navigation_entry_url_value(scope, current_entry)
        .and_then(|url| url::Url::parse(&url).ok())
        .is_none()
    {
        return vec![current_entry];
    }
    let owner = runtime_window_owner(scope, current_entry);

    let mut start = current_index;
    while start > 0 {
        let Some(candidate) = entry_at(scope, entries, start - 1) else {
            break;
        };
        if entry_is_hidden_for_owner(scope, owner, current_entry, candidate) {
            start -= 1;
            continue;
        }
        if !entries_are_same_origin(scope, owner, current_entry, candidate) {
            break;
        }
        start -= 1;
    }

    let mut end = current_index;
    while end + 1 < entries.length() {
        let Some(candidate) = entry_at(scope, entries, end + 1) else {
            break;
        };
        if entry_is_hidden_for_owner(scope, owner, current_entry, candidate) {
            end += 1;
            continue;
        }
        if !entries_are_same_origin(scope, owner, current_entry, candidate) {
            break;
        }
        end += 1;
    }

    let mut visible = Vec::new();
    for raw_index in start..=end {
        let entry = if raw_index == current_index {
            Some(current_entry)
        } else {
            entry_at(scope, entries, raw_index)
        };
        if let Some(entry) = entry {
            if entry_is_hidden_for_owner(scope, owner, current_entry, entry) {
                continue;
            }
            push_visible_entry(scope, &mut visible, entry);
        }
    }
    visible
}

fn entry_is_hidden_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_entry: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) -> bool {
    if !runtime_window_uses_top_level_history_model(scope, owner) {
        return false;
    }
    if !runtime_window_owner(scope, entry).strict_equals(owner.into()) {
        return true;
    }
    !entry.strict_equals(current_entry.into())
        && navigation_entry_url_value(scope, entry)
            .is_some_and(|url| url.split('#').next() == Some("about:blank"))
}

fn push_visible_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    visible: &mut Vec<v8::Local<'s, v8::Object>>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(index) = navigation_entry_initial_index(scope, entry) else {
        visible.push(entry);
        return;
    };
    if let Some(position) = visible.iter().position(|candidate| {
        navigation_entry_initial_index(scope, *candidate)
            .is_some_and(|candidate| candidate == index)
    }) {
        visible[position] = entry;
    } else {
        visible.push(entry);
    }
}

fn all_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
) -> Vec<v8::Local<'s, v8::Object>> {
    (0..entries.length())
        .filter_map(|index| entry_at(scope, entries, index))
        .collect()
}

fn raw_index_for_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    target_entry: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    if let Some(index) = (0..entries.length()).find(|index| {
        entry_at(scope, entries, *index)
            .is_some_and(|entry| entry.strict_equals(target_entry.into()))
    }) {
        return Some(index);
    }
    let target_key = navigation_entry_key_value(scope, target_entry)?;
    (0..entries.length()).find(|index| {
        entry_at(scope, entries, *index).is_some_and(|entry| {
            navigation_entry_key_value(scope, entry).as_deref() == Some(target_key.as_str())
        })
    })
}

fn entry_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    entries
        .get_index(scope, index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn entries_are_same_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_entry: v8::Local<'s, v8::Object>,
    candidate: v8::Local<'s, v8::Object>,
) -> bool {
    let current_url = navigation_entry_url_value(scope, current_entry)
        .and_then(|url| entry_origin_url(scope, owner, &url));
    let candidate_url = navigation_entry_url_value(scope, candidate)
        .and_then(|url| entry_origin_url(scope, owner, &url));
    match (current_url, candidate_url) {
        (Some(current), Some(candidate)) => moli_url::same_origin(&current, &candidate),
        _ => false,
    }
}

fn entry_origin_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    raw_url: &str,
) -> Option<url::Url> {
    let url = url::Url::parse(raw_url).ok()?;
    if url.scheme() == "blob"
        && let Some(inner) = raw_url.strip_prefix("blob:")
        && let Ok(inner_url) = url::Url::parse(inner)
    {
        return Some(inner_url);
    }
    if child_navigation_entry_url_inherits_origin(&url)
        && !runtime_window_is_global(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && child_browsing_context_handle_for_runtime_owner(scope, owner).is_some()
    {
        return Some(unsafe { &*host_ptr }.document_url().clone());
    }
    Some(url)
}

fn child_navigation_entry_url_inherits_origin(url: &url::Url) -> bool {
    url.scheme() == "about" && matches!(url.as_str(), "about:blank" | "about:srcdoc")
}

pub(super) fn set_history_length_from_visible_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    entries: v8::Local<'s, v8::Array>,
) {
    let length = history_length_floor_from_visible_entries(scope, history, entries);
    set_history_length(scope, history, length);
    set_top_history_length_at_least(scope, history, length);
}

pub(super) fn set_history_length_at_least_visible_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    entries: v8::Local<'s, v8::Array>,
) {
    let length = history_length_floor_from_visible_entries(scope, history, entries);
    let current_length = history_length_number(scope, history)
        .unwrap_or(0.0)
        .max(0.0);
    let length = current_length.max(length);
    set_history_length(scope, history, length);
    set_top_history_length_at_least(scope, history, length);
}

fn history_length_floor_from_visible_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    entries: v8::Local<'s, v8::Array>,
) -> f64 {
    let visible_length = all_entries(scope, entries)
        .into_iter()
        .filter_map(|entry| navigation_entry_initial_index(scope, entry))
        .max()
        .map(|index| index + 1)
        .unwrap_or_else(|| entries.length()) as f64;
    let owner = runtime_window_owner(scope, history);
    if runtime_window_uses_top_level_history_model(scope, owner) {
        return visible_length;
    }

    // A child sees the joint session-history length. The primary top-level
    // runtime keeps one hidden initial about:blank predecessor, while an
    // auxiliary Page's first navigation replaces its initial empty entry.
    // Project the offset of the owning root instead of assuming every child
    // belongs to the primary top-level runtime.
    let top_owner = runtime_top_window_owner(scope, owner);
    let root_predecessor_offset = if runtime_window_is_global(scope, top_owner) {
        1.0
    } else {
        0.0
    };
    visible_length + root_predecessor_offset
}

fn set_top_history_length_at_least<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    length: f64,
) {
    let owner = runtime_window_owner(scope, history);
    if runtime_window_uses_top_level_history_model(scope, owner) {
        return;
    }
    let top_window = runtime_top_window_owner(scope, owner);
    let Some(top_history) = window_history_for_holder(scope, top_window) else {
        return;
    };
    let current_length = history_length_number(scope, top_history)
        .unwrap_or(0.0)
        .max(0.0);
    set_history_length(scope, top_history, current_length.max(length));
}
