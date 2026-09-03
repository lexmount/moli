use super::*;

fn token_list_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    kind: DomTokenListKind,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let token_list =
        runtime
            .native_bridge_mut()
            .wrap_dom_token_list(scope, runtime_ptr, handle, kind);
    match token_list {
        Some(token_list) => rv.set(token_list.into()),
        None => rv.set_null(),
    }
}

fn set_token_list_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
    attribute: &'static str,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, value, owner, property) else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
}

pub(in crate::native_bridge) fn html_rel_list_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    token_list_getter(scope, args, rv, DomTokenListKind::Rel);
}

pub(in crate::native_bridge) fn html_rel_list_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(interface) =
        super::super::reflection::ElementReflectionInterface::from_callback_data(scope, args.data())
    {
        set_token_list_for_receiver(
            scope,
            args.this(),
            args.get(0),
            interface.name(),
            "relList",
            "rel",
        );
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn svg_rel_list_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_token_list_for_receiver(
        scope,
        args.this(),
        args.get(0),
        "SVGAElement",
        "relList",
        "rel",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn output_html_for_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    token_list_getter(scope, args, rv, DomTokenListKind::HtmlFor);
}

pub(in crate::native_bridge) fn output_html_for_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_token_list_for_receiver(
        scope,
        args.this(),
        args.get(0),
        "HTMLOutputElement",
        "htmlFor",
        "for",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn iframe_sandbox_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    token_list_getter(scope, args, rv, DomTokenListKind::Sandbox);
}

pub(in crate::native_bridge) fn iframe_sandbox_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_token_list_for_receiver(
        scope,
        args.this(),
        args.get(0),
        "HTMLIFrameElement",
        "sandbox",
        "sandbox",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn link_sizes_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    token_list_getter(scope, args, rv, DomTokenListKind::Sizes);
}

pub(in crate::native_bridge) fn link_sizes_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_token_list_for_receiver(
        scope,
        args.this(),
        args.get(0),
        "HTMLLinkElement",
        "sizes",
        "sizes",
    );
    rv.set_undefined();
}
