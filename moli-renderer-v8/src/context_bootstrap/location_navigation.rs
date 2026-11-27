pub(super) use moli_webapi::navigation_values::options::NavigationNavigateHistoryKind;

use super::location_runtime::{
    is_same_document_fragment_navigation, location_href_slot, resolve_location_navigation_target,
    sync_location_object, urls_equal_ignoring_fragment,
};
use super::navigation_activation::install_navigation_transition;
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
    dispatch_navigation_navigate_event_with_outcome, dispatch_popstate_event,
    queue_hash_change_for_runtime_owner,
};
use super::navigation_lifecycle::finish_navigation_error_events;
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
use super::navigation_serialize::{
    apply_current_document_referrer_policy_to_entry_snapshots, serialize_history_entries,
};
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_can_update_current_entry,
    navigation_document_has_opaque_origin, navigation_unload_event_active,
    runtime_window_is_global, runtime_window_owner, runtime_window_uses_top_level_history_model,
    url_is_about_blank_document, window_history_for_holder, window_location_for_holder,
};
use super::*;
use crate::native_bridge::{LocationNavigationSource, NavigationHistoryEntrySeed};
use crate::util::context_host_ptr_for_navigation_owner;
use moli_page_types::{NavigationHistoryMutation, cross_document_navigation_seed};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocationNavigationKind {
    Assign,
    Replace,
    Reload,
}

#[derive(Default)]
struct LocationNavigationOptions<'s> {
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    dispatch_child_navigate_event_for_all_kinds: bool,
    force_exact_same_document_navigation: bool,
}

pub(crate) fn resolve_cross_window_location_target(
    scope: &mut v8::PinScope<'_, '_>,
    raw: &str,
) -> Option<String> {
    let entry = scope.get_entered_or_microtask_context();
    let host = crate::util::context_host_ptr_from_context_slot(entry)
        .or_else(|| {
            scope
                .get_incumbent_context()
                .and_then(crate::util::context_host_ptr_from_context_slot)
        })
        .or_else(|| context_host_ptr_from_global_bridge(scope))?;
    super::window_runtime::entered_window_api_base_url(scope, unsafe { &*host })
        .join(raw)
        .ok()
        .map(|url| url.to_string())
}

pub(crate) fn navigate_location_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) {
    navigate_location_object_with_options(
        scope,
        location,
        kind,
        raw_target,
        LocationNavigationOptions::default(),
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
    replace_current: bool,
) -> bool {
    navigate_top_level_same_document(
        scope,
        target,
        replace_current,
        LocationNavigationOptions::default(),
    )
}

fn navigate_top_level_same_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: String,
    replace_current: bool,
    options: LocationNavigationOptions<'s>,
) -> bool {
    let kind = if replace_current {
        LocationNavigationKind::Replace
    } else {
        LocationNavigationKind::Assign
    };
    let owner = scope.get_current_context().global(scope);
    let Some(location) = window_location_for_holder(scope, owner) else {
        return false;
    };
    let Some(current_href) = location_href_slot(scope, location) else {
        return false;
    };
    let Some(resolved) =
        resolve_location_navigation_target(scope, &current_href, kind, Some(target.clone()))
    else {
        return false;
    };
    let current = url::Url::parse(&current_href).ok();
    if !is_same_document_fragment_navigation(current.as_ref(), &resolved) {
        return false;
    }

    // Browser and popup requests still apply history and dispatch events
    // when the serialized fragment URL stays the same.
    navigate_location_object_with_options(
        scope,
        location,
        kind,
        Some(target),
        LocationNavigationOptions {
            force_exact_same_document_navigation: true,
            ..options
        },
    );
    true
}

#[cfg(feature = "molio")]
pub(crate) fn queue_related_popup_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    target: url::Url,
    same_origin: bool,
    navigation_source: LocationNavigationSource,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
) {
    let Some(context) = window.get_creation_context(scope) else {
        return;
    };
    let Some(host) = crate::util::context_host_ptr_from_context_slot(context) else {
        return;
    };
    let can_dispatch_navigate_event =
        same_origin && crate::native_bridge::caller_can_access_window(scope, window);
    let scope = &mut v8::ContextScope::new(scope, context);
    let window = context.global(scope);
    if navigation_unload_event_active(scope, window) {
        return;
    }
    let current_url = unsafe { &*host }.document_url().clone();
    let initial_empty =
        super::navigation_window::navigation_document_is_initial_empty(scope, window);
    let replace_current = initial_empty || (same_origin && current_url == target);
    if same_origin
        && navigate_top_level_same_document(
            scope,
            target.to_string(),
            replace_current,
            LocationNavigationOptions {
                source_element: source_element.filter(|_| can_dispatch_navigate_event),
                user_initiated: can_dispatch_navigate_event && user_initiated,
                ..Default::default()
            },
        )
    {
        return;
    }
    let kind = if replace_current {
        LocationNavigationKind::Replace
    } else {
        LocationNavigationKind::Assign
    };
    // Same-origin access is checked in the source realm before entering the
    // target context. The initial about:blank Document has no navigate event.
    let navigate_event = (can_dispatch_navigate_event
        && !initial_empty
        && matches!(
            target.scheme(),
            "about" | "blob" | "data" | "file" | "http" | "https"
        ))
    .then(|| LocationNavigateEvent {
        source_element,
        user_initiated,
        can_intercept: urls_equal_ignoring_fragment(&current_url, &target)
            || (matches!(current_url.scheme(), "http" | "https")
                && moli_url::same_origin(&current_url, &target)
                && current_url.username() == target.username()
                && current_url.password() == target.password()),
    });
    navigate_top_level_cross_document_location(
        scope,
        window,
        None,
        current_url.as_str(),
        target,
        kind,
        navigate_event,
        Some(navigation_source),
    );
}

#[cfg(feature = "molio")]
pub(crate) fn navigate_top_level_from_browser(
    scope: &mut v8::PinScope<'_, '_>,
    target: Option<String>,
) -> bool {
    if let Some(target) = target.as_ref()
        && navigate_top_level_same_document_from_browser(scope, target.clone(), false)
    {
        return true;
    }
    let window = scope.get_current_context().global(scope);
    let Some(location) = window_location_for_holder(scope, window) else {
        return false;
    };
    let kind = if target.is_some() {
        LocationNavigationKind::Assign
    } else {
        LocationNavigationKind::Reload
    };
    navigate_location_object(scope, location, kind, target);
    context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host_ptr| unsafe { &*host_ptr }.has_pending_location_navigation())
}

pub(crate) fn meta_refresh_navigation_kind(
    current_url: &url::Url,
    target_url: &url::Url,
    delay_ms: u32,
) -> LocationNavigationKind {
    if urls_equal_ignoring_fragment(current_url, target_url) {
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
    navigate_location_object_with_options(
        scope,
        location,
        kind,
        Some(target.to_string()),
        LocationNavigationOptions {
            force_exact_same_document_navigation: same_document_fragment,
            ..Default::default()
        },
    );
    context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host_ptr| unsafe { &*host_ptr }.has_pending_location_navigation())
        || (same_document_fragment
            && location_href_slot(scope, location).as_deref() == Some(target.as_str()))
}

/// Classify a delayed child refresh against the URL at execution time, including
/// exact fragment targets that must keep the current Document.
pub(crate) fn navigate_child_meta_refresh<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    target: &url::Url,
    delay_ms: u32,
) {
    let Some(current_href) = location_href_slot(scope, location) else {
        return;
    };
    let Ok(current_url) = url::Url::parse(&current_href) else {
        return;
    };
    let kind = meta_refresh_navigation_kind(&current_url, target, delay_ms);
    let same_document_fragment = kind == LocationNavigationKind::Assign
        && is_same_document_fragment_navigation(Some(&current_url), target);
    navigate_location_object_with_options(
        scope,
        location,
        kind,
        Some(target.to_string()),
        LocationNavigationOptions {
            dispatch_child_navigate_event_for_all_kinds: true,
            force_exact_same_document_navigation: same_document_fragment,
            ..Default::default()
        },
    );
}

pub(crate) fn navigate_location_object_with_source_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) {
    navigate_location_object_with_options(
        scope,
        location,
        kind,
        raw_target,
        LocationNavigationOptions {
            source_element,
            force_exact_same_document_navigation: true,
            ..Default::default()
        },
    );
}

pub(crate) fn navigate_location_object_with_child_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) {
    navigate_location_object_with_options(
        scope,
        location,
        kind,
        raw_target,
        LocationNavigationOptions {
            dispatch_child_navigate_event_for_all_kinds: true,
            ..Default::default()
        },
    );
}

fn navigate_location_object_with_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
    options: LocationNavigationOptions<'s>,
) {
    let LocationNavigationOptions {
        source_element,
        user_initiated,
        dispatch_child_navigate_event_for_all_kinds,
        force_exact_same_document_navigation,
    } = options;
    let owner = runtime_window_owner(scope, location);
    if let Some(context) = location.get_creation_context(scope)
        && let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(context)
    {
        let host = unsafe { &*host_ptr };
        if host
            .window_execution_context_identity_for_access_check(context)
            .is_none_or(|identity| !host.window_execution_context_identity_is_current(identity))
        {
            return;
        }
    }
    let location = window_location_for_holder(scope, owner).unwrap_or(location);
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
    // Entering the destination realm must not turn it into the initiator.
    // Freeze the incumbent Document before navigation events can run script.
    let navigation_source =
        if matches!(kind, LocationNavigationKind::Reload) && resolved.scheme() != "file" {
            None
        } else {
            capture_location_navigation_source(scope, owner, &resolved)
        };
    if resolved.scheme() == "javascript" {
        let source_context = scope
            .get_incumbent_context()
            .unwrap_or_else(|| scope.get_entered_or_microtask_context());
        let source_context = v8::Local::new(scope, source_context);
        if let Some(source_host) = crate::util::context_host_ptr_from_context_slot(source_context) {
            let host = unsafe { &mut *source_host };
            let dispatch = host
                .window_execution_context_identity_for_access_check(source_context)
                .map(|identity| identity.dispatch_scope())
                .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
            let scope = &mut v8::ContextScope::new(scope, source_context);
            if !host.allows_inline_javascript_navigation_by_csp(
                scope,
                dispatch,
                &crate::javascript_url::csp_source(&resolved),
            ) {
                return;
            }
        }
    }
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
            NavigationReloadAdmission::PendingInitialAttributeNavigation
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
        let child_handle = if runtime_window_is_global(scope, owner) {
            None
        } else {
            child_browsing_context_handle_for_runtime_owner(scope, owner)
        };
        let replaces_initial_about_blank =
            super::navigation_window::navigation_document_is_initial_empty(scope, owner)
                || child_handle.is_some_and(|handle| {
                    context_host_ptr_for_navigation_owner(scope, owner).is_some_and(|host_ptr| {
                        unsafe { &*host_ptr }.child_current_document_is_initial_empty(handle)
                    })
                });
        let effective_kind = match kind {
            LocationNavigationKind::Assign if replaces_initial_about_blank => {
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
            LocationNavigationKind::Assign => "push",
            LocationNavigationKind::Replace => "replace",
            LocationNavigationKind::Reload => "reload",
        };
        let mut navigate_outcome = navigation.map(|navigation| {
            let _ = cancel_active_navigation_event(scope, navigation);
            cancel_active_intercepted_same_document_navigation(scope, navigation);
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
                user_initiated,
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
        // Hyperlink fragment activation runs after navigate accepts it.
        // Intercepted navigation controls its own scroll timing.
        if source_element.is_some()
            && runtime_window_is_global(scope, owner)
            && !navigate_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.intercepted)
            && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
            && let Err(error) = crate::native_bridge::element::scroll_to_document_fragment_target(
                scope, host_ptr, &resolved,
            )
        {
            if let Some(message) = v8_string(
                scope,
                &format!("Layout failed while scrolling to fragment: {error}"),
            ) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
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
                host.record_same_document_navigation(&resolved, "fragment");
            }
        } else {
            sync_local_document_front_from_window(scope, owner);
        }
        if let Some(navigation) = navigation {
            if navigate_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.intercepted)
            {
                let outcome = navigate_outcome
                    .take()
                    .expect("checked intercepted outcome");
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
                    Some(resolved.as_str()),
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
    if let Some(handle) = child_handle {
        let file_source = if resolved.scheme() == "file" {
            let Some(source) = navigation_source else {
                return;
            };
            Some(source.initiator)
        } else {
            None
        };
        let is_javascript_url = resolved.scheme() == "javascript";
        let host_ptr = context_host_ptr_for_navigation_owner(scope, owner);
        let replaces_initial_empty_document = !is_javascript_url
            && matches!(kind, LocationNavigationKind::Assign)
            && host_ptr.is_some_and(|host_ptr| {
                unsafe { &*host_ptr }.child_current_document_is_initial_empty(handle)
            });
        let kind = if replaces_initial_empty_document {
            LocationNavigationKind::Replace
        } else {
            kind
        };
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
                user_initiated,
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
        if let Some(host_ptr) = host_ptr {
            let host = unsafe { &mut *host_ptr };
            if let Some(initiator) = file_source {
                host.queue_child_file_navigation_without_seed_update(
                    scope, handle, resolved, initiator,
                );
            } else if matches!(kind, LocationNavigationKind::Reload) {
                host.queue_child_browsing_context_reload_from_existing_seed(
                    scope,
                    handle,
                    resolved.as_str(),
                );
            } else {
                host.queue_child_browsing_context_navigation_without_seed_update(
                    scope,
                    handle,
                    resolved.as_str(),
                );
            }
        }
        return;
    }

    navigate_top_level_cross_document_location(
        scope,
        owner,
        Some(location),
        &current_href,
        resolved,
        kind,
        Some(LocationNavigateEvent {
            source_element,
            user_initiated,
            can_intercept: true,
        }),
        navigation_source,
    );
}

struct LocationNavigateEvent<'s> {
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    can_intercept: bool,
}

fn navigate_top_level_cross_document_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    location: Option<v8::Local<'s, v8::Object>>,
    current_href: &str,
    resolved: url::Url,
    kind: LocationNavigationKind,
    navigate_event: Option<LocationNavigateEvent<'s>>,
    navigation_source: Option<LocationNavigationSource>,
) {
    if let Some(event) = navigate_event
        && let Some(navigation) =
            super::navigation_window::window_navigation_for_holder(scope, owner)
    {
        let _ = cancel_active_navigation_event(scope, navigation);
        cancel_active_intercepted_same_document_navigation(scope, navigation);
        cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
        let navigation_type = match kind {
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
        let outcome = dispatch_navigation_navigate_event_with_outcome(
            scope,
            navigation,
            resolved.as_str(),
            navigation_type,
            false,
            false,
            event.can_intercept,
            event.user_initiated,
            None,
            destination_state,
            None,
            event.source_element,
        );
        if outcome.abort_error.is_some() {
            return;
        }
        if !outcome.proceed {
            finish_location_navigation_canceled(scope, navigation, &outcome, current_href);
            return;
        }
        cancel_pending_same_document_navigation_finishes(scope, navigation);
        if outcome.intercepted {
            commit_intercepted_location_navigation(
                scope,
                navigation,
                outcome,
                resolved.as_str(),
                kind,
                current_href,
            );
            return;
        }
    }

    if let Some(location) = location
        && resolved.scheme() != "javascript"
    {
        sync_location_object(scope, location, resolved.as_str());
    }

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let entry_seed = match kind {
        LocationNavigationKind::Assign => history_entry_seed_for_cross_document_location(
            scope,
            owner,
            &resolved,
            NavigationHistoryMutation::Push,
        ),
        LocationNavigationKind::Replace => history_entry_seed_for_cross_document_location(
            scope,
            owner,
            &resolved,
            NavigationHistoryMutation::Replace,
        ),
        LocationNavigationKind::Reload => history_entry_seed_for_reload(scope, owner),
    };
    unsafe { &mut *host_ptr }.record_pending_location_navigation_with_kind(
        resolved,
        entry_seed,
        if matches!(kind, LocationNavigationKind::Reload) {
            moli_fetch::BrowserNavigationRequestKind::Reload
        } else {
            moli_fetch::BrowserNavigationRequestKind::Navigate
        },
        navigation_source,
    );
}

/// Cross-origin Location functions are borrowable onto any concrete Location.
/// Validate the native wrapper instead of the function's original receiver.
pub(crate) fn is_native_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    crate::util::get_private_value(scope, receiver, WINDOW_LOCATION_HREF_SLOT).is_some()
}

pub(crate) fn navigate_borrowed_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    kind: LocationNavigationKind,
) -> bool {
    if !is_native_location(scope, receiver) {
        return false;
    }
    let raw = match crate::webidl::convert::<crate::webidl::UsvString>(
        scope,
        value,
        crate::webidl::Context::member("Location", "href"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return true;
        }
    };
    let Some(context) = receiver.get_creation_context(scope) else {
        return true;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    navigate_location_object_with_child_navigate_event(scope, receiver, kind, Some(raw));
    true
}

fn capture_location_navigation_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    url: &url::Url,
) -> Option<LocationNavigationSource> {
    if url.scheme() == "file" {
        let context = scope
            .get_incumbent_context()
            .unwrap_or_else(|| scope.get_entered_or_microtask_context());
        let host_ptr = crate::util::context_host_ptr_from_context_slot(context)?;
        let host = unsafe { &*host_ptr };
        let identity = host.window_execution_context_identity_for_access_check(context)?;
        if !host.window_execution_context_identity_is_current(identity) {
            return None;
        }
        return host.local_file_navigation_source(identity.dispatch_scope());
    }
    let source_context = scope.get_incumbent_context()?;
    let source_host = crate::util::context_host_ptr_from_context_slot(source_context)?;
    let target_host = crate::util::context_host_ptr_from_window_object(scope, target)?;
    // The ordinary same-realm path already carries its Document environment.
    if source_host == target_host {
        return None;
    }
    let source = source_context.global(scope);
    // Only the browsing context's fixed restrictions survive navigation.
    // Sandbox CSP from its old response belongs to the retiring Document.
    let target_sandbox = unsafe { &*target_host }
        .auxiliary_window()
        .map(|window| window.frame_sandbox());
    let scope = &mut v8::ContextScope::new(scope, source_context);
    let host = unsafe { &mut *source_host };
    let dispatch = host
        .window_execution_context_identity_for_access_check(source_context)?
        .dispatch_scope();
    let loader = host.document_resource_loader_for_dispatch_scope(dispatch)?;
    let initiator_url = host
        .document_referrer_source_url_for_dispatch_scope(dispatch)
        .unwrap_or_else(|| host.document_url().clone());
    let mut policy = match dispatch {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_policy_container_snapshot(handle)?
        }
        _ => host.document_policy_container().clone(),
    };
    policy.referrer_policy =
        current_document_referrer_policy(scope, source).or(policy.referrer_policy);
    let initiator = crate::runtime::RendererNavigationInitiator::new(
        initiator_url,
        loader.fetch_context().request_origin(),
        policy.referrer_policy.clone(),
        loader.request_client().has_document_file_access(),
    );
    policy.document_referrer = initiator.outgoing_referrer();
    let base_url = super::window_runtime::entered_window_api_base_url(scope, host);
    let child = match dispatch {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => Some(handle),
        _ => None,
    };
    match host.capture_blank_navigation_environment(
        scope,
        source,
        child,
        url.as_str(),
        base_url,
        policy,
        target_sandbox,
    ) {
        Ok(initial_document_environment) => Some(LocationNavigationSource {
            initiator,
            initial_document_environment,
        }),
        Err(error) => {
            tracing::debug!(%error, "Location initiator environment capture failed");
            None
        }
    }
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
    sandbox_blocks_ancestor_or_top_navigation_from_source(scope, source_handle, owner)
}

pub(crate) fn sandbox_blocks_ancestor_or_top_navigation_from_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source_handle: crate::document_runtime::DomHandle,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
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
    host: &PageScriptHost,
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

/// Returns whether the caller should queue a cross-document navigation.
/// Intercepted navigations commit and settle here without redispatching the event.
pub(crate) fn dispatch_top_level_navigation_event_with_source_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    navigation_type: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    can_intercept: bool,
    user_initiated: bool,
    download_request: Option<&str>,
) -> bool {
    let Some(navigation) = super::navigation_window::window_navigation_for_holder(scope, owner)
    else {
        return true;
    };
    let current_href = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let _ = cancel_active_navigation_event(scope, navigation);
    cancel_active_intercepted_same_document_navigation(scope, navigation);
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
    if outcome.intercepted {
        let kind = match navigation_type {
            "replace" => LocationNavigationKind::Replace,
            "reload" => LocationNavigationKind::Reload,
            _ => LocationNavigationKind::Assign,
        };
        commit_intercepted_location_navigation(
            scope,
            navigation,
            outcome,
            href,
            kind,
            &current_href,
        );
        return false;
    }
    true
}

pub(crate) fn dispatch_top_level_form_navigation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    navigation_type: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    form_data: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(navigation) = super::navigation_window::window_navigation_for_holder(scope, owner)
    else {
        return true;
    };
    let current_href = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let _ = cancel_active_navigation_event(scope, navigation);
    cancel_active_intercepted_same_document_navigation(scope, navigation);
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

fn finish_location_navigation_canceled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: &NavigationDispatchOutcome<'s>,
    href: &str,
) {
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    if let Some(signal) = outcome.signal {
        crate::native_bridge::abort::abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, href);
}

fn commit_intercepted_location_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: NavigationDispatchOutcome<'s>,
    href: &str,
    kind: LocationNavigationKind,
    current_href: &str,
) {
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let owner = runtime_window_owner(scope, navigation);
    let effective_href = outcome.redirected_url.as_deref().unwrap_or(href).to_owned();
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
    if let Some(location) = window_location_for_holder(scope, owner) {
        sync_location_object(scope, location, &effective_href);
    }
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
        host.record_same_document_navigation(&url, "fragment");
    }
    settle_location_intercepted_same_document_navigation(
        scope,
        navigation,
        outcome,
        transition_resolver,
        current_href,
    );
}

fn settle_location_intercepted_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: NavigationDispatchOutcome<'s>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    filename: &str,
) {
    let owner = runtime_window_owner(scope, navigation);
    let resolved_value = navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into());
    super::navigation_callbacks::settle_intercepted_same_document_navigation(
        scope,
        navigation,
        outcome,
        None,
        None,
        None,
        transition_resolver,
        resolved_value,
        filename,
    );
}

pub(crate) fn history_entry_seed_for_cross_document_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    resolved: &url::Url,
    mutation: NavigationHistoryMutation,
) -> Option<NavigationHistoryEntrySeed> {
    let history = window_history_for_holder(scope, owner)?;
    let current_index = history_index(scope, history);
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let mut entries = serialize_history_entries(scope, history);
    apply_current_document_referrer_policy_to_entry_snapshots(
        scope,
        owner,
        current_index,
        &mut entries,
    );
    let mut seed = cross_document_navigation_seed(
        entries,
        current_index,
        current_navigation_index,
        resolved,
        mutation,
    );
    super::session_history::capture_for_navigation(scope, owner, &mut seed);
    Some(seed)
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
