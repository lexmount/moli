use super::navigation_activation::navigation_activation_value;
use super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entry_document_id,
    navigation_entry_referrer_policy_value, navigation_entry_url_value,
};
use super::navigation_entry_state::{
    history_entry_state_snapshot, navigation_entry_state_snapshot,
};
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, runtime_window_is_global,
    runtime_window_owner, window_history_for_holder, window_navigation_for_holder,
};
use super::*;
use crate::native_bridge::{
    NavigationActivationSeed, NavigationHistoryDocumentId, NavigationHistoryEntrySeed,
    NavigationHistorySerializedEntry,
};
use crate::{
    document_runtime::DomHandle, native_bridge::node_runtime_and_handle_from_object,
    referrer_policy::normalize_referrer_policy, util::context_host_ptr_from_window_object,
};

fn capture_navigation_entry_seed_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<NavigationHistoryEntrySeed> {
    let history = window_history_for_holder(scope, owner)?;
    let navigation = window_navigation_for_holder(scope, owner)?;
    Some(NavigationHistoryEntrySeed {
        entries: serialize_history_entries(scope, history),
        current_index: history_index(scope, history),
        activation: serialize_navigation_activation_seed(scope, navigation),
    })
}

pub(super) fn sync_child_navigation_entry_seed_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    sync_child_navigation_entry_seed_from_owner_with_document_url(scope, owner, true);
}

pub(super) fn sync_child_pending_navigation_entry_seed_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    sync_child_navigation_entry_seed_from_owner_with_document_url(scope, owner, false);
}

fn sync_child_navigation_entry_seed_from_owner_with_document_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    sync_document_url: bool,
) {
    if runtime_window_is_global(scope, owner) {
        return;
    }
    let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return;
    };
    let Some(entry_seed) = capture_navigation_entry_seed_for_holder(scope, owner) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_for_navigation_seed_owner(scope, owner) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    if host.set_child_browsing_context_navigation_entry_seed(handle, entry_seed)
        && sync_document_url
    {
        host.sync_child_browsing_context_document_url(scope, handle);
    }
}

fn context_host_ptr_for_navigation_seed_owner(
    scope: &mut v8::PinScope<'_, '_>,
    owner: v8::Local<'_, v8::Object>,
) -> Option<*mut crate::native_bridge::JsContextHost> {
    context_host_ptr_from_global_bridge(scope)
        .or_else(|| context_host_ptr_from_window_object(scope, owner))
        .or_else(|| {
            owner
                .get(scope, v8str(scope, "parent").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .and_then(|parent| context_host_ptr_from_window_object(scope, parent))
        })
}

pub(super) fn serialize_navigation_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    history_entries: &[NavigationHistorySerializedEntry],
) -> NavigationHistorySerializedEntry {
    let id = get_own_static_property(scope, entry, "id")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let key = get_own_static_property(scope, entry, "key")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(snapshot) = history_entries
        .iter()
        .find(|snapshot| snapshot.id == id && snapshot.key == key)
    {
        return snapshot.clone();
    }
    let url = navigation_entry_url_value(scope, entry).unwrap_or_else(|| "about:blank".to_owned());
    let history_state_json = history_entry_state_snapshot(scope, entry)
        .and_then(|value| v8::json::stringify(scope, value))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| value != "null");
    let navigation_state_json = navigation_entry_state_snapshot(scope, entry)
        .and_then(|value| v8::json::stringify(scope, value))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| value != "null");
    let entry_index = get_own_static_property(scope, entry, "index")
        .and_then(|value| value.integer_value(scope))
        .filter(|value| *value >= 0)
        .map(|value| value as u32)
        .unwrap_or(0);
    let document_id = navigation_entry_document_id(scope, entry).unwrap_or_else(|| id.clone());
    NavigationHistorySerializedEntry {
        url,
        history_state_json,
        navigation_state_json,
        referrer_policy: navigation_entry_referrer_policy_value(scope, entry),
        document_id: NavigationHistoryDocumentId::from_serialized(document_id),
        history_index: entry_index,
        index: entry_index,
        id,
        key,
    }
}

fn serialize_navigation_activation_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<NavigationActivationSeed> {
    let owner = runtime_window_owner(scope, navigation);
    let history = window_history_for_holder(scope, owner)?;
    let history_entries = serialize_history_entries(scope, history);
    let activation = navigation_activation_value(scope, navigation)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let entry = get_own_static_property(scope, activation, "entry")
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|entry| serialize_navigation_entry_object(scope, entry, &history_entries))?;
    let from = get_own_static_property(scope, activation, "from")
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|entry| serialize_navigation_entry_object(scope, entry, &history_entries));
    let navigation_type = get_own_static_property(scope, activation, "navigationType")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty() && value != "null");
    Some(NavigationActivationSeed {
        entry,
        from,
        navigation_type,
    })
}

pub(super) fn parse_history_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state_json: Option<&str>,
) -> v8::Local<'s, v8::Value> {
    if let Some(state_json) = state_json {
        v8_string(scope, state_json)
            .and_then(|json| v8::json::parse(scope, json))
            .unwrap_or_else(|| v8::null(scope).into())
    } else {
        v8::null(scope).into()
    }
}

pub(super) fn parse_navigation_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state_json: Option<&str>,
) -> v8::Local<'s, v8::Value> {
    if let Some(state_json) = state_json {
        v8_string(scope, state_json)
            .and_then(|json| v8::json::parse(scope, json))
            .unwrap_or_else(|| v8::undefined(scope).into())
    } else {
        v8::undefined(scope).into()
    }
}

pub(super) fn serialize_history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Vec<NavigationHistorySerializedEntry> {
    let Some(entries) = history_entries(scope, history) else {
        return Vec::new();
    };
    let mut snapshots = Vec::new();
    for index in 0..entries.length() {
        let Some(entry) = entries
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        let url =
            navigation_entry_url_value(scope, entry).unwrap_or_else(|| "about:blank".to_owned());
        let history_state_json = history_entry_state_snapshot(scope, entry)
            .and_then(|value| v8::json::stringify(scope, value))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| value != "null");
        let navigation_state_json = navigation_entry_state_snapshot(scope, entry)
            .and_then(|value| v8::json::stringify(scope, value))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| value != "null");
        let entry_index = get_own_static_property(scope, entry, "index")
            .and_then(|value| value.integer_value(scope))
            .filter(|value| *value >= 0)
            .map(|value| value as u32)
            .unwrap_or(index);
        let id = get_own_static_property(scope, entry, "id")
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| format!("entry-{entry_index}"));
        let key = get_own_static_property(scope, entry, "key")
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| format!("key-{entry_index}"));
        let document_id = navigation_entry_document_id(scope, entry).unwrap_or_else(|| id.clone());
        snapshots.push(NavigationHistorySerializedEntry {
            url,
            history_state_json,
            navigation_state_json,
            referrer_policy: navigation_entry_referrer_policy_value(scope, entry),
            document_id: NavigationHistoryDocumentId::from_serialized(document_id),
            history_index: index,
            index: entry_index,
            id,
            key,
        });
    }
    snapshots
}

pub(super) fn apply_current_document_referrer_policy_to_entry_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_index: u32,
    snapshots: &mut [NavigationHistorySerializedEntry],
) {
    let current_document_id = snapshots
        .iter()
        .find(|entry| entry.history_index == current_index)
        .map(|entry| entry.document_id.clone());
    let policy = current_document_referrer_policy(scope, owner).or_else(|| {
        current_document_id
            .as_ref()
            .and_then(|current_document_id| {
                snapshots
                    .iter()
                    .find(|entry| {
                        entry.document_id == *current_document_id && entry.referrer_policy.is_some()
                    })
                    .and_then(|entry| entry.referrer_policy.clone())
            })
    });
    let mut applied = false;
    if let Some(current_document_id) = current_document_id {
        for snapshot in snapshots
            .iter_mut()
            .filter(|entry| entry.document_id == current_document_id)
        {
            snapshot.referrer_policy = policy.clone();
            applied = true;
        }
    }
    if !applied
        && let Some(current) = snapshots
            .iter_mut()
            .find(|entry| entry.history_index == current_index)
    {
        current.referrer_policy = policy;
    }
}

pub(super) fn current_document_referrer_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let document = get_own_static_property(scope, owner, "document")
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    if let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, document)
    {
        let runtime = unsafe { &*runtime_ptr };
        return document_referrer_policy_for_native_document(runtime, document_handle)
            .or_else(|| current_entry_referrer_policy(scope, owner));
    }
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let document_handle = crate::native_bridge::document::detached_native_handle_for_runtime(
        scope,
        runtime_ptr,
        document,
    )?;
    let runtime = unsafe { &*runtime_ptr };
    document_referrer_policy_for_native_document(runtime, document_handle)
        .or_else(|| current_entry_referrer_policy(scope, owner))
}

pub(super) fn document_referrer_policy_for_native_document(
    runtime: &crate::native_bridge::JsContextHost,
    document_handle: DomHandle,
) -> Option<String> {
    document_referrer_policy_in_subtree(runtime, document_handle)
        .or_else(|| {
            (document_handle == runtime.document_handle())
                .then(|| runtime.response_referrer_policy().map(ToOwned::to_owned))
                .flatten()
        })
        .or_else(|| {
            runtime
                .child_browsing_context_referrer_policy_for_document_handle(document_handle)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            runtime
                .lightweight_popup_referrer_policy_for_document_handle(document_handle)
                .map(ToOwned::to_owned)
        })
}

pub(super) fn current_document_content_security_policies<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let Some(document) = get_own_static_property(scope, owner, "document")
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Vec::new();
    };
    if let Ok((runtime_ptr, document_handle)) = node_runtime_and_handle_from_object(scope, document)
    {
        let mut policies = Vec::new();
        document_content_security_policies_in_subtree(
            unsafe { &*runtime_ptr },
            document_handle,
            &mut policies,
        );
        return policies;
    }
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Vec::new();
    };
    let Some(document_handle) = crate::native_bridge::document::detached_native_handle_for_runtime(
        scope,
        runtime_ptr,
        document,
    ) else {
        return Vec::new();
    };
    let mut policies = Vec::new();
    document_content_security_policies_in_subtree(
        unsafe { &*runtime_ptr },
        document_handle,
        &mut policies,
    );
    policies
}

fn current_entry_referrer_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_current_entry(scope, owner)
        .and_then(|entry| navigation_entry_referrer_policy_value(scope, entry))
}

fn document_referrer_policy_in_subtree(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    if let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        && element.local_name().eq_ignore_ascii_case("meta")
        && element
            .attribute("name")
            .is_some_and(|name| name.eq_ignore_ascii_case("referrer"))
        && let Some(policy) = element
            .attribute("content")
            .and_then(normalize_meta_referrer_policy)
    {
        return Some(policy);
    }

    let mut child = runtime.dom_host().first_child(handle);
    while let Some(child_handle) = child {
        if let Some(policy) = document_referrer_policy_in_subtree(runtime, child_handle) {
            return Some(policy);
        }
        child = runtime.dom_host().next_sibling(child_handle);
    }
    None
}

fn document_content_security_policies_in_subtree(
    runtime: &crate::native_bridge::JsContextHost,
    handle: DomHandle,
    policies: &mut Vec<String>,
) {
    if let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        && element.local_name().eq_ignore_ascii_case("meta")
        && element
            .attribute("http-equiv")
            .is_some_and(|value| value.eq_ignore_ascii_case("Content-Security-Policy"))
        && let Some(policy) = element.attribute("content")
    {
        policies.push(policy.to_owned());
    }

    let mut child = runtime.dom_host().first_child(handle);
    while let Some(child_handle) = child {
        document_content_security_policies_in_subtree(runtime, child_handle, policies);
        child = runtime.dom_host().next_sibling(child_handle);
    }
}

fn normalize_meta_referrer_policy(raw: &str) -> Option<String> {
    normalize_referrer_policy(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_referrer_policy_uses_last_valid_token() {
        assert_eq!(
            normalize_meta_referrer_policy("not-yet-standardized, no-referrer"),
            Some("no-referrer".to_owned())
        );
        assert_eq!(
            normalize_meta_referrer_policy("same-origin, not-yet-standardized"),
            Some("same-origin".to_owned())
        );
    }
}
