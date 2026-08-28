use super::super::{JsContextHost, OwnerDispatchScope};
use super::WindowExecutionContextBinding;
use crate::{
    context_bootstrap::{CHILD_BROWSING_CONTEXT_HANDLE_SLOT, is_window_receiver},
    native_bridge::child_window_handle_from_marker_data,
    util::get_private_value,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowOperationReceiverCaptureError {
    IllegalInvocation,
    CrossOrigin,
}

/// Window receiver frozen at the start of a WebIDL operation.
///
/// Blink separates three phases in generated Window bindings:
///
/// 1. validate and authorize the receiver;
/// 2. convert arguments in the current calling realm;
/// 3. pass the receiver's `ScriptState` and `LocalDOMWindow` to the native
///    implementation.
///
/// Keeping the exact binding here prevents an author getter run during phase
/// 2 from navigating an iframe and making phase 3 silently bind to the new
/// `LocalWindow` generation behind the same child-frame handle.
pub(crate) struct WindowOperationReceiver {
    binding_at_capture: Option<WindowExecutionContextBinding>,
}

impl WindowOperationReceiver {
    pub(crate) fn capture_and_authorize<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
        receiver_host: &JsContextHost,
        accessing_host: &JsContextHost,
    ) -> Result<Self, WindowOperationReceiverCaptureError> {
        let marked_scope = marked_window_dispatch_scope(scope, receiver);
        if marked_scope.is_none() && !is_window_receiver(scope, receiver) {
            return Err(WindowOperationReceiverCaptureError::IllegalInvocation);
        }
        if crate::native_bridge::is_cross_origin_top_window_proxy(scope, receiver)
            || crate::native_bridge::is_cross_origin_remote_frame_window_proxy(scope, receiver)
        {
            return Err(WindowOperationReceiverCaptureError::CrossOrigin);
        }

        let relevant_context = receiver.get_creation_context(scope);
        let relevant_identity = relevant_context.and_then(|context| {
            receiver_host.window_execution_context_identity_for_v8_context(scope, context)
        });
        let target_scope = marked_scope
            .or_else(|| relevant_identity.map(|identity| identity.dispatch_scope()))
            .or_else(|| {
                receiver
                    .strict_equals(scope.get_current_context().global(scope).into())
                    .then(|| {
                        receiver_host
                            .current_runtime_window_execution_context_identity(scope)
                            .map(|identity| identity.dispatch_scope())
                    })
                    .flatten()
            });

        // A discarded Window is still a valid WebIDL receiver. It must pass
        // the brand check so required-argument conversion can happen first,
        // then fail the native operation as a rejected current-realm Promise.
        let Some(target_scope) = target_scope else {
            return Ok(Self {
                binding_at_capture: None,
            });
        };
        let Some(target_owner) = receiver_host.current_window_execution_context_owner(target_scope)
        else {
            return Ok(Self {
                binding_at_capture: None,
            });
        };
        let Some(accessing_identity) =
            accessing_host.current_runtime_window_execution_context_identity(scope)
        else {
            return Ok(Self {
                binding_at_capture: None,
            });
        };
        let access_allowed = if std::ptr::eq(receiver_host, accessing_host) {
            accessing_host.window_execution_context_can_access_dispatch_scope(
                accessing_identity,
                target_scope,
            )
        } else {
            accessing_host.window_execution_context_can_access_related_page_dispatch_scope(
                accessing_identity,
                receiver_host,
                target_scope,
            )
        };
        if !access_allowed {
            return Err(WindowOperationReceiverCaptureError::CrossOrigin);
        }

        let binding_at_capture = match (relevant_context, relevant_identity) {
            (Some(context), Some(identity))
                if identity.owner() == target_owner
                    && identity.dispatch_scope() == target_scope =>
            {
                Some(WindowExecutionContextBinding::new(
                    target_owner,
                    target_scope,
                    identity.realm_token(),
                    v8::Global::new(scope, context),
                ))
            }
            _ => None,
        };

        Ok(Self { binding_at_capture })
    }

    /// Returns only the exact receiver realm captured before argument
    /// conversion. It never looks up a replacement binding by child handle.
    pub(crate) fn resolve_live_binding(
        self,
        host: &JsContextHost,
    ) -> Option<WindowExecutionContextBinding> {
        self.binding_at_capture
            .filter(|binding| binding.is_current(host))
    }
}

fn marked_window_dispatch_scope(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) -> Option<OwnerDispatchScope> {
    // Re-localize the receiver into this handle scope before private lookup;
    // borrowed Window methods commonly receive an object created in another
    // context of the same isolate.
    let receiver = {
        let receiver = v8::Global::new(scope, receiver);
        v8::Local::new(scope, receiver)
    };
    get_private_value(scope, receiver, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
        .map(OwnerDispatchScope::Child)
}
