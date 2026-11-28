use super::*;
use crate::{
    util::{callback_data_index_value, callback_data_item},
    web_api_interfaces,
};
use moli_css_parse::CssCounterStyleProperty::{self, Descriptor, Name};
use style::counter_style::DescriptorId;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSCounterStyleRule, enumerable)]
pub(crate) struct CssCounterStyleRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 0))]
    name: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 1))]
    system: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 2))]
    symbols: (),
    #[webapi(accessor_property = "additiveSymbols", getter = get_property, setter = set_property, data = callback_data_index_value(scope, 3))]
    additive_symbols: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 4))]
    negative: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 5))]
    prefix: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 6))]
    suffix: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 7))]
    range: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 8))]
    pad: (),
    #[webapi(accessor_property = "speakAs", getter = get_property, setter = set_property, data = callback_data_index_value(scope, 9))]
    speak_as: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = callback_data_index_value(scope, 10))]
    fallback: (),
}

const COUNTER_STYLE_PROPERTIES: &[(&str, CssCounterStyleProperty)] = &[
    ("name", Name),
    ("system", Descriptor(DescriptorId::System)),
    ("symbols", Descriptor(DescriptorId::Symbols)),
    ("additiveSymbols", Descriptor(DescriptorId::AdditiveSymbols)),
    ("negative", Descriptor(DescriptorId::Negative)),
    ("prefix", Descriptor(DescriptorId::Prefix)),
    ("suffix", Descriptor(DescriptorId::Suffix)),
    ("range", Descriptor(DescriptorId::Range)),
    ("pad", Descriptor(DescriptorId::Pad)),
    ("speakAs", Descriptor(DescriptorId::SpeakAs)),
    ("fallback", Descriptor(DescriptorId::Fallback)),
];

fn get_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((_, property)) = callback_data_item(
        scope,
        &args,
        COUNTER_STYLE_PROPERTIES,
        "CSSCounterStyleRule",
    ) else {
        return;
    };
    let rule = args.this();
    let value = css_rule_attached_native_counter_style_property(scope, rule, property)
        .or_else(|| {
            css_rule_detached_snapshot_typed_view(scope, rule, |text| property.read_detached(text))
        })
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &value));
}

fn set_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((name, property)) = callback_data_item(
        scope,
        &args,
        COUNTER_STYLE_PROPERTIES,
        "CSSCounterStyleRule",
    ) else {
        return;
    };
    let Some(value) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSCounterStyleRule", name)
    else {
        return;
    };
    let rule = args.this();
    if let Some(sheet) = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        && let Some(path) = css_rule_attached_native_path(scope, rule, sheet)
        && let Some(stylesheet) = css_style_sheet_live_stylesheet(scope, sheet)
    {
        if stylesheet.set_counter_style_property(&path, property, &value) {
            sync_css_style_sheet_change(scope, sheet);
        }
        return;
    }
    if let Some(text) = css_rule_detached_snapshot_typed_view(scope, rule, |text| {
        property.set_detached(text, &value)
    }) {
        set_detached_css_rule_snapshot_text(scope, rule, &text);
    }
}
