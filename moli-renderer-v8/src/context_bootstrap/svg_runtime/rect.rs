//! Native SVGRect values, detached or bound to SVGAnimatedRect viewBox values.
//!
//! JSXGraph uses createSVGRect() to detect SVG support. Returning a DOMRect would
//! pass that check but expose the wrong interface and double (not float) fields.

use crate::web_api_interfaces;
use crate::{
    native_bridge::{node_runtime_and_handle_from_object_or_detached, throw_dom_exception},
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const X: &str = "__moliSvgRectX";
const Y: &str = "__moliSvgRectY";
const WIDTH: &str = "__moliSvgRectWidth";
const HEIGHT: &str = "__moliSvgRectHeight";
const VIEW_BOX_OWNER: &str = "__moliSvgRectViewBoxOwner";
const READ_ONLY: &str = "__moliSvgRectReadOnly";
const FIELDS: &[(&str, &str)] = &[("x", X), ("y", Y), ("width", WIDTH), ("height", HEIGHT)];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGRect)]
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
#[webapi(interface = web_api_interfaces::SVGRect, enumerable)]
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

pub(super) fn build_svg_view_box_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    values: [f64; 4],
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let [x, y, width, height] = values.map(|value| f64::from(value as f32));
    let rect = SvgRectObjectDeclaration::new(x, y, width, height)
        .bind(scope)
        .expect("SVGRect declaration should bind");
    set_private_value(scope, rect, VIEW_BOX_OWNER, owner.into());
    set_private_value(
        scope,
        rect,
        READ_ONLY,
        v8::Boolean::new(scope, read_only).into(),
    );
    rect
}

pub(super) fn svg_view_box_rect_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, rect, VIEW_BOX_OWNER)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn svg_view_box_rect_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) -> [f64; 4] {
    [X, Y, WIDTH, HEIGHT].map(|slot| {
        get_private_value(scope, rect, slot)
            .and_then(|value| v8::Local::<v8::Number>::try_from(value).ok())
            .map_or(0.0, |value| value.value())
    })
}

pub(super) fn set_svg_view_box_rect_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
    values: [f64; 4],
) {
    for (slot, value) in [X, Y, WIDTH, HEIGHT].into_iter().zip(values) {
        set_private_value(
            scope,
            rect,
            slot,
            v8::Number::new(scope, f64::from(value as f32)).into(),
        );
    }
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
    super::builders::sync_svg_view_box_rect_from_owner(scope, args.this());
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
    if get_private_value(scope, args.this(), READ_ONLY)
        .is_some_and(|value| value.boolean_value(scope))
    {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG rectangle is read-only.",
        );
        return;
    }
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
    // ToNumber may re-enter JS and change the owner's viewBox. Preserve those
    // changes to the other fields before applying this one-field mutation.
    super::builders::sync_svg_view_box_rect_from_owner(scope, args.this());
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, f64::from(value)).into(),
    );
    super::builders::reflect_svg_view_box_rect_mutation(scope, args.this());
}
