use crate::document_runtime::DomHandle;

use super::super::super::super::{BridgeHandle, JsContextHost, bridge_handle_from_object};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StyleMode {
    Inline,
    Computed,
}

pub(in crate::native_bridge::element::styles) fn style_runtime_and_handle_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, DomHandle, StyleMode), String> {
    let (runtime_ptr, handle) = bridge_handle_from_object(scope, object)?;
    match handle {
        BridgeHandle::Style(handle) => Ok((runtime_ptr, handle, StyleMode::Inline)),
        BridgeHandle::ComputedStyle(handle, _) => Ok((runtime_ptr, handle, StyleMode::Computed)),
        BridgeHandle::Window
        | BridgeHandle::Node(_)
        | BridgeHandle::ClassList(_, _)
        | BridgeHandle::Dataset(_) => {
            Err("wrapper did not contain a CSSStyleDeclaration identity".to_owned())
        }
    }
}
