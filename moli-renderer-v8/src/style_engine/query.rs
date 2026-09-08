use dom::ElementState as StyloElementState;
use indexmap::IndexSet;
use moli_selector::{
    MoliStyleMutationSnapshot as MoliMutationSnapshot,
    StyloRetainedStyleInvalidationQuery as RetainedStyleInvalidationQuery,
    stylo_focus_change_invalidation_roots, stylo_retained_queries_for_current_element,
    stylo_state_change_can_use_retained_invalidator,
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
        style_mutation_effects_are_all_attributes, style_mutation_effects_are_all_character_data,
        style_mutation_effects_are_all_slot_assignments,
    },
    retained_plan::RetainedBaseQueryPlan,
};

fn retained_stylo_single_query(
    query: RetainedStyleInvalidationQuery,
) -> IndexSet<RetainedStyleInvalidationQuery> {
    let mut queries = IndexSet::new();
    queries.insert(query);
    queries
}

fn retained_stylo_base_queries_for_attribute_mutation_effects(
    effects: &[StyleMutationEffect],
) -> Option<RetainedBaseQueryPlan> {
    let mut queries = IndexSet::new();
    for effect in effects {
        let StyleMutationEffect::Attribute { element, name, .. } = effect else {
            return None;
        };
        if !attribute_effect_can_use_retained_stylo_invalidator(name) {
            return None;
        }
        let change = effect.attribute_dependency_change()?;
        queries.insert(RetainedStyleInvalidationQuery::attribute(
            *element,
            change.attribute_name.to_owned(),
        ));
        for token in change
            .removed_class_tokens
            .iter()
            .chain(change.added_class_tokens.iter())
        {
            queries.insert(RetainedStyleInvalidationQuery::class(
                *element,
                token.clone(),
            ));
        }
        for value in change.removed_id.iter().chain(change.added_id.iter()) {
            queries.insert(RetainedStyleInvalidationQuery::id(*element, value.clone()));
        }
    }
    Some(RetainedBaseQueryPlan::exact(queries))
}

fn retained_stylo_base_queries_for_character_data_mutations(
    host: &DomHost,
    effects: &[StyleMutationEffect],
) -> Option<RetainedBaseQueryPlan> {
    let mut queries = IndexSet::new();
    let mut base_roots = IndexSet::new();
    for effect in effects {
        let StyleMutationEffect::CharacterData { node } = effect else {
            return None;
        };
        let parent = host.parent_node(*node).unwrap_or(*node);
        if host.node(parent).and_then(Node::as_element).is_none() {
            continue;
        }
        base_roots.insert(parent);
        queries.extend(stylo_retained_queries_for_current_element(
            host, parent, None,
        ));
    }
    (!queries.is_empty()).then(|| {
        RetainedBaseQueryPlan::structural_boundary_cleanup_roots_for_all_queries(
            queries, base_roots,
        )
    })
}

fn retained_stylo_base_queries_for_slot_assignment_mutations(
    host: &DomHost,
    effects: &[StyleMutationEffect],
) -> Option<RetainedBaseQueryPlan> {
    let mut queries = IndexSet::new();
    let mut base_roots = IndexSet::new();
    let mut has_slot_assignment = false;
    for effect in effects {
        let StyleMutationEffect::SlotAssignment {
            previous_assigned_nodes,
            assigned_nodes,
            ..
        } = effect
        else {
            continue;
        };
        has_slot_assignment = true;
        let (Some(previous), Some(current)) = (previous_assigned_nodes, assigned_nodes) else {
            continue;
        };
        if previous == current {
            continue;
        }
        for &root in previous.iter().chain(current) {
            if host.node(root).and_then(Node::as_element).is_none() {
                continue;
            }
            base_roots.insert(root);
            queries.extend(stylo_retained_queries_for_current_element(host, root, None));
        }
    }
    has_slot_assignment
        .then(|| RetainedBaseQueryPlan::exact_with_empty_target_fallback_roots(queries, base_roots))
}

pub(super) fn retained_base_query_plan_for_pending_cause(
    host: &DomHost,
    cause: &PendingStyleInvalidationCause,
    mutation_snapshot: &MoliMutationSnapshot,
) -> Option<RetainedBaseQueryPlan> {
    match cause {
        PendingStyleInvalidationCause::Mutation(effects) => {
            if style_mutation_effects_are_all_attributes(effects) {
                return retained_stylo_base_queries_for_attribute_mutation_effects(effects);
            }
            if style_mutation_effects_are_all_character_data(effects) {
                return retained_stylo_base_queries_for_character_data_mutations(host, effects);
            }
            if style_mutation_effects_are_all_slot_assignments(effects) {
                return retained_stylo_base_queries_for_slot_assignment_mutations(host, effects);
            }
            let mut child_list = retained_stylo_invalidation_queries_for_child_list_mutations(
                host,
                effects,
                mutation_snapshot,
            )?;
            if let Some(slot_assignments) =
                retained_stylo_base_queries_for_slot_assignment_mutations(host, effects)
            {
                child_list.merge_from(slot_assignments);
            }
            Some(child_list)
        }
        PendingStyleInvalidationCause::StateChange {
            element,
            state,
            old_state,
        } => {
            if !stylo_state_change_can_use_retained_invalidator(*state, *old_state) {
                return None;
            }
            Some(RetainedBaseQueryPlan::exact(retained_stylo_single_query(
                RetainedStyleInvalidationQuery::state(*element, *state),
            )))
        }
        PendingStyleInvalidationCause::CustomStateChange {
            element,
            state_names,
            ..
        } => {
            let mut queries = IndexSet::new();
            for state_name in state_names {
                if !state_name.is_empty() {
                    queries.insert(RetainedStyleInvalidationQuery::custom_state(
                        *element,
                        state_name.clone(),
                    ));
                }
            }
            (!queries.is_empty()).then(|| RetainedBaseQueryPlan::exact(queries))
        }
        PendingStyleInvalidationCause::FocusChange {
            previous,
            next,
            previous_focus_within,
        } => Some(RetainedBaseQueryPlan::exact(
            retained_stylo_invalidation_queries_for_focus_change(
                host,
                *previous,
                *next,
                previous_focus_within.as_deref(),
            ),
        )),
        PendingStyleInvalidationCause::TargetChange { previous, next } => {
            Some(RetainedBaseQueryPlan::exact(
                retained_stylo_invalidation_queries_for_target_change(*previous, *next),
            ))
        }
    }
}

fn retained_stylo_invalidation_queries_for_child_list_mutations(
    host: &DomHost,
    effects: &[StyleMutationEffect],
    mutation_snapshot: &MoliMutationSnapshot,
) -> Option<RetainedBaseQueryPlan> {
    for effect in effects {
        if !matches!(effect, StyleMutationEffect::ChildList { .. }) {
            if matches!(
                effect,
                StyleMutationEffect::ConnectedSubtrees { .. }
                    | StyleMutationEffect::DisconnectedSubtrees { .. }
                    | StyleMutationEffect::SlotAssignment { .. }
            ) {
                continue;
            }
            return None;
        }
    }
    let child_list = mutation_snapshot.child_list_invalidation_queries(host)?;
    Some(RetainedBaseQueryPlan::child_list_structural_boundary_cleanup_roots(child_list))
}

pub(super) fn retained_stylo_invalidation_queries_for_focus_change(
    host: &DomHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    previous_focus_within: Option<&[DomHandle]>,
) -> IndexSet<RetainedStyleInvalidationQuery> {
    let mut queries = stylo_focus_change_invalidation_roots(host, previous, next)
        .into_iter()
        .map(|query| RetainedStyleInvalidationQuery::state(query.root(), query.state()))
        .collect::<IndexSet<_>>();
    for handle in previous_focus_within.into_iter().flatten().copied() {
        queries.insert(RetainedStyleInvalidationQuery::state(
            handle,
            StyloElementState::FOCUS_WITHIN,
        ));
    }
    queries
}

pub(super) fn retained_stylo_invalidation_queries_for_target_change(
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
) -> IndexSet<RetainedStyleInvalidationQuery> {
    let mut queries = IndexSet::new();
    for root in [previous, next].into_iter().flatten() {
        queries.insert(RetainedStyleInvalidationQuery::state(
            root,
            StyloElementState::URLTARGET,
        ));
    }
    queries
}
