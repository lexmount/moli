use crate::document_runtime::DomHandle;
use crate::native_bridge::context_host::ChildBrowsingContextNavigationRequest;
use crate::util::{context_host_ptr_from_global_bridge, get_private_value, v8_string};
use moli_dom::forms::normalize_form_submission_newlines;
use moli_encoding::{form_submission_encoding, form_urlencoded_serialize_pairs};
use url::Url;

use super::*;

const CHILD_DOCUMENT_CONTEXT_HANDLE_SLOT: &str = "__lmChildDocumentContextHandle";

pub(in crate::native_bridge::document) fn install_detached_form_instance_properties(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) {
    for name in ["name", "elements", "length"] {
        let _ = object.delete(scope, v8str(scope, name).into());
    }
    for name in ["submit", "reset"] {
        let _ = object.delete(scope, v8str(scope, name).into());
    }
}

pub(in crate::native_bridge) fn detached_form_submit_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let form = args.this();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(form_handle) = detached_native_handle(scope, form) else {
        rv.set_undefined();
        return;
    };
    let Some(owner_document) = detached_owner_document_object(scope, form) else {
        rv.set_undefined();
        return;
    };
    let owner_urls =
        detached_form_owner_urls(scope, runtime_ptr, owner_document).unwrap_or_else(|| {
            let url = unsafe { &*runtime_ptr }.document_url().clone();
            (url.clone(), url)
        });
    let action_attribute = read_detached_native_attribute(scope, form, "action");
    let action = action_attribute
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(owner_urls.0.as_str());
    let method = read_detached_native_attribute(scope, form, "method").unwrap_or_default();
    let Ok(mut url) = Url::parse(action).or_else(|_| owner_urls.1.join(action)) else {
        rv.set_undefined();
        return;
    };
    let submission_encoding =
        detached_form_submission_encoding(scope, runtime_ptr, owner_document, form);
    let mut entries = Vec::new();
    for handle in detached_form_control_descendants(runtime_ptr, form_handle) {
        let Some(control) = detached_native_object_for_handle(scope, runtime_ptr, handle) else {
            continue;
        };
        let Some(name) = detached_control_submission_name(scope, control) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let mut value = detached_control_submission_value(scope, runtime_ptr, handle, control);
        if moli_encoding::is_charset_sentinel_name(&name) {
            value = submission_encoding.name().to_owned();
        }
        entries.push((
            normalize_form_submission_newlines(&name),
            normalize_form_submission_newlines(&value),
        ));
    }
    let body = form_urlencoded_serialize_pairs(
        entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        submission_encoding,
    );
    if method.eq_ignore_ascii_case("post") {
        let Some(child_handle) = detached_child_document_context_handle(scope, owner_document)
        else {
            rv.set_undefined();
            return;
        };
        unsafe { &mut *runtime_ptr }.navigate_child_browsing_context_with_request(
            scope,
            child_handle,
            ChildBrowsingContextNavigationRequest {
                url,
                method: "POST".to_owned(),
                body: Some(body.into_bytes()),
                request_headers: vec![(
                    "Content-Type".to_owned(),
                    "application/x-www-form-urlencoded".to_owned(),
                )],
            },
        );
    } else if let Some(target_iframe) =
        detached_form_target_iframe(scope, runtime_ptr, owner_document, form)
    {
        url.set_query(Some(&body));
        navigate_detached_iframe_to_url(scope, target_iframe, url.as_str());
        let _ = crate::detached_event_target::dispatch_detached_simple_event(
            scope,
            target_iframe,
            "load",
            false,
            false,
            false,
        );
    } else {
        let Some(child_handle) = detached_child_document_context_handle(scope, owner_document)
        else {
            rv.set_undefined();
            return;
        };
        url.set_query(Some(&body));
        let _ = unsafe { &mut *runtime_ptr }.navigate_child_browsing_context_to_url(
            scope,
            child_handle,
            url.as_str(),
        );
    }
    rv.set_undefined();
}

fn detached_child_document_context_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let child_handle_value =
        get_private_value(scope, owner_document, CHILD_DOCUMENT_CONTEXT_HANDLE_SLOT)?;
    let child_handle_bigint = v8::Local::<v8::BigInt>::try_from(child_handle_value).ok()?;
    let (child_handle_index, child_handle_lossless) = child_handle_bigint.u64_value();
    child_handle_lossless.then(|| DomHandle::new(child_handle_index as usize))
}

fn detached_form_target_iframe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    owner_document: v8::Local<'s, v8::Object>,
    form: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = read_detached_native_attribute(scope, form, "target")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;
    let document_handle = detached_native_handle(scope, owner_document)?;
    let runtime = unsafe { &*runtime_ptr };
    let target_handle = find_detached_iframe_by_name(runtime, document_handle, &target)?;
    detached_native_object_for_handle(scope, runtime_ptr, target_handle)
}

fn detached_form_submission_encoding<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    _owner_document: v8::Local<'s, v8::Object>,
    form: v8::Local<'s, v8::Object>,
) -> &'static encoding_rs::Encoding {
    let accept_charset = read_detached_native_attribute(scope, form, "accept-charset");
    form_submission_encoding(
        accept_charset.as_deref(),
        unsafe { &*runtime_ptr }.document_character_set(),
    )
}

fn find_detached_iframe_by_name(
    runtime: &crate::native_bridge::JsContextHost,
    root: DomHandle,
    target: &str,
) -> Option<DomHandle> {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        if runtime
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.is_html_element("iframe"))
            && runtime.dom_host().get_attribute(handle, "name").as_deref() == Some(target)
        {
            return Some(handle);
        }
        stack.extend(runtime.dom_host().child_handles_reversed(handle));
    }
    None
}

fn detached_form_owner_urls<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    owner_document: v8::Local<'s, v8::Object>,
) -> Option<(Url, Url)> {
    let document_handle = detached_native_handle(scope, owner_document)?;
    let runtime = unsafe { &*runtime_ptr };
    runtime
        .dom_host()
        .node(document_handle)
        .and_then(|node| node.as_document())
        .map(|document| (document.url().clone(), document.base_url().clone()))
}

pub(in crate::native_bridge) fn detached_form_reset_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let form = args.this();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(form_handle) = detached_native_handle(scope, form) else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if runtime
        .dom_host()
        .node(form_handle)
        .and_then(|node| node.as_element())
        .is_none_or(|element| !element.is_html_form())
    {
        rv.set_undefined();
        return;
    }

    if let Some(event) =
        crate::native_bridge::element::construct_simple_event(scope, "reset", true, true, false)
    {
        align_detached_form_reset_event_realm(scope, runtime_ptr, form, event);
        if !crate::native_bridge::element::dispatch_public_event(
            scope,
            runtime_ptr,
            form_handle,
            event,
        )
        .allows_default()
        {
            rv.set_undefined();
            return;
        }
        let _ = crate::native_bridge::element::reset_form_default_action(
            scope,
            runtime_ptr,
            form_handle,
            crate::native_bridge::element::FormAssociatedResetCallbackTiming::Sync,
        );
    }
    rv.set_undefined();
}

fn align_detached_form_reset_event_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _runtime_ptr: *mut crate::native_bridge::JsContextHost,
    form: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    let Some(owner_document) = detached_owner_document_object(scope, form) else {
        crate::native_bridge::element::align_event_constructor_function_realm_with_target(
            scope, event, form,
        );
        return;
    };
    let Some(default_view) = owner_document
        .get(scope, v8str(scope, "defaultView").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        crate::native_bridge::element::align_event_constructor_function_realm_with_target(
            scope, event, form,
        );
        return;
    };
    let Some(constructor_key) = v8_string(scope, "HTMLFormElement") else {
        return;
    };
    let Some(constructor) = default_view
        .get(scope, constructor_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        crate::native_bridge::element::align_event_constructor_function_realm_with_target(
            scope, event, form,
        );
        return;
    };
    crate::native_bridge::element::align_event_constructor_function_realm_with_constructor(
        scope,
        event,
        constructor,
    );
}

fn detached_form_control_descendants(
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    form_handle: DomHandle,
) -> Vec<DomHandle> {
    let mut controls = Vec::new();
    let mut stack = unsafe { &*runtime_ptr }
        .dom_host()
        .child_handles_reversed(form_handle)
        .collect::<Vec<_>>();
    while let Some(handle) = stack.pop() {
        if let Some(local_name) = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())
            .map(|element| element.local_name())
            && matches!(
                local_name,
                "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
            )
        {
            controls.push(handle);
        }
        stack.extend(
            unsafe { &*runtime_ptr }
                .dom_host()
                .child_handles_reversed(handle),
        );
    }
    controls
}

fn detached_control_submission_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    control: v8::Local<'s, v8::Object>,
) -> Option<String> {
    read_detached_native_attribute(scope, control, "name").filter(|name| !name.is_empty())
}

fn detached_control_submission_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: DomHandle,
    control: v8::Local<'s, v8::Object>,
) -> String {
    let runtime = unsafe { &*runtime_ptr };
    if let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
    {
        if element.is_html_input() {
            return crate::native_bridge::element::text_control_value(runtime, handle);
        }
        if element.is_html_textarea() {
            let mut value = crate::native_bridge::element::text_control_value(runtime, handle);
            if !element.input_value_dirty()
                && value.is_empty()
                && let Some(attribute_value) = element.attribute_ns("", "value")
            {
                value = attribute_value.to_owned();
            }
            return moli_dom::forms::apply_textarea_wrapping_transformation(
                value,
                element.attribute_ns("", "wrap"),
                element.attribute_ns("", "cols"),
            );
        }
        if element.is_html_button() {
            return element
                .attribute_ns("", "value")
                .map(str::to_owned)
                .unwrap_or_default();
        }
    }
    control
        .get(scope, v8str(scope, "value").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub(in crate::native_bridge::document) fn install_detached_form_control_instance_properties<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let _ = object.delete(scope, v8str(scope, "form").into());
}
