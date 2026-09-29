use dom::ElementState as StyloElementState;
use indexmap::IndexSet;
use moli_selector::{
    StyloPlannedFallbackRootInvalidationTarget, StyloRuntimeFallbackRootInput,
    StyloSourceDependencyBoundaryRoots, StyloSourceDependencyInvalidationBatchPlan,
    StyloSourceDependencyInvalidationRequest, StyloSourceInvalidationFallbackReason,
    StyloStyleSourceScope as StyleSourceScope, stylo_fallback_roots_plan,
    stylo_runtime_fallback_roots_for_mutation_inputs, stylo_runtime_or_source_scope_fallback_plan,
    stylo_source_dependency_invalidation_batch_plan,
    stylo_source_fallback_reason_for_unretained_state_change,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    StyleMutationEffect,
    eligibility::attribute_has_non_css_runtime_side_effect,
    mutation_effect::{
        style_mutation_effects_are_all_attributes, style_mutation_effects_are_child_list_structural,
    },
    source_record::MatchingStyleDependencySource,
    target_queries::{PendingStyleInvalidationTargetQueries, merge_pending_target_queries},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PendingStyleInvalidationCause {
    Mutation(Vec<StyleMutationEffect>),
    StateChange {
        element: DomHandle,
        state: StyloElementState,
        old_state: Option<StyloElementState>,
    },
    CustomStateChange {
        element: DomHandle,
        state_names: Vec<String>,
        old_custom_states: Vec<String>,
    },
    FocusChange {
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        previous_focus_states: Option<Vec<moli_selector::StyloStateInvalidationRoot>>,
    },
    TargetChange {
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PendingStyleInvalidationWorkKind {
    Mutation,
    StateChange,
    CustomStateChange,
    FocusChange,
    TargetChange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PendingStyleInvalidationMergeClass {
    #[default]
    None,
    AttributeMutation,
    ChildListStructuralMutation,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PendingCauseFallback {
    roots: IndexSet<DomHandle>,
    reason: Option<StyloSourceInvalidationFallbackReason>,
}

impl PendingStyleInvalidationCause {
    pub(super) fn work_kind(&self) -> PendingStyleInvalidationWorkKind {
        match self {
            Self::Mutation(_) => PendingStyleInvalidationWorkKind::Mutation,
            Self::StateChange { .. } => PendingStyleInvalidationWorkKind::StateChange,
            Self::CustomStateChange { .. } => PendingStyleInvalidationWorkKind::CustomStateChange,
            Self::FocusChange { .. } => PendingStyleInvalidationWorkKind::FocusChange,
            Self::TargetChange { .. } => PendingStyleInvalidationWorkKind::TargetChange,
        }
    }

    pub(super) fn pending_merge_class(&self) -> PendingStyleInvalidationMergeClass {
        match self {
            Self::Mutation(effects) if style_mutation_effects_are_all_attributes(effects) => {
                PendingStyleInvalidationMergeClass::AttributeMutation
            }
            Self::Mutation(effects)
                if style_mutation_effects_are_child_list_structural(effects) =>
            {
                PendingStyleInvalidationMergeClass::ChildListStructuralMutation
            }
            _ => PendingStyleInvalidationMergeClass::None,
        }
    }
}

impl PendingCauseFallback {
    pub(super) fn from_cause(host: &DomHost, cause: &PendingStyleInvalidationCause) -> Self {
        let roots = cause_default_fallback_roots(host, cause);
        Self {
            roots,
            reason: fallback_reason_for_unretained_pending_cause(cause),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.roots.is_empty() && self.reason.is_none()
    }

    fn roots_for_selector_plan(&self) -> Vec<DomHandle> {
        self.roots.iter().copied().collect()
    }

    pub(super) fn merge_cause_roots_into_source_dependency_target_queries(
        &self,
        host: &DomHost,
        target_queries: &mut Vec<PendingStyleInvalidationTargetQueries>,
    ) {
        merge_pending_target_queries(
            target_queries,
            PendingStyleInvalidationTargetQueries::planned_fallback_root_target(
                host,
                stylo_fallback_roots_plan(self.roots_for_selector_plan(), std::iter::empty()),
            ),
        );
    }

    pub(super) fn runtime_or_source_scope_fallback_target(
        &self,
        host: &DomHost,
        source_scope: &StyleSourceScope,
    ) -> StyloPlannedFallbackRootInvalidationTarget {
        stylo_runtime_or_source_scope_fallback_plan(
            host,
            source_scope,
            self.roots_for_selector_plan(),
            self.reason,
        )
    }

    pub(super) fn source_dependency_batch_plan(
        &self,
        host: &DomHost,
        matching_sources: &[MatchingStyleDependencySource],
        requests: &[StyloSourceDependencyInvalidationRequest<'_>],
        boundary_roots: StyloSourceDependencyBoundaryRoots<'_>,
    ) -> StyloSourceDependencyInvalidationBatchPlan {
        let cause_fallback_roots = self.roots_for_selector_plan();
        let batch_sources = matching_sources
            .iter()
            .map(|source| source.stylo_batch_source(&cause_fallback_roots))
            .collect::<Vec<_>>();
        stylo_source_dependency_invalidation_batch_plan(
            host,
            &batch_sources,
            requests,
            boundary_roots,
        )
    }
}

impl PendingStyleInvalidationWorkKind {
    #[cfg(test)]
    pub(super) fn name_for_test(self) -> &'static str {
        match self {
            Self::Mutation => "mutation",
            Self::StateChange => "state",
            Self::CustomStateChange => "custom-state",
            Self::FocusChange => "focus",
            Self::TargetChange => "target",
        }
    }
}

fn fallback_reason_for_unretained_pending_cause(
    cause: &PendingStyleInvalidationCause,
) -> Option<StyloSourceInvalidationFallbackReason> {
    match cause {
        PendingStyleInvalidationCause::StateChange {
            state, old_state, ..
        } => stylo_source_fallback_reason_for_unretained_state_change(*state, *old_state),
        _ => None,
    }
}

fn cause_default_fallback_roots(
    host: &DomHost,
    cause: &PendingStyleInvalidationCause,
) -> IndexSet<DomHandle> {
    let PendingStyleInvalidationCause::Mutation(effects) = cause else {
        return IndexSet::new();
    };
    let mut roots = IndexSet::new();
    for effect in effects {
        if let StyleMutationEffect::ConnectedSubtrees {
            roots: connected_roots,
        } = effect
        {
            roots.extend(connected_roots.iter().copied());
        }
    }
    roots.extend(stylo_runtime_fallback_roots_for_mutation_inputs(
        host,
        effects
            .iter()
            .filter_map(runtime_fallback_root_input_for_mutation_effect),
    ));
    roots
}

fn runtime_fallback_root_input_for_mutation_effect(
    effect: &StyleMutationEffect,
) -> Option<StyloRuntimeFallbackRootInput<'_>> {
    Some(match effect {
        StyleMutationEffect::Attribute { element, name, .. } => {
            StyloRuntimeFallbackRootInput::Attribute {
                element: *element,
                attribute_name: name,
                has_dependency_change: effect.attribute_dependency_change().is_some(),
                has_non_css_runtime_side_effect: attribute_has_non_css_runtime_side_effect(name),
            }
        }
        StyleMutationEffect::ChildList { added_nodes, .. } => {
            StyloRuntimeFallbackRootInput::ChildList { added_nodes }
        }
        StyleMutationEffect::SlotAssignment {
            slot,
            previous_assigned_nodes,
            assigned_nodes,
        } => StyloRuntimeFallbackRootInput::SlotAssignment {
            slot: *slot,
            has_assignment_snapshot: previous_assigned_nodes
                .as_ref()
                .zip(assigned_nodes.as_ref())
                .is_some(),
        },
        StyleMutationEffect::ConnectedSubtrees { roots } => {
            StyloRuntimeFallbackRootInput::ConnectedSubtree {
                root: *roots.first()?,
            }
        }
        StyleMutationEffect::CharacterData { .. }
        | StyleMutationEffect::DisconnectedSubtrees { .. } => {
            StyloRuntimeFallbackRootInput::OtherMutation
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeDom;

    #[test]
    fn batched_connected_subtrees_keep_every_runtime_fallback_root() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        let document = host.document_handle();
        let first = host.create_element("div");
        let second = host.create_element("span");
        assert!(host.append_child(document, first));
        assert!(host.append_child(document, second));
        let cause = PendingStyleInvalidationCause::Mutation(vec![
            StyleMutationEffect::ConnectedSubtrees {
                roots: vec![first, second].into(),
            },
            StyleMutationEffect::ChildList {
                parent: document,
                added_nodes: vec![first, second],
                removed_nodes: Vec::new(),
                removed_element_snapshots: Vec::new(),
                previous_sibling: None,
                next_sibling: None,
            },
        ]);

        assert_eq!(
            cause_default_fallback_roots(&host, &cause)
                .into_iter()
                .collect::<Vec<_>>(),
            [first, second]
        );
    }
}
