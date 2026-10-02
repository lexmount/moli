use super::super::{
    attribute_property_getter_from_object_or_detached, html_element_getter_receiver,
    html_element_setter_receiver, set_dom_string_attribute_property_on_object,
};

pub(in crate::native_bridge::element) fn meta_content_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    meta_string_getter(scope, args.this(), "content", "content", rv);
}

pub(in crate::native_bridge::element) fn meta_content_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    meta_string_setter(scope, args.this(), "content", "content", args.get(0));
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn meta_http_equiv_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    meta_string_getter(scope, args.this(), "httpEquiv", "http-equiv", rv);
}

pub(in crate::native_bridge::element) fn meta_http_equiv_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    meta_string_setter(scope, args.this(), "httpEquiv", "http-equiv", args.get(0));
    rv.set_undefined();
}

fn meta_string_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
    attribute: &'static str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if html_element_getter_receiver(scope, receiver, "HTMLMetaElement", member, "meta").is_none() {
        return;
    }
    attribute_property_getter_from_object_or_detached(scope, receiver, attribute, rv);
}

fn meta_string_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
    attribute: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    if html_element_setter_receiver(scope, receiver, "HTMLMetaElement", member, "meta").is_none() {
        return;
    }
    set_dom_string_attribute_property_on_object(
        scope,
        receiver,
        attribute,
        value,
        "HTMLMetaElement",
        member,
    );
}

macro_rules! body_attr_reflection {
    (
        $getter:ident,
        $setter:ident,
        $attr_name:expr,
        $idl_name:expr,
        $set_attribute:ident $(,)?
    ) => {
        pub(in crate::native_bridge::element) fn $getter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            attribute_property_getter_from_object_or_detached(scope, args.this(), $attr_name, rv);
        }

        pub(in crate::native_bridge::element) fn $setter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            $set_attribute(
                scope,
                args.this(),
                $attr_name,
                args.get(0),
                "HTMLBodyElement",
                $idl_name,
            );
            rv.set_undefined();
        }
    };
}

body_attr_reflection!(
    body_background_getter_function,
    body_background_setter_function,
    "background",
    "background",
    set_dom_string_attribute_property_on_object
);
