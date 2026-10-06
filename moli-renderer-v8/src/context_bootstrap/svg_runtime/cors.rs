use moli_webapi_declare::WebApiFunctionTemplate;

use super::callbacks::svg_animated_string_attribute_getter;
use crate::{
    native_bridge::{
        element::{
            canonical_cross_origin_value, remove_reflected_attribute, set_reflected_attribute_utf16,
        },
        node_runtime_and_handle_from_object_or_detached,
    },
    util::v8str,
    web_api_interfaces, webidl,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "crossOrigin")]
struct CrossOriginSetterArgs {
    #[webidl(required, nullable, converter = "dom_string16")]
    value: Option<Vec<u16>>,
}

macro_rules! cors_accessors {
    ($declaration:ident, $interface:ident) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = web_api_interfaces::$interface, enumerable, receiver)]
        struct $declaration {
            #[webapi(accessor_property = "crossOrigin", getter = cross_origin_getter, setter = cross_origin_setter)]
            cross_origin: (),
        }
    };
}

cors_accessors!(ImageDeclaration, SVGImageElement);
cors_accessors!(ScriptDeclaration, SVGScriptElement);

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGFEImageElement, enumerable, receiver)]
struct FilterImageDeclaration {
    #[webapi(accessor_property = "crossOrigin", getter = filter_image_cross_origin_getter)]
    cross_origin: (),
}

pub(super) fn install_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "SVGImageElement" => ImageDeclaration::initialize_prototype_template(scope, prototype),
        "SVGScriptElement" => ScriptDeclaration::initialize_prototype_template(scope, prototype),
        "SVGFEImageElement" => {
            FilterImageDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn cross_origin_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVG CORS receiver check validates native identity");
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .expect("native SVG element has a node handle");
    let raw = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute_ns(handle, None, "crossorigin");
    match raw {
        Some(raw) => rv.set(v8str(scope, canonical_cross_origin_value(&raw)).into()),
        None => rv.set_null(),
    }
}

fn cross_origin_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CrossOriginSetterArgs>(scope, &args) else {
        return;
    };
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVG CORS receiver check validates native identity");
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .expect("native SVG element has a node handle");
    match parsed.value {
        Some(units) => {
            set_reflected_attribute_utf16(scope, runtime_ptr, handle, "crossorigin", units)
        }
        None => remove_reflected_attribute(scope, runtime_ptr, handle, "crossorigin"),
    }
}

fn filter_image_cross_origin_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGFEImageElement receiver check validates native identity");
    let context = receiver
        .get_creation_context(scope)
        .expect("native SVGFEImageElement has a realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    svg_animated_string_attribute_getter(
        scope,
        receiver,
        rv,
        "__moliSvgFeImageCrossOrigin",
        "crossorigin",
    );
}
