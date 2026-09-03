use super::location_runtime::{location_href_slot, sync_location_object};
use super::navigation_activation::bind_navigation_entry_runtime_owner;
use super::navigation_callbacks::cancel_active_intercepted_same_document_navigation;
use super::navigation_entry::{
    copy_navigation_entry_document_id, create_navigation_entry, history_entries, history_index,
    navigation_current_entry, navigation_current_entry_index, navigation_entry_key_value,
    new_navigation_entry_id, new_navigation_entry_key, set_history_entries, set_history_index,
    set_history_state, stringify_history_state, sync_navigation_current_entry_from_history_entry,
};
use super::navigation_entry_state::{clone_history_entry_state, set_history_entry_state};
use super::navigation_events::{
    cancel_active_navigation_event, dispatch_navigation_currententrychange,
    dispatch_navigation_entry_dispose, dispatch_navigation_navigate_event_with_outcome,
    dispatch_navigation_success, refresh_navigation_destination_indexes,
    run_navigation_precommit_deferred_handlers,
};
use super::navigation_lifecycle::finish_navigation_error_events;
use super::navigation_projection::set_history_length_at_least_visible_entries;
use super::navigation_result::{
    cancel_pending_same_document_navigation_finishes,
    cancel_pending_same_document_navigation_finishes_including_reentrant,
    queue_same_document_navigation_success,
};
use super::navigation_serialize::sync_child_navigation_entry_seed_from_owner;
use super::navigation_window::{
    history_owner_if_fully_active, runtime_window_is_global, window_location_for_holder,
    window_navigation_for_holder,
};
use super::*;
use crate::webidl;
use moli_page_types::SameDocumentHistoryUpdate;

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
    let Some(owner) = history_owner_if_fully_active(scope, history) else {
        return;
    };
    let Some(state) = structured_clone_value_for_storage(scope, parsed.state) else {
        return;
    };
    let state_json = stringify_history_state(scope, state);
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
        Some(target) if !target.is_empty() => resolve_history_state_url(resolve_base_href, &target),
        _ => Some(current_url.clone()),
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

    let entries = history_entries(scope, history).unwrap_or_else(|| v8::Array::new(scope, 0));
    let current_index = history_index(scope, history);
    let current_navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
    let previous_entry = navigation_current_entry(scope, owner);
    let entry = match kind {
        HistoryMutationKind::Push => {
            let next_entries = v8::Array::new(scope, (current_index + 2) as i32);
            for index in 0..=current_index {
                if let Some(entry) = entries.get_index(scope, index) {
                    let _ = next_entries.set_index(scope, index, entry);
                }
            }
            let next_index = current_index + 1;
            let next_navigation_index = current_navigation_index + 1;
            let entry = create_navigation_entry(
                scope,
                url.as_str(),
                state_json.as_deref(),
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
            let _ = next_entries.set_index(scope, next_index, entry.into());
            set_history_entries(scope, history, next_entries);
            set_history_index(scope, history, next_index);
            set_history_length_at_least_visible_entries(scope, history, next_entries);
            entry
        }
        HistoryMutationKind::Replace => {
            let key = previous_entry
                .and_then(|entry| navigation_entry_key_value(scope, entry))
                .unwrap_or_else(|| new_navigation_entry_key().as_str().to_owned());
            let entry = create_navigation_entry(
                scope,
                url.as_str(),
                state_json.as_deref(),
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
            let _ = entries.set_index(scope, current_index, entry.into());
            set_history_entries(scope, history, entries);
            entry
        }
    };
    // `history.state` is a structured-clone value, not a JSON value. Keep the
    // live entry's cloned snapshot authoritative even when the optional
    // cross-runtime JSON projection cannot represent values such as Map,
    // ArrayBuffer, or BigInt.
    set_history_entry_state(scope, entry, state);
    let current_state =
        clone_history_entry_state(scope, entry).unwrap_or_else(|| v8::null(scope).into());
    set_history_state(scope, history, current_state);
    sync_location_object(scope, location, url.as_str());
    sync_navigation_current_entry_from_history_entry(scope, owner, entry);
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
            queue_same_document_navigation_success(
                scope,
                navigation,
                navigate_outcome.as_ref().and_then(|outcome| outcome.signal),
                url.as_str(),
            );
        }
    }
    sync_child_navigation_entry_seed_from_owner(scope, owner);
    if runtime_window_is_global(scope, owner) {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return;
        };
        let host = unsafe { &mut *host_ptr };
        host.set_document_url(url.clone());
        let history_update = match kind {
            HistoryMutationKind::Push => SameDocumentHistoryUpdate::Push,
            HistoryMutationKind::Replace => SameDocumentHistoryUpdate::Replace,
        };
        host.record_same_document_navigation(&url, "historyApi", history_update);
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
