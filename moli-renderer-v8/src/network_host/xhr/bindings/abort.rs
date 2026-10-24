use super::*;

pub(super) fn xhr_abort_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = xhr_receiver_target(scope, args.this());
    if crate::worker::try_worker_xhr_abort_callback(scope, xhr) {
        return;
    }

    super::super::delivery::cancel_xhr_timeout(scope, xhr);
    super::super::delivery::clear_xhr_progress_throttle(scope, xhr);
    super::super::delivery::clear_xhr_timeout_start(scope, xhr);
    let internal_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u64;
    if internal_id != 0
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let _ = unsafe { &mut *host_ptr }.abort_subresource_fetch(internal_id);
    }

    super::super::delivery::finish_xhr_abort(scope, xhr);
}
