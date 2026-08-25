use super::super::{
    location_navigation::{LocationNavigationKind, navigate_location_object},
    navigation_cancellation::inform_about_canceled_navigation_for_window,
};
use crate::{
    context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    document_runtime::{DocumentPolicyContainer, DomHandle},
    native_bridge::{
        child_window_handle_from_marker_data,
        element::{
            SpecialBrowsingContextTarget, navigate_existing_browsing_context_target,
            navigate_named_iframe_target,
        },
        entered_child_window_handle,
    },
    runtime::{
        RendererPendingJavaScriptDialog, RendererPendingPopupActivation,
        RendererPendingWindowOpenEvent,
    },
    util::{context_host_ptr_from_global_bridge, get_private_value},
    webidl,
};
use url::Url;

use super::window_features::WindowOpenFeatures;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window dialog")]
struct WindowDialogMessageArgs {
    #[webidl(default = "")]
    message: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.prompt")]
struct WindowPromptArgs {
    #[webidl(default = "")]
    message: String,
    #[webidl(default = "")]
    default_prompt: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.open")]
struct WindowOpenArgs {
    #[webidl(default = "", converter = "usv_string")]
    raw_url: String,
    #[webidl(default = "")]
    target_name: String,
    #[webidl(default = "")]
    features: String,
}

pub(in crate::context_bootstrap) fn window_alert_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WindowDialogMessageArgs>(scope, &args) else {
        return;
    };
    let _ = open_dialog(scope, "alert", &parsed.message, "");
}

pub(crate) fn window_noop_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

pub(crate) fn window_stop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    inform_about_canceled_navigation_for_window(scope, args.this());
}

pub(in crate::context_bootstrap) fn window_confirm_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WindowDialogMessageArgs>(scope, &args) else {
        return;
    };
    let accepted =
        open_dialog(scope, "confirm", &parsed.message, "").is_some_and(|result| result.accepted);
    rv.set(v8::Boolean::new(scope, accepted).into());
}

pub(in crate::context_bootstrap) fn window_prompt_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WindowPromptArgs>(scope, &args) else {
        return;
    };
    if let Some(result) = open_dialog(scope, "prompt", &parsed.message, &parsed.default_prompt)
        && result.accepted
    {
        if let Some(user_input) = v8::String::new(scope, &result.user_input) {
            rv.set(user_input.into());
        }
        return;
    }
    rv.set(v8::null(scope).into());
}

pub(crate) fn window_const_false_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Boolean::new(scope, false).into());
}

pub(crate) fn window_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WindowOpenArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let entered_window = {
        let host = unsafe { &*host_ptr };
        window_open_entered_window(scope, host).unwrap_or_else(|| args.this())
    };
    let special_target = SpecialBrowsingContextTarget::parse(&parsed.target_name);
    let entered_base_url = {
        let host = unsafe { &*host_ptr };
        entered_window_api_base_url(scope, host)
    };
    let url = if parsed.raw_url.is_empty() {
        Url::parse("about:blank").expect("about:blank should parse")
    } else {
        match Url::options()
            .base_url(Some(&entered_base_url))
            .parse(&parsed.raw_url)
        {
            Ok(url) => url,
            Err(_) => {
                webidl::throw_dom_exception(
                    scope,
                    "SyntaxError",
                    "Unable to open a window with an invalid URL.",
                );
                return;
            }
        }
    };
    if special_target == Some(SpecialBrowsingContextTarget::Current) {
        navigate_window_open_self(scope, entered_window, url.as_str(), &mut rv);
        return;
    }
    let parsed_features = WindowOpenFeatures::parse(&parsed.features);
    let suppress_opener = parsed_features.suppresses_opener();
    let mut creator_policy_container = {
        let host = unsafe { &*host_ptr };
        window_open_entered_policy_container(scope, host)
    };
    creator_policy_container.document_referrer = if suppress_opener {
        String::new()
    } else {
        let host = unsafe { &*host_ptr };
        window_open_entered_document_url(scope, host).to_string()
    };
    if url.scheme() == "javascript" {
        let source = crate::native_bridge::javascript_url_csp_source(&url);
        let host = unsafe { &mut *host_ptr };
        let owner = host.entered_owner_dispatch_scope(scope);
        if !host.allows_inline_javascript_navigation_by_csp(scope, owner, &source) {
            rv.set(v8::null(scope).into());
            return;
        }
    }
    let url = url.to_string();
    if let Some(
        target @ (SpecialBrowsingContextTarget::Parent | SpecialBrowsingContextTarget::Top),
    ) = special_target
    {
        match navigate_existing_browsing_context_target(scope, host_ptr, target, &url) {
            Some(window) => rv.set(window.into()),
            None => rv.set(v8::null(scope).into()),
        }
        return;
    }
    if let Some(target_window) =
        existing_named_child_window_for_window_open(scope, host_ptr, &parsed.target_name)
        && !suppress_opener
        && navigate_named_iframe_target(scope, host_ptr, &parsed.target_name, &url, None)
    {
        rv.set(target_window.into());
        return;
    }
    let host = unsafe { &mut *host_ptr };
    let window_open_event = RendererPendingWindowOpenEvent {
        url: url.clone(),
        window_name: if parsed.target_name.is_empty() {
            "_blank".to_owned()
        } else {
            parsed.target_name.clone()
        },
        window_features: parsed_features.enabled_feature_strings(),
        user_gesture: host.protocol_user_gesture_activation(),
    };
    let source_scope = host.entered_owner_dispatch_scope(scope);
    let Some((_, root_document, source)) =
        host.renderer_window_document_source_for_dispatch_scope(source_scope)
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    let opener = (!suppress_opener).then_some(entered_window);
    let opener_child_handle =
        opener.and_then(|opener| window_open_receiver_child_handle(scope, opener));
    if popup_target_can_use_lightweight_window(&parsed.target_name, &url)
        && let Some(opened_popup) = host.open_lightweight_popup_window(
            scope,
            host_ptr,
            opener,
            opener_child_handle,
            &parsed.target_name,
            &url,
            entered_base_url,
            creator_policy_container,
        )
    {
        let popup_id = opened_popup.popup_id;
        let session_storage_store = host.lightweight_popup_session_storage_store(popup_id);
        let initial_empty_document_storage_key =
            host.lightweight_popup_initial_empty_document_storage_key(popup_id);
        let window_open_event = opened_popup
            .created_new_browsing_context
            .then_some(window_open_event);
        let popup_disposition = host
            .current_input_popup_disposition()
            .unwrap_or(crate::RendererPopupDisposition::Foreground);
        host.record_pending_popup_activation(
            RendererPendingPopupActivation::window(
                root_document,
                source,
                !suppress_opener,
                Some(popup_id),
                url,
                parsed.target_name,
                popup_disposition,
            )
            .with_initial_auxiliary_state(
                session_storage_store,
                initial_empty_document_storage_key,
            ),
            window_open_event,
        );
        if suppress_opener {
            rv.set(v8::null(scope).into());
        } else {
            rv.set(opened_popup.window.into());
        }
        return;
    }
    let popup_disposition = host
        .current_input_popup_disposition()
        .unwrap_or(crate::RendererPopupDisposition::Foreground);
    host.record_pending_popup_activation(
        RendererPendingPopupActivation::window(
            root_document,
            source,
            !suppress_opener,
            None,
            url,
            parsed.target_name,
            popup_disposition,
        )
        .with_initial_auxiliary_state(None, None),
        Some(window_open_event),
    );
    rv.set(v8::null(scope).into());
}

fn window_open_receiver_child_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, receiver, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
}

pub(in crate::context_bootstrap) fn entered_window_api_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> Url {
    if let Some(handle) = entered_child_window_handle(scope)
        && let Some(url) = host.child_browsing_context_base_url(handle)
    {
        return url;
    }
    if let Some(url) = host.active_lightweight_popup_base_url(scope) {
        return url;
    }
    host.dom_host()
        .document_base_url()
        .unwrap_or_else(|| host.document_url().clone())
}

fn window_open_entered_document_url(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> Url {
    if let Some(handle) = entered_child_window_handle(scope) {
        return host.document_url_for_child_context(handle);
    }
    if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope)
        && let Some(url) = host.lightweight_popup_document_url(popup_id)
    {
        return url;
    }
    host.dom_host()
        .document_url()
        .cloned()
        .unwrap_or_else(|| host.document_url().clone())
}

fn window_open_entered_policy_container(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> DocumentPolicyContainer {
    if let Some(handle) = entered_child_window_handle(scope)
        && let Some(policy_container) =
            host.child_browsing_context_policy_container_snapshot(handle)
    {
        return policy_container;
    }
    if let Some(policy_container) = host.active_lightweight_popup_policy_container(scope) {
        return policy_container.clone();
    }
    host.document_policy_container().clone()
}

fn window_open_entered_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> Option<v8::Local<'s, v8::Object>> {
    match host.entered_owner_dispatch_scope(scope) {
        crate::native_bridge::OwnerDispatchScope::Top => {
            Some(scope.get_current_context().global(scope))
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.existing_child_browsing_context_window_wrapper(scope, handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.lightweight_popup_window(scope, popup_id)
        }
    }
}

fn popup_target_can_use_lightweight_window(target_name: &str, href: &str) -> bool {
    Url::parse(href).is_ok()
        && (target_name.is_empty()
            || SpecialBrowsingContextTarget::parse(target_name)
                == Some(SpecialBrowsingContextTarget::Blank)
            || trackable_named_popup_target_name(target_name).is_some())
}

fn trackable_named_popup_target_name(target_name: &str) -> Option<&str> {
    if target_name.is_empty() || SpecialBrowsingContextTarget::parse(target_name).is_some() {
        return None;
    }
    Some(target_name)
}

fn navigate_window_open_self<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    url: &str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(location) =
        super::super::navigation_window::window_location_for_holder(scope, receiver)
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    navigate_location_object(
        scope,
        location,
        LocationNavigationKind::Assign,
        Some(url.to_owned()),
    );
    rv.set(receiver.into());
}

fn existing_named_child_window_for_window_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    target_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    if target_name.is_empty() || SpecialBrowsingContextTarget::parse(target_name).is_some() {
        return None;
    }
    let host = unsafe { &mut *host_ptr };
    let handle = host.child_browsing_context_handle_by_name(target_name)?;
    host.child_browsing_context_window_wrapper(scope, handle)
}

fn open_dialog(
    scope: &mut v8::PinScope<'_, '_>,
    dialog_type: &str,
    message: &str,
    default_prompt: &str,
) -> Option<crate::runtime::RendererJavaScriptDialogResult> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &mut *host_ptr };
    // Protocol handling starts only after this request is bound to an exact
    // Page/Document source. A standalone or stale realm uses the headless
    // default result instead of claiming a dialog that cannot be emitted.
    let (target, source_document, source) = host.current_renderer_window_document_source(scope)?;
    let source_url = window_open_entered_document_url(scope, host).to_string();
    let dialog_id = host.allocate_javascript_dialog_id();
    host.open_modal_javascript_dialog(
        target,
        RendererPendingJavaScriptDialog::new(
            dialog_id,
            source_document,
            source,
            source_url,
            dialog_type.to_owned(),
            message.to_owned(),
            default_prompt.to_owned(),
            None,
        ),
    )
}
