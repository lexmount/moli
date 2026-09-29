use super::*;
use crate::custom_elements;

pub(in crate::native_bridge) fn set_text_content_in_reaction_scope(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    value: impl Into<moli_dom::native::DomStringValue>,
) -> bool {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.set_text_content_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            value,
        )
    })
}
