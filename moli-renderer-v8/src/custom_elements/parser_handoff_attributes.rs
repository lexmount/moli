use crate::{document_runtime::DomHandle, dom::native::Attribute, native_bridge::JsContextHost};

pub(super) fn append_parser_custom_element_token_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    token_attributes: &moli_dom::native::ParserAttributes,
) {
    for attribute in token_attributes.iter() {
        set_parser_token_attribute(
            scope,
            host_ptr,
            handle,
            attribute,
            token_attributes.source_location().cloned(),
        );
    }
}

pub(super) fn set_parser_token_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    attribute: &Attribute,
    source_location: Option<std::sync::Arc<moli_dom::native::SourceLocation>>,
) {
    let _ = unsafe { &mut *host_ptr }.set_parser_custom_element_token_attribute(
        scope,
        host_ptr,
        handle,
        attribute,
        source_location,
    );
}
