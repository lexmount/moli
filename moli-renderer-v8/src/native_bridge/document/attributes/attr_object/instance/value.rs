use super::*;
use crate::dom::native::DomStringValue;

pub(super) fn attr_owner_element_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    AttrReference::from_object(scope, attr)?.owner_element(scope)
}

pub(super) fn attr_owner_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    AttrReference::from_object(scope, attr)?.owner_document(scope)
}

pub(in crate::native_bridge::document) fn attr_current_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> DomStringValue {
    AttrReference::from_object(scope, attr)
        .map(|attr| attr.value(scope))
        .unwrap_or_default()
}
