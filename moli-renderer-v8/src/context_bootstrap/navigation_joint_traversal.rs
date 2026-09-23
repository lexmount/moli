//! A joint step is admitted as a unit. No participant may independently move
//! the shared cursor while its siblings are still running navigation script.

use super::history_runtime::{apply, traversal};
use super::navigation_entry::{history_entries, history_index, navigation_current_entry};
use super::navigation_events::{
    NavigationDispatchOutcome, dispatch_beforeunload_for_runtime_owner,
    dispatch_navigation_traverse_event_with_outcome, dispatch_pagehide_for_runtime_owner,
    dispatch_unload_for_runtime_owner,
};
use super::navigation_result::navigation_dom_exception;
use super::navigation_seed::history_entry_seed_for_traversal;
use super::navigation_traversal_execution::{
    TraversalTarget, queue_history_traversal_without_result,
};
use super::navigation_traversal_plan::JointTraversalPlan;
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_has_opaque_origin,
    navigation_document_is_active, window_history_for_holder, window_navigation_for_holder,
};
use crate::native_bridge::PendingNavigationResult;
use crate::util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value};
use moli_session_history::SessionHistoryStepId;

const PENDING: &str = "__lmPendingJointTraversal";
const ACTIVE: &str = "__lmJointActive";
const OWNER: &str = "__lmJointOwner";
const STEP: &str = "__lmJointStep";
const ADMISSION: &str = "__lmJointAdmission";
const PARTICIPANTS: &str = "__lmJointParticipants";
const REMAINING: &str = "__lmJointRemaining";
const COMMITTED: &str = "__lmJointCommitted";
const FINISHED: &str = "__lmJointFinished";
const HISTORY: &str = "__lmJointHistory";
const SOURCE: &str = "__lmJointSource";
const DESTINATION: &str = "__lmJointDestination";
const NAVIGATION: &str = "__lmJointNavigation";
const SIGNAL: &str = "__lmJointSignal";
const EVENT: &str = "__lmJointEvent";
const INTERCEPTED: &str = "__lmJointIntercepted";
const INTERCEPT_RESULT: &str = "__lmJointInterceptResult";
const INTERCEPT_ERROR: &str = "__lmJointInterceptError";

pub(super) fn queue_plan<'s>(scope: &mut v8::PinScope<'s, '_>, mut plan: JointTraversalPlan<'s>) {
    // A root Document replacement owns the whole descendant tree and must be
    // performed by its browser/popup loader. Otherwise queue the step on its caller, never
    // select one changing frame as a substitute for the joint operation.
    if let Some(index) = plan.cross_document_root_index(scope) {
        queue_history_traversal_without_result(scope, plan.targets.remove(index));
        return;
    }
    let Some(history) = window_history_for_holder(scope, plan.owner) else {
        return;
    };
    let index = history_index(scope, history);
    queue_history_traversal_without_result(
        scope,
        TraversalTarget {
            owner: plan.owner,
            history,
            current_index: index,
            target_index: index,
            joint_step: Some(plan.step()),
        },
    );
}

pub(super) fn requires_joint_execution<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    plan: &JointTraversalPlan<'s>,
) -> bool {
    match plan.targets.as_slice() {
        [target] => history_entry_seed_for_traversal(
            scope,
            target.owner,
            target.current_index,
            target.target_index,
        )
        .is_some(),
        _ => true,
    }
}

fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, data, key).and_then(|value| value.try_into().ok())
}

fn array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, data, key).and_then(|value| value.try_into().ok())
}

fn active<'s>(scope: &mut v8::PinScope<'s, '_>, data: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, data, ACTIVE).is_some_and(|value| value.is_true())
}

fn entry_signature<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<String> {
    let entry = history_entries(scope, history)?
        .get_index(scope, index)?
        .try_into()
        .ok()?;
    let entry = super::session_history::entry_reference(scope, entry)?;
    serde_json::to_string(&(entry.key.as_str(), entry.document.as_str())).ok()
}

fn signature_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    key: &str,
    signature: Option<String>,
) -> bool {
    let stored = get_private_value(scope, data, key)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    stored.is_some() && stored == signature
}

/// All participants and their source/destination identities are captured
/// before the first callback. Precommit promises delay the whole operation.
pub(super) fn execute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    plan: JointTraversalPlan<'s>,
    initiator_info: Option<v8::Local<'s, v8::Value>>,
    results: &[PendingNavigationResult],
) {
    let data = v8::Object::new(scope);
    set_private_value(scope, data, ACTIVE, v8::Boolean::new(scope, true).into());
    set_private_value(scope, data, OWNER, plan.owner.into());
    set_private_value(
        scope,
        data,
        STEP,
        v8::BigInt::new_from_u64(scope, plan.step().raw()).into(),
    );
    let signature = super::session_history::traversal_admission_signature(&plan.core);
    set_private_value(
        scope,
        data,
        ADMISSION,
        v8::String::new(scope, &signature).unwrap().into(),
    );
    let participants = v8::Array::new(scope, plan.targets.len() as i32);
    set_private_value(scope, data, PARTICIPANTS, participants.into());
    let (committed, finished) = traversal::pending_result_resolver_arrays(scope, results);
    set_private_value(scope, data, COMMITTED, committed.into());
    set_private_value(scope, data, FINISHED, finished.into());
    for (index, target) in plan.targets.iter().enumerate() {
        let participant = v8::Object::new(scope);
        let _ = participants.set_index(scope, index as u32, participant.into());
        set_private_value(scope, participant, HISTORY, target.history.into());
        if !navigation_document_has_opaque_origin(scope, target.owner)
            && let Some(navigation) = window_navigation_for_holder(scope, target.owner)
        {
            set_private_value(scope, participant, NAVIGATION, navigation.into());
        }
        for (slot, index) in [
            (SOURCE, target.current_index),
            (DESTINATION, target.target_index),
        ] {
            let Some(signature) = entry_signature(scope, target.history, index) else {
                abort(scope, data, None);
                return;
            };
            set_private_value(
                scope,
                participant,
                slot,
                v8::String::new(scope, &signature).unwrap().into(),
            );
        }
    }
    let mut precommit = Vec::new();
    for (index, target) in plan.targets.iter().enumerate() {
        let participant = participants
            .get_index(scope, index as u32)
            .unwrap()
            .try_into()
            .unwrap();
        if history_entry_seed_for_traversal(
            scope,
            target.owner,
            target.current_index,
            target.target_index,
        )
        .is_some()
        {
            dispatch_beforeunload_for_runtime_owner(scope, target.owner);
        }
        if validate(scope, data).is_none() {
            abort(scope, data, None);
            return;
        }
        // Only the initiating Navigation owns this API method's info.
        let participant_info =
            initiator_info.filter(|_| target.owner.strict_equals(plan.owner.into()));
        let outcome = object(scope, participant, NAVIGATION)
            .map(|navigation| {
                dispatch_navigation_traverse_event_with_outcome(
                    scope,
                    navigation,
                    target.history,
                    target.target_index,
                    participant_info,
                )
            })
            .unwrap_or_else(NavigationDispatchOutcome::proceed);
        for value in [outcome.precommit_result, outcome.intercept_result]
            .into_iter()
            .flatten()
        {
            if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
                super::navigation_result::suppress_unhandled_rejection(scope, promise);
            }
        }
        for (slot, value) in [
            (SIGNAL, outcome.signal.map(Into::into)),
            (EVENT, outcome.precommit_event.map(Into::into)),
            (INTERCEPT_RESULT, outcome.intercept_result),
            (INTERCEPT_ERROR, outcome.intercept_error),
        ] {
            if let Some(value) = value {
                set_private_value(scope, participant, slot, value);
            }
        }
        set_private_value(
            scope,
            participant,
            INTERCEPTED,
            v8::Boolean::new(scope, outcome.intercepted).into(),
        );
        if !outcome.proceed
            || outcome.abort_error.is_some()
            || outcome.precommit_error.is_some()
            || validate(scope, data).is_none()
        {
            super::navigation_events::mark_navigation_outcome_default_prevented(scope, &outcome);
            abort(scope, data, outcome.abort_error.or(outcome.precommit_error));
            return;
        }
        if let Some(promise) = outcome.precommit_result {
            precommit.push(promise);
        }
    }
    if precommit.is_empty() {
        commit(scope, data);
        return;
    }
    set_private_value(
        scope,
        data,
        REMAINING,
        v8::Integer::new_from_unsigned(scope, precommit.len() as u32).into(),
    );
    for index in 0..participants.length() {
        let participant = participants
            .get_index(scope, index)
            .unwrap()
            .try_into()
            .unwrap();
        if let Some(navigation) = object(scope, participant, NAVIGATION) {
            cancel_pending(scope, navigation);
            set_private_value(scope, navigation, PENDING, data.into());
        }
    }
    for value in precommit {
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            abort(scope, data, None);
            return;
        };
        let _ = resolver.resolve(scope, value);
        let promise = resolver.get_promise(scope);
        let fulfilled = v8::Function::builder(precommit_fulfilled)
            .data(data.into())
            .build(scope)
            .unwrap();
        let rejected = v8::Function::builder(precommit_rejected)
            .data(data.into())
            .build(scope)
            .unwrap();
        let _ = promise.then2(scope, fulfilled, rejected);
    }
}

fn validate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> Option<JointTraversalPlan<'s>> {
    if !active(scope, data) {
        return None;
    }
    let owner = object(scope, data, OWNER)?;
    if !navigation_document_is_active(scope, owner) {
        return None;
    }
    let step = get_private_value(scope, data, STEP)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())?;
    let plan = JointTraversalPlan::resolve(
        scope,
        owner,
        SessionHistoryStepId::from_raw(step.u64_value().0),
    )?;
    let signature = get_private_value(scope, data, ADMISSION)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    if signature != super::session_history::traversal_admission_signature(&plan.core) {
        return None;
    }
    let participants = array(scope, data, PARTICIPANTS)?;
    if participants.length() as usize != plan.targets.len() {
        return None;
    }
    for (index, target) in plan.targets.iter().enumerate() {
        let participant = participants
            .get_index(scope, index as u32)?
            .try_into()
            .ok()?;
        if !navigation_document_is_active(scope, target.owner)
            || !object(scope, participant, HISTORY)?.strict_equals(target.history.into())
        {
            return None;
        }
        let source = entry_signature(scope, target.history, target.current_index);
        let destination = entry_signature(scope, target.history, target.target_index);
        if !signature_matches(scope, participant, SOURCE, source)
            || !signature_matches(scope, participant, DESTINATION, destination)
        {
            return None;
        }
        if let Some(signal) = object(scope, participant, SIGNAL)
            && let Some(host) = context_host_ptr_from_global_bridge(scope)
            && unsafe { &mut *host }.abort_signal_aborted(scope, signal)
        {
            return None;
        }
    }
    Some(plan)
}

fn deactivate<'s>(scope: &mut v8::PinScope<'s, '_>, data: v8::Local<'s, v8::Object>) {
    set_private_value(scope, data, ACTIVE, v8::Boolean::new(scope, false).into());
    if let Some(participants) = array(scope, data, PARTICIPANTS) {
        for index in 0..participants.length() {
            let Some(participant) = participants
                .get_index(scope, index)
                .and_then(|value| value.try_into().ok())
            else {
                continue;
            };
            if let Some(navigation) = object(scope, participant, NAVIGATION)
                && object(scope, navigation, PENDING)
                    .is_some_and(|pending| pending.strict_equals(data.into()))
            {
                set_private_value(scope, navigation, PENDING, v8::undefined(scope).into());
            }
        }
    }
}

fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    if !active(scope, data) {
        return;
    }
    deactivate(scope, data);
    let error = error.unwrap_or_else(|| {
        navigation_dom_exception(scope, "Joint history traversal was canceled", "AbortError")
    });
    for slot in [COMMITTED, FINISHED] {
        if let Some(resolvers) = array(scope, data, slot) {
            traversal::reject_resolver_array(scope, resolvers, error, slot == FINISHED);
        }
    }
    if let Some(participants) = array(scope, data, PARTICIPANTS) {
        for index in 0..participants.length() {
            let Some(participant) = participants
                .get_index(scope, index)
                .and_then(|value| value.try_into().ok())
            else {
                continue;
            };
            if let Some(signal) = object(scope, participant, SIGNAL)
                && let Some(host) = context_host_ptr_from_global_bridge(scope)
            {
                unsafe { &mut *host }.abort_signal(scope, signal, error);
            }
            if let Some(navigation) = object(scope, participant, NAVIGATION) {
                super::navigation_lifecycle::finish_navigation_error_events(
                    scope, navigation, error, "",
                );
            }
        }
    }
}

pub(super) fn cancel_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(data) = object(scope, navigation, PENDING) else {
        return false;
    };
    if !active(scope, data) {
        return false;
    }
    abort(scope, data, None);
    true
}

fn precommit_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if !active(scope, data) {
        return;
    }
    let Some(remaining) =
        get_private_value(scope, data, REMAINING).and_then(|value| value.uint32_value(scope))
    else {
        return;
    };
    if remaining <= 1 {
        commit(scope, data);
    } else {
        set_private_value(
            scope,
            data,
            REMAINING,
            v8::Integer::new_from_unsigned(scope, remaining - 1).into(),
        );
    }
}

fn precommit_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) {
        abort(scope, data, Some(args.get(0)));
    }
}

fn commit<'s>(scope: &mut v8::PinScope<'s, '_>, data: v8::Local<'s, v8::Object>) {
    let Some(plan) = validate(scope, data) else {
        abort(scope, data, None);
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        abort(scope, data, None);
        return;
    };
    let mut cross_document = Vec::new();
    for target in &plan.targets {
        if let Some((url, mut seed)) = history_entry_seed_for_traversal(
            scope,
            target.owner,
            target.current_index,
            target.target_index,
        ) {
            let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, target.owner)
            else {
                abort(scope, data, None);
                return;
            };
            seed.session_history.target_step = Some(plan.step());
            seed.session_history.admitted_entry = history_entries(scope, target.history)
                .and_then(|entries| entries.get_index(scope, target.target_index))
                .and_then(|entry| entry.try_into().ok())
                .and_then(|entry| super::session_history::entry_reference(scope, entry));
            cross_document.push((target.owner, handle, url, seed));
        }
    }
    // Unload is script too. Finish it for every participant and validate the
    // entire plan again before scheduling any replacement or changing a view.
    for (owner, _, _, _) in &cross_document {
        dispatch_pagehide_for_runtime_owner(scope, *owner);
        if validate(scope, data).is_none() {
            abort(scope, data, None);
            return;
        }
        dispatch_unload_for_runtime_owner(scope, *owner);
        if validate(scope, data).is_none() {
            abort(scope, data, None);
            return;
        }
    }
    let mut prepared = Vec::new();
    for (index, target) in plan.targets.iter().enumerate() {
        if cross_document
            .iter()
            .any(|(owner, _, _, _)| owner.strict_equals(target.owner.into()))
        {
            continue;
        }
        let Some(entry) =
            apply::prepare_local_history_entry_commit(scope, target.history, target.target_index)
        else {
            abort(scope, data, None);
            return;
        };
        prepared.push((index as u32, entry));
    }
    for (_, handle, url, seed) in &cross_document {
        let host = unsafe { &mut *host_ptr };
        let _ =
            host.mark_current_child_document_unload_dispatched_after_navigation_traversal(*handle);
        if !host.queue_deferred_child_browsing_context_navigation_from_entry_seed(
            *handle,
            url.as_str(),
            seed.clone(),
        ) {
            for (_, queued, _, _) in &cross_document {
                host.cancel_joint_child_history_navigation(*queued, plan.step());
            }
            abort(scope, data, None);
            return;
        }
    }
    // Acceptance point: all admissions passed, all child navigations have
    // reservations, and same-Document views can commit without script or failure.
    let Some(delta) = super::session_history::commit_traversal(scope, plan.owner, &plan.core)
    else {
        for (_, handle, _, _) in &cross_document {
            unsafe { &mut *host_ptr }.cancel_joint_child_history_navigation(*handle, plan.step());
        }
        abort(scope, data, None);
        return;
    };
    let applied = prepared
        .into_iter()
        .map(|(index, entry)| (index, apply::commit_prepared_history_entry(scope, entry)))
        .collect::<Vec<_>>();
    deactivate(scope, data);
    if delta != 0 {
        super::session_history::publish(
            scope,
            plan.owner,
            moli_page_types::SessionHistoryUpdateKind::Traverse { delta },
        );
    }
    let participants = array(scope, data, PARTICIPANTS).unwrap();
    let committed = array(scope, data, COMMITTED).unwrap();
    let finished = array(scope, data, FINISHED).unwrap();
    let resolved_entry = navigation_current_entry(scope, plan.owner)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let requester_replaces_document = cross_document
        .iter()
        .any(|(owner, _, _, _)| owner.strict_equals(plan.owner.into()));
    if !requester_replaces_document {
        traversal::resolve_resolver_array(scope, committed, resolved_entry);
    }
    let mut completions = Vec::new();
    for (index, entry) in &applied {
        let participant = participants
            .get_index(scope, *index)
            .unwrap()
            .try_into()
            .unwrap();
        let mut outcome = NavigationDispatchOutcome::proceed();
        outcome.intercepted =
            get_private_value(scope, participant, INTERCEPTED).is_some_and(|value| value.is_true());
        outcome.signal = object(scope, participant, SIGNAL);
        outcome.precommit_event = object(scope, participant, EVENT);
        outcome.intercept_result = get_private_value(scope, participant, INTERCEPT_RESULT);
        outcome.intercept_error = get_private_value(scope, participant, INTERCEPT_ERROR);
        let resolvers = if entry.owner.strict_equals(plan.owner.into()) {
            finished
        } else {
            v8::Array::new(scope, 0)
        };
        let settlement =
            traversal::prepare_joint_history_participant(scope, entry, &outcome, resolvers);
        completions.push((outcome, resolvers, settlement));
    }
    for (_, entry) in &applied {
        apply::dispatch_history_entry_currententrychange(scope, entry);
    }
    for ((_, entry), (outcome, resolvers, settlement)) in applied.iter().zip(completions) {
        traversal::finish_joint_history_participant(scope, entry, outcome, resolvers, settlement);
    }
    if !requester_replaces_document
        && !applied
            .iter()
            .any(|(_, entry)| entry.owner.strict_equals(plan.owner.into()))
    {
        traversal::resolve_resolver_array(scope, finished, resolved_entry);
    }
}
