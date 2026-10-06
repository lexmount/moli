use super::*;
use indexmap::IndexSet;

pub(super) fn class_list_tokens(
    runtime: &JsContextHost,
    handle: DomHandle,
    kind: DomTokenListKind,
) -> Vec<Vec<u16>> {
    runtime
        .dom_host()
        .get_attribute_ns_utf16_units(handle, None, token_list_attribute_name(kind))
        .map(|value| {
            value
                .split(|unit| matches!(*unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
                .filter(|token| !token.is_empty())
                .map(<[u16]>::to_vec)
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
    tokens: &[Vec<u16>],
) {
    let value = tokens.join(&u16::from(b' '));
    super::super::reflection::set_reflected_attribute_utf16(
        scope,
        runtime_ptr,
        handle,
        token_list_attribute_name(kind),
        value,
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
