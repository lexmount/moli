use super::super::{JsContextHost, OwnerDispatchScope, lightweight_popup_id_from_window};
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

impl WindowOperationReceiverCaptureError {
    /// Ordinary operation functions report receiver errors in their own realm.
    /// Cross-origin WindowProxy/Location access surfaces have a separate error path.
    pub(crate) fn throw(self, scope: &mut v8::PinScope<'_, '_>) {
        match self {
            Self::IllegalInvocation => crate::util::throw_type_error(scope, "Illegal invocation"),
            Self::CrossOrigin => crate::native_bridge::throw_dom_exception(
                scope,
                "SecurityError",
                18,
                "Blocked access to a cross-origin Window.",
            ),
        }
    }
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
        host: &JsContextHost,
    ) -> Result<Self, WindowOperationReceiverCaptureError> {
        let current_context = scope.get_current_context();
        if receiver.strict_equals(current_context.global(scope).into())
            && is_window_receiver(scope, receiver)
            && host
                .window_execution_context_identity_for_access_check(current_context)
                .is_none_or(|identity| !host.window_execution_context_identity_is_current(identity))
        {
            // The context registry owns this exact native global, including
            // its retirement. Avoid private-slot reads on a detached proxy and
            // never resolve its old child marker to a replacement Window.
            return Ok(Self {
                binding_at_capture: None,
            });
        }
        let marked_scope = marked_window_dispatch_scope(scope, receiver);
        if marked_scope.is_none() && !is_window_receiver(scope, receiver) {
            return Err(WindowOperationReceiverCaptureError::IllegalInvocation);
        }

        let relevant_context = receiver.get_creation_context(scope);
        let relevant_identity = relevant_context.and_then(|context| {
            host.window_execution_context_identity_for_v8_context(scope, context)
        });
        let target_scope = marked_scope
            .or_else(|| relevant_identity.map(|identity| identity.dispatch_scope()))
            .or_else(|| {
                receiver
                    .strict_equals(scope.get_current_context().global(scope).into())
                    .then(|| {
                        host.current_runtime_window_execution_context_identity(scope)
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
        let Some(target_owner) = host.current_window_execution_context_owner(target_scope) else {
            return Ok(Self {
                binding_at_capture: None,
            });
        };
        let Some(accessing_identity) =
            host.current_runtime_window_execution_context_identity(scope)
        else {
            return Ok(Self {
                binding_at_capture: None,
            });
        };
        if !host
            .window_execution_context_can_access_dispatch_scope(accessing_identity, target_scope)
        {
            return Err(WindowOperationReceiverCaptureError::CrossOrigin);
        }

        let binding_at_capture = match target_scope {
            OwnerDispatchScope::LightweightPopup(_) => host
                .clone_window_execution_context_binding(scope, target_owner, target_scope)
                .filter(|binding| {
                    relevant_context.is_some_and(|context| binding.context(scope) == context)
                }),
            OwnerDispatchScope::Top | OwnerDispatchScope::Child(_) => {
                match (relevant_context, relevant_identity) {
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
                }
            }
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
        .or_else(|| {
            lightweight_popup_id_from_window(scope, receiver)
                .map(OwnerDispatchScope::LightweightPopup)
        })
}
