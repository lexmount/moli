use super::helpers::{
    window_current_dispatch_scope, window_hidden_value, window_host_ptr, window_is_closed,
};
use super::*;

pub(super) fn window_alias_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    if matches!(slot, WINDOW_PARENT_SLOT | WINDOW_TOP_SLOT) && window_is_closed(scope, receiver) {
        return Some(v8::null(scope).into());
    }
    let context = receiver
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    Some(window_hidden_value(scope, receiver, slot).unwrap_or_else(|| context.global(scope).into()))
}

macro_rules! alias {
    ($name:ident, $slot:ident) => {
        pub(crate) fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            receiver: v8::Local<'s, v8::Object>,
        ) -> Option<v8::Local<'s, v8::Value>> {
            window_alias_value(scope, receiver, $slot)
        }
    };
}
alias!(window_self_value, WINDOW_SELF_SLOT);
alias!(window_frames_value, WINDOW_FRAMES_SLOT);
alias!(window_parent_value, WINDOW_PARENT_SLOT);
alias!(window_top_value, WINDOW_TOP_SLOT);

pub(crate) fn window_closed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let closed = window_is_closed(scope, receiver);
    Some(v8::Boolean::new(scope, closed).into())
}

pub(crate) fn window_length_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let count = if !window_is_closed(scope, receiver)
        && let Some(host) = window_host_ptr(scope, receiver)
    {
        let host = unsafe { &mut *host };
        match super::helpers::window_document_handle(scope, receiver, host) {
            Some(document) => {
                host.child_browsing_context_count_for_document(document)
            }
            None => 0,
        }
    } else {
        0
    };
    Some(v8::Number::new(scope, count as f64).into())
}

pub(crate) fn window_opener_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let null = v8::null(scope).into();
    if window_is_closed(scope, receiver) {
        return Some(null);
    }
    if let Some(host_ptr) = window_host_ptr(scope, receiver) {
        let host = unsafe { &*host_ptr };
        if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, receiver) {
            return Some(host.lightweight_popup_opener_window(scope, popup_id).map(Into::into).unwrap_or(null));
        }
        if let Some(handle) = window_child_context_handle(scope, receiver) {
            return Some(host.child_browsing_context_opener(scope, handle).map(Into::into).unwrap_or(null));
        }
        if let Some(opener) = host.top_window_opener(scope) {
            return Some(opener.into());
        }
    }
    let opener = (|| {
        let host = receiver
            .get_creation_context(scope)
            .and_then(context_host_ptr_from_context_slot)?;
        let environment = unsafe { &*host }.page_script_environment()?;
        let opener = environment.opener_in_scope(scope)?;
        let host = opener
            .get_creation_context(scope)
            .and_then(context_host_ptr_from_context_slot)?;
        (!unsafe { &*host }.browsing_context_is_closed()).then_some(opener)
    })();
    Some(opener.map(Into::into).unwrap_or(null))
}
