//! Admit, reserve and commit complete traversal plans, including zero or one participant.

use super::history_runtime::{apply, results, traversal};
use super::navigation_activation::{
    navigation_transition_matches_resolver, precommit_transition_resolver_from_event,
    take_navigation_transition_committed_resolver,
};
use super::navigation_entry::{history_entries, history_index, navigation_current_entry};
use super::navigation_events::{
    NavigationDispatchOutcome, dispatch_beforeunload_for_runtime_owner,
    dispatch_navigation_traverse_event_with_outcome,
};
use super::navigation_result::navigation_dom_exception;
use super::navigation_seed::history_entry_seed_for_traversal;
use super::navigation_traversal_execution::{
    TraversalTarget, queue_history_traversal_without_result,
};
use super::navigation_traversal_plan::JointTraversalPlan;
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_has_opaque_origin,
    navigation_document_is_active, window_history_for_holder, window_location_for_holder,
    window_navigation_for_holder, window_task_target_for_runtime_owner,
};
use crate::document_runtime::DomHandle;
use crate::native_bridge::history_traversal::{
    HistoryTraversalId, HistoryTraversalParticipant, PendingHistoryTraversalAdmission,
    TraversalParticipantOutcome,
};
use crate::native_bridge::{NavigationHistoryEntrySeed, PendingNavigationResult};
use crate::util::context_host_ptr_from_global_bridge;
use moli_session_history::SessionHistoryEntry;

pub(super) fn queue_plan<'s>(scope: &mut v8::PinScope<'s, '_>, mut plan: JointTraversalPlan<'s>) {
    // Root Document replacement belongs to the browser/popup loader.
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

fn entry_reference<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<SessionHistoryEntry> {
    let entries = history_entries(scope, history)?;
    let entry = entries.get(index as usize)?;
    Some(super::session_history::native_entry_reference(
        &entry.borrow(),
    ))
}

impl TraversalParticipantOutcome {
    fn capture(scope: &mut v8::PinScope<'_, '_>, outcome: &NavigationDispatchOutcome<'_>) -> Self {
        Self {
            intercepted: outcome.intercepted,
            destination: outcome.destination.map(|value| v8::Global::new(scope, value)),
            signal: outcome.signal.map(|value| v8::Global::new(scope, value)),
            event: outcome
                .precommit_event
                .map(|value| v8::Global::new(scope, value)),
            intercept_result: outcome
                .intercept_result
                .map(|value| v8::Global::new(scope, value)),
            intercept_error: outcome
                .intercept_error
                .map(|value| v8::Global::new(scope, value)),
        }
    }

    fn local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> NavigationDispatchOutcome<'s> {
        let mut outcome = NavigationDispatchOutcome::proceed();
        outcome.intercepted = self.intercepted;
        outcome.destination = self.destination.as_ref().map(|value| v8::Local::new(scope, value));
        outcome.signal = self
            .signal
            .as_ref()
            .map(|value| v8::Local::new(scope, value));
        outcome.precommit_event = self
            .event
            .as_ref()
            .map(|value| v8::Local::new(scope, value));
        outcome.intercept_result = self
            .intercept_result
            .as_ref()
            .map(|value| v8::Local::new(scope, value));
        outcome.intercept_error = self
            .intercept_error
            .as_ref()
            .map(|value| v8::Local::new(scope, value));
        outcome
    }
}

fn capture_participants<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    plan: &JointTraversalPlan<'s>,
) -> Option<Vec<HistoryTraversalParticipant>> {
    let host = unsafe { &*context_host_ptr_from_global_bridge(scope)? };
    plan.targets
        .iter()
        .map(|target| {
            let navigation = (!navigation_document_has_opaque_origin(scope, target.owner))
                .then(|| window_navigation_for_holder(scope, target.owner))
                .flatten();
            Some(HistoryTraversalParticipant {
                execution_owner: window_task_target_for_runtime_owner(scope, host, target.owner)?
                    .owner(),
                history: v8::Global::new(scope, target.history),
                navigation: navigation.map(|value| v8::Global::new(scope, value)),
                source: entry_reference(scope, target.history, target.current_index)?,
                destination: entry_reference(scope, target.history, target.target_index)?,
                outcome: Default::default(),
            })
        })
        .collect()
}

pub(super) fn execute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    plan: JointTraversalPlan<'s>,
    initiator_info: Option<v8::Local<'s, v8::Value>>,
    results: Vec<PendingNavigationResult>,
) {
    let captured = capture_participants(scope, &plan).and_then(|participants| {
        let host = unsafe { &*context_host_ptr_from_global_bridge(scope)? };
        let owner = window_task_target_for_runtime_owner(scope, host, plan.owner)?.owner();
        Some((participants, owner))
    });
    let Some((participants, execution_owner)) = captured else {
        let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
        results::reject_pending_navigation_results(scope, &results, error);
        return;
    };
    let mut admission = PendingHistoryTraversalAdmission {
        id: HistoryTraversalId::allocate(),
        active: true.into(),
        plan: plan.core,
        initiator: v8::Global::new(scope, plan.owner),
        execution_owner,
        participants,
        results,
        remaining_precommit: 0.into(),
    };
    let mut precommit = Vec::new();
    for (index, target) in plan.targets.iter().enumerate() {
        if history_entry_seed_for_traversal(
            scope,
            target.owner,
            target.current_index,
            target.target_index,
        )
        .is_some()
        {
            if let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, target.owner)
                && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
            {
                unsafe { &mut *host_ptr }.dispatch_child_document_tree_beforeunload_for_traversal(scope, handle);
            } else {
                dispatch_beforeunload_for_runtime_owner(scope, target.owner);
            }
        }
        if validate(scope, &admission).is_none() {
            abort(scope, &admission, None);
            return;
        }
        // Only the initiating Navigation owns this API method's info.
        let info = initiator_info.filter(|_| target.owner.strict_equals(plan.owner.into()));
        let navigation = admission.participants[index]
            .navigation
            .as_ref()
            .map(|value| v8::Local::new(scope, value));
        let outcome = navigation
            .map(|navigation| {
                dispatch_navigation_traverse_event_with_outcome(
                    scope,
                    navigation,
                    target.history,
                    target.target_index,
                    info,
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
        admission.participants[index].outcome =
            TraversalParticipantOutcome::capture(scope, &outcome);
        if !outcome.proceed
            || outcome.abort_error.is_some()
            || outcome.precommit_error.is_some()
            || validate(scope, &admission).is_none()
        {
            super::navigation_events::mark_navigation_outcome_default_prevented(scope, &outcome);
            abort(
                scope,
                &admission,
                outcome.abort_error.or(outcome.precommit_error),
            );
            return;
        }
        if let Some(promise) = outcome.precommit_result {
            precommit.push(promise);
        }
    }
    if precommit.is_empty() {
        commit(scope, &admission);
        return;
    }
    for participant in &admission.participants {
        if let Some(navigation) = &participant.navigation {
            let navigation = v8::Local::new(scope, navigation);
            cancel_pending(scope, navigation);
        }
    }
    if validate(scope, &admission).is_none() {
        abort(scope, &admission, None);
        return;
    }
    admission.remaining_precommit.set(precommit.len());
    let Some(host) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let id = unsafe { &mut *host }
        .pending_history_traversal_admissions
        .insert(admission);
    let data = v8::BigInt::new_from_u64(scope, id.raw());
    for value in precommit {
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            if let Some(admission) = unsafe { &mut *host }
                .pending_history_traversal_admissions
                .take(id)
            {
                abort(scope, &admission, None);
            }
            return;
        };
        let _ = resolver.resolve(scope, value);
        let fulfilled = v8::Function::builder(precommit_fulfilled)
            .data(data.into())
            .build(scope)
            .unwrap();
        let rejected = v8::Function::builder(precommit_rejected)
            .data(data.into())
            .build(scope)
            .unwrap();
        let _ = resolver
            .get_promise(scope)
            .then2(scope, fulfilled, rejected);
    }
}

fn validate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    admission: &PendingHistoryTraversalAdmission,
) -> Option<Vec<TraversalTarget<'s>>> {
    let owner = v8::Local::new(scope, &admission.initiator);
    if !admission.active.get() || !navigation_document_is_active(scope, owner) {
        return None;
    }
    let targets = super::session_history::project_traversal(scope, owner, &admission.plan)?;
    if targets.len() != admission.participants.len() {
        return None;
    }
    for (target, participant) in targets.iter().zip(&admission.participants) {
        let history = v8::Local::new(scope, &participant.history);
        if !navigation_document_is_active(scope, target.owner)
            || !history.strict_equals(target.history.into())
            || entry_reference(scope, history, target.current_index).as_ref()
                != Some(&participant.source)
            || entry_reference(scope, history, target.target_index).as_ref()
                != Some(&participant.destination)
        {
            return None;
        }
        if let Some(signal) = &participant.outcome.signal {
            let signal = v8::Local::new(scope, signal);
            let host = context_host_ptr_from_global_bridge(scope)?;
            if unsafe { &mut *host }.abort_signal_aborted(scope, signal) {
                return None;
            }
        }
    }
    Some(targets)
}

fn deactivate(scope: &mut v8::PinScope<'_, '_>, admission: &PendingHistoryTraversalAdmission) {
    admission.active.set(false);
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }
            .pending_history_traversal_admissions
            .take(admission.id);
    }
}

fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    admission: &PendingHistoryTraversalAdmission,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    if !admission.active.get() {
        return;
    }
    deactivate(scope, admission);
    settle_aborted_admission(scope, admission, error);
}

fn settle_aborted_admission<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    admission: &PendingHistoryTraversalAdmission,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    let canceled = error.is_none();
    let _execution = crate::script_cleanup::ScriptExecutionScope::enter(scope);
    let error = error.unwrap_or_else(|| {
        navigation_dom_exception(scope, "History traversal was canceled", "AbortError")
    });
    let mut results_rejected = false;
    for participant in &admission.participants {
        let navigation = participant.navigation.as_ref().map(|value| v8::Local::new(scope, value));
        let transition_resolver = participant.outcome.event.as_ref().and_then(|event| {
            let event = v8::Local::new(scope, event);
            precommit_transition_resolver_from_event(scope, event)
        });
        let committed_resolver = navigation.and_then(|navigation| {
            transition_resolver
                .filter(|resolver| navigation_transition_matches_resolver(scope, navigation, *resolver))
                .and_then(|_| take_navigation_transition_committed_resolver(scope, navigation))
        });
        if let Some(signal) = &participant.outcome.signal {
            let signal = v8::Local::new(scope, signal);
            crate::native_bridge::abort::abort_signal(scope, signal, error);
        }
        if !results_rejected {
            results::reject_pending_navigation_results(scope, &admission.results, error);
            results_rejected = true;
        }
        if let Some(navigation) = navigation {
            let filename = if canceled {
                let owner = super::navigation_window::runtime_window_owner(scope, navigation);
                window_location_for_holder(scope, owner)
                    .and_then(|location| {
                        super::location_runtime::location_href_slot(scope, location)
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };
            super::navigation_lifecycle::finish_navigation_error_events(
                scope, navigation, error, &filename,
            );
            if let Some(resolver) = committed_resolver {
                let _ = resolver.reject(scope, error);
            }
            super::navigation_lifecycle::settle_navigation_transition_finished_local(
                scope, navigation, transition_resolver, Some(error),
            );
        }
    }
    if !results_rejected {
        results::reject_pending_navigation_results(scope, &admission.results, error);
    }
}

pub(super) fn cancel_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let admission = unsafe { &mut *host }
        .pending_history_traversal_admissions
        .take_for_navigation(scope, navigation);
    let Some(admission) = admission else {
        return false;
    };
    abort(scope, &admission, None);
    true
}

fn callback_id(value: v8::Local<'_, v8::Value>) -> Option<HistoryTraversalId> {
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    Some(HistoryTraversalId::from_raw(value.u64_value().0))
}

/// The caller has removed and invalidated the entire batch under a short native
/// borrow. Neither extraction nor further native retirement happens around JS.
pub(crate) fn abort_history_traversal_admissions(
    scope: &mut v8::PinScope<'_, '_>,
    admissions: Vec<std::rc::Rc<PendingHistoryTraversalAdmission>>,
) {
    for admission in admissions {
        debug_assert!(!admission.active.get());
        settle_aborted_admission(scope, &admission, None);
    }
}

fn precommit_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(id) = callback_id(args.data()) else {
        return;
    };
    let Some(host) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let admission = unsafe { &mut *host }
        .pending_history_traversal_admissions
        .fulfill_precommit(id);
    if let Some(admission) = admission {
        commit(scope, &admission);
    }
}

fn precommit_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(id) = callback_id(args.data()) else {
        return;
    };
    let Some(host) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let admission = unsafe { &mut *host }
        .pending_history_traversal_admissions
        .take(id);
    if let Some(admission) = admission {
        abort(scope, &admission, Some(args.get(0)));
    }
}

struct CrossDocumentParticipant<'s> {
    owner: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    url: url::Url,
    seed: NavigationHistoryEntrySeed,
}

fn commit(scope: &mut v8::PinScope<'_, '_>, admission: &PendingHistoryTraversalAdmission) {
    for participant in &admission.participants {
        if let Some(event) = &participant.outcome.event {
            let event = v8::Local::new(scope, event);
            super::navigation_events::finish_navigation_precommit(scope, event);
        }
    }
    let Some(targets) = validate(scope, admission) else {
        abort(scope, admission, None);
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let owner = v8::Local::new(scope, &admission.initiator);
    let step = admission.plan.target_step();
    let mut cross_document = Vec::new();
    for target in &targets {
        if let Some((url, mut seed)) = history_entry_seed_for_traversal(
            scope,
            target.owner,
            target.current_index,
            target.target_index,
        ) {
            let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, target.owner)
            else {
                abort(scope, admission, None);
                return;
            };
            seed.session_history.target_step = Some(step);
            seed.session_history.admitted_entry =
                entry_reference(scope, target.history, target.target_index);
            cross_document.push(CrossDocumentParticipant {
                owner: target.owner,
                handle,
                url,
                seed,
            });
        }
    }
    let mut prepared = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        if cross_document
            .iter()
            .any(|participant| participant.owner.strict_equals(target.owner.into()))
        {
            continue;
        }
        let Some(entry) =
            apply::prepare_local_history_entry_commit(scope, target.history, target.target_index)
        else {
            abort(scope, admission, None);
            return;
        };
        prepared.push((index, entry));
    }
    for participant in &cross_document {
        let host = unsafe { &mut *host_ptr };
        if !host.queue_deferred_child_browsing_context_navigation_from_entry_seed(
            participant.handle,
            participant.url.as_str(),
            participant.seed.clone(),
            None,
        ) {
            for queued in &cross_document {
                host.cancel_joint_child_history_navigation(queued.handle, step);
            }
            abort(scope, admission, None);
            return;
        }
    }
    // Acceptance point: every admission and reservation passed. Installing all
    // same-Document views below runs no author callbacks and cannot fail.
    let Some(delta) = super::session_history::commit_traversal(scope, owner, &admission.plan)
    else {
        for participant in &cross_document {
            unsafe { &mut *host_ptr }
                .cancel_joint_child_history_navigation(participant.handle, step);
        }
        abort(scope, admission, None);
        return;
    };
    let applied = prepared
        .into_iter()
        .map(|(index, entry)| (index, apply::commit_prepared_history_entry(scope, entry, Some("other"))))
        .collect::<Vec<_>>();
    deactivate(scope, admission);
    if delta != 0 {
        super::session_history::publish(
            scope,
            owner,
            moli_page_types::SessionHistoryUpdateKind::Traverse { delta },
        );
    }
    let finished = traversal::finished_resolver_array(scope, &admission.results);
    let resolved_entry = navigation_current_entry(scope, owner)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let requester_replaces_document = cross_document
        .iter()
        .any(|participant| participant.owner.strict_equals(owner.into()));
    if !requester_replaces_document {
        results::resolve_pending_navigation_committed(scope, &admission.results, resolved_entry);
    }
    let mut completions = Vec::new();
    for (index, entry) in &applied {
        let outcome = admission.participants[*index].outcome.local(scope);
        let resolvers = if entry.owner.strict_equals(owner.into()) {
            finished
        } else {
            v8::Array::new(scope, 0)
        };
        let settlement = traversal::prepare_history_participant(scope, entry, &outcome, resolvers);
        completions.push((outcome, resolvers, settlement));
    }
    for (_, entry) in &applied {
        apply::dispatch_history_entry_currententrychange(scope, entry);
    }
    for ((_, entry), (outcome, resolvers, settlement)) in applied.iter().zip(completions) {
        traversal::finish_history_participant(scope, entry, outcome, resolvers, settlement);
    }
    if !requester_replaces_document
        && !applied
            .iter()
            .any(|(_, entry)| entry.owner.strict_equals(owner.into()))
    {
        results::resolve_pending_navigation_finished(scope, &admission.results, resolved_entry);
    }
}
