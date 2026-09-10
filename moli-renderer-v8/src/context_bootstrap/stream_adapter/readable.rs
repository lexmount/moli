use super::*;

/// Installs an internal read request without materializing a JavaScript read
/// result. The caller runs `maybe_pull_stream` after its synchronous read loop.
pub(crate) fn prepare_readable_stream_read_with_steps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk_steps: v8::Local<'s, v8::Function>,
    close_steps: v8::Local<'s, v8::Function>,
    error_steps: v8::Local<'s, v8::Function>,
) -> bool {
    let request = new_internal_read_request(scope, chunk_steps, close_steps, error_steps);
    perform_read_from_stream(scope, stream, request)
}

pub(in crate::context_bootstrap) fn read_from_stream_as_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let prepared = prepare_read_from_stream_as_promise(scope, stream)?;
    if prepared.pull_after_attach() {
        maybe_pull_stream(scope, stream);
    }
    Some(prepared.promise())
}

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) struct PreparedReadableStreamRead<'s> {
    promise: v8::Local<'s, v8::Promise>,
    pull_after_attach: bool,
}

impl<'s> PreparedReadableStreamRead<'s> {
    pub(in crate::context_bootstrap) const fn new(
        promise: v8::Local<'s, v8::Promise>,
        pull_after_attach: bool,
    ) -> Self {
        Self {
            promise,
            pull_after_attach,
        }
    }

    pub(in crate::context_bootstrap) const fn promise(self) -> v8::Local<'s, v8::Promise> {
        self.promise
    }

    pub(in crate::context_bootstrap) const fn pull_after_attach(self) -> bool {
        self.pull_after_attach
    }
}

/// Creates the public reader's promise-backed read request and commits it
/// without invoking the source pull algorithm. Composite promise owners can
/// install their reactions before applying the returned pull effect.
pub(in crate::context_bootstrap) fn prepare_read_from_stream_as_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<PreparedReadableStreamRead<'s>> {
    let (promise, pending) = new_pending_read_promise(scope)?;
    let pull_after_attach = perform_read_from_stream(scope, stream, pending);
    Some(PreparedReadableStreamRead {
        promise,
        pull_after_attach,
    })
}

/// Performs the default-reader read algorithm with either a public
/// promise-backed request or an internal read request.
pub(in crate::context_bootstrap::stream_adapter) fn perform_read_from_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    disturb_readable_stream(scope, stream);
    match readable_stream_snapshot(scope, stream).plan_read_start() {
        moli_streams::readable::ReadStartPlan::RejectStoredError => {
            if let Some(error) = readable_stream_error(scope, stream) {
                error_read_request(scope, request, error);
                return false;
            }
        }
        moli_streams::readable::ReadStartPlan::ResolveDone => {
            fulfill_read_request(scope, request, v8::undefined(scope).into(), true);
            return false;
        }
        moli_streams::readable::ReadStartPlan::Continue => {}
    }
    match dequeue_readable_stream_queue_value(scope, stream) {
        Ok(Some(value)) => {
            finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
            fulfill_read_request(scope, request, value, false);
            return true;
        }
        Ok(None) => {}
        Err(_) => {
            error_read_request_for_queue_error(scope, stream, request);
            return false;
        }
    }
    finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
    if readable_stream_closed(scope, stream) {
        fulfill_read_request(scope, request, v8::undefined(scope).into(), true);
        return false;
    }
    if readable_stream_is_byte_stream(scope, stream) {
        match enqueue_auto_allocate_pull_into(scope, stream, request) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(error) => {
                error_stream(scope, stream, error);
                error_read_request(scope, request, error);
                return false;
            }
        }
    }
    enqueue_pending_read(scope, stream, request);
    true
}

pub(in crate::context_bootstrap) fn readable_stream_closed_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Value>, Option<v8::Local<'s, v8::Object>>)> {
    match readable_stream_snapshot(scope, stream).plan_closed_promise() {
        moli_streams::readable::ClosedPromisePlan::RejectStoredError => {
            if let Some(error) = readable_stream_error(scope, stream) {
                return rejected_promise_value(scope, error).map(|promise| (promise, None));
            }
        }
        moli_streams::readable::ClosedPromisePlan::Resolve => {
            return resolved_promise_value(scope, v8::undefined(scope).into())
                .map(|promise| (promise, None));
        }
        moli_streams::readable::ClosedPromisePlan::Wait => {}
    }
    let (promise, pending) = new_pending_read_promise(scope)?;
    enqueue_pending_closed_promise(scope, stream, pending);
    Some((promise.into(), Some(pending)))
}

fn error_read_request_for_queue_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
) {
    let error = readable_stream_queue_error_value(scope);
    error_stream(scope, stream, error);
    error_read_request(scope, request, error);
}

pub(crate) fn maybe_pull_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let snapshot = readable_stream_default_controller_snapshot(scope, stream);
    if !snapshot.can_consider_pull() {
        return;
    }
    let plan = snapshot.plan_pull(readable_stream_has_pull_demand(scope, stream));
    match plan {
        moli_streams::readable::default_controller::PullPlan::None => return,
        moli_streams::readable::default_controller::PullPlan::MarkPullAgain(plan) => {
            apply_readable_stream_pull_state(scope, stream, plan.next());
            return;
        }
        moli_streams::readable::default_controller::PullPlan::Start(_) => {}
    }
    let Some(pull_algorithm) = readable_stream_controller_algorithm_value(
        scope,
        stream,
        READABLE_STREAM_ALGORITHM_PULL_INDEX,
    ) else {
        return;
    };
    let webidl_callback = stored_stream_algorithm_is_webidl(pull_algorithm);
    let Some(controller) = stream_slot_object(scope, stream, READABLE_STREAM_CONTROLLER_SLOT)
    else {
        return;
    };
    let Some(source) = readable_stream_controller_algorithm_object(
        scope,
        stream,
        READABLE_STREAM_ALGORITHM_SOURCE_INDEX,
    ) else {
        return;
    };
    let moli_streams::readable::default_controller::PullPlan::Start(plan) = plan else {
        return;
    };
    apply_readable_stream_pull_state(scope, stream, plan.next());
    set_readable_stream_pull_state_bit(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_CALLBACK_ACTIVE,
        true,
    );
    let invocation = if webidl_callback {
        invoke_stored_stream_promise_algorithm(
            scope,
            pull_algorithm,
            source.into(),
            &[controller.into()],
        )
    } else {
        invoke_stored_stream_algorithm(scope, pull_algorithm, source.into(), &[controller.into()])
    };
    set_readable_stream_pull_state_bit(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_CALLBACK_ACTIVE,
        false,
    );
    match invocation {
        Ok(result) => {
            if let Some(result) = result {
                attach_readable_stream_pull_reaction_handlers(scope, stream, result);
            } else {
                readable_stream_pull_fulfilled(scope, stream, false);
            }
        }
        Err(error) => {
            super::pipe::clear_pipe_drain_pull_barrier(scope, stream);
            let settlement = readable_stream_pull_state(scope, stream).pull_rejected();
            apply_readable_stream_pull_state(scope, stream, settlement.next());
            error_stream(scope, stream, error);
        }
    }
    super::pipe::flush_pipe_drain_after_pull_invocation(scope, stream);
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_pull_callback_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    readable_stream_pull_state_has(scope, stream, READABLE_STREAM_PULL_STATE_CALLBACK_ACTIVE)
}

fn readable_stream_has_pull_demand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    if readable_stream_locked(scope, stream)
        && stream_slot_array(scope, stream, READABLE_STREAM_PENDING_READS_SLOT)
            .is_some_and(|pending| pending.length() > 0)
    {
        return true;
    }
    if let Some(pipe_state) = super::pipe::pipe_owner_state_for_source(scope, stream) {
        let high_water_mark =
            stream_slot_number(scope, stream, READABLE_STREAM_HWM_SLOT).unwrap_or(1.0);
        let source_has_capacity = moli_streams::strategy::StrategySnapshot::new(
            high_water_mark,
            readable_stream_queue_total_size(scope, stream),
        )
        .has_capacity();
        return pipe_state.has_pull_demand(source_has_capacity);
    }
    let high_water_mark =
        stream_slot_number(scope, stream, READABLE_STREAM_HWM_SLOT).unwrap_or(1.0);
    moli_streams::strategy::StrategySnapshot::new(
        high_water_mark,
        readable_stream_queue_total_size(scope, stream),
    )
    .has_capacity()
}

pub(in crate::context_bootstrap::stream_adapter) fn mark_readable_stream_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let next = readable_stream_pull_state(scope, stream).mark_started();
    apply_readable_stream_pull_state(scope, stream, next);
}

fn attach_readable_stream_pull_reaction_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    pull_result: v8::Local<'s, v8::Value>,
) {
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(pull_result) else {
        readable_stream_pull_fulfilled(scope, stream, false);
        return;
    };
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(readable_stream_pull_fulfilled_callback).data(stream.into()),
        "readable pull fulfillment",
        v8::Function::builder(readable_stream_pull_rejected_callback).data(stream.into()),
        "readable pull rejection",
        "readable pull",
    )
    .finish_at_owner_boundary();
}

fn readable_stream_pull_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(stream) = v8::Local::<v8::Object>::try_from(args.data()) {
        readable_stream_pull_fulfilled(scope, stream, true);
    }
    rv.set_undefined();
}

fn readable_stream_pull_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(stream) = v8::Local::<v8::Object>::try_from(args.data()) {
        super::pipe::clear_pipe_drain_pull_barrier(scope, stream);
        let settlement = readable_stream_pull_state(scope, stream).pull_rejected();
        apply_readable_stream_pull_state(scope, stream, settlement.next());
        error_stream(scope, stream, args.get(0));
    }
    rv.set_undefined();
}

fn readable_stream_pull_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    continue_pipe: bool,
) {
    super::pipe::clear_pipe_drain_pull_barrier(scope, stream);
    let settlement = readable_stream_pull_state(scope, stream).pull_fulfilled();
    apply_readable_stream_pull_state(scope, stream, settlement.next());
    if settlement.action()
        == moli_streams::readable::default_controller::PullSettlementAction::PullAgain
    {
        maybe_pull_stream(scope, stream);
        return;
    }
    if continue_pipe {
        super::pipe::schedule_pipe_to_drain(scope, stream);
    }
}

fn readable_stream_default_controller_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> moli_streams::readable::default_controller::DefaultControllerSnapshot {
    let readable = readable_stream_snapshot(scope, stream);
    moli_streams::readable::default_controller::DefaultControllerSnapshot::new(
        readable.state(),
        readable.close_requested(),
        readable_stream_queue_exists(scope, stream),
        readable_stream_pull_state(scope, stream),
    )
}

fn readable_stream_pull_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> moli_streams::readable::default_controller::PullState {
    moli_streams::readable::default_controller::PullState::new(
        readable_stream_pull_state_has(scope, stream, READABLE_STREAM_PULL_STATE_STARTED),
        readable_stream_pull_state_has(scope, stream, READABLE_STREAM_PULL_STATE_PULLING),
        readable_stream_pull_state_has(scope, stream, READABLE_STREAM_PULL_STATE_PULL_AGAIN),
    )
}

fn apply_readable_stream_pull_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    state: moli_streams::readable::default_controller::PullState,
) {
    set_readable_stream_pull_state_bit(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_STARTED,
        state.started(),
    );
    set_readable_stream_pull_state_bit(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_PULLING,
        state.pulling(),
    );
    set_readable_stream_pull_state_bit(
        scope,
        stream,
        READABLE_STREAM_PULL_STATE_PULL_AGAIN,
        state.pull_again(),
    );
}

/// Fetch ignores the cancellation promise and keeps its own abort reason.
/// This must run the source algorithm before returning to the fetch caller.
pub(crate) fn cancel_readable_stream_for_fetch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    if !readable_stream_snapshot(scope, stream)
        .state()
        .is_readable()
    {
        return;
    }
    if let Some(result) = cancel_readable_stream(scope, stream, reason)
        && let Ok(promise) = v8::Local::<v8::Promise>::try_from(result)
    {
        promise.mark_as_handled();
    }
}

pub(crate) fn cancel_readable_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    disturb_readable_stream(scope, stream);
    let finish_requested_close = match readable_stream_snapshot(scope, stream).plan_cancel() {
        moli_streams::readable::CancelPlan::RejectStoredError => {
            if let Some(error) = readable_stream_error(scope, stream) {
                return rejected_promise_value(scope, error);
            }
            false
        }
        moli_streams::readable::CancelPlan::Resolve => {
            return resolved_promise_value(scope, v8::undefined(scope).into());
        }
        moli_streams::readable::CancelPlan::RunAlgorithm {
            finish_requested_close,
        } => finish_requested_close,
    };
    reset_readable_stream_queue(scope, stream);
    if readable_stream_is_byte_stream(scope, stream) {
        reset_byte_stream_pending_pull_intos(scope, stream);
    }
    if finish_requested_close {
        finish_readable_stream_close(scope, stream);
    } else {
        close_stream(scope, stream);
    }
    if let Some(cancel_algorithm) = readable_stream_controller_algorithm_value(
        scope,
        stream,
        READABLE_STREAM_ALGORITHM_CANCEL_INDEX,
    ) && let Some(source) = readable_stream_controller_algorithm_object(
        scope,
        stream,
        READABLE_STREAM_ALGORITHM_SOURCE_INDEX,
    ) {
        let webidl_callback = stored_stream_algorithm_is_webidl(cancel_algorithm);
        let invocation = if webidl_callback {
            invoke_stored_stream_promise_algorithm(
                scope,
                cancel_algorithm,
                source.into(),
                &[reason],
            )
        } else {
            invoke_stored_stream_algorithm(scope, cancel_algorithm, source.into(), &[reason])
        };
        let result = match invocation {
            Ok(result) => result.unwrap_or_else(|| v8::undefined(scope).into()),
            Err(error) => return rejected_promise_value(scope, error),
        };
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) {
            let on_fulfilled =
                v8::Function::builder(super::utils::promise_return_undefined_callback)
                    .build(scope)?;
            return promise.then(scope, on_fulfilled).map(Into::into);
        }
    }
    resolved_promise_value(scope, v8::undefined(scope).into())
}
