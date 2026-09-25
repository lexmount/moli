use super::history_runtime::cancel_pending_precommit_history_traversal;
use super::location_runtime::{
    is_same_document_fragment_navigation, location_has_relevant_document, location_href_slot,
    resolve_location_navigation_target, sync_location_object,
};
use super::navigation_activation::install_navigation_transition;
use super::navigation_callbacks::{
    cancel_active_intercepted_same_document_navigation,
    cancel_pending_precommit_same_document_navigation,
    queue_pending_precommit_same_document_navigation,
};
use super::navigation_entry::history_state_value;
use super::navigation_entry::{
    history_index, navigation_current_entry, navigation_current_entry_index,
};
use super::navigation_entry_state::clone_navigation_entry_state;
use super::navigation_events::{
    NavigationDispatchOutcome, cancel_active_navigation_event,
    dispatch_cross_document_navigation_navigate_event_for_window_with_type_and_form_data,
    dispatch_navigation_navigate_event_with_outcome, dispatch_popstate_event,
    finish_navigation_precommit, queue_hash_change_for_runtime_owner,
};
use super::navigation_lifecycle::finish_navigation_error_events;
use super::navigation_mutation::{
    apply_navigation_navigate_same_document, update_navigation_current_entry_for_same_document,
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
    child_browsing_context_handle_for_runtime_owner, navigation_document_has_disabled_entries,
    navigation_document_is_active, navigation_unload_event_active, runtime_window_is_global,
    runtime_window_owner, window_history_for_holder, window_location_for_holder,
};
use super::*;
use crate::native_bridge::NavigationHistoryEntrySeed;
use crate::util::context_host_ptr_from_window_object;
use crate::webidl;
use moli_page_types::{NavigationHistoryMutation, cross_document_navigation_seed};

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

#[derive(Default)]
struct LocationNavigationOptions {
    dispatch_child_navigate_event_for_all_kinds: bool,
    force_exact_same_document_navigation: bool,
    explicit_initiator_url: Option<url::Url>,
    user_initiated: bool,
    source_can_access_target: Option<bool>,
    hyperlink: bool,
    named_hyperlink_popup: Option<crate::native_bridge::element::NamedHyperlinkPopup>,
}

pub(crate) struct HyperlinkNavigationOptions {
    pub(crate) user_initiated: bool,
    pub(crate) source_can_access_target: bool,
    pub(crate) initiator_url: url::Url,
    pub(crate) named_popup: Option<crate::native_bridge::element::NamedHyperlinkPopup>,
}

pub(crate) fn navigate_location_object_for_hyperlink<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    href: &str,
    source_element: v8::Local<'s, v8::Object>,
    options: HyperlinkNavigationOptions,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        LocationNavigationKind::Assign,
        Some(href.to_owned()),
        Some(source_element),
        LocationNavigationOptions {
            user_initiated: options.user_initiated,
            source_can_access_target: Some(options.source_can_access_target),
            hyperlink: true,
            force_exact_same_document_navigation: true,
            explicit_initiator_url: Some(options.initiator_url),
            named_hyperlink_popup: options.named_popup,
            ..LocationNavigationOptions::default()
        },
    );
}

pub(crate) fn navigate_location_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) {
    let owner = runtime_window_owner(scope, location);
    let replaces_unloaded_document = kind == LocationNavigationKind::Assign
        && location_document_is_before_load_complete(scope, owner)
        && !context_host_ptr_for_navigation_owner(scope, owner).is_some_and(|host| {
            let host = unsafe { &*host };
            host.protocol_user_gesture_activation()
                || host.window_has_transient_user_activation(location_navigation_initiator_scope(
                    scope, host,
                ))
        });
    let kind = if replaces_unloaded_document {
        LocationNavigationKind::Replace
    } else {
        kind
    };
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        kind,
        raw_target,
        None,
        LocationNavigationOptions {
            dispatch_child_navigate_event_for_all_kinds: replaces_unloaded_document,
            ..LocationNavigationOptions::default()
        },
    );
}

pub(crate) fn navigate_location_object_for_form_fragment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    href: &str,
    kind: LocationNavigationKind,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        kind,
        Some(href.to_owned()),
        source_element,
        LocationNavigationOptions {
            user_initiated,
            source_can_access_target: Some(source_element.is_some()),
            force_exact_same_document_navigation: true,
            ..LocationNavigationOptions::default()
        },
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
        LocationNavigationOptions {
            force_exact_same_document_navigation: true,
            ..LocationNavigationOptions::default()
        },
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
        LocationNavigationOptions {
            force_exact_same_document_navigation: same_document_fragment,
            ..LocationNavigationOptions::default()
        },
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
        LocationNavigationOptions::default(),
    );
}

pub(crate) fn navigate_location_object_with_child_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        kind,
        raw_target,
        None,
        LocationNavigationOptions {
            dispatch_child_navigate_event_for_all_kinds: true,
            ..LocationNavigationOptions::default()
        },
    );
}

pub(crate) fn navigate_location_object_with_child_navigate_event_and_initiator_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
    initiator_url: url::Url,
) {
    navigate_location_object_with_source_element_and_child_navigate_event(
        scope,
        location,
        kind,
        raw_target,
        None,
        LocationNavigationOptions {
            dispatch_child_navigate_event_for_all_kinds: true,
            explicit_initiator_url: Some(initiator_url),
            ..LocationNavigationOptions::default()
        },
    );
}

fn navigate_location_object_with_source_element_and_child_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
    source_element: Option<v8::Local<'s, v8::Object>>,
    options: LocationNavigationOptions,
) {
    // The Location setters and methods return before URL parsing when their
    // relevant Document is null, including when argument conversion removed it.
    if !location_has_relevant_document(scope, location) {
        return;
    }
    let owner = runtime_window_owner(scope, location);
    let location = window_location_for_holder(scope, owner).unwrap_or(location);
    let LocationNavigationOptions {
        dispatch_child_navigate_event_for_all_kinds,
        force_exact_same_document_navigation,
        explicit_initiator_url,
        user_initiated,
        source_can_access_target,
        hyperlink,
        named_hyperlink_popup,
    } = options;
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
    let owner = runtime_window_owner(scope, location);
    let source_can_access_target = source_can_access_target.unwrap_or_else(|| {
        let Some(host_ptr) = context_host_ptr_for_navigation_owner(scope, owner) else {
            return true;
        };
        let Some(target) = super::navigation_window::runtime_window_dispatch_scope(scope, owner)
        else {
            return true;
        };
        let host = unsafe { &*host_ptr };
        host.window_scopes_have_same_origin_domain(
            location_navigation_initiator_scope(scope, host),
            target,
        )
    });
    let event_source_element = source_element.filter(|_| source_can_access_target);
    if navigation_unload_event_active(scope, owner)
        || super::navigation_cancellation::window_navigation_is_stopping(scope, owner)
    {
        return;
    }
    if blocks_ancestor_location_navigation(scope, owner, &resolved) {
        crate::context_bootstrap::throw_dom_exception_value(
            scope,
            "The source frame is not allowed to navigate this ancestor browsing context.",
            "SecurityError",
        );
        return;
    }
    if !matches!(kind, LocationNavigationKind::Reload)
        && exact_same_href
        && source_element.is_none()
        && raw_target_is_fragment_only
        && !force_exact_same_document_navigation
    {
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
    let popup_id = crate::native_bridge::lightweight_popup_id_from_window(scope, owner);
    let kind = if kind == LocationNavigationKind::Assign
        && exact_same_href
        && popup_id.is_some_and(|popup_id| {
            context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
                let host = unsafe { &*host_ptr };
                host.window_scopes_have_same_origin(
                    location_navigation_initiator_scope(scope, host),
                    crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id),
                )
            })
        }) {
        LocationNavigationKind::Replace
    } else {
        kind
    };
    if !matches!(kind, LocationNavigationKind::Reload)
        && (popup_id.is_none() || resolved.fragment().is_some())
        && (!exact_same_href
            || force_exact_same_document_navigation
            || (popup_id.is_some() && resolved.fragment().is_some()))
        && is_same_document_fragment_navigation(current_url.as_ref(), &resolved)
    {
        let popup_document_owner = popup_id.and_then(|popup_id| {
            context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
                unsafe { &*host_ptr }.current_lightweight_popup_document_owner(popup_id)
            })
        });
        let entries_disabled = navigation_document_has_disabled_entries(scope, owner);
        let navigation = if entries_disabled {
            None
        } else {
            super::navigation_window::window_navigation_for_holder(scope, owner)
        };
        let child_handle = if runtime_window_is_global(scope, owner) {
            None
        } else {
            child_browsing_context_handle_for_runtime_owner(scope, owner)
        };
        let replaces_initial_about_blank = child_handle.is_some_and(|handle| {
            context_host_ptr_for_navigation_owner(scope, owner).is_some_and(|host_ptr| {
                unsafe { &*host_ptr }.child_current_document_is_initial_empty(handle)
            })
        }) || popup_id.is_some_and(|popup_id| {
            context_host_ptr_for_navigation_owner(scope, owner).is_some_and(|host_ptr| {
                unsafe { &*host_ptr }.lightweight_popup_current_document_is_initial_empty(popup_id)
            })
        });
        let effective_kind = match kind {
            LocationNavigationKind::Assign if replaces_initial_about_blank => {
                LocationNavigationKind::Replace
            }
            LocationNavigationKind::Assign if source_element.is_some() && exact_same_href => {
                LocationNavigationKind::Replace
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
            cancel_pending_precommit_same_document_navigation(scope, navigation);
            cancel_pending_precommit_history_traversal(scope, navigation);
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
                event_source_element,
            )
        });
        if navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.abort_error)
            .is_some()
        {
            return;
        }
        // Detect a close/removal from navigate handlers. Disabled entries have
        // no such callback, and a popup's retained Document can still accept a
        // fragment change before its queued close task retires the Document.
        if !location_has_relevant_document(scope, location)
            || (navigate_outcome.is_some() && !navigation_document_is_active(scope, owner))
        {
            return;
        }
        if let Some(document_owner) = popup_document_owner
            && !context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
                unsafe { &*host_ptr }.lightweight_popup_document_owner_is_current(document_owner)
            })
        {
            return;
        }
        if navigate_outcome
            .as_ref()
            .is_some_and(|outcome| !outcome.proceed)
        {
            return;
        }
        // Preserve synchronous fragment activation only after the navigate event
        // accepts it. Intercepted navigation owns its scroll timing separately.
        if hyperlink
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
        if let Some(outcome) = navigate_outcome.as_ref()
            && let Some(event) = outcome.precommit_event
        {
            let redirected_kind = match outcome.redirected_history.as_deref() {
                Some("replace") => LocationNavigationKind::Replace,
                Some("push") => LocationNavigationKind::Assign,
                _ => effective_kind,
            };
            if queue_pending_precommit_same_document_navigation(
                scope,
                owner,
                event,
                outcome,
                &current_href,
                outcome
                    .redirected_url
                    .as_deref()
                    .unwrap_or(resolved.as_str()),
                redirected_kind,
                None,
                None,
                None,
                None,
                None,
            ) {
                return;
            }
            finish_navigation_precommit(scope, event);
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
        let intercepted = navigate_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.intercepted);
        let protocol_navigation_type = if intercepted { "other" } else { "fragment" };
        sync_location_object(scope, location, resolved.as_str());
        if entries_disabled {
            apply_navigation_navigate_same_document(
                scope,
                owner,
                resolved.as_str(),
                effective_kind,
                None,
                protocol_navigation_type,
            );
        } else {
            update_navigation_current_entry_for_same_document(
                scope,
                owner,
                resolved.as_str(),
                effective_kind,
                protocol_navigation_type,
            );
        }
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            if !location_has_relevant_document(scope, location)
                || !navigation_document_is_active(scope, owner)
            {
                return;
            }
            if let Some(document_owner) = popup_document_owner
                && !unsafe { &*host_ptr }
                    .lightweight_popup_document_owner_is_current(document_owner)
            {
                return;
            }
            let state = window_history_for_holder(scope, owner)
                .map(|history| history_state_value(scope, history))
                .unwrap_or_else(|| v8::null(scope).into());
            dispatch_popstate_event(scope, host_ptr, owner, state);
            if !location_has_relevant_document(scope, location)
                || !navigation_document_is_active(scope, owner)
            {
                return;
            }
            if let Some(document_owner) = popup_document_owner
                && !unsafe { &*host_ptr }
                    .lightweight_popup_document_owner_is_current(document_owner)
            {
                return;
            }
            queue_hash_change_for_runtime_owner(
                scope,
                owner,
                Some(&current_href),
                resolved.as_str(),
            );
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

    if let Some(popup_id) = popup_id
        && resolved.scheme() == "javascript"
    {
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            if let Some(popup) = named_hyperlink_popup {
                popup.navigate(scope, host_ptr, resolved.as_str());
            } else {
                let _ = unsafe { &mut *host_ptr }
                    .navigate_lightweight_popup_window_to_url(scope, popup_id, resolved, kind);
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
        && source_can_access_target
        && let Some(navigation) =
            super::navigation_window::window_navigation_for_holder(scope, owner)
    {
        if popup_id.is_some() {
            super::navigation_result::cancel_active_cross_document_navigation(
                scope, navigation, None,
            );
        }
        let _ = cancel_active_navigation_event(scope, navigation);
        cancel_pending_precommit_same_document_navigation(scope, navigation);
        cancel_pending_precommit_history_traversal(scope, navigation);
        cancel_active_intercepted_same_document_navigation(scope, navigation);
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
        let outcome = dispatch_navigation_navigate_event_with_outcome(
            scope,
            navigation,
            resolved.as_str(),
            navigation_type,
            false,
            false,
            current_url.as_ref().is_some_and(|current| {
                super::history_mutation::document_can_have_url_rewritten(current, &resolved)
            }),
            user_initiated,
            None,
            destination_state,
            None,
            event_source_element,
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
            if let Some(event) = outcome.precommit_event {
                if queue_pending_precommit_same_document_navigation(
                    scope,
                    owner,
                    event,
                    &outcome,
                    &current_href,
                    &effective_href,
                    effective_kind,
                    None,
                    None,
                    None,
                    None,
                    None,
                ) {
                    return;
                }
                finish_navigation_precommit(scope, event);
            }
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
                "other",
            );
            settle_location_intercepted_same_document_navigation(
                scope,
                navigation,
                outcome,
                transition_resolver,
                &current_href,
            );
            return;
        }
        if popup_id.is_some() {
            super::navigation_result::track_cross_document_location_navigation(
                scope,
                navigation,
                outcome.signal,
                resolved.as_str(),
            );
        }
    }

    if let Some(popup_id) = popup_id {
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            if let Some(popup) = named_hyperlink_popup {
                popup.navigate(scope, host_ptr, resolved.as_str());
            } else {
                let _ = unsafe { &mut *host_ptr }
                    .navigate_lightweight_popup_window_to_url(scope, popup_id, resolved, kind);
            }
        }
        return;
    }

    if let Some(handle) = child_handle {
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
            && source_can_access_target
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
                event_source_element,
                user_initiated,
                None,
                None,
            )
        {
            return;
        }
        if let Some(host_ptr) = host_ptr {
            let host = unsafe { &mut *host_ptr };
            let initiator_url =
                explicit_initiator_url.or_else(|| location_navigation_initiator_url(scope, host));
            if is_javascript_url {
                let initiator = location_navigation_initiator_scope(scope, host);
                host.queue_child_browsing_context_navigation_without_seed_update(
                    handle,
                    resolved.as_str(),
                    initiator_url,
                    Some(initiator),
                );
            } else {
                // A cross-document navigation only prepares the next history
                // entry. The old Document's Location, history state and entry
                // identity remain visible until the replacement commits.
                let entry_seed = if matches!(kind, LocationNavigationKind::Reload) {
                    history_entry_seed_for_reload(scope, owner)
                } else {
                    history_entry_seed_for_cross_document_location(scope, owner, &resolved, kind)
                };
                if let Some(entry_seed) = entry_seed
                    && host.queue_deferred_child_browsing_context_navigation_from_entry_seed(
                        handle,
                        resolved.as_str(),
                        entry_seed,
                        initiator_url,
                    )
                    && let Some(navigation_load) = host.current_child_navigation_load(handle)
                {
                    host.check_child_navigation_beforeunload(scope, handle, navigation_load);
                }
            }
        }
        return;
    }

    // A new top-level hyperlink leaves the source Location and history intact
    // until the browser commits the response, just as the activation path did.
    let browser_owned_hyperlink = hyperlink && !exact_same_href;
    if !browser_owned_hyperlink && resolved.scheme() != "javascript" {
        sync_location_object(scope, location, resolved.as_str());
    }

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let entry_seed = if browser_owned_hyperlink {
        None
    } else if matches!(kind, LocationNavigationKind::Reload) {
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

fn blocks_ancestor_location_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    destination: &url::Url,
) -> bool {
    let Some(host_ptr) = context_host_ptr_for_navigation_owner(scope, owner)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
    else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    let Some(target) = super::navigation_window::runtime_window_dispatch_scope(scope, owner) else {
        return false;
    };
    host.blocks_ancestor_navigation(
        location_navigation_initiator_scope(scope, host),
        target,
        destination,
    )
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

fn location_document_is_before_load_complete<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_for_navigation_owner(scope, owner) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    super::window_accessors::window_document_handle(scope, owner, host)
        .is_some_and(|document| host.document_is_completely_loaded(document) == Some(false))
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
    kind: LocationNavigationKind,
) -> Option<NavigationHistoryEntrySeed> {
    let history = window_history_for_holder(scope, owner)?;
    let current_index = history_index(scope, history);
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let mutation = match kind {
        LocationNavigationKind::Assign => NavigationHistoryMutation::Push,
        LocationNavigationKind::Replace => NavigationHistoryMutation::Replace,
        LocationNavigationKind::Reload => return history_entry_seed_for_reload(scope, owner),
    };
    let mut seed = cross_document_navigation_seed(
        serialize_history_entries(scope, history),
        current_index,
        current_navigation_index,
        resolved,
        mutation,
    );
    super::session_history::capture_for_navigation(scope, owner, &mut seed);
    Some(seed)
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

fn location_navigation_initiator_url(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
) -> Option<url::Url> {
    match location_navigation_initiator_scope(scope, host) {
        crate::native_bridge::OwnerDispatchScope::Top => Some(host.document_url().clone()),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_current_url(handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.lightweight_popup_document_url(popup_id)
        }
    }
}

fn location_navigation_initiator_scope(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
) -> crate::native_bridge::OwnerDispatchScope {
    let incumbent = scope
        .get_incumbent_context()
        .and_then(|context| host.window_dispatch_scope_for_context(scope, context));
    // A popup can share its opener's concrete realm. Preserve its own source
    // identity, as well as that of a real child calling an ancestor's binding.
    if let Some(
        source @ (crate::native_bridge::OwnerDispatchScope::Child(_)
        | crate::native_bridge::OwnerDispatchScope::LightweightPopup(_)),
    ) = incumbent
    {
        return source;
    }
    if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
        return crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id);
    }
    incumbent
        .or_else(|| {
            crate::context_bootstrap::current_child_browsing_context_handle_for_runtime_scope(scope)
                .map(crate::native_bridge::OwnerDispatchScope::Child)
        })
        .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top)
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
