use super::history_mutation::document_can_have_url_rewritten;
use super::history_runtime::cancel_pending_precommit_history_traversal;
use super::location_navigation::{
    LocationNavigationKind, cancel_active_location_intercepted_same_document_navigation,
};
use super::location_runtime::location_href_slot;
use super::navigation_activation::{
    install_navigation_transition, navigation_transition_matches_resolver,
    precommit_transition_resolver_from_event, resolve_navigation_transition_committed,
    take_navigation_transition_committed_resolver,
};
use super::navigation_callbacks::{
    cancel_active_intercepted_same_document_navigation,
    cancel_pending_precommit_same_document_navigation, commit_navigation_navigate_same_document,
    queue_pending_precommit_same_document_navigation, settle_intercepted_same_document_navigation,
};
use super::navigation_entry::navigation_current_entry;
use super::navigation_events::{
    NavigationDispatchOutcome, cancel_active_navigation_event,
    dispatch_navigation_navigate_event_with_outcome, run_navigation_precommit_deferred_handlers,
};
use super::navigation_lifecycle::{
    finish_navigation_error_events, settle_navigation_transition_finished_local,
};
use super::navigation_result::{
    cancel_active_cross_document_navigation, cancel_pending_same_document_navigation_finishes,
    cancel_pending_same_document_navigation_finishes_including_reentrant, navigation_dom_exception,
};
use super::navigation_window::{
    navigation_document_has_disabled_entries, navigation_document_is_active,
    window_location_for_holder, window_navigation_for_holder,
};
use super::*;
use crate::script_cleanup::ScriptExecutionScope;

pub(super) fn dispatch_download_navigation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    filename: &str,
) -> bool {
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return true;
    };
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let _execution = ScriptExecutionScope::enter(scope);
    if navigation_document_has_disabled_entries(scope, owner) {
        return true;
    }
    let _ = cancel_active_navigation_event(scope, navigation);
    cancel_pending_precommit_same_document_navigation(scope, navigation);
    cancel_pending_precommit_history_traversal(scope, navigation);
    cancel_active_intercepted_same_document_navigation(scope, navigation);
    cancel_active_location_intercepted_same_document_navigation(scope, navigation);
    cancel_active_cross_document_navigation(scope, navigation, None);
    cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
    if !navigation_document_is_active(scope, owner) {
        return false;
    }
    let current_href = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let can_intercept = url::Url::parse(&current_href)
        .ok()
        .zip(url::Url::parse(href).ok())
        .is_some_and(|(current, target)| document_can_have_url_rewritten(&current, &target));
    let state = v8::null(scope).into();
    let mut outcome = dispatch_navigation_navigate_event_with_outcome(
        scope,
        navigation,
        href,
        "push",
        false,
        false,
        can_intercept,
        user_initiated,
        Some(filename),
        Some(state),
        None,
        source_element,
    );
    if outcome.abort_error.is_some() {
        return false;
    }
    if !outcome.proceed || !navigation_document_is_active(scope, owner) {
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        finish_download_navigation_error(scope, navigation, &outcome, error, &current_href);
        return false;
    }
    if let Some(error) = outcome.precommit_error {
        finish_download_navigation_error(scope, navigation, &outcome, error, &current_href);
        return false;
    }
    if !outcome.intercepted {
        return true;
    }
    cancel_pending_same_document_navigation_finishes(scope, navigation);
    let effective_href = outcome.redirected_url.as_deref().unwrap_or(href).to_owned();
    let effective_kind = if outcome.redirected_history.as_deref() == Some("replace") {
        LocationNavigationKind::Replace
    } else {
        LocationNavigationKind::Assign
    };
    // Downloads have no API method tracker that supplies state to the committed entry.
    let effective_state = None;
    if let Some(event) = outcome.precommit_event
        && queue_pending_precommit_same_document_navigation(
            scope,
            owner,
            event,
            &outcome,
            &current_href,
            &effective_href,
            effective_kind,
            effective_state,
            None,
            None,
            None,
            None,
        )
    {
        return false;
    }
    let transition_resolver = outcome
        .precommit_event
        .and_then(|event| precommit_transition_resolver_from_event(scope, event))
        .or_else(|| {
            navigation_current_entry(scope, owner).and_then(|from| {
                install_navigation_transition(
                    scope,
                    navigation,
                    from,
                    outcome.destination,
                    if effective_kind == LocationNavigationKind::Replace {
                        "replace"
                    } else {
                        "push"
                    },
                )
            })
        });
    let resolved_value = commit_navigation_navigate_same_document(
        scope,
        owner,
        &current_href,
        &effective_href,
        effective_kind,
        effective_state,
        true,
    );
    resolve_navigation_transition_committed(scope, navigation);
    if let Some(event) = outcome.precommit_event {
        let (error, result) = run_navigation_precommit_deferred_handlers(scope, event);
        outcome.intercept_error = error;
        outcome.intercept_result = result.or(outcome.intercept_result);
    }
    settle_intercepted_same_document_navigation(
        scope,
        navigation,
        outcome,
        None,
        None,
        None,
        transition_resolver,
        resolved_value,
        &current_href,
    );
    false
}

fn finish_download_navigation_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    outcome: &NavigationDispatchOutcome<'s>,
    error: v8::Local<'s, v8::Value>,
    href: &str,
) {
    let transition_resolver = outcome
        .precommit_event
        .and_then(|event| precommit_transition_resolver_from_event(scope, event));
    let committed_resolver = transition_resolver
        .filter(|resolver| navigation_transition_matches_resolver(scope, navigation, *resolver))
        .and_then(|_| take_navigation_transition_committed_resolver(scope, navigation));
    if let Some(signal) = outcome.signal
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, href);
    if let Some(resolver) = committed_resolver {
        let _ = resolver.reject(scope, error);
    }
    settle_navigation_transition_finished_local(
        scope,
        navigation,
        transition_resolver,
        Some(error),
    );
}
