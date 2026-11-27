use super::*;
use crate::{util::v8str, web_api_interfaces};
use moli_css_parse::CssCounterStyleProperty::{self, Descriptor, Name};
use style::counter_style::DescriptorId;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSCounterStyleRule, enumerable)]
pub(crate) struct CssCounterStyleRulePrototypeDeclaration {
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "name"))]
    name: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "system"))]
    system: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "symbols"))]
    symbols: (),
    #[webapi(accessor_property = "additiveSymbols", getter = get_property, setter = set_property, data = v8str(scope, "additiveSymbols"))]
    additive_symbols: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "negative"))]
    negative: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "prefix"))]
    prefix: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "suffix"))]
    suffix: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "range"))]
    range: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "pad"))]
    pad: (),
    #[webapi(accessor_property = "speakAs", getter = get_property, setter = set_property, data = v8str(scope, "speakAs"))]
    speak_as: (),
    #[webapi(accessor_property, getter = get_property, setter = set_property, data = v8str(scope, "fallback"))]
    fallback: (),
}

fn property_from_callback_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(&'static str, CssCounterStyleProperty)> {
    let name = v8::Local::<v8::String>::try_from(data).ok()?;
    let name = name.to_rust_string_lossy(scope);
    [
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
    ]
    .into_iter()
    .find(|(property_name, _)| *property_name == name)
}

fn get_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((_, property)) = property_from_callback_data(scope, args.data()) else {
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
    let Some((name, property)) = property_from_callback_data(scope, args.data()) else {
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
