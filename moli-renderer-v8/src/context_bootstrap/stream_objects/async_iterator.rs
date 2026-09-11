use super::*;
use crate::util::materialize_hidden_function_template_prototype;
use crate::web_api_interfaces;
use moli_streams::readable::iterator::{
    IteratorLifecycle, IteratorNextOutcome, IteratorOperationKind, IteratorPumpPlan, IteratorState,
    IteratorTransition,
};
use moli_webapi_declare::WebApiFunctionTemplate;

const ITERATOR_OPERATION_KIND_INDEX: u32 = 0;
const ITERATOR_OPERATION_VALUE_INDEX: u32 = 1;
const ITERATOR_OPERATION_PENDING_INDEX: u32 = 2;
const ITERATOR_OPERATION_OWNER_INDEX: u32 = 3;
const ITERATOR_OPERATION_NEXT: u32 = 0;
const ITERATOR_OPERATION_RETURN: u32 = 1;

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::ReadableStreamAsyncIterator,
    intrinsic_prototype_parent = v8::Intrinsic::AsyncIteratorPrototype,
    prototype_to_string_tag = "ReadableStream AsyncIterator",
    readonly_prototype,
    enumerable
)]
struct ReadableStreamAsyncIteratorPrototypeDeclaration {
    #[webapi(
        method,
        name = "next",
        callback = readable_stream_async_iterator_next_callback,
        length = 0
    )]
    next: (),
    #[webapi(
        method,
        name = "return",
        callback = readable_stream_async_iterator_return_callback,
        length = 1
    )]
    return_method: (),
}

pub(in crate::context_bootstrap) fn readable_stream_async_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) =
        get_private_value(scope, global, READABLE_STREAM_ASYNC_ITERATOR_PROTOTYPE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }

    let template = ReadableStreamAsyncIteratorPrototypeDeclaration::build(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;

    set_private_value(
        scope,
        global,
        READABLE_STREAM_ASYNC_ITERATOR_PROTOTYPE_SLOT,
        prototype.into(),
    );
    Some(prototype)
}

pub(in crate::context_bootstrap) fn readable_stream_async_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    enqueue_iterator_operation(
        scope,
        args.this(),
        IteratorOperationKind::Next,
        v8::undefined(scope).into(),
        &mut rv,
    );
}

fn enqueue_iterator_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    kind: IteratorOperationKind,
    value: v8::Local<'s, v8::Value>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(queue) = stream_slot_array(scope, iterator, READABLE_STREAM_ITERATOR_OPERATIONS_SLOT)
    else {
        reject_iterator_method(scope, rv);
        return;
    };
    let (promise, pending) =
        new_pending_read_promise(scope).expect("async iterator operation promise must be created");
    let operation = v8::Array::new(scope, 4);
    let kind = match kind {
        IteratorOperationKind::Next => ITERATOR_OPERATION_NEXT,
        IteratorOperationKind::Return => ITERATOR_OPERATION_RETURN,
    };
    let _ = operation.set_index(
        scope,
        ITERATOR_OPERATION_KIND_INDEX,
        v8::Integer::new_from_unsigned(scope, kind).into(),
    );
    let _ = operation.set_index(scope, ITERATOR_OPERATION_VALUE_INDEX, value);
    let _ = operation.set_index(scope, ITERATOR_OPERATION_PENDING_INDEX, pending.into());
    let _ = operation.set_index(scope, ITERATOR_OPERATION_OWNER_INDEX, iterator.into());
    let _ = queue.set_index(scope, queue.length(), operation.into());
    pump_readable_stream_async_iterator(scope, iterator);
    rv.set(promise.into());
}

fn reject_iterator_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let error = v8::Exception::type_error(
        scope,
        v8str(
            scope,
            "ReadableStream async iterator method called on incompatible receiver",
        ),
    );
    if let Some(promise) = rejected_promise_value(scope, error) {
        rv.set(promise);
    }
}

fn iterator_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) -> IteratorState {
    let lifecycle = if stream_slot_bool(scope, iterator, READABLE_STREAM_ITERATOR_CLOSED_SLOT)
        .unwrap_or(false)
    {
        IteratorLifecycle::Closed
    } else if stream_slot_bool(scope, iterator, READABLE_STREAM_ITERATOR_RETURNING_SLOT)
        .unwrap_or(false)
    {
        IteratorLifecycle::Returning
    } else {
        IteratorLifecycle::Active
    };
    IteratorState::new(
        lifecycle,
        stream_slot_bool(
            scope,
            iterator,
            READABLE_STREAM_ITERATOR_OPERATION_ACTIVE_SLOT,
        )
        .unwrap_or(false),
    )
}

fn apply_iterator_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    transition: IteratorTransition,
) {
    assert_eq!(
        iterator_state(scope, iterator),
        transition.source(),
        "async iterator transition source must match live owner state"
    );
    let next = transition.next();
    set_stream_slot_bool(
        scope,
        iterator,
        READABLE_STREAM_ITERATOR_CLOSED_SLOT,
        matches!(next.lifecycle(), IteratorLifecycle::Closed),
    );
    set_stream_slot_bool(
        scope,
        iterator,
        READABLE_STREAM_ITERATOR_RETURNING_SLOT,
        matches!(next.lifecycle(), IteratorLifecycle::Returning),
    );
    set_stream_slot_bool(
        scope,
        iterator,
        READABLE_STREAM_ITERATOR_OPERATION_ACTIVE_SLOT,
        next.operation_active(),
    );
}

fn current_iterator_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, iterator, READABLE_STREAM_ITERATOR_OPERATIONS_SLOT)?
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn iterator_operation_kind(
    scope: &mut v8::PinScope<'_, '_>,
    operation: v8::Local<'_, v8::Array>,
) -> Option<IteratorOperationKind> {
    match operation
        .get_index(scope, ITERATOR_OPERATION_KIND_INDEX)?
        .uint32_value(scope)?
    {
        ITERATOR_OPERATION_NEXT => Some(IteratorOperationKind::Next),
        ITERATOR_OPERATION_RETURN => Some(IteratorOperationKind::Return),
        _ => None,
    }
}

fn iterator_operation_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    operation
        .get_index(scope, ITERATOR_OPERATION_OWNER_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn iterator_operation_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    operation
        .get_index(scope, ITERATOR_OPERATION_PENDING_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("async iterator queue head must retain its promise residence")
}

fn iterator_operation_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Value> {
    operation
        .get_index(scope, ITERATOR_OPERATION_VALUE_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn iterator_operation_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    operation: v8::Local<'s, v8::Array>,
) -> bool {
    current_iterator_operation(scope, iterator)
        .is_some_and(|current| current.strict_equals(operation.into()))
}

fn shift_iterator_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    operation: v8::Local<'s, v8::Array>,
) {
    assert!(
        iterator_operation_is_current(scope, iterator, operation),
        "only the current async iterator operation may leave the queue"
    );
    let queue = stream_slot_array(scope, iterator, READABLE_STREAM_ITERATOR_OPERATIONS_SLOT)
        .expect("async iterator must retain its operation queue");
    let next = v8::Array::new(scope, 0);
    for index in 1..queue.length() {
        if let Some(entry) = queue.get_index(scope, index) {
            let _ = next.set_index(scope, next.length(), entry);
        }
    }
    set_stream_slot_value(
        scope,
        iterator,
        READABLE_STREAM_ITERATOR_OPERATIONS_SLOT,
        next.into(),
    );
}

fn pump_readable_stream_async_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) {
    loop {
        let operation = current_iterator_operation(scope, iterator);
        let head = operation.and_then(|operation| iterator_operation_kind(scope, operation));
        match iterator_state(scope, iterator).plan_pump(head) {
            IteratorPumpPlan::Idle | IteratorPumpPlan::WaitForInFlight => return,
            IteratorPumpPlan::StartNext(transition) => {
                let operation = operation.expect("next plan requires a queue head");
                apply_iterator_transition(scope, iterator, transition);
                start_iterator_next(scope, iterator, operation);
                return;
            }
            IteratorPumpPlan::StartReturn(transition) => {
                let operation = operation.expect("return plan requires a queue head");
                apply_iterator_transition(scope, iterator, transition);
                start_iterator_return(scope, iterator, operation);
                return;
            }
            IteratorPumpPlan::ResolveClosedNext => {
                let operation = operation.expect("closed next plan requires a queue head");
                let pending = iterator_operation_pending(scope, operation);
                shift_iterator_operation(scope, iterator, operation);
                let result = iter_result(scope, v8::undefined(scope).into(), true);
                resolve_pending_promise(scope, pending, result.into());
            }
            IteratorPumpPlan::ResolveClosedReturn => {
                let operation = operation.expect("closed return plan requires a queue head");
                let pending = iterator_operation_pending(scope, operation);
                let value = iterator_operation_value(scope, operation);
                shift_iterator_operation(scope, iterator, operation);
                let result = iter_result(scope, value, true);
                resolve_pending_promise(scope, pending, result.into());
            }
        }
    }
}

fn start_iterator_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    operation: v8::Local<'s, v8::Array>,
) {
    let reader = stream_slot_object(scope, iterator, READABLE_STREAM_ITERATOR_READER_SLOT)
        .expect("an active async iterator must retain its reader");
    let stream = stream_slot_object(scope, reader, READABLE_STREAM_READER_STREAM_SLOT)
        .expect("an active async iterator reader must retain its stream");
    let prepared = prepare_read_from_stream_as_promise(scope, stream)
        .expect("async iterator internal read promise must be created");
    if matches!(
        publish_required_stream_promise_reactions(
            scope,
            prepared.promise(),
            v8::Function::builder(readable_stream_async_iterator_next_fulfilled_callback)
                .data(operation.into()),
            "readable async iterator next fulfillment",
            v8::Function::builder(readable_stream_async_iterator_next_rejected_callback)
                .data(operation.into()),
            "readable async iterator next rejection",
            "readable async iterator next",
        ),
        StreamOwnerPublication::OwnerTerminating
    ) {
        return;
    }
    if prepared.pull_after_attach() {
        maybe_pull_stream(scope, stream);
    }
}

fn readable_stream_async_iterator_next_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(operation) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(iterator) = iterator_operation_owner(scope, operation) else {
        rv.set_undefined();
        return;
    };
    if !iterator_operation_is_current(scope, iterator, operation) {
        rv.set_undefined();
        return;
    }
    let Ok(read_result) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        let error = v8::Exception::type_error(
            scope,
            v8str(
                scope,
                "ReadableStream async iterator read result is invalid",
            ),
        );
        settle_iterator_next_rejected(scope, iterator, operation, error);
        rv.set_undefined();
        return;
    };
    let done = read_result
        .get(scope, v8str(scope, "done").into())
        .is_some_and(|done| done.boolean_value(scope));
    let value = read_result
        .get(scope, v8str(scope, "value").into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let transition = iterator_state(scope, iterator).plan_next_settlement(if done {
        IteratorNextOutcome::Done
    } else {
        IteratorNextOutcome::Chunk
    });
    let pending = iterator_operation_pending(scope, operation);
    let result = iter_result(scope, value, done);
    apply_iterator_transition(scope, iterator, transition);
    shift_iterator_operation(scope, iterator, operation);
    if done {
        release_readable_stream_async_iterator_reader(scope, iterator);
    }
    resolve_pending_promise(scope, pending, result.into());
    schedule_readable_stream_async_iterator_pump(scope, iterator);
    rv.set_undefined();
}

fn readable_stream_async_iterator_next_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(operation) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(iterator) = iterator_operation_owner(scope, operation) else {
        rv.set_undefined();
        return;
    };
    if iterator_operation_is_current(scope, iterator, operation) {
        settle_iterator_next_rejected(scope, iterator, operation, args.get(0));
    }
    rv.set_undefined();
}

fn settle_iterator_next_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    operation: v8::Local<'s, v8::Array>,
    reason: v8::Local<'s, v8::Value>,
) {
    let transition =
        iterator_state(scope, iterator).plan_next_settlement(IteratorNextOutcome::Rejected);
    let pending = iterator_operation_pending(scope, operation);
    apply_iterator_transition(scope, iterator, transition);
    shift_iterator_operation(scope, iterator, operation);
    release_readable_stream_async_iterator_reader(scope, iterator);
    reject_pending_read(scope, pending, reason);
    schedule_readable_stream_async_iterator_pump(scope, iterator);
}

fn release_readable_stream_async_iterator_reader<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) {
    if let Some(reader) = stream_slot_object(scope, iterator, READABLE_STREAM_ITERATOR_READER_SLOT)
    {
        release_readable_stream_reader(scope, reader);
    }
    set_stream_slot_value(
        scope,
        iterator,
        READABLE_STREAM_ITERATOR_READER_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(in crate::context_bootstrap) fn readable_stream_async_iterator_return_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = if args.length() > 0 {
        args.get(0)
    } else {
        v8::undefined(scope).into()
    };
    enqueue_iterator_operation(
        scope,
        args.this(),
        IteratorOperationKind::Return,
        value,
        &mut rv,
    );
}

fn start_iterator_return<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    operation: v8::Local<'s, v8::Array>,
) {
    let prevent_cancel = stream_slot_bool(
        scope,
        iterator,
        READABLE_STREAM_ITERATOR_PREVENT_CANCEL_SLOT,
    )
    .unwrap_or(false);
    if prevent_cancel {
        release_readable_stream_async_iterator_reader(scope, iterator);
        settle_iterator_return(scope, iterator, operation, None);
        return;
    }
    let reader = stream_slot_object(scope, iterator, READABLE_STREAM_ITERATOR_READER_SLOT)
        .expect("a returning async iterator must retain its reader");
    let stream = stream_slot_object(scope, reader, READABLE_STREAM_READER_STREAM_SLOT)
        .expect("a returning async iterator reader must retain its stream");
    let reason = iterator_operation_value(scope, operation);
    let cancel_result = cancel_readable_stream(scope, stream, reason)
        .expect("async iterator cancel must return a promise");
    release_readable_stream_async_iterator_reader(scope, iterator);
    let cancel_promise = v8::Local::<v8::Promise>::try_from(cancel_result)
        .expect("async iterator cancel result must be a promise");
    publish_required_stream_promise_reactions(
        scope,
        cancel_promise,
        v8::Function::builder(readable_stream_async_iterator_return_fulfilled_callback)
            .data(operation.into()),
        "readable async iterator return fulfillment",
        v8::Function::builder(readable_stream_async_iterator_return_rejected_callback)
            .data(operation.into()),
        "readable async iterator return rejection",
        "readable async iterator return",
    )
    .finish_at_owner_boundary();
}

fn readable_stream_async_iterator_return_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(operation) = v8::Local::<v8::Array>::try_from(args.data())
        && let Some(iterator) = iterator_operation_owner(scope, operation)
        && iterator_operation_is_current(scope, iterator, operation)
    {
        settle_iterator_return(scope, iterator, operation, None);
    }
    rv.set_undefined();
}

fn readable_stream_async_iterator_return_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(operation) = v8::Local::<v8::Array>::try_from(args.data())
        && let Some(iterator) = iterator_operation_owner(scope, operation)
        && iterator_operation_is_current(scope, iterator, operation)
    {
        settle_iterator_return(scope, iterator, operation, Some(args.get(0)));
    }
    rv.set_undefined();
}

fn settle_iterator_return<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    operation: v8::Local<'s, v8::Array>,
    rejection: Option<v8::Local<'s, v8::Value>>,
) {
    let transition = iterator_state(scope, iterator).plan_return_settlement();
    let pending = iterator_operation_pending(scope, operation);
    let value = iterator_operation_value(scope, operation);
    apply_iterator_transition(scope, iterator, transition);
    shift_iterator_operation(scope, iterator, operation);
    if let Some(reason) = rejection {
        reject_pending_read(scope, pending, reason);
    } else {
        let result = iter_result(scope, value, true);
        resolve_pending_promise(scope, pending, result.into());
    }
    schedule_readable_stream_async_iterator_pump(scope, iterator);
}

fn schedule_readable_stream_async_iterator_pump<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) {
    let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_stream_async_iterator_pump_callback).data(iterator.into()),
        "readable async iterator operation continuation",
    ) else {
        return;
    };
    scope.enqueue_microtask(callback);
}

fn readable_stream_async_iterator_pump_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(iterator) = v8::Local::<v8::Object>::try_from(args.data()) {
        pump_readable_stream_async_iterator(scope, iterator);
    }
    rv.set_undefined();
}
