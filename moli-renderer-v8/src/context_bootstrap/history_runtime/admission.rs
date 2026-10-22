use super::super::navigation_window::{
    runtime_window_document_is_fully_active, runtime_window_owner,
};
use super::super::throw_dom_exception_value;

/// History API algorithms call this after WebIDL argument conversion.
pub(in crate::context_bootstrap) fn require_fully_active_history_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let owner = runtime_window_owner(scope, history);
    if runtime_window_document_is_fully_active(scope, owner) {
        return Some(owner);
    }
    throw_dom_exception_value(
        scope,
        "The associated Document is not fully active.",
        "SecurityError",
    );
    None
}
