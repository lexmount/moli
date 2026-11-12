use crate::{document_runtime::DomHandle, dom::native::Attribute, native_bridge::PageScriptHost};

pub(super) fn append_parser_custom_element_token_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
    token_attributes: &[Attribute],
) {
    for attribute in token_attributes {
        set_parser_token_attribute(scope, host_ptr, handle, attribute);
    }
}

pub(super) fn set_parser_token_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
    attribute: &Attribute,
) {
    let _ = unsafe { &mut *host_ptr }
        .set_parser_custom_element_token_attribute(scope, host_ptr, handle, attribute);
}
