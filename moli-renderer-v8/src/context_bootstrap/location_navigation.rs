use super::location_runtime::{
    is_same_document_fragment_navigation, location_href_slot, resolve_location_navigation_target,
    sync_location_object,
};
use super::navigation_activation::{clear_navigation_transition, install_navigation_transition};
use super::navigation_callbacks::cancel_active_intercepted_same_document_navigation;
use super::navigation_entry::history_state_value;
use super::navigation_entry::{
    history_index, navigation_current_entry, navigation_current_entry_index,
};
use super::navigation_entry_state::clone_navigation_entry_state;
use super::navigation_events::{
    NavigationDispatchOutcome, cancel_active_navigation_event,
    dispatch_cross_document_navigation_navigate_event_for_window_with_type_and_form_data,
    dispatch_navigation_navigate_event_with_form_data_and_outcome,
    dispatch_navigation_navigate_event_with_outcome, dispatch_navigation_success,
    dispatch_popstate_event, queue_hash_change_for_runtime_owner,
    run_navigation_precommit_deferred_handlers,
};
use super::navigation_lifecycle::{
    begin_navigation_attempt, cancel_navigation_attempt, complete_navigation_attempt,
    finish_navigation_error_events, navigation_attempt_id_from_slot, navigation_attempt_is_active,
    settle_navigation_transition_finished_local,
};
use super::navigation_mutation::{
    apply_local_window_location_navigation, apply_navigation_navigate_same_document,
    sync_local_document_front_from_window, update_navigation_current_entry_for_same_document,
};
use super::navigation_reload::{NavigationReloadAdmission, navigation_reload_admission};
use super::navigation_result::{
    cancel_pending_same_document_navigation_finishes,
    cancel_pending_same_document_navigation_finishes_including_reentrant, navigation_dom_exception,
    queue_same_document_navigation_success,
};
use super::navigation_seed::history_entry_seed_for_reload;
use super::navigation_serialize::serialize_history_entries;
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_can_update_current_entry,
    navigation_document_has_opaque_origin, navigation_unload_event_active,
    runtime_window_is_global, runtime_window_owner, runtime_window_uses_top_level_history_model,
    url_is_about_blank_document, window_history_for_holder, window_location_for_holder,
};
use super::*;
use crate::native_bridge::NavigationHistoryEntrySeed;
use crate::util::{context_host_ptr_from_window_object, get_private_value, set_private_value};
use crate::webidl;
use moli_page_types::{
    NavigationHistoryMutation, SameDocumentHistoryUpdate, cross_document_navigation_seed,
};
use moli_webapi_declare::WebApiObject;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocationNavigationKind {
    Assign,
    Replace,
    Reload,
}

#[derive(Clone, Copy, PartialEq, Eq, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "NavigationHistoryBehavior")]
#[strum(serialize_all = "lowercase")]
pub(super) enum NavigationNavigateHistoryKind {
    #[webidl(token = "auto")]
    #[strum(serialize = "auto")]
    Default,
    Push,
    Replace,
}

pub(crate) fn navigate_location_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope, location, kind, raw_target, None, false, false,
    );
}

/// Applies a browser-initiated top-level navigation that protocol already
/// classified as same-document.
///
/// Keep this as a native renderer command rather than evaluating a synthetic
/// `location = ...` expression: CDP navigation must not depend on page-visible
/// properties being unmodified.
pub(crate) fn navigate_top_level_same_document_from_browser(
    scope: &mut v8::PinScope<'_, '_>,
    target: String,
) -> bool {
    let owner = scope.get_current_context().global(scope);
    let Some(location) = window_location_for_holder(scope, owner) else {
        return false;
    };
    let Some(current_href) = location_href_slot(scope, location) else {
        return false;
    };
    let Some(resolved) = resolve_location_navigation_target(
        scope,
        &current_href,
        LocationNavigationKind::Assign,
        Some(target.clone()),
    ) else {
        return false;
    };
    let current = url::Url::parse(&current_href).ok();
    if !is_same_document_fragment_navigation(current.as_ref(), &resolved) {
        return false;
    }

    // A repeated Page.navigate to the current fragment is unlike assigning a
    // fragment-only string through Location: Chromium pushes a same-document
    // history entry and runs the Navigation/popstate surfaces even though the
    // serialized URL does not change.
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        LocationNavigationKind::Assign,
        Some(target),
        None,
        false,
        true,
    );
    true
}

pub(crate) fn meta_refresh_navigation_kind(
    current_url: &url::Url,
    target_url: &url::Url,
    delay_ms: u32,
) -> LocationNavigationKind {
    let mut current_without_fragment = current_url.clone();
    current_without_fragment.set_fragment(None);
    let mut target_without_fragment = target_url.clone();
    target_without_fragment.set_fragment(None);
    if current_without_fragment == target_without_fragment {
        if target_url.fragment().is_some() {
            LocationNavigationKind::Assign
        } else {
            LocationNavigationKind::Reload
        }
    } else if delay_ms <= 1_000 {
        LocationNavigationKind::Replace
    } else {
        LocationNavigationKind::Assign
    }
}

/// Activates a top-level refresh through the normal Location/Navigation path.
/// This preserves reload/replace history semantics and page-visible navigate
/// cancellation instead of writing a browser handoff directly.
pub(crate) fn navigate_top_level_meta_refresh(
    scope: &mut v8::PinScope<'_, '_>,
    target: &url::Url,
    delay_ms: u32,
) -> bool {
    let window = scope.get_current_context().global(scope);
    let Some(location) = window_location_for_holder(scope, window) else {
        return false;
    };
    let Some(current_href) = location_href_slot(scope, location) else {
        return false;
    };
    let Some(current_url) = url::Url::parse(&current_href).ok() else {
        return false;
    };
    let kind = meta_refresh_navigation_kind(&current_url, target, delay_ms);
    let same_document_fragment = kind == LocationNavigationKind::Assign
        && is_same_document_fragment_navigation(Some(&current_url), target);
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        kind,
        Some(target.to_string()),
        None,
        false,
        same_document_fragment,
    );
    context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host_ptr| unsafe { &*host_ptr }.has_pending_location_navigation())
        || (same_document_fragment
            && location_href_slot(scope, location).as_deref() == Some(target.as_str()))
}

pub(crate) fn navigate_location_object_with_source_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        kind,
        raw_target,
        source_element,
        false,
        false,
    );
}

pub(crate) fn navigate_location_object_with_child_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope, location, kind, raw_target, None, true, false,
    );
}

fn navigate_location_object_with_source_element_and_child_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
    source_element: Option<v8::Local<'s, v8::Object>>,
    dispatch_child_navigate_event_for_all_kinds: bool,
    force_exact_same_document_navigation: bool,
) {
    let current_href = location_href_slot(scope, location).unwrap_or_default();
    let current_url = url::Url::parse(&current_href).ok();
    let raw_target_is_fragment_only = raw_target
        .as_deref()
        .is_some_and(|target| target.starts_with('#'));
    let resolved = match resolve_location_navigation_target(scope, &current_href, kind, raw_target)
    {
        Some(url) => url,
        None => {
            // Location APIs report an invalid URL synchronously. Element activation
            // resolves its target before entering this boundary and silently aborts
            // if an unresolved target nevertheless reaches it.
            if source_element.is_none() && !matches!(kind, LocationNavigationKind::Reload) {
                crate::context_bootstrap::throw_dom_exception_value(
                    scope,
                    "The provided value is not a valid URL.",
                    "SyntaxError",
                );
            }
            return;
        }
    };
    let exact_same_href = current_href == resolved.as_str();
    if !matches!(kind, LocationNavigationKind::Reload)
        && exact_same_href
        && source_element.is_none()
        && raw_target_is_fragment_only
        && !force_exact_same_document_navigation
    {
        return;
    }
    let owner = runtime_window_owner(scope, location);
    if navigation_unload_event_active(scope, owner) {
        return;
    }
    if sandbox_blocks_ancestor_or_top_location_navigation(scope, owner) {
        crate::context_bootstrap::throw_dom_exception_value(
            scope,
            "Blocked a sandboxed frame from navigating an ancestor browsing context.",
            "SecurityError",
        );
        return;
    }
    if matches!(kind, LocationNavigationKind::Reload)
        && matches!(
            navigation_reload_admission(scope, owner),
            NavigationReloadAdmission::NoCommittedHistoryItem
        )
    {
        return;
    }
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && unsafe { &mut *host_ptr }.navigate_lightweight_popup_window_to_url(
            scope,
            popup_id,
            resolved.clone(),
            kind,
        )
    {
        return;
    }

    if !matches!(kind, LocationNavigationKind::Reload)
        && (!exact_same_href || force_exact_same_document_navigation)
        && is_same_document_fragment_navigation(current_url.as_ref(), &resolved)
    {
        let opaque_origin = navigation_document_has_opaque_origin(scope, owner);
        if !opaque_origin
            && !runtime_window_is_global(scope, owner)
            && current_url
                .as_ref()
                .is_some_and(url_is_about_blank_document)
            && !navigation_document_can_update_current_entry(scope, owner)
        {
            sync_location_object(scope, location, resolved.as_str());
            sync_local_document_front_from_window(scope, owner);
            return;
        }
        let navigation = if opaque_origin {
            None
        } else {
            super::navigation_window::window_navigation_for_holder(scope, owner)
        };
        let effective_kind = match kind {
            LocationNavigationKind::Assign if source_element.is_some() && exact_same_href => {
                LocationNavigationKind::Replace
            }
            LocationNavigationKind::Assign
                if source_element.is_none()
                    && runtime_window_is_global(scope, owner)
                    && top_level_document_is_before_load_complete(scope) =>
            {
                if force_exact_same_document_navigation {
                    LocationNavigationKind::Assign
                } else {
                    LocationNavigationKind::Replace
                }
            }
            _ => kind,
        };
        let navigation_type = match effective_kind {
            LocationNavigationKind::Assign if source_element.is_some() => "push",
            LocationNavigationKind::Assign => "push",
            LocationNavigationKind::Replace => "replace",
            LocationNavigationKind::Reload => "reload",
        };
        let mut navigate_outcome = navigation.map(|navigation| {
            let _ = cancel_active_navigation_event(scope, navigation);
            cancel_active_intercepted_same_document_navigation(scope, navigation);
            cancel_active_location_intercepted_same_document_navigation(scope, navigation);
            cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
            dispatch_navigation_navigate_event_with_outcome(
                scope,
                navigation,
                resolved.as_str(),
                navigation_type,
                super::navigation_window::should_dispatch_hash_change(
                    &current_href,
                    resolved.as_str(),
                ),
                true,
                true,
                false,
                None,
                None,
                None,
                source_element,
            )
        });
        if navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.abort_error)
            .is_some()
        {
            return;
        }
        if navigate_outcome
            .as_ref()
            .is_some_and(|outcome| !outcome.proceed)
        {
            return;
        }
        if let Some(navigation) = navigation {
            cancel_pending_same_document_navigation_finishes(scope, navigation);
        }
        let transition_resolver = navigation.and_then(|navigation| {
            navigate_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.intercepted)
                .then(|| {
                    navigation_current_entry(scope, owner).and_then(|from| {
                        install_navigation_transition(
                            scope,
                            navigation,
                            from,
                            navigate_outcome
                                .as_ref()
                                .and_then(|outcome| outcome.destination),
                            navigation_type,
                        )
                    })
                })
                .flatten()
        });
        sync_location_object(scope, location, resolved.as_str());
        if opaque_origin {
            apply_navigation_navigate_same_document(
                scope,
                owner,
                resolved.as_str(),
                effective_kind,
                None,
            );
        } else {
            update_navigation_current_entry_for_same_document(
                scope,
                owner,
                resolved.as_str(),
                effective_kind,
            );
        }
        let child_handle = if runtime_window_is_global(scope, owner) {
            None
        } else {
            child_browsing_context_handle_for_runtime_owner(scope, owner)
        };
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let state = window_history_for_holder(scope, owner)
                .map(|history| history_state_value(scope, history))
                .unwrap_or_else(|| v8::null(scope).into());
            dispatch_popstate_event(scope, host_ptr, child_handle, state);
            queue_hash_change_for_runtime_owner(
                scope,
                owner,
                Some(&current_href),
                resolved.as_str(),
            );
        }
        if child_handle.is_none() {
            if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
                let host = unsafe { &mut *host_ptr };
                host.set_document_url(resolved.clone());
                let history_update = match effective_kind {
                    LocationNavigationKind::Assign => SameDocumentHistoryUpdate::Push,
                    LocationNavigationKind::Replace | LocationNavigationKind::Reload => {
                        SameDocumentHistoryUpdate::Replace
                    }
                };
                host.record_same_document_navigation(&resolved, "fragment", history_update);
            }
        } else {
            sync_local_document_front_from_window(scope, owner);
        }
        if let Some(navigation) = navigation {
            if navigate_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.intercepted)
            {
                let mut outcome = navigate_outcome
                    .take()
                    .expect("checked intercepted outcome");
                if let Some(precommit_event) = outcome.precommit_event {
                    let (intercept_error, intercept_result) =
                        run_navigation_precommit_deferred_handlers(scope, precommit_event);
                    outcome.intercept_error = intercept_error;
                    outcome.intercept_result = intercept_result.or(outcome.intercept_result);
                }
                settle_location_intercepted_same_document_navigation(
                    scope,
                    navigation,
                    outcome,
                    transition_resolver,
                    &current_href,
                );
            } else {
                queue_same_document_navigation_success(
                    scope,
                    navigation,
                    navigate_outcome.as_ref().and_then(|outcome| outcome.signal),
                    resolved.as_str(),
                );
            }
        }
        return;
    }

    let child_handle = if runtime_window_is_global(scope, owner) {
        None
    } else {
        child_browsing_context_handle_for_runtime_owner(scope, owner)
    };
    if child_handle.is_none()
        && let Some(navigation) =
            super::navigation_window::window_navigation_for_holder(scope, owner)
    {
        let _ = cancel_active_navigation_event(scope, navigation);
        cancel_active_intercepted_same_document_navigation(scope, navigation);
        cancel_active_location_intercepted_same_document_navigation(scope, navigation);
        cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
        let navigation_type = match kind {
            LocationNavigationKind::Assign if source_element.is_some() && exact_same_href => {
                "replace"
            }
            LocationNavigationKind::Assign => "push",
            LocationNavigationKind::Replace => "replace",
            LocationNavigationKind::Reload => "reload",
        };
        let destination_state = if matches!(kind, LocationNavigationKind::Reload) {
            navigation_current_entry(scope, owner)
                .and_then(|entry| clone_navigation_entry_state(scope, entry))
        } else {
            None
        };
        let mut outcome = dispatch_navigation_navigate_event_with_outcome(
            scope,
            navigation,
            resolved.as_str(),
            navigation_type,
            false,
            false,
            true,
            false,
            None,
            destination_state,
            None,
            source_element,
        );
        if outcome.abort_error.is_some() {
            return;
        }
        if !outcome.proceed {
            finish_location_navigation_canceled(scope, navigation, &outcome, &current_href);
            return;
        }
        cancel_pending_same_document_navigation_finishes(scope, navigation);
        if outcome.intercepted {
            let effective_href = outcome
                .redirected_url
                .as_deref()
                .unwrap_or(resolved.as_str())
                .to_owned();
            let effective_kind = outcome
                .redirected_history
                .as_deref()
                .map(|history| match history {
                    "replace" => LocationNavigationKind::Replace,
                    _ => LocationNavigationKind::Assign,
                })
                .unwrap_or(kind);
            let transition_resolver = navigation_current_entry(scope, owner).and_then(|from| {
                install_navigation_transition(
                    scope,
                    navigation,
                    from,
                    outcome.destination,
                    match effective_kind {
                        LocationNavigationKind::Assign => "push",
                        LocationNavigationKind::Replace => "replace",
                        LocationNavigationKind::Reload => "reload",
                    },
                )
            });
            sync_location_object(scope, location, &effective_href);
            update_navigation_current_entry_for_same_document(
                scope,
                owner,
                &effective_href,
                effective_kind,
            );
            if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
                && let Ok(url) = url::Url::parse(&effective_href)
            {
                let host = unsafe { &mut *host_ptr };
                host.set_document_url(url.clone());
                let history_update = match effective_kind {
                    LocationNavigationKind::Assign => SameDocumentHistoryUpdate::Push,
                    LocationNavigationKind::Replace | LocationNavigationKind::Reload => {
                        SameDocumentHistoryUpdate::Replace
                    }
                };
                host.record_same_document_navigation(&url, "fragment", history_update);
            }
            if let Some(precommit_event) = outcome.precommit_event {
                let (intercept_error, intercept_result) =
                    run_navigation_precommit_deferred_handlers(scope, precommit_event);
                outcome.intercept_error = intercept_error;
                outcome.intercept_result = intercept_result.or(outcome.intercept_result);
            }
            settle_location_intercepted_same_document_navigation(
                scope,
                navigation,
                outcome,
                transition_resolver,
                &current_href,
            );
            return;
        }
    }

    if let Some(handle) = child_handle {
        let is_javascript_url = resolved.scheme() == "javascript";
        if (matches!(kind, LocationNavigationKind::Assign)
            || dispatch_child_navigate_event_for_all_kinds)
            && !is_javascript_url
            && let Some(window) = window_for_child_cross_document_location_navigation(scope, owner)
            && !dispatch_cross_document_navigation_navigate_event_for_window_with_type_and_form_data(
                scope,
                window,
                resolved.as_str(),
                match kind {
                    LocationNavigationKind::Assign => "push",
                    LocationNavigationKind::Replace => "replace",
                    LocationNavigationKind::Reload => "reload",
                },
                source_element,
                false,
                None,
                None,
            )
        {
            return;
        }
        if !is_javascript_url {
            sync_location_object(scope, location, resolved.as_str());
            apply_local_window_location_navigation(scope, owner, &resolved, kind);
        }
        if let Some(host_ptr) = context_host_ptr_for_navigation_owner(scope, owner) {
            let host = unsafe { &mut *host_ptr };
            if matches!(kind, LocationNavigationKind::Assign) && !is_javascript_url {
                host.mark_child_browsing_context_top_level_history_increment(handle);
            }
            if matches!(kind, LocationNavigationKind::Reload) {
                host.queue_child_browsing_context_reload_from_existing_seed(
                    handle,
                    resolved.as_str(),
                );
            } else {
                host.queue_child_browsing_context_navigation_without_seed_update(
                    handle,
                    resolved.as_str(),
                );
            }
        }
        return;
    }

    if resolved.scheme() != "javascript" {
        sync_location_object(scope, location, resolved.as_str());
    }

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let entry_seed = if matches!(kind, LocationNavigationKind::Reload) {
        history_entry_seed_for_reload(scope, owner)
    } else {
        history_entry_seed_for_cross_document_location(scope, owner, &resolved, kind)
    };
    unsafe { &mut *host_ptr }.record_pending_location_navigation_with_kind(
        resolved,
        entry_seed,
        if matches!(kind, LocationNavigationKind::Reload) {
            moli_fetch::BrowserNavigationRequestKind::Reload
        } else {
            moli_fetch::BrowserNavigationRequestKind::Navigate
        },
    );
}

fn sandbox_blocks_ancestor_or_top_location_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(source_handle) =
        crate::context_bootstrap::current_child_browsing_context_handle_for_runtime_scope(scope)
    else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_for_navigation_owner(scope, owner)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
    else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    if host.child_browsing_context_allows_top_navigation(source_handle) {
        return false;
    }
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
        && host.lightweight_popup_id_for_node_owner_document(source_handle) != Some(popup_id)
    {
        return false;
    }
    match child_browsing_context_handle_for_runtime_owner(scope, owner) {
        Some(target_handle) => {
            child_browsing_context_is_ancestor(host, target_handle, source_handle)
        }
        None => runtime_window_uses_top_level_history_model(scope, owner),
    }
}

fn child_browsing_context_is_ancestor(
    host: &JsContextHost,
    ancestor: crate::document_runtime::DomHandle,
    child: crate::document_runtime::DomHandle,
) -> bool {
    let mut current = host.child_browsing_context_parent_handle(child);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = host.child_browsing_context_parent_handle(handle);
    }
    false
}

fn window_for_child_cross_document_location_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if runtime_window_is_global(scope, owner) {
        return None;
    }
    let host_ptr = context_host_ptr_for_navigation_owner(scope, owner)?;
    let handle = child_browsing_context_handle_for_runtime_owner(scope, owner)?;
    unsafe { &mut *host_ptr }.existing_child_browsing_context_window_wrapper(scope, handle)
}

pub(crate) fn dispatch_top_level_navigation_event_with_source_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    href: &str,
    navigation_type: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    can_intercept: bool,
    user_initiated: bool,
    download_request: Option<&str>,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(navigation) = super::navigation_window::window_navigation_for_holder(scope, global)
    else {
        return true;
    };
    let current_href = global
        .get(scope, v8str(scope, "location").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let _ = cancel_active_navigation_event(scope, navigation);
    cancel_active_intercepted_same_document_navigation(scope, navigation);
    cancel_active_location_intercepted_same_document_navigation(scope, navigation);
    cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
    let outcome = dispatch_navigation_navigate_event_with_outcome(
        scope,
        navigation,
        href,
        navigation_type,
        false,
        false,
        can_intercept,
        user_initiated,
        download_request,
        None,
        None,
        source_element,
    );
    if outcome.abort_error.is_some() {
        return false;
    }
    if !outcome.proceed {
        finish_location_navigation_canceled(scope, navigation, &outcome, &current_href);
        return false;
    }
    cancel_pending_same_document_navigation_finishes(scope, navigation);
    !outcome.intercepted
}

pub(crate) fn dispatch_top_level_form_navigation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    href: &str,
    navigation_type: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    form_data: v8::Local<'s, v8::Value>,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(navigation) = super::navigation_window::window_navigation_for_holder(scope, global)
    else {
        return true;
    };
    let current_href = global
        .get(scope, v8str(scope, "location").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let _ = cancel_active_navigation_event(scope, navigation);
    cancel_active_intercepted_same_document_navigation(scope, navigation);
    cancel_active_location_intercepted_same_document_navigation(scope, navigation);
    cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
    let outcome = dispatch_navigation_navigate_event_with_form_data_and_outcome(
        scope,
        navigation,
        href,
        navigation_type,
        false,
        false,
        true,
        user_initiated,
        None,
        None,
        None,
        Some(form_data),
        source_element,
    );
    if outcome.abort_error.is_some() {
        return false;
    }
    if !outcome.proceed {
        finish_location_navigation_canceled(scope, navigation, &outcome, &current_href);
        return false;
    }
    cancel_pending_same_document_navigation_finishes(scope, navigation);
    !outcome.intercepted
}

fn top_level_document_is_before_load_complete(scope: &mut v8::PinScope<'_, '_>) -> bool {
    context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
        unsafe { &*host_ptr }.host_document().ready_state()
            != crate::dom::native::DocumentReadyState::Complete
    })
}

const LOCATION_INTERCEPT_SETTLEMENT_NAVIGATION_SLOT: &str = "__lmLocationInterceptNavigation";
const LOCATION_INTERCEPT_SETTLEMENT_SIGNAL_SLOT: &str = "__lmLocationInterceptSignal";
const LOCATION_INTERCEPT_SETTLEMENT_FILENAME_SLOT: &str = "__lmLocationInterceptFilename";
const LOCATION_INTERCEPT_SETTLEMENT_PROMISE_SLOT: &str = "__lmLocationInterceptPromise";
const LOCATION_INTERCEPT_SETTLEMENT_TRANSITION_RESOLVER_SLOT: &str =
    "__lmLocationInterceptTransitionResolver";
const LOCATION_INTERCEPT_SETTLEMENT_ACTIVE_SLOT: &str = "__lmLocationInterceptActive";
const LOCATION_INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT: &str = "__lmLocationInterceptAttemptId";
const NAVIGATION_ACTIVE_LOCATION_INTERCEPT_SETTLEMENT_SLOT: &str =
    "__lmNavigationActiveLocationInterceptSettlement";

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct LocationInterceptSettlementDataDeclaration<'scope> {
    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_ACTIVE_SLOT)]
    active: bool,

    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT)]
    attempt_id: Option<v8::Local<'scope, v8::BigInt>>,

    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_PROMISE_SLOT)]
    promise: Option<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_TRANSITION_RESOLVER_SLOT)]
    transition_resolver: Option<v8::Local<'scope, v8::PromiseResolver>>,

    #[webapi(slot = LOCATION_INTERCEPT_SETTLEMENT_FILENAME_SLOT)]
    filename: v8::Local<'scope, v8::String>,
}

fn navigation_active_location_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_LOCATION_INTERCEPT_SETTLEMENT_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_active_location_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_LOCATION_INTERCEPT_SETTLEMENT_SLOT,
        data.into(),
    );
}

fn clear_navigation_active_location_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_LOCATION_INTERCEPT_SETTLEMENT_SLOT,
        v8::undefined(scope).into(),
    );
}

fn finish_location_navigation_canceled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: &NavigationDispatchOutcome<'s>,
    href: &str,
) {
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    if let Some(signal) = outcome.signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, href);
}

fn settle_location_intercepted_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: NavigationDispatchOutcome<'s>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    filename: &str,
) {
    if let Some(error) = outcome.intercept_error {
        finish_location_intercepted_navigation_rejected(
            scope,
            navigation,
            outcome.signal,
            transition_resolver,
            error,
            filename,
        );
        return;
    }
    let Some(result) = outcome.intercept_result else {
        dispatch_navigation_success(scope, navigation);
        settle_navigation_transition_finished_local(scope, navigation, transition_resolver, None);
        return;
    };
    let Some(result_object) = v8::Local::<v8::Object>::try_from(result).ok() else {
        dispatch_navigation_success(scope, navigation);
        settle_navigation_transition_finished_local(scope, navigation, transition_resolver, None);
        return;
    };
    let Some(then) = result_object
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        dispatch_navigation_success(scope, navigation);
        settle_navigation_transition_finished_local(scope, navigation, transition_resolver, None);
        return;
    };
    let attempt_id = begin_navigation_attempt(scope, "location-intercept-settlement")
        .map(|attempt_id| v8::BigInt::new_from_u64(scope, attempt_id.raw()));
    let promise = v8::Local::<v8::Promise>::try_from(result)
        .is_ok()
        .then_some(result);
    let filename = v8_string(scope, filename).unwrap_or_else(|| v8::String::empty(scope));
    let data = LocationInterceptSettlementDataDeclaration::new(
        true,
        attempt_id,
        promise,
        navigation,
        outcome.signal,
        transition_resolver,
        filename,
    )
    .bind(scope)
    .expect("location intercept settlement data should bind");
    set_navigation_active_location_intercept_settlement(scope, navigation, data);
    let Some(on_fulfilled) =
        v8::Function::builder(location_intercept_settlement_fulfilled_callback)
            .data(data.into())
            .build(scope)
    else {
        complete_location_intercept_settlement_attempt(scope, data);
        set_location_intercept_settlement_active(scope, navigation, data, false);
        dispatch_navigation_success(scope, navigation);
        settle_navigation_transition_finished_local(scope, navigation, transition_resolver, None);
        return;
    };
    let Some(on_rejected) = v8::Function::builder(location_intercept_settlement_rejected_callback)
        .data(data.into())
        .build(scope)
    else {
        complete_location_intercept_settlement_attempt(scope, data);
        set_location_intercept_settlement_active(scope, navigation, data, false);
        dispatch_navigation_success(scope, navigation);
        settle_navigation_transition_finished_local(scope, navigation, transition_resolver, None);
        return;
    };
    if then
        .call(scope, result, &[on_fulfilled.into(), on_rejected.into()])
        .is_none()
    {
        cancel_location_intercept_settlement_attempt(scope, data);
        set_location_intercept_settlement_active(scope, navigation, data, false);
    }
}

fn location_intercept_settlement_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    Option<v8::Local<'s, v8::Object>>,
    Option<v8::Local<'s, v8::Promise>>,
    Option<v8::Local<'s, v8::PromiseResolver>>,
    String,
)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let navigation = get_private_value(scope, data, LOCATION_INTERCEPT_SETTLEMENT_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = get_private_value(scope, data, LOCATION_INTERCEPT_SETTLEMENT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let promise = get_private_value(scope, data, LOCATION_INTERCEPT_SETTLEMENT_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    let transition_resolver = get_private_value(
        scope,
        data,
        LOCATION_INTERCEPT_SETTLEMENT_TRANSITION_RESOLVER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) });
    let filename = get_private_value(scope, data, LOCATION_INTERCEPT_SETTLEMENT_FILENAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    Some((navigation, signal, promise, transition_resolver, filename))
}

fn location_intercept_settlement_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !location_intercept_settlement_is_active(scope, args.data()) {
        return;
    }
    let Some((navigation, _, _, transition_resolver, _)) =
        location_intercept_settlement_data(scope, args.data())
    else {
        return;
    };
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        complete_location_intercept_settlement_attempt(scope, data);
        set_location_intercept_settlement_active(scope, navigation, data, false);
    }
    dispatch_navigation_success(scope, navigation);
    settle_navigation_transition_finished_local(scope, navigation, transition_resolver, None);
}

fn location_intercept_settlement_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !location_intercept_settlement_is_active(scope, args.data()) {
        return;
    }
    let Some((navigation, signal, promise, transition_resolver, filename)) =
        location_intercept_settlement_data(scope, args.data())
    else {
        return;
    };
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        complete_location_intercept_settlement_attempt(scope, data);
        set_location_intercept_settlement_active(scope, navigation, data, false);
    }
    let error = promise
        .filter(|promise| promise.state() == v8::PromiseState::Rejected)
        .map(|promise| promise.result(scope))
        .unwrap_or_else(|| args.get(0));
    finish_location_intercepted_navigation_rejected(
        scope,
        navigation,
        signal,
        transition_resolver,
        error,
        &filename,
    );
}

fn finish_location_intercepted_navigation_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    error: v8::Local<'s, v8::Value>,
    filename: &str,
) {
    if let Some(signal) = signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, filename);
    settle_navigation_transition_finished_local(
        scope,
        navigation,
        transition_resolver,
        Some(error),
    );
}

fn cancel_active_location_intercepted_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(data) = navigation_active_location_intercept_settlement(scope, navigation) else {
        return false;
    };
    if !location_intercept_settlement_is_active(scope, data.into()) {
        return false;
    }
    cancel_location_intercept_settlement_attempt(scope, data);
    set_location_intercept_settlement_active(scope, navigation, data, false);
    let Some((navigation, signal, _, transition_resolver, filename)) =
        location_intercept_settlement_data(scope, data.into())
    else {
        return false;
    };
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    if let Some(signal) = signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, &filename);
    if transition_resolver.is_some() {
        clear_navigation_transition(scope, navigation);
    }
    if let Some(transition_resolver) = transition_resolver {
        let _ = transition_resolver.reject(scope, error);
    }
    true
}

fn location_intercept_settlement_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(data)
        .ok()
        .is_some_and(|data| {
            location_intercept_settlement_active(scope, data)
                && navigation_attempt_id_from_slot(
                    scope,
                    data,
                    LOCATION_INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT,
                )
                .is_some_and(|attempt_id| navigation_attempt_is_active(scope, attempt_id))
        })
}

fn location_intercept_settlement_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, data, LOCATION_INTERCEPT_SETTLEMENT_ACTIVE_SLOT)
        .is_some_and(|value| value.is_boolean() && value.boolean_value(scope))
}

fn complete_location_intercept_settlement_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    if let Some(attempt_id) =
        navigation_attempt_id_from_slot(scope, data, LOCATION_INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT)
    {
        complete_navigation_attempt(scope, attempt_id);
    }
}

fn cancel_location_intercept_settlement_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    if let Some(attempt_id) =
        navigation_attempt_id_from_slot(scope, data, LOCATION_INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT)
    {
        cancel_navigation_attempt(scope, attempt_id);
    }
}

fn set_location_intercept_settlement_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_private_value(
        scope,
        data,
        LOCATION_INTERCEPT_SETTLEMENT_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
    if !active {
        clear_navigation_active_location_intercept_settlement(scope, navigation);
    }
}

fn history_entry_seed_for_cross_document_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    resolved: &url::Url,
    kind: LocationNavigationKind,
) -> Option<NavigationHistoryEntrySeed> {
    let history = window_history_for_holder(scope, owner)?;
    let current_index = history_index(scope, history);
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let mutation = match kind {
        LocationNavigationKind::Assign => NavigationHistoryMutation::Push,
        LocationNavigationKind::Replace => NavigationHistoryMutation::Replace,
        LocationNavigationKind::Reload => return None,
    };
    Some(cross_document_navigation_seed(
        serialize_history_entries(scope, history),
        current_index,
        current_navigation_index,
        resolved,
        mutation,
    ))
}

fn context_host_ptr_for_navigation_owner(
    scope: &mut v8::PinScope<'_, '_>,
    owner: v8::Local<'_, v8::Object>,
) -> Option<*mut JsContextHost> {
    context_host_ptr_from_global_bridge(scope)
        .or_else(|| context_host_ptr_from_window_object(scope, owner))
        .or_else(|| {
            owner
                .get(scope, v8str(scope, "parent").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .and_then(|parent| context_host_ptr_from_window_object(scope, parent))
        })
}

#[cfg(test)]
mod meta_refresh_tests {
    use super::*;

    #[test]
    fn meta_refresh_uses_reload_replace_and_assign_history_kinds() {
        let current = url::Url::parse("https://example.test/current").unwrap();
        assert_eq!(
            meta_refresh_navigation_kind(&current, &current, 5_000),
            LocationNavigationKind::Reload
        );
        assert_eq!(
            meta_refresh_navigation_kind(
                &current,
                &url::Url::parse("https://example.test/quick").unwrap(),
                1_000,
            ),
            LocationNavigationKind::Replace
        );
        assert_eq!(
            meta_refresh_navigation_kind(
                &current,
                &url::Url::parse("https://example.test/later").unwrap(),
                1_001,
            ),
            LocationNavigationKind::Assign
        );
        assert_eq!(
            meta_refresh_navigation_kind(
                &current,
                &url::Url::parse("https://example.test/current#done").unwrap(),
                0,
            ),
            LocationNavigationKind::Assign,
            "Blink leaves fragment refreshes as standard same-document navigations"
        );
    }
}
