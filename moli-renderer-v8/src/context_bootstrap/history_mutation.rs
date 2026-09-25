use super::history_runtime::require_fully_active_history_owner;
use super::history_runtime::state::{push_history_entry, replace_history_entry};
use super::location_runtime::{location_href_slot, sync_location_object};
use super::navigation_activation::bind_navigation_entry_runtime_owner;
use super::navigation_callbacks::cancel_active_intercepted_same_document_navigation;
use super::navigation_entry::{
    cache_current_history_state, copy_navigation_entry_document_id, create_navigation_entry,
    navigation_current_entry, navigation_current_entry_index, navigation_entry_key_value,
    new_navigation_entry_id, new_navigation_entry_key,
    sync_navigation_current_entry_from_history_entry,
};
use super::navigation_entry_state::clone_history_entry_state;
use super::navigation_events::{
    cancel_active_navigation_event, dispatch_navigation_currententrychange,
    dispatch_navigation_entry_dispose, dispatch_navigation_navigate_event_with_outcome,
    dispatch_navigation_success, refresh_navigation_destination_indexes,
    run_navigation_precommit_deferred_handlers,
};
use super::navigation_lifecycle::finish_navigation_error_events;
use super::navigation_result::{
    cancel_pending_same_document_navigation_finishes,
    cancel_pending_same_document_navigation_finishes_including_reentrant,
    queue_same_document_navigation_success,
};
use super::navigation_serialize::sync_child_navigation_entry_seed_from_owner;
use super::navigation_window::{
    runtime_window_is_global, window_location_for_holder, window_navigation_for_holder,
};
use super::*;
use crate::structured_clone::{deserialize_history_state, serialize_history_state};
use crate::webidl;

struct ParsedHistoryMutationArgs<'s> {
    state: v8::Local<'s, v8::Value>,
    unused: String,
    url: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "History.pushState")]
struct HistoryPushStateArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'pushState' on 'History': 2 arguments required."
    )]
    state: v8::Local<'s, v8::Value>,
    #[webidl(
        required,
        missing_message = "Failed to execute 'pushState' on 'History': 2 arguments required."
    )]
    unused: String,
    #[webidl(index = 2, converter = "usv_string", nullable)]
    url: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "History.replaceState")]
struct HistoryReplaceStateArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'replaceState' on 'History': 2 arguments required."
    )]
    state: v8::Local<'s, v8::Value>,
    #[webidl(
        required,
        missing_message = "Failed to execute 'replaceState' on 'History': 2 arguments required."
    )]
    unused: String,
    #[webidl(index = 2, converter = "usv_string", nullable)]
    url: Option<String>,
}

#[derive(Clone, Copy)]
enum HistoryMutationKind {
    Push,
    Replace,
}

pub(super) fn history_push_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    mutate_history_object(scope, args.this(), &args, HistoryMutationKind::Push);
}

pub(super) fn history_replace_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    mutate_history_object(scope, args.this(), &args, HistoryMutationKind::Replace);
}

fn mutate_history_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: HistoryMutationKind,
) {
    let Some(parsed) = parse_history_mutation_args(scope, args, kind) else {
        return;
    };
    let _ = &parsed.unused;
    let Some(owner) = require_fully_active_history_owner(scope, history) else {
        return;
    };
    let Some(snapshot) = serialize_history_state(scope, parsed.state) else {
        return;
    };
    // Argument conversion/serialization belongs to the caller. Entry objects
    // and navigation events belong to the Window, regardless of the caller's world.
    let Some(context) = owner.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(state) = deserialize_history_state(scope, &snapshot) else {
        return;
    };
    let Some(location) = window_location_for_holder(scope, owner) else {
        return;
    };
    let current_href =
        location_href_slot(scope, location).unwrap_or_else(|| "about:blank".to_owned());
    let current_url = match history_same_origin_reference_url(scope, owner, &current_href) {
        Ok(url) => url,
        Err(_) => {
            throw_history_security_error(
                scope,
                "Failed to execute 'pushState' or 'replaceState' on 'History': The current URL is invalid.",
            );
            return;
        }
    };
    let resolve_base_href = if history_url_inherits_origin(scope, owner, &current_href) {
        current_url.as_str()
    } else {
        &current_href
    };
    let url = match parsed.url {
        Some(target) => resolve_history_state_url(resolve_base_href, &target),
        None => Some(current_url.clone()),
    };
    let Some(url) = url else {
        throw_history_security_error(
            scope,
            "Failed to execute 'pushState' or 'replaceState' on 'History': The provided URL is invalid.",
        );
        return;
    };
    if !moli_url::same_origin(&url, &current_url) {
        throw_history_security_error(
            scope,
            "Failed to execute 'pushState' or 'replaceState' on 'History': A history state object with URL of a different origin cannot be created in a document with origin.",
        );
        return;
    }

    let mut navigate_outcome = None;
    if let Some(navigation) = window_navigation_for_holder(scope, owner) {
        let _ = cancel_active_navigation_event(scope, navigation);
        cancel_active_intercepted_same_document_navigation(scope, navigation);
        cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
        let navigation_type = match kind {
            HistoryMutationKind::Push => "push",
            HistoryMutationKind::Replace => "replace",
        };
        let outcome = dispatch_navigation_navigate_event_with_outcome(
            scope,
            navigation,
            url.as_str(),
            navigation_type,
            false,
            true,
            true,
            false,
            None,
            Some(state),
            None,
            None,
        );
        if !outcome.proceed {
            return;
        }
        cancel_pending_same_document_navigation_finishes(scope, navigation);
        navigate_outcome = Some(outcome);
    }

    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let previous_entry = navigation_current_entry(scope, owner);
    let mut pruned = Vec::new();
    let entry = match kind {
        HistoryMutationKind::Push => {
            let next_navigation_index = current_navigation_index + 1;
            let entry = create_navigation_entry(
                scope,
                url.as_str(),
                None,
                None,
                None,
                next_navigation_index,
                &new_navigation_entry_id(),
                &new_navigation_entry_key(),
            );
            if let Some(previous_entry) = previous_entry {
                copy_navigation_entry_document_id(scope, previous_entry, entry);
            }
            bind_navigation_entry_runtime_owner(scope, entry, owner);
            pruned = push_history_entry(scope, history, entry);
            entry
        }
        HistoryMutationKind::Replace => {
            let key = previous_entry
                .and_then(|entry| navigation_entry_key_value(scope, entry))
                .unwrap_or_else(|| new_navigation_entry_key().as_str().to_owned());
            let entry = create_navigation_entry(
                scope,
                url.as_str(),
                None,
                None,
                None,
                current_navigation_index,
                &new_navigation_entry_id(),
                &key,
            );
            if let Some(previous_entry) = previous_entry {
                copy_navigation_entry_document_id(scope, previous_entry, entry);
            }
            bind_navigation_entry_runtime_owner(scope, entry, owner);
            replace_history_entry(scope, history, entry);
            entry
        }
    };
    // Keep the pre-event serialized snapshot authoritative even if a callback
    // has modified an exposed JS value during the navigation.
    if let Some(record) = super::history_runtime::native::entry(scope, entry) {
        record.borrow_mut().history_state = Some(snapshot);
    }
    let current_state =
        clone_history_entry_state(scope, entry).unwrap_or_else(|| v8::null(scope).into());
    cache_current_history_state(scope, history, current_state);
    sync_location_object(scope, location, url.as_str());
    sync_navigation_current_entry_from_history_entry(scope, owner, entry);
    super::session_history::commit(
        scope,
        owner,
        entry,
        match kind {
            HistoryMutationKind::Push => moli_page_types::SessionHistoryCommit::Push,
            HistoryMutationKind::Replace => moli_page_types::SessionHistoryCommit::Replace,
        },
    );
    pruned.extend(super::session_history::prune_views(scope, owner));
    if let Some(navigation) = window_navigation_for_holder(scope, owner) {
        refresh_navigation_destination_indexes(scope, navigation, history);
        dispatch_navigation_currententrychange(
            scope,
            navigation,
            previous_entry,
            Some(match kind {
                HistoryMutationKind::Push => "push",
                HistoryMutationKind::Replace => "replace",
            }),
        );
        if matches!(kind, HistoryMutationKind::Replace)
            && let Some(previous_entry) = previous_entry
        {
            dispatch_navigation_entry_dispose(scope, previous_entry);
        }
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
            if let Some(error) = outcome.intercept_error {
                finish_navigation_error_events(scope, navigation, error, url.as_str());
            } else {
                dispatch_navigation_success(scope, navigation);
            }
        } else {
            // History API updates preserve scroll, even when the new URL has a
            // fragment. Only queue completion, without a fragment/top fallback.
            queue_same_document_navigation_success(
                scope,
                navigation,
                navigate_outcome.as_ref().and_then(|outcome| outcome.signal),
                None,
            );
        }
    }
    for entry in pruned {
        dispatch_navigation_entry_dispose(scope, entry);
    }
    sync_child_navigation_entry_seed_from_owner(scope, owner);
    if runtime_window_is_global(scope, owner) {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return;
        };
        let host = unsafe { &mut *host_ptr };
        host.set_document_url(url.clone());
        host.record_same_document_navigation(&url, "historyApi");
    } else if let Some(popup_id) =
        crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(document_handle) =
            unsafe { &*host_ptr }.lightweight_popup_document_handle(popup_id)
    {
        let _ = unsafe { &mut *host_ptr }.set_dom_document_url_for_handle(document_handle, url);
    }
}

fn resolve_history_state_url(base_href: &str, target: &str) -> Option<url::Url> {
    if let Ok(absolute) = url::Url::parse(target) {
        return Some(absolute);
    }
    url::Url::parse(base_href).ok()?.join(target).ok()
}

fn parse_history_mutation_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: HistoryMutationKind,
) -> Option<ParsedHistoryMutationArgs<'s>> {
    match kind {
        HistoryMutationKind::Push => {
            let parsed = webidl::parse_args::<HistoryPushStateArgs>(scope, args)?;
            Some(ParsedHistoryMutationArgs {
                state: parsed.state,
                unused: parsed.unused,
                url: parsed.url,
            })
        }
        HistoryMutationKind::Replace => {
            let parsed = webidl::parse_args::<HistoryReplaceStateArgs>(scope, args)?;
            Some(ParsedHistoryMutationArgs {
                state: parsed.state,
                unused: parsed.unused,
                url: parsed.url,
            })
        }
    }
}

fn history_same_origin_reference_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_href: &str,
) -> Result<url::Url, url::ParseError> {
    if history_url_inherits_origin(scope, owner, current_href)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        return Ok(unsafe { &mut *host_ptr }.host_document().url().clone());
    }
    url::Url::parse(current_href)
}

fn history_url_inherits_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    current_href: &str,
) -> bool {
    !runtime_window_is_global(scope, owner)
        && matches!(current_href, "about:blank" | "about:srcdoc")
}

fn throw_history_security_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    crate::context_bootstrap::throw_dom_exception_value(scope, message, "SecurityError");
}
