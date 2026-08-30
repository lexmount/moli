use super::*;
use indexmap::IndexSet;

pub(super) fn class_list_tokens(
    runtime: &JsContextHost,
    handle: DomHandle,
    kind: DomTokenListKind,
) -> Vec<String> {
    element_attribute(runtime, handle, token_list_attribute_name(kind))
        .map(|value| {
            value
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect::<IndexSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn set_class_list_tokens(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    kind: DomTokenListKind,
    tokens: &[String],
) {
    let value = tokens.join(" ");
    set_reflected_attribute(
        scope,
        runtime_ptr,
        handle,
        token_list_attribute_name(kind),
        &value,
    );
}

pub(super) fn token_list_attribute_name(kind: DomTokenListKind) -> &'static str {
    match kind {
        DomTokenListKind::Class => "class",
        DomTokenListKind::Part => "part",
        DomTokenListKind::Rel => "rel",
        DomTokenListKind::HtmlFor => "for",
        DomTokenListKind::Sandbox => "sandbox",
        DomTokenListKind::Sizes => "sizes",
    }
}
