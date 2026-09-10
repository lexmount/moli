/// `Worker.prototype.postMessage(data)`
pub(in crate::context_bootstrap) fn worker_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let worker = args.this();
    if args.length() == 0 {
        super::throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'Worker': 1 argument required, but only 0 present.",
        );
        return;
    }
    let val = args.get(0);
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(data) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        val,
        transfer_arg,
        "Worker",
    ) else {
        return;
    };
    if let Some(worker_id) = super::constructor::worker_id(scope, worker)
        && let Some(host_ptr) = super::context_host_ptr_from_global_bridge(scope)
    {
        let _ = unsafe { &mut *host_ptr }.post_worker_message(worker_id, data);
        return;
    }
    if let Some(worker_id) = super::constructor::worker_id(scope, worker) {
        crate::worker::post_nested_worker_message(scope, worker_id, data);
        #[cfg(test)]
        return;
    }
    #[cfg(test)]
    if let Some(handle_ptr) = super::constructor::get_worker_handle(scope, worker) {
        unsafe { &*handle_ptr }.post_message(data);
    }
}

/// `Worker.prototype.terminate()`
pub(in crate::context_bootstrap) fn worker_terminate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let worker = args.this();
    if let Some(worker_id) = super::constructor::worker_id(scope, worker)
        && let Some(host_ptr) = super::context_host_ptr_from_global_bridge(scope)
        && unsafe { &mut *host_ptr }.terminate_worker(worker_id)
    {
        unsafe { &mut *host_ptr }.forget_worker(worker_id);
        return;
    }
    if let Some(worker_id) = super::constructor::worker_id(scope, worker) {
        let _ = crate::worker::forget_nested_worker_context(scope, worker_id);
        #[cfg(test)]
        return;
    }
    #[cfg(test)]
    if let Some(handle_ptr) = super::constructor::get_worker_handle(scope, worker) {
        unsafe { Box::from_raw(handle_ptr) }.terminate();
        super::set_private_value(
            scope,
            worker,
            super::WORKER_HANDLE_SLOT,
            v8::null(scope).into(),
        );
    }
}
