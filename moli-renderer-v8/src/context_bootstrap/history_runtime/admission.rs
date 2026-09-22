use super::super::navigation_window::{navigation_document_is_active, runtime_window_owner};
use super::super::{context_host_ptr_from_global_bridge, throw_dom_exception_value};

/// History API algorithms call this after WebIDL argument conversion.
pub(in crate::context_bootstrap) fn require_fully_active_history_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner = runtime_window_owner(scope, history);
    let is_fully_active = if let Some(popup_id) =
        crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
    {
        context_host_ptr_from_global_bridge(scope)
            .is_some_and(|host_ptr| unsafe { &*host_ptr }.lightweight_popup_is_open(popup_id))
    } else {
        navigation_document_is_active(scope, owner)
    };
    if is_fully_active {
        return Some(owner);
    }
    throw_dom_exception_value(
        scope,
        "The associated Document is not fully active.",
        "SecurityError",
    );
    None
}
