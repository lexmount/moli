use crate::{
    css_style::top_level_comma_separated_component_values,
    detached_css_style::css_style_declaration_exposes_property_name,
    native_bridge,
    util::{get_private_object, get_private_value, set_private_value, throw_type_error, v8_string},
    webidl, window_host,
};
use cssparser::{Parser, ParserInput, Token};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod factories;
mod map;
mod math;
mod parse;
mod transforms;
mod unparsed;
mod values;

pub(in crate::context_bootstrap) use transforms::{
    css_matrix_component_constructor_callback, css_perspective_constructor_callback,
    css_rotate_constructor_callback, css_scale_constructor_callback, css_skew_constructor_callback,
    css_skew_x_constructor_callback, css_skew_y_constructor_callback,
    css_transform_value_constructor_callback, css_translate_constructor_callback,
};

pub(super) use factories::install as install_numeric_factories;

pub(in crate::context_bootstrap) use unparsed::{
    css_unparsed_value_constructor_callback, css_variable_reference_value_constructor_callback,
};

pub(in crate::context_bootstrap) use values::{
    css_keyword_value_constructor_callback, css_unit_value_constructor_callback,
};

pub(in crate::context_bootstrap) use math::{
    css_math_clamp_constructor_callback, css_math_invert_constructor_callback,
    css_math_max_constructor_callback, css_math_min_constructor_callback,
    css_math_negate_constructor_callback, css_math_product_constructor_callback,
    css_math_sum_constructor_callback,
};

pub(in crate::context_bootstrap) fn install_css_typed_om_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
    profile: crate::context_bootstrap::exposed_interfaces::TemplateBuildProfile,
) {
    parse::install_template_bindings(scope, template, interface_name, profile);
    unparsed::install_template_bindings(scope, template, interface_name);
    map::install_computed_style_map_template_bindings(scope, template, interface_name);
    values::install_typed_value_template_bindings(scope, template, interface_name);
    math::install(scope, template, interface_name, profile);
    transforms::install(scope, template, interface_name);
}
