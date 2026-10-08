//! Native standalone angle values returned by SVGSVGElement.createSVGAngle.
use super::*;
use crate::util::callback_data_index_value;
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const SVG_ANGLE_ACCESSOR_NAMES: &[&str] = &[
    "unitType",
    "value",
    "valueInSpecifiedUnits",
    "valueAsString",
];
const SVG_ANGLE_UNIT_TYPE_SLOT: &str = "__moliSvgAngleUnitType";
const SVG_ANGLE_VALUE_SLOT: &str = "__moliSvgAngleValue";
const SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT: &str = "__moliSvgAngleValueInSpecifiedUnits";
const SVG_ANGLE_VALUE_AS_STRING_SLOT: &str = "__moliSvgAngleValueAsString";
const SVG_ANGLE_READ_ONLY_SLOT: &str = "__moliSvgAngleReadOnly";
const SVG_ANGLE_TYPE_UNKNOWN: u32 = 0;
const SVG_ANGLE_TYPE_UNSPECIFIED: u32 = 1;
const SVG_ANGLE_TYPE_DEG: u32 = 2;
const SVG_ANGLE_TYPE_RAD: u32 = 3;
const SVG_ANGLE_TYPE_GRAD: u32 = 4;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGAngle, fallback_to_string_tag = "SVGAngle")]
struct SvgAngleObjectDeclaration {
    #[webapi(slot = SVG_ANGLE_UNIT_TYPE_SLOT)]
    unit_type: u32,
    #[webapi(slot = SVG_ANGLE_VALUE_SLOT)]
    value: f64,
    #[webapi(slot = SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT)]
    value_in_specified_units: f64,
    #[webapi(slot = SVG_ANGLE_VALUE_AS_STRING_SLOT)]
    value_as_string: String,
    #[webapi(slot = SVG_ANGLE_READ_ONLY_SLOT)]
    read_only: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAngle, enumerable, receiver)]
struct SvgAngleTemplateMethodsDeclaration {
    #[webapi(constant = "SVG_ANGLETYPE_UNKNOWN", value = SVG_ANGLE_TYPE_UNKNOWN)]
    angle_type_unknown: (),

    #[webapi(
        constant = "SVG_ANGLETYPE_UNSPECIFIED",
        value = SVG_ANGLE_TYPE_UNSPECIFIED
    )]
    angle_type_unspecified: (),

    #[webapi(constant = "SVG_ANGLETYPE_DEG", value = SVG_ANGLE_TYPE_DEG)]
    angle_type_deg: (),

    #[webapi(constant = "SVG_ANGLETYPE_RAD", value = SVG_ANGLE_TYPE_RAD)]
    angle_type_rad: (),

    #[webapi(constant = "SVG_ANGLETYPE_GRAD", value = SVG_ANGLE_TYPE_GRAD)]
    angle_type_grad: (),

    #[webapi(
        method = "newValueSpecifiedUnits",
        length = 2,
        callback = svg_angle_new_value_specified_units_callback
    )]
    new_value_specified_units: (),

    #[webapi(
        method = "convertToSpecifiedUnits",
        length = 1,
        callback = svg_angle_convert_to_specified_units_callback
    )]
    convert_to_specified_units: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAngle, enumerable, receiver)]
struct SvgAngleTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "unitType",
        getter = svg_angle_getter,
        data = callback_data_index_value(scope, 0)
    )]
    unit_type: (),

    #[webapi(
        accessor_property = "value",
        getter = svg_angle_getter,
        setter = svg_angle_setter,
        data = callback_data_index_value(scope, 1)
    )]
    value: (),

    #[webapi(
        accessor_property = "valueInSpecifiedUnits",
        getter = svg_angle_getter,
        setter = svg_angle_setter,
        data = callback_data_index_value(scope, 2)
    )]
    value_in_specified_units: (),

    #[webapi(
        accessor_property = "valueAsString",
        getter = svg_angle_getter,
        setter = svg_angle_setter,
        data = callback_data_index_value(scope, 3)
    )]
    value_as_string: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG angle newValueSpecifiedUnits")]
struct SvgAngleNewValueSpecifiedUnitsArgs {
    #[webidl(required, converter = "unsigned_short")]
    unit_type: u16,
    #[webidl(required, converter = "float")]
    value: f32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG angle convertToSpecifiedUnits")]
struct SvgAngleConvertToSpecifiedUnitsArgs {
    #[webidl(required, converter = "unsigned_short")]
    unit_type: u16,
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAngleTemplateMethodsDeclaration::initialize_template(scope, template);
    SvgAngleTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgAngleTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

#[derive(Clone, Debug)]
pub(super) struct SvgParsedAngle {
    value: f64,
    value_in_specified_units: f64,
    unit_type: u32,
    value_as_string: String,
}

impl Default for SvgParsedAngle {
    fn default() -> Self {
        Self {
            value: 0.0,
            value_in_specified_units: 0.0,
            unit_type: SVG_ANGLE_TYPE_UNSPECIFIED,
            value_as_string: "0".to_owned(),
        }
    }
}

pub(super) fn parse_svg_angle_value(raw: &str) -> Option<SvgParsedAngle> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let lowercase = raw.to_ascii_lowercase();
    let (number_raw, unit_type, degrees_per_unit) = if lowercase.ends_with("deg") {
        (&raw[..raw.len() - 3], SVG_ANGLE_TYPE_DEG, 1.0)
    } else if lowercase.ends_with("grad") {
        (&raw[..raw.len() - 4], SVG_ANGLE_TYPE_GRAD, 0.9)
    } else if lowercase.ends_with("rad") {
        (
            &raw[..raw.len() - 3],
            SVG_ANGLE_TYPE_RAD,
            180.0 / std::f64::consts::PI,
        )
    } else if lowercase.ends_with("turn") {
        (&raw[..raw.len() - 4], SVG_ANGLE_TYPE_UNKNOWN, 360.0)
    } else {
        (raw, SVG_ANGLE_TYPE_UNSPECIFIED, 1.0)
    };
    let value_in_specified_units = moli_css_parse::parse_number(number_raw.trim())?;
    if !(value_in_specified_units as f32).is_finite()
        || !((value_in_specified_units * degrees_per_unit) as f32).is_finite()
    {
        return None;
    }
    Some(SvgParsedAngle {
        value: value_in_specified_units * degrees_per_unit,
        value_in_specified_units,
        unit_type,
        value_as_string: raw.to_owned(),
    })
}

pub(super) fn svg_angle_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    get_private_value(scope, object, slot)?.number_value(scope)
}

pub(super) fn svg_angle_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn set_svg_angle_parsed_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    parsed: &SvgParsedAngle,
) {
    set_private_value(
        scope,
        object,
        SVG_ANGLE_UNIT_TYPE_SLOT,
        v8::Integer::new_from_unsigned(scope, parsed.unit_type).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANGLE_VALUE_SLOT,
        v8::Number::new(scope, parsed.value).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT,
        v8::Number::new(scope, parsed.value_in_specified_units).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANGLE_VALUE_AS_STRING_SLOT,
        v8_string(scope, &parsed.value_as_string)
            .unwrap_or_else(|| v8str(scope, "0"))
            .into(),
    );
}

fn svg_angle_current_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> (u32, bool) {
    let unit_type = svg_angle_number_slot(scope, object, SVG_ANGLE_UNIT_TYPE_SLOT)
        .unwrap_or(SVG_ANGLE_TYPE_UNSPECIFIED as f64) as u32;
    let is_turn = unit_type == SVG_ANGLE_TYPE_UNKNOWN
        && svg_angle_string_slot(scope, object, SVG_ANGLE_VALUE_AS_STRING_SLOT)
            .is_some_and(|value| value.trim().to_ascii_lowercase().ends_with("turn"));
    (unit_type, is_turn)
}

fn svg_angle_degrees_per_unit(unit_type: u32, is_turn: bool) -> f64 {
    match unit_type {
        SVG_ANGLE_TYPE_RAD => 180.0 / std::f64::consts::PI,
        SVG_ANGLE_TYPE_GRAD => 0.9,
        SVG_ANGLE_TYPE_UNKNOWN if is_turn => 360.0,
        _ => 1.0,
    }
}

fn svg_angle_unit_suffix(unit_type: u32, is_turn: bool) -> &'static str {
    match unit_type {
        SVG_ANGLE_TYPE_DEG => "deg",
        SVG_ANGLE_TYPE_RAD => "rad",
        SVG_ANGLE_TYPE_GRAD => "grad",
        SVG_ANGLE_TYPE_UNKNOWN if is_turn => "turn",
        _ => "",
    }
}

fn svg_angle_from_specified_value(
    value_in_specified_units: f64,
    unit_type: u32,
    is_turn: bool,
) -> SvgParsedAngle {
    SvgParsedAngle {
        value: value_in_specified_units * svg_angle_degrees_per_unit(unit_type, is_turn),
        value_in_specified_units,
        unit_type,
        value_as_string: format!(
            "{}{}",
            svg_geometry::serialize_number(value_in_specified_units),
            svg_angle_unit_suffix(unit_type, is_turn)
        ),
    }
}

pub(super) fn set_svg_angle_value_degrees<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
) {
    let (unit_type, is_turn) = svg_angle_current_unit(scope, object);
    let specified = value / svg_angle_degrees_per_unit(unit_type, is_turn);
    let parsed = svg_angle_from_specified_value(specified, unit_type, is_turn);
    set_svg_angle_parsed_value(scope, object, &parsed);
}

pub(super) fn set_svg_angle_value_in_specified_units<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
) {
    let (unit_type, is_turn) = svg_angle_current_unit(scope, object);
    let parsed = svg_angle_from_specified_value(value, unit_type, is_turn);
    set_svg_angle_parsed_value(scope, object, &parsed);
}

pub(super) fn set_svg_angle_new_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit_type: u32,
    value: f64,
) -> bool {
    if !(SVG_ANGLE_TYPE_UNSPECIFIED..=SVG_ANGLE_TYPE_GRAD).contains(&unit_type) {
        return false;
    }
    let parsed = svg_angle_from_specified_value(value, unit_type, false);
    set_svg_angle_parsed_value(scope, object, &parsed);
    true
}

pub(super) fn convert_svg_angle_to_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit_type: u32,
) -> bool {
    if !(SVG_ANGLE_TYPE_UNSPECIFIED..=SVG_ANGLE_TYPE_GRAD).contains(&unit_type) {
        return false;
    }
    let value = svg_angle_number_slot(scope, object, SVG_ANGLE_VALUE_SLOT).unwrap_or(0.0);
    let specified = value / svg_angle_degrees_per_unit(unit_type, false);
    let parsed = svg_angle_from_specified_value(specified, unit_type, false);
    set_svg_angle_parsed_value(scope, object, &parsed);
    true
}

fn build_svg_angle_from_parsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: &SvgParsedAngle,
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let object = crate::context_bootstrap::exposed_interfaces::build_intrinsic_interface_instance(
        scope, "SVGAngle",
    )
    .expect("SVGAngle intrinsic instance should materialize");
    SvgAngleObjectDeclaration::new(
        parsed.unit_type,
        parsed.value,
        parsed.value_in_specified_units,
        parsed.value_as_string.clone(),
        read_only,
    )
    .initialize(scope, object)
    .expect("SVGAngle native slots should initialize");
    object
}

pub(super) fn build_svg_angle<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    build_svg_angle_from_parsed(scope, &SvgParsedAngle::default(), false)
}

pub(super) fn svg_angle_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGAngle receiver check validates native identity");
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANGLE_ACCESSOR_NAMES,
        "SVGAngle attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    match name {
        "unitType" => {
            let value = svg_angle_number_slot(scope, receiver, SVG_ANGLE_UNIT_TYPE_SLOT)
                .unwrap_or(SVG_ANGLE_TYPE_UNSPECIFIED as f64);
            rv.set(v8::Integer::new_from_unsigned(scope, value as u32).into());
        }
        "value" => {
            let value = svg_angle_number_slot(scope, receiver, SVG_ANGLE_VALUE_SLOT).unwrap_or(0.0);
            rv.set(v8::Number::new(scope, f64::from(value as f32)).into());
        }
        "valueInSpecifiedUnits" => {
            let value =
                svg_angle_number_slot(scope, receiver, SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT)
                    .unwrap_or(0.0);
            rv.set(v8::Number::new(scope, f64::from(value as f32)).into());
        }
        "valueAsString" => rv.set(
            get_private_value(scope, receiver, SVG_ANGLE_VALUE_AS_STRING_SLOT)
                .unwrap_or_else(|| v8str(scope, "0").into()),
        ),
        _ => rv.set_undefined(),
    }
}

pub(super) fn svg_angle_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGAngle receiver check validates native identity");
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANGLE_ACCESSOR_NAMES,
        "SVGAngle attributes",
    ) else {
        return;
    };
    match name {
        "value" | "valueInSpecifiedUnits" => {
            let value = match webidl::convert::<webidl::Float>(
                scope,
                args.get(0),
                webidl::Context::member("SVGAngle", name),
            ) {
                Ok(value) => f64::from(value.0),
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            if name == "value" {
                set_svg_angle_value_degrees(scope, receiver, value);
            } else {
                set_svg_angle_value_in_specified_units(scope, receiver, value);
            }
        }
        "valueAsString" => {
            let value = match webidl::convert::<webidl::DomString>(
                scope,
                args.get(0),
                webidl::Context::member("SVGAngle", "valueAsString"),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            let Some(parsed) = parse_svg_angle_value(&value) else {
                throw_dom_exception(scope, "SyntaxError", 12, "Invalid SVG angle value.");
                return;
            };
            set_svg_angle_parsed_value(scope, receiver, &parsed);
        }
        _ => (),
    }
}

pub(super) fn svg_angle_new_value_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGAngle receiver check validates native identity");
    let Some(parsed) = webidl::parse_args::<SvgAngleNewValueSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    if !set_svg_angle_new_value(
        scope,
        receiver,
        parsed.unit_type as u32,
        f64::from(parsed.value),
    ) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The SVG angle unit type is not supported.",
        );
        return;
    }
    rv.set_undefined();
}

pub(super) fn svg_angle_convert_to_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGAngle receiver check validates native identity");
    let Some(parsed) = webidl::parse_args::<SvgAngleConvertToSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    if !convert_svg_angle_to_unit(scope, receiver, parsed.unit_type as u32) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The SVG angle unit type is not supported.",
        );
        return;
    }
    rv.set_undefined();
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGSVGElement, enumerable, receiver)]
struct Factory {
    #[webapi(method = "createSVGAngle", length = 0, callback = create_angle)]
    create_svg_angle: (),
}

pub(super) fn install_factory<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    Factory::initialize_prototype_template(scope, prototype);
}

fn create_angle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(build_svg_angle(scope).into());
}
