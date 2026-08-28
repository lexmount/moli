use crate::{
    context_bootstrap::{
        navigation_entry::history_length_number,
        navigation_events::{
            dispatch_beforeunload_for_runtime_owner, dispatch_pagehide_for_runtime_owner,
            dispatch_unload_for_runtime_owner,
        },
        navigation_window::window_history_for_holder,
        window_accessors::{window_child_context_handle, window_host_ptr},
        window_receiver,
    },
    native_bridge::OwnerDispatchScope,
    runtime::RendererTopLevelCloseSource,
    util::{context_host_ptr_from_context_slot, context_host_ptr_from_global_bridge},
    webidl,
};

fn valid_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if window_receiver::is_window_receiver(scope, receiver) {
        return true;
    }
    webidl::throw_type_error(scope, "Window operation called on incompatible receiver.");
    false
}

pub(in crate::context_bootstrap) fn window_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    if !valid_window_receiver(scope, receiver) {
        return;
    }
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        return;
    };
    if let Some(child_handle) = window_child_context_handle(scope, receiver) {
        crate::native_bridge::element::update_focus(scope, host_ptr, Some(child_handle));
    }
    let _ = request_top_level_browsing_context_focus(scope, host_ptr);
}

/// Applies Blink's top-level `DOMWindow::focus()` admission and publishes the
/// exact target Page owner action. An active Page may still move its focused
/// frame without another browser transaction; an inactive Page requires the
/// incumbent's transient activation or the target's opener exemption.
pub(crate) fn request_top_level_browsing_context_focus(
    scope: &mut v8::PinScope<'_, '_>,
    target_host_ptr: *mut crate::native_bridge::JsContextHost,
) -> bool {
    if unsafe { &*target_host_ptr }.top_level_browsing_context_is_closed() {
        return false;
    }
    if unsafe { &*target_host_ptr }.top_level_page_is_active() {
        return true;
    }
    let Some(target_page) = (unsafe { &*target_host_ptr }).top_level_page_residence() else {
        return false;
    };

    let current_context = scope.get_current_context();
    let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
    let incumbent_window = incumbent_context.global(scope);
    let incumbent_host_ptr = context_host_ptr_from_context_slot(incumbent_context);
    let consumed_interaction = incumbent_host_ptr.is_some_and(|source_host_ptr| unsafe {
        (&mut *source_host_ptr).consume_transient_user_activation_for_window_focus()
    });
    let opener_exemption = unsafe { &*target_host_ptr }
        .top_level_opener_value(scope)
        .is_some_and(|opener| opener.strict_equals(incumbent_window.into()));
    if !consumed_interaction && !opener_exemption {
        return false;
    }
    let output_host_ptr = incumbent_host_ptr.unwrap_or(target_host_ptr);
    unsafe { &*output_host_ptr }.append_live_turn_owner_action(
        crate::runtime::RendererOwnerAction::TopLevelFocus(target_page),
    )
}

/// Applies a focus request whose source-side activation/opener admission was
/// completed before crossing a RemoteWindowProxy transport boundary. The
/// target renderer still rechecks Page liveness and publishes the exact Page
/// residence; it must not try to rediscover the remote incumbent V8 context.
pub(crate) fn accept_remote_top_level_browsing_context_focus(
    target_host_ptr: *mut crate::native_bridge::JsContextHost,
) -> bool {
    if unsafe { &*target_host_ptr }.top_level_browsing_context_is_closed() {
        return false;
    }
    if unsafe { &*target_host_ptr }.top_level_page_is_active() {
        return true;
    }
    let Some(target_page) = (unsafe { &*target_host_ptr }).top_level_page_residence() else {
        return false;
    };
    unsafe { &*target_host_ptr }.append_live_turn_owner_action(
        crate::runtime::RendererOwnerAction::TopLevelFocus(target_page),
    )
}

pub(in crate::context_bootstrap) fn window_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    if !valid_window_receiver(scope, receiver) {
        return;
    }
    // Nested browsing contexts do not own a top-level target. Their legacy
    // `Window.close()` surface remains a no-op.
    if window_child_context_handle(scope, receiver).is_some() {
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ = request_top_level_browsing_context_close(
        scope,
        host_ptr,
        RendererTopLevelCloseSource::Window,
    );
}

fn top_level_window_is_script_closable(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
) -> bool {
    if unsafe { &*host_ptr }.top_level_browsing_context_opened_by_dom()
        || unsafe { &*host_ptr }.allow_scripts_to_close_windows()
    {
        return true;
    }
    let window = scope.get_current_context().global(scope);
    window_history_for_holder(scope, window)
        .and_then(|history| history_length_number(scope, history))
        .is_none_or(|length| length <= 1.0)
}

fn beforeunload_request_allows_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    dispatch_scope: OwnerDispatchScope,
    owner: v8::Local<'s, v8::Object>,
    force_browser_dialog_handler: bool,
    prior_prompt_was_accepted: &mut bool,
) -> bool {
    if !dispatch_beforeunload_for_runtime_owner(scope, owner) {
        return true;
    }
    // Chromium suppresses the confirmation panel without sticky activation.
    // Script still receives beforeunload, but its cancellation request cannot
    // veto the close in that case.
    if !unsafe { &*host_ptr }.sticky_user_activation() || *prior_prompt_was_accepted {
        return true;
    }
    let accepted = unsafe { &mut *host_ptr }
        .open_beforeunload_dialog_for_dispatch_scope(dispatch_scope, force_browser_dialog_handler)
        .is_some_and(|result| result.accepted);
    if accepted {
        *prior_prompt_was_accepted = true;
    }
    accepted
}

fn dispatch_top_level_beforeunload_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    source: RendererTopLevelCloseSource,
) -> bool {
    let mut prior_prompt_was_accepted = false;
    let force_browser_dialog_handler = source == RendererTopLevelCloseSource::Page;
    let window = scope.get_current_context().global(scope);
    if !beforeunload_request_allows_close(
        scope,
        host_ptr,
        OwnerDispatchScope::Top,
        window,
        force_browser_dialog_handler,
        &mut prior_prompt_was_accepted,
    ) {
        return false;
    }

    let child_handles = unsafe { &*host_ptr }.child_browsing_context_handles_in_document_order();
    for child_handle in child_handles {
        let Some(child_window) = (unsafe { &mut *host_ptr })
            .existing_child_browsing_context_window_wrapper(scope, child_handle)
        else {
            continue;
        };
        if !beforeunload_request_allows_close(
            scope,
            host_ptr,
            OwnerDispatchScope::Child(child_handle),
            child_window,
            force_browser_dialog_handler,
            &mut prior_prompt_was_accepted,
        ) {
            return false;
        }
    }
    true
}

/// Runs the renderer-owned close preflight and commits Closing only after every
/// current local frame has accepted beforeunload.
pub(crate) fn request_top_level_browsing_context_close(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    source: RendererTopLevelCloseSource,
) -> bool {
    if unsafe { &*host_ptr }.top_level_browsing_context_is_closed() {
        return true;
    }
    if source == RendererTopLevelCloseSource::Window
        && !top_level_window_is_script_closable(scope, host_ptr)
    {
        tracing::debug!(
            "Scripts may close only the windows that were opened by them or have a single history entry"
        );
        return false;
    }
    if !unsafe { &mut *host_ptr }.begin_top_level_beforeunload_dispatch() {
        // A close requested re-entrantly from beforeunload is blocked. The
        // outer decision retains sole authority over the transaction.
        return false;
    }
    let allows_close = dispatch_top_level_beforeunload_subtree(scope, host_ptr, source);
    unsafe { &mut *host_ptr }.finish_top_level_beforeunload_dispatch();
    if !allows_close {
        return false;
    }
    unsafe { &mut *host_ptr }.accept_top_level_browsing_context_close(source)
}

/// Dispatches the accepted close's non-cancelable Page lifecycle exactly once.
/// The browser owner treats return as the renderer unload ACK.
pub(crate) fn dispatch_top_level_browsing_context_close_unload(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
) -> bool {
    if !unsafe { &mut *host_ptr }.begin_top_level_unload_dispatch() {
        return false;
    }
    let window = scope.get_current_context().global(scope);
    dispatch_pagehide_for_runtime_owner(scope, window);
    dispatch_unload_for_runtime_owner(scope, window);
    let child_handles = unsafe { &*host_ptr }.child_browsing_context_handles_in_document_order();
    for child_handle in child_handles {
        let _ = unsafe { &mut *host_ptr }
            .dispatch_child_browsing_context_close_unload_lifecycle_if_needed(scope, child_handle);
    }
    true
}

pub(in crate::context_bootstrap) fn window_closed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    if !valid_window_receiver(scope, receiver) {
        return;
    }
    if window_child_context_handle(scope, receiver).is_some() {
        rv.set_bool(false);
        return;
    }
    let closed = context_host_ptr_from_global_bridge(scope)
        .is_none_or(|host_ptr| unsafe { &*host_ptr }.top_level_browsing_context_is_closed());
    rv.set_bool(closed);
}
