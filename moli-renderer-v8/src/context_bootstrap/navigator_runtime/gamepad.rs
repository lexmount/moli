use super::super::context_host_ptr_from_global_bridge;
use super::navigator::navigator_receiver_branded;
use crate::{util::throw_type_error, webidl};

pub(super) fn navigator_get_gamepads_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !navigator_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }

    // The Gamepad algorithm uses the current global's document, including
    // when a method from one Window is borrowed by another Navigator.
    let context = scope.get_current_context();
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the bridge owns the host for the lifetime of this callback.
        let host = unsafe { &*host_ptr };
        if let Some(identity) =
            host.window_execution_context_identity_for_v8_context(scope, context)
            && host.window_execution_context_identity_is_current(identity)
            && host
                .document_permissions_policy_for_owner(identity.dispatch_scope())
                .is_some_and(|policy| !policy.gamepad_enabled())
        {
            webidl::throw_dom_exception(
                scope,
                "SecurityError",
                "Access to gamepads is disallowed by permissions policy.",
            );
            return;
        }
    }

    // No gamepad backend is connected. An inactive document also returns an
    // empty sequence before checking policy. Each call produces a new Array.
    rv.set(v8::Array::new(scope, 0).into());
}
