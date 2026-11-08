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

    super::super::delivery::clear_xhr_request_timers(scope, xhr);
    terminate_window_xhr_fetch(scope, xhr);

    super::super::delivery::finish_xhr_abort(scope, xhr);
}
