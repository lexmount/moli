use crate::{
    context_bootstrap::BODY_OR_FRAMESET_WINDOW_EVENT_HANDLER_PROPERTIES,
    document_runtime::{DomHandle, EventTargetHandle},
    native_bridge::{JsContextHost, OwnerDispatchScope},
    util::{v8_string, v8str},
};

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::element_attribute;
use super::shared::compile_event_attribute_handler_for_owner_with_context;

fn body_or_frameset_window_event_handler_properties() -> impl Iterator<Item = &'static str> {
    BODY_OR_FRAMESET_WINDOW_EVENT_HANDLER_PROPERTIES
        .iter()
        .copied()
}

pub(crate) fn body_or_frameset_reflects_window_event_type(event_type: &str) -> bool {
    body_or_frameset_window_event_handler_properties()
        .any(|name| name.strip_prefix("on") == Some(event_type))
}

pub(crate) fn install_body_or_frameset_window_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    for name in body_or_frameset_window_event_handler_properties() {
        let data = v8str(scope, name).into();
        let getter = v8::FunctionTemplate::builder(body_window_event_handler_getter_function)
            .data(data)
            .length(0)
            .build(scope);
        let setter = v8::FunctionTemplate::builder(body_window_event_handler_setter_function)
            .data(data)
            .length(1)
            .build(scope);
        if let Some(function_name) = v8_string(scope, &format!("get {name}")) {
            getter.set_class_name(function_name);
        }
        if let Some(function_name) = v8_string(scope, &format!("set {name}")) {
            setter.set_class_name(function_name);
        }
        prototype.set_accessor_property(
            v8str(scope, name).into(),
            Some(getter),
            Some(setter),
            v8::PropertyAttribute::NONE,
        );
    }
}

fn body_window_event_handler_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler_name) = handler_name_from_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let Some(event_type) = handler_name.strip_prefix("on") else {
        rv.set_null();
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = match super::body_or_frameset_window_owner(unsafe { &*runtime_ptr }, handle) {
        Some(OwnerDispatchScope::Top) => {
            resolve_window_event_handler_content_attribute(scope, runtime_ptr, event_type)
        }
        Some(OwnerDispatchScope::Child(child_handle)) => unsafe { &mut *runtime_ptr }
            .child_window_event_handler_property_value(scope, child_handle, &handler_name),
        Some(OwnerDispatchScope::LightweightPopup(popup_id)) => unsafe { &mut *runtime_ptr }
            .lightweight_popup_event_handler_property_value(scope, popup_id, &handler_name),
        None => None,
    };
    match value {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

fn body_window_event_handler_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler_name) = handler_name_from_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(event_type) = handler_name.strip_prefix("on") else {
        rv.set_undefined();
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let handler = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    match super::body_or_frameset_window_owner(unsafe { &*runtime_ptr }, handle) {
        Some(OwnerDispatchScope::Top) => unsafe { &mut *runtime_ptr }
            .set_registered_event_handler_property(
                scope,
                EventTargetHandle::Window,
                event_type,
                handler,
            ),
        Some(OwnerDispatchScope::Child(child_handle)) => {
            let relevant_context = handler
                .and_then(|handler| handler.get_creation_context(scope))
                .unwrap_or_else(|| scope.get_current_context());
            unsafe { &mut *runtime_ptr }.set_child_window_event_handler_property(
                scope,
                child_handle,
                &handler_name,
                handler,
                relevant_context,
            );
        }
        Some(OwnerDispatchScope::LightweightPopup(popup_id)) => unsafe { &mut *runtime_ptr }
            .set_lightweight_popup_event_handler_property(scope, popup_id, &handler_name, handler),
        None => {}
    }
    rv.set_undefined();
}

pub(crate) fn resolve_window_event_handler_content_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(value) = runtime.registered_event_handler_property_value(
        scope,
        EventTargetHandle::Window,
        event_type,
    ) {
        return Some(value);
    }
    let owner = runtime
        .uncompiled_event_handler_content_attribute_owner(EventTargetHandle::Window, event_type)?;
    if !super::body_or_frameset_uses_runtime_window(runtime, owner) {
        return None;
    }

    // Compilation can report an error, which dispatches a Window `error`
    // event and re-enters the corresponding getter. Replace the uncompiled
    // state before invoking V8 so that re-entry observes null instead of
    // recursively compiling the same content attribute.
    let target_context = scope.get_current_context();
    runtime.set_registered_content_attribute_event_handler_property(
        scope,
        EventTargetHandle::Window,
        event_type,
        None,
        target_context,
    );
    let Some(handler) = compile_body_window_event_attribute(scope, runtime_ptr, owner, event_type)
    else {
        // Error reporting can replace or deactivate the handler, even through
        // document.open(). Keep those changes and return null for this read.
        return Some(v8::null(scope).into());
    };
    let target_context = scope.get_current_context();
    unsafe { &mut *runtime_ptr }.set_registered_content_attribute_event_handler_property(
        scope,
        EventTargetHandle::Window,
        event_type,
        Some(handler),
        target_context,
    );
    Some(handler.into())
}

pub(crate) fn compile_body_window_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    owner: DomHandle,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let runtime = unsafe { &*runtime_ptr };
    let source = element_attribute(runtime, owner, &format!("on{event_type}"))?;
    let window_owner = runtime.owner_dispatch_scope_for_node(owner)?;
    let document = runtime.dom_host().owner_document_handle(owner)?;
    let base_url = runtime.document_base_url_for_handle(document);
    compile_window_event_attribute_handler(
        scope,
        runtime_ptr,
        window_owner,
        &base_url,
        &source,
        event_type,
    )
}

pub(crate) fn compile_window_event_attribute_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    window_owner: OwnerDispatchScope,
    base_url: &url::Url,
    source: &str,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let handler_name = format!("on{event_type}");
    let argument_names: &[&str] = if event_type == "error" {
        &["event", "source", "lineno", "colno", "error"]
    } else {
        &["event"]
    };
    let arguments = argument_names
        .iter()
        .filter_map(|name| v8_string(scope, name))
        .collect::<Vec<_>>();
    if arguments.len() != argument_names.len() {
        return None;
    }
    let runtime = unsafe { &*runtime_ptr };
    // Lightweight popups share a V8 context with their opener. A body handler
    // has only its Window in scope, even if the source element was adopted.
    let extensions = match window_owner {
        OwnerDispatchScope::LightweightPopup(popup_id) => {
            vec![runtime.lightweight_popup_window(scope, popup_id)?]
        }
        OwnerDispatchScope::Top | OwnerDispatchScope::Child(_) => Vec::new(),
    };
    let handler = compile_event_attribute_handler_for_owner_with_context(
        scope,
        runtime_ptr,
        window_owner,
        base_url,
        source,
        &arguments,
        &extensions,
    )?;
    if let Some(name) = v8_string(scope, &handler_name) {
        handler.set_name(name);
    }
    Some(handler)
}

pub(crate) fn initialize_parser_inserted_body_window_event_handlers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    initialize_parser_body_window_event_handlers(
        scope,
        runtime_ptr,
        handle,
        body_or_frameset_window_event_handler_properties(),
    );
}

/// Attributes accepted by a parser merge, captured before the DOM changes.
/// An ignored duplicate must not replace a compiled handler or reactivate one
/// that author code cleared through the corresponding IDL property.
pub(crate) struct ParserAddedBodyWindowHandlers {
    handle: DomHandle,
    names: Vec<&'static str>,
}

impl ParserAddedBodyWindowHandlers {
    pub(crate) fn capture(
        dom_host: &crate::dom::native::DomHost,
        handle: DomHandle,
        attrs: &[crate::dom::native::Attribute],
    ) -> Option<Self> {
        if !dom_host.node(handle).is_some_and(|node| {
            node.is_html_element_named("body") || node.is_html_element_named("frameset")
        }) {
            return None;
        }
        let names = body_or_frameset_window_event_handler_properties()
            .filter(|name| {
                dom_host.get_attribute(handle, name).is_none()
                    && attrs
                        .iter()
                        .any(|attr| attr.namespace().is_empty() && attr.local_name() == *name)
            })
            .collect::<Vec<_>>();
        (!names.is_empty()).then_some(Self { handle, names })
    }

    pub(crate) fn initialize(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut JsContextHost,
    ) {
        initialize_parser_body_window_event_handlers(scope, runtime_ptr, self.handle, self.names);
    }
}

fn initialize_parser_body_window_event_handlers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    names: impl IntoIterator<Item = &'static str>,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if super::body_or_frameset_window_owner(runtime, handle).is_none() {
        return;
    }
    for handler_name in names {
        if runtime
            .dom_host()
            .get_attribute(handle, handler_name)
            .is_some()
            && let Some(previous) = runtime.sync_event_handler_content_attribute(
                scope,
                runtime_ptr,
                handle,
                handler_name,
                None,
                true,
            )
        {
            runtime.release_event_callback(previous);
        }
    }
}

fn handler_name_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<String> {
    v8::Local::<v8::String>::try_from(data)
        .ok()
        .map(|name| name.to_rust_string_lossy(scope))
}
