//! Detached SVGRect values created by SVGSVGElement.createSVGRect().
//!
//! JSXGraph uses this method to detect SVG support. Returning a DOMRect would
//! pass that check but expose the wrong interface and double (not float) fields.

use crate::{
    native_bridge::node_runtime_and_handle_from_object_or_detached,
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const X: &str = "__moliSvgRectX";
const Y: &str = "__moliSvgRectY";
const WIDTH: &str = "__moliSvgRectWidth";
const HEIGHT: &str = "__moliSvgRectHeight";
const FIELDS: &[(&str, &str)] = &[("x", X), ("y", Y), ("width", WIDTH), ("height", HEIGHT)];

#[derive(WebApiObject)]
#[webapi(interface = "SVGRect")]
struct SvgRectObjectDeclaration {
    #[webapi(slot = X)]
    x: f64,
    #[webapi(slot = Y)]
    y: f64,
    #[webapi(slot = WIDTH)]
    width: f64,
    #[webapi(slot = HEIGHT)]
    height: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGRect", enumerable)]
struct SvgRectAccessorsDeclaration {
    #[webapi(accessor_property, getter = get_field, setter = set_field,
        data = callback_data_index_value(scope, 0))]
    x: (),
    #[webapi(accessor_property, getter = get_field, setter = set_field,
        data = callback_data_index_value(scope, 1))]
    y: (),
    #[webapi(accessor_property, getter = get_field, setter = set_field,
        data = callback_data_index_value(scope, 2))]
    width: (),
    #[webapi(accessor_property, getter = get_field, setter = set_field,
        data = callback_data_index_value(scope, 3))]
    height: (),
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SvgRectAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(super) fn create_svg_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let is_svg_root = node_runtime_and_handle_from_object_or_detached(scope, args.this())
        .ok()
        .is_some_and(|(host, handle)| {
            unsafe { &*host }
                .dom_host()
                .node(handle)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.is_svg_element("svg"))
        });
    if !is_svg_root {
        webidl::throw_type_error(
            scope,
            "SVGSVGElement.createSVGRect called on incompatible receiver.",
        );
        return;
    }
    let rect = SvgRectObjectDeclaration::new(0.0, 0.0, 0.0, 0.0)
        .bind(scope)
        .expect("SVGRect declaration should bind");
    rv.set(rect.into());
}

fn field_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(&'static str, &'static str)> {
    if get_private_value(scope, args.this(), X).is_none() {
        webidl::throw_type_error(scope, "SVGRect accessor called on incompatible receiver.");
        return None;
    }
    callback_data_item(scope, args, FIELDS, "SVGRect fields")
}

fn get_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_, slot)) = field_for_receiver(scope, &args) else {
        return;
    };
    if let Some(value) = get_private_value(scope, args.this(), slot) {
        rv.set(value);
    }
}

fn set_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((name, slot)) = field_for_receiver(scope, &args) else {
        return;
    };
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("SVGRect", name),
    ) {
        Ok(value) => value.0 as f32,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    // SVGRect uses restricted WebIDL float: round to binary32, reject both
    // non-finite input and finite doubles which overflow that representation.
    if !value.is_finite() {
        webidl::throw_type_error(scope, "SVGRect value is outside the finite float range.");
        return;
    }
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, f64::from(value)).into(),
    );
}
