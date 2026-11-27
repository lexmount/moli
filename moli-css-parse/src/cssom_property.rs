use crate::declaration::split_declaration_priority;
use style::{
    context::QuirksMode,
    moli_declaration_block::{ParsedCssomProperty, style_context, with_detached_style_context},
    stylesheets::UrlExtraData,
};

/// Parse a nonempty CSSOM setter value in the element's style context.
pub fn parse_cssom_property(
    name: &str,
    value: &str,
    important: bool,
    url_data: &UrlExtraData,
    quirks_mode: QuirksMode,
) -> Option<ParsedCssomProperty> {
    if !cssom_value_allowed(value) {
        return None;
    }
    ParsedCssomProperty::parse(
        name,
        value,
        important,
        &style_context(url_data, quirks_mode),
    )
}

/// Parse a nonempty CSSOM setter value without a document base URL.
pub fn parse_cssom_property_without_document(
    name: &str,
    value: &str,
    important: bool,
) -> Option<ParsedCssomProperty> {
    if !cssom_value_allowed(value) {
        return None;
    }
    with_detached_style_context(|context| {
        ParsedCssomProperty::parse(name, value, important, context)
    })
}

fn cssom_value_allowed(value: &str) -> bool {
    !value.is_empty()
        && (!value.contains(';') || crate::escape_top_level_semicolons(value) == value)
        && !split_declaration_priority(value).is_some_and(|(_, important)| important)
}
