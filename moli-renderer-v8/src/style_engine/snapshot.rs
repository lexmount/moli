use dom::ElementState as StyloElementState;
use indexmap::IndexSet;
use moli_selector::{
    MoliStyleMutationSnapshot as MoliMutationSnapshot, StyloDomStyleAdapter,
    StyloRetainedStyleInvalidationQuery as RetainedStyleInvalidationQuery,
    stylo_focus_state_matches_handle, stylo_focus_within_state_matches_handle,
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

use super::{
    StyleMutationEffect,
    cause::PendingStyleInvalidationCause,
    eligibility::attribute_effect_can_use_retained_stylo_invalidator,
    mutation_effect::{
        style_mutation_effects_are_all_attributes, style_mutation_effects_are_child_list_structural,
    },
    query::{
        retained_stylo_invalidation_queries_for_focus_change,
        retained_stylo_invalidation_queries_for_target_change,
    },
};

pub(super) fn moli_style_mutation_snapshot_for_pending_cause(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    cause: &PendingStyleInvalidationCause,
) -> Option<MoliMutationSnapshot> {
    let mut inputs = MoliMutationSnapshot::default();
    merge_moli_style_mutation_snapshot_for_pending_cause(host, dom_adapter, cause, &mut inputs)?;
    Some(inputs)
}

fn merge_moli_style_mutation_snapshot_for_pending_cause(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    cause: &PendingStyleInvalidationCause,
    inputs: &mut MoliMutationSnapshot,
) -> Option<()> {
    match cause {
        PendingStyleInvalidationCause::Mutation(effects)
            if style_mutation_effects_are_all_attributes(effects) =>
        {
            for effect in effects {
                let StyleMutationEffect::Attribute {
                    element,
                    name,
                    old_value,
                    ..
                } = effect
                else {
                    return None;
                };
                if !attribute_effect_can_use_retained_stylo_invalidator(name) {
                    return None;
                }
                inputs.record_attribute_change(*element, name, old_value.clone());
            }
        }
        PendingStyleInvalidationCause::Mutation(effects)
            if style_mutation_effects_are_child_list_structural(effects) =>
        {
            for effect in effects {
                let StyleMutationEffect::ChildList {
                    parent,
                    added_nodes,
                    removed_nodes,
                    removed_element_snapshots,
                    previous_sibling,
                    next_sibling,
                } = effect
                else {
                    continue;
                };
                inputs.record_child_list_mutation(
                    *parent,
                    added_nodes,
                    removed_nodes,
                    removed_element_snapshots,
                    *previous_sibling,
                    *next_sibling,
                );
            }
        }
        PendingStyleInvalidationCause::Mutation(_) => return None,
        PendingStyleInvalidationCause::StateChange {
            element, old_state, ..
        } => {
            inputs.try_record_old_state(*element, *old_state.as_ref()?)?;
        }
        PendingStyleInvalidationCause::CustomStateChange {
            element,
            old_custom_states,
            ..
        } => {
            inputs.record_old_custom_states(*element, old_custom_states.clone());
        }
        PendingStyleInvalidationCause::FocusChange {
            previous,
            next,
            previous_focus_states,
        } => {
            if host.active_element_handle() != *next {
                return None;
            }
            let queries = retained_stylo_invalidation_queries_for_focus_change(
                host,
                *previous,
                *next,
                previous_focus_states.as_deref(),
            );
            for (handle, old_state) in retained_stylo_old_states_for_query_roots(
                host,
                dom_adapter,
                &queries,
                |handle, state| {
                    old_focus_snapshot_state(
                        host,
                        handle,
                        *previous,
                        previous_focus_states.as_deref(),
                        state,
                    )
                },
            )? {
                inputs.try_record_old_state(handle, old_state)?;
            }
        }
        PendingStyleInvalidationCause::TargetChange { previous, next } => {
            if !target_change_matches_current_host_state(host, *previous, *next) {
                return None;
            }
            let queries = retained_stylo_invalidation_queries_for_target_change(*previous, *next);
            for (handle, old_state) in retained_stylo_old_states_for_query_roots(
                host,
                dom_adapter,
                &queries,
                |handle, state| old_target_snapshot_state(handle, *previous, state),
            )? {
                inputs.try_record_old_state(handle, old_state)?;
            }
        }
    }
    Some(())
}

fn retained_stylo_old_states_for_query_roots(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    queries: &IndexSet<RetainedStyleInvalidationQuery>,
    old_state_for_handle: impl Fn(DomHandle, StyloElementState) -> StyloElementState,
) -> Option<Vec<(DomHandle, StyloElementState)>> {
    let handles = retained_stylo_query_roots(queries);
    dom_adapter.with_bound_host(host, |adapter| {
        handles
            .into_iter()
            .map(|handle| {
                let current_state = adapter.computed_element_state(host, handle)?;
                Some((handle, old_state_for_handle(handle, current_state)))
            })
            .collect::<Option<Vec<_>>>()
    })
}

fn retained_stylo_query_roots(
    queries: &IndexSet<RetainedStyleInvalidationQuery>,
) -> IndexSet<DomHandle> {
    queries
        .iter()
        .map(RetainedStyleInvalidationQuery::root)
        .collect()
}

fn old_focus_snapshot_state(
    host: &DomHost,
    handle: DomHandle,
    previous: Option<DomHandle>,
    previous_focus_states: Option<&[moli_selector::StyloStateInvalidationRoot]>,
    mut state: StyloElementState,
) -> StyloElementState {
    state.remove(StyloElementState::FOCUS | StyloElementState::FOCUSRING);
    state.remove(StyloElementState::FOCUS_WITHIN);
    if let Some(previous_focus_states) = previous_focus_states {
        // A removal/adoption can sever the old shadow-host chain. Captured
        // focus states are authoritative; never reconstruct them from the new tree.
        for old in previous_focus_states
            .iter()
            .filter(|old| old.root() == handle)
        {
            state.insert(old.state());
        }
        return state;
    }
    let Some(previous) = previous else {
        return state;
    };
    if stylo_focus_state_matches_handle(host, previous, handle) {
        state.insert(StyloElementState::FOCUS | StyloElementState::FOCUSRING);
    }
    if stylo_focus_within_state_matches_handle(host, previous, handle) {
        state.insert(StyloElementState::FOCUS_WITHIN);
    }
    state
}

fn old_target_snapshot_state(
    handle: DomHandle,
    previous: Option<DomHandle>,
    mut state: StyloElementState,
) -> StyloElementState {
    state.remove(StyloElementState::URLTARGET);
    if previous == Some(handle) {
        state.insert(StyloElementState::URLTARGET);
    }
    state
}

fn target_change_matches_current_host_state(
    host: &DomHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
) -> bool {
    let Some(document) = next
        .or(previous)
        .and_then(|handle| host.node(handle))
        .and_then(Node::owner_document)
    else {
        return false;
    };
    host.document_target_element(document) == next
}
