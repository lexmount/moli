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
    crate::network::context::SubresourceRequestEnvironment,
    bool,
)> {
    let binding = host.current_runtime_window_execution_context_binding(scope)?;
    let owner = binding.dispatch_scope();
    let base_url = crate::network_host::subresource_api_base_url(scope, host, owner)?;
    let (frame_id, document_url) = match owner {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => (
            Some(host.frame_owner_frame_id_for_child_handle(handle)?.0),
            host.child_browsing_context_current_url(handle)?,
        ),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            (None, host.lightweight_popup_document_url(popup_id)?)
        }
        crate::native_bridge::OwnerDispatchScope::Top => (None, host.document_url().clone()),
    };
    let request_origin = moli_url::WebOrigin::from_serialized(&host.window_document_origin(owner)?);
    let is_secure_context = host.window_is_secure_context(scope, owner)?;
    Some((
        binding,
        crate::network::context::SubresourceRequestEnvironment {
            document_url,
            base_url,
            request_origin,
            frame_id,
        },
        is_secure_context,
    ))
}
