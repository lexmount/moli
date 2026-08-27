use indexmap::{IndexMap, IndexSet};
use moli_selector::{
    MoliStyleMutationSnapshot as MoliMutationSnapshot, StyloDependencyInvalidationFallbackContext,
    StyloRetainedStyleInvalidationQuery as RetainedStyleInvalidationQuery,
    StyloSourceDependencyInvalidationRequest, StyloSourceDependencyRequestRequirement,
    StyloSourceDependencySummary, stylo_merge_source_dependency_request_requirement,
    stylo_retained_next_element_sibling, stylo_retained_previous_element_sibling,
};

use crate::dom::native::{DomHost, Node};

use super::StyleMutationEffect;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct RetainedSourceDependencyRequestPlan {
    query_requirements:
        IndexMap<RetainedStyleInvalidationQuery, StyloSourceDependencyRequestRequirement>,
}

pub(super) enum RetainedSourceDependencyRequestContext<'a> {
    None,
    Attribute,
    CurrentElement {
        host: &'a DomHost,
    },
    ChildList(&'a MoliMutationSnapshot),
    CharacterData {
        host: &'a DomHost,
        effects: &'a [StyleMutationEffect],
    },
    SlotAssignment(&'a [StyleMutationEffect]),
}

impl RetainedSourceDependencyRequestPlan {
    pub(super) fn exact(queries: IndexSet<RetainedStyleInvalidationQuery>) -> Self {
        Self::with_requirement(queries, StyloSourceDependencyRequestRequirement::exact())
    }

    pub(super) fn all_queries_require_child_list_structural_dependency(
        queries: IndexSet<RetainedStyleInvalidationQuery>,
    ) -> Self {
        Self::with_requirement(
            queries,
            StyloSourceDependencyRequestRequirement::child_list_structural(),
        )
    }

    pub(super) fn is_empty(&self) -> bool {
        self.query_requirements.is_empty()
    }

    pub(super) fn extend(&mut self, incoming: Self) {
        for (query, requirement) in incoming.query_requirements {
            self.record_query_requirement(query, requirement);
        }
    }

    pub(super) fn record_query_requirement(
        &mut self,
        query: RetainedStyleInvalidationQuery,
        requirement: StyloSourceDependencyRequestRequirement,
    ) {
        self.query_requirements
            .entry(query)
            .and_modify(|existing| {
                *existing =
                    stylo_merge_source_dependency_request_requirement(*existing, requirement)
            })
            .or_insert(requirement);
    }

    pub(super) fn source_dependency_requests<'a>(
        &'a self,
        context: &RetainedSourceDependencyRequestContext<'_>,
    ) -> Vec<StyloSourceDependencyInvalidationRequest<'a>> {
        self.query_requirements
            .iter()
            .map(|(query, requirement)| {
                StyloSourceDependencyInvalidationRequest::new(
                    query,
                    context.dependency_fallback_context(query),
                    *requirement,
                )
            })
            .collect()
    }

    pub(super) fn matches_dependency_summary(
        &self,
        summary: &StyloSourceDependencySummary,
    ) -> bool {
        self.query_requirements.keys().any(|query| {
            summary
                .query_result(query.as_stylo_query())
                .has_any_dependency()
        })
    }

    fn with_requirement(
        queries: IndexSet<RetainedStyleInvalidationQuery>,
        requirement: StyloSourceDependencyRequestRequirement,
    ) -> Self {
        Self {
            query_requirements: queries
                .into_iter()
                .map(|query| (query, requirement))
                .collect(),
        }
    }
}

impl RetainedSourceDependencyRequestContext<'_> {
    /// Whether an active source reporting zero matching dependencies proves
    /// that this request has no style effect.
    ///
    /// Direct keyed attribute/class/id mutations have enough source-local
    /// dependency information for that proof. Structural, relative, state,
    /// and slot requests keep their safety roots because a zero match can also
    /// mean that the retained query did not cover the mutation boundary.
    pub(super) fn zero_matched_dependency_result_is_exact(&self) -> bool {
        matches!(self, Self::Attribute)
    }

    fn dependency_fallback_context(
        &self,
        query: &RetainedStyleInvalidationQuery,
    ) -> Option<StyloDependencyInvalidationFallbackContext> {
        match self {
            Self::None => None,
            Self::Attribute => Some(StyloDependencyInvalidationFallbackContext::default()),
            Self::CurrentElement { host } => {
                Some(current_element_dependency_fallback_context(host, query))
            }
            Self::ChildList(mutation_snapshot) => Some(
                mutation_snapshot
                    .child_list_dependency_fallback_context(query)
                    .unwrap_or_default(),
            ),
            Self::CharacterData { host, effects } => Some(
                character_data_dependency_fallback_context(host, effects, query),
            ),
            Self::SlotAssignment(effects) => {
                slot_assignment_dependency_fallback_context(effects, query)
            }
        }
    }
}

fn current_element_dependency_fallback_context(
    host: &DomHost,
    query: &RetainedStyleInvalidationQuery,
) -> StyloDependencyInvalidationFallbackContext {
    let root = query.root();
    StyloDependencyInvalidationFallbackContext::from_mutation_relation(
        host.parent_node(root),
        stylo_retained_previous_element_sibling(host, host.node(root).and_then(Node::prev_sibling)),
        stylo_retained_next_element_sibling(host, host.next_sibling(root)),
    )
}

fn character_data_dependency_fallback_context(
    host: &DomHost,
    effects: &[StyleMutationEffect],
    query: &RetainedStyleInvalidationQuery,
) -> StyloDependencyInvalidationFallbackContext {
    let root = query.root();
    for effect in effects {
        let StyleMutationEffect::CharacterData { node } = effect else {
            continue;
        };
        let changed_parent = host.parent_node(*node).unwrap_or(*node);
        if changed_parent != root {
            continue;
        }
        return StyloDependencyInvalidationFallbackContext::from_mutation_relation(
            host.parent_node(root),
            stylo_retained_previous_element_sibling(
                host,
                host.node(root).and_then(Node::prev_sibling),
            ),
            stylo_retained_next_element_sibling(host, host.next_sibling(root)),
        );
    }
    StyloDependencyInvalidationFallbackContext::default()
}

fn slot_assignment_dependency_fallback_context(
    effects: &[StyleMutationEffect],
    query: &RetainedStyleInvalidationQuery,
) -> Option<StyloDependencyInvalidationFallbackContext> {
    let root = query.root();
    for effect in effects {
        let StyleMutationEffect::SlotAssignment {
            previous_assigned_nodes,
            assigned_nodes,
            ..
        } = effect
        else {
            continue;
        };
        if previous_assigned_nodes
            .as_ref()
            .is_some_and(|nodes| nodes.contains(&root))
            || assigned_nodes
                .as_ref()
                .is_some_and(|nodes| nodes.contains(&root))
        {
            return Some(StyloDependencyInvalidationFallbackContext::default());
        }
    }
    None
}
