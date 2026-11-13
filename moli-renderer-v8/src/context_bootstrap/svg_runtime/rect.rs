//! Native SVGRect values, detached or bound to SVGAnimatedRect viewBox values.
//!
//! JSXGraph uses createSVGRect() to detect SVG support. Returning a DOMRect would
//! pass that check but expose the wrong interface and double (not float) fields.

use crate::web_api_interfaces;
use crate::{
    native_bridge::throw_dom_exception,
    util::{callback_data_index_value, callback_data_item, get_private_value, set_private_value},
    webidl,
};
use moli_webapi::svg_values::{SVG_RECT_FIELD_SLOTS as FIELDS, new_svg_rect};
pub(super) use moli_webapi::svg_values::{set_svg_view_box_rect_values, svg_view_box_rect_values};
use moli_webapi_declare::WebApiFunctionTemplate;

const VIEW_BOX_OWNER: &str = "__moliSvgRectViewBoxOwner";
const READ_ONLY: &str = "__moliSvgRectReadOnly";

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
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(new_svg_rect(scope, [0.0; 4]).into());
}

pub(super) fn build_svg_view_box_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    values: [f64; 4],
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let rect = new_svg_rect(scope, values);
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

fn get_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_, slot)) = callback_data_item(scope, &args, FIELDS, "SVGRect fields") else {
        return;
    };
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated SVGRect receiver");
    super::builders::sync_svg_view_box_rect_from_owner(scope, receiver);
    if let Some(value) = get_private_value(scope, receiver, slot) {
        rv.set(value);
    }
}

fn set_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((name, slot)) = callback_data_item(scope, &args, FIELDS, "SVGRect fields") else {
        return;
    };
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated SVGRect receiver");
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
    // WebIDL conversion runs before the implementation checks mutability.
    if get_private_value(scope, receiver, READ_ONLY).is_some_and(|value| value.boolean_value(scope))
    {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG rectangle is read-only.",
        );
        return;
    }
    // ToNumber may re-enter JS and change the owner's viewBox. Preserve those
    // changes to the other fields before applying this one-field mutation.
    super::builders::sync_svg_view_box_rect_from_owner(scope, receiver);
    set_private_value(
        scope,
        receiver,
        slot,
        v8::Number::new(scope, f64::from(value)).into(),
    );
    super::builders::reflect_svg_view_box_rect_mutation(scope, receiver);
}
