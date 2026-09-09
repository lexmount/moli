use crate::{
    context_bootstrap::touch_feature_detection::{self, TOUCH_EVENT_HANDLER_PROPERTIES},
    document_runtime::EventTargetHandle,
    util::{context_host_ptr_from_global_bridge, node_wrapper_from_handle, v8_string, v8str},
};

use super::super::super::node::{
    node_is_document, node_is_element, node_runtime_and_handle_from_object_or_detached,
};
use super::super::forms::form_associated_form_owner;
use super::super::{element_attribute, queue_text_track_load_if_needed};
use super::shared::compile_event_attribute_handler;

const EVENT_HANDLER_SLOT_PREFIX: &str = "__moliEventHandler_";

pub(crate) const GENERIC_EVENT_HANDLER_PROPERTIES: &[&str] = &[
    "onclick",
    "onauxclick",
    "onload",
    "onerror",
    "onfocus",
    "onblur",
    "onkeydown",
    "onkeyup",
    "onkeypress",
    "onmousedown",
    "onmouseup",
    "onmousemove",
    "onmouseover",
    "onmouseout",
    "onmouseenter",
    "onmouseleave",
    "ondblclick",
    "onpointerdown",
    "onpointerup",
    "onpointermove",
    "onpointerrawupdate",
    "onpointerover",
    "onpointerout",
    "onpointerenter",
    "onpointerleave",
    "onpointercancel",
    "ongotpointercapture",
    "onlostpointercapture",
    "ontouchstart",
    "ontouchend",
    "ontouchmove",
    "ontouchcancel",
    "onsubmit",
    "onreset",
    "onchange",
    "oninput",
    "oninvalid",
    "ondrag",
    "ondragstart",
    "ondragend",
    "ondragenter",
    "ondragleave",
    "ondragover",
    "ondrop",
    "oncopy",
    "oncut",
    "onpaste",
    "onscroll",
    "onscrollend",
    "onslotchange",
    "onresize",
    "onstorage",
    "onanimationstart",
    "onanimationend",
    "onanimationiteration",
    "onanimationcancel",
    "ontransitionstart",
    "ontransitionend",
    "ontransitionrun",
    "ontransitioncancel",
    "onwheel",
    "onbeforetoggle",
    "ontoggle",
    "oncontextmenu",
    "onselect",
    "onselectionchange",
    "onabort",
    "oncancel",
    "onclose",
    "onplay",
    "onpause",
    "onplaying",
    "onended",
    "onvolumechange",
    "onwaiting",
    "onseeking",
    "onseeked",
    "ontimeupdate",
    "onloadstart",
    "onprogress",
    "onstalled",
    "onsuspend",
    "oncanplay",
    "oncanplaythrough",
    "ondurationchange",
    "onemptied",
    "onloadeddata",
    "onloadedmetadata",
    "onratechange",
];

const DOCUMENT_EVENT_HANDLER_PROPERTIES: &[&str] = &["onpointerlockchange", "onpointerlockerror"];

#[derive(Clone, Copy)]
pub(crate) enum GlobalEventHandlerOwner {
    Document,
    Element,
}

pub(crate) fn install_global_event_handler_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    owner: GlobalEventHandlerOwner,
) {
    let owner_properties: &[&str] = match owner {
        GlobalEventHandlerOwner::Document => DOCUMENT_EVENT_HANDLER_PROPERTIES,
        GlobalEventHandlerOwner::Element => &[],
    };
    let prototype = template.prototype_template(scope);
    for name in GENERIC_EVENT_HANDLER_PROPERTIES
        .iter()
        .chain(owner_properties)
    {
        // Document does not include WindowEventHandlers, which owns `onstorage`.
        if matches!(owner, GlobalEventHandlerOwner::Document) && *name == "onstorage" {
            continue;
        }
        if TOUCH_EVENT_HANDLER_PROPERTIES.contains(name) {
            continue;
        }
        let (getter, setter) = event_handler_templates(scope, name, owner);
        prototype.set_accessor_property(
            v8str(scope, name).into(),
            Some(getter),
            Some(setter),
            v8::PropertyAttribute::NONE,
        );
    }
}

fn event_handler_templates<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    name: &'static str,
    owner: GlobalEventHandlerOwner,
) -> (
    v8::Local<'s, v8::FunctionTemplate>,
    v8::Local<'s, v8::FunctionTemplate>,
) {
    let data = v8str(scope, name).into();
    let (getter, setter) = match owner {
        GlobalEventHandlerOwner::Document => (
            v8::FunctionTemplate::builder(document_event_handler_getter_function)
                .data(data)
                .length(0)
                .build(scope),
            v8::FunctionTemplate::builder(document_event_handler_setter_function)
                .data(data)
                .length(1)
                .build(scope),
        ),
        GlobalEventHandlerOwner::Element => (
            v8::FunctionTemplate::builder(node_event_handler_getter_function)
                .data(data)
                .length(0)
                .build(scope),
            v8::FunctionTemplate::builder(node_event_handler_setter_function)
                .data(data)
                .length(1)
                .build(scope),
        ),
    };
    if let Some(function_name) = v8_string(scope, &format!("get {name}")) {
        getter.set_class_name(function_name);
    }
    if let Some(function_name) = v8_string(scope, &format!("set {name}")) {
        setter.set_class_name(function_name);
    }
    (getter, setter)
}

pub(crate) fn install_touch_event_handler_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    owner: GlobalEventHandlerOwner,
) -> anyhow::Result<()> {
    if !touch_feature_detection::enabled(scope) {
        return Ok(());
    }
    for name in TOUCH_EVENT_HANDLER_PROPERTIES {
        let (getter, setter) = event_handler_templates(scope, name, owner);
        let getter = getter.get_function(scope).expect("touch handler getter");
        let setter = setter.get_function(scope).expect("touch handler setter");
        crate::definitions::define_get_set_property(
            scope,
            prototype,
            v8str(scope, name).into(),
            getter.into(),
            setter.into(),
            v8::PropertyAttribute::NONE,
            name,
        )?;
    }
    Ok(())
}

fn event_handler_property_value_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut super::super::super::JsContextHost,
    target: EventTargetHandle,
    data: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let Some(handler_name) = event_handler_name_from_data(scope, data) else {
        return v8::null(scope).into();
    };
    let Some(event_type) = event_handler_event_type(&handler_name) else {
        return v8::null(scope).into();
    };
    unsafe { &*runtime_ptr }
        .registered_event_handler_property_value(scope, target, event_type)
        .unwrap_or_else(|| v8::null(scope).into())
}

fn set_event_handler_property_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut super::super::super::JsContextHost,
    target: EventTargetHandle,
    data: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(handler_name) = event_handler_name_from_data(scope, data) else {
        return;
    };
    let Some(event_type) = event_handler_event_type(&handler_name) else {
        return;
    };
    let handler = v8::Local::<v8::Function>::try_from(value).ok();
    unsafe { &mut *runtime_ptr }
        .set_registered_event_handler_property(scope, target, event_type, handler);
}

fn document_event_handler_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    rv.set(event_handler_property_value_for_target(
        scope,
        runtime_ptr,
        EventTargetHandle::Node(handle),
        args.data(),
    ));
}

fn document_event_handler_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
        && node_is_document(unsafe { &*runtime_ptr }, handle)
    {
        set_event_handler_property_for_target(
            scope,
            runtime_ptr,
            EventTargetHandle::Node(handle),
            args.data(),
            args.get(0),
        );
    }
    rv.set_undefined();
}

pub(crate) fn node_event_handler_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler_name) = event_handler_name_from_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let object = args.this();
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) || !handler_name.starts_with("on") {
        rv.set_null();
        return;
    }
    if let Some(event_type) = event_handler_event_type(&handler_name)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(current) = unsafe { &*host_ptr }.registered_event_handler_property_value(
            scope,
            EventTargetHandle::Node(handle),
            event_type,
        )
    {
        rv.set(current);
        return;
    }
    let slot_name = event_handler_slot_name(&handler_name);
    let Some(slot_key) = v8_string(scope, &slot_name) else {
        rv.set_null();
        return;
    };
    if let Some(current) = object.get(scope, slot_key.into())
        && !current.is_undefined()
    {
        rv.set(current);
        return;
    }
    let Some(source) = element_attribute(unsafe { &*runtime_ptr }, handle, &handler_name) else {
        rv.set(v8::null(scope).into());
        return;
    };
    if source.is_empty() {
        rv.set(v8::null(scope).into());
        return;
    }

    let Some(target_context) = node_event_handler_target_context(scope, runtime_ptr, handle) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let handler = if target_context == scope.get_current_context() {
        compile_node_event_attribute_handler(
            scope,
            runtime_ptr,
            handle,
            object,
            &handler_name,
            &source,
        )
        .map(|handler| v8::Global::new(scope, handler))
    } else {
        let object = v8::Global::new(scope, object);
        let target_scope = &mut v8::ContextScope::new(scope, target_context);
        let object = v8::Local::new(target_scope, &object);
        compile_node_event_attribute_handler(
            target_scope,
            runtime_ptr,
            handle,
            object,
            &handler_name,
            &source,
        )
        .map(|handler| v8::Global::new(target_scope, handler))
    };
    match handler {
        Some(handler) => rv.set(v8::Local::new(scope, &handler).into()),
        None => rv.set(v8::null(scope).into()),
    }
}

fn node_event_handler_target_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Context>> {
    let runtime = unsafe { &*runtime_ptr };
    let owner_document = runtime.dom_host().owner_document_handle(handle)?;
    let dispatch_scope = if owner_document == runtime.dom_host().document_handle() {
        crate::native_bridge::OwnerDispatchScope::Top
    } else {
        crate::native_bridge::OwnerDispatchScope::Child(
            runtime.child_browsing_context_host_for_document_handle(owner_document)?,
        )
    };
    let owner = runtime.current_window_execution_context_owner(dispatch_scope)?;
    runtime
        .window_execution_context(scope, owner, dispatch_scope)
        .map(|(_, context)| context)
}

fn compile_node_event_attribute_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    object: v8::Local<'s, v8::Object>,
    handler_name: &str,
    source: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let event_argument = v8_string(scope, "event")?;
    let global = scope.get_current_context().global(scope);
    let mut context_extensions = Vec::with_capacity(3);
    if let Some(document) = global
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        context_extensions.push(document);
    }
    if let Some(form_owner) = form_associated_form_owner(unsafe { &*runtime_ptr }, handle)
        .and_then(|form_owner| node_wrapper_from_handle(scope, form_owner))
    {
        context_extensions.push(form_owner);
    }
    context_extensions.push(object);

    let handler = compile_event_attribute_handler(
        scope,
        runtime_ptr,
        handle,
        source,
        &[event_argument],
        &context_extensions,
    );
    if let Some(handler) = handler {
        if let Some(name) = v8_string(scope, handler_name) {
            handler.set_name(name);
        }
        if let Some(event_type) = event_handler_event_type(handler_name)
            && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        {
            let target_context = scope.get_current_context();
            unsafe { &mut *host_ptr }.set_registered_content_attribute_event_handler_property(
                scope,
                EventTargetHandle::Node(handle),
                event_type,
                Some(handler),
                target_context,
            );
        }
        Some(handler)
    } else {
        if let Some(event_type) = event_handler_event_type(handler_name)
            && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        {
            let target_context = scope.get_current_context();
            unsafe { &mut *host_ptr }.set_registered_content_attribute_event_handler_property(
                scope,
                EventTargetHandle::Node(handle),
                event_type,
                None,
                target_context,
            );
        }
        None
    }
}

pub(crate) fn node_event_handler_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler_name) = event_handler_name_from_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let slot_name = event_handler_slot_name(&handler_name);
    let Some(slot_key) = v8_string(scope, &slot_name) else {
        rv.set_undefined();
        return;
    };
    let object = args.this();
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    let _ = object.set(scope, slot_key.into(), stored);
    let runtime_and_handle = node_runtime_and_handle_from_object_or_detached(scope, object).ok();
    if let Some(event_type) = event_handler_event_type(&handler_name)
        && let Some((_runtime_ptr, handle)) = runtime_and_handle
    {
        let handler = v8::Local::<v8::Function>::try_from(value).ok();
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            unsafe { &mut *host_ptr }.set_registered_event_handler_property(
                scope,
                EventTargetHandle::Node(handle),
                event_type,
                handler,
            );
        }
    }
    if matches!(handler_name.as_str(), "onload" | "onerror")
        && let Some((runtime_ptr, handle)) = runtime_and_handle
        && unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(handle, "track")
    {
        queue_text_track_load_if_needed(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

fn event_handler_name_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<String> {
    v8::Local::<v8::String>::try_from(data)
        .ok()
        .map(|name| name.to_rust_string_lossy(scope))
}

fn event_handler_slot_name(name: &str) -> String {
    format!("{EVENT_HANDLER_SLOT_PREFIX}{name}")
}

fn event_handler_event_type(name: &str) -> Option<&str> {
    name.strip_prefix("on")
        .filter(|event_type| !event_type.is_empty())
}

pub(super) fn invalidate_node_event_attribute_handler(
    runtime: &mut super::super::super::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    name: &str,
) {
    let normalized_name = name.to_ascii_lowercase();
    let Some(event_type) = event_handler_event_type(&normalized_name) else {
        return;
    };
    runtime.clear_event_handler_property(EventTargetHandle::Node(handle), event_type);
}
