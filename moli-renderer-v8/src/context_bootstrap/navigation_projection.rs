use super::history_runtime::native;
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, runtime_window_dispatch_scope,
    runtime_window_is_global, runtime_window_owner, runtime_window_uses_top_level_history_model,
};
use crate::context_bootstrap::navigation_entry::wrappers as entry_wrappers;
use crate::util::{context_host_ptr_from_global_bridge, serialize_v8_iter_array};
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
            let wrapper = entry_wrappers::for_window(scope, owner, entry);
            entry_wrappers::in_realm(scope, wrapper, context)
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
    let current = current_entry.borrow();
    let inherited_origin = current.inherited_origin.clone().or_else(|| {
        url::Url::parse(&current.url)
            .ok()
            .filter(navigation_entry_url_inherits_origin)
            .and_then(|_| current_document_origin(scope, owner))
    });
    let current_url = entry_origin_url(scope, owner, &current.url, inherited_origin.as_deref());
    drop(current);
    let hidden = |entry: &HistoryEntryRef| {
        top_level
            && !Rc::ptr_eq(entry, &current_entry)
            && entry.borrow().inherited_origin.is_none()
            && entry.borrow().url.split('#').next() == Some("about:blank")
    };
    let same_origin = |scope: &mut v8::PinScope<'s, '_>, entry: &HistoryEntryRef| {
        let entry = entry.borrow();
        let candidate_url =
            entry_origin_url(scope, owner, &entry.url, entry.inherited_origin.as_deref());
        match (&current_url, candidate_url) {
            (Some(current), Some(candidate)) => moli_url::same_origin(current, &candidate),
            _ => false,
        }
    };
    let mut start = current_index;
    while start > 0 {
        let candidate = &entries[start - 1];
        if !hidden(candidate) && !same_origin(scope, candidate) {
            break;
        }
        start -= 1;
    }
    let mut end = current_index;
    while end + 1 < entries.len() {
        let candidate = &entries[end + 1];
        if !hidden(candidate) && !same_origin(scope, candidate) {
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

fn entry_origin_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    raw_url: &str,
    inherited_origin: Option<&str>,
) -> Option<url::Url> {
    if let Some(origin) = inherited_origin {
        // An opaque inherited origin must not fall back to another Document's
        // URL. Only a recorded tuple origin can expose this history entry.
        return url::Url::parse(origin).ok();
    }
    let url = url::Url::parse(raw_url).ok()?;
    if url.scheme() == "blob"
        && let Some(inner) = raw_url.strip_prefix("blob:")
        && let Ok(inner_url) = url::Url::parse(inner)
    {
        return Some(inner_url);
    }
    if navigation_entry_url_inherits_origin(&url)
        && !runtime_window_is_global(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && child_browsing_context_handle_for_runtime_owner(scope, owner).is_some()
    {
        return Some(unsafe { &*host_ptr }.document_url().clone());
    }
    Some(url)
}

pub(super) fn navigation_entry_url_inherits_origin(url: &url::Url) -> bool {
    moli_url::is_about_blank(url) || (url.scheme() == "about" && url.path() == "srcdoc")
}

pub(super) fn current_document_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let host_ptr = owner
        .get_creation_context(scope)
        .and_then(crate::util::context_host_ptr_from_context_slot)
        .or_else(|| context_host_ptr_from_global_bridge(scope))?;
    let dispatch_scope = runtime_window_dispatch_scope(scope, owner)?;
    let host = unsafe { &*host_ptr };
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => host
            .current_main_document_resource_loader()
            .map(|loader| loader.fetch_context().origin().to_owned()),
        crate::native_bridge::OwnerDispatchScope::Child(_) => host
            .document_resource_loader_for_dispatch_scope(dispatch_scope)
            .map(|loader| loader.fetch_context().origin().to_owned()),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(id) => {
            host.lightweight_popup_origin(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::navigation_entry_url_inherits_origin;
    use url::Url;

    #[test]
    fn about_document_history_urls_inherit_origin_with_fragments() {
        for raw_url in [
            "about:blank",
            "about:blank#history",
            "about:srcdoc",
            "about:srcdoc#history",
        ] {
            assert!(navigation_entry_url_inherits_origin(
                &Url::parse(raw_url).expect("about URL should parse")
            ));
        }
        assert!(!navigation_entry_url_inherits_origin(
            &Url::parse("about:other#history").expect("about URL should parse")
        ));
    }
}
