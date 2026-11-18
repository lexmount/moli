use super::pipe_owner::PipeOwner;
use super::*;
use moli_streams::pipe::{
    PipeAbortCleanup, PipeAdmission, PipeDestinationObservation, PipeDrainCommand, PipeDrainEvent,
    PipeEndpointObservation, PipeEndpointTerminalTrigger, PipeFinalSettlement, PipeFinalization,
    PipeInitialTerminalPlan, PipeOptions, PipeOwnerMutation, PipeOwnerState, PipePullObservation,
    PipePullPlan, PipeShutdownAction, PipeShutdownCommand, PipeShutdownEvent,
    PipeShutdownOperation, PipeShutdownOperationObservation, PipeShutdownOperationPlan,
    PipeTerminalTrigger, PipeWritePublicationCommand,
};
use moli_streams::readable::ReadableState;

pub(in crate::streams::adapter) fn pipe_owner_state_for_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<PipeOwnerState> {
    PipeOwner::from_source(scope, stream).map(|owner| owner.state(scope))
}

pub fn readable_stream_has_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    PipeOwner::from_source(scope, stream).is_some()
}

pub(in crate::streams) fn schedule_pipe_to_drain<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = PipeOwner::from_source(scope, source) else {
        return;
    };
    let transition = owner
        .state(scope)
        .transition_drain(PipeDrainEvent::Schedule);
    owner.apply(scope, transition);
    if matches!(transition.command(), Some(PipeDrainCommand::Enqueue)) {
        enqueue_pipe_drain_callback(scope, owner);
    }
}

pub(in crate::streams::adapter) fn schedule_pipe_owner_drain<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_value: v8::Local<'s, v8::Object>,
) {
    let owner = PipeOwner::from_value(owner_value.into())
        .expect("writable pipe registration must contain a PipeOwner");
    if owner.state(scope).is_active() {
        let source = owner.source(scope);
        schedule_pipe_to_drain(scope, source);
    }
}

fn enqueue_pipe_drain_callback<'s>(scope: &mut v8::PinScope<'s, '_>, owner: PipeOwner<'s>) {
    let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
        scope,
        v8::Function::builder(pipe_to_drain_microtask_callback).data(owner.object().into()),
        "pipe drain microtask",
    ) else {
        return;
    };
    scope.enqueue_microtask(callback);
}

pub(in crate::streams::adapter) fn schedule_pipe_drain_after_incoming_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    fulfills_pending_read: bool,
    queue_was_empty: bool,
) {
    let Some(owner) = PipeOwner::from_source(scope, source) else {
        return;
    };
    apply_owner_mutation(scope, owner, PipeOwnerMutation::IncomingChunk);
    if super::readable::readable_stream_pull_callback_active(scope, source)
        && !fulfills_pending_read
    {
        if queue_was_empty {
            apply_owner_mutation(scope, owner, PipeOwnerMutation::SetDrainBarrier);
        }
        return;
    }
    schedule_pipe_to_drain(scope, source);
}

pub(in crate::streams::adapter) fn clear_pipe_drain_pull_barrier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) {
    if let Some(owner) = PipeOwner::from_source(scope, source) {
        apply_owner_mutation(scope, owner, PipeOwnerMutation::ClearDrainBarrier);
    }
}

pub(in crate::streams::adapter) fn flush_pipe_drain_after_pull_invocation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) {
    if PipeOwner::from_source(scope, source).is_some_and(|owner| owner.state(scope).is_active())
        && !readable_stream_queue_is_empty(scope, source)
    {
        schedule_pipe_to_drain(scope, source);
    }
}

/// Acquires the pipe's first internal read synchronously, matching
/// Chromium's `PipeToEngine::HandleNextEvent`.
pub(in crate::streams) fn prime_readable_stream_pipe_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = PipeOwner::from_source(scope, source) else {
        return;
    };
    if claim_initial_terminal_condition(scope, owner) {
        return;
    }
    let destination = owner.destination(scope);
    let observation = PipePullObservation::new(
        readable_stream_snapshot(scope, source).state(),
        readable_stream_queue_is_empty(scope, source),
        writable_stream_has_capacity(scope, destination),
    );
    match owner.state(scope).plan_before_pull(observation) {
        PipePullPlan::MarkReadPendingAndPull => {
            apply_owner_mutation(scope, owner, PipeOwnerMutation::MarkReadPending);
            maybe_pull_stream(scope, source);
        }
        PipePullPlan::Continue => schedule_pipe_to_drain(scope, source),
        PipePullPlan::Stop | PipePullPlan::BlockedByDestination => {}
    }
}

fn pipe_to_drain_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner =
        PipeOwner::from_value(args.data()).expect("pipe drain callback must retain its owner");
    let transition = owner
        .state(scope)
        .transition_drain(PipeDrainEvent::Callback);
    owner.apply(scope, transition);
    match transition.command() {
        Some(PipeDrainCommand::Enqueue) => {
            enqueue_pipe_drain_callback(scope, owner);
        }
        Some(PipeDrainCommand::Run) => drain_pipe_owner(scope, owner),
        None => {}
    }
    rv.set_undefined();
}

fn drain_pipe_owner<'s>(scope: &mut v8::PinScope<'s, '_>, owner: PipeOwner<'s>) {
    if !owner.state(scope).is_active() {
        return;
    }
    if claim_initial_terminal_condition(scope, owner) {
        return;
    }
    let source = owner.source(scope);
    let destination = owner.destination(scope);

    if readable_stream_queue_is_empty(scope, source) {
        finish_readable_stream_close_if_requested_and_queue_empty(scope, source);
        if claim_initial_terminal_condition(scope, owner) {
            return;
        }
    }

    if !writable_stream_has_capacity(scope, destination) {
        return;
    }

    if readable_stream_queue_is_empty(scope, source) {
        if matches!(
            readable_stream_snapshot(scope, source).state(),
            ReadableState::Readable
        ) && !owner.state(scope).read_pending()
        {
            apply_owner_mutation(scope, owner, PipeOwnerMutation::MarkReadPending);
            maybe_pull_stream(scope, source);
        }
        return;
    }

    let chunk = match dequeue_readable_stream_queue_value(scope, source) {
        Ok(Some(chunk)) => chunk,
        Ok(None) => return,
        Err(_) => {
            let error = readable_stream_queue_error_value(scope);
            error_stream(scope, source, error);
            return;
        }
    };

    // Publish the write intent before either refill or destination callbacks
    // can re-enter JavaScript. A terminal event in that window must wait for
    // the returned write promise to become the owner's last-write residence.
    let begin_write = owner
        .state(scope)
        .transition_mutation(PipeOwnerMutation::BeginWrite);
    assert_eq!(begin_write.command(), None);
    assert_eq!(begin_write.admission(), PipeAdmission::Applied);
    owner.apply(scope, begin_write);

    // The internal read made room in the source strategy queue. Refill that
    // slot before the destination's write applies its own backpressure.
    maybe_pull_stream(scope, source);
    let live = owner.state(scope);
    if !live.is_active() && !live.is_waiting_for_write_publication() {
        let abandoned = live.transition_mutation(PipeOwnerMutation::AbandonWrite);
        owner.apply(scope, abandoned);
        return;
    }
    let write_result = super::writable::writable_stream_write_from_pipe(scope, destination, chunk)
        .expect("a pipe write must return its settlement promise");
    let promise = v8::Local::<v8::Promise>::try_from(write_result)
        .expect("a pipe write result must be a promise");
    suppress_promise_unhandled_rejection(scope, promise.into());

    let publication = owner.state(scope).publish_last_write();
    owner.apply(scope, publication);
    match publication.command() {
        Some(PipeWritePublicationCommand::StoreLastWrite) => {
            owner.set_last_write(scope, promise);
        }
        Some(PipeWritePublicationCommand::StoreLastWriteAndWait) => {
            owner.set_last_write(scope, promise);
            wait_for_pipe_last_write(scope, owner);
        }
        None => {}
    }
    attach_pipe_write_reactions(scope, owner, promise);

    if owner.state(scope).is_active() {
        finish_readable_stream_close_if_requested_and_queue_empty(scope, source);
        schedule_pipe_to_drain(scope, source);
    }
}

fn claim_initial_terminal_condition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
) -> bool {
    let source = owner.source(scope);
    let destination = owner.destination(scope);
    let source_state = readable_stream_snapshot(scope, source).state();
    let destination_snapshot = writable_stream_snapshot(scope, destination);
    let destination_observation = PipeDestinationObservation::new(
        destination_snapshot.state(),
        destination_snapshot.close_requested(),
    );
    let endpoint_trigger =
        match owner
            .state(scope)
            .plan_initial_terminal(PipeEndpointObservation::new(
                source_state,
                destination_observation,
            )) {
            PipeInitialTerminalPlan::Stop => return true,
            PipeInitialTerminalPlan::Continue => return false,
            PipeInitialTerminalPlan::Claim(trigger) => trigger,
        };
    let (trigger, reason) = match endpoint_trigger {
        PipeEndpointTerminalTrigger::SourceErrored => (
            PipeTerminalTrigger::SourceErrored,
            Some(
                readable_stream_error(scope, source)
                    .unwrap_or_else(|| readable_stream_queue_error_value(scope)),
            ),
        ),
        PipeEndpointTerminalTrigger::DestinationErrored => (
            PipeTerminalTrigger::DestinationErrored,
            Some(
                writable_stream_stored_error(scope, destination)
                    .unwrap_or_else(|| v8::undefined(scope).into()),
            ),
        ),
        PipeEndpointTerminalTrigger::SourceClosed => (PipeTerminalTrigger::SourceClosed, None),
        PipeEndpointTerminalTrigger::DestinationClosed => (
            PipeTerminalTrigger::DestinationClosed,
            Some(destination_closed_error(scope)),
        ),
    };
    claim_pipe_shutdown_with_destination(scope, owner, trigger, reason, destination_observation);
    true
}

fn destination_closed_error<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(scope, v8str(scope, "Destination stream closed"))
}

pub(in crate::streams::adapter) fn source_pipe_errored<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    if let Some(owner) = PipeOwner::from_source(scope, source) {
        claim_pipe_shutdown(
            scope,
            owner,
            PipeTerminalTrigger::SourceErrored,
            Some(reason),
        );
    }
}

pub(in crate::streams::adapter) fn destination_pipe_errored<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_value: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let owner = PipeOwner::from_value(owner_value.into())
        .expect("writable pipe registration must contain a PipeOwner");
    claim_pipe_shutdown(
        scope,
        owner,
        PipeTerminalTrigger::DestinationErrored,
        Some(reason),
    );
}

fn claim_pipe_shutdown<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    trigger: PipeTerminalTrigger,
    reason: Option<v8::Local<'s, v8::Value>>,
) {
    if !owner.state(scope).is_active() {
        return;
    }
    let destination = owner.destination(scope);
    let snapshot = writable_stream_snapshot(scope, destination);
    let destination = PipeDestinationObservation::new(snapshot.state(), snapshot.close_requested());
    claim_pipe_shutdown_with_destination(scope, owner, trigger, reason, destination);
}

fn claim_pipe_shutdown_with_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    trigger: PipeTerminalTrigger,
    reason: Option<v8::Local<'s, v8::Value>>,
    destination: PipeDestinationObservation,
) {
    let state = owner.state(scope);
    let transition = state.transition_shutdown(PipeShutdownEvent::ClaimTerminal {
        trigger,
        destination,
    });
    if matches!(transition.admission(), PipeAdmission::Ignored) {
        return;
    }
    if let Some(reason) = reason {
        owner.set_shutdown_reason(scope, reason);
    }
    owner.apply(scope, transition);
    perform_pipe_shutdown_command(scope, owner, transition.command());
}

fn perform_pipe_shutdown_command<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    command: Option<PipeShutdownCommand>,
) {
    match command {
        None => {}
        Some(PipeShutdownCommand::WaitForLastWrite) => {
            wait_for_pipe_last_write(scope, owner);
        }
        Some(PipeShutdownCommand::RunActions { action }) => {
            run_pipe_shutdown_actions(scope, owner, action);
        }
        Some(PipeShutdownCommand::EnqueueFinalize) => {
            let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
                scope,
                v8::Function::builder(pipe_finalize_callback).data(owner.object().into()),
                "pipe finalization job",
            ) else {
                return;
            };
            scope.enqueue_microtask(callback);
        }
        Some(PipeShutdownCommand::Finalize { finalization }) => {
            finalize_pipe_owner(scope, owner, finalization);
        }
    }
}

fn wait_for_pipe_last_write<'s>(scope: &mut v8::PinScope<'s, '_>, owner: PipeOwner<'s>) {
    let promise = owner.last_write(scope).unwrap_or_else(|| {
        let value = resolved_promise_value(scope, v8::undefined(scope).into())
            .expect("pipe shutdown must create its empty write barrier");
        v8::Local::<v8::Promise>::try_from(value)
            .expect("pipe empty write barrier must be a promise")
    });
    // WriteQueuedChunks waits for the latest write *and ignores its outcome*,
    // producing a new fulfilled promise before invoking the shutdown action.
    let StreamOwnerPublication::Published(settled) = publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(pipe_ignore_last_write_settlement_callback),
        "pipe last write ignored fulfillment",
        v8::Function::builder(pipe_ignore_last_write_settlement_callback),
        "pipe last write ignored rejection",
        "pipe last write outcome normalization",
    ) else {
        return;
    };
    suppress_promise_unhandled_rejection(scope, settled.into());
    publish_required_stream_promise_reactions(
        scope,
        settled,
        v8::Function::builder(pipe_last_write_settled_callback).data(owner.object().into()),
        "pipe last write fulfillment",
        v8::Function::builder(pipe_last_write_settled_callback).data(owner.object().into()),
        "pipe last write rejection",
        "pipe last write shutdown barrier",
    )
    .finish_at_owner_boundary();
}

fn pipe_finalize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner =
        PipeOwner::from_value(args.data()).expect("pipe finalization job must retain its owner");
    let transition = owner
        .state(scope)
        .transition_shutdown(PipeShutdownEvent::Finalize);
    owner.apply(scope, transition);
    perform_pipe_shutdown_command(scope, owner, transition.command());
    rv.set_undefined();
}

fn pipe_ignore_last_write_settlement_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

fn pipe_last_write_settled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner =
        PipeOwner::from_value(args.data()).expect("pipe last-write reaction must retain its owner");
    let transition = owner
        .state(scope)
        .transition_shutdown(PipeShutdownEvent::LastWriteSettled);
    owner.apply(scope, transition);
    perform_pipe_shutdown_command(scope, owner, transition.command());
    rv.set_undefined();
}

fn run_pipe_shutdown_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    action: PipeShutdownAction,
) {
    match action {
        PipeShutdownAction::AbortDestination => {
            run_pipe_abort_destination(scope, owner, PipeShutdownOperation::Destination);
        }
        PipeShutdownAction::CancelSource => {
            run_pipe_cancel_source(scope, owner, PipeShutdownOperation::Source);
        }
        PipeShutdownAction::CloseDestination => {
            run_pipe_close_destination(scope, owner, PipeShutdownOperation::Destination);
        }
        PipeShutdownAction::AbortDestinationAndCancelSource => {
            run_pipe_abort_destination(scope, owner, PipeShutdownOperation::Destination);
            run_pipe_cancel_source(scope, owner, PipeShutdownOperation::Source);
        }
    }
}

fn run_pipe_abort_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    operation: PipeShutdownOperation,
) {
    let destination = owner.destination(scope);
    let plan = owner.state(scope).plan_shutdown_operation(
        PipeShutdownOperationObservation::AbortDestination {
            destination_state: writable_stream_snapshot(scope, destination).state(),
        },
    );
    if matches!(plan, PipeShutdownOperationPlan::FulfillWithoutRunning) {
        pipe_shutdown_action_fulfilled(scope, owner, operation);
        return;
    }
    let reason = owner
        .shutdown_reason(scope)
        .expect("pipe destination abort must retain the original reason");
    let result = writable_stream_abort_internal(scope, destination, reason)
        .expect("pipe destination abort must return a promise");
    attach_pipe_shutdown_action_reactions(scope, owner, operation, result);
}

fn run_pipe_cancel_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    operation: PipeShutdownOperation,
) {
    let source = owner.source(scope);
    let plan = owner.state(scope).plan_shutdown_operation(
        PipeShutdownOperationObservation::CancelSource {
            source_state: readable_stream_snapshot(scope, source).state(),
        },
    );
    if matches!(plan, PipeShutdownOperationPlan::FulfillWithoutRunning) {
        pipe_shutdown_action_fulfilled(scope, owner, operation);
        return;
    }
    let reason = owner
        .shutdown_reason(scope)
        .expect("pipe source cancel must retain the original reason");
    let result = cancel_readable_stream(scope, source, reason)
        .expect("pipe source cancel must return a promise");
    attach_pipe_shutdown_action_reactions(scope, owner, operation, result);
}

fn run_pipe_close_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    operation: PipeShutdownOperation,
) {
    let destination = owner.destination(scope);
    let result = super::writable::writable_stream_close_with_error_propagation(scope, destination);
    attach_pipe_shutdown_action_reactions(scope, owner, operation, result);
}

fn attach_pipe_shutdown_action_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    operation: PipeShutdownOperation,
    result: v8::Local<'s, v8::Value>,
) {
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) else {
        pipe_shutdown_action_fulfilled(scope, owner, operation);
        return;
    };
    suppress_promise_unhandled_rejection(scope, promise.into());
    let fulfilled_data = pipe_shutdown_action_reaction_data(scope, owner, operation);
    let rejected_data = pipe_shutdown_action_reaction_data(scope, owner, operation);
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(pipe_shutdown_action_fulfilled_callback).data(fulfilled_data.into()),
        "pipe shutdown action fulfillment",
        v8::Function::builder(pipe_shutdown_action_rejected_callback).data(rejected_data.into()),
        "pipe shutdown action rejection",
        "pipe shutdown action",
    )
    .finish_at_owner_boundary();
}

fn pipe_shutdown_action_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    operation: PipeShutdownOperation,
) -> v8::Local<'s, v8::Array> {
    let data = v8::Array::new(scope, 2);
    data.set_index(scope, 0, owner.object().into())
        .expect("pipe shutdown reaction owner must publish");
    let operation = match operation {
        PipeShutdownOperation::Destination => 0,
        PipeShutdownOperation::Source => 1,
    };
    data.set_index(
        scope,
        1,
        v8::Integer::new_from_unsigned(scope, operation).into(),
    )
    .expect("pipe shutdown reaction operation must publish");
    data
}

fn decode_pipe_shutdown_action_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(PipeOwner<'s>, PipeShutdownOperation)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    let owner = data.get_index(scope, 0).and_then(PipeOwner::from_value)?;
    let operation = match data.get_index(scope, 1)?.uint32_value(scope)? {
        0 => PipeShutdownOperation::Destination,
        1 => PipeShutdownOperation::Source,
        _ => return None,
    };
    Some((owner, operation))
}

fn pipe_shutdown_action_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let (owner, operation) = decode_pipe_shutdown_action_reaction_data(scope, args.data())
        .expect("pipe action fulfillment must retain owner and operation");
    pipe_shutdown_action_fulfilled(scope, owner, operation);
    rv.set_undefined();
}

fn pipe_shutdown_action_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    operation: PipeShutdownOperation,
) {
    let transition = owner
        .state(scope)
        .transition_shutdown(PipeShutdownEvent::ActionFulfilled { operation });
    owner.apply(scope, transition);
    perform_pipe_shutdown_command(scope, owner, transition.command());
}

fn pipe_shutdown_action_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let (owner, operation) = decode_pipe_shutdown_action_reaction_data(scope, args.data())
        .expect("pipe action rejection must retain owner and operation");
    let transition = owner
        .state(scope)
        .transition_shutdown(PipeShutdownEvent::ActionRejected { operation });
    if matches!(transition.admission(), PipeAdmission::Ignored) {
        rv.set_undefined();
        return;
    }
    owner.set_shutdown_action_error(scope, operation, args.get(0));
    owner.apply(scope, transition);
    perform_pipe_shutdown_command(scope, owner, transition.command());
    rv.set_undefined();
}

fn attach_pipe_write_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    promise: v8::Local<'s, v8::Promise>,
) {
    publish_required_stream_promise_reactions(
        scope,
        promise,
        v8::Function::builder(pipe_write_fulfilled_callback).data(owner.object().into()),
        "pipe write fulfillment",
        v8::Function::builder(pipe_write_rejected_callback).data(owner.object().into()),
        "pipe write rejection",
        "pipe write",
    )
    .finish_at_owner_boundary();
}

fn pipe_write_fulfilled_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

fn pipe_write_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner =
        PipeOwner::from_value(args.data()).expect("pipe write rejection must retain its owner");
    claim_pipe_shutdown(
        scope,
        owner,
        PipeTerminalTrigger::DestinationErrored,
        Some(args.get(0)),
    );
    rv.set_undefined();
}

pub(in crate::streams) fn new_readable_stream_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    destination: v8::Local<'s, v8::Object>,
    options: PipeOptions,
) -> Option<(v8::Local<'s, v8::Promise>, v8::Local<'s, v8::Object>)> {
    let (promise, pending) = new_pending_read_promise(scope)?;
    let owner = PipeOwner::new(scope, source, destination, pending, options);
    Some((promise, owner.object()))
}

pub(in crate::streams) fn register_readable_stream_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    set_required_stream_slot_value(
        scope,
        source,
        READABLE_STREAM_PIPE_OWNER_SLOT,
        owner.into(),
        "readable pipe owner registration",
    );
}

pub(in crate::streams) fn install_readable_stream_pipe_to_abort_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    signal: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = PipeOwner::from_source(scope, source) else {
        return;
    };
    if pipe_to_abort_signal_aborted(scope, signal) {
        let reason = pipe_to_abort_signal_reason(scope, signal);
        claim_pipe_shutdown(scope, owner, PipeTerminalTrigger::Aborted, Some(reason));
        return;
    }

    let StreamOwnerPublication::Published(listener) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_stream_pipe_to_abort_signal_callback)
            .data(owner.object().into())
            .length(1),
        "pipe abort listener",
    ) else {
        return;
    };
    assert!(
        register_pipe_to_abort_algorithm(scope, signal, listener),
        "a validated pipe AbortSignal must accept its internal algorithm"
    );
    owner.set_abort_registration(scope, signal, listener);
    apply_owner_mutation(scope, owner, PipeOwnerMutation::AbortListenerRegistered);
}

fn readable_stream_pipe_to_abort_signal_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner =
        PipeOwner::from_value(args.data()).expect("pipe abort listener must retain its owner");
    if !owner.state(scope).is_active() {
        rv.set_undefined();
        return;
    }
    apply_owner_mutation(scope, owner, PipeOwnerMutation::AbortDispatchStarted);
    let reason = if args.length() > 0 {
        args.get(0)
    } else {
        let (signal, _) = owner
            .abort_registration(scope)
            .expect("registered pipe abort listener must retain its signal");
        pipe_to_abort_signal_reason(scope, signal)
    };
    claim_pipe_shutdown(scope, owner, PipeTerminalTrigger::Aborted, Some(reason));
    rv.set_undefined();
}

fn register_pipe_to_abort_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    algorithm: v8::Local<'s, v8::Function>,
) -> bool {
    ResolvedAbortSignal::resolve(scope, signal)
        .is_some_and(|signal| signal.register_algorithm(scope, algorithm))
}

fn unregister_pipe_to_abort_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    algorithm: v8::Local<'s, v8::Function>,
) -> bool {
    ResolvedAbortSignal::resolve(scope, signal)
        .is_some_and(|signal| signal.unregister_algorithm(scope, algorithm))
}

fn pipe_to_abort_signal_aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> bool {
    ResolvedAbortSignal::resolve(scope, signal).is_some_and(|signal| signal.is_aborted(scope))
}

fn pipe_to_abort_signal_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    if let Some(reason) =
        ResolvedAbortSignal::resolve(scope, signal).and_then(|signal| signal.reason(scope))
    {
        return reason;
    }
    signal
        .get(scope, moli_v8_util::v8str(scope, "reason").into())
        .unwrap_or_else(|| {
            crate::dom_exception::new_dom_exception_value(
                scope,
                "The operation was aborted.",
                "AbortError",
            )
        })
}

fn finalize_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    finalization: PipeFinalization,
) {
    let source = owner.source(scope);
    let destination = owner.destination(scope);
    let result_resolver = owner.result_resolver(scope);

    match finalization.abort_cleanup() {
        PipeAbortCleanup::Unregister => {
            let (signal, listener) = owner
                .abort_registration(scope)
                .expect("a registered pipe abort listener must retain its registration");
            assert!(
                unregister_pipe_to_abort_algorithm(scope, signal, listener),
                "a registered pipe abort algorithm must be removable"
            );
        }
        PipeAbortCleanup::None | PipeAbortCleanup::ClearDispatching => {}
    }
    owner.clear_abort_registration(scope);

    clear_source_pipe_owner(scope, source, owner);
    super::writable::clear_writable_stream_pipe_owner(scope, destination, owner.object());
    unlock_readable_stream(scope, source);
    set_writable_stream_locked(scope, destination, false);

    match finalization.settlement() {
        PipeFinalSettlement::Resolve => {
            resolve_pending_promise(scope, result_resolver, v8::undefined(scope).into());
        }
        PipeFinalSettlement::RejectOriginal => {
            let reason = owner
                .shutdown_reason(scope)
                .expect("a rejecting pipe finalization must retain its reason");
            reject_pending_read(scope, result_resolver, reason);
        }
        PipeFinalSettlement::RejectShutdownAction { operation } => {
            let reason = owner
                .shutdown_action_error(scope, operation)
                .expect("a rejected pipe shutdown action must retain its reason");
            reject_pending_read(scope, result_resolver, reason);
        }
    }
    owner.clear_residences(scope);
}

fn clear_source_pipe_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    owner: PipeOwner<'s>,
) {
    let is_current = stream_slot_object(scope, source, READABLE_STREAM_PIPE_OWNER_SLOT)
        .is_some_and(|current| current.strict_equals(owner.object().into()));
    if is_current {
        set_required_stream_slot_value(
            scope,
            source,
            READABLE_STREAM_PIPE_OWNER_SLOT,
            v8::null(scope).into(),
            "readable pipe owner cleanup",
        );
    }
}

fn apply_owner_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: PipeOwner<'s>,
    mutation: PipeOwnerMutation,
) {
    let transition = owner.state(scope).transition_mutation(mutation);
    owner.apply(scope, transition);
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;

    #[test]
    fn old_owner_cannot_clear_source_registration_for_new_owner() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let source = v8::Object::new(scope);
        let old_owner_object = v8::Object::new(scope);
        let new_owner_object = v8::Object::new(scope);
        let old_owner = PipeOwner::from_value(old_owner_object.into()).expect("old owner object");
        let new_owner = PipeOwner::from_value(new_owner_object.into()).expect("new owner object");

        set_stream_slot_value(
            scope,
            source,
            READABLE_STREAM_PIPE_OWNER_SLOT,
            new_owner_object.into(),
        );

        clear_source_pipe_owner(scope, source, old_owner);
        let registered = stream_slot_object(scope, source, READABLE_STREAM_PIPE_OWNER_SLOT)
            .expect("new owner registration must remain");
        assert!(registered.strict_equals(new_owner_object.into()));

        clear_source_pipe_owner(scope, source, new_owner);
        assert!(stream_slot_object(scope, source, READABLE_STREAM_PIPE_OWNER_SLOT).is_none());
    }

    #[test]
    fn old_owner_cannot_clear_destination_registration_for_new_owner() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let destination = v8::Object::new(scope);
        let old_owner = v8::Object::new(scope);
        let new_owner = v8::Object::new(scope);
        set_stream_slot_value(
            scope,
            destination,
            WRITABLE_STREAM_PIPE_OWNER_SLOT,
            new_owner.into(),
        );

        super::super::writable::clear_writable_stream_pipe_owner(scope, destination, old_owner);
        let registered = stream_slot_object(scope, destination, WRITABLE_STREAM_PIPE_OWNER_SLOT)
            .expect("new destination owner registration must remain");
        assert!(registered.strict_equals(new_owner.into()));

        super::super::writable::clear_writable_stream_pipe_owner(scope, destination, new_owner);
        assert!(stream_slot_object(scope, destination, WRITABLE_STREAM_PIPE_OWNER_SLOT).is_none());
    }
}
