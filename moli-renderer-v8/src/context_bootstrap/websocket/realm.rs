use crate::util::{callable_relevant_context, throw_type_error};

pub(super) fn websocket_constructor_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    new_target: v8::Local<'s, v8::Value>,
    interface: &'static str,
) -> Option<v8::Local<'s, v8::Context>> {
    if let Some(context) = callable_relevant_context(scope, new_target) {
        return Some(context);
    }
    throw_type_error(
        scope,
        &format!("Failed to construct '{interface}': the constructor realm is unavailable."),
    );
    None
}

pub(super) fn effective_websocket_document_scope(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> Option<(
    crate::native_bridge::WindowExecutionContextBinding,
    Option<String>,
    url::Url,
)> {
    let binding = host.current_runtime_window_execution_context_binding(scope)?;
    match binding.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            let (frame_id, document_url) = host.child_browsing_context_request_scope(handle)?;
            Some((binding, Some(frame_id), document_url))
        }
        crate::native_bridge::OwnerDispatchScope::Top => {
            Some((binding, None, host.document_url().clone()))
        }
    }
}
