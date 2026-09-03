use super::history_runtime::{
    cancel_pending_precommit_history_traversal, pending_history_traversal_target_index,
};
use super::location_navigation::{
    LocationNavigationKind, navigate_location_object,
    navigate_location_object_with_child_navigate_event,
};
use super::navigation_activation::install_navigation_transition;
use super::navigation_callbacks::{
    cancel_active_intercepted_same_document_navigation,
    cancel_pending_precommit_same_document_navigation,
    queue_pending_precommit_same_document_navigation, settle_intercepted_same_document_navigation,
};
use super::navigation_entry::{
    history_entries, history_index, navigation_current_entry, navigation_entries_share_document,
};
use super::navigation_entry_state::{
    clone_navigation_entry_state, clone_navigation_state_arg_for_result,
};
use super::navigation_events::cancel_active_navigation_event;
use super::navigation_events::{
    dispatch_navigation_currententrychange, dispatch_navigation_navigate_event_with_outcome,
    run_navigation_precommit_deferred_handlers,
};
use super::navigation_lifecycle::finish_navigation_error_events;
use super::navigation_projection::visible_navigation_entries_len;
use super::navigation_reload::{NavigationReloadAdmission, navigation_reload_admission};
use super::navigation_result::{
    navigation_current_entry_result_with_pending_finished, navigation_dom_exception,
    navigation_immediate_current_entry_result, navigation_pending_result,
    navigation_rejected_dom_exception_result, navigation_rejected_invalid_state_result,
    navigation_rejected_value_result, navigation_result_with_pending_commit,
};
use super::navigation_traversal_execution::{
    queue_history_traversal_without_result, queue_navigation_traversal_with_result,
};
use super::navigation_traversal_plan::{
    NavigationTraversalPlan, history_delta_traversal_target, navigation_delta_traversal_plan,
    navigation_index_traversal_plan,
};
use super::navigation_window::{
    history_owner_if_fully_active, navigation_document_can_update_current_entry,
    navigation_document_is_active, navigation_unload_event_active, runtime_window_is_global,
    runtime_window_owner, window_history_for_holder, window_location_for_holder,
    window_navigation_for_holder,
};
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigation.traverseTo")]
struct NavigationTraverseToArgs {
    #[webidl(
        required,
        name = "key",
        missing_message = "Failed to execute 'traverseTo' on 'Navigation': 1 argument required, but only 0 present."
    )]
    key: String,
}

pub(super) fn history_go_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() > 0 && (args.get(0).is_symbol() || args.get(0).is_big_int()) {
        throw_history_type_error(
            scope,
            "Failed to execute 'go' on 'History': The provided value cannot be converted to a number.",
        );
        return;
    }
    let action = if args.length() == 0 {
        Some(HistoryGoAction::Reload)
    } else {
        coerce_history_go_delta(scope, args.get(0))
    };
    let Some(action) = action else {
        return;
    };
    let Some(owner) = history_owner_if_fully_active(scope, args.this()) else {
        return;
    };
    match action {
        HistoryGoAction::Reload => {
            let Some(location) = window_location_for_holder(scope, owner) else {
                return;
            };
            navigate_location_object_with_child_navigate_event(
                scope,
                location,
                LocationNavigationKind::Reload,
                None,
            );
        }
        HistoryGoAction::Traverse(delta) => history_traverse(scope, args.this(), delta),
        HistoryGoAction::Noop => {}
    }
}

pub(super) fn history_back_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if history_owner_if_fully_active(scope, args.this()).is_none() {
        return;
    }
    history_traverse(scope, args.this(), -1);
}

pub(super) fn history_forward_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if history_owner_if_fully_active(scope, args.this()).is_none() {
        return;
    }
    history_traverse(scope, args.this(), 1);
}

pub(super) fn navigation_back_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = runtime_window_owner(scope, args.this());
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
    let info = navigation_traversal_info_arg(scope, &args, 0);
    rv.set(
        navigation_traverse_result(scope, args.this(), -1, info)
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn navigation_forward_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = runtime_window_owner(scope, args.this());
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
    let info = navigation_traversal_info_arg(scope, &args, 0);
    rv.set(
        navigation_traverse_result(scope, args.this(), 1, info)
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn navigation_traverse_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NavigationTraverseToArgs>(scope, &args) else {
        return;
    };
    let owner = runtime_window_owner(scope, args.this());
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
    let target_key = parsed.key;
    let Some(history) = window_history_for_holder(scope, owner) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(entries) = history_entries(scope, history) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let target_index = (0..entries.length()).find(|index| {
        entries
            .get_index(scope, *index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|entry| {
                get_own_static_property(scope, entry, "key")
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
            })
            .is_some_and(|key| key == target_key)
    });
    let Some(target_index) = target_index else {
        rv.set(navigation_rejected_invalid_state_result(scope, "Invalid key").into());
        return;
    };
    let info = navigation_traversal_info_arg(scope, &args, 1);
    rv.set(
        navigation_traverse_to_index_result(scope, args.this(), target_index, info)
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn navigation_reload_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = runtime_window_owner(scope, args.this());
    let Some(location) = window_location_for_holder(scope, owner) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let options = args.get(0).to_object(scope);
    let navigation_info = options.and_then(|options| {
        options
            .get(scope, v8str(scope, "info").into())
            .filter(|value| !value.is_undefined())
    });
    let cloned_navigation_state = match clone_navigation_state_arg_for_result(scope, options) {
        Ok(state) => state,
        Err(error) => {
            rv.set(navigation_rejected_value_result(scope, error).into());
            return;
        }
    };
    if !navigation_document_is_active(scope, owner) {
        rv.set(
            navigation_rejected_invalid_state_result(
                scope,
                "Navigation current entry cannot be updated for a non-fully-active document.",
            )
            .into(),
        );
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
    if matches!(
        navigation_reload_admission(scope, owner),
        NavigationReloadAdmission::NoCommittedHistoryItem
    ) {
        // Navigation API methods on an initial-empty Document return a
        // correctly-shaped result whose promises intentionally never settle.
        rv.set(navigation_pending_result(scope).into());
        return;
    }
    if navigation_document_can_update_current_entry(scope, owner)
        && let Some(navigation) = window_navigation_for_holder(scope, owner)
    {
        let _ = cancel_active_navigation_event(scope, navigation);
        cancel_pending_precommit_same_document_navigation(scope, navigation);
        cancel_pending_precommit_history_traversal(scope, navigation);
        cancel_active_intercepted_same_document_navigation(scope, navigation);
        let current_href = super::location_runtime::location_href_slot(scope, location)
            .unwrap_or_else(|| "about:blank".to_owned());
        let destination_state = reload_destination_state(scope, owner, cloned_navigation_state);
        let mut outcome = dispatch_navigation_navigate_event_with_outcome(
            scope,
            navigation,
            &current_href,
            "reload",
            false,
            false,
            true,
            false,
            None,
            destination_state,
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
                reload_canceled_after_dispatch_result(scope, navigation, &outcome, &current_href)
                    .into(),
            );
            return;
        }
        if !outcome.proceed {
            rv.set(
                reload_canceled_after_dispatch_result(scope, navigation, &outcome, &current_href)
                    .into(),
            );
            return;
        }
        if outcome.intercepted {
            if let Some(precommit_event) = outcome.precommit_event
                && let Some(pending) = navigation_result_with_pending_commit(scope)
                && queue_pending_precommit_same_document_navigation(
                    scope,
                    owner,
                    precommit_event,
                    &outcome,
                    &current_href,
                    &current_href,
                    LocationNavigationKind::Reload,
                    cloned_navigation_state,
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
                install_navigation_transition(scope, navigation, from, transition_to, "reload")
            });
            let current_entry = navigation_current_entry(scope, owner);
            dispatch_navigation_currententrychange(
                scope,
                navigation,
                current_entry,
                Some("reload"),
            );
            if let Some(state) = cloned_navigation_state {
                set_reload_current_entry_state(scope, owner, state);
            }
            if let Some(precommit_event) = outcome.precommit_event {
                let (intercept_error, intercept_result) =
                    run_navigation_precommit_deferred_handlers(scope, precommit_event);
                outcome.intercept_error = intercept_error;
                outcome.intercept_result = intercept_result.or(outcome.intercept_result);
            }
            let Some(pending) = navigation_current_entry_result_with_pending_finished(scope, owner)
            else {
                rv.set(navigation_immediate_current_entry_result(scope, owner).into());
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
    }
    navigate_location_object(scope, location, LocationNavigationKind::Reload, None);
    rv.set(navigation_pending_result(scope).into());
}

fn reload_canceled_after_dispatch_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: &super::navigation_events::NavigationDispatchOutcome<'s>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    super::navigation_events::mark_navigation_outcome_default_prevented(scope, outcome);
    if let Some(signal) = outcome.signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, href);
    navigation_rejected_value_result(scope, error)
}

fn reload_destination_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    requested_state: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Value>> {
    requested_state.or_else(|| {
        navigation_current_entry(scope, owner)
            .and_then(|entry| clone_navigation_entry_state(scope, entry))
    })
}

fn set_reload_current_entry_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let Some(current_entry) = navigation_current_entry(scope, owner) else {
        return;
    };
    super::navigation_entry_state::set_navigation_entry_state(scope, current_entry, state);
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
    super::navigation_entry_state::set_navigation_entry_state(scope, history_entry, state);
}

fn navigation_traverse_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    delta: i64,
    info: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    match navigation_delta_traversal_plan(scope, navigation, delta)? {
        NavigationTraversalPlan::RejectInvalidState(message) => {
            Some(navigation_rejected_invalid_state_result(scope, message))
        }
        NavigationTraversalPlan::ResolveCurrentEntry(owner) => {
            Some(navigation_immediate_current_entry_result(scope, owner))
        }
        NavigationTraversalPlan::Traverse(target) => {
            queue_navigation_traversal_with_result(scope, target, info)
        }
    }
}

fn navigation_traverse_to_index_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    target_index: u32,
    info: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    match navigation_index_traversal_plan(scope, navigation, target_index)? {
        NavigationTraversalPlan::RejectInvalidState(message) => {
            Some(navigation_rejected_invalid_state_result(scope, message))
        }
        NavigationTraversalPlan::ResolveCurrentEntry(owner) => {
            Some(navigation_immediate_current_entry_result(scope, owner))
        }
        NavigationTraversalPlan::Traverse(target) => {
            queue_navigation_traversal_with_result(scope, target, info)
        }
    }
}

fn navigation_traversal_info_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Option<v8::Local<'s, v8::Value>> {
    args.get(index)
        .to_object(scope)
        .and_then(|options| options.get(scope, v8str(scope, "info").into()))
        .filter(|value| !value.is_undefined())
}

enum HistoryGoAction {
    Reload,
    Traverse(i64),
    Noop,
}

fn coerce_history_go_delta(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<HistoryGoAction> {
    let raw = value.number_value(scope)?;
    if raw.is_nan() {
        return Some(HistoryGoAction::Reload);
    }
    if !raw.is_finite() {
        return Some(HistoryGoAction::Noop);
    }
    let truncated = raw.trunc();
    if truncated == 0.0 {
        return Some(HistoryGoAction::Reload);
    }
    if truncated >= i64::MAX as f64 || truncated <= i64::MIN as f64 {
        return Some(HistoryGoAction::Noop);
    }
    Some(HistoryGoAction::Traverse(truncated as i64))
}

fn throw_history_type_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let Some(message) = v8_string(scope, message) else {
        return;
    };
    scope.throw_exception(v8::Exception::type_error(scope, message));
}

fn history_traverse<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    delta: i64,
) {
    let Some(target) = history_delta_traversal_target(scope, history, delta) else {
        queue_browser_owned_top_level_history_traversal(scope, history, delta);
        return;
    };
    queue_history_traversal_without_result(scope, target);
}

fn queue_browser_owned_top_level_history_traversal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    delta: i64,
) {
    let owner = runtime_window_owner(scope, history);
    if !runtime_window_is_global(scope, owner) {
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }.record_pending_top_level_history_traversal(delta);
}

pub(crate) fn queue_top_level_history_traversal_by_delta(
    scope: &mut v8::PinScope<'_, '_>,
    delta: i64,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(history) = window_history_for_holder(scope, global) else {
        return false;
    };
    let Some(target) = history_delta_traversal_target(scope, history, delta) else {
        return false;
    };
    let Some(entries) = history_entries(scope, history) else {
        return false;
    };
    let Some(current_entry) = entries
        .get_index(scope, target.current_index)
        .and_then(|entry| v8::Local::<v8::Object>::try_from(entry).ok())
    else {
        return false;
    };
    let Some(target_entry) = entries
        .get_index(scope, target.target_index)
        .and_then(|entry| v8::Local::<v8::Object>::try_from(entry).ok())
    else {
        return false;
    };
    if !navigation_entries_share_document(scope, current_entry, target_entry) {
        return false;
    }
    queue_history_traversal_without_result(scope, target);
    true
}

pub(super) fn pending_or_current_navigation_entry_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> u32 {
    let target_history_index = pending_history_traversal_target_index(scope, history)
        .unwrap_or_else(|| history_index(scope, history));
    history_entry_navigation_index(scope, history, target_history_index)
        .unwrap_or(target_history_index)
}

fn history_entry_navigation_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    history_index: u32,
) -> Option<u32> {
    history_entries(scope, history)?
        .get_index(scope, history_index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|entry| {
            get_own_static_property(scope, entry, "index")
                .and_then(|value| value.integer_value(scope))
        })
        .filter(|value| *value >= 0)
        .map(|value| value as u32)
}

pub(super) fn navigation_entries_len<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> u32 {
    let owner = runtime_window_owner(scope, history);
    let current_entry = navigation_current_entry(scope, owner);
    history_entries(scope, history)
        .map(|entries| visible_navigation_entries_len(scope, entries, current_entry))
        .unwrap_or(0)
}
