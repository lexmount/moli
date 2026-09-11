use super::super::navigation_activation::{
    clear_navigation_transition, install_navigation_transition,
    reject_navigation_transition_committed, resolve_navigation_transition_committed,
};
use super::super::navigation_events::dispatch_popstate_event;
use super::super::navigation_lifecycle::{
    begin_navigation_attempt, cancel_navigation_attempt, complete_navigation_attempt,
    finish_navigation_error_events, navigation_attempt_id_from_slot, navigation_attempt_is_active,
    settle_navigation_finished_rejected_after_reactions,
    settle_navigation_transition_finished_local,
};
use super::super::navigation_window::{
    navigation_document_can_update_current_entry, navigation_document_is_active,
    navigation_unload_event_active,
};
use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_url_policy::{LocalFileNavigationAccess, route_navigation_url};
use moli_webapi_declare::WebApiObject;

const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionResolver";

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigationNavigateOptions")]
struct NavigationNavigateOptionsMembers {
    #[webidl(name = "history", converter = "enum", default = NavigationNavigateHistoryKind::Default)]
    history: NavigationNavigateHistoryKind,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigation.navigate")]
struct NavigationNavigateArgs {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to execute 'navigate' on 'Navigation': 1 argument required, but only 0 present."
    )]
    url: String,
}

pub(in crate::context_bootstrap) fn navigation_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let navigation = args.this();
    let owner = runtime_window_owner(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    }
    let Some(history) = window_history_for_holder(scope, owner) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let Some(entries) = history_entries(scope, history) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let current_entry = navigation_current_entry(scope, owner);
    let copied = build_visible_navigation_entries_array(scope, entries, current_entry);
    rv.set(copied.into());
}

fn parse_navigation_navigate_history_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<'s, v8::Object>>,
) -> Option<NavigationNavigateHistoryKind> {
    let Some(options) = options else {
        return Some(NavigationNavigateHistoryKind::Default);
    };
    match webidl::parse_dictionary_object::<NavigationNavigateOptionsMembers>(scope, options) {
        Ok(parsed) => Some(parsed.history),
        Err(_) => {
            let value = webidl::property(scope, options, "history")
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            throw_navigation_type_error(
                scope,
                &format!(
                    "Failed to execute 'navigate' on 'Navigation': Failed to read the 'history' property from 'NavigationNavigateOptions': The provided value '{value}' is not a valid enum value of type NavigationHistoryBehavior."
                ),
            );
            None
        }
    }
}

fn resolve_navigation_navigate_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_href: &str,
    raw_target: String,
) -> Option<url::Url> {
    if raw_target.is_empty() {
        return url::Url::parse(current_href).ok();
    }
    let base = navigation_document_base_url(scope, owner, current_href)?;
    base.join(&raw_target).ok()
}

pub(in crate::context_bootstrap) fn navigation_navigate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NavigationNavigateArgs>(scope, &args) else {
        return;
    };
    let owner = runtime_window_owner(scope, args.this());
    let Some(location) = window_location_for_holder(scope, owner) else {
        return;
    };
    let current_href = location_href_slot(scope, location).unwrap_or_default();
    let raw_url = parsed.url;
    let Some(next_url) =
        resolve_navigation_navigate_target(scope, owner, &current_href, raw_url.clone())
    else {
        rv.set(
            navigation_rejected_dom_exception_result(scope, "Invalid URL", "SyntaxError").into(),
        );
        return;
    };
    let options = args.get(1).to_object(scope);
    let Some(navigate_history_kind) = parse_navigation_navigate_history_kind(scope, options) else {
        return;
    };
    let cloned_navigation_state = match clone_navigation_state_arg_for_result(scope, options) {
        Ok(state) => state,
        Err(error) => {
            rv.set(navigation_rejected_value_result(scope, error).into());
            return;
        }
    };
    if !navigation_document_is_active(scope, owner) {
        rv.set(
            navigation_rejected_dom_exception_result(
                scope,
                "Navigation current entry cannot be updated for a non-fully-active document.",
                "InvalidStateError",
            )
            .into(),
        );
        return;
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        rv.set(navigation_pending_result(scope).into());
        return;
    }
    if navigation_unload_event_active(scope, owner) {
        rv.set(
            navigation_rejected_invalid_state_result(
                scope,
                "Navigation was canceled because the document is unloading.",
            )
            .into(),
        );
        return;
    }
    if next_url.scheme() == "javascript" {
        rv.set(
            navigation_rejected_dom_exception_result(
                scope,
                "Navigation to javascript: URLs is not supported.",
                "NotSupportedError",
            )
            .into(),
        );
        return;
    }
    if route_navigation_url(&next_url, LocalFileNavigationAccess::Denied).is_err() {
        rv.set(
            navigation_rejected_dom_exception_result(
                scope,
                "Navigation was canceled.",
                "AbortError",
            )
            .into(),
        );
        return;
    }
    if current_href == "about:blank"
        && raw_url.starts_with('#')
        && matches!(navigate_history_kind, NavigationNavigateHistoryKind::Push)
    {
        rv.set(
            navigation_rejected_dom_exception_result(
                scope,
                "Cannot push a same-document navigation in the initial about:blank document",
                "NotSupportedError",
            )
            .into(),
        );
        return;
    }
    if current_href == "about:blank"
        && raw_url.starts_with('#')
        && matches!(
            navigate_history_kind,
            NavigationNavigateHistoryKind::Default
        )
    {
        rv.set(navigation_pending_result(scope).into());
        return;
    }
    let navigation_info = options.and_then(|options| {
        options
            .get(scope, v8str(scope, "info").into())
            .filter(|value| !value.is_undefined())
    });
    let same_document_kind = match navigate_history_kind {
        NavigationNavigateHistoryKind::Replace => LocationNavigationKind::Replace,
        NavigationNavigateHistoryKind::Default if next_url.as_str() == current_href => {
            LocationNavigationKind::Replace
        }
        NavigationNavigateHistoryKind::Default | NavigationNavigateHistoryKind::Push => {
            LocationNavigationKind::Assign
        }
    };
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let current_url = url::Url::parse(&current_href).ok();
    let can_update_current_entry = navigation_document_can_update_current_entry(scope, owner);
    let exact_same_url_push = matches!(navigate_history_kind, NavigationNavigateHistoryKind::Push)
        && next_url.as_str() == current_href;
    if can_update_current_entry
        && is_same_document_fragment_navigation(current_url.as_ref(), &next_url)
        && !exact_same_url_push
    {
        let navigation_for_event = window_navigation_for_holder(scope, owner);
        let canceled_cross_document = if let Some(navigation) = navigation_for_event {
            let _ = cancel_active_navigation_event(scope, navigation);
            cancel_pending_precommit_same_document_navigation(scope, navigation);
            cancel_pending_precommit_history_traversal(scope, navigation);
            cancel_active_intercepted_same_document_navigation(scope, navigation);
            let canceled =
                cancel_active_cross_document_navigation(scope, navigation, Some(next_url.as_str()));
            cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
            canceled
        } else {
            false
        };
        let mut navigate_outcome = navigation_for_event.map(|navigation| {
            dispatch_navigation_navigate_event_with_outcome(
                scope,
                navigation,
                next_url.as_str(),
                match same_document_kind {
                    LocationNavigationKind::Assign => "push",
                    LocationNavigationKind::Replace => "replace",
                    LocationNavigationKind::Reload => "reload",
                },
                should_dispatch_hash_change(&current_href, next_url.as_str()),
                true,
                true,
                false,
                None,
                cloned_navigation_state,
                navigation_info,
                None,
            )
        });
        if let Some(error) = navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.abort_error)
        {
            rv.set(navigation_rejected_value_result(scope, error).into());
            return;
        }
        if let Some(error) = navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.precommit_error)
        {
            if let Some(navigation) = navigation_for_event {
                finish_navigation_error_events(scope, navigation, error, &current_href);
            }
            rv.set(
                navigation_rejected_dom_exception_result(
                    scope,
                    "Navigation was canceled before commit",
                    "AbortError",
                )
                .into(),
            );
            return;
        }
        if !navigation_document_is_active(scope, owner) {
            if let Some(navigation) = navigation_for_event
                && let Some(outcome) = navigate_outcome.as_ref()
            {
                rv.set(
                    navigation_canceled_after_dispatch_result(
                        scope,
                        navigation,
                        outcome,
                        &current_href,
                    )
                    .into(),
                );
            } else {
                rv.set(
                    navigation_rejected_dom_exception_result(
                        scope,
                        "Navigation was canceled",
                        "AbortError",
                    )
                    .into(),
                );
            }
            return;
        }
        if navigate_outcome
            .as_ref()
            .is_some_and(|outcome| !outcome.proceed)
        {
            if let Some(navigation) = navigation_for_event
                && let Some(outcome) = navigate_outcome.as_ref()
            {
                rv.set(
                    navigation_canceled_after_dispatch_result(
                        scope,
                        navigation,
                        outcome,
                        &current_href,
                    )
                    .into(),
                );
            } else {
                rv.set(
                    navigation_rejected_dom_exception_result(
                        scope,
                        "Navigation was canceled",
                        "AbortError",
                    )
                    .into(),
                );
            }
            return;
        }
        if let Some(navigation) = navigation_for_event {
            cancel_pending_same_document_navigation_finishes(scope, navigation);
        }
        let effective_href = navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.redirected_url.as_deref())
            .unwrap_or(next_url.as_str())
            .to_owned();
        let effective_kind = navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.redirected_history.as_deref())
            .map(|history| match history {
                "replace" => LocationNavigationKind::Replace,
                _ => LocationNavigationKind::Assign,
            })
            .unwrap_or(same_document_kind);
        let effective_state = navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.redirected_state)
            .or(cloned_navigation_state);
        if effective_href == current_href
            && matches!(effective_kind, LocationNavigationKind::Replace)
            && !runtime_window_is_global(scope, owner)
            && !navigate_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.intercepted)
        {
            rv.set(handle_navigation_navigate_cross_document(
                scope,
                owner,
                history,
                &next_url,
                NavigationNavigateHistoryKind::Replace,
                navigation_for_event.map(|navigation| {
                    (
                        navigation,
                        navigate_outcome.as_ref().and_then(|outcome| outcome.signal),
                    )
                }),
            ));
            return;
        }
        if let Some(outcome) = navigate_outcome.as_ref()
            && let Some(precommit_event) = outcome.precommit_event
            && let Some(pending) = navigation_result_with_pending_commit(scope)
            && queue_pending_precommit_same_document_navigation(
                scope,
                owner,
                precommit_event,
                outcome,
                &current_href,
                &effective_href,
                effective_kind,
                effective_state,
                pending.committed_resolve,
                pending.committed_reject,
                pending.finished_resolve,
                pending.finished_reject,
            )
        {
            rv.set(pending.object.into());
            return;
        }
        let transition_from = navigate_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.intercepted)
            .then(|| navigation_current_entry(scope, owner))
            .flatten();
        let transition_to = navigate_outcome
            .as_ref()
            .and_then(|outcome| outcome.destination);
        let transition_resolver =
            if let Some(navigation) = window_navigation_for_holder(scope, owner) {
                transition_from.and_then(|from| {
                    install_navigation_transition(
                        scope,
                        navigation,
                        from,
                        transition_to,
                        match effective_kind {
                            LocationNavigationKind::Assign => "push",
                            LocationNavigationKind::Replace => "replace",
                            LocationNavigationKind::Reload => "reload",
                        },
                    )
                })
            } else {
                None
            };
        let resolved_value = commit_navigation_navigate_same_document(
            scope,
            owner,
            &current_href,
            &effective_href,
            effective_kind,
            effective_state,
        );
        if let Some(outcome) = navigate_outcome.as_mut()
            && let Some(precommit_event) = outcome.precommit_event
        {
            let (intercept_error, intercept_result) =
                run_navigation_precommit_deferred_handlers(scope, precommit_event);
            outcome.intercept_error = intercept_error;
            outcome.intercept_result = intercept_result.or(outcome.intercept_result);
        }
        let intercepted = navigate_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.intercepted);
        let result = if intercepted {
            let Some(pending) = navigation_current_entry_result_with_pending_finished(scope, owner)
            else {
                let result = navigation_immediate_result_with_value(scope, resolved_value);
                rv.set(result.into());
                return;
            };
            if let Some(navigation) = window_navigation_for_holder(scope, owner)
                && let Some(outcome) = navigate_outcome
            {
                settle_intercepted_same_document_navigation(
                    scope,
                    navigation,
                    outcome,
                    Some(pending.committed_resolve),
                    pending.finished_resolve,
                    pending.finished_reject,
                    transition_resolver,
                    pending.resolved_value,
                    &current_href,
                );
            }
            pending.object
        } else if let Some(navigation) = window_navigation_for_holder(scope, owner) {
            let signal = navigate_outcome.as_ref().and_then(|outcome| outcome.signal);
            if canceled_cross_document {
                navigation_current_entry_result_with_task_finished(
                    scope,
                    owner,
                    navigation,
                    signal,
                    &effective_href,
                )
            } else {
                navigation_current_entry_result_with_deferred_finished(
                    scope,
                    owner,
                    navigation,
                    signal,
                    &effective_href,
                )
            }
        } else {
            navigation_immediate_result_with_value(scope, resolved_value)
        };
        rv.set(result.into());
        return;
    }
    if can_update_current_entry && let Some(navigation) = window_navigation_for_holder(scope, owner)
    {
        let _ = cancel_active_navigation_event(scope, navigation);
        cancel_pending_precommit_same_document_navigation(scope, navigation);
        cancel_pending_precommit_history_traversal(scope, navigation);
        cancel_active_intercepted_same_document_navigation(scope, navigation);
        cancel_active_cross_document_navigation(scope, navigation, Some(next_url.as_str()));
        cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
        let cross_document_kind = match navigate_history_kind {
            NavigationNavigateHistoryKind::Replace => LocationNavigationKind::Replace,
            NavigationNavigateHistoryKind::Default if next_url.as_str() == current_href => {
                LocationNavigationKind::Replace
            }
            NavigationNavigateHistoryKind::Default | NavigationNavigateHistoryKind::Push => {
                LocationNavigationKind::Assign
            }
        };
        let mut outcome = dispatch_navigation_navigate_event_with_outcome(
            scope,
            navigation,
            next_url.as_str(),
            match cross_document_kind {
                LocationNavigationKind::Assign => "push",
                LocationNavigationKind::Replace => "replace",
                LocationNavigationKind::Reload => "reload",
            },
            false,
            false,
            true,
            false,
            None,
            cloned_navigation_state,
            navigation_info,
            None,
        );
        if let Some(error) = outcome.abort_error {
            rv.set(navigation_rejected_value_result(scope, error).into());
            return;
        }
        if let Some(error) = outcome.precommit_error {
            finish_navigation_error_events(scope, navigation, error, &current_href);
            rv.set(
                navigation_rejected_dom_exception_result(
                    scope,
                    "Navigation was canceled before commit",
                    "AbortError",
                )
                .into(),
            );
            return;
        }
        if !navigation_document_is_active(scope, owner) {
            rv.set(
                navigation_canceled_after_dispatch_result(
                    scope,
                    navigation,
                    &outcome,
                    &current_href,
                )
                .into(),
            );
            return;
        }
        if !outcome.proceed {
            rv.set(
                navigation_canceled_after_dispatch_result(
                    scope,
                    navigation,
                    &outcome,
                    &current_href,
                )
                .into(),
            );
            return;
        }
        if outcome.intercepted {
            let effective_href = outcome
                .redirected_url
                .as_deref()
                .unwrap_or(next_url.as_str())
                .to_owned();
            let effective_kind = outcome
                .redirected_history
                .as_deref()
                .map(|history| match history {
                    "replace" => LocationNavigationKind::Replace,
                    _ => LocationNavigationKind::Assign,
                })
                .unwrap_or(cross_document_kind);
            let effective_state = outcome.redirected_state.or(cloned_navigation_state);
            if let Some(precommit_event) = outcome.precommit_event
                && let Some(pending) = navigation_result_with_pending_commit(scope)
                && queue_pending_precommit_same_document_navigation(
                    scope,
                    owner,
                    precommit_event,
                    &outcome,
                    &current_href,
                    &effective_href,
                    effective_kind,
                    effective_state,
                    pending.committed_resolve,
                    pending.committed_reject,
                    pending.finished_resolve,
                    pending.finished_reject,
                )
            {
                rv.set(pending.object.into());
                return;
            }
            let transition_from = navigation_current_entry(scope, owner);
            let transition_to = outcome.destination;
            let transition_resolver = transition_from.and_then(|from| {
                install_navigation_transition(
                    scope,
                    navigation,
                    from,
                    transition_to,
                    match effective_kind {
                        LocationNavigationKind::Assign => "push",
                        LocationNavigationKind::Replace => "replace",
                        LocationNavigationKind::Reload => "reload",
                    },
                )
            });
            let resolved_value = commit_navigation_navigate_same_document(
                scope,
                owner,
                &current_href,
                &effective_href,
                effective_kind,
                effective_state,
            );
            if let Some(precommit_event) = outcome.precommit_event {
                let (intercept_error, intercept_result) =
                    run_navigation_precommit_deferred_handlers(scope, precommit_event);
                outcome.intercept_error = intercept_error;
                outcome.intercept_result = intercept_result.or(outcome.intercept_result);
            }
            let Some(pending) = navigation_current_entry_result_with_pending_finished(scope, owner)
            else {
                let result = navigation_immediate_result_with_value(scope, resolved_value);
                rv.set(result.into());
                return;
            };
            settle_intercepted_same_document_navigation(
                scope,
                navigation,
                outcome,
                Some(pending.committed_resolve),
                pending.finished_resolve,
                pending.finished_reject,
                transition_resolver,
                pending.resolved_value,
                &current_href,
            );
            rv.set(pending.object.into());
            return;
        }
        rv.set(handle_navigation_navigate_cross_document(
            scope,
            owner,
            history,
            &next_url,
            navigate_history_kind,
            Some((navigation, outcome.signal)),
        ));
        return;
    }
    rv.set(handle_navigation_navigate_cross_document(
        scope,
        owner,
        history,
        &next_url,
        navigate_history_kind,
        None,
    ));
}

fn throw_navigation_type_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let Some(message) = v8_string(scope, message) else {
        return;
    };
    scope.throw_exception(v8::Exception::type_error(scope, message));
}

fn navigation_canceled_after_dispatch_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: &NavigationDispatchOutcome<'s>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    super::super::navigation_events::mark_navigation_outcome_default_prevented(scope, outcome);
    if let Some(signal) = outcome.signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, href);
    navigation_rejected_value_result(scope, error)
}

fn commit_navigation_navigate_same_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_href: &str,
    effective_href: &str,
    effective_kind: LocationNavigationKind,
    effective_state: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Value> {
    if matches!(effective_kind, LocationNavigationKind::Reload) {
        if let Some(state) = effective_state {
            set_reload_current_entry_state(scope, owner, state);
        }
        return navigation_current_entry(scope, owner)
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::undefined(scope).into());
    }
    apply_navigation_navigate_same_document(
        scope,
        owner,
        effective_href,
        effective_kind,
        effective_state,
    );
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let popstate_state = navigation_current_entry(scope, owner)
            .and_then(|entry| clone_navigation_entry_state(scope, entry))
            .unwrap_or_else(|| v8::null(scope).into());
        let child_handle = (!runtime_window_is_global(scope, owner))
            .then(|| {
                super::super::navigation_window::child_browsing_context_handle_for_runtime_owner(
                    scope, owner,
                )
            })
            .flatten();
        dispatch_popstate_event(scope, host_ptr, child_handle, popstate_state);
        queue_hash_change_for_runtime_owner(scope, owner, Some(current_href), effective_href);
        if runtime_window_is_global(scope, owner) {
            if let Ok(effective_url) = url::Url::parse(effective_href) {
                unsafe { &mut *host_ptr }.set_document_url(effective_url);
            }
        } else {
            sync_local_document_front_from_window(scope, owner);
        }
    } else if !runtime_window_is_global(scope, owner) {
        sync_local_document_front_from_window(scope, owner);
    }
    navigation_current_entry(scope, owner)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn set_reload_current_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let Some(current_entry) = navigation_current_entry(scope, owner) else {
        return;
    };
    set_navigation_entry_state(scope, current_entry, state);
    let Some(history) = window_history_for_holder(scope, owner) else {
        return;
    };
    let Some(entries) = history_entries(scope, history) else {
        return;
    };
    let current_index = history_index(scope, history);
    let Some(history_entry) = entries
        .get_index(scope, current_index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    if history_entry.strict_equals(current_entry.into()) {
        return;
    }
    set_navigation_entry_state(scope, history_entry, state);
}

const PRECOMMIT_COMMIT_OWNER_SLOT: &str = "__lmPrecommitCommitOwner";
const PRECOMMIT_COMMIT_NAVIGATION_SLOT: &str = "__lmPrecommitCommitNavigation";
const PRECOMMIT_COMMIT_EVENT_SLOT: &str = "__lmPrecommitCommitEvent";
const PRECOMMIT_COMMIT_SIGNAL_SLOT: &str = "__lmPrecommitCommitSignal";
const PRECOMMIT_COMMIT_CURRENT_HREF_SLOT: &str = "__lmPrecommitCommitCurrentHref";
const PRECOMMIT_COMMIT_EFFECTIVE_HREF_SLOT: &str = "__lmPrecommitCommitEffectiveHref";
const PRECOMMIT_COMMIT_KIND_SLOT: &str = "__lmPrecommitCommitKind";
const PRECOMMIT_COMMIT_STATE_SLOT: &str = "__lmPrecommitCommitState";
const PRECOMMIT_COMMIT_COMMITTED_RESOLVE_SLOT: &str = "__lmPrecommitCommitCommittedResolve";
const PRECOMMIT_COMMIT_COMMITTED_REJECT_SLOT: &str = "__lmPrecommitCommitCommittedReject";
const PRECOMMIT_COMMIT_FINISHED_RESOLVE_SLOT: &str = "__lmPrecommitCommitFinishedResolve";
const PRECOMMIT_COMMIT_FINISHED_REJECT_SLOT: &str = "__lmPrecommitCommitFinishedReject";
const PRECOMMIT_COMMIT_PROMISE_SLOT: &str = "__lmPrecommitCommitPromise";
const PRECOMMIT_COMMIT_TRANSITION_RESOLVER_SLOT: &str = "__lmPrecommitCommitTransitionResolver";
const PRECOMMIT_COMMIT_ACTIVE_SLOT: &str = "__lmPrecommitCommitActive";
const PRECOMMIT_COMMIT_ATTEMPT_ID_SLOT: &str = "__lmPrecommitCommitAttemptId";
const NAVIGATION_PENDING_PRECOMMIT_COMMIT_SLOT: &str = "__lmNavigationPendingPrecommitCommit";

#[derive(WebApiObject)]
#[webapi(record)]
struct PrecommitCommitDataDeclaration<'scope> {
    #[webapi(slot = PRECOMMIT_COMMIT_ACTIVE_SLOT)]
    active: bool,

    #[webapi(slot = PRECOMMIT_COMMIT_ATTEMPT_ID_SLOT)]
    attempt_id: Option<v8::Local<'scope, v8::BigInt>>,

    #[webapi(slot = PRECOMMIT_COMMIT_OWNER_SLOT)]
    owner: v8::Local<'scope, v8::Object>,

    #[webapi(slot = PRECOMMIT_COMMIT_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = PRECOMMIT_COMMIT_EVENT_SLOT)]
    event: v8::Local<'scope, v8::Object>,

    #[webapi(slot = PRECOMMIT_COMMIT_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(slot = PRECOMMIT_COMMIT_CURRENT_HREF_SLOT)]
    current_href: v8::Local<'scope, v8::String>,

    #[webapi(slot = PRECOMMIT_COMMIT_EFFECTIVE_HREF_SLOT)]
    effective_href: v8::Local<'scope, v8::String>,

    #[webapi(slot = PRECOMMIT_COMMIT_KIND_SLOT)]
    kind: v8::Local<'scope, v8::String>,

    #[webapi(slot = PRECOMMIT_COMMIT_STATE_SLOT)]
    state: Option<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = PRECOMMIT_COMMIT_COMMITTED_RESOLVE_SLOT)]
    committed_resolve: v8::Local<'scope, v8::Function>,

    #[webapi(slot = PRECOMMIT_COMMIT_COMMITTED_REJECT_SLOT)]
    committed_reject: v8::Local<'scope, v8::Function>,

    #[webapi(slot = PRECOMMIT_COMMIT_FINISHED_RESOLVE_SLOT)]
    finished_resolve: v8::Local<'scope, v8::Function>,

    #[webapi(slot = PRECOMMIT_COMMIT_FINISHED_REJECT_SLOT)]
    finished_reject: v8::Local<'scope, v8::Function>,

    #[webapi(slot = PRECOMMIT_COMMIT_PROMISE_SLOT)]
    promise: Option<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = PRECOMMIT_COMMIT_TRANSITION_RESOLVER_SLOT)]
    transition_resolver: Option<v8::Local<'scope, v8::PromiseResolver>>,
}

fn navigation_pending_precommit_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, navigation, NAVIGATION_PENDING_PRECOMMIT_COMMIT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_pending_precommit_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_PENDING_PRECOMMIT_COMMIT_SLOT,
        data.into(),
    );
}

fn clear_navigation_pending_precommit_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_PENDING_PRECOMMIT_COMMIT_SLOT,
        v8::undefined(scope).into(),
    );
}

#[allow(clippy::too_many_arguments)]
pub(in crate::context_bootstrap) fn queue_pending_precommit_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    outcome: &NavigationDispatchOutcome<'s>,
    current_href: &str,
    effective_href: &str,
    effective_kind: LocationNavigationKind,
    effective_state: Option<v8::Local<'s, v8::Value>>,
    committed_resolve: v8::Local<'s, v8::Function>,
    committed_reject: v8::Local<'s, v8::Function>,
    finished_resolve: v8::Local<'s, v8::Function>,
    finished_reject: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(precommit_result) = outcome.precommit_result else {
        return false;
    };
    let Some(result_object) = v8::Local::<v8::Object>::try_from(precommit_result).ok() else {
        return false;
    };
    let Some(then) = result_object
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return false;
    };
    let attempt_id = begin_navigation_attempt(scope, "precommit-commit")
        .map(|attempt_id| v8::BigInt::new_from_u64(scope, attempt_id.raw()));
    let current_href = v8_string(scope, current_href).unwrap_or_else(|| v8::String::empty(scope));
    let effective_href =
        v8_string(scope, effective_href).unwrap_or_else(|| v8::String::empty(scope));
    let navigation_type = match effective_kind {
        LocationNavigationKind::Assign => "push",
        LocationNavigationKind::Replace => "replace",
        LocationNavigationKind::Reload => "reload",
    };
    let kind = v8str(scope, navigation_type);
    let promise = v8::Local::<v8::Promise>::try_from(precommit_result)
        .is_ok()
        .then_some(precommit_result);
    let transition_resolver =
        precommit_transition_resolver_from_event(scope, event).or_else(|| {
            navigation_current_entry(scope, owner).and_then(|from| {
                install_navigation_transition(
                    scope,
                    navigation,
                    from,
                    outcome.destination,
                    navigation_type,
                )
            })
        });
    let has_transition_resolver = transition_resolver.is_some();
    let data = PrecommitCommitDataDeclaration {
        active: true,
        attempt_id,
        owner,
        navigation,
        event,
        signal: outcome.signal,
        current_href,
        effective_href,
        kind,
        state: effective_state,
        committed_resolve,
        committed_reject,
        finished_resolve,
        finished_reject,
        promise,
        transition_resolver,
    }
    .bind(scope)
    .expect("precommit commit data should bind");
    set_navigation_pending_precommit_commit(scope, navigation, data);
    let Some(on_fulfilled) = v8::Function::builder(precommit_commit_fulfilled_callback)
        .data(data.into())
        .build(scope)
    else {
        cancel_precommit_commit_attempt(scope, data);
        set_pending_precommit_commit_active(scope, navigation, data, false);
        if has_transition_resolver {
            clear_navigation_transition(scope, navigation);
        }
        return false;
    };
    let Some(on_rejected) = v8::Function::builder(precommit_commit_rejected_callback)
        .data(data.into())
        .build(scope)
    else {
        cancel_precommit_commit_attempt(scope, data);
        set_pending_precommit_commit_active(scope, navigation, data, false);
        if has_transition_resolver {
            clear_navigation_transition(scope, navigation);
        }
        return false;
    };
    let queued = then
        .call(
            scope,
            precommit_result,
            &[on_fulfilled.into(), on_rejected.into()],
        )
        .is_some();
    if !queued {
        cancel_precommit_commit_attempt(scope, data);
        set_pending_precommit_commit_active(scope, navigation, data, false);
        if has_transition_resolver {
            clear_navigation_transition(scope, navigation);
        }
    }
    queued
}

pub(in crate::context_bootstrap) fn cancel_pending_precommit_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    let Some(data) = navigation_pending_precommit_commit(scope, navigation) else {
        return;
    };
    if !precommit_commit_active(scope, data) {
        return;
    }
    cancel_precommit_commit_attempt(scope, data);
    set_pending_precommit_commit_active(scope, navigation, data, false);
    let Some(data) = pending_precommit_commit_data(scope, data.into()) else {
        return;
    };
    let error =
        navigation_dom_exception(scope, "Navigation was canceled before commit", "AbortError");
    if let Some(signal) = data.signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, data.navigation, error, &data.current_href);
    let receiver = v8::undefined(scope).into();
    let _ = data.committed_reject.call(scope, receiver, &[error]);
    reject_navigation_transition_committed(scope, data.navigation, error);
    let _ = data.finished_reject.call(scope, receiver, &[error]);
    settle_navigation_transition_finished_local(
        scope,
        data.navigation,
        data.transition_resolver,
        Some(error),
    );
}

pub(in crate::context_bootstrap) fn cancel_pending_precommit_same_document_navigation_for_window_stop<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(data) = navigation_pending_precommit_commit(scope, navigation) else {
        return false;
    };
    if !precommit_commit_active(scope, data) {
        return false;
    }
    let Some(data) = pending_precommit_commit_data(scope, data.into()) else {
        return false;
    };
    let current = url::Url::parse(&data.current_href).ok();
    let target = url::Url::parse(&data.effective_href).ok();
    if target
        .as_ref()
        .is_some_and(|target| is_same_document_fragment_navigation(current.as_ref(), target))
    {
        return false;
    }
    cancel_pending_precommit_same_document_navigation(scope, navigation);
    true
}

fn pending_precommit_commit_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(data)
        .ok()
        .is_some_and(|data| {
            precommit_commit_active(scope, data)
                && navigation_attempt_id_from_slot(scope, data, PRECOMMIT_COMMIT_ATTEMPT_ID_SLOT)
                    .is_some_and(|attempt_id| navigation_attempt_is_active(scope, attempt_id))
        })
}

fn precommit_commit_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, data, PRECOMMIT_COMMIT_ACTIVE_SLOT)
        .is_some_and(|value| value.is_boolean() && value.boolean_value(scope))
}

fn complete_precommit_commit_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    if let Some(attempt_id) =
        navigation_attempt_id_from_slot(scope, data, PRECOMMIT_COMMIT_ATTEMPT_ID_SLOT)
    {
        complete_navigation_attempt(scope, attempt_id);
    }
}

fn cancel_precommit_commit_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    if let Some(attempt_id) =
        navigation_attempt_id_from_slot(scope, data, PRECOMMIT_COMMIT_ATTEMPT_ID_SLOT)
    {
        cancel_navigation_attempt(scope, attempt_id);
    }
}

fn precommit_transition_resolver_from_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    get_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

fn set_pending_precommit_commit_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_private_value(
        scope,
        data,
        PRECOMMIT_COMMIT_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
    if !active {
        clear_navigation_pending_precommit_commit(scope, navigation);
    }
}

struct PendingPrecommitCommitData<'s> {
    owner: v8::Local<'s, v8::Object>,
    navigation: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    current_href: String,
    effective_href: String,
    effective_kind: LocationNavigationKind,
    effective_state: Option<v8::Local<'s, v8::Value>>,
    committed_resolve: v8::Local<'s, v8::Function>,
    committed_reject: v8::Local<'s, v8::Function>,
    finished_resolve: v8::Local<'s, v8::Function>,
    finished_reject: v8::Local<'s, v8::Function>,
    promise: Option<v8::Local<'s, v8::Promise>>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
}

fn pending_precommit_commit_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<PendingPrecommitCommitData<'s>> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let owner = get_private_value(scope, data, PRECOMMIT_COMMIT_OWNER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let navigation = get_private_value(scope, data, PRECOMMIT_COMMIT_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let event = get_private_value(scope, data, PRECOMMIT_COMMIT_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = get_private_value(scope, data, PRECOMMIT_COMMIT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let current_href = get_private_value(scope, data, PRECOMMIT_COMMIT_CURRENT_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))?;
    let effective_href = get_private_value(scope, data, PRECOMMIT_COMMIT_EFFECTIVE_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))?;
    let effective_kind = get_private_value(scope, data, PRECOMMIT_COMMIT_KIND_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .map(|value| match value.as_str() {
            "replace" => LocationNavigationKind::Replace,
            "reload" => LocationNavigationKind::Reload,
            _ => LocationNavigationKind::Assign,
        })?;
    let effective_state = get_private_value(scope, data, PRECOMMIT_COMMIT_STATE_SLOT)
        .filter(|value| !value.is_undefined());
    let committed_resolve = get_private_value(scope, data, PRECOMMIT_COMMIT_COMMITTED_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let committed_reject = get_private_value(scope, data, PRECOMMIT_COMMIT_COMMITTED_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let finished_resolve = get_private_value(scope, data, PRECOMMIT_COMMIT_FINISHED_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let finished_reject = get_private_value(scope, data, PRECOMMIT_COMMIT_FINISHED_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let promise = get_private_value(scope, data, PRECOMMIT_COMMIT_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    let transition_resolver =
        get_private_value(scope, data, PRECOMMIT_COMMIT_TRANSITION_RESOLVER_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) });
    Some(PendingPrecommitCommitData {
        owner,
        navigation,
        event,
        signal,
        current_href,
        effective_href,
        effective_kind,
        effective_state,
        committed_resolve,
        committed_reject,
        finished_resolve,
        finished_reject,
        promise,
        transition_resolver,
    })
}

fn precommit_commit_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !pending_precommit_commit_is_active(scope, args.data()) {
        return;
    }
    let Some(data) = pending_precommit_commit_data(scope, args.data()) else {
        return;
    };
    if let Ok(raw_data) = v8::Local::<v8::Object>::try_from(args.data()) {
        complete_precommit_commit_attempt(scope, raw_data);
        set_pending_precommit_commit_active(scope, data.navigation, raw_data, false);
    }
    let resolved_value = commit_navigation_navigate_same_document(
        scope,
        data.owner,
        &data.current_href,
        &data.effective_href,
        data.effective_kind,
        data.effective_state,
    );
    if matches!(data.effective_kind, LocationNavigationKind::Reload) {
        let current_entry = navigation_current_entry(scope, data.owner);
        dispatch_navigation_currententrychange(
            scope,
            data.navigation,
            current_entry,
            Some("reload"),
        );
    }
    let receiver = v8::undefined(scope).into();
    let _ = data
        .committed_resolve
        .call(scope, receiver, &[resolved_value]);
    resolve_navigation_transition_committed(scope, data.navigation, resolved_value);
    let (intercept_error, intercept_result) =
        run_navigation_precommit_deferred_handlers(scope, data.event);
    let outcome = NavigationDispatchOutcome {
        proceed: true,
        intercepted: true,
        signal: data.signal,
        destination: data
            .event
            .get(scope, v8str(scope, "destination").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()),
        redirected_url: None,
        redirected_history: None,
        redirected_state: None,
        precommit_event: None,
        precommit_error: None,
        precommit_result: None,
        intercept_error,
        intercept_result,
        abort_error: None,
    };
    settle_intercepted_same_document_navigation(
        scope,
        data.navigation,
        outcome,
        None,
        data.finished_resolve,
        data.finished_reject,
        data.transition_resolver,
        resolved_value,
        &data.current_href,
    );
}

fn precommit_commit_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !pending_precommit_commit_is_active(scope, args.data()) {
        return;
    }
    let Some(data) = pending_precommit_commit_data(scope, args.data()) else {
        return;
    };
    if let Ok(raw_data) = v8::Local::<v8::Object>::try_from(args.data()) {
        complete_precommit_commit_attempt(scope, raw_data);
        set_pending_precommit_commit_active(scope, data.navigation, raw_data, false);
    }
    let error = data
        .promise
        .filter(|promise| promise.state() == v8::PromiseState::Rejected)
        .map(|promise| promise.result(scope))
        .unwrap_or_else(|| args.get(0));
    if let Some(signal) = data.signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, data.navigation, error, &data.current_href);
    let receiver = v8::undefined(scope).into();
    let _ = data.committed_reject.call(scope, receiver, &[error]);
    reject_navigation_transition_committed(scope, data.navigation, error);
    let _ = data.finished_reject.call(scope, receiver, &[error]);
    settle_navigation_transition_finished_local(
        scope,
        data.navigation,
        data.transition_resolver,
        Some(error),
    );
}

const INTERCEPT_SETTLEMENT_NAVIGATION_SLOT: &str = "__lmInterceptSettlementNavigation";
const INTERCEPT_SETTLEMENT_SIGNAL_SLOT: &str = "__lmInterceptSettlementSignal";
const INTERCEPT_SETTLEMENT_COMMITTED_RESOLVE_SLOT: &str = "__lmInterceptSettlementCommittedResolve";
const INTERCEPT_SETTLEMENT_RESOLVE_SLOT: &str = "__lmInterceptSettlementResolve";
const INTERCEPT_SETTLEMENT_REJECT_SLOT: &str = "__lmInterceptSettlementReject";
const INTERCEPT_SETTLEMENT_VALUE_SLOT: &str = "__lmInterceptSettlementValue";
const INTERCEPT_SETTLEMENT_FILENAME_SLOT: &str = "__lmInterceptSettlementFilename";
const INTERCEPT_SETTLEMENT_PROMISE_SLOT: &str = "__lmInterceptSettlementPromise";
const INTERCEPT_SETTLEMENT_TRANSITION_RESOLVER_SLOT: &str =
    "__lmInterceptSettlementTransitionResolver";
const INTERCEPT_SETTLEMENT_ACTIVE_SLOT: &str = "__lmInterceptSettlementActive";
const INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT: &str = "__lmInterceptSettlementAttemptId";
const NAVIGATION_ACTIVE_INTERCEPT_SETTLEMENT_SLOT: &str = "__lmNavigationActiveInterceptSettlement";

#[derive(WebApiObject)]
#[webapi(record)]
struct InterceptSettlementDataDeclaration<'scope> {
    #[webapi(slot = INTERCEPT_SETTLEMENT_ACTIVE_SLOT)]
    active: bool,

    #[webapi(slot = INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT)]
    attempt_id: Option<v8::Local<'scope, v8::BigInt>>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_PROMISE_SLOT)]
    promise: Option<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_COMMITTED_RESOLVE_SLOT)]
    committed_resolve: Option<v8::Local<'scope, v8::Function>>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_RESOLVE_SLOT)]
    resolve: v8::Local<'scope, v8::Function>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_REJECT_SLOT)]
    reject: v8::Local<'scope, v8::Function>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_TRANSITION_RESOLVER_SLOT)]
    transition_resolver: Option<v8::Local<'scope, v8::PromiseResolver>>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_VALUE_SLOT)]
    value: v8::Local<'scope, v8::Value>,

    #[webapi(slot = INTERCEPT_SETTLEMENT_FILENAME_SLOT)]
    filename: v8::Local<'scope, v8::String>,
}

fn navigation_active_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_INTERCEPT_SETTLEMENT_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_active_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_INTERCEPT_SETTLEMENT_SLOT,
        data.into(),
    );
}

fn clear_navigation_active_intercept_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_INTERCEPT_SETTLEMENT_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(in crate::context_bootstrap) fn settle_intercepted_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: NavigationDispatchOutcome<'s>,
    mut committed_resolve: Option<v8::Local<'s, v8::Function>>,
    finished_resolve: v8::Local<'s, v8::Function>,
    finished_reject: v8::Local<'s, v8::Function>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    resolved_value: v8::Local<'s, v8::Value>,
    filename: &str,
) {
    if (outcome.intercept_error.is_some() || outcome.intercept_result.is_some())
        && let Some(resolve) = committed_resolve.take()
    {
        let receiver = v8::undefined(scope).into();
        let _ = resolve.call(scope, receiver, &[resolved_value]);
        resolve_navigation_transition_committed(scope, navigation, resolved_value);
    }
    if let Some(error) = outcome.intercept_error {
        finish_intercepted_navigation_rejected(
            scope,
            navigation,
            outcome.signal,
            committed_resolve,
            resolved_value,
            finished_reject,
            transition_resolver,
            error,
            filename,
        );
        return;
    }
    let Some(result) = outcome.intercept_result else {
        finish_intercepted_navigation_fulfilled(
            scope,
            navigation,
            outcome.signal,
            committed_resolve,
            finished_resolve,
            finished_reject,
            transition_resolver,
            resolved_value,
        );
        return;
    };
    let Some(result_object) = v8::Local::<v8::Object>::try_from(result).ok() else {
        finish_intercepted_navigation_fulfilled(
            scope,
            navigation,
            outcome.signal,
            committed_resolve,
            finished_resolve,
            finished_reject,
            transition_resolver,
            resolved_value,
        );
        return;
    };
    let Some(then) = result_object
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        finish_intercepted_navigation_fulfilled(
            scope,
            navigation,
            outcome.signal,
            committed_resolve,
            finished_resolve,
            finished_reject,
            transition_resolver,
            resolved_value,
        );
        return;
    };
    let attempt_id = begin_navigation_attempt(scope, "intercept-settlement")
        .map(|attempt_id| v8::BigInt::new_from_u64(scope, attempt_id.raw()));
    let promise = v8::Local::<v8::Promise>::try_from(result)
        .is_ok()
        .then_some(result);
    let filename = v8_string(scope, filename).unwrap_or_else(|| v8::String::empty(scope));
    let data = InterceptSettlementDataDeclaration {
        active: true,
        attempt_id,
        promise,
        navigation,
        committed_resolve,
        signal: outcome.signal,
        resolve: finished_resolve,
        reject: finished_reject,
        transition_resolver,
        value: resolved_value,
        filename,
    }
    .bind(scope)
    .expect("intercept settlement data should bind");
    set_navigation_active_intercept_settlement(scope, navigation, data);
    let Some(on_fulfilled) = v8::Function::builder(intercept_settlement_fulfilled_callback)
        .data(data.into())
        .build(scope)
    else {
        complete_intercept_settlement_attempt(scope, data);
        set_intercept_settlement_active(scope, navigation, data, false);
        finish_intercepted_navigation_fulfilled(
            scope,
            navigation,
            outcome.signal,
            committed_resolve,
            finished_resolve,
            finished_reject,
            transition_resolver,
            resolved_value,
        );
        return;
    };
    let Some(on_rejected) = v8::Function::builder(intercept_settlement_rejected_callback)
        .data(data.into())
        .build(scope)
    else {
        complete_intercept_settlement_attempt(scope, data);
        set_intercept_settlement_active(scope, navigation, data, false);
        finish_intercepted_navigation_fulfilled(
            scope,
            navigation,
            outcome.signal,
            committed_resolve,
            finished_resolve,
            finished_reject,
            transition_resolver,
            resolved_value,
        );
        return;
    };
    if then
        .call(scope, result, &[on_fulfilled.into(), on_rejected.into()])
        .is_none()
    {
        cancel_intercept_settlement_attempt(scope, data);
        set_intercept_settlement_active(scope, navigation, data, false);
    }
}

pub(in crate::context_bootstrap) fn cancel_active_intercepted_same_document_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(data) = navigation_active_intercept_settlement(scope, navigation) else {
        return false;
    };
    if !intercept_settlement_is_active(scope, data.into()) {
        return false;
    }
    cancel_intercept_settlement_attempt(scope, data);
    set_intercept_settlement_active(scope, navigation, data, false);
    let Some((
        navigation,
        signal,
        committed_resolve,
        _,
        reject,
        resolved_value,
        _,
        transition_resolver,
        filename,
    )) = intercept_settlement_data(scope, data.into())
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
    let receiver = v8::undefined(scope).into();
    if let Some(committed_resolve) = committed_resolve {
        let _ = committed_resolve.call(scope, receiver, &[resolved_value]);
        resolve_navigation_transition_committed(scope, navigation, resolved_value);
    }
    let _ = reject.call(scope, receiver, &[error]);
    if transition_resolver.is_some() {
        clear_navigation_transition(scope, navigation);
    }
    if let Some(transition_resolver) = transition_resolver {
        let _ = transition_resolver.reject(scope, error);
    }
    true
}

fn intercept_settlement_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(data)
        .ok()
        .is_some_and(|data| {
            intercept_settlement_active(scope, data)
                && navigation_attempt_id_from_slot(
                    scope,
                    data,
                    INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT,
                )
                .is_some_and(|attempt_id| navigation_attempt_is_active(scope, attempt_id))
        })
}

fn intercept_settlement_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, data, INTERCEPT_SETTLEMENT_ACTIVE_SLOT)
        .is_some_and(|value| value.is_boolean() && value.boolean_value(scope))
}

fn complete_intercept_settlement_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    if let Some(attempt_id) =
        navigation_attempt_id_from_slot(scope, data, INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT)
    {
        complete_navigation_attempt(scope, attempt_id);
    }
}

fn cancel_intercept_settlement_attempt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) {
    if let Some(attempt_id) =
        navigation_attempt_id_from_slot(scope, data, INTERCEPT_SETTLEMENT_ATTEMPT_ID_SLOT)
    {
        cancel_navigation_attempt(scope, attempt_id);
    }
}

fn set_intercept_settlement_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_private_value(
        scope,
        data,
        INTERCEPT_SETTLEMENT_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
    if !active {
        clear_navigation_active_intercept_settlement(scope, navigation);
    }
}

fn intercept_settlement_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    Option<v8::Local<'s, v8::Object>>,
    Option<v8::Local<'s, v8::Function>>,
    v8::Local<'s, v8::Function>,
    v8::Local<'s, v8::Function>,
    v8::Local<'s, v8::Value>,
    Option<v8::Local<'s, v8::Promise>>,
    Option<v8::Local<'s, v8::PromiseResolver>>,
    String,
)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let navigation = get_private_value(scope, data, INTERCEPT_SETTLEMENT_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = get_private_value(scope, data, INTERCEPT_SETTLEMENT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let committed_resolve =
        get_private_value(scope, data, INTERCEPT_SETTLEMENT_COMMITTED_RESOLVE_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    let resolve = get_private_value(scope, data, INTERCEPT_SETTLEMENT_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    let resolve = resolve?;
    let reject = get_private_value(scope, data, INTERCEPT_SETTLEMENT_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let resolved_value = get_private_value(scope, data, INTERCEPT_SETTLEMENT_VALUE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let promise = get_private_value(scope, data, INTERCEPT_SETTLEMENT_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    let transition_resolver =
        get_private_value(scope, data, INTERCEPT_SETTLEMENT_TRANSITION_RESOLVER_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) });
    let filename = get_private_value(scope, data, INTERCEPT_SETTLEMENT_FILENAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    Some((
        navigation,
        signal,
        committed_resolve,
        resolve,
        reject,
        resolved_value,
        promise,
        transition_resolver,
        filename,
    ))
}

fn intercept_settlement_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !intercept_settlement_is_active(scope, args.data()) {
        return;
    }
    let Some((
        navigation,
        signal,
        committed_resolve,
        resolve,
        reject,
        resolved_value,
        _,
        transition_resolver,
        _,
    )) = intercept_settlement_data(scope, args.data())
    else {
        return;
    };
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        complete_intercept_settlement_attempt(scope, data);
        set_intercept_settlement_active(scope, navigation, data, false);
    }
    finish_intercepted_navigation_fulfilled(
        scope,
        navigation,
        signal,
        committed_resolve,
        resolve,
        reject,
        transition_resolver,
        resolved_value,
    );
}

fn intercept_settlement_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !intercept_settlement_is_active(scope, args.data()) {
        return;
    }
    let Some((
        navigation,
        signal,
        committed_resolve,
        _,
        reject,
        resolved_value,
        promise,
        transition_resolver,
        filename,
    )) = intercept_settlement_data(scope, args.data())
    else {
        return;
    };
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        complete_intercept_settlement_attempt(scope, data);
        set_intercept_settlement_active(scope, navigation, data, false);
    }
    let error = promise
        .filter(|promise| promise.state() == v8::PromiseState::Rejected)
        .map(|promise| promise.result(scope))
        .unwrap_or_else(|| args.get(0));
    finish_intercepted_navigation_rejected(
        scope,
        navigation,
        signal,
        committed_resolve,
        resolved_value,
        reject,
        transition_resolver,
        error,
        &filename,
    );
}

fn finish_intercepted_navigation_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    committed_resolve: Option<v8::Local<'s, v8::Function>>,
    finished_resolve: v8::Local<'s, v8::Function>,
    finished_reject: v8::Local<'s, v8::Function>,
    transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    resolved_value: v8::Local<'s, v8::Value>,
) {
    queue_same_document_navigation_finished(
        scope,
        navigation,
        signal,
        committed_resolve,
        Some(finished_resolve),
        Some(finished_reject),
        Some(resolved_value),
        transition_resolver,
        "",
    );
}

fn finish_intercepted_navigation_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    committed_resolve: Option<v8::Local<'s, v8::Function>>,
    resolved_value: v8::Local<'s, v8::Value>,
    finished_reject: v8::Local<'s, v8::Function>,
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
    if let Some(committed_resolve) = committed_resolve {
        let receiver = v8::undefined(scope).into();
        let _ = committed_resolve.call(scope, receiver, &[resolved_value]);
        resolve_navigation_transition_committed(scope, navigation, resolved_value);
    }
    settle_navigation_finished_rejected_after_reactions(scope, finished_reject, error);
    settle_navigation_transition_finished_local(
        scope,
        navigation,
        transition_resolver,
        Some(error),
    );
}

pub(in crate::context_bootstrap) fn navigation_update_current_entry_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() < 1 {
        throw_navigation_type_error(
            scope,
            "Failed to execute 'updateCurrentEntry' on 'Navigation': 1 argument required, but only 0 present.",
        );
        return;
    }
    let owner = runtime_window_owner(scope, args.this());
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return;
    };
    if !navigation_document_can_update_current_entry(scope, owner) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Navigation current entry cannot be updated for a non-fully-active document.",
        );
        return;
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Navigation current entry cannot be updated for an opaque origin document.",
        );
        return;
    }
    let Some(current_entry) = navigation_current_entry(scope, owner) else {
        return;
    };
    let previous_entry = current_entry;
    let Some(options) = args.get(0).to_object(scope) else {
        return;
    };
    let Some(requested_state) = options.get(scope, v8str(scope, "state").into()) else {
        return;
    };
    if requested_state.is_undefined() {
        throw_navigation_type_error(
            scope,
            "Failed to execute 'updateCurrentEntry' on 'Navigation': Failed to read the 'state' property from 'NavigationUpdateCurrentEntryOptions': Required member is undefined.",
        );
        return;
    }
    let Some(cloned_state) = structured_clone_value(scope, requested_state) else {
        return;
    };
    set_navigation_entry_state(scope, current_entry, cloned_state);

    if let Some(history) = window_history_for_holder(scope, owner)
        && let Some(entries) = history_entries(scope, history)
    {
        let current_index = history_index(scope, history);
        if let Some(history_entry) = entries
            .get_index(scope, current_index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && !history_entry.strict_equals(current_entry.into())
        {
            set_navigation_entry_state(scope, history_entry, cloned_state);
        }
    }

    dispatch_navigation_currententrychange(scope, navigation, Some(previous_entry), None);
    sync_child_navigation_entry_seed_from_owner(scope, owner);
}
