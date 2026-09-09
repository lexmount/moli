use super::*;
use crate::native_bridge::throw_dom_exception;
use crate::util::{callback_data_index_value, get_private_value, set_private_value};
use crate::webidl;
use moli_geometry::{
    DOM_MATRIX_COMPONENT_COUNT, DomMatrixComponents, dom_matrix_components_from_values,
    parse_dom_matrix_value,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const DOM_POINT_X_SLOT: &str = "__moliDomPointX";
const DOM_POINT_Y_SLOT: &str = "__moliDomPointY";
const DOM_POINT_Z_SLOT: &str = "__moliDomPointZ";
const DOM_POINT_W_SLOT: &str = "__moliDomPointW";
const DOM_POINT_BRAND_SLOT: &str = "__moliDomPointBrand";

const DOM_MATRIX_M11_SLOT: &str = "__moliDomMatrixM11";
const DOM_MATRIX_M12_SLOT: &str = "__moliDomMatrixM12";
const DOM_MATRIX_M13_SLOT: &str = "__moliDomMatrixM13";
const DOM_MATRIX_M14_SLOT: &str = "__moliDomMatrixM14";
const DOM_MATRIX_M21_SLOT: &str = "__moliDomMatrixM21";
const DOM_MATRIX_M22_SLOT: &str = "__moliDomMatrixM22";
const DOM_MATRIX_M23_SLOT: &str = "__moliDomMatrixM23";
const DOM_MATRIX_M24_SLOT: &str = "__moliDomMatrixM24";
const DOM_MATRIX_M31_SLOT: &str = "__moliDomMatrixM31";
const DOM_MATRIX_M32_SLOT: &str = "__moliDomMatrixM32";
const DOM_MATRIX_M33_SLOT: &str = "__moliDomMatrixM33";
const DOM_MATRIX_M34_SLOT: &str = "__moliDomMatrixM34";
const DOM_MATRIX_M41_SLOT: &str = "__moliDomMatrixM41";
const DOM_MATRIX_M42_SLOT: &str = "__moliDomMatrixM42";
const DOM_MATRIX_M43_SLOT: &str = "__moliDomMatrixM43";
const DOM_MATRIX_M44_SLOT: &str = "__moliDomMatrixM44";
const DOM_MATRIX_READONLY_BRAND_SLOT: &str = "__moliDomMatrixReadOnlyBrand";
const DOM_MATRIX_MUTABLE_BRAND_SLOT: &str = "__moliDomMatrixMutableBrand";
const DOM_MATRIX_TYPED_ARRAY_LENGTH: usize = DOM_MATRIX_COMPONENT_COUNT;

#[derive(WebApiObject)]
#[webapi(interface = "DOMPoint", fallback_to_string_tag = "DOMPoint")]
struct DomPointObjectDeclaration {
    #[webapi(slot = DOM_POINT_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = DOM_POINT_X_SLOT)]
    x: f64,
    #[webapi(slot = DOM_POINT_Y_SLOT)]
    y: f64,
    #[webapi(slot = DOM_POINT_Z_SLOT)]
    z: f64,
    #[webapi(slot = DOM_POINT_W_SLOT)]
    w: f64,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "DOMPointReadOnly",
    fallback_to_string_tag = "DOMPointReadOnly"
)]
struct DomPointReadOnlyObjectDeclaration {
    #[webapi(slot = DOM_POINT_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = DOM_POINT_X_SLOT)]
    x: f64,
    #[webapi(slot = DOM_POINT_Y_SLOT)]
    y: f64,
    #[webapi(slot = DOM_POINT_Z_SLOT)]
    z: f64,
    #[webapi(slot = DOM_POINT_W_SLOT)]
    w: f64,
}

macro_rules! dom_matrix_object_declaration {
    ($name:ident, $interface:literal, mutable) => {
        dom_matrix_object_declaration!(
            @body
            $name,
            $interface,
            {
                #[webapi(slot = DOM_MATRIX_MUTABLE_BRAND_SLOT, init = true)]
                mutable_brand: (),
            },
            {
                mutable_brand: (),
            }
        );
    };
    ($name:ident, $interface:literal, readonly) => {
        dom_matrix_object_declaration!(@body $name, $interface, {}, {});
    };
    (@body $name:ident, $interface:literal, {$($extra_field:tt)*}, {$($extra_init:tt)*}) => {
        #[derive(WebApiObject)]
        #[webapi(interface = $interface, fallback_to_string_tag = $interface)]
        struct $name {
            #[webapi(slot = DOM_MATRIX_READONLY_BRAND_SLOT, init = true)]
            readonly_brand: (),

            $($extra_field)*

            #[webapi(slot = DOM_MATRIX_M11_SLOT)]
            m11: f64,
            #[webapi(slot = DOM_MATRIX_M12_SLOT)]
            m12: f64,
            #[webapi(slot = DOM_MATRIX_M13_SLOT)]
            m13: f64,
            #[webapi(slot = DOM_MATRIX_M14_SLOT)]
            m14: f64,
            #[webapi(slot = DOM_MATRIX_M21_SLOT)]
            m21: f64,
            #[webapi(slot = DOM_MATRIX_M22_SLOT)]
            m22: f64,
            #[webapi(slot = DOM_MATRIX_M23_SLOT)]
            m23: f64,
            #[webapi(slot = DOM_MATRIX_M24_SLOT)]
            m24: f64,
            #[webapi(slot = DOM_MATRIX_M31_SLOT)]
            m31: f64,
            #[webapi(slot = DOM_MATRIX_M32_SLOT)]
            m32: f64,
            #[webapi(slot = DOM_MATRIX_M33_SLOT)]
            m33: f64,
            #[webapi(slot = DOM_MATRIX_M34_SLOT)]
            m34: f64,
            #[webapi(slot = DOM_MATRIX_M41_SLOT)]
            m41: f64,
            #[webapi(slot = DOM_MATRIX_M42_SLOT)]
            m42: f64,
            #[webapi(slot = DOM_MATRIX_M43_SLOT)]
            m43: f64,
            #[webapi(slot = DOM_MATRIX_M44_SLOT)]
            m44: f64,
        }

        impl $name {
            fn from_components(components: DomMatrixComponents) -> Self {
                Self {
                    readonly_brand: (),
                    $($extra_init)*
                    m11: components.m11,
                    m12: components.m12,
                    m13: components.m13,
                    m14: components.m14,
                    m21: components.m21,
                    m22: components.m22,
                    m23: components.m23,
                    m24: components.m24,
                    m31: components.m31,
                    m32: components.m32,
                    m33: components.m33,
                    m34: components.m34,
                    m41: components.m41,
                    m42: components.m42,
                    m43: components.m43,
                    m44: components.m44,
                }
            }

            fn identity() -> Self {
                Self::from_components(DomMatrixComponents::identity())
            }
        }
    };
}

dom_matrix_object_declaration!(DomMatrixObjectDeclaration, "DOMMatrix", mutable);
dom_matrix_object_declaration!(
    DomMatrixReadOnlyObjectDeclaration,
    "DOMMatrixReadOnly",
    readonly
);

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct DomPointJsonDeclaration {
    x: f64,
    y: f64,
    z: f64,
    w: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct DomMatrixJsonDeclaration {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
    m11: f64,
    m12: f64,
    m13: f64,
    m14: f64,
    m21: f64,
    m22: f64,
    m23: f64,
    m24: f64,
    m31: f64,
    m32: f64,
    m33: f64,
    m34: f64,
    m41: f64,
    m42: f64,
    m43: f64,
    m44: f64,
    is_2d: bool,
    is_identity: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMPoint")]
struct DomPointPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_point_getter_callback,
        setter = dom_point_setter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    x: (),
    #[webapi(
        accessor_property,
        getter = dom_point_getter_callback,
        setter = dom_point_setter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    y: (),
    #[webapi(
        accessor_property,
        getter = dom_point_getter_callback,
        setter = dom_point_setter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    z: (),
    #[webapi(
        accessor_property,
        getter = dom_point_getter_callback,
        setter = dom_point_setter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    w: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMPointReadOnly")]
struct DomPointReadOnlyPrototypeMethodsDeclaration {
    #[webapi(method = "toJSON", enumerable, callback = dom_point_to_json_callback)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMPointReadOnly")]
struct DomPointReadOnlyPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = dom_point_getter_callback, data = callback_data_index_value(scope, 0), enumerable)]
    x: (),
    #[webapi(accessor_property, getter = dom_point_getter_callback, data = callback_data_index_value(scope, 1), enumerable)]
    y: (),
    #[webapi(accessor_property, getter = dom_point_getter_callback, data = callback_data_index_value(scope, 2), enumerable)]
    z: (),
    #[webapi(accessor_property, getter = dom_point_getter_callback, data = callback_data_index_value(scope, 3), enumerable)]
    w: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMPoint")]
struct DomPointConstructorDeclaration {
    #[webapi(static_method = "fromPoint", length = 0, callback = dom_point_from_point_callback)]
    from_point: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMPointReadOnly")]
struct DomPointReadOnlyConstructorDeclaration {
    #[webapi(
        static_method = "fromPoint",
        length = 0,
        callback = dom_point_readonly_from_point_callback
    )]
    from_point: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMMatrixReadOnly")]
struct DomMatrixReadOnlyPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 0), enumerable)]
    a: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 1), enumerable)]
    b: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 2), enumerable)]
    c: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 3), enumerable)]
    d: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 4), enumerable)]
    e: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 5), enumerable)]
    f: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 6), enumerable)]
    m11: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 7), enumerable)]
    m12: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 8), enumerable)]
    m13: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 9), enumerable)]
    m14: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 10), enumerable)]
    m21: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 11), enumerable)]
    m22: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 12), enumerable)]
    m23: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 13), enumerable)]
    m24: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 14), enumerable)]
    m31: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 15), enumerable)]
    m32: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 16), enumerable)]
    m33: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 17), enumerable)]
    m34: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 18), enumerable)]
    m41: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 19), enumerable)]
    m42: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 20), enumerable)]
    m43: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 21), enumerable)]
    m44: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 22), enumerable)]
    is_2d: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, data = callback_data_index_value(scope, 23), enumerable)]
    is_identity: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMMatrixReadOnly")]
struct DomMatrixReadOnlyPrototypeMethodsDeclaration {
    #[webapi(method = "toJSON", enumerable, callback = dom_matrix_to_json_callback)]
    to_json: (),

    #[webapi(method = "toString", callback = dom_matrix_to_string_callback)]
    to_string: (),

    #[webapi(method = "toFloat32Array", callback = dom_matrix_to_float32_array_callback)]
    to_float32_array: (),

    #[webapi(method = "toFloat64Array", callback = dom_matrix_to_float64_array_callback)]
    to_float64_array: (),

    #[webapi(method = "translate", callback = dom_matrix_translate_callback)]
    translate: (),

    #[webapi(method = "scale", callback = dom_matrix_scale_callback)]
    scale: (),

    #[webapi(method = "scaleNonUniform", callback = dom_matrix_scale_non_uniform_callback)]
    scale_non_uniform: (),

    #[webapi(method = "scale3d", callback = dom_matrix_scale_3d_callback)]
    scale_3d: (),

    #[webapi(method = "rotate", callback = dom_matrix_rotate_callback)]
    rotate: (),

    #[webapi(method = "rotateFromVector", callback = dom_matrix_rotate_from_vector_callback)]
    rotate_from_vector: (),

    #[webapi(method = "rotateAxisAngle", callback = dom_matrix_rotate_axis_angle_callback)]
    rotate_axis_angle: (),

    #[webapi(method = "skewX", callback = dom_matrix_skew_x_callback)]
    skew_x: (),

    #[webapi(method = "skewY", callback = dom_matrix_skew_y_callback)]
    skew_y: (),

    #[webapi(method = "multiply", callback = dom_matrix_multiply_callback)]
    multiply: (),

    #[webapi(method = "flipX", callback = dom_matrix_flip_x_callback)]
    flip_x: (),

    #[webapi(method = "flipY", callback = dom_matrix_flip_y_callback)]
    flip_y: (),

    #[webapi(method = "inverse", callback = dom_matrix_inverse_callback)]
    inverse: (),

    #[webapi(method = "transformPoint", callback = dom_matrix_transform_point_callback)]
    transform_point: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMMatrixReadOnly")]
struct DomMatrixReadOnlyConstructorDeclaration {
    #[webapi(
        static_method = "fromMatrix",
        length = 0,
        callback = dom_matrix_readonly_from_matrix_callback
    )]
    from_matrix: (),

    #[webapi(
        static_method = "fromFloat32Array",
        length = 1,
        callback = dom_matrix_readonly_from_float32_array_callback
    )]
    from_float32_array: (),

    #[webapi(
        static_method = "fromFloat64Array",
        length = 1,
        callback = dom_matrix_readonly_from_float64_array_callback
    )]
    from_float64_array: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMMatrix")]
struct DomMatrixPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 0), enumerable)]
    a: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 1), enumerable)]
    b: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 2), enumerable)]
    c: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 3), enumerable)]
    d: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 4), enumerable)]
    e: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 5), enumerable)]
    f: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 6), enumerable)]
    m11: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 7), enumerable)]
    m12: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 8), enumerable)]
    m13: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 9), enumerable)]
    m14: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 10), enumerable)]
    m21: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 11), enumerable)]
    m22: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 12), enumerable)]
    m23: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 13), enumerable)]
    m24: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 14), enumerable)]
    m31: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 15), enumerable)]
    m32: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 16), enumerable)]
    m33: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 17), enumerable)]
    m34: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 18), enumerable)]
    m41: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 19), enumerable)]
    m42: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 20), enumerable)]
    m43: (),
    #[webapi(accessor_property, getter = dom_matrix_getter_callback, setter = dom_matrix_setter_callback, data = callback_data_index_value(scope, 21), enumerable)]
    m44: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMMatrix")]
struct DomMatrixPrototypeMethodsDeclaration {
    #[webapi(method = "translateSelf", callback = dom_matrix_translate_self_callback)]
    translate_self: (),

    #[webapi(method = "scaleSelf", callback = dom_matrix_scale_self_callback)]
    scale_self: (),

    #[webapi(method = "scale3dSelf", callback = dom_matrix_scale_3d_self_callback)]
    scale_3d_self: (),

    #[webapi(method = "rotateSelf", callback = dom_matrix_rotate_self_callback)]
    rotate_self: (),

    #[webapi(
        method = "rotateFromVectorSelf",
        callback = dom_matrix_rotate_from_vector_self_callback
    )]
    rotate_from_vector_self: (),

    #[webapi(
        method = "rotateAxisAngleSelf",
        callback = dom_matrix_rotate_axis_angle_self_callback
    )]
    rotate_axis_angle_self: (),

    #[webapi(method = "skewXSelf", callback = dom_matrix_skew_x_self_callback)]
    skew_x_self: (),

    #[webapi(method = "skewYSelf", callback = dom_matrix_skew_y_self_callback)]
    skew_y_self: (),

    #[webapi(method = "multiplySelf", callback = dom_matrix_multiply_self_callback)]
    multiply_self: (),

    #[webapi(method = "preMultiplySelf", callback = dom_matrix_pre_multiply_self_callback)]
    pre_multiply_self: (),

    #[webapi(method = "invertSelf", callback = dom_matrix_invert_self_callback)]
    invert_self: (),

    #[webapi(method = "setMatrixValue", callback = dom_matrix_set_matrix_value_callback)]
    set_matrix_value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMMatrix")]
struct DomMatrixConstructorDeclaration {
    #[webapi(
        static_method = "fromMatrix",
        length = 0,
        callback = dom_matrix_from_matrix_callback
    )]
    from_matrix: (),

    #[webapi(
        static_method = "fromFloat32Array",
        length = 1,
        callback = dom_matrix_from_float32_array_callback
    )]
    from_float32_array: (),

    #[webapi(
        static_method = "fromFloat64Array",
        length = 1,
        callback = dom_matrix_from_float64_array_callback
    )]
    from_float64_array: (),
}

#[derive(Clone, Copy, webidl::WebIdlDictionary)]
#[webidl(prefix = "DOMPointInit")]
pub(super) struct DomPointInit {
    #[webidl(default = 0.0)]
    pub(super) x: f64,
    #[webidl(default = 0.0)]
    pub(super) y: f64,
    #[webidl(default = 0.0)]
    pub(super) z: f64,
    #[webidl(default = 1.0)]
    pub(super) w: f64,
}

impl Default for DomPointInit {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

pub(super) fn dom_point_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMPoint': Please use the 'new' operator.",
        );
        return;
    }
    let Some(init) = dom_point_constructor_init(scope, &args, "DOMPoint") else {
        return;
    };
    initialize_dom_point_object(scope, args.this(), init.x, init.y, init.z, init.w);
    rv.set(args.this().into());
}

pub(super) fn dom_point_readonly_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMPointReadOnly': Please use the 'new' operator.",
        );
        return;
    }
    let Some(init) = dom_point_constructor_init(scope, &args, "DOMPointReadOnly") else {
        return;
    };
    initialize_dom_point_readonly_object(scope, args.this(), init.x, init.y, init.z, init.w);
    rv.set(args.this().into());
}

fn dom_point_constructor_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> Option<DomPointInit> {
    Some(DomPointInit {
        x: geometry_number_arg(scope, args, 0, 0.0, prefix)?,
        y: geometry_number_arg(scope, args, 1, 0.0, prefix)?,
        z: geometry_number_arg(scope, args, 2, 0.0, prefix)?,
        w: geometry_number_arg(scope, args, 3, 1.0, prefix)?,
    })
}

pub(super) fn dom_matrix_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMMatrix': Please use the 'new' operator.",
        );
        return;
    }
    initialize_dom_matrix_identity_object(scope, args.this());
    if args.length() > 0
        && !args.get(0).is_undefined()
        && !apply_dom_matrix_init(scope, args.this(), args.get(0))
    {
        return;
    }
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn initialize_dom_point_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
    z: f64,
    w: f64,
) {
    DomPointObjectDeclaration::new(x, y, z, w)
        .initialize(scope, object)
        .expect("DOMPoint declaration should initialize object");
}

fn initialize_dom_point_readonly_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
    z: f64,
    w: f64,
) {
    DomPointReadOnlyObjectDeclaration::new(x, y, z, w)
        .initialize(scope, object)
        .expect("DOMPointReadOnly declaration should initialize object");
}

pub(in crate::context_bootstrap) fn build_dom_point_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    z: f64,
    w: f64,
) -> v8::Local<'s, v8::Object> {
    DomPointObjectDeclaration::new(x, y, z, w)
        .bind(scope)
        .expect("DOMPoint declaration should bind")
}

fn build_dom_point_readonly_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    z: f64,
    w: f64,
) -> v8::Local<'s, v8::Object> {
    DomPointReadOnlyObjectDeclaration::new(x, y, z, w)
        .bind(scope)
        .expect("DOMPointReadOnly declaration should bind")
}

pub(in crate::context_bootstrap) fn build_dom_matrix_identity_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    DomMatrixObjectDeclaration::identity()
        .bind(scope)
        .expect("DOMMatrix declaration should bind")
}

fn build_dom_matrix_readonly_identity_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    DomMatrixReadOnlyObjectDeclaration::identity()
        .bind(scope)
        .expect("DOMMatrixReadOnly declaration should bind")
}

pub(in crate::context_bootstrap) fn install_geometry_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "DOMPointReadOnly" => {
            DomPointReadOnlyConstructorDeclaration::initialize_template(scope, template);
            DomPointReadOnlyPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            DomPointReadOnlyPrototypeMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "DOMPoint" => {
            DomPointConstructorDeclaration::initialize_template(scope, template);
            DomPointPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "DOMMatrixReadOnly" => {
            DomMatrixReadOnlyConstructorDeclaration::initialize_template(scope, template);
            DomMatrixReadOnlyPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            DomMatrixReadOnlyPrototypeMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "DOMMatrix" => {
            DomMatrixConstructorDeclaration::initialize_template(scope, template);
            DomMatrixPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
            DomMatrixPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

fn dom_point_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, DOM_POINT_ATTRIBUTE_SLOTS, "DOMPoint slots")
    else {
        rv.set_undefined();
        return;
    };
    if !dom_point_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn dom_point_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, DOM_POINT_ATTRIBUTE_SLOTS, "DOMPoint slots")
    else {
        rv.set_undefined();
        return;
    };
    if !dom_point_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(value) = geometry_number_value(
        scope,
        args.get(0),
        webidl::Context::member("DOMPoint", slot),
    ) else {
        return;
    };
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, value).into(),
    );
    rv.set_undefined();
}

fn dom_matrix_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) =
        callback_data_item(scope, &args, DOM_MATRIX_ATTRIBUTES, "DOMMatrix attributes")
    else {
        rv.set_undefined();
        return;
    };
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    match attribute.kind {
        DomMatrixAttributeKind::Number(slot) => {
            let value = dom_matrix_slot(scope, args.this(), slot, attribute.default);
            rv.set(v8::Number::new(scope, value).into());
        }
        DomMatrixAttributeKind::Is2D => rv.set_bool(dom_matrix_is_2d(scope, args.this())),
        DomMatrixAttributeKind::IsIdentity => {
            rv.set_bool(dom_matrix_is_identity(scope, args.this()))
        }
    }
}

fn dom_matrix_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        DOM_MATRIX_MUTABLE_ATTRIBUTES,
        "DOMMatrix mutable attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let DomMatrixAttributeKind::Number(slot) = attribute.kind else {
        rv.set_undefined();
        return;
    };
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(value) = geometry_number_value(
        scope,
        args.get(0),
        webidl::Context::member("DOMMatrix", attribute.name),
    ) else {
        return;
    };
    set_dom_matrix_slot(scope, args.this(), slot, value);
    rv.set_undefined();
}

fn dom_point_from_point_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(init) = optional_dom_point_init_arg(scope, &args, 0, "DOMPoint.fromPoint") else {
        return;
    };
    rv.set(build_dom_point_object(scope, init.x, init.y, init.z, init.w).into());
}

fn dom_point_readonly_from_point_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(init) = optional_dom_point_init_arg(scope, &args, 0, "DOMPointReadOnly.fromPoint")
    else {
        return;
    };
    rv.set(build_dom_point_readonly_object(scope, init.x, init.y, init.z, init.w).into());
}

fn dom_matrix_from_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let matrix = build_dom_matrix_identity_object(scope);
    if args.length() > 0
        && !args.get(0).is_undefined()
        && !apply_dom_matrix_init(scope, matrix, args.get(0))
    {
        return;
    }
    rv.set(matrix.into());
}

fn dom_matrix_readonly_from_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let matrix = build_dom_matrix_readonly_identity_object(scope);
    if args.length() > 0
        && !args.get(0).is_undefined()
        && !apply_dom_matrix_init(scope, matrix, args.get(0))
    {
        return;
    }
    rv.set(matrix.into());
}

fn dom_matrix_from_float32_array_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(components) = dom_matrix_components_from_typed_array(
        scope,
        args.get(0),
        DomMatrixTypedArrayKind::Float32,
    ) else {
        return;
    };
    let matrix = build_dom_matrix_identity_object(scope);
    set_dom_matrix_components(scope, matrix, components);
    rv.set(matrix.into());
}

fn dom_matrix_from_float64_array_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(components) = dom_matrix_components_from_typed_array(
        scope,
        args.get(0),
        DomMatrixTypedArrayKind::Float64,
    ) else {
        return;
    };
    let matrix = build_dom_matrix_identity_object(scope);
    set_dom_matrix_components(scope, matrix, components);
    rv.set(matrix.into());
}

fn dom_matrix_readonly_from_float32_array_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(components) = dom_matrix_components_from_typed_array(
        scope,
        args.get(0),
        DomMatrixTypedArrayKind::Float32,
    ) else {
        return;
    };
    let matrix = build_dom_matrix_readonly_identity_object(scope);
    set_dom_matrix_components(scope, matrix, components);
    rv.set(matrix.into());
}

fn dom_matrix_readonly_from_float64_array_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(components) = dom_matrix_components_from_typed_array(
        scope,
        args.get(0),
        DomMatrixTypedArrayKind::Float64,
    ) else {
        return;
    };
    let matrix = build_dom_matrix_readonly_identity_object(scope);
    set_dom_matrix_components(scope, matrix, components);
    rv.set(matrix.into());
}

fn dom_point_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if !dom_point_receiver_branded(scope, this) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let declaration = DomPointJsonDeclaration {
        x: dom_point_slot(scope, this, DOM_POINT_X_SLOT, 0.0),
        y: dom_point_slot(scope, this, DOM_POINT_Y_SLOT, 0.0),
        z: dom_point_slot(scope, this, DOM_POINT_Z_SLOT, 0.0),
        w: dom_point_slot(scope, this, DOM_POINT_W_SLOT, 1.0),
    };
    let Ok(object) = declaration.bind(scope) else {
        return;
    };
    rv.set(object.into());
}

fn dom_point_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    default: f64,
) -> f64 {
    get_private_value(scope, object, slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(default)
}

fn dom_point_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, DOM_POINT_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn dom_matrix_require_readonly_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if dom_matrix_receiver_branded(scope, receiver, DOM_MATRIX_READONLY_BRAND_SLOT) {
        return true;
    }
    throw_type_error(scope, "Illegal invocation");
    false
}

fn dom_matrix_require_mutable_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if dom_matrix_receiver_branded(scope, receiver, DOM_MATRIX_MUTABLE_BRAND_SLOT) {
        return true;
    }
    throw_type_error(scope, "Illegal invocation");
    false
}

fn dom_matrix_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    brand: &'static str,
) -> bool {
    get_private_value(scope, receiver, brand).is_some_and(|value| value.boolean_value(scope))
}

fn dom_matrix_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if !dom_matrix_require_readonly_receiver(scope, this) {
        return;
    }
    let declaration = DomMatrixJsonDeclaration {
        a: dom_matrix_json_number(scope, this, DOM_MATRIX_M11_SLOT, 1.0),
        b: dom_matrix_json_number(scope, this, DOM_MATRIX_M12_SLOT, 0.0),
        c: dom_matrix_json_number(scope, this, DOM_MATRIX_M21_SLOT, 0.0),
        d: dom_matrix_json_number(scope, this, DOM_MATRIX_M22_SLOT, 1.0),
        e: dom_matrix_json_number(scope, this, DOM_MATRIX_M41_SLOT, 0.0),
        f: dom_matrix_json_number(scope, this, DOM_MATRIX_M42_SLOT, 0.0),
        m11: dom_matrix_json_number(scope, this, DOM_MATRIX_M11_SLOT, 1.0),
        m12: dom_matrix_json_number(scope, this, DOM_MATRIX_M12_SLOT, 0.0),
        m13: dom_matrix_json_number(scope, this, DOM_MATRIX_M13_SLOT, 0.0),
        m14: dom_matrix_json_number(scope, this, DOM_MATRIX_M14_SLOT, 0.0),
        m21: dom_matrix_json_number(scope, this, DOM_MATRIX_M21_SLOT, 0.0),
        m22: dom_matrix_json_number(scope, this, DOM_MATRIX_M22_SLOT, 1.0),
        m23: dom_matrix_json_number(scope, this, DOM_MATRIX_M23_SLOT, 0.0),
        m24: dom_matrix_json_number(scope, this, DOM_MATRIX_M24_SLOT, 0.0),
        m31: dom_matrix_json_number(scope, this, DOM_MATRIX_M31_SLOT, 0.0),
        m32: dom_matrix_json_number(scope, this, DOM_MATRIX_M32_SLOT, 0.0),
        m33: dom_matrix_json_number(scope, this, DOM_MATRIX_M33_SLOT, 1.0),
        m34: dom_matrix_json_number(scope, this, DOM_MATRIX_M34_SLOT, 0.0),
        m41: dom_matrix_json_number(scope, this, DOM_MATRIX_M41_SLOT, 0.0),
        m42: dom_matrix_json_number(scope, this, DOM_MATRIX_M42_SLOT, 0.0),
        m43: dom_matrix_json_number(scope, this, DOM_MATRIX_M43_SLOT, 0.0),
        m44: dom_matrix_json_number(scope, this, DOM_MATRIX_M44_SLOT, 1.0),
        is_2d: dom_matrix_is_2d(scope, this),
        is_identity: dom_matrix_is_identity(scope, this),
    };
    let Ok(object) = declaration.bind(scope) else {
        return;
    };
    rv.set(object.into());
}

fn dom_matrix_json_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    default: f64,
) -> f64 {
    dom_matrix_slot(scope, object, slot, default)
}

fn dom_matrix_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let Some(text) = dom_matrix_css_text(scope, args.this()) else {
        return;
    };
    let Some(text) = v8_string(scope, &text) else {
        return;
    };
    rv.set(text.into());
}

fn dom_matrix_to_float32_array_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let components = dom_matrix_components(scope, args.this());
    let array = build_dom_matrix_float32_array(scope, components);
    rv.set(array);
}

fn dom_matrix_to_float64_array_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let components = dom_matrix_components(scope, args.this());
    let array = build_dom_matrix_float64_array(scope, components);
    rv.set(array);
}

fn dom_matrix_translate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(translation) = dom_matrix_translate_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_translate(scope, matrix, translation);
    rv.set(matrix.into());
}

fn dom_matrix_translate_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(translation) = dom_matrix_translate_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_translate(scope, args.this(), translation);
    rv.set(args.this().into());
}

fn dom_matrix_scale_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(scale) = dom_matrix_scale_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_scale(scope, matrix, scale);
    rv.set(matrix.into());
}

fn dom_matrix_scale_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(scale) = dom_matrix_scale_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_scale(scope, args.this(), scale);
    rv.set(args.this().into());
}

fn dom_matrix_scale_non_uniform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(scale_x) = geometry_number_arg(scope, &args, 0, 1.0, "DOMMatrix") else {
        return;
    };
    let Some(scale_y) = geometry_number_arg(scope, &args, 1, 1.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_scale_2d(scope, matrix, scale_x, scale_y);
    rv.set(matrix.into());
}

fn dom_matrix_scale_3d_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some((scale, origin_x, origin_y, origin_z)) = dom_matrix_scale_3d_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_scale(
        scope,
        matrix,
        (scale, scale, scale, origin_x, origin_y, origin_z),
    );
    rv.set(matrix.into());
}

fn dom_matrix_scale_3d_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some((scale, origin_x, origin_y, origin_z)) = dom_matrix_scale_3d_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_scale(
        scope,
        args.this(),
        (scale, scale, scale, origin_x, origin_y, origin_z),
    );
    rv.set(args.this().into());
}

fn dom_matrix_rotate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(rotation) = dom_matrix_rotation_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_rotate(scope, matrix, rotation);
    rv.set(matrix.into());
}

fn dom_matrix_rotate_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(rotation) = dom_matrix_rotation_args(scope, &args) else {
        return;
    };
    apply_dom_matrix_rotate(scope, args.this(), rotation);
    rv.set(args.this().into());
}

fn dom_matrix_rotate_from_vector_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let Some(x) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(y) = geometry_number_arg(scope, &args, 1, 0.0, "DOMMatrix") else {
        return;
    };
    let degrees = if x == 0.0 && y == 0.0 {
        0.0
    } else {
        y.atan2(x).to_degrees()
    };
    let matrix = copied_dom_matrix(scope, args.this());
    apply_dom_matrix_rotate_z(scope, matrix, degrees);
    rv.set(matrix.into());
}

fn dom_matrix_rotate_from_vector_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(x) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(y) = geometry_number_arg(scope, &args, 1, 0.0, "DOMMatrix") else {
        return;
    };
    let degrees = if x == 0.0 && y == 0.0 {
        0.0
    } else {
        y.atan2(x).to_degrees()
    };
    apply_dom_matrix_rotate_z(scope, args.this(), degrees);
    rv.set(args.this().into());
}

fn dom_matrix_rotate_axis_angle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(x) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(y) = geometry_number_arg(scope, &args, 1, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(z) = geometry_number_arg(scope, &args, 2, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(degrees) = geometry_number_arg(scope, &args, 3, 0.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_rotate_axis_angle(scope, matrix, x, y, z, degrees);
    rv.set(matrix.into());
}

fn dom_matrix_rotate_axis_angle_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(x) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(y) = geometry_number_arg(scope, &args, 1, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(z) = geometry_number_arg(scope, &args, 2, 0.0, "DOMMatrix") else {
        return;
    };
    let Some(degrees) = geometry_number_arg(scope, &args, 3, 0.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_rotate_axis_angle(scope, args.this(), x, y, z, degrees);
    rv.set(args.this().into());
}

fn dom_matrix_skew_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(degrees) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_skew_x(scope, matrix, degrees);
    rv.set(matrix.into());
}

fn dom_matrix_skew_x_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(degrees) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_skew_x(scope, args.this(), degrees);
    rv.set(args.this().into());
}

fn dom_matrix_skew_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(degrees) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_skew_y(scope, matrix, degrees);
    rv.set(matrix.into());
}

fn dom_matrix_skew_y_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(degrees) = geometry_number_arg(scope, &args, 0, 0.0, "DOMMatrix") else {
        return;
    };
    apply_dom_matrix_skew_y(scope, args.this(), degrees);
    rv.set(args.this().into());
}

fn dom_matrix_multiply_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let Some(other) = dom_matrix_argument_components(scope, &args, 0) else {
        return;
    };
    apply_dom_matrix_multiply(scope, matrix, other);
    rv.set(matrix.into());
}

fn dom_matrix_multiply_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(other) = dom_matrix_argument_components(scope, &args, 0) else {
        return;
    };
    apply_dom_matrix_multiply(scope, args.this(), other);
    rv.set(args.this().into());
}

fn dom_matrix_pre_multiply_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(other) = dom_matrix_argument_components(scope, &args, 0) else {
        return;
    };
    let current = dom_matrix_components(scope, args.this());
    set_dom_matrix_components(scope, args.this(), other.multiply(current));
    rv.set(args.this().into());
}

fn dom_matrix_flip_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    apply_dom_matrix_scale_2d(scope, matrix, -1.0, 1.0);
    rv.set(matrix.into());
}

fn dom_matrix_flip_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    apply_dom_matrix_scale_2d(scope, matrix, 1.0, -1.0);
    rv.set(matrix.into());
}

fn dom_matrix_inverse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let matrix = copied_dom_matrix(scope, args.this());
    let inverted = dom_matrix_components(scope, matrix).inverse();
    set_dom_matrix_components(scope, matrix, inverted);
    rv.set(matrix.into());
}

fn dom_matrix_invert_self_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let inverted = dom_matrix_components(scope, args.this()).inverse();
    set_dom_matrix_components(scope, args.this(), inverted);
    rv.set(args.this().into());
}

fn dom_matrix_set_matrix_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_mutable_receiver(scope, args.this()) {
        return;
    }
    let Some(text) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("DOMMatrix.setMatrixValue", 1),
        "Failed to execute 'setMatrixValue' on 'DOMMatrix': 1 argument required.",
    ) else {
        return;
    };
    let text = String::from(text);
    if !apply_dom_matrix_transform_list_string(scope, args.this(), &text) {
        return;
    }
    rv.set(args.this().into());
}

fn dom_matrix_transform_point_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !dom_matrix_require_readonly_receiver(scope, args.this()) {
        return;
    }
    let Some(init) = optional_dom_point_init_arg(scope, &args, 0, "DOMMatrix.transformPoint")
    else {
        return;
    };
    let components = dom_matrix_components(scope, args.this());
    let point = build_dom_point_object(
        scope,
        components.m11 * init.x
            + components.m21 * init.y
            + components.m31 * init.z
            + components.m41 * init.w,
        components.m12 * init.x
            + components.m22 * init.y
            + components.m32 * init.z
            + components.m42 * init.w,
        components.m13 * init.x
            + components.m23 * init.y
            + components.m33 * init.z
            + components.m43 * init.w,
        components.m14 * init.x
            + components.m24 * init.y
            + components.m34 * init.z
            + components.m44 * init.w,
    );
    rv.set(point.into());
}

fn copied_dom_matrix<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let matrix = build_dom_matrix_identity_object(scope);
    let components = dom_matrix_components(scope, source);
    set_dom_matrix_components(scope, matrix, components);
    matrix
}

fn dom_matrix_argument_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Option<DomMatrixComponents> {
    if index >= args.length() || args.get(index).is_undefined() {
        return Some(DomMatrixComponents::identity());
    }
    let matrix = build_dom_matrix_identity_object(scope);
    if !apply_dom_matrix_init(scope, matrix, args.get(index)) {
        return None;
    }
    Some(dom_matrix_components(scope, matrix))
}

fn dom_matrix_translate_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(f64, f64, f64)> {
    Some((
        geometry_number_arg(scope, args, 0, 0.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 1, 0.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 2, 0.0, "DOMMatrix")?,
    ))
}

fn dom_matrix_scale_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(f64, f64, f64, f64, f64, f64)> {
    let scale_x = geometry_number_arg(scope, args, 0, 1.0, "DOMMatrix")?;
    let scale_y = geometry_number_arg(scope, args, 1, scale_x, "DOMMatrix")?;
    let scale_z = geometry_number_arg(scope, args, 2, 1.0, "DOMMatrix")?;
    let origin_x = geometry_number_arg(scope, args, 3, 0.0, "DOMMatrix")?;
    let origin_y = geometry_number_arg(scope, args, 4, 0.0, "DOMMatrix")?;
    let origin_z = geometry_number_arg(scope, args, 5, 0.0, "DOMMatrix")?;
    Some((scale_x, scale_y, scale_z, origin_x, origin_y, origin_z))
}

fn dom_matrix_scale_3d_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(f64, f64, f64, f64)> {
    Some((
        geometry_number_arg(scope, args, 0, 1.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 1, 0.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 2, 0.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 3, 0.0, "DOMMatrix")?,
    ))
}

fn dom_matrix_rotation_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(f64, f64, f64)> {
    if args.length() == 1 || args.get(1).is_undefined() {
        return Some((
            0.0,
            0.0,
            geometry_number_arg(scope, args, 0, 0.0, "DOMMatrix")?,
        ));
    }
    Some((
        geometry_number_arg(scope, args, 0, 0.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 1, 0.0, "DOMMatrix")?,
        geometry_number_arg(scope, args, 2, 0.0, "DOMMatrix")?,
    ))
}

fn apply_dom_matrix_translate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    (tx, ty, tz): (f64, f64, f64),
) {
    let components = dom_matrix_components(scope, matrix).translated(tx, ty, tz);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_scale<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    (scale_x, scale_y, scale_z, origin_x, origin_y, origin_z): (f64, f64, f64, f64, f64, f64),
) {
    let components = dom_matrix_components(scope, matrix)
        .scaled_with_origin(scale_x, scale_y, scale_z, origin_x, origin_y, origin_z);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_scale_2d<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    scale_x: f64,
    scale_y: f64,
) {
    let components = dom_matrix_components(scope, matrix).scaled_2d(scale_x, scale_y);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_rotate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    (rot_x, rot_y, rot_z): (f64, f64, f64),
) {
    let components = dom_matrix_components(scope, matrix).rotated(rot_x, rot_y, rot_z);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_rotate_z<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    degrees: f64,
) {
    let components = dom_matrix_components(scope, matrix).rotated_z(degrees);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_rotate_axis_angle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
    z: f64,
    degrees: f64,
) {
    let components = dom_matrix_components(scope, matrix).rotated_axis_angle(x, y, z, degrees);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_skew_x<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    degrees: f64,
) {
    let components = dom_matrix_components(scope, matrix).skewed_x(degrees);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_skew_y<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    degrees: f64,
) {
    let components = dom_matrix_components(scope, matrix).skewed_y(degrees);
    set_dom_matrix_components(scope, matrix, components);
}

fn apply_dom_matrix_multiply<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    other: DomMatrixComponents,
) {
    let current = dom_matrix_components(scope, matrix);
    set_dom_matrix_components(scope, matrix, current.multiply(other));
}

#[derive(Clone, Copy)]
enum DomMatrixTypedArrayKind {
    Float32,
    Float64,
}

fn dom_matrix_components_from_typed_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: DomMatrixTypedArrayKind,
) -> Option<DomMatrixComponents> {
    let expected = match kind {
        DomMatrixTypedArrayKind::Float32 => v8::Local::<v8::Float32Array>::try_from(value).is_ok(),
        DomMatrixTypedArrayKind::Float64 => v8::Local::<v8::Float64Array>::try_from(value).is_ok(),
    };
    if !expected {
        throw_type_error(
            scope,
            "DOMMatrix typed array input must match the requested array type.",
        );
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let Some(length) = object
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.number_value(scope))
        .map(|value| value as usize)
    else {
        throw_type_error(scope, "DOMMatrix typed array input is missing length.");
        return None;
    };
    if length != 6 && length != DOM_MATRIX_TYPED_ARRAY_LENGTH {
        throw_type_error(scope, "DOMMatrix typed array length must be 6 or 16.");
        return None;
    }

    let mut values = Vec::with_capacity(length);
    for index in 0..length {
        let Some(value) = object
            .get_index(scope, index as u32)
            .and_then(|value| value.number_value(scope))
        else {
            throw_type_error(
                scope,
                "DOMMatrix typed array input contains a non-number value.",
            );
            return None;
        };
        values.push(value);
    }
    dom_matrix_components_from_values(&values)
}

fn dom_matrix_components_from_sequence_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
) -> Option<DomMatrixComponents> {
    let length = array.length();
    if length != 6 && length != DOM_MATRIX_TYPED_ARRAY_LENGTH as u32 {
        throw_type_error(scope, "DOMMatrix sequence length must be 6 or 16.");
        return None;
    }

    let mut values = Vec::with_capacity(length as usize);
    for index in 0..length {
        let value = array
            .get_index(scope, index)
            .unwrap_or_else(|| v8::undefined(scope).into());
        let value =
            geometry_number_value(scope, value, webidl::Context::member("DOMMatrix", "init"))?;
        values.push(value);
    }

    dom_matrix_components_from_values(&values)
}

fn dom_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> DomMatrixComponents {
    DomMatrixComponents {
        m11: dom_matrix_slot(scope, object, DOM_MATRIX_M11_SLOT, 1.0),
        m12: dom_matrix_slot(scope, object, DOM_MATRIX_M12_SLOT, 0.0),
        m13: dom_matrix_slot(scope, object, DOM_MATRIX_M13_SLOT, 0.0),
        m14: dom_matrix_slot(scope, object, DOM_MATRIX_M14_SLOT, 0.0),
        m21: dom_matrix_slot(scope, object, DOM_MATRIX_M21_SLOT, 0.0),
        m22: dom_matrix_slot(scope, object, DOM_MATRIX_M22_SLOT, 1.0),
        m23: dom_matrix_slot(scope, object, DOM_MATRIX_M23_SLOT, 0.0),
        m24: dom_matrix_slot(scope, object, DOM_MATRIX_M24_SLOT, 0.0),
        m31: dom_matrix_slot(scope, object, DOM_MATRIX_M31_SLOT, 0.0),
        m32: dom_matrix_slot(scope, object, DOM_MATRIX_M32_SLOT, 0.0),
        m33: dom_matrix_slot(scope, object, DOM_MATRIX_M33_SLOT, 1.0),
        m34: dom_matrix_slot(scope, object, DOM_MATRIX_M34_SLOT, 0.0),
        m41: dom_matrix_slot(scope, object, DOM_MATRIX_M41_SLOT, 0.0),
        m42: dom_matrix_slot(scope, object, DOM_MATRIX_M42_SLOT, 0.0),
        m43: dom_matrix_slot(scope, object, DOM_MATRIX_M43_SLOT, 0.0),
        m44: dom_matrix_slot(scope, object, DOM_MATRIX_M44_SLOT, 1.0),
    }
}

fn dom_matrix_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    default: f64,
) -> f64 {
    get_private_value(scope, object, slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(default)
}

fn set_dom_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    components: DomMatrixComponents,
) {
    for (slot, value) in [
        (DOM_MATRIX_M11_SLOT, components.m11),
        (DOM_MATRIX_M12_SLOT, components.m12),
        (DOM_MATRIX_M13_SLOT, components.m13),
        (DOM_MATRIX_M14_SLOT, components.m14),
        (DOM_MATRIX_M21_SLOT, components.m21),
        (DOM_MATRIX_M22_SLOT, components.m22),
        (DOM_MATRIX_M23_SLOT, components.m23),
        (DOM_MATRIX_M24_SLOT, components.m24),
        (DOM_MATRIX_M31_SLOT, components.m31),
        (DOM_MATRIX_M32_SLOT, components.m32),
        (DOM_MATRIX_M33_SLOT, components.m33),
        (DOM_MATRIX_M34_SLOT, components.m34),
        (DOM_MATRIX_M41_SLOT, components.m41),
        (DOM_MATRIX_M42_SLOT, components.m42),
        (DOM_MATRIX_M43_SLOT, components.m43),
        (DOM_MATRIX_M44_SLOT, components.m44),
    ] {
        set_dom_matrix_slot(scope, object, slot, value);
    }
}

fn set_dom_matrix_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    set_private_value(scope, object, slot, v8::Number::new(scope, value).into());
}

fn dom_matrix_array_values(
    components: DomMatrixComponents,
) -> [f64; DOM_MATRIX_TYPED_ARRAY_LENGTH] {
    [
        components.m11,
        components.m12,
        components.m13,
        components.m14,
        components.m21,
        components.m22,
        components.m23,
        components.m24,
        components.m31,
        components.m32,
        components.m33,
        components.m34,
        components.m41,
        components.m42,
        components.m43,
        components.m44,
    ]
}

fn build_dom_matrix_float32_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    components: DomMatrixComponents,
) -> v8::Local<'s, v8::Value> {
    let mut bytes = Vec::with_capacity(DOM_MATRIX_TYPED_ARRAY_LENGTH * std::mem::size_of::<f32>());
    for value in dom_matrix_array_values(components) {
        bytes.extend_from_slice(&(value as f32).to_ne_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Float32Array::new(scope, buffer, 0, DOM_MATRIX_TYPED_ARRAY_LENGTH)
        .expect("Float32Array construction should succeed")
        .into()
}

fn build_dom_matrix_float64_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    components: DomMatrixComponents,
) -> v8::Local<'s, v8::Value> {
    let mut bytes = Vec::with_capacity(DOM_MATRIX_TYPED_ARRAY_LENGTH * std::mem::size_of::<f64>());
    for value in dom_matrix_array_values(components) {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    v8::Float64Array::new(scope, buffer, 0, DOM_MATRIX_TYPED_ARRAY_LENGTH)
        .expect("Float64Array construction should succeed")
        .into()
}

fn apply_dom_matrix_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    init: v8::Local<'s, v8::Value>,
) -> bool {
    if init.is_string() {
        let Some(text) = init.to_string(scope) else {
            return false;
        };
        return apply_dom_matrix_transform_list_string(
            scope,
            matrix,
            &text.to_rust_string_lossy(scope),
        );
    }
    if v8::Local::<v8::Float32Array>::try_from(init).is_ok() {
        if let Some(components) =
            dom_matrix_components_from_typed_array(scope, init, DomMatrixTypedArrayKind::Float32)
        {
            set_dom_matrix_components(scope, matrix, components);
        }
        return true;
    }
    if v8::Local::<v8::Float64Array>::try_from(init).is_ok() {
        if let Some(components) =
            dom_matrix_components_from_typed_array(scope, init, DomMatrixTypedArrayKind::Float64)
        {
            set_dom_matrix_components(scope, matrix, components);
        }
        return true;
    }
    if let Ok(array) = v8::Local::<v8::Array>::try_from(init) {
        let Some(components) = dom_matrix_components_from_sequence_array(scope, array) else {
            return false;
        };
        set_dom_matrix_components(scope, matrix, components);
        return true;
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(init) {
        for attribute in DOM_MATRIX_MUTABLE_ATTRIBUTES {
            if let DomMatrixAttributeKind::Number(slot) = attribute.kind {
                match property_number(scope, object, attribute.name, "DOMMatrixInit") {
                    Some(Some(value)) => {
                        set_dom_matrix_slot(scope, matrix, slot, value);
                    }
                    Some(None) => {}
                    None => return false,
                }
            }
        }
        return true;
    }

    match webidl::convert::<webidl::DomString>(
        scope,
        init,
        webidl::Context::argument("DOMMatrix", 1),
    ) {
        Ok(text) => {
            let text = String::from(text);
            apply_dom_matrix_transform_list_string(scope, matrix, &text)
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            false
        }
    }
}

fn apply_dom_matrix_transform_list_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    text: &str,
) -> bool {
    let Some(components) = parse_dom_matrix_value(text) else {
        throw_dom_exception(
            scope,
            "SyntaxError",
            12,
            "Failed to parse DOMMatrix transform list.",
        );
        return false;
    };
    set_dom_matrix_components(scope, matrix, components);
    true
}

fn initialize_dom_matrix_identity_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    DomMatrixObjectDeclaration::identity()
        .initialize(scope, object)
        .expect("DOMMatrix declaration should initialize object");
}

fn dom_matrix_is_2d<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    dom_matrix_components(scope, object).is_2d()
}

fn dom_matrix_is_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    dom_matrix_components(scope, object).is_identity()
}

fn dom_matrix_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let Some(css_text) = dom_matrix_components(scope, object).css_text() else {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "DOMMatrix contains non-finite values.",
        );
        return None;
    };
    Some(css_text)
}

fn geometry_number_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    default: f64,
    prefix: &'static str,
) -> Option<f64> {
    if index >= args.length() || args.get(index).is_undefined() {
        return Some(default);
    }
    geometry_number_value(
        scope,
        args.get(index),
        webidl::Context::argument(prefix, (index + 1) as usize),
    )
}

pub(super) fn optional_dom_point_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Option<DomPointInit> {
    if args.length() <= index || args.get(index).is_undefined() {
        return Some(DomPointInit::default());
    }
    dom_point_init_value(
        scope,
        args.get(index),
        webidl::Context::argument(prefix, (index + 1) as usize),
    )
}

pub(super) fn dom_point_init_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Option<DomPointInit> {
    match webidl::parse_dictionary::<DomPointInit>(scope, value, context) {
        Ok(Some(init)) => Some(init),
        Ok(None) => Some(DomPointInit::default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn property_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    prefix: &'static str,
) -> Option<Option<f64>> {
    let key = v8_string(scope, name)?;
    let value = object.get(scope, key.into())?;
    if value.is_undefined() {
        return Some(None);
    }
    geometry_number_value(scope, value, webidl::Context::member(prefix, name)).map(Some)
}

fn geometry_number_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Option<f64> {
    match webidl::convert::<webidl::UnrestrictedDouble>(scope, value, context) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

#[derive(Clone, Copy)]
enum DomMatrixAttributeKind {
    Number(&'static str),
    Is2D,
    IsIdentity,
}

#[derive(Clone, Copy)]
struct DomMatrixAttribute {
    name: &'static str,
    kind: DomMatrixAttributeKind,
    default: f64,
}

const DOM_POINT_ATTRIBUTE_SLOTS: &[&str] = &[
    DOM_POINT_X_SLOT,
    DOM_POINT_Y_SLOT,
    DOM_POINT_Z_SLOT,
    DOM_POINT_W_SLOT,
];

const DOM_MATRIX_MUTABLE_ATTRIBUTES: &[DomMatrixAttribute] = &[
    DomMatrixAttribute {
        name: "a",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M11_SLOT),
        default: 1.0,
    },
    DomMatrixAttribute {
        name: "b",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M12_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "c",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M21_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "d",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M22_SLOT),
        default: 1.0,
    },
    DomMatrixAttribute {
        name: "e",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M41_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "f",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M42_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m11",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M11_SLOT),
        default: 1.0,
    },
    DomMatrixAttribute {
        name: "m12",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M12_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m13",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M13_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m14",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M14_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m21",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M21_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m22",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M22_SLOT),
        default: 1.0,
    },
    DomMatrixAttribute {
        name: "m23",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M23_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m24",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M24_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m31",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M31_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m32",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M32_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m33",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M33_SLOT),
        default: 1.0,
    },
    DomMatrixAttribute {
        name: "m34",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M34_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m41",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M41_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m42",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M42_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m43",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M43_SLOT),
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "m44",
        kind: DomMatrixAttributeKind::Number(DOM_MATRIX_M44_SLOT),
        default: 1.0,
    },
];

const DOM_MATRIX_READONLY_ATTRIBUTES: &[DomMatrixAttribute] = &[
    DomMatrixAttribute {
        name: "is2D",
        kind: DomMatrixAttributeKind::Is2D,
        default: 0.0,
    },
    DomMatrixAttribute {
        name: "isIdentity",
        kind: DomMatrixAttributeKind::IsIdentity,
        default: 0.0,
    },
];

const DOM_MATRIX_ATTRIBUTES: &[DomMatrixAttribute] = &[
    DOM_MATRIX_MUTABLE_ATTRIBUTES[0],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[1],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[2],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[3],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[4],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[5],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[6],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[7],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[8],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[9],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[10],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[11],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[12],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[13],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[14],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[15],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[16],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[17],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[18],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[19],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[20],
    DOM_MATRIX_MUTABLE_ATTRIBUTES[21],
    DOM_MATRIX_READONLY_ATTRIBUTES[0],
    DOM_MATRIX_READONLY_ATTRIBUTES[1],
];
