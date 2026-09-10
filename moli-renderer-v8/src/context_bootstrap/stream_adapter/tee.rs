//! V8 storage adapter for the runtime-independent tee state machine.

use super::*;
use moli_streams::readable::ReadableState;
use moli_streams::tee::{
    BranchCancelPlan, BranchPair, ByteChunkAction, ByteCloseAction, ByteDistributionContinuation,
    ByteReadFailure, ByteReadFulfillmentPlan, ByteReadMode, ByteReadResultSnapshot,
    CancelSettlementPlan, DefaultChunkAction, DefaultDistributionContinuation,
    DefaultReadFulfillmentPlan, DefaultReadResultSnapshot, SourceClosePlan, TeeBranch,
    TeeBranchPullPlan, TeeBranchSnapshot, TeeEntryPlan, TeeEntrySnapshot, TeeKind, TeeSnapshot,
    TeeStartPlan, TerminalBranchAction,
};

const TEE_BRANCH_ALGORITHM_DATA_STATE_INDEX: u32 = 0;
const TEE_BRANCH_ALGORITHM_DATA_BRANCH_INDEX: u32 = 1;
const STREAM_TEE_BRANCH1_INDEX: u32 = 0;
const STREAM_TEE_BRANCH2_INDEX: u32 = 1;
const STREAM_TEE_ORIGINAL_INDEX: u32 = 2;
const STREAM_TEE_CANCEL_PROMISE_INDEX: u32 = 3;
const STREAM_TEE_CANCEL_PENDING_INDEX: u32 = 4;
const STREAM_TEE_CANCELED1_INDEX: u32 = 5;
const STREAM_TEE_CANCELED2_INDEX: u32 = 6;
const STREAM_TEE_REASON1_INDEX: u32 = 7;
const STREAM_TEE_REASON2_INDEX: u32 = 8;
const STREAM_TEE_CANCEL_SETTLED_INDEX: u32 = 9;
const STREAM_TEE_BYTE_STREAM_INDEX: u32 = 10;
const STREAM_TEE_READING_INDEX: u32 = 11;
const STREAM_TEE_BYOB_BRANCH_INDEX: u32 = 12;
const STREAM_TEE_READ_AGAIN1_INDEX: u32 = 13;
const STREAM_TEE_READ_AGAIN2_INDEX: u32 = 14;
const STREAM_TEE_CLONE_BRANCH2_INDEX: u32 = 15;
const TEE_READ_MICROTASK_STATE_INDEX: u32 = 0;
const TEE_READ_MICROTASK_CHUNK_INDEX: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap) enum TeeStartError {
    Locked,
    Unavailable,
}

pub(in crate::context_bootstrap) fn tee_readable_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Array>, TeeStartError> {
    tee_readable_stream_with_options(scope, stream, false)
}

/// Specification consumers such as Fetch clone the second branch's chunks.
/// The public ReadableStream.tee() method instead shares default-stream chunks.
pub(in crate::context_bootstrap) fn tee_readable_stream_with_cloned_branch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Array>, TeeStartError> {
    tee_readable_stream_with_options(scope, stream, true)
}

fn tee_readable_stream_with_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    clone_branch2: bool,
) -> Result<v8::Local<'s, v8::Array>, TeeStartError> {
    let relevant_context = stream
        .get_creation_context(scope)
        .ok_or(TeeStartError::Unavailable)?;
    let [branch1, branch2] = if relevant_context == scope.get_current_context() {
        tee_readable_stream_in_current_realm(scope, stream, clone_branch2)?
    } else {
        let stream = v8::Global::new(scope, stream);
        let (branch1, branch2) = {
            let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
            let stream = v8::Local::new(target_scope, &stream);
            let [branch1, branch2] =
                tee_readable_stream_in_current_realm(target_scope, stream, clone_branch2)?;
            (
                v8::Global::new(target_scope, branch1),
                v8::Global::new(target_scope, branch2),
            )
        };
        [
            v8::Local::new(scope, &branch1),
            v8::Local::new(scope, &branch2),
        ]
    };
    let result = v8::Array::new(scope, 2);
    let _ = result.set_index(scope, 0, branch1.into());
    let _ = result.set_index(scope, 1, branch2.into());
    Ok(result)
}

fn tee_readable_stream_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    clone_branch2: bool,
) -> Result<[v8::Local<'s, v8::Object>; 2], TeeStartError> {
    match TeeEntrySnapshot::new(readable_stream_locked(scope, stream)).plan() {
        TeeEntryPlan::RejectLocked => return Err(TeeStartError::Locked),
        TeeEntryPlan::Start => {}
    }

    let kind = TeeKind::from_byte_stream(readable_stream_is_byte_stream(scope, stream));
    let branch1 = if matches!(kind, TeeKind::Byte) {
        new_readable_byte_stream_object(scope)
    } else {
        new_readable_stream_object(scope, None, 1.0, None)
    };
    let branch2 = if matches!(kind, TeeKind::Byte) {
        new_readable_byte_stream_object(scope)
    } else {
        new_readable_stream_object(scope, None, 1.0, None)
    };
    let (cancel_promise, cancel_pending) =
        new_pending_read_promise(scope).ok_or(TeeStartError::Unavailable)?;
    let tee_state = v8::Array::new(scope, 16);
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CLONE_BRANCH2_INDEX,
        v8::Boolean::new(scope, clone_branch2).into(),
    );
    let _ = tee_state.set_index(scope, STREAM_TEE_BRANCH1_INDEX, branch1.into());
    let _ = tee_state.set_index(scope, STREAM_TEE_BRANCH2_INDEX, branch2.into());
    let _ = tee_state.set_index(scope, STREAM_TEE_ORIGINAL_INDEX, stream.into());
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCEL_PROMISE_INDEX,
        cancel_promise.into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCEL_PENDING_INDEX,
        cancel_pending.into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCELED1_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCELED2_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = tee_state.set_index(scope, STREAM_TEE_REASON1_INDEX, v8::undefined(scope).into());
    let _ = tee_state.set_index(scope, STREAM_TEE_REASON2_INDEX, v8::undefined(scope).into());
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCEL_SETTLED_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_BYTE_STREAM_INDEX,
        v8::Boolean::new(scope, matches!(kind, TeeKind::Byte)).into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_READING_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_BYOB_BRANCH_INDEX,
        v8::Integer::new_from_unsigned(scope, 0).into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_READ_AGAIN1_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_READ_AGAIN2_INDEX,
        v8::Boolean::new(scope, false).into(),
    );
    install_readable_stream_tee_branch_cancel_algorithm(scope, branch1, tee_state, 0);
    install_readable_stream_tee_branch_cancel_algorithm(scope, branch2, tee_state, 1);
    set_stream_slot_value(
        scope,
        stream,
        READABLE_STREAM_TEE_STATE_SLOT,
        tee_state.into(),
    );
    if !lock_readable_stream(scope, stream) {
        return Err(TeeStartError::Locked);
    }

    match readable_stream_tee_snapshot(scope, tee_state).plan_start() {
        TeeStartPlan::ErrorBranches => {
            if let Some(error) = readable_stream_error(scope, stream) {
                error_stream(scope, branch1, error);
                error_stream(scope, branch2, error);
            }
        }
        TeeStartPlan::CloseByteBranches => {
            close_readable_byte_stream_tee_branches(scope, tee_state);
        }
        TeeStartPlan::CloseDefaultBranches => {
            let _ = close_stream(scope, branch1);
            let _ = close_stream(scope, branch2);
            finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
            resolve_teed_readable_stream_cancel_promise(
                scope,
                tee_state,
                v8::undefined(scope).into(),
            );
        }
        TeeStartPlan::WaitForDefaultBranchStarts => {}
        TeeStartPlan::WaitForByteBranchDemand => {}
    }

    Ok([branch1, branch2])
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_tee_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, stream, READABLE_STREAM_TEE_STATE_SLOT)
}

fn readable_stream_tee_original<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    state
        .get_index(scope, STREAM_TEE_ORIGINAL_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn readable_stream_tee_branch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Array>,
    branch: TeeBranch,
) -> Option<v8::Local<'s, v8::Object>> {
    state
        .get_index(scope, tee_branch_slot(branch))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn tee_read_again(
    scope: &mut v8::PinScope<'_, '_>,
    state: v8::Local<'_, v8::Array>,
    branch: TeeBranch,
) -> bool {
    state
        .get_index(scope, tee_read_again_slot(branch))
        .is_some_and(|value| value.boolean_value(scope))
}

fn set_tee_read_again(
    scope: &mut v8::PinScope<'_, '_>,
    state: v8::Local<'_, v8::Array>,
    branch: TeeBranch,
    value: bool,
) {
    let _ = state.set_index(
        scope,
        tee_read_again_slot(branch),
        v8::Boolean::new(scope, value).into(),
    );
}

fn clear_tee_read_again(scope: &mut v8::PinScope<'_, '_>, state: v8::Local<'_, v8::Array>) {
    set_tee_read_again(scope, state, TeeBranch::First, false);
    set_tee_read_again(scope, state, TeeBranch::Second, false);
}

fn set_tee_reading(scope: &mut v8::PinScope<'_, '_>, state: v8::Local<'_, v8::Array>, value: bool) {
    let _ = state.set_index(
        scope,
        STREAM_TEE_READING_INDEX,
        v8::Boolean::new(scope, value).into(),
    );
}

pub(in crate::context_bootstrap::stream_adapter) fn readable_stream_tee_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Array>,
) -> TeeSnapshot {
    let kind = TeeKind::from_byte_stream(
        state
            .get_index(scope, STREAM_TEE_BYTE_STREAM_INDEX)
            .is_some_and(|value| value.boolean_value(scope)),
    );
    let source = readable_stream_tee_original(scope, state);
    let source_lifecycle = source.map(|source| readable_stream_snapshot(scope, source));
    let cancel_settled = state
        .get_index(scope, STREAM_TEE_CANCEL_SETTLED_INDEX)
        .is_some_and(|value| value.boolean_value(scope));
    let reading = state
        .get_index(scope, STREAM_TEE_READING_INDEX)
        .is_some_and(|value| value.boolean_value(scope));
    let byob_owner = match state
        .get_index(scope, STREAM_TEE_BYOB_BRANCH_INDEX)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0)
    {
        1 => Some(TeeBranch::First),
        2 => Some(TeeBranch::Second),
        _ => None,
    };
    TeeSnapshot::new(
        kind,
        source_lifecycle
            .map(|snapshot| snapshot.state())
            .unwrap_or(ReadableState::Closed),
        source_lifecycle.is_some_and(|snapshot| snapshot.close_requested()),
        BranchPair::new(
            tee_branch_snapshot(scope, state, TeeBranch::First),
            tee_branch_snapshot(scope, state, TeeBranch::Second),
        ),
        cancel_settled,
        reading,
        byob_owner,
    )
    .with_read_again(BranchPair::new(
        tee_read_again(scope, state, TeeBranch::First),
        tee_read_again(scope, state, TeeBranch::Second),
    ))
}

fn tee_branch_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Array>,
    branch: TeeBranch,
) -> TeeBranchSnapshot {
    let canceled = state
        .get_index(scope, tee_canceled_slot(branch))
        .is_some_and(|value| value.boolean_value(scope));
    let Some(stream) = readable_stream_tee_branch(scope, state, branch) else {
        return TeeBranchSnapshot::missing(canceled);
    };
    let lifecycle = readable_stream_snapshot(scope, stream);
    TeeBranchSnapshot::new(
        true,
        canceled,
        lifecycle.state(),
        lifecycle.close_requested(),
    )
}

pub(in crate::context_bootstrap::stream_adapter) fn close_teed_readable_stream_branches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let Some(state) = readable_stream_tee_state(scope, stream) else {
        return;
    };
    match readable_stream_tee_snapshot(scope, state).plan_source_close() {
        SourceClosePlan::WaitForReadReaction => {}
        SourceClosePlan::CloseDefaultBranches {
            branches,
            settle_cancel,
        } => {
            apply_terminal_branch_actions(scope, state, branches, None);
            if settle_cancel {
                resolve_teed_readable_stream_cancel_promise(
                    scope,
                    state,
                    v8::undefined(scope).into(),
                );
            }
        }
    }
}

pub(in crate::context_bootstrap::stream_adapter) fn error_teed_readable_stream_branches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(state) = readable_stream_tee_state(scope, stream) else {
        return;
    };
    let moli_streams::tee::SourceErrorPlan::ErrorBranches {
        branches,
        settle_cancel,
    } = readable_stream_tee_snapshot(scope, state).plan_source_error();
    apply_terminal_branch_actions(scope, state, branches, Some(reason));
    if settle_cancel {
        resolve_teed_readable_stream_cancel_promise(scope, state, v8::undefined(scope).into());
    }
}

fn apply_terminal_branch_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Array>,
    actions: BranchPair<TerminalBranchAction>,
    reason: Option<v8::Local<'s, v8::Value>>,
) {
    for branch in [TeeBranch::First, TeeBranch::Second] {
        let Some(stream) = readable_stream_tee_branch(scope, state, branch) else {
            continue;
        };
        match actions.get(branch) {
            TerminalBranchAction::Skip => {}
            TerminalBranchAction::Close => {
                let _ = close_stream(scope, stream);
                if let Err(error) = finish_byte_stream_tee_branch_close(scope, stream) {
                    error_stream(scope, stream, error);
                }
            }
            TerminalBranchAction::Error => {
                if let Some(reason) = reason {
                    error_stream(scope, stream, reason);
                }
            }
        }
    }
}

fn resolve_teed_readable_stream_cancel_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Array>,
    value: v8::Local<'s, v8::Value>,
) {
    if matches!(
        readable_stream_tee_snapshot(scope, state).plan_settle_cancel(),
        CancelSettlementPlan::AlreadySettled
    ) {
        return;
    }
    let Some(pending) = state
        .get_index(scope, STREAM_TEE_CANCEL_PENDING_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let _ = state.set_index(
        scope,
        STREAM_TEE_CANCEL_SETTLED_INDEX,
        v8::Boolean::new(scope, true).into(),
    );
    resolve_pending_promise(scope, pending, value);
}

const fn tee_branch_slot(branch: TeeBranch) -> u32 {
    match branch {
        TeeBranch::First => STREAM_TEE_BRANCH1_INDEX,
        TeeBranch::Second => STREAM_TEE_BRANCH2_INDEX,
    }
}

const fn tee_canceled_slot(branch: TeeBranch) -> u32 {
    match branch {
        TeeBranch::First => STREAM_TEE_CANCELED1_INDEX,
        TeeBranch::Second => STREAM_TEE_CANCELED2_INDEX,
    }
}

const fn tee_read_again_slot(branch: TeeBranch) -> u32 {
    match branch {
        TeeBranch::First => STREAM_TEE_READ_AGAIN1_INDEX,
        TeeBranch::Second => STREAM_TEE_READ_AGAIN2_INDEX,
    }
}

fn install_readable_stream_tee_branch_cancel_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    branch: v8::Local<'s, v8::Object>,
    tee_state: v8::Local<'s, v8::Array>,
    branch_index: u32,
) {
    let Some(controller) = stream_slot_object(scope, branch, READABLE_STREAM_CONTROLLER_SLOT)
    else {
        return;
    };
    let Some(algorithms) = stream_slot_array(scope, controller, STREAM_CONTROLLER_ALGORITHMS_SLOT)
    else {
        return;
    };
    let data = v8::Array::new(scope, 2);
    let _ = data.set_index(
        scope,
        TEE_BRANCH_ALGORITHM_DATA_STATE_INDEX,
        tee_state.into(),
    );
    let _ = data.set_index(
        scope,
        TEE_BRANCH_ALGORITHM_DATA_BRANCH_INDEX,
        v8::Integer::new_from_unsigned(scope, branch_index).into(),
    );
    let StreamOwnerPublication::Published(pull_algorithm) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_stream_tee_branch_pull_callback).data(data.into()),
        "tee branch pull algorithm",
    ) else {
        return;
    };
    let StreamOwnerPublication::Published(cancel_algorithm) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_stream_tee_branch_cancel_callback).data(data.into()),
        "tee branch cancel algorithm",
    ) else {
        return;
    };
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_SOURCE_INDEX,
        tee_state.into(),
    );
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_PULL_INDEX,
        pull_algorithm.into(),
    );
    let _ = algorithms.set_index(
        scope,
        READABLE_STREAM_ALGORITHM_CANCEL_INDEX,
        cancel_algorithm.into(),
    );
}

fn readable_stream_tee_branch_pull_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = v8::Local::<v8::Array>::try_from(args.data()).ok() else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(tee_state) = data
        .get_index(scope, TEE_BRANCH_ALGORITHM_DATA_STATE_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let branch = TeeBranch::from_index(
        data.get_index(scope, TEE_BRANCH_ALGORITHM_DATA_BRANCH_INDEX)
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(0),
    );
    match readable_stream_tee_snapshot(scope, tee_state).plan_branch_pull(branch) {
        TeeBranchPullPlan::RecordReadAgain { branch } => {
            set_tee_read_again(scope, tee_state, branch, true);
        }
        TeeBranchPullPlan::StartDefaultRead => {
            start_readable_default_stream_tee_read(scope, tee_state);
        }
        TeeBranchPullPlan::CloseBranches => {
            let byte_stream = tee_state
                .get_index(scope, STREAM_TEE_BYTE_STREAM_INDEX)
                .is_some_and(|value| value.boolean_value(scope));
            if byte_stream {
                close_readable_byte_stream_tee_branches(scope, tee_state);
            } else if let Some(original) = readable_stream_tee_original(scope, tee_state) {
                close_teed_readable_stream_branches(scope, original);
            }
        }
        TeeBranchPullPlan::InspectByteReadMode { branch } => {
            let has_pending_byob_view = readable_stream_tee_branch(scope, tee_state, branch)
                .is_some_and(|stream| {
                    readable_byte_stream_pending_byob_view(scope, stream).is_some()
                });
            let Some(start) = readable_stream_tee_snapshot(scope, tee_state)
                .plan_byte_read_start(branch, has_pending_byob_view)
            else {
                rv.set(v8::undefined(scope).into());
                return;
            };
            start_readable_byte_stream_tee_read(scope, tee_state, start.branch(), start.mode());
        }
        TeeBranchPullPlan::Ignore => {}
    }
    rv.set(v8::undefined(scope).into());
}

fn start_readable_default_stream_tee_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
) {
    let Some(original) = readable_stream_tee_original(scope, tee_state) else {
        return;
    };
    let StreamOwnerPublication::Published(chunk_steps) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_default_stream_tee_chunk_steps).data(tee_state.into()),
        "default tee read chunk steps",
    ) else {
        return;
    };
    let StreamOwnerPublication::Published(close_steps) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_default_stream_tee_close_steps).data(tee_state.into()),
        "default tee read close steps",
    ) else {
        return;
    };
    let StreamOwnerPublication::Published(error_steps) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_stream_tee_error_steps).data(tee_state.into()),
        "default tee read error steps",
    ) else {
        return;
    };
    let request = new_internal_read_request(scope, chunk_steps, close_steps, error_steps);
    set_tee_reading(scope, tee_state, true);
    if perform_read_from_stream(scope, original, request) {
        maybe_pull_stream(scope, original);
    }
}

fn readable_default_stream_tee_chunk_steps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(tee_state) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let data = v8::Array::new(scope, 2);
    let _ = data.set_index(scope, TEE_READ_MICROTASK_STATE_INDEX, tee_state.into());
    let _ = data.set_index(scope, TEE_READ_MICROTASK_CHUNK_INDEX, args.get(0));
    let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_default_stream_tee_chunk_microtask).data(data.into()),
        "default tee chunk microtask",
    ) else {
        rv.set_undefined();
        return;
    };
    scope.enqueue_microtask(callback);
    rv.set_undefined();
}

fn readable_default_stream_tee_chunk_microtask<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = v8::Local::<v8::Array>::try_from(args.data()).ok() else {
        rv.set_undefined();
        return;
    };
    let Some(tee_state) = data
        .get_index(scope, TEE_READ_MICROTASK_STATE_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let value = data
        .get_index(scope, TEE_READ_MICROTASK_CHUNK_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let plan = readable_stream_tee_snapshot(scope, tee_state)
        .plan_default_read_fulfilled(DefaultReadResultSnapshot::new(true, false));
    match plan {
        DefaultReadFulfillmentPlan::InvalidResult => unreachable!("validated tee read result"),
        DefaultReadFulfillmentPlan::CloseBranches { .. } => {
            unreachable!("chunk steps cannot produce a close plan")
        }
        DefaultReadFulfillmentPlan::Distribute { branches } => {
            // Enqueue may synchronously cause either branch controller to call
            // its pull algorithm. Keep this read in-flight and record that
            // demand instead of allowing a second source read to overlap.
            clear_tee_read_again(scope, tee_state);
            if !apply_default_tee_chunk_actions(scope, tee_state, branches, value) {
                set_tee_reading(scope, tee_state, false);
                rv.set_undefined();
                return;
            }
            let source_closed = readable_stream_tee_original(scope, tee_state)
                .is_some_and(|source| readable_stream_closed(scope, source));
            set_tee_reading(scope, tee_state, false);
            match readable_stream_tee_snapshot(scope, tee_state)
                .plan_after_default_distribution(source_closed)
            {
                DefaultDistributionContinuation::StartRead => {
                    start_readable_default_stream_tee_read(scope, tee_state);
                }
                DefaultDistributionContinuation::CloseBranches => {
                    if let Some(original) = readable_stream_tee_original(scope, tee_state) {
                        close_teed_readable_stream_branches(scope, original);
                    }
                }
                DefaultDistributionContinuation::Idle => {}
            }
        }
    }
    rv.set_undefined();
}

fn readable_default_stream_tee_close_steps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(tee_state) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let plan = readable_stream_tee_snapshot(scope, tee_state)
        .plan_default_read_fulfilled(DefaultReadResultSnapshot::new(true, true));
    let DefaultReadFulfillmentPlan::CloseBranches {
        branches,
        settle_cancel,
    } = plan
    else {
        unreachable!("default tee close steps require a close plan")
    };
    set_tee_reading(scope, tee_state, false);
    apply_terminal_branch_actions(scope, tee_state, branches, None);
    if settle_cancel {
        resolve_teed_readable_stream_cancel_promise(scope, tee_state, v8::undefined(scope).into());
    }
    rv.set_undefined();
}

fn apply_default_tee_chunk_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    actions: BranchPair<DefaultChunkAction>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let clone_branch2 = matches!(actions.get(TeeBranch::Second), DefaultChunkAction::Enqueue)
        && tee_state
            .get_index(scope, STREAM_TEE_CLONE_BRANCH2_INDEX)
            .is_some_and(|value| value.boolean_value(scope));
    let second_value = if clone_branch2 {
        let cloned = {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let mut scope = try_catch.init();
            structured_clone_value(&mut scope, value).ok_or_else(|| {
                scope
                    .exception()
                    .unwrap_or_else(|| v8::undefined(&scope).into())
            })
        };
        match cloned {
            Ok(cloned) => cloned,
            Err(error) => {
                error_default_tee_clone(scope, tee_state, error);
                return false;
            }
        }
    } else {
        value
    };
    for branch in [TeeBranch::First, TeeBranch::Second] {
        if !matches!(actions.get(branch), DefaultChunkAction::Enqueue) {
            continue;
        }
        let Some(stream) = readable_stream_tee_branch(scope, tee_state, branch) else {
            continue;
        };
        let chunk = if branch == TeeBranch::Second {
            second_value
        } else {
            value
        };
        match enqueue_chunk(scope, stream, chunk) {
            Ok(()) => maybe_pull_stream(scope, stream),
            Err(EnqueueChunkError::ClosedOrErrored) => {}
            Err(EnqueueChunkError::Strategy(error)) => {
                error_default_readable_stream_tee(scope, tee_state, error);
                return false;
            }
        }
    }
    true
}

fn error_default_tee_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    error: v8::Local<'s, v8::Value>,
) {
    let moli_streams::tee::SourceErrorPlan::ErrorBranches { branches, .. } =
        readable_stream_tee_snapshot(scope, tee_state).plan_source_error();
    apply_terminal_branch_actions(scope, tee_state, branches, Some(error));
    let Some(original) = readable_stream_tee_original(scope, tee_state) else {
        return;
    };
    let Some(pending) = tee_state
        .get_index(scope, STREAM_TEE_CANCEL_PENDING_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    // Source cancellation can synchronously close the stream. Reserve the
    // shared promise before that callback so it adopts the cancel result.
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCEL_SETTLED_INDEX,
        v8::Boolean::new(scope, true).into(),
    );
    let result = cancel_readable_stream(scope, original, error)
        .unwrap_or_else(|| v8::undefined(scope).into());
    resolve_pending_promise(scope, pending, result);
}

fn readable_stream_tee_error_steps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(tee_state) = v8::Local::<v8::Array>::try_from(args.data()) {
        set_tee_reading(scope, tee_state, false);
    }
    rv.set_undefined();
}

fn error_default_readable_stream_tee<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    error: v8::Local<'s, v8::Value>,
) {
    let plan = readable_stream_tee_snapshot(scope, tee_state).plan_source_error();
    let moli_streams::tee::SourceErrorPlan::ErrorBranches {
        branches,
        settle_cancel,
    } = plan;
    apply_terminal_branch_actions(scope, tee_state, branches, Some(error));
    if settle_cancel {
        resolve_teed_readable_stream_cancel_promise(scope, tee_state, v8::undefined(scope).into());
    }
}

fn start_readable_byte_stream_tee_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    branch: TeeBranch,
    mode: ByteReadMode,
) {
    let Some(original) = readable_stream_tee_original(scope, tee_state) else {
        return;
    };
    let Some(branch_stream) = readable_stream_tee_branch(scope, tee_state, branch) else {
        return;
    };
    let StreamOwnerPublication::Published(chunk_steps) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_byte_stream_tee_chunk_steps).data(tee_state.into()),
        "byte tee read chunk steps",
    ) else {
        return;
    };
    let StreamOwnerPublication::Published(close_steps) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_byte_stream_tee_close_steps).data(tee_state.into()),
        "byte tee read close steps",
    ) else {
        return;
    };
    let StreamOwnerPublication::Published(error_steps) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_stream_tee_error_steps).data(tee_state.into()),
        "byte tee read error steps",
    ) else {
        return;
    };
    let request = new_internal_read_request(scope, chunk_steps, close_steps, error_steps);
    let pull_after_attach = match mode {
        ByteReadMode::Byob => {
            let Some(view) = readable_byte_stream_pending_byob_view(scope, branch_stream) else {
                return;
            };
            let _ = tee_state.set_index(
                scope,
                STREAM_TEE_BYOB_BRANCH_INDEX,
                v8::Integer::new_from_unsigned(scope, branch.index() + 1).into(),
            );
            set_tee_reading(scope, tee_state, true);
            perform_read_into_byte_stream(scope, original, view, 1, request)
        }
        ByteReadMode::Default => {
            let _ = tee_state.set_index(
                scope,
                STREAM_TEE_BYOB_BRANCH_INDEX,
                v8::Integer::new_from_unsigned(scope, 0).into(),
            );
            set_tee_reading(scope, tee_state, true);
            perform_read_from_stream(scope, original, request)
        }
    };
    if pull_after_attach {
        maybe_pull_stream(scope, original);
    }
}

fn readable_byte_stream_tee_chunk_steps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(tee_state) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let data = v8::Array::new(scope, 2);
    let _ = data.set_index(scope, TEE_READ_MICROTASK_STATE_INDEX, tee_state.into());
    let _ = data.set_index(scope, TEE_READ_MICROTASK_CHUNK_INDEX, args.get(0));
    let StreamOwnerPublication::Published(callback) = build_required_stream_callback(
        scope,
        v8::Function::builder(readable_byte_stream_tee_chunk_microtask).data(data.into()),
        "byte tee chunk microtask",
    ) else {
        rv.set_undefined();
        return;
    };
    scope.enqueue_microtask(callback);
    rv.set_undefined();
}

fn readable_byte_stream_tee_chunk_microtask<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = v8::Local::<v8::Array>::try_from(args.data()).ok() else {
        rv.set_undefined();
        return;
    };
    let Some(tee_state) = data
        .get_index(scope, TEE_READ_MICROTASK_STATE_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let value = data
        .get_index(scope, TEE_READ_MICROTASK_CHUNK_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let bytes = value_buffer_source_bytes(scope, value);
    let plan = readable_stream_tee_snapshot(scope, tee_state).plan_byte_read_fulfilled(
        ByteReadResultSnapshot::new(true, false, !value.is_undefined(), bytes.is_some()),
    );
    match plan {
        ByteReadFulfillmentPlan::Error(failure) => {
            set_tee_reading(scope, tee_state, false);
            apply_byte_tee_read_failure(scope, tee_state, failure);
        }
        ByteReadFulfillmentPlan::CloseBranches { .. } => {
            unreachable!("byte tee chunk steps cannot produce a close plan")
        }
        ByteReadFulfillmentPlan::Distribute { branches } => {
            let Some(bytes) = bytes else {
                unreachable!("byte tee distribution requires validated bytes")
            };
            clear_tee_read_again(scope, tee_state);
            if !apply_byte_tee_chunk_actions(scope, tee_state, branches, value, &bytes) {
                set_tee_reading(scope, tee_state, false);
                rv.set_undefined();
                return;
            }
            let source_closed = readable_stream_tee_original(scope, tee_state)
                .is_some_and(|source| readable_stream_closed(scope, source));
            set_tee_reading(scope, tee_state, false);
            match readable_stream_tee_snapshot(scope, tee_state)
                .plan_after_byte_distribution(source_closed)
            {
                ByteDistributionContinuation::CloseBranches => {
                    close_readable_byte_stream_tee_branches(scope, tee_state);
                }
                ByteDistributionContinuation::PullBranch(branch) => {
                    if let Some(stream) = readable_stream_tee_branch(scope, tee_state, branch) {
                        maybe_pull_stream(scope, stream);
                    }
                }
                ByteDistributionContinuation::Idle => {}
            }
        }
    }
    rv.set_undefined();
}

fn readable_byte_stream_tee_close_steps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(tee_state) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let value = args.get(0);
    let plan = readable_stream_tee_snapshot(scope, tee_state).plan_byte_read_fulfilled(
        ByteReadResultSnapshot::new(true, true, !value.is_undefined(), false),
    );
    let ByteReadFulfillmentPlan::CloseBranches {
        branches,
        settle_cancel,
    } = plan
    else {
        unreachable!("byte tee close steps require a close plan")
    };
    set_tee_reading(scope, tee_state, false);
    apply_byte_tee_close_actions(scope, tee_state, branches, Some(value), settle_cancel);
    rv.set_undefined();
}

fn apply_byte_tee_read_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    failure: ByteReadFailure,
) {
    let message = match failure {
        ByteReadFailure::InvalidResult => "Byte tee read result is invalid",
        ByteReadFailure::MissingChunk => "Byte tee chunk is missing",
        ByteReadFailure::ChunkIsNotBytes => "Byte tee chunk is not bytes",
    };
    let error = v8::Exception::type_error(scope, v8str(scope, message));
    error_readable_byte_stream_tee(scope, tee_state, error);
}

fn apply_byte_tee_chunk_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    actions: BranchPair<ByteChunkAction>,
    original_view: v8::Local<'s, v8::Value>,
    bytes: &[u8],
) -> bool {
    for branch in [TeeBranch::First, TeeBranch::Second] {
        let Some(stream) = readable_stream_tee_branch(scope, tee_state, branch) else {
            continue;
        };
        match actions.get(branch) {
            ByteChunkAction::Skip => {}
            ByteChunkAction::RespondWithOriginalView => {
                if let Err(error) = respond_byte_stream_with_new_view(scope, stream, original_view)
                {
                    error_readable_byte_stream_tee(scope, tee_state, error);
                    return false;
                }
            }
            ByteChunkAction::EnqueueOriginalView => {
                match enqueue_byte_chunk(scope, stream, original_view) {
                    Ok(()) => maybe_pull_stream(scope, stream),
                    Err(EnqueueChunkError::ClosedOrErrored) => {}
                    Err(EnqueueChunkError::Strategy(error)) => {
                        error_readable_byte_stream_tee(scope, tee_state, error);
                        return false;
                    }
                }
            }
            ByteChunkAction::EnqueueClonedBytes => {
                let chunk = super::utils::require_internal_stream_value(
                    crate::context_bootstrap::shared::new_uint8_array_from_bytes(
                        scope,
                        bytes.to_vec(),
                    ),
                    "Uint8Array allocation",
                    "byte tee cloned branch chunk",
                );
                match enqueue_byte_chunk(scope, stream, chunk.into()) {
                    Ok(()) => maybe_pull_stream(scope, stream),
                    Err(EnqueueChunkError::ClosedOrErrored) => {}
                    Err(EnqueueChunkError::Strategy(error)) => {
                        error_readable_byte_stream_tee(scope, tee_state, error);
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn apply_byte_tee_close_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    actions: BranchPair<ByteCloseAction>,
    terminal_view: Option<v8::Local<'s, v8::Value>>,
    settle_cancel: bool,
) {
    for branch in [TeeBranch::First, TeeBranch::Second] {
        let Some(stream) = readable_stream_tee_branch(scope, tee_state, branch) else {
            continue;
        };
        let terminal_result = match actions.get(branch) {
            ByteCloseAction::Skip => continue,
            ByteCloseAction::CloseAndFinish => {
                let _ = close_stream(scope, stream);
                finish_byte_stream_tee_branch_close(scope, stream)
            }
            ByteCloseAction::CloseAndRespondWithView => {
                let _ = close_stream(scope, stream);
                let Some(view) = terminal_view else {
                    unreachable!("terminal BYOB close requires its result view")
                };
                respond_byte_stream_with_new_view(scope, stream, view)
            }
        };
        if let Err(error) = terminal_result {
            error_stream(scope, stream, error);
        }
    }
    if settle_cancel {
        resolve_teed_readable_stream_cancel_promise(scope, tee_state, v8::undefined(scope).into());
    }
}

fn close_readable_byte_stream_tee_branches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
) {
    let plan = readable_stream_tee_snapshot(scope, tee_state).plan_byte_close(None);
    apply_byte_tee_close_actions(
        scope,
        tee_state,
        plan.branches(),
        None,
        plan.settle_cancel(),
    );
}

fn error_readable_byte_stream_tee<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
    error: v8::Local<'s, v8::Value>,
) {
    let plan = readable_stream_tee_snapshot(scope, tee_state).plan_byte_read_rejected();
    for branch in [TeeBranch::First, TeeBranch::Second] {
        if matches!(plan.branches().get(branch), TerminalBranchAction::Error)
            && let Some(stream) = readable_stream_tee_branch(scope, tee_state, branch)
        {
            error_stream(scope, stream, error);
        }
    }
    if plan.settle_cancel() {
        resolve_teed_readable_stream_cancel_promise(scope, tee_state, v8::undefined(scope).into());
    }
}

fn readable_stream_tee_branch_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = v8::Local::<v8::Array>::try_from(args.data()).ok() else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(tee_state) = data
        .get_index(scope, TEE_BRANCH_ALGORITHM_DATA_STATE_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let branch = TeeBranch::from_index(
        data.get_index(scope, TEE_BRANCH_ALGORITHM_DATA_BRANCH_INDEX)
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(0),
    );
    let Some(cancel_promise) = tee_state.get_index(scope, STREAM_TEE_CANCEL_PROMISE_INDEX) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let reason = args.get(0);
    let plan = readable_stream_tee_snapshot(scope, tee_state).plan_branch_cancel(branch);
    let (canceled_index, reason_index) = match branch {
        TeeBranch::First => (STREAM_TEE_CANCELED1_INDEX, STREAM_TEE_REASON1_INDEX),
        TeeBranch::Second => (STREAM_TEE_CANCELED2_INDEX, STREAM_TEE_REASON2_INDEX),
    };
    let _ = tee_state.set_index(scope, canceled_index, v8::Boolean::new(scope, true).into());
    let _ = tee_state.set_index(scope, reason_index, reason);

    if matches!(plan, BranchCancelPlan::RecordReasonAndCancelSource) {
        resolve_readable_stream_tee_cancel_from_original(scope, tee_state);
    }
    rv.set(cancel_promise);
}

fn resolve_readable_stream_tee_cancel_from_original<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    tee_state: v8::Local<'s, v8::Array>,
) {
    if matches!(
        readable_stream_tee_snapshot(scope, tee_state).plan_settle_cancel(),
        moli_streams::tee::CancelSettlementPlan::AlreadySettled
    ) {
        return;
    }
    let Some(original) = readable_stream_tee_original(scope, tee_state) else {
        return;
    };
    let Some(pending) = tee_state
        .get_index(scope, STREAM_TEE_CANCEL_PENDING_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let composite_reason = v8::Array::new(scope, 2);
    let reason1 = tee_state
        .get_index(scope, STREAM_TEE_REASON1_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let reason2 = tee_state
        .get_index(scope, STREAM_TEE_REASON2_INDEX)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = composite_reason.set_index(scope, 0, reason1);
    let _ = composite_reason.set_index(scope, 1, reason2);
    let _ = tee_state.set_index(
        scope,
        STREAM_TEE_CANCEL_SETTLED_INDEX,
        v8::Boolean::new(scope, true).into(),
    );
    let cancel_result = cancel_readable_stream(scope, original, composite_reason.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    resolve_pending_promise(scope, pending, cancel_result);
}
