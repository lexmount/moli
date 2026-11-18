use super::*;
use crate::web_api_interfaces;
use moli_css_parse::{CssFontPaletteValuesProperty, parse_font_palette_values_property_with_stylo};

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSFontPaletteValuesRule, enumerable)]
pub(crate) struct CssFontPaletteValuesRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = name_getter)]
    name: (),
    #[webapi(accessor_property, getter = font_family_getter)]
    font_family: (),
    #[webapi(accessor_property, getter = base_palette_getter)]
    base_palette: (),
    #[webapi(accessor_property, getter = override_colors_getter)]
    override_colors: (),
}

fn property_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    property: CssFontPaletteValuesProperty,
) {
    let value = css_rule_attached_native_font_palette_values_property(scope, args.this(), property)
        .or_else(|| {
            css_rule_detached_snapshot_typed_view(scope, args.this(), |text| {
                parse_font_palette_values_property_with_stylo(text, property)
            })
        })
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &value));
}

fn name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    property_getter(scope, args, rv, CssFontPaletteValuesProperty::Name);
}

fn font_family_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    property_getter(scope, args, rv, CssFontPaletteValuesProperty::FontFamily);
}

fn base_palette_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    property_getter(scope, args, rv, CssFontPaletteValuesProperty::BasePalette);
}

fn override_colors_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    property_getter(
        scope,
        args,
        rv,
        CssFontPaletteValuesProperty::OverrideColors,
    );
}
