use super::*;

pub(super) fn computed_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    for longhand in style::properties::ShorthandId::Font.longhands() {
        let property = longhand.name();
        let value = resolution.computed_property(runtime, handle, property);
        if value.is_empty()
            || block.set_property(property, &value, false)
                == moli_css_parse::CssSetResult::ParseError
        {
            return String::new();
        }
    }
    // The native serializer handles CSS2 variant/stretch restrictions,
    // reset-only longhands and omission of initial components.
    block.property_value("font").unwrap_or_default()
}
