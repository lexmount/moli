//! Shared value interfaces. Resource allocation and shader reflection remain
//! owned by the WebGL backend; exposing their types does not create GPU objects.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, v8str};
use moli_webapi_declare::WebApiFunctionTemplate;

pub const PRECISION: &str = "__moliWebGlPrecision";
pub const RANGE_MIN: &str = "__moliWebGlRangeMin";
pub const RANGE_MAX: &str = "__moliWebGlRangeMax";
const ACTIVE_SIZE: &str = "__moliWebGlActiveSize";
const ACTIVE_TYPE: &str = "__moliWebGlActiveType";
const ACTIVE_NAME: &str = "__moliWebGlActiveName";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebGLShaderPrecisionFormat, enumerable, receiver)]
struct PrecisionAttributes {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, PRECISION))]
    precision: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, RANGE_MIN))]
    range_min: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, RANGE_MAX))]
    range_max: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebGLActiveInfo, enumerable, receiver)]
struct ActiveInfoAttributes {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ACTIVE_SIZE))]
    size: (),
    #[webapi(accessor_property = "type", getter = slot_getter, data = v8str(scope, ACTIVE_TYPE))]
    type_: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ACTIVE_NAME))]
    name: (),
}

pub fn install_value_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "WebGLShaderPrecisionFormat" => {
            PrecisionAttributes::initialize_prototype_template(scope, prototype)
        }
        "WebGLActiveInfo" => ActiveInfoAttributes::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}
