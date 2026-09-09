/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Moli-facing selector invalidation summaries.
//!
//! This module exposes a small, read-only view over Stylo's existing selector
//! invalidation maps. It lets Moli consume selector dependency truth
//! without adopting Servo's full restyle lifecycle.

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;

use crate::context::QuirksMode;
use crate::derives::*;
use crate::dom::{TDocument, TElement, TNode};
use crate::invalidation::element::element_wrapper::ElementWrapper;
use crate::invalidation::element::invalidation_map::{
    AdditionalRelativeSelectorInvalidationMap, Dependency, DependencyInvalidationKind,
    InvalidationMap, NormalDependencyInvalidationKind, RelativeDependencyInvalidationKind,
    ScopeDependencyInvalidationKind, StateDependency,
};
use crate::invalidation::element::invalidator::{
    any_next_has_scope_in_negation, note_scope_dependency_force_at_subject,
    DescendantInvalidationLists, Invalidation, InvalidationProcessor, InvalidationResult,
    InvalidationVector, SiblingTraversalMap,
};
use crate::invalidation::element::relative_selector::RelativeSelectorInvalidator;
use crate::invalidation::element::state_and_attributes::check_dependency;
use crate::selector_map::SelectorMap;
use crate::selector_parser::SnapshotMap;
use crate::stylist::{CascadeData, Stylist};
use crate::values::AtomIdent;
use crate::{Atom, LocalName};
use dom::ElementState;
use indexmap::{IndexMap, IndexSet};
use selectors::matching::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, SelectorCaches,
};
use selectors::OpaqueElement;
use servo_arc::Arc as ServoArc;

/// Moli-facing invalidation dependency kind extracted from Stylo's
/// selector invalidation maps.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliDependencyKind {
    /// The changed element itself can be invalidated.
    Element,
    /// The changed element and descendants can be invalidated.
    ElementAndDescendants,
    /// Descendants of the changed element can be invalidated.
    Descendants,
    /// Following siblings of the changed element can be invalidated.
    Siblings,
    /// Slotted elements can be invalidated.
    SlottedElements,
    /// Exposed parts can be invalidated.
    Parts,
    /// Relative selector anchors among ancestors can be invalidated.
    RelativeAncestors,
    /// A relative selector parent anchor can be invalidated.
    RelativeParent,
    /// A relative selector previous-sibling anchor can be invalidated.
    RelativePrevSibling,
    /// Relative selector ancestor previous-sibling anchors can be invalidated.
    RelativeAncestorPrevSibling,
    /// Relative selector earlier-sibling anchors can be invalidated.
    RelativeEarlierSibling,
    /// Relative selector ancestor earlier-sibling anchors can be invalidated.
    RelativeAncestorEarlierSibling,
    /// Scope dependencies can be invalidated.
    Scope,
}

/// Moli-facing action represented by one raw Stylo dependency.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliDependencyInvalidationAction {
    /// The changed element itself can be invalidated.
    Element,
    /// The changed element and descendants can be invalidated.
    ElementAndDescendants,
    /// Descendants of the changed element can be invalidated.
    Descendants,
    /// Following siblings of the changed element can be invalidated.
    Siblings,
    /// Assigned/slotted descendants can be invalidated.
    SlottedElements,
    /// Exposed part descendants can be invalidated.
    Parts,
    /// Scope dependency handling needs the scope-specific retained path.
    Scope(MoliScopeDependencyInvalidationAction),
    /// This dependency cannot be executed by Moli's retained path.
    Fallback(MoliSourceInvalidationFallbackReason),
}

/// Sink for applying one retained dependency invalidation action.
trait MoliDependencyInvalidationActionSink {
    /// The changed element itself should be invalidated.
    fn invalidate_element(&mut self);

    /// The changed element and its descendants should be invalidated.
    fn invalidate_element_and_descendants(&mut self);

    /// Descendants of the changed element should be invalidated.
    fn invalidate_descendants(&mut self);

    /// Following siblings of the changed element should be invalidated.
    fn invalidate_siblings(&mut self);

    /// Assigned/slotted descendants should be invalidated.
    fn invalidate_slotted_elements(&mut self);

    /// Exposed part descendants should be invalidated.
    fn invalidate_parts(&mut self);

    /// The dependency requires fallback handling.
    fn invalidate_fallback(&mut self, reason: MoliSourceInvalidationFallbackReason);

    /// Scope-specific retained invalidation handling should run.
    fn invalidate_scope(&mut self, action: MoliScopeDependencyInvalidationAction);
}

/// Scope dependency branch Moli should execute for one raw Stylo
/// dependency.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliScopeDependencyInvalidationAction {
    /// The dependency is an implicit `@scope` edge and only its next
    /// dependencies should be propagated to descendants.
    ImplicitScope,
    /// The scope dependency must add invalidations at the subject.
    ForceAtSubject {
        /// Whether the subject add was forced by `:scope` in negation.
        force_add: bool,
    },
    /// Check next dependencies against the current element under this scope.
    CheckNextInScope,
    /// Push the scope dependency itself by the combinator to the right.
    PushByCombinator,
}

/// Sink for applying one scope dependency invalidation action.
trait MoliScopeDependencyInvalidationActionSink {
    /// Propagate implicit `@scope` next dependencies to descendants.
    fn invalidate_implicit_scope(&mut self);

    /// Add invalidations at the scope subject.
    fn invalidate_scope_force_at_subject(&mut self, force_add: bool);

    /// Check next dependencies against the current element under this scope.
    fn invalidate_scope_check_next(&mut self);

    /// Push the scope dependency by the combinator to the right.
    fn invalidate_scope_by_combinator(&mut self);
}

/// Whether Moli's retained invalidation processor can execute one raw
/// dependency, and how its query result should be classified.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliRetainedProcessorDependencyEffect {
    /// The retained processor can execute this dependency.
    Retained {
        /// Whether an empty result for this dependency is an exact no-op.
        empty_result_is_exact: bool,
    },
    /// The dependency requires source-level fallback handling.
    Fallback(MoliSourceInvalidationFallbackReason),
}

/// Relative selector candidate traversal represented by one raw Stylo
/// dependency.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliRelativeDependencyInvalidationAction {
    /// Visit ancestor candidates.
    Ancestors,
    /// Visit the parent candidate.
    Parent,
    /// Visit the previous sibling candidate.
    PrevSibling,
    /// Visit previous siblings.
    EarlierSibling,
    /// Visit previous siblings of ancestors.
    AncestorPrevSibling,
    /// Visit earlier siblings of ancestors.
    AncestorEarlierSibling,
}

/// Sink for applying one relative selector candidate traversal action.
trait MoliRelativeDependencyInvalidationActionSink {
    /// Visit ancestor candidates.
    fn visit_relative_ancestor_candidates(&mut self);

    /// Visit the parent candidate.
    fn visit_relative_parent_candidate(&mut self);

    /// Visit the previous sibling candidate.
    fn visit_relative_previous_sibling_candidate(&mut self);

    /// Visit previous sibling candidates.
    fn visit_relative_earlier_sibling_candidates(&mut self);

    /// Visit previous sibling candidates for ancestors.
    fn visit_relative_ancestor_previous_sibling_candidates(&mut self);

    /// Visit earlier sibling candidates for ancestors.
    fn visit_relative_ancestor_earlier_sibling_candidates(&mut self);
}

/// A Moli style invalidation query that can be answered from Stylo
/// invalidation maps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoliStyleInvalidationQuery<'a> {
    /// An element matching `*` was inserted or removed.
    Universal,
    /// An element with this local name was inserted or removed.
    Type(&'a str),
    /// An attribute local-name changed on the root element.
    Attribute(&'a str),
    /// A class token was added or removed on the root element.
    Class(&'a str),
    /// An id value was added or removed on the root element.
    Id(&'a str),
    /// A pseudo-class state changed on the root element.
    State(ElementState),
    /// A custom state changed on the root element.
    CustomState(&'a str),
}

/// One pseudo-class state invalidation root derived for Moli.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliStateInvalidationRoot<Root> {
    root: Root,
    state: ElementState,
}

/// Source-local retained invalidation query after the runtime-owned retained
/// query has been borrowed into Stylo invalidation-map query shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationQuery<'a, Root> {
    root: Root,
    query: MoliStyleInvalidationQuery<'a>,
    previous_sibling: Option<Root>,
    next_sibling: Option<Root>,
}

impl<Root: Copy> MoliStateInvalidationRoot<Root> {
    /// Create one state invalidation root.
    #[inline]
    pub fn new(root: Root, state: ElementState) -> Self {
        Self { root, state }
    }

    /// Return the root that should be invalidated for this state change.
    #[inline]
    pub fn root(&self) -> Root {
        self.root
    }

    /// Return the pseudo-class state represented by this root.
    #[inline]
    pub fn state(&self) -> ElementState {
        self.state
    }
}

/// Sibling context used when querying retained style invalidation for a root
/// that has already moved out of its original child list position.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct MoliRetainedStyleSiblingTraversal<Root> {
    previous_sibling: Option<Root>,
    next_sibling: Option<Root>,
}

impl<Root: Copy> MoliRetainedStyleSiblingTraversal<Root> {
    /// Create sibling traversal context.
    #[inline]
    pub fn new(previous_sibling: Option<Root>, next_sibling: Option<Root>) -> Self {
        Self {
            previous_sibling,
            next_sibling,
        }
    }

    /// Return the previous sibling from the original child-list position.
    #[inline]
    pub fn previous_sibling(&self) -> Option<Root> {
        self.previous_sibling
    }

    /// Return the next sibling from the original child-list position.
    #[inline]
    pub fn next_sibling(&self) -> Option<Root> {
        self.next_sibling
    }
}

impl<'a, Root: Copy> MoliSourceStyleInvalidationQuery<'a, Root> {
    /// Create one source-local invalidation query row.
    #[inline]
    pub fn new(
        root: Root,
        query: MoliStyleInvalidationQuery<'a>,
        previous_sibling: Option<Root>,
        next_sibling: Option<Root>,
    ) -> Self {
        Self {
            root,
            query,
            previous_sibling,
            next_sibling,
        }
    }

    /// Return the query root.
    #[inline]
    pub fn root(&self) -> Root {
        self.root
    }

    /// Return the Stylo invalidation-map query shape.
    #[inline]
    pub fn query(&self) -> MoliStyleInvalidationQuery<'a> {
        self.query
    }

    /// Return the mutation-time previous sibling, when available.
    #[inline]
    pub fn previous_sibling(&self) -> Option<Root> {
        self.previous_sibling
    }

    /// Return the mutation-time next sibling, when available.
    #[inline]
    pub fn next_sibling(&self) -> Option<Root> {
        self.next_sibling
    }
}

/// Owned retained invalidation query used by Moli's runtime queue.
///
/// The runtime owns mutation collection and cache clearing, but this keeps the
/// retained Stylo dependency query shape in the fork.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MoliRetainedStyleInvalidationQuery<Root> {
    root: Root,
    kind: MoliRetainedStyleInvalidationQueryKind,
    sibling_traversal: Option<MoliRetainedStyleSiblingTraversal<Root>>,
}

/// Requirement for running a retained query against a stylesheet source's
/// dependency summary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MoliSourceDependencyRequestRequirement {
    requires_child_list_structural_dependency: bool,
    requires_relative_previous_sibling_dependency: bool,
}

/// Mutation-time relation for fallback roots when the changed element has
/// already been inserted or removed from its original sibling position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliDependencyInvalidationFallbackContext<Root> {
    parent: Option<Root>,
    previous_sibling: Option<Root>,
    next_sibling: Option<Root>,
}

/// Per-element mutation before-state captured by Moli before retained
/// invalidation is drained through Stylo.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MoliStyleMutationElementSnapshot {
    attribute_changes: IndexMap<String, Option<String>>,
    old_state: Option<ElementState>,
    old_custom_states: Option<Vec<String>>,
}

/// Materialized old element state used by Stylo's invalidation selector
/// matching for one Moli element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliStyleInvalidationSnapshot<Root> {
    element: Root,
    state: Option<ElementState>,
    custom_states: Option<Vec<String>>,
    changed_attributes: Vec<String>,
    attributes: Vec<MoliStyleInvalidationSnapshotAttribute>,
}

/// One materialized attribute in a [`MoliStyleInvalidationSnapshot`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliStyleInvalidationSnapshotAttribute {
    local_name: String,
    name: String,
    namespace: String,
    prefix: Option<String>,
    value: String,
}

/// One retained mutation attribute change borrowed from
/// [`MoliStyleMutationElementSnapshot`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliStyleMutationAttributeChange<'a> {
    name: &'a str,
    old_value: Option<&'a str>,
}

/// Context-derived fallback roots for one dependency query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliDependencyInvalidationContextRoots<Root> {
    requires_source_fallback: bool,
    roots: Vec<Root>,
}

/// Opaque context-root plan derived from a dependency query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliDependencyContextRootPlan {
    flags: MoliDependencyContextRootFlags,
    allow_direct_previous_following_sibling_fallback: bool,
}

/// Adapter used by the source dependency planner when mutation-context roots
/// require DOM traversal.
pub trait MoliSourceDependencyInvalidationContextRootsProvider<Root> {
    /// Build conservative context roots from a Stylo-derived root plan.
    fn context_roots_for_source_dependency(
        &mut self,
        root: Root,
        plan: MoliDependencyContextRootPlan,
        context: MoliDependencyInvalidationFallbackContext<Root>,
    ) -> MoliDependencyInvalidationContextRoots<Root>;
}

/// Source-local invalidation request for one retained query, including
/// mutation context and source dependency gates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliSourceDependencyInvalidationRequest<'a, Root> {
    query: &'a MoliRetainedStyleInvalidationQuery<Root>,
    context: Option<MoliDependencyInvalidationFallbackContext<Root>>,
    requirement: MoliSourceDependencyRequestRequirement,
}

/// Selector-derived keys for DOM boundaries whose child structure can affect
/// a source's tree-structural selectors.
///
/// These keys are separate from normal state/attribute invalidation metadata:
/// selectors such as `section:empty` and
/// `details > summary:first-of-type` are driven by mutations to the boundary
/// element even though no attribute on the selector subject changed.
#[derive(Clone, Debug, Default, Eq, Hash, MallocSizeOf, PartialEq)]
pub(crate) struct MoliChildListStructuralBoundaryDependencySummary {
    class_dependencies: Vec<Atom>,
    id_dependencies: Vec<Atom>,
    type_dependencies: Vec<LocalName>,
    attribute_dependencies: Vec<LocalName>,
    universal_dependency: bool,
}

/// Source-local Stylo dependency metadata used by Moli invalidation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct MoliSourceDependencySummary {
    dependency_summary: MoliDependencyInvalidationSummary,
    has_child_list_structural_dependency: bool,
    child_list_structural_boundary_dependencies:
        MoliChildListStructuralBoundaryDependencySummary,
}

/// One stylesheet source participating in a source dependency invalidation
/// batch.
pub struct MoliSourceDependencyInvalidationBatchSource<'a, Root> {
    dependency_summary: &'a MoliSourceDependencySummary,
    fallback_roots: MoliSourceDependencyFallbackRoots<'a, Root>,
}

/// Fallback roots available for one stylesheet source dependency batch.
///
/// Source-local roots describe the stylesheet source's own scope. Cause roots
/// describe a narrower runtime mutation boundary when one is available. The
/// Stylo-facing source input owns the policy for choosing between them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MoliSourceDependencyFallbackRoots<'a, Root> {
    source_local_roots: &'a [Root],
    cause_roots: &'a [Root],
}

/// Retained invalidation queries and base cleanup roots for child-list changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliRetainedStyleChildListInvalidationQueries<Root> {
    queries: Vec<MoliRetainedStyleChildListInvalidationQuery<Root>>,
    base_roots: Vec<Root>,
    empty_target_fallback_roots: Vec<Root>,
    relative_previous_sibling_cleanup_roots: Vec<Root>,
}

/// Sink used to drain child-list retained invalidation query batches into a
/// runtime-owned pending plan.
pub trait MoliRetainedStyleChildListInvalidationQueriesSink<Root> {
    /// Record one retained query and its source dependency requirement.
    fn record_child_list_retained_query(
        &mut self,
        query: MoliRetainedStyleInvalidationQuery<Root>,
        requirement: MoliSourceDependencyRequestRequirement,
    );

    /// Extend base structural-boundary cleanup roots.
    fn extend_child_list_base_roots(&mut self, roots: Vec<Root>);

    /// Extend empty-target fallback roots.
    fn extend_child_list_empty_target_fallback_roots(&mut self, roots: Vec<Root>);

    /// Extend direct previous-sibling relative cleanup roots.
    fn extend_child_list_relative_previous_sibling_cleanup_roots(&mut self, roots: Vec<Root>);
}

/// Builder for retained invalidation queries and cleanup roots produced from
/// child-list mutation facts.
#[derive(Clone, Debug)]
pub struct MoliRetainedStyleChildListInvalidationQueryBuilder<Root: Eq + Hash> {
    queries: IndexMap<
        MoliRetainedStyleInvalidationQuery<Root>,
        MoliSourceDependencyRequestRequirement,
    >,
    base_roots: IndexSet<Root>,
    empty_target_fallback_roots: IndexSet<Root>,
    relative_previous_sibling_cleanup_roots: IndexSet<Root>,
}

/// The child-list sibling boundary whose retained cleanup buckets are being
/// materialized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoliChildListSiblingBoundaryKind {
    /// The previous element sibling around an inserted range.
    AddedPreviousSibling {
        /// Whether the inserted range was appended at the end of the parent.
        inserted_at_end: bool,
    },
    /// The next element sibling around an inserted range.
    AddedNextSibling,
    /// The previous element sibling around a removed range.
    RemovedPreviousSibling,
    /// The next element sibling around a removed range.
    RemovedNextSibling,
    /// The sibling before the previous boundary around a removed range.
    RemovedEarlierSibling,
}

/// Retained cleanup bucket decisions for one child-list sibling boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliChildListSiblingBoundaryPlan<Root> {
    root: Root,
    include_base_root: bool,
    include_empty_target_fallback_root: bool,
    include_relative_previous_sibling_cleanup_root: bool,
}

/// One child-list retained invalidation query and whether it should only run
/// against sources with specific child-list dependency gates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliRetainedStyleChildListInvalidationQuery<Root> {
    query: MoliRetainedStyleInvalidationQuery<Root>,
    requirement: MoliSourceDependencyRequestRequirement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MoliRetainedStyleInvalidationQueryKind {
    Universal,
    Type { local_name: String },
    Attribute { name: String },
    Class { token: String },
    Id { value: String },
    State { state: ElementState },
    CustomState { name: String },
}

impl Hash for MoliRetainedStyleInvalidationQueryKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Universal => {
                0u8.hash(state);
            },
            Self::Type { local_name } => {
                1u8.hash(state);
                local_name.hash(state);
            },
            Self::Attribute { name } => {
                2u8.hash(state);
                name.hash(state);
            },
            Self::Class { token } => {
                3u8.hash(state);
                token.hash(state);
            },
            Self::Id { value } => {
                4u8.hash(state);
                value.hash(state);
            },
            Self::State {
                state: element_state,
            } => {
                5u8.hash(state);
                element_state.bits().hash(state);
            },
            Self::CustomState { name } => {
                6u8.hash(state);
                name.hash(state);
            },
        }
    }
}

impl<Root: Copy> MoliRetainedStyleInvalidationQuery<Root> {
    /// Create a universal retained query.
    #[inline]
    pub fn universal(root: Root) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::Universal,
            sibling_traversal: None,
        }
    }

    /// Create a type retained query.
    #[inline]
    pub fn element_type(root: Root, local_name: String) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::Type { local_name },
            sibling_traversal: None,
        }
    }

    /// Create an attribute retained query.
    #[inline]
    pub fn attribute(root: Root, name: String) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::Attribute { name },
            sibling_traversal: None,
        }
    }

    /// Create a class retained query.
    #[inline]
    pub fn class(root: Root, token: String) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::Class { token },
            sibling_traversal: None,
        }
    }

    /// Create an id retained query.
    #[inline]
    pub fn id(root: Root, value: String) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::Id { value },
            sibling_traversal: None,
        }
    }

    /// Create a state retained query.
    #[inline]
    pub fn state(root: Root, state: ElementState) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::State { state },
            sibling_traversal: None,
        }
    }

    /// Create a custom-state retained query.
    #[inline]
    pub fn custom_state(root: Root, name: String) -> Self {
        Self {
            root,
            kind: MoliRetainedStyleInvalidationQueryKind::CustomState { name },
            sibling_traversal: None,
        }
    }

    /// Attach sibling traversal context.
    #[inline]
    pub fn with_sibling_traversal(
        mut self,
        sibling_traversal: Option<MoliRetainedStyleSiblingTraversal<Root>>,
    ) -> Self {
        self.sibling_traversal = sibling_traversal;
        self
    }

    /// Return the query root.
    #[inline]
    pub fn root(&self) -> Root {
        self.root
    }

    /// Return sibling traversal context.
    #[inline]
    pub fn sibling_traversal(&self) -> Option<MoliRetainedStyleSiblingTraversal<Root>> {
        self.sibling_traversal
    }

    /// Return whether this query targets the universal invalidation map.
    #[inline]
    pub fn is_universal(&self) -> bool {
        matches!(
            self.kind,
            MoliRetainedStyleInvalidationQueryKind::Universal
        )
    }

    /// Return whether this state query can use direct previous-sibling
    /// fallback roots when child-list sibling context is available.
    #[inline]
    pub fn allows_direct_previous_following_sibling_fallback(&self) -> bool {
        matches!(
            self.kind,
            MoliRetainedStyleInvalidationQueryKind::State { state }
                if state.intersects(ElementState::HEADING_LEVEL_BITS)
        )
    }

    /// Borrow this retained query as the Stylo invalidation-map query shape.
    #[inline]
    pub fn as_stylo_query(&self) -> MoliStyleInvalidationQuery<'_> {
        match &self.kind {
            MoliRetainedStyleInvalidationQueryKind::Universal => {
                MoliStyleInvalidationQuery::Universal
            },
            MoliRetainedStyleInvalidationQueryKind::Type { local_name } => {
                MoliStyleInvalidationQuery::Type(local_name)
            },
            MoliRetainedStyleInvalidationQueryKind::Attribute { name } => {
                MoliStyleInvalidationQuery::Attribute(name)
            },
            MoliRetainedStyleInvalidationQueryKind::Class { token } => {
                MoliStyleInvalidationQuery::Class(token)
            },
            MoliRetainedStyleInvalidationQueryKind::Id { value } => {
                MoliStyleInvalidationQuery::Id(value)
            },
            MoliRetainedStyleInvalidationQueryKind::State { state } => {
                MoliStyleInvalidationQuery::State(*state)
            },
            MoliRetainedStyleInvalidationQueryKind::CustomState { name } => {
                MoliStyleInvalidationQuery::CustomState(name)
            },
        }
    }

    /// Borrow this retained query as a source-local invalidation query row.
    #[inline]
    pub fn as_source_query(&self) -> MoliSourceStyleInvalidationQuery<'_, Root> {
        let sibling_traversal = self.sibling_traversal();
        MoliSourceStyleInvalidationQuery::new(
            self.root(),
            self.as_stylo_query(),
            sibling_traversal.and_then(|traversal| traversal.previous_sibling()),
            sibling_traversal.and_then(|traversal| traversal.next_sibling()),
        )
    }
}

/// Dependency keys captured for one element before retained invalidation query
/// construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliElementDependencySnapshot<Root> {
    handle: Root,
    local_name: String,
    state: ElementState,
    attribute_names: Vec<String>,
    class_tokens: Vec<String>,
    custom_states: Vec<String>,
    id: Option<String>,
}

impl<Root: Hash> Hash for MoliElementDependencySnapshot<Root> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.handle.hash(state);
        self.local_name.hash(state);
        self.state.bits().hash(state);
        self.attribute_names.hash(state);
        self.class_tokens.hash(state);
        self.custom_states.hash(state);
        self.id.hash(state);
    }
}

impl<Root: Copy> MoliElementDependencySnapshot<Root> {
    /// Create dependency snapshot keys for one element.
    #[inline]
    pub fn new(
        handle: Root,
        local_name: String,
        state: ElementState,
        attribute_names: Vec<String>,
        class_tokens: Vec<String>,
        custom_states: Vec<String>,
        id: Option<String>,
    ) -> Self {
        Self {
            handle,
            local_name,
            state,
            attribute_names,
            class_tokens,
            custom_states,
            id,
        }
    }

    /// Return the element handle this snapshot describes.
    #[inline]
    pub fn handle(&self) -> Root {
        self.handle
    }

    /// Return captured class tokens.
    #[inline]
    pub fn class_tokens(&self) -> &[String] {
        &self.class_tokens
    }
}

/// Borrowed child-list mutation facts used to derive retained dependency
/// fallback context for a retained query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliRetainedStyleChildListMutationContext<'a, Root> {
    parent: Root,
    added_nodes: &'a [Root],
    removed_nodes: &'a [Root],
    removed_element_snapshots: &'a [MoliElementDependencySnapshot<Root>],
    previous_sibling: Option<Root>,
    next_sibling: Option<Root>,
}

impl<'a, Root> MoliRetainedStyleChildListMutationContext<'a, Root>
where
    Root: Copy + Eq + 'a,
{
    /// Create child-list mutation context from runtime-captured mutation facts.
    #[inline]
    pub fn new(
        parent: Root,
        added_nodes: &'a [Root],
        removed_nodes: &'a [Root],
        removed_element_snapshots: &'a [MoliElementDependencySnapshot<Root>],
        previous_sibling: Option<Root>,
        next_sibling: Option<Root>,
    ) -> Self {
        Self {
            parent,
            added_nodes,
            removed_nodes,
            removed_element_snapshots,
            previous_sibling,
            next_sibling,
        }
    }

    fn contains_query_root(&self, root: Root) -> bool {
        self.parent == root
            || self.added_nodes.contains(&root)
            || self.removed_nodes.contains(&root)
            || self.previous_sibling == Some(root)
            || self.next_sibling == Some(root)
            || self
                .removed_element_snapshots
                .iter()
                .any(|snapshot| snapshot.handle() == root)
    }

    /// Return the retained dependency fallback context for this query when it
    /// belongs to this child-list mutation context.
    pub fn fallback_context_for_query(
        &self,
        query: &MoliRetainedStyleInvalidationQuery<Root>,
    ) -> Option<MoliDependencyInvalidationFallbackContext<Root>> {
        if !self.contains_query_root(query.root()) {
            return None;
        }
        let traversal = query.sibling_traversal();
        Some(
            MoliDependencyInvalidationFallbackContext::from_mutation_relation(
                Some(self.parent),
                traversal
                    .and_then(|traversal| traversal.previous_sibling())
                    .or(self.previous_sibling),
                traversal
                    .and_then(|traversal| traversal.next_sibling())
                    .or(self.next_sibling),
            ),
        )
    }
}

/// Return the first child-list mutation fallback context matching a retained
/// query.
pub fn moli_child_list_dependency_fallback_context_for_query<'a, Root>(
    contexts: impl IntoIterator<Item = MoliRetainedStyleChildListMutationContext<'a, Root>>,
    query: &MoliRetainedStyleInvalidationQuery<Root>,
) -> Option<MoliDependencyInvalidationFallbackContext<Root>>
where
    Root: Copy + Eq + 'a,
{
    contexts
        .into_iter()
        .find_map(|context| context.fallback_context_for_query(query))
}

/// Build retained dependency queries from captured element dependency keys.
pub fn moli_retained_queries_for_element_dependency_snapshot<Root: Copy>(
    snapshot: &MoliElementDependencySnapshot<Root>,
    sibling_traversal: Option<MoliRetainedStyleSiblingTraversal<Root>>,
) -> Vec<MoliRetainedStyleInvalidationQuery<Root>> {
    moli_retained_queries_for_element_dependency_snapshot_with_universal(
        snapshot,
        sibling_traversal,
        true,
    )
}

/// Build non-universal retained dependency queries from captured element keys.
pub fn moli_retained_non_universal_queries_for_element_dependency_snapshot<Root: Copy>(
    snapshot: &MoliElementDependencySnapshot<Root>,
    sibling_traversal: Option<MoliRetainedStyleSiblingTraversal<Root>>,
) -> Vec<MoliRetainedStyleInvalidationQuery<Root>> {
    moli_retained_queries_for_element_dependency_snapshot_with_universal(
        snapshot,
        sibling_traversal,
        false,
    )
}

fn moli_retained_queries_for_element_dependency_snapshot_with_universal<Root: Copy>(
    snapshot: &MoliElementDependencySnapshot<Root>,
    sibling_traversal: Option<MoliRetainedStyleSiblingTraversal<Root>>,
    include_universal: bool,
) -> Vec<MoliRetainedStyleInvalidationQuery<Root>> {
    let mut queries = Vec::new();
    if include_universal {
        queries.push(
            MoliRetainedStyleInvalidationQuery::universal(snapshot.handle)
                .with_sibling_traversal(sibling_traversal),
        );
    }
    queries.push(
        MoliRetainedStyleInvalidationQuery::element_type(
            snapshot.handle,
            snapshot.local_name.clone(),
        )
        .with_sibling_traversal(sibling_traversal),
    );
    if !snapshot.state.is_empty() {
        queries.push(
            MoliRetainedStyleInvalidationQuery::state(snapshot.handle, snapshot.state)
                .with_sibling_traversal(sibling_traversal),
        );
    }
    for attribute_name in &snapshot.attribute_names {
        queries.push(
            MoliRetainedStyleInvalidationQuery::attribute(
                snapshot.handle,
                attribute_name.to_owned(),
            )
            .with_sibling_traversal(sibling_traversal),
        );
    }
    for token in &snapshot.class_tokens {
        queries.push(
            MoliRetainedStyleInvalidationQuery::class(snapshot.handle, token.to_owned())
                .with_sibling_traversal(sibling_traversal),
        );
    }
    if let Some(id) = &snapshot.id {
        queries.push(
            MoliRetainedStyleInvalidationQuery::id(snapshot.handle, id.to_owned())
                .with_sibling_traversal(sibling_traversal),
        );
    }
    for state in &snapshot.custom_states {
        queries.push(
            MoliRetainedStyleInvalidationQuery::custom_state(snapshot.handle, state.clone())
                .with_sibling_traversal(sibling_traversal),
        );
    }
    queries
}

impl MoliSourceDependencyRequestRequirement {
    /// No additional source dependency gate is required.
    #[inline]
    pub fn exact() -> Self {
        Self::default()
    }

    /// The source must contain child-list structural dependencies.
    #[inline]
    pub fn child_list_structural() -> Self {
        Self {
            requires_child_list_structural_dependency: true,
            requires_relative_previous_sibling_dependency: false,
        }
    }

    /// The source must contain direct previous-sibling relative dependencies.
    #[inline]
    pub fn relative_previous_sibling() -> Self {
        Self {
            requires_child_list_structural_dependency: false,
            requires_relative_previous_sibling_dependency: true,
        }
    }

    /// The source must contain both child-list structural and direct
    /// previous-sibling relative dependencies.
    #[inline]
    pub fn child_list_structural_relative_previous_sibling() -> Self {
        Self {
            requires_child_list_structural_dependency: true,
            requires_relative_previous_sibling_dependency: true,
        }
    }

    /// Merge requirements for duplicate retained queries.
    #[inline]
    fn merged_with(self, incoming: Self) -> Self {
        Self {
            requires_child_list_structural_dependency: self
                .requires_child_list_structural_dependency
                && incoming.requires_child_list_structural_dependency,
            requires_relative_previous_sibling_dependency: self
                .requires_relative_previous_sibling_dependency
                || incoming.requires_relative_previous_sibling_dependency,
        }
    }

    /// Return whether child-list structural dependencies are required.
    #[inline]
    pub fn requires_child_list_structural_dependency(self) -> bool {
        self.requires_child_list_structural_dependency
    }

    /// Return whether direct previous-sibling relative dependencies are
    /// required.
    #[inline]
    pub fn requires_relative_previous_sibling_dependency(self) -> bool {
        self.requires_relative_previous_sibling_dependency
    }
}

/// Merge source dependency request requirements for duplicate retained queries.
#[inline]
pub fn moli_merge_source_dependency_request_requirement(
    existing: MoliSourceDependencyRequestRequirement,
    incoming: MoliSourceDependencyRequestRequirement,
) -> MoliSourceDependencyRequestRequirement {
    existing.merged_with(incoming)
}

impl<Root> MoliDependencyInvalidationFallbackContext<Root> {
    /// Create fallback context from the mutation-time sibling relation.
    #[inline]
    pub fn from_mutation_relation(
        parent: Option<Root>,
        previous_sibling: Option<Root>,
        next_sibling: Option<Root>,
    ) -> Self {
        Self {
            parent,
            previous_sibling,
            next_sibling,
        }
    }

    /// Return the mutation-time parent.
    #[inline]
    pub fn parent(&self) -> Option<Root>
    where
        Root: Copy,
    {
        self.parent
    }

    /// Return the mutation-time previous sibling.
    #[inline]
    pub fn previous_sibling(&self) -> Option<Root>
    where
        Root: Copy,
    {
        self.previous_sibling
    }

    /// Return the mutation-time next sibling.
    #[inline]
    pub fn next_sibling(&self) -> Option<Root>
    where
        Root: Copy,
    {
        self.next_sibling
    }
}

impl<Root> Default for MoliDependencyInvalidationFallbackContext<Root> {
    #[inline]
    fn default() -> Self {
        Self {
            parent: None,
            previous_sibling: None,
            next_sibling: None,
        }
    }
}

impl MoliStyleMutationElementSnapshot {
    /// Record one attribute's old value, keeping the first old value observed
    /// in a mutation batch.
    #[inline]
    pub fn record_attribute_change(&mut self, name: &str, old_value: Option<String>) {
        self.attribute_changes
            .entry(name.to_owned())
            .or_insert(old_value);
    }

    /// Record the element's old state if no old state has already been
    /// captured for this element.
    #[inline]
    pub fn try_record_old_state(&mut self, old_state: ElementState) -> Option<()> {
        if self.old_state.is_some() {
            return None;
        }
        self.old_state = Some(old_state);
        Some(())
    }

    /// Record the old custom states if this element has not already captured
    /// them in the current mutation batch.
    #[inline]
    pub fn record_old_custom_states(&mut self, old_custom_states: Vec<String>) {
        self.old_custom_states.get_or_insert(old_custom_states);
    }

    /// Merge another element snapshot into this one, preserving first observed
    /// old values and states.
    pub fn merge_from(&mut self, incoming: Self) {
        for (name, old_value) in incoming.attribute_changes {
            self.record_attribute_change(&name, old_value);
        }
        if self.old_state.is_none() {
            self.old_state = incoming.old_state;
        }
        if self.old_custom_states.is_none() {
            self.old_custom_states = incoming.old_custom_states;
        }
    }

    /// Return captured attribute changes in insertion order.
    pub fn attribute_changes(
        &self,
    ) -> impl Iterator<Item = MoliStyleMutationAttributeChange<'_>> {
        self.attribute_changes.iter().map(|(name, old_value)| {
            MoliStyleMutationAttributeChange {
                name,
                old_value: old_value.as_deref(),
            }
        })
    }

    /// Return the number of captured attribute changes.
    #[inline]
    pub fn attribute_change_count(&self) -> usize {
        self.attribute_changes.len()
    }

    /// Return the captured old element state.
    #[inline]
    pub fn old_state(&self) -> Option<ElementState> {
        self.old_state
    }

    /// Return the captured old custom states.
    #[inline]
    pub fn old_custom_states(&self) -> Option<&[String]> {
        self.old_custom_states.as_deref()
    }
}

impl<Root: Copy> MoliStyleInvalidationSnapshot<Root> {
    /// Create a materialized invalidation snapshot.
    #[inline]
    pub fn new(
        element: Root,
        state: Option<ElementState>,
        custom_states: Option<Vec<String>>,
        changed_attributes: Vec<String>,
        attributes: Vec<MoliStyleInvalidationSnapshotAttribute>,
    ) -> Self {
        Self {
            element,
            state,
            custom_states,
            changed_attributes,
            attributes,
        }
    }

    /// Return the element this snapshot belongs to.
    #[inline]
    pub fn element(&self) -> Root {
        self.element
    }

    /// Return the old pseudo-class state, when captured.
    #[inline]
    pub fn state(&self) -> Option<ElementState> {
        self.state
    }

    /// Return the old custom-state set, when captured.
    #[inline]
    pub fn custom_states(&self) -> Option<&[String]> {
        self.custom_states.as_deref()
    }

    /// Return old attribute values after applying captured mutation facts.
    #[inline]
    pub fn attributes(&self) -> &[MoliStyleInvalidationSnapshotAttribute] {
        &self.attributes
    }

    /// Return attribute local names changed by this mutation.
    #[inline]
    pub fn changed_attributes(&self) -> &[String] {
        &self.changed_attributes
    }
}

impl MoliStyleInvalidationSnapshotAttribute {
    /// Create one materialized invalidation snapshot attribute.
    #[inline]
    pub fn new(
        local_name: String,
        name: String,
        namespace: String,
        prefix: Option<String>,
        value: String,
    ) -> Self {
        Self {
            local_name,
            name,
            namespace,
            prefix,
            value,
        }
    }

    /// Return the local name.
    #[inline]
    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    /// Return the qualified attribute name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the namespace string.
    #[inline]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Return the prefix, when present.
    #[inline]
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// Return the attribute value.
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Replace the materialized attribute value.
    #[inline]
    pub fn set_value(&mut self, value: String) {
        self.value = value;
    }

    /// Return whether this attribute has no namespace and the given local name.
    #[inline]
    pub fn is_no_namespace_local_name(&self, local_name: &str) -> bool {
        self.namespace.is_empty() && self.local_name == local_name
    }
}

impl<'a> MoliStyleMutationAttributeChange<'a> {
    /// Return the changed attribute local name.
    #[inline]
    pub fn name(&self) -> &str {
        self.name
    }

    /// Return the captured old attribute value.
    #[inline]
    pub fn old_value(&self) -> Option<&str> {
        self.old_value
    }
}

impl<Root> MoliDependencyInvalidationContextRoots<Root> {
    /// Create context-derived fallback roots.
    #[inline]
    fn new(requires_source_fallback: bool, roots: Vec<Root>) -> Self {
        Self {
            requires_source_fallback,
            roots,
        }
    }

    /// Returns whether these roots are insufficient for exact-safety fallback
    /// and source fallback roots are required.
    #[inline]
    fn requires_source_fallback(&self) -> bool {
        self.requires_source_fallback
    }

    /// Return the context-derived roots.
    #[inline]
    fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// Consume this value and return the context-derived roots.
    #[inline]
    fn into_roots(self) -> Vec<Root> {
        self.roots
    }
}

impl<'a, Root> MoliSourceDependencyInvalidationRequest<'a, Root>
where
    Root: Copy,
{
    /// Create a source dependency invalidation request.
    #[inline]
    pub fn new(
        query: &'a MoliRetainedStyleInvalidationQuery<Root>,
        context: Option<MoliDependencyInvalidationFallbackContext<Root>>,
        requirement: MoliSourceDependencyRequestRequirement,
    ) -> Self {
        Self {
            query,
            context,
            requirement,
        }
    }

    /// Return the retained query for this request.
    #[inline]
    fn query(&self) -> &'a MoliRetainedStyleInvalidationQuery<Root> {
        self.query
    }

    /// Return the optional mutation-time fallback context.
    #[inline]
    fn context(&self) -> Option<MoliDependencyInvalidationFallbackContext<Root>> {
        self.context
    }

    /// Return whether this request requires child-list structural dependencies
    /// in the source.
    #[inline]
    fn requires_child_list_structural_dependency(&self) -> bool {
        self.requirement.requires_child_list_structural_dependency()
    }

    /// Return whether this request requires direct previous-sibling relative
    /// dependencies in the source.
    #[inline]
    fn requires_relative_previous_sibling_dependency(&self) -> bool {
        self.requirement
            .requires_relative_previous_sibling_dependency()
    }
}

impl MoliChildListStructuralBoundaryDependencySummary {
    #[inline]
    pub(crate) fn note_class_dependency(&mut self, class: Atom) {
        if !self.class_dependencies.contains(&class) {
            self.class_dependencies.push(class);
        }
    }

    #[inline]
    pub(crate) fn note_id_dependency(&mut self, id: Atom) {
        if !self.id_dependencies.contains(&id) {
            self.id_dependencies.push(id);
        }
    }

    #[inline]
    pub(crate) fn note_type_dependency(&mut self, local_name: LocalName) {
        if !self.type_dependencies.contains(&local_name) {
            self.type_dependencies.push(local_name);
        }
    }

    #[inline]
    pub(crate) fn note_attribute_dependency(&mut self, attribute: LocalName) {
        if !self.attribute_dependencies.contains(&attribute) {
            self.attribute_dependencies.push(attribute);
        }
    }

    #[inline]
    pub(crate) fn note_universal_dependency(&mut self) {
        self.universal_dependency = true;
    }

    #[inline]
    pub(crate) fn matches_query(&self, query: MoliStyleInvalidationQuery<'_>) -> bool {
        if self.universal_dependency {
            return true;
        }
        match query {
            MoliStyleInvalidationQuery::Universal => false,
            MoliStyleInvalidationQuery::Type(local_name) => self
                .type_dependencies
                .contains(&LocalName::from(local_name)),
            MoliStyleInvalidationQuery::Attribute(name) => {
                self.attribute_dependencies.contains(&LocalName::from(name))
            },
            MoliStyleInvalidationQuery::Class(token) => {
                self.class_dependencies.contains(&Atom::from(token))
            },
            MoliStyleInvalidationQuery::Id(value) => {
                self.id_dependencies.contains(&Atom::from(value))
            },
            MoliStyleInvalidationQuery::State(_)
            | MoliStyleInvalidationQuery::CustomState(_) => false,
        }
    }
}

impl MoliSourceDependencySummary {
    /// Create conservative metadata for a source known to have child-list
    /// structural dependencies without selector-derived boundary keys.
    /// Prefer [`Self::from_cascade_data`] whenever parsed selector metadata is
    /// available.
    #[inline]
    pub fn conservative_child_list_structural() -> Self {
        let mut child_list_structural_boundary_dependencies =
            MoliChildListStructuralBoundaryDependencySummary::default();
        child_list_structural_boundary_dependencies.note_universal_dependency();
        Self::from_parts(
            MoliDependencyInvalidationSummary::default(),
            true,
            child_list_structural_boundary_dependencies,
        )
    }

    #[inline]
    fn from_parts(
        dependency_summary: MoliDependencyInvalidationSummary,
        has_child_list_structural_dependency: bool,
        child_list_structural_boundary_dependencies:
            MoliChildListStructuralBoundaryDependencySummary,
    ) -> Self {
        Self {
            dependency_summary,
            has_child_list_structural_dependency,
            child_list_structural_boundary_dependencies,
        }
    }

    /// Create source dependency metadata from Stylo cascade data.
    #[inline]
    pub fn from_cascade_data(cascade_data: &CascadeData) -> Self {
        Self::from_parts(
            cascade_data.moli_dependency_invalidation_summary(),
            cascade_data.has_child_list_structural_dependency(),
            cascade_data
                .moli_child_list_structural_boundary_dependency_summary()
                .clone(),
        )
    }

    /// Query dependencies for a changed class through this source summary.
    #[inline]
    pub fn query_class(&self, class: &Atom) -> MoliDependencyQueryResult {
        self.dependency_summary.query_class(class)
    }

    /// Query dependencies for a changed id through this source summary.
    #[inline]
    pub fn query_id(&self, id: &Atom) -> MoliDependencyQueryResult {
        self.dependency_summary.query_id(id)
    }

    /// Query dependencies for a changed attribute through this source summary.
    #[inline]
    pub fn query_attribute(&self, attribute: &LocalName) -> MoliDependencyQueryResult {
        self.dependency_summary.query_attribute(attribute)
    }

    /// Query dependencies for an inserted or removed element local name.
    #[inline]
    pub fn query_type(&self, local_name: &LocalName) -> MoliDependencyQueryResult {
        self.dependency_summary.query_type(local_name)
    }

    /// Query dependencies for an inserted or removed element matching `*`.
    #[inline]
    pub fn query_universal(&self) -> MoliDependencyQueryResult {
        self.dependency_summary.query_universal()
    }

    /// Query dependencies for a changed element state bitset.
    #[inline]
    pub fn query_state(&self, state: ElementState) -> MoliDependencyQueryResult {
        self.dependency_summary.query_state(state)
    }

    /// Query dependencies for a changed CSS custom state.
    #[inline]
    pub fn query_custom_state(&self, state: &AtomIdent) -> MoliDependencyQueryResult {
        self.dependency_summary.query_custom_state(state)
    }

    /// Query dependencies for focus-like state changes.
    #[inline]
    pub fn query_focus(&self) -> MoliDependencyQueryResult {
        self.dependency_summary.query_focus()
    }

    /// Query dependencies for `:focus-within` state changes.
    #[inline]
    pub fn query_focus_within(&self) -> MoliDependencyQueryResult {
        self.dependency_summary.query_focus_within()
    }

    /// Query dependencies for `:target` state changes.
    #[inline]
    pub fn query_target(&self) -> MoliDependencyQueryResult {
        self.dependency_summary.query_target()
    }

    /// Return whether this source has child-list structural dependencies.
    #[inline]
    pub fn has_child_list_structural_dependency(&self) -> bool {
        self.has_child_list_structural_dependency
    }

    #[inline]
    fn has_child_list_structural_boundary_dependency_for_request<Root>(
        &self,
        request: &MoliSourceDependencyInvalidationRequest<'_, Root>,
    ) -> bool
    where
        Root: Copy,
    {
        self.has_child_list_structural_dependency
            && request.requires_child_list_structural_dependency()
            && self
                .child_list_structural_boundary_dependencies
                .matches_query(request.query().as_stylo_query())
    }

    /// Return whether this source has any relative selector dependency.
    #[inline]
    pub fn has_relative_selector_dependency(&self) -> bool {
        self.dependency_summary.has_relative_selector_dependency()
    }

    /// Return whether this source has any focus-like state dependency.
    #[inline]
    pub fn has_focus_dependency(&self) -> bool {
        self.dependency_summary.query_focus().has_any_dependency()
    }

    /// Return whether this source has any `:focus-within` dependency.
    #[inline]
    pub fn has_focus_within_dependency(&self) -> bool {
        self.dependency_summary
            .query_focus_within()
            .has_any_dependency()
    }

    /// Return whether this source has any `:target` dependency.
    #[inline]
    pub fn has_target_dependency(&self) -> bool {
        self.dependency_summary.query_target().has_any_dependency()
    }

    /// Return whether this source has any following-sibling dependency.
    #[inline]
    pub fn has_sibling_dependency(&self) -> bool {
        self.dependency_summary.has_sibling_dependency()
    }

    /// Query this source dependency summary for one retained invalidation
    /// query shape.
    #[inline]
    pub fn query_result(
        &self,
        query: MoliStyleInvalidationQuery<'_>,
    ) -> MoliDependencyQueryResult {
        match query {
            MoliStyleInvalidationQuery::Universal => self.query_universal(),
            MoliStyleInvalidationQuery::Type(local_name) => {
                self.query_type(&LocalName::from(local_name))
            },
            MoliStyleInvalidationQuery::Attribute(name) => {
                self.query_attribute(&LocalName::from(name))
            },
            MoliStyleInvalidationQuery::Class(token) => self.query_class(&Atom::from(token)),
            MoliStyleInvalidationQuery::Id(value) => self.query_id(&Atom::from(value)),
            MoliStyleInvalidationQuery::State(state) => self.query_state(state),
            MoliStyleInvalidationQuery::CustomState(name) => {
                self.query_custom_state(&AtomIdent::from(name))
            },
        }
    }

    /// Return whether child-list structural dependencies can participate in
    /// any request that requires them.
    ///
    /// The source-level structural bit is intentionally paired with
    /// selector-derived boundary keys. A structural selector elsewhere in the
    /// same source must not turn an unrelated type or universal query into an
    /// empty-target fallback.
    #[inline]
    fn has_child_list_structural_dependency_for_requests<Root>(
        &self,
        requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
    ) -> bool
    where
        Root: Copy,
    {
        requests
            .iter()
            .any(|request| self.has_child_list_structural_boundary_dependency_for_request(request))
    }

    /// Return whether direct previous-sibling relative dependencies are
    /// present for any request.
    #[inline]
    fn has_relative_previous_sibling_dependency_for_requests<Root>(
        &self,
        requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
    ) -> bool
    where
        Root: Copy,
    {
        requests.iter().any(|request| {
            self.query_result(request.query().as_stylo_query())
                .has_relative_previous_sibling_dependency()
        })
    }

    /// Return whether slotted dependencies are present for any request.
    #[inline]
    fn has_slotted_dependency_for_requests<Root>(
        &self,
        requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
    ) -> bool
    where
        Root: Copy,
    {
        requests.iter().any(|request| {
            self.query_result(request.query().as_stylo_query())
                .has_slotted_dependency()
        })
    }

    /// Return whether this source needs an empty-target fallback that cannot
    /// coexist with source-local work for the requested dependency batch.
    #[inline]
    fn requires_nonstructural_empty_target_fallback_for_requests<Root>(
        &self,
        requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
    ) -> bool
    where
        Root: Copy,
    {
        self.has_relative_previous_sibling_dependency_for_requests(requests)
            || self.has_slotted_dependency_for_requests(requests)
    }

    /// Return structural-boundary cleanup roots for the requested source
    /// dependency batch.
    #[inline]
    fn structural_boundary_cleanup_roots_for_requests<Root>(
        &self,
        requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
        relative_previous_sibling_cleanup_roots: &[Root],
    ) -> Vec<Root>
    where
        Root: Copy,
    {
        if self.has_relative_previous_sibling_dependency_for_requests(requests) {
            relative_previous_sibling_cleanup_roots.to_vec()
        } else {
            Vec::new()
        }
    }
}

impl<'a, Root> MoliSourceDependencyInvalidationBatchSource<'a, Root> {
    /// Create one source dependency invalidation batch source.
    #[inline]
    pub fn new(
        dependency_summary: &'a MoliSourceDependencySummary,
        source_local_fallback_roots: &'a [Root],
        cause_fallback_roots: &'a [Root],
    ) -> Self {
        Self {
            dependency_summary,
            fallback_roots: MoliSourceDependencyFallbackRoots::new(
                source_local_fallback_roots,
                cause_fallback_roots,
            ),
        }
    }

    /// Return this source's dependency summary.
    #[inline]
    fn dependency_summary(&self) -> &'a MoliSourceDependencySummary {
        self.dependency_summary
    }

    /// Return the selected fallback roots, preferring cause roots when present.
    #[inline]
    fn selected_fallback_roots(&self) -> &'a [Root] {
        self.fallback_roots.selected_roots()
    }
}

impl<'a, Root> MoliSourceDependencyFallbackRoots<'a, Root> {
    fn new(source_local_roots: &'a [Root], cause_roots: &'a [Root]) -> Self {
        Self {
            source_local_roots,
            cause_roots,
        }
    }

    fn selected_roots(&self) -> &'a [Root] {
        if self.cause_roots.is_empty() {
            self.source_local_roots
        } else {
            self.cause_roots
        }
    }
}

impl<Root> MoliRetainedStyleChildListInvalidationQueries<Root> {
    /// Create a child-list retained invalidation batch.
    #[inline]
    fn new(
        queries: Vec<MoliRetainedStyleChildListInvalidationQuery<Root>>,
        base_roots: Vec<Root>,
        empty_target_fallback_roots: Vec<Root>,
        relative_previous_sibling_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self {
            queries,
            base_roots,
            empty_target_fallback_roots,
            relative_previous_sibling_cleanup_roots,
        }
    }

    /// Consume this batch into a runtime-owned pending plan sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliRetainedStyleChildListInvalidationQueriesSink<Root>,
    ) {
        for row in self.queries {
            let (query, requirement) = row.into_query_and_requirement();
            target.record_child_list_retained_query(query, requirement);
        }
        target.extend_child_list_base_roots(self.base_roots);
        target.extend_child_list_empty_target_fallback_roots(self.empty_target_fallback_roots);
        target.extend_child_list_relative_previous_sibling_cleanup_roots(
            self.relative_previous_sibling_cleanup_roots,
        );
    }
}

impl<Root> Default for MoliRetainedStyleChildListInvalidationQueryBuilder<Root>
where
    Root: Eq + Hash,
{
    #[inline]
    fn default() -> Self {
        Self {
            queries: IndexMap::new(),
            base_roots: IndexSet::new(),
            empty_target_fallback_roots: IndexSet::new(),
            relative_previous_sibling_cleanup_roots: IndexSet::new(),
        }
    }
}

impl<Root> MoliRetainedStyleChildListInvalidationQueryBuilder<Root>
where
    Root: Eq + Hash,
{
    /// Create an empty child-list retained invalidation query builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert retained queries, merging the source dependency requirement for
    /// duplicate rows.
    pub fn insert_queries(
        &mut self,
        queries: impl IntoIterator<Item = MoliRetainedStyleInvalidationQuery<Root>>,
        requirement: MoliSourceDependencyRequestRequirement,
    ) {
        for query in queries {
            self.queries
                .entry(query)
                .and_modify(|existing| {
                    *existing = moli_merge_source_dependency_request_requirement(
                        *existing,
                        requirement,
                    );
                })
                .or_insert(requirement);
        }
    }

    /// Insert a base cleanup root.
    #[inline]
    pub fn insert_base_root(&mut self, root: Root) {
        self.base_roots.insert(root);
    }

    /// Insert a fallback root for empty-target structural invalidation.
    #[inline]
    pub fn insert_empty_target_fallback_root(&mut self, root: Root) {
        self.empty_target_fallback_roots.insert(root);
    }

    /// Insert a cleanup root for direct previous-sibling relative dependency
    /// handling.
    #[inline]
    pub fn insert_relative_previous_sibling_cleanup_root(&mut self, root: Root) {
        self.relative_previous_sibling_cleanup_roots.insert(root);
    }

    /// Consume the builder into a typed child-list invalidation batch.
    pub fn into_queries(self) -> Option<MoliRetainedStyleChildListInvalidationQueries<Root>> {
        (!self.queries.is_empty()).then(|| {
            MoliRetainedStyleChildListInvalidationQueries::new(
                self.queries
                    .into_iter()
                    .map(|(query, requirement)| {
                        MoliRetainedStyleChildListInvalidationQuery::new(query, requirement)
                    })
                    .collect(),
                self.base_roots.into_iter().collect(),
                self.empty_target_fallback_roots.into_iter().collect(),
                self.relative_previous_sibling_cleanup_roots
                    .into_iter()
                    .collect(),
            )
        })
    }
}

impl<Root> MoliChildListSiblingBoundaryPlan<Root> {
    #[inline]
    fn new(
        root: Root,
        include_base_root: bool,
        include_empty_target_fallback_root: bool,
        include_relative_previous_sibling_cleanup_root: bool,
    ) -> Self {
        Self {
            root,
            include_base_root,
            include_empty_target_fallback_root,
            include_relative_previous_sibling_cleanup_root,
        }
    }

    /// Return the sibling boundary root.
    #[cfg(test)]
    #[inline]
    fn root(&self) -> &Root {
        &self.root
    }

    /// Return whether this boundary contributes to base cleanup roots.
    #[cfg(test)]
    #[inline]
    fn includes_base_root(&self) -> bool {
        self.include_base_root
    }

    /// Return whether this boundary contributes to empty-target fallback roots.
    #[cfg(test)]
    #[inline]
    fn includes_empty_target_fallback_root(&self) -> bool {
        self.include_empty_target_fallback_root
    }

    /// Return whether this boundary contributes to relative previous-sibling
    /// cleanup roots.
    #[cfg(test)]
    #[inline]
    fn includes_relative_previous_sibling_cleanup_root(&self) -> bool {
        self.include_relative_previous_sibling_cleanup_root
    }

    /// Apply this plan to a child-list retained query builder.
    pub fn apply_to_builder(
        &self,
        builder: &mut MoliRetainedStyleChildListInvalidationQueryBuilder<Root>,
    ) where
        Root: Clone + Eq + Hash,
    {
        if self.include_base_root {
            builder.insert_base_root(self.root.clone());
        }
        if self.include_empty_target_fallback_root {
            builder.insert_empty_target_fallback_root(self.root.clone());
        }
        if self.include_relative_previous_sibling_cleanup_root {
            builder.insert_relative_previous_sibling_cleanup_root(self.root.clone());
        }
    }
}

/// Return the retained cleanup bucket plan for one child-list sibling boundary.
#[inline]
pub fn moli_child_list_sibling_boundary_plan<Root>(
    root: Option<Root>,
    sibling_is_changed_by_mutation_batch: bool,
    kind: MoliChildListSiblingBoundaryKind,
) -> Option<MoliChildListSiblingBoundaryPlan<Root>> {
    if sibling_is_changed_by_mutation_batch {
        return None;
    }

    let root = root?;
    let (include_base_root, include_empty_target_fallback_root, include_relative_cleanup_root) =
        match kind {
            MoliChildListSiblingBoundaryKind::AddedPreviousSibling { inserted_at_end } => {
                (inserted_at_end, true, true)
            },
            MoliChildListSiblingBoundaryKind::AddedNextSibling => (true, true, false),
            MoliChildListSiblingBoundaryKind::RemovedPreviousSibling => (true, true, true),
            MoliChildListSiblingBoundaryKind::RemovedNextSibling => (true, true, false),
            MoliChildListSiblingBoundaryKind::RemovedEarlierSibling => (false, false, true),
        };

    Some(MoliChildListSiblingBoundaryPlan::new(
        root,
        include_base_root,
        include_empty_target_fallback_root,
        include_relative_cleanup_root,
    ))
}

impl<Root> MoliRetainedStyleChildListInvalidationQuery<Root> {
    /// Create one child-list retained invalidation query row.
    #[inline]
    fn new(
        query: MoliRetainedStyleInvalidationQuery<Root>,
        requirement: MoliSourceDependencyRequestRequirement,
    ) -> Self {
        Self { query, requirement }
    }

    /// Consume this row into query and requirement parts.
    #[inline]
    fn into_query_and_requirement(
        self,
    ) -> (
        MoliRetainedStyleInvalidationQuery<Root>,
        MoliSourceDependencyRequestRequirement,
    ) {
        (self.query, self.requirement)
    }
}

/// Reason a dependency query cannot be represented as exact dependency kinds.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliDependencyFallbackReason {
    /// Stylo recorded an unknown dependency for this query.
    UnknownDependency,
    /// Stylo's invalidation map requires full-selector invalidation.
    FullSelector,
    /// Relative selector dependencies are present but not exposed precisely.
    RelativeAnySelector,
    /// Scope dependencies are present but not exposed precisely.
    ScopeDependency,
    /// State dependencies are present but not exposed precisely.
    UnsupportedStateDependency,
    /// The dependency shape cannot currently be represented by an exact retained
    /// invalidator path.
    UnsupportedDependency,
    /// `:nth-child(... of ...)` dependency exactness could not be represented
    /// by the retained invalidator's current root set.
    NthOfDependency,
    /// A selector-list dependency nested inside a relative selector cannot be
    /// represented by the retained invalidator's exact root set.
    NestedRelativeSelectorDependency,
}

/// Why a source-aware invalidation batch could not produce exact roots.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoliSourceInvalidationFallbackReason {
    /// Stylo recorded an unknown dependency for this query.
    UnknownDependency,
    /// Stylo's invalidation map requires full-selector invalidation.
    FullSelector,
    /// Relative selector dependencies are present but not exposed precisely.
    RelativeAnySelector,
    /// Scope dependencies are present but not exposed precisely.
    ScopeDependency,
    /// State dependencies are present but not exposed precisely.
    UnsupportedStateDependency,
    /// Shadow dependency exactness could not be represented by this query.
    UnsupportedShadowDependency,
    /// The renderer had to use conservative fallback roots from the active
    /// source scope instead of cause-local or source-local roots.
    SourceScopeFallback,
    /// Stylo dependency metadata says the dependency cannot be represented by
    /// the exact retained invalidator path.
    UnsupportedDependency,
    /// `:nth-child(... of ...)` dependency exactness still needs reasoned
    /// fallback handling.
    NthOfDependency,
    /// A selector-list dependency nested inside a relative selector still needs
    /// reasoned fallback handling.
    NestedRelativeSelectorDependency,
    /// The retained invalidator produced no roots, but the result was not
    /// proven to be an exact no-op for this source/query batch.
    InexactEmptyResult,
    /// The batch needed source fallback roots, but none were provided by the
    /// source/scope owner.
    MissingFallbackRoots,
    /// The retained style system was unavailable when source queries were
    /// drained.
    MissingRetainedStyleSystem,
    /// The retained style system did not have per-source cascade data for this
    /// source.
    MissingRetainedCascadeData,
}

/// Return whether an attribute mutation can be represented by the retained
/// source-local invalidator.
#[inline]
pub fn moli_attribute_change_can_use_retained_invalidator(
    _attribute_name: &str,
    has_non_css_runtime_side_effect: bool,
) -> bool {
    !has_non_css_runtime_side_effect
}

/// Return whether an attribute mutation may avoid fallback roots once a
/// retained dependency path is available.
#[inline]
pub fn moli_attribute_change_can_skip_fallback_without_dependency(
    attribute_name: &str,
) -> bool {
    attribute_name == "class"
        || attribute_name == "dir"
        || attribute_name == "lang"
        || attribute_name.starts_with("data-")
        || attribute_name.starts_with("aria-")
}

/// Runtime mutation facts used to select conservative source-local fallback
/// roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoliRuntimeFallbackRootInput<'a, Root> {
    /// Attribute mutation fallback policy.
    Attribute {
        /// Mutated element.
        element: Root,
        /// Mutated attribute local name.
        attribute_name: &'a str,
        /// Whether Stylo dependency metadata reported a relevant dependency.
        has_dependency_change: bool,
        /// Whether this attribute also has non-CSS runtime side effects.
        has_non_css_runtime_side_effect: bool,
    },
    /// Child-list mutation fallback policy.
    ChildList {
        /// Nodes added by the mutation.
        added_nodes: &'a [Root],
    },
    /// Slot-assignment mutation fallback policy.
    SlotAssignment {
        /// Mutated slot element.
        slot: Root,
        /// Whether previous and current assigned-node snapshots are available.
        has_assignment_snapshot: bool,
    },
    /// Newly connected subtree fallback policy.
    ConnectedSubtree {
        /// Connected subtree root.
        root: Root,
    },
    /// Mutation kind that does not need source-local fallback roots.
    OtherMutation,
}

/// Runtime-owned resolver for DOM-specific fallback roots.
pub trait MoliRuntimeFallbackRootResolver<Root> {
    /// Return the conservative fallback root for an unknown slot assignment.
    fn unknown_slot_assignment_fallback_root(&self, slot: Root) -> Root;
}

/// Build conservative source-local fallback roots with mutation-time context.
///
/// This owns the CSS-facing fallback policy and de-duplication. The runtime
/// resolver keeps DOM traversal and shadow-host lookup outside Stylo.
pub fn moli_runtime_fallback_roots_for_mutation_inputs<'a, Root>(
    inputs: impl IntoIterator<Item = MoliRuntimeFallbackRootInput<'a, Root>>,
    resolver: &impl MoliRuntimeFallbackRootResolver<Root>,
) -> Vec<Root>
where
    Root: Copy + Eq + Hash + 'a,
{
    let inputs = inputs.into_iter().collect::<Vec<_>>();
    let has_child_list_input = inputs
        .iter()
        .any(|input| matches!(input, MoliRuntimeFallbackRootInput::ChildList { .. }));
    let all_inputs_are_child_list = has_child_list_input
        && inputs
            .iter()
            .all(|input| matches!(input, MoliRuntimeFallbackRootInput::ChildList { .. }));
    let mut roots = IndexSet::new();

    for input in inputs {
        match input {
            MoliRuntimeFallbackRootInput::Attribute {
                element,
                attribute_name,
                has_dependency_change,
                has_non_css_runtime_side_effect,
            } => {
                if moli_attribute_change_can_use_retained_invalidator(
                    attribute_name,
                    has_non_css_runtime_side_effect,
                ) && has_dependency_change
                    && moli_attribute_change_can_skip_fallback_without_dependency(
                        attribute_name,
                    )
                {
                    continue;
                }
                roots.insert(element);
            },
            MoliRuntimeFallbackRootInput::ChildList { added_nodes } => {
                if !all_inputs_are_child_list {
                    roots.extend(added_nodes.iter().copied());
                }
            },
            MoliRuntimeFallbackRootInput::SlotAssignment {
                slot,
                has_assignment_snapshot,
            } => {
                if !has_assignment_snapshot {
                    roots.insert(resolver.unknown_slot_assignment_fallback_root(slot));
                }
            },
            MoliRuntimeFallbackRootInput::ConnectedSubtree { root } => {
                roots.insert(root);
            },
            MoliRuntimeFallbackRootInput::OtherMutation => {},
        }
    }

    roots.into_iter().collect()
}

/// Return whether a state mutation can be represented by the retained
/// source-local invalidator.
#[inline]
pub fn moli_state_change_can_use_retained_invalidator(
    state: ElementState,
    old_state: Option<ElementState>,
) -> bool {
    old_state.is_some() || moli_retained_exact_state_change(state).is_some()
}

/// Return the source fallback reason for a state mutation that cannot use the
/// retained source-local invalidator.
#[inline]
pub fn moli_source_fallback_reason_for_unretained_state_change(
    state: ElementState,
    old_state: Option<ElementState>,
) -> Option<MoliSourceInvalidationFallbackReason> {
    (!moli_state_change_can_use_retained_invalidator(state, old_state))
        .then_some(MoliSourceInvalidationFallbackReason::UnsupportedStateDependency)
}

fn moli_retained_exact_state_change(state: ElementState) -> Option<ElementState> {
    if state.bits().count_ones() != 1 {
        return None;
    }
    let exact_states = ElementState::CHECKED
        | ElementState::INDETERMINATE
        | ElementState::PLACEHOLDER_SHOWN
        | ElementState::DEFINED
        | ElementState::PAUSED
        | ElementState::MUTED
        | ElementState::SEEKING;
    state.intersects(exact_states).then_some(state)
}

/// Whether a retained source had fallback roots available while producing its
/// source-local result.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoliSourceFallbackRootAvailability {
    /// Source fallback roots were available.
    Available {
        /// Number of available fallback roots.
        root_count: usize,
    },
    /// Source fallback roots were required but unavailable.
    Missing,
}

impl MoliSourceFallbackRootAvailability {
    /// Returns source fallback root availability for a concrete root count.
    #[inline]
    pub fn for_root_count(root_count: usize) -> Option<Self> {
        (root_count > 0).then_some(Self::Available { root_count })
    }
}

/// Merge two retained source invalidation kinds using Moli's source
/// result priority.
#[inline]
pub fn moli_merge_retained_source_invalidation_kind(
    existing: MoliRetainedSourceStyleInvalidationKind,
    incoming: MoliRetainedSourceStyleInvalidationKind,
) -> MoliRetainedSourceStyleInvalidationKind {
    existing.merged_with(incoming)
}

/// Return whether this kind can be used as a fallback-root payload instead of
/// retained source-local queries.
#[inline]
pub fn moli_retained_source_invalidation_kind_can_use_fallback_payload(
    kind: MoliRetainedSourceStyleInvalidationKind,
) -> bool {
    !kind.carries_retained_queries()
}

/// Merge optional fallback-root retained source kinds.
///
/// `RetainedQueries` is intentionally rejected here because this helper only
/// describes fallback-root target priority for retained-query sources.
#[inline]
pub fn moli_merge_retained_source_invalidation_fallback_kind(
    existing: Option<MoliRetainedSourceStyleInvalidationKind>,
    incoming: Option<MoliRetainedSourceStyleInvalidationKind>,
) -> Option<MoliRetainedSourceStyleInvalidationKind> {
    let Some(incoming) = incoming else {
        return existing;
    };
    debug_assert!(
        moli_retained_source_invalidation_kind_can_use_fallback_payload(incoming),
        "fallback kind should describe fallback roots"
    );
    Some(match existing {
        Some(existing) => {
            debug_assert!(
                moli_retained_source_invalidation_kind_can_use_fallback_payload(existing),
                "fallback kind should describe fallback roots"
            );
            existing.merged_with(incoming)
        },
        None => incoming,
    })
}

/// How one retained source invalidation input was resolved.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoliSourceStyleInvalidationSourceResultKind {
    /// The retained invalidator produced an exact-enough result for this source.
    Exact,
    /// The source intentionally carried fallback roots without retained queries.
    FallbackOnly,
    /// The source used mutation-context roots for a dependency that Stylo still
    /// cannot report as exact affected handles.
    ContextFallback,
    /// The source fell back to the wider source scope because no narrower
    /// cause/source-local roots could represent the invalidation safely.
    SourceScopeFallback,
    /// The batch needed source fallback roots, but none were available.
    MissingFallbackRoots,
    /// The retained style system was unavailable for this source query.
    MissingRetainedStyleSystem,
    /// The retained style system was available, but did not have per-source
    /// cascade data for this source query.
    MissingRetainedCascadeData,
    /// The source could not be answered exactly and used fallback roots, or
    /// requested a wider fallback when no source-local fallback roots were
    /// available. See fallback reasons on the source result for why.
    Fallback,
}

/// Sink used to summarize retained source-result kind categories.
pub trait MoliSourceStyleInvalidationSourceResultKindSummarySink {
    /// Record a retained source target whose retained system or cascade data was
    /// unavailable.
    fn record_retained_source_unavailable_target(&mut self);

    /// Record a source-scope fallback target.
    fn record_source_scope_fallback_target(&mut self);

    /// Record a context-fallback target.
    fn record_context_fallback_target(&mut self);
}

/// Summary view for retained source-result kind categories.
pub trait MoliSourceStyleInvalidationSourceResultKindSummary {
    /// Record summary counters into a runtime-owned sink.
    fn record_summary_into(
        &self,
        target: &mut impl MoliSourceStyleInvalidationSourceResultKindSummarySink,
    );
}

impl MoliSourceStyleInvalidationSourceResultKindSummary
    for MoliSourceStyleInvalidationSourceResultKind
{
    #[inline]
    fn record_summary_into(
        &self,
        target: &mut impl MoliSourceStyleInvalidationSourceResultKindSummarySink,
    ) {
        match self {
            Self::MissingRetainedStyleSystem | Self::MissingRetainedCascadeData => {
                target.record_retained_source_unavailable_target();
            },
            Self::SourceScopeFallback => {
                target.record_source_scope_fallback_target();
            },
            Self::ContextFallback => {
                target.record_context_fallback_target();
            },
            _ => {},
        }
    }
}

/// Sink used to summarize retained source fallback-root availability.
pub trait MoliSourceFallbackRootAvailabilitySummarySink {
    /// Record a target that required fallback roots but did not have them.
    fn record_missing_fallback_roots_target(&mut self);
}

/// Summary view for retained source fallback-root availability.
pub trait MoliSourceFallbackRootAvailabilitySummary {
    /// Record summary counters into a runtime-owned sink.
    fn record_summary_into(
        &self,
        target: &mut impl MoliSourceFallbackRootAvailabilitySummarySink,
    );
}

impl MoliSourceFallbackRootAvailabilitySummary for MoliSourceFallbackRootAvailability {
    #[inline]
    fn record_summary_into(
        &self,
        target: &mut impl MoliSourceFallbackRootAvailabilitySummarySink,
    ) {
        if matches!(self, Self::Missing) {
            target.record_missing_fallback_roots_target();
        }
    }
}

/// One retained stylesheet source input for a source-aware invalidation batch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoliRetainedSourceStyleInvalidationKind {
    /// Run retained Stylo invalidation queries against this source's cascade data.
    RetainedQueries,
    /// Apply the source's fallback roots without retained queries.
    FallbackOnly,
    /// Apply mutation-context fallback roots without retained queries.
    ContextFallback,
    /// Apply fallback roots from a conservative source-scope fallback.
    SourceScopeFallback,
    /// The source required fallback roots, but none were available.
    MissingFallbackRoots,
}

impl MoliRetainedSourceStyleInvalidationKind {
    /// Merge two fallback kinds using the conservative priority Moli needs
    /// when co-batching source dependency targets.
    fn merged_with(self, incoming: Self) -> Self {
        if self == Self::RetainedQueries || incoming == Self::RetainedQueries {
            return Self::RetainedQueries;
        }
        if self == Self::MissingFallbackRoots || incoming == Self::MissingFallbackRoots {
            return Self::MissingFallbackRoots;
        }
        if self == Self::SourceScopeFallback || incoming == Self::SourceScopeFallback {
            return Self::SourceScopeFallback;
        }
        if self == Self::FallbackOnly || incoming == Self::FallbackOnly {
            return Self::FallbackOnly;
        }
        if self == Self::ContextFallback || incoming == Self::ContextFallback {
            return Self::ContextFallback;
        }
        Self::FallbackOnly
    }

    /// Returns whether this kind carries retained invalidation queries.
    #[inline]
    fn carries_retained_queries(self) -> bool {
        self == Self::RetainedQueries
    }

    /// Returns whether this kind can be represented as a fallback-root target.
    #[inline]
    fn can_target_fallback_root(self) -> bool {
        matches!(self, Self::FallbackOnly | Self::SourceScopeFallback)
    }

    /// Returns the source result kind represented by this retained source kind.
    #[inline]
    fn fallback_source_result_kind(
        self,
        has_fallback_reasons: bool,
    ) -> MoliSourceStyleInvalidationSourceResultKind {
        match self {
            Self::RetainedQueries => MoliSourceStyleInvalidationSourceResultKind::Fallback,
            Self::FallbackOnly if has_fallback_reasons => {
                MoliSourceStyleInvalidationSourceResultKind::Fallback
            },
            Self::FallbackOnly => MoliSourceStyleInvalidationSourceResultKind::FallbackOnly,
            Self::ContextFallback => {
                MoliSourceStyleInvalidationSourceResultKind::ContextFallback
            },
            Self::SourceScopeFallback => {
                MoliSourceStyleInvalidationSourceResultKind::SourceScopeFallback
            },
            Self::MissingFallbackRoots => {
                MoliSourceStyleInvalidationSourceResultKind::MissingFallbackRoots
            },
        }
    }

    /// Returns fallback root availability represented by this source kind and
    /// the number of fallback roots available to it.
    #[inline]
    fn fallback_root_availability(
        self,
        fallback_root_count: usize,
    ) -> Option<MoliSourceFallbackRootAvailability> {
        if self == Self::MissingFallbackRoots {
            return Some(MoliSourceFallbackRootAvailability::Missing);
        }
        MoliSourceFallbackRootAvailability::for_root_count(fallback_root_count)
    }

    /// Returns the fallback reason implied directly by this kind, if any.
    #[inline]
    fn fallback_reason(self) -> Option<MoliSourceInvalidationFallbackReason> {
        match self {
            Self::SourceScopeFallback => {
                Some(MoliSourceInvalidationFallbackReason::SourceScopeFallback)
            },
            Self::MissingFallbackRoots => {
                Some(MoliSourceInvalidationFallbackReason::MissingFallbackRoots)
            },
            Self::RetainedQueries | Self::FallbackOnly | Self::ContextFallback => None,
        }
    }
}

/// One retained stylesheet source input for a source-aware invalidation batch.
pub struct MoliRetainedSourceStyleInvalidation<'a, Root, Snapshot> {
    input: MoliRetainedSourceStyleInvalidationInput<'a, Root, Snapshot>,
}

enum MoliRetainedSourceStyleInvalidationInput<'a, Root, Snapshot> {
    /// Run retained Stylo invalidation queries against this source's cascade
    /// data.
    RetainedQueries {
        /// Fallback-root target kind to use if retained query exactness fails.
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        /// Per-source cascade data, if available.
        cascade_data: Option<&'a ServoArc<CascadeData>>,
        /// Shadow root whose cascade data should be installed for this source.
        shadow_root: Option<Root>,
        /// Source-local retained invalidation queries.
        queries: &'a IndexSet<MoliRetainedStyleInvalidationQuery<Root>>,
        /// Whether the runtime request and source plan together prove that no
        /// matched active dependency is an exact no-op.
        zero_matched_dependency_result_is_exact: bool,
        /// Reasoned fallback roots from source/cause planning.
        reasoned_fallback_roots: &'a IndexSet<Root>,
        /// Exact-safety fallback roots used when exact query capability is
        /// unavailable.
        exact_safety_fallback_roots: &'a IndexSet<Root>,
        /// Fallback reasons already known before source-local query execution.
        fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
        /// Runtime-owned mutation snapshot payload.
        mutation_snapshot: &'a Snapshot,
    },
    /// Apply fallback roots without retained source-local queries.
    Fallback {
        /// Fallback source input kind.
        kind: MoliRetainedSourceStyleInvalidationKind,
        /// Fallback roots selected by the runtime/source owner.
        fallback_roots: &'a IndexSet<Root>,
        /// Fallback reasons selected by the runtime/source owner.
        fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
    },
}

impl<'a, Root, Snapshot> Copy for MoliRetainedSourceStyleInvalidationInput<'a, Root, Snapshot> where
    Root: Copy
{
}

impl<'a, Root, Snapshot> Clone
    for MoliRetainedSourceStyleInvalidationInput<'a, Root, Snapshot>
where
    Root: Copy,
{
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

/// Sink used by a runtime owner to execute one retained source invalidation
/// input without matching on the input variant in the runtime adapter.
pub trait MoliRetainedSourceStyleInvalidationSink<'a, Root, Snapshot> {
    /// Run retained source-local queries.
    fn run_retained_source_style_invalidation_queries(
        &mut self,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        cascade_data: Option<&'a ServoArc<CascadeData>>,
        shadow_root: Option<Root>,
        queries: &'a IndexSet<MoliRetainedStyleInvalidationQuery<Root>>,
        zero_matched_dependency_result_is_exact: bool,
        reasoned_fallback_roots: &'a IndexSet<Root>,
        exact_safety_fallback_roots: &'a IndexSet<Root>,
        fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
        mutation_snapshot: &'a Snapshot,
    );

    /// Apply a fallback-only source input.
    fn run_fallback_source_style_invalidation(
        &mut self,
        kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: &'a IndexSet<Root>,
        fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
    );
}

impl<'a, Root, Snapshot> Copy for MoliRetainedSourceStyleInvalidation<'a, Root, Snapshot> where
    Root: Copy
{
}

impl<'a, Root, Snapshot> Clone for MoliRetainedSourceStyleInvalidation<'a, Root, Snapshot>
where
    Root: Copy,
{
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

/// Build a retained source invalidation input from typed source parts.
#[inline]
pub fn moli_retained_source_style_invalidation_from_parts<'a, Root, Snapshot>(
    kind: MoliRetainedSourceStyleInvalidationKind,
    fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
    cascade_data: Option<&'a ServoArc<CascadeData>>,
    shadow_root: Option<Root>,
    retained_queries: Option<&'a IndexSet<MoliRetainedStyleInvalidationQuery<Root>>>,
    zero_matched_dependency_result_is_exact: bool,
    reasoned_fallback_roots: &'a IndexSet<Root>,
    exact_safety_fallback_roots: &'a IndexSet<Root>,
    fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
    mutation_snapshot: &'a Snapshot,
) -> MoliRetainedSourceStyleInvalidation<'a, Root, Snapshot> {
    if kind.carries_retained_queries() {
        let queries =
            retained_queries.expect("retained source invalidation must carry retained queries");
        return MoliRetainedSourceStyleInvalidation::retained_queries(
            fallback_kind,
            cascade_data,
            shadow_root,
            queries,
            zero_matched_dependency_result_is_exact,
            reasoned_fallback_roots,
            exact_safety_fallback_roots,
            fallback_reasons,
            mutation_snapshot,
        );
    }

    MoliRetainedSourceStyleInvalidation::fallback(
        kind,
        reasoned_fallback_roots,
        fallback_reasons,
    )
}

impl<'a, Root, Snapshot> MoliRetainedSourceStyleInvalidation<'a, Root, Snapshot> {
    /// Create retained source-local query input.
    #[inline]
    fn retained_queries(
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        cascade_data: Option<&'a ServoArc<CascadeData>>,
        shadow_root: Option<Root>,
        queries: &'a IndexSet<MoliRetainedStyleInvalidationQuery<Root>>,
        zero_matched_dependency_result_is_exact: bool,
        reasoned_fallback_roots: &'a IndexSet<Root>,
        exact_safety_fallback_roots: &'a IndexSet<Root>,
        fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
        mutation_snapshot: &'a Snapshot,
    ) -> Self {
        debug_assert!(
            !queries.is_empty(),
            "retained source invalidation must carry retained queries"
        );
        debug_assert!(
            fallback_kind.is_none_or(|kind| !kind.carries_retained_queries()),
            "retained-query fallback kind must describe fallback roots"
        );
        Self {
            input: MoliRetainedSourceStyleInvalidationInput::RetainedQueries {
                fallback_kind,
                cascade_data,
                shadow_root,
                queries,
                zero_matched_dependency_result_is_exact,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
                mutation_snapshot,
            },
        }
    }

    /// Create fallback-only retained source input.
    #[inline]
    fn fallback(
        kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: &'a IndexSet<Root>,
        fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        debug_assert!(
            !kind.carries_retained_queries(),
            "fallback source invalidation must not carry retained queries"
        );
        Self {
            input: MoliRetainedSourceStyleInvalidationInput::Fallback {
                kind,
                fallback_roots,
                fallback_reasons,
            },
        }
    }

    /// Drain this source input into a runtime-owned sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliRetainedSourceStyleInvalidationSink<'a, Root, Snapshot>,
    ) {
        match self.input {
            MoliRetainedSourceStyleInvalidationInput::RetainedQueries {
                fallback_kind,
                cascade_data,
                shadow_root,
                queries,
                zero_matched_dependency_result_is_exact,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
                mutation_snapshot,
            } => {
                target.run_retained_source_style_invalidation_queries(
                    fallback_kind,
                    cascade_data,
                    shadow_root,
                    queries,
                    zero_matched_dependency_result_is_exact,
                    reasoned_fallback_roots,
                    exact_safety_fallback_roots,
                    fallback_reasons,
                    mutation_snapshot,
                );
            },
            MoliRetainedSourceStyleInvalidationInput::Fallback {
                kind,
                fallback_roots,
                fallback_reasons,
            } => {
                target.run_fallback_source_style_invalidation(
                    kind,
                    fallback_roots,
                    fallback_reasons,
                );
            },
        }
    }
}

impl From<MoliDependencyFallbackReason> for MoliSourceInvalidationFallbackReason {
    fn from(reason: MoliDependencyFallbackReason) -> Self {
        match reason {
            MoliDependencyFallbackReason::UnknownDependency => Self::UnknownDependency,
            MoliDependencyFallbackReason::FullSelector => Self::FullSelector,
            MoliDependencyFallbackReason::RelativeAnySelector => Self::RelativeAnySelector,
            MoliDependencyFallbackReason::ScopeDependency => Self::ScopeDependency,
            MoliDependencyFallbackReason::UnsupportedStateDependency => {
                Self::UnsupportedStateDependency
            },
            MoliDependencyFallbackReason::UnsupportedDependency => {
                Self::UnsupportedDependency
            },
            MoliDependencyFallbackReason::NthOfDependency => Self::NthOfDependency,
            MoliDependencyFallbackReason::NestedRelativeSelectorDependency => {
                Self::NestedRelativeSelectorDependency
            },
        }
    }
}

impl MoliDependencyInvalidationAction {
    /// Apply this action to a retained dependency invalidation sink.
    #[inline]
    fn drain_into(self, target: &mut impl MoliDependencyInvalidationActionSink) {
        match self {
            Self::Element => target.invalidate_element(),
            Self::ElementAndDescendants => target.invalidate_element_and_descendants(),
            Self::Descendants => target.invalidate_descendants(),
            Self::Siblings => target.invalidate_siblings(),
            Self::SlottedElements => target.invalidate_slotted_elements(),
            Self::Parts => target.invalidate_parts(),
            Self::Scope(action) => target.invalidate_scope(action),
            Self::Fallback(reason) => target.invalidate_fallback(reason),
        }
    }
}

impl MoliScopeDependencyInvalidationAction {
    /// Apply this scope action to a retained scope dependency invalidation sink.
    #[inline]
    fn drain_into(self, target: &mut impl MoliScopeDependencyInvalidationActionSink) {
        match self {
            Self::ImplicitScope => target.invalidate_implicit_scope(),
            Self::ForceAtSubject { force_add } => {
                target.invalidate_scope_force_at_subject(force_add)
            },
            Self::CheckNextInScope => target.invalidate_scope_check_next(),
            Self::PushByCombinator => target.invalidate_scope_by_combinator(),
        }
    }
}

impl MoliRelativeDependencyInvalidationAction {
    /// Apply this relative traversal action to a candidate traversal sink.
    #[inline]
    fn drain_into(self, target: &mut impl MoliRelativeDependencyInvalidationActionSink) {
        match self {
            Self::Ancestors => target.visit_relative_ancestor_candidates(),
            Self::Parent => target.visit_relative_parent_candidate(),
            Self::PrevSibling => target.visit_relative_previous_sibling_candidate(),
            Self::EarlierSibling => target.visit_relative_earlier_sibling_candidates(),
            Self::AncestorPrevSibling => {
                target.visit_relative_ancestor_previous_sibling_candidates()
            },
            Self::AncestorEarlierSibling => {
                target.visit_relative_ancestor_earlier_sibling_candidates()
            },
        }
    }
}

/// Check whether a dependency changes when matched against old-value snapshots
/// for the candidate element.
#[inline]
fn moli_dependency_changes_anchor_with_snapshot<E>(
    dependency: &Dependency,
    element: E,
    snapshot_map: &SnapshotMap,
    matching_context: &mut MatchingContext<'_, E::Impl>,
    scope: Option<OpaqueElement>,
) -> bool
where
    E: TElement + Copy,
{
    let wrapper = ElementWrapper::new(element, snapshot_map);
    check_dependency(dependency, &element, &wrapper, matching_context, scope)
}

/// Check whether a relative dependency's outer anchor changes for a candidate.
#[inline]
pub fn moli_relative_dependency_changes_anchor<E>(
    dependency: &Dependency,
    candidate: E,
    scope: Option<OpaqueElement>,
    snapshot_map: &SnapshotMap,
    quirks_mode: QuirksMode,
) -> bool
where
    E: TElement + Copy,
{
    let mut selector_caches = SelectorCaches::default();
    let mut matching_context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut selector_caches,
        quirks_mode,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::Yes,
    );
    matching_context.current_host = scope;
    moli_dependency_changes_anchor_with_snapshot(
        dependency,
        candidate,
        snapshot_map,
        &mut matching_context,
        scope,
    )
}

/// Visit candidate elements for one relative dependency.
#[inline]
pub fn moli_visit_relative_dependency_candidates<E, Visit>(
    root: E,
    dependency: &Dependency,
    sibling_traversal: &SiblingTraversalMap<E>,
    visit: Visit,
) where
    E: TElement + Copy,
    Visit: FnMut(E),
{
    debug_assert!(moli_dependency_is_relative_selector(dependency));
    let Some(action) = moli_relative_dependency_invalidation_action(dependency) else {
        return;
    };
    let mut visitor = MoliRelativeDependencyCandidateVisitor {
        root,
        sibling_traversal,
        visit,
    };
    action.drain_into(&mut visitor);
}

struct MoliRelativeDependencyCandidateVisitor<'a, E: TElement, Visit> {
    root: E,
    sibling_traversal: &'a SiblingTraversalMap<E>,
    visit: Visit,
}

impl<E, Visit> MoliRelativeDependencyInvalidationActionSink
    for MoliRelativeDependencyCandidateVisitor<'_, E, Visit>
where
    E: TElement + Copy,
    Visit: FnMut(E),
{
    fn visit_relative_ancestor_candidates(&mut self) {
        let mut current = moli_style_parent_element_or_host(self.root);
        while let Some(candidate) = current {
            (self.visit)(candidate);
            current = moli_style_parent_element_or_host(candidate);
        }
    }

    fn visit_relative_parent_candidate(&mut self) {
        if let Some(parent) = moli_style_parent_element_or_host(self.root) {
            (self.visit)(parent);
        }
    }

    fn visit_relative_previous_sibling_candidate(&mut self) {
        if let Some(previous) = self.sibling_traversal.prev_sibling_for(&self.root) {
            (self.visit)(previous);
        }
    }

    fn visit_relative_earlier_sibling_candidates(&mut self) {
        let mut current = self.sibling_traversal.prev_sibling_for(&self.root);
        while let Some(candidate) = current {
            (self.visit)(candidate);
            current = candidate.prev_sibling_element();
        }
    }

    fn visit_relative_ancestor_previous_sibling_candidates(&mut self) {
        let mut current = moli_style_parent_element_or_host(self.root);
        while let Some(parent) = current {
            if let Some(previous) = parent.prev_sibling_element() {
                (self.visit)(previous);
            }
            current = moli_style_parent_element_or_host(parent);
        }
    }

    fn visit_relative_ancestor_earlier_sibling_candidates(&mut self) {
        let mut current = moli_style_parent_element_or_host(self.root);
        while let Some(parent) = current {
            let mut sibling = parent.prev_sibling_element();
            while let Some(candidate) = sibling {
                (self.visit)(candidate);
                sibling = candidate.prev_sibling_element();
            }
            current = moli_style_parent_element_or_host(parent);
        }
    }
}

#[inline]
fn moli_style_parent_element_or_host<E>(element: E) -> Option<E>
where
    E: TElement + Copy,
{
    element.as_node().parent_element_or_host()
}

/// Mutation-boundary roots available to source dependency planning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliSourceDependencyBoundaryRoots<'a, Root> {
    empty_target_fallback_roots: &'a [Root],
    relative_previous_sibling_cleanup_roots: &'a [Root],
}

impl<'a, Root> MoliSourceDependencyBoundaryRoots<'a, Root> {
    /// Create mutation-boundary roots for source dependency planning.
    #[inline]
    pub fn new(
        empty_target_fallback_roots: &'a [Root],
        relative_previous_sibling_cleanup_roots: &'a [Root],
    ) -> Self {
        Self {
            empty_target_fallback_roots,
            relative_previous_sibling_cleanup_roots,
        }
    }
}

impl<Root> Default for MoliSourceDependencyBoundaryRoots<'_, Root> {
    #[inline]
    fn default() -> Self {
        Self::new(&[], &[])
    }
}

/// Planned retained invalidation work for one stylesheet source in a batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliPlannedSourceDependencyInvalidation<Root> {
    source_index: usize,
    target: MoliPlannedSourceDependencyInvalidationTarget<Root>,
    structural_boundary_cleanup_roots: Vec<Root>,
}

/// Planned source dependency invalidation target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliPlannedSourceDependencyInvalidationTarget<Root> {
    target: MoliPlannedSourceDependencyInvalidationTargetKind<Root>,
}

/// Fork-private planned source dependency invalidation target shape.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MoliPlannedSourceDependencyInvalidationTargetKind<Root> {
    /// Run retained queries, optionally carrying a fallback target for
    /// dependency shapes that cannot be exact.
    RetainedQueries {
        /// Exact retained dependency queries to run for this source.
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<Root>>,
        /// Optional fallback kind for inexact dependency branches.
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        /// Fallback roots tied to explicit dependency fallback reasons.
        reasoned_fallback_roots: Vec<Root>,
        /// Fallback roots that are safe to use if exact invalidation is
        /// unavailable.
        exact_safety_fallback_roots: Vec<Root>,
        /// Reasons why fallback handling may be needed.
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    },
    /// Do not run retained queries; apply fallback roots for this source.
    FallbackOnly {
        /// Fallback policy represented by this target.
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        /// Runtime roots to clear for fallback handling.
        fallback_roots: Vec<Root>,
        /// Reasons why fallback handling is required.
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    },
}

/// Drainable parts for a planned source dependency invalidation target.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MoliPlannedSourceDependencyInvalidationTargetParts<Root> {
    /// Retained-query target parts.
    RetainedQueries {
        /// Exact retained dependency queries to run for this source.
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<Root>>,
        /// Optional fallback kind for inexact dependency branches.
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        /// Fallback roots tied to explicit dependency fallback reasons.
        reasoned_fallback_roots: Vec<Root>,
        /// Fallback roots that are safe to use if exact invalidation is
        /// unavailable.
        exact_safety_fallback_roots: Vec<Root>,
        /// Reasons why fallback handling may be needed.
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    },
    /// Fallback target with explicit roots.
    FallbackWithRoots {
        /// Fallback policy represented by this target.
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        /// Runtime roots to clear for fallback handling.
        fallback_roots: Vec<Root>,
        /// Reasons why fallback handling is required.
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    },
    /// Fallback target whose roots are unavailable.
    MissingFallbackRoots {
        /// Reasons why fallback handling is required.
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    },
}

/// Sink for planned source dependency target parts.
pub trait MoliPlannedSourceDependencyInvalidationTargetPartsSink<Root> {
    /// Record retained-query target parts.
    fn set_planned_retained_source_dependency_target_parts(
        &mut self,
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<Root>>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<Root>,
        exact_safety_fallback_roots: Vec<Root>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    );

    /// Record fallback target parts with roots.
    fn set_planned_fallback_source_dependency_target_parts(
        &mut self,
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<Root>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    );

    /// Record fallback target parts when fallback roots are unavailable.
    fn set_planned_missing_fallback_roots_source_dependency_target_parts(
        &mut self,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    );
}

/// Sink for a planned source dependency invalidation row.
pub trait MoliPlannedSourceDependencyInvalidationPartsSink<Root>:
    MoliPlannedSourceDependencyInvalidationTargetPartsSink<Root>
{
    /// Record the stylesheet source index for this planned row.
    fn set_planned_source_dependency_source_index(&mut self, source_index: usize);

    /// Record structural-boundary cleanup roots for this planned row.
    fn set_planned_source_dependency_structural_boundary_cleanup_roots(
        &mut self,
        structural_boundary_cleanup_roots: Vec<Root>,
    );
}

/// Fallback-root invalidation target used when no source retained query target
/// is available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliPlannedFallbackRootInvalidationTarget<Root> {
    target: MoliPlannedSourceDependencyInvalidationTarget<Root>,
}

/// Runtime source/scope fallback input for one stylesheet source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoliStylesheetSourceScopeFallbackInput<Root> {
    /// Inline or linked stylesheet owner element.
    StylesheetOwner {
        /// Stylesheet owner handle.
        owner: Root,
    },
    /// Adopted stylesheet scoped to a document.
    DocumentAdopted {
        /// Document handle.
        document: Root,
    },
    /// Adopted stylesheet scoped to a shadow root.
    ShadowRootAdopted {
        /// Shadow root handle.
        root: Root,
    },
    /// No source/scope fallback roots are available.
    Unscoped,
}

/// DOM-backed resolver for stylesheet source/scope fallback roots.
pub trait MoliStylesheetSourceScopeFallbackRootsResolver<Root> {
    /// Return fallback roots for a stylesheet owner element.
    fn stylesheet_owner_source_scope_fallback_roots(&self, owner: Root) -> Vec<Root>;

    /// Return fallback roots for an adopted stylesheet scoped to a document.
    fn document_source_scope_fallback_roots(&self, document: Root) -> Vec<Root>;

    /// Return fallback roots for an adopted stylesheet scoped to a shadow root.
    fn shadow_root_source_scope_fallback_roots(&self, root: Root) -> Vec<Root>;
}

/// Source-local dependency invalidation plan before it is added to a batch.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MoliSourceDependencyInvalidationSourcePlan<Root> {
    /// Source-local retained or fallback work. `None` means this source does
    /// not need a planned row for the current request batch.
    Work {
        /// Planned source dependency target, if this source has work.
        target: Option<MoliPlannedSourceDependencyInvalidationTarget<Root>>,
        /// Mutation-local structural roots that are already known to cover
        /// every affected subtree without widening to the stylesheet scope.
        exact_structural_cleanup_roots: Vec<Root>,
    },
    /// The source requires fallback and no roots are available at the requested
    /// boundary.
    RequiresSourceFallback {
        /// Source-level fallback target.
        target: MoliPlannedSourceDependencyInvalidationTarget<Root>,
    },
}

/// Drainable fallback-root invalidation target parts.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MoliPlannedFallbackRootInvalidationTargetParts<Root> {
    fallback_kind: MoliRetainedSourceStyleInvalidationKind,
    fallback_roots: Vec<Root>,
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
}

/// Sink for fallback-root invalidation target parts.
pub trait MoliPlannedFallbackRootInvalidationTargetPartsSink<Root> {
    /// Record fallback-root target parts.
    fn set_planned_fallback_root_target_parts(
        &mut self,
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<Root>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    );
}

impl<Root> MoliPlannedSourceDependencyInvalidation<Root> {
    /// Create a planned source dependency invalidation from a typed target.
    #[inline]
    fn from_target(
        source_index: usize,
        target: MoliPlannedSourceDependencyInvalidationTarget<Root>,
        structural_boundary_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self {
            source_index,
            target,
            structural_boundary_cleanup_roots,
        }
    }

    /// Create a retained-query planned source dependency invalidation with a
    /// fallback kind for inexact dependency branches.
    #[cfg(test)]
    #[inline]
    fn retained_queries_with_fallback_kind(
        source_index: usize,
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<Root>>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<Root>,
        exact_safety_fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
        structural_boundary_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self::from_target(
            source_index,
            MoliPlannedSourceDependencyInvalidationTarget::retained_queries_with_fallback_kind(
                exact_queries,
                fallback_kind,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
            ),
            structural_boundary_cleanup_roots,
        )
    }

    /// Create a fallback-only planned source dependency invalidation.
    #[cfg(test)]
    #[inline]
    fn fallback_only(
        source_index: usize,
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
        structural_boundary_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self::fallback_with_kind(
            source_index,
            MoliRetainedSourceStyleInvalidationKind::FallbackOnly,
            fallback_roots,
            fallback_reasons,
            structural_boundary_cleanup_roots,
        )
    }

    /// Create a fallback planned source dependency invalidation with an
    /// explicit fallback kind.
    #[cfg(test)]
    #[inline]
    fn fallback_with_kind(
        source_index: usize,
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
        structural_boundary_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self::from_target(
            source_index,
            MoliPlannedSourceDependencyInvalidationTarget::fallback_with_kind(
                fallback_kind,
                fallback_roots,
                fallback_reasons,
            ),
            structural_boundary_cleanup_roots,
        )
    }

    /// Create a planned source dependency invalidation for unavailable fallback
    /// roots.
    #[cfg(test)]
    #[inline]
    fn missing_fallback_roots(
        source_index: usize,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
        structural_boundary_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self::fallback_with_kind(
            source_index,
            MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots,
            Vec::new(),
            fallback_reasons,
            structural_boundary_cleanup_roots,
        )
    }

    /// Drain this row into a sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliPlannedSourceDependencyInvalidationPartsSink<Root>,
    ) {
        target.set_planned_source_dependency_source_index(self.source_index);
        target.set_planned_source_dependency_structural_boundary_cleanup_roots(
            self.structural_boundary_cleanup_roots,
        );
        self.target.drain_into(target);
    }
}

impl<Root> MoliPlannedSourceDependencyInvalidationTarget<Root> {
    /// Create a target from source dependency planner work parts.
    ///
    /// Exact-safety fallback roots are only a retained-query safety net. If a
    /// source produced no exact queries, those roots become an explicit
    /// inexact-empty fallback target instead of being silently dropped.
    #[inline]
    fn from_source_dependency_work_parts(
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<Root>>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<Root>,
        exact_safety_fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Option<Self> {
        let mut fallback_reasons = fallback_reasons.into_iter().collect::<IndexSet<_>>();
        if exact_queries.is_empty() {
            if reasoned_fallback_roots.is_empty() && exact_safety_fallback_roots.is_empty() {
                return None;
            }
            let fallback_roots = if reasoned_fallback_roots.is_empty() {
                fallback_reasons
                    .insert(MoliSourceInvalidationFallbackReason::InexactEmptyResult);
                exact_safety_fallback_roots
            } else {
                reasoned_fallback_roots
            };
            return Some(Self::fallback_with_kind(
                fallback_kind
                    .unwrap_or(MoliRetainedSourceStyleInvalidationKind::FallbackOnly),
                fallback_roots,
                fallback_reasons,
            ));
        }

        Some(Self::retained_queries_with_fallback_kind(
            exact_queries,
            fallback_kind,
            reasoned_fallback_roots,
            exact_safety_fallback_roots,
            fallback_reasons,
        ))
    }

    /// Create a source dependency fallback target from selected fallback roots.
    ///
    /// Missing roots are represented as an explicit fallback kind and reason,
    /// not as an empty generic fallback-root payload.
    #[inline]
    fn source_dependency_fallback(
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        let mut fallback_reasons = fallback_reasons.into_iter().collect::<IndexSet<_>>();
        let fallback_kind = if fallback_roots.is_empty() {
            fallback_reasons
                .insert(MoliSourceInvalidationFallbackReason::MissingFallbackRoots);
            MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots
        } else {
            MoliRetainedSourceStyleInvalidationKind::FallbackOnly
        };
        Self::fallback_with_kind(fallback_kind, fallback_roots, fallback_reasons)
    }

    /// Create a retained-query target with optional fallback target policy.
    #[inline]
    fn retained_queries_with_fallback_kind(
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<Root>>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: Vec<Root>,
        exact_safety_fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        debug_assert!(
            !exact_queries.is_empty(),
            "retained planned source dependency invalidation should carry exact queries"
        );
        debug_assert!(
            fallback_kind.is_none_or(|kind| !kind.carries_retained_queries()),
            "retained planned source dependency fallback kind should describe fallback roots"
        );
        Self {
            target: MoliPlannedSourceDependencyInvalidationTargetKind::RetainedQueries {
                exact_queries,
                fallback_kind,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons: fallback_reasons.into_iter().collect(),
            },
        }
    }

    /// Create a fallback target with an explicit fallback kind.
    #[inline]
    fn fallback_with_kind(
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        debug_assert!(
            !fallback_kind.carries_retained_queries(),
            "fallback planned source dependency invalidation should not carry retained-query kind"
        );
        let mut fallback_reasons = fallback_reasons.into_iter().collect::<IndexSet<_>>();
        if let Some(fallback_reason) = fallback_kind.fallback_reason() {
            fallback_reasons.insert(fallback_reason);
        }
        Self {
            target: MoliPlannedSourceDependencyInvalidationTargetKind::FallbackOnly {
                fallback_kind,
                fallback_roots,
                fallback_reasons,
            },
        }
    }

    /// Consume this target into drainable parts.
    #[inline]
    fn into_parts(self) -> MoliPlannedSourceDependencyInvalidationTargetParts<Root> {
        match self.target {
            MoliPlannedSourceDependencyInvalidationTargetKind::RetainedQueries {
                exact_queries,
                fallback_kind,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
            } => MoliPlannedSourceDependencyInvalidationTargetParts::RetainedQueries {
                exact_queries,
                fallback_kind,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
            },
            MoliPlannedSourceDependencyInvalidationTargetKind::FallbackOnly {
                fallback_kind: MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots,
                fallback_roots,
                fallback_reasons,
            } => {
                debug_assert!(
                    fallback_roots.is_empty(),
                    "missing fallback roots target should not carry fallback roots"
                );
                MoliPlannedSourceDependencyInvalidationTargetParts::MissingFallbackRoots {
                    fallback_reasons,
                }
            },
            MoliPlannedSourceDependencyInvalidationTargetKind::FallbackOnly {
                fallback_kind,
                fallback_roots,
                fallback_reasons,
            } => {
                debug_assert!(
                    !fallback_kind.carries_retained_queries(),
                    "fallback planned source dependency target should not carry retained-query kind"
                );
                MoliPlannedSourceDependencyInvalidationTargetParts::FallbackWithRoots {
                    fallback_kind,
                    fallback_roots,
                    fallback_reasons,
                }
            },
        }
    }

    /// Drain this target into a sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliPlannedSourceDependencyInvalidationTargetPartsSink<Root>,
    ) {
        self.into_parts().drain_into(target);
    }
}

impl<Root> MoliPlannedSourceDependencyInvalidationTargetParts<Root> {
    /// Drain these target parts into a sink.
    #[inline]
    fn drain_into(
        self,
        target: &mut impl MoliPlannedSourceDependencyInvalidationTargetPartsSink<Root>,
    ) {
        match self {
            Self::RetainedQueries {
                exact_queries,
                fallback_kind,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
            } => target.set_planned_retained_source_dependency_target_parts(
                exact_queries,
                fallback_kind,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
                fallback_reasons,
            ),
            Self::FallbackWithRoots {
                fallback_kind,
                fallback_roots,
                fallback_reasons,
            } => target.set_planned_fallback_source_dependency_target_parts(
                fallback_kind,
                fallback_roots,
                fallback_reasons,
            ),
            Self::MissingFallbackRoots { fallback_reasons } => target
                .set_planned_missing_fallback_roots_source_dependency_target_parts(
                    fallback_reasons,
                ),
        }
    }
}

impl<Root> MoliPlannedFallbackRootInvalidationTarget<Root> {
    /// Create a fallback-only target.
    #[inline]
    fn fallback_only(
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        Self::fallback_with_kind(
            MoliRetainedSourceStyleInvalidationKind::FallbackOnly,
            fallback_roots,
            fallback_reasons,
        )
    }

    /// Create a source-scope fallback target.
    #[inline]
    fn source_scope_fallback(
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        Self::fallback_with_kind(
            MoliRetainedSourceStyleInvalidationKind::SourceScopeFallback,
            fallback_roots,
            fallback_reasons,
        )
    }

    /// Create a fallback-root target with an explicit fallback kind.
    #[inline]
    fn fallback_with_kind(
        fallback_kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_roots: Vec<Root>,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        Self {
            target: MoliPlannedSourceDependencyInvalidationTarget::fallback_with_kind(
                fallback_kind,
                fallback_roots,
                fallback_reasons,
            ),
        }
    }

    /// Consume this fallback target into drainable parts.
    #[inline]
    fn into_parts(self) -> MoliPlannedFallbackRootInvalidationTargetParts<Root> {
        let MoliPlannedSourceDependencyInvalidationTargetKind::FallbackOnly {
            fallback_kind,
            fallback_roots,
            fallback_reasons,
        } = self.target.target
        else {
            unreachable!("fallback-root invalidation target should not carry retained queries");
        };
        debug_assert!(
            fallback_kind.can_target_fallback_root(),
            "fallback-root target should carry fallback-only or source-scope fallback kind"
        );
        MoliPlannedFallbackRootInvalidationTargetParts {
            fallback_kind,
            fallback_roots,
            fallback_reasons,
        }
    }

    /// Drain this fallback-root target into a sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliPlannedFallbackRootInvalidationTargetPartsSink<Root>,
    ) {
        self.into_parts().drain_into(target);
    }
}

/// Return source/scope fallback roots for a stylesheet source input.
#[inline]
pub fn moli_stylesheet_source_scope_fallback_roots<Root: Copy>(
    input: MoliStylesheetSourceScopeFallbackInput<Root>,
    resolver: &impl MoliStylesheetSourceScopeFallbackRootsResolver<Root>,
) -> Vec<Root> {
    match input {
        MoliStylesheetSourceScopeFallbackInput::StylesheetOwner { owner } => {
            resolver.stylesheet_owner_source_scope_fallback_roots(owner)
        },
        MoliStylesheetSourceScopeFallbackInput::DocumentAdopted { document } => {
            resolver.document_source_scope_fallback_roots(document)
        },
        MoliStylesheetSourceScopeFallbackInput::ShadowRootAdopted { root } => {
            resolver.shadow_root_source_scope_fallback_roots(root)
        },
        MoliStylesheetSourceScopeFallbackInput::Unscoped => Vec::new(),
    }
}

/// Create a source-scope fallback target from embedder-provided fallback roots.
#[inline]
pub fn moli_source_scope_fallback_plan<Root>(
    source_scope_fallback_roots: impl FnOnce() -> Vec<Root>,
    fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
) -> MoliPlannedFallbackRootInvalidationTarget<Root> {
    MoliPlannedFallbackRootInvalidationTarget::source_scope_fallback(
        source_scope_fallback_roots(),
        fallback_reasons,
    )
}

/// Create a generic fallback-root target from embedder-provided fallback roots.
#[inline]
pub fn moli_fallback_roots_plan<Root>(
    fallback_roots: Vec<Root>,
    fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
) -> MoliPlannedFallbackRootInvalidationTarget<Root> {
    MoliPlannedFallbackRootInvalidationTarget::fallback_only(fallback_roots, fallback_reasons)
}

/// Create a fallback target from runtime fallback roots, falling back to the
/// source-scope roots only when no narrower runtime roots are available.
#[inline]
pub fn moli_runtime_or_source_scope_fallback_plan<Root>(
    runtime_fallback_roots: Vec<Root>,
    source_scope_fallback_roots: impl FnOnce() -> Vec<Root>,
    fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
) -> MoliPlannedFallbackRootInvalidationTarget<Root> {
    if runtime_fallback_roots.is_empty() {
        MoliPlannedFallbackRootInvalidationTarget::source_scope_fallback(
            source_scope_fallback_roots(),
            fallback_reasons,
        )
    } else {
        MoliPlannedFallbackRootInvalidationTarget::fallback_only(
            runtime_fallback_roots,
            fallback_reasons,
        )
    }
}

impl<Root> MoliPlannedFallbackRootInvalidationTargetParts<Root> {
    /// Drain these fallback target parts into a sink.
    #[inline]
    fn drain_into(
        self,
        target: &mut impl MoliPlannedFallbackRootInvalidationTargetPartsSink<Root>,
    ) {
        target.set_planned_fallback_root_target_parts(
            self.fallback_kind,
            self.fallback_roots,
            self.fallback_reasons,
        );
    }
}

impl<Root> MoliSourceDependencyInvalidationSourcePlan<Root> {
    /// Create a source-local work plan.
    #[inline]
    fn work(
        target: Option<MoliPlannedSourceDependencyInvalidationTarget<Root>>,
        exact_structural_cleanup_roots: Vec<Root>,
    ) -> Self {
        Self::Work {
            target,
            exact_structural_cleanup_roots,
        }
    }

    /// Create a source-local plan that requires source fallback.
    #[inline]
    fn requires_source_fallback(
        target: MoliPlannedSourceDependencyInvalidationTarget<Root>,
    ) -> Self {
        Self::RequiresSourceFallback { target }
    }
}

/// Source dependency planning result for a batch of stylesheet sources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceDependencyInvalidationBatchPlan<Root> {
    plan: MoliSourceDependencyInvalidationBatchPlanKind<Root>,
}

/// Fork-private backing shape for source dependency batch plans.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MoliSourceDependencyInvalidationBatchPlanKind<Root> {
    /// At least one source has retained or fallback work, with optional
    /// fallback-root work for an empty exact target.
    Work {
        /// Planned rows for stylesheet sources participating in the batch.
        sources: Vec<MoliPlannedSourceDependencyInvalidation<Root>>,
        /// Optional fallback-root target when no source row was planned for an
        /// empty structural target.
        boundary_fallback: Option<MoliPlannedFallbackRootInvalidationTarget<Root>>,
    },
    /// A source dependency requires fallback and no fallback roots are
    /// available at the requested boundary.
    RequiresSourceFallback {
        /// Source row that forces source-level fallback.
        source: MoliPlannedSourceDependencyInvalidation<Root>,
    },
}

/// Sink for a source dependency batch plan.
pub trait MoliSourceDependencyInvalidationBatchPlanSink<Root> {
    /// Record source-local planned work with an optional empty-target boundary
    /// fallback.
    fn set_source_dependency_batch_work(
        &mut self,
        sources: Vec<MoliPlannedSourceDependencyInvalidation<Root>>,
        boundary_fallback: Option<MoliPlannedFallbackRootInvalidationTarget<Root>>,
    );

    /// Record the source row that requires fallback when boundary roots are
    /// unavailable.
    fn set_source_dependency_batch_requires_source_fallback(
        &mut self,
        source: MoliPlannedSourceDependencyInvalidation<Root>,
    );
}

impl<Root> MoliSourceDependencyInvalidationBatchPlan<Root> {
    /// Create a source dependency batch plan with source-local work.
    #[inline]
    fn work(
        sources: Vec<MoliPlannedSourceDependencyInvalidation<Root>>,
        boundary_fallback: Option<MoliPlannedFallbackRootInvalidationTarget<Root>>,
    ) -> Self {
        Self {
            plan: MoliSourceDependencyInvalidationBatchPlanKind::Work {
                sources,
                boundary_fallback,
            },
        }
    }

    /// Create a source dependency batch plan that requires source fallback.
    #[inline]
    fn requires_source_fallback(
        source: MoliPlannedSourceDependencyInvalidation<Root>,
    ) -> Self {
        Self {
            plan: MoliSourceDependencyInvalidationBatchPlanKind::RequiresSourceFallback {
                source,
            },
        }
    }

    /// Drain this batch plan into a runtime-owned pending target sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliSourceDependencyInvalidationBatchPlanSink<Root>,
    ) {
        match self.plan {
            MoliSourceDependencyInvalidationBatchPlanKind::Work {
                sources,
                boundary_fallback,
            } => target.set_source_dependency_batch_work(sources, boundary_fallback),
            MoliSourceDependencyInvalidationBatchPlanKind::RequiresSourceFallback {
                source,
            } => target.set_source_dependency_batch_requires_source_fallback(source),
        }
    }
}

/// Return source-local invalidation planning for a batch of queries against one
/// Stylo source dependency summary.
///
/// The embedder supplies DOM-backed mutation-context roots through
/// `context_roots_provider`; dependency interpretation, fallback priority,
/// exact-safety roots, and source fallback target shape stay in the
/// Moli-facing Stylo boundary.
fn moli_source_dependency_invalidation_plan<Root: Copy + Eq + Hash, ContextRootsProvider>(
    summary: &MoliSourceDependencySummary,
    selected_fallback_roots: &[Root],
    requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
    context_roots_provider: &mut ContextRootsProvider,
) -> MoliSourceDependencyInvalidationSourcePlan<Root>
where
    ContextRootsProvider: MoliSourceDependencyInvalidationContextRootsProvider<Root>,
{
    let mut exact_queries = Vec::new();
    let mut fallback_kind = None;
    let mut reasoned_fallback_roots = Vec::new();
    let mut reasoned_fallback_seen = HashSet::new();
    // Exact-query safety roots are separate from reasoned fallback roots:
    // they are used only when retained exact invalidation is unavailable or
    // returns an inexact result.
    let mut exact_safety_fallback_roots = Vec::new();
    let mut exact_safety_fallback_seen = HashSet::new();
    // Some structural selector dependencies have a complete mutation-local
    // candidate region even though Stylo's retained invalidator cannot derive
    // every affected sibling from the changed element alone. Keep those roots
    // separate from reasoned fallback roots so the result remains exact.
    let mut exact_structural_cleanup_roots = Vec::new();
    let mut exact_structural_cleanup_seen = HashSet::new();
    let mut fallback_reasons = IndexSet::new();
    let mut missing_fallback_root_reasons = IndexSet::new();
    for request in requests {
        let dependency = summary.query_result(request.query().as_stylo_query());
        if !dependency.has_any_dependency() {
            continue;
        }
        if request.requires_child_list_structural_dependency()
            && !summary.has_child_list_structural_boundary_dependency_for_request(request)
            && !dependency.has_relative_selector_dependency()
        {
            continue;
        }
        if request.requires_relative_previous_sibling_dependency()
            && !dependency.has_relative_previous_sibling_dependency()
        {
            continue;
        }
        let uses_exact_nth_of_structural_roots =
            dependency.can_use_exact_nth_of_structural_roots();
        if uses_exact_nth_of_structural_roots {
            if let Some(context) = request.context() {
                let context_plan = MoliDependencyContextRootPlan::new(
                    &dependency,
                    request
                        .query()
                        .allows_direct_previous_following_sibling_fallback(),
                );
                let structural_roots =
                    context_roots_provider.context_roots_for_source_dependency(
                        request.query().root(),
                        context_plan,
                        context,
                    );
                let query_root = [request.query().root()];
                // `requires_source_fallback` is derived from the presence of
                // the nth-of classification itself. This branch has already
                // proved that no other fallback reason is present, so the
                // mutation-local sibling region supersedes that generic bit.
                // Membership changes can alter the changed element's own nth
                // match and every following element sibling's rank.
                moli_push_unique_roots(
                    &mut exact_structural_cleanup_roots,
                    &mut exact_structural_cleanup_seen,
                    &query_root,
                );
                moli_push_unique_roots(
                    &mut exact_structural_cleanup_roots,
                    &mut exact_structural_cleanup_seen,
                    structural_roots.roots(),
                );
                // Keep the same narrow region as the retained query's safety
                // net if its per-source cascade is temporarily unavailable.
                moli_push_unique_roots(
                    &mut exact_safety_fallback_roots,
                    &mut exact_safety_fallback_seen,
                    &query_root,
                );
                moli_push_unique_roots(
                    &mut exact_safety_fallback_roots,
                    &mut exact_safety_fallback_seen,
                    structural_roots.roots(),
                );
                exact_queries.push((*request.query()).clone());
                continue;
            }
        }
        if dependency.requires_fallback() {
            match dependency.source_dependency_fallback_handling() {
                MoliDependencyFallbackHandling::ContextRoots(reasons)
                    if request.context().is_some() =>
                {
                    let context = request.context().expect("checked above");
                    let context_plan = MoliDependencyContextRootPlan::new(
                        &dependency,
                        request
                            .query()
                            .allows_direct_previous_following_sibling_fallback(),
                    );
                    let fallback = context_roots_provider.context_roots_for_source_dependency(
                        request.query().root(),
                        context_plan,
                        context,
                    );
                    let context_roots = fallback.roots();
                    if !context_roots.is_empty() {
                        if dependency.tries_retained_query_before_context_fallback() {
                            // Nested relative-selector classifications come
                            // from the conservative source summary. The
                            // retained relative invalidator can still answer a
                            // concrete query exactly, so run it first and keep
                            // these mutation-context roots only as its safety
                            // net.
                            fallback_kind =
                                moli_merge_retained_source_invalidation_fallback_kind(
                                    fallback_kind,
                                    Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback),
                                );
                            moli_push_unique_roots(
                                &mut exact_safety_fallback_roots,
                                &mut exact_safety_fallback_seen,
                                context_roots,
                            );
                            exact_queries.push((*request.query()).clone());
                        } else {
                            fallback_kind =
                                moli_merge_retained_source_invalidation_fallback_kind(
                                    fallback_kind,
                                    Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback),
                                );
                            fallback_reasons.extend(reasons);
                            moli_push_unique_roots(
                                &mut reasoned_fallback_roots,
                                &mut reasoned_fallback_seen,
                                context_roots,
                            );
                        }
                        continue;
                    }
                    if selected_fallback_roots.is_empty() {
                        missing_fallback_root_reasons.extend(reasons);
                    } else {
                        fallback_kind = moli_merge_retained_source_invalidation_fallback_kind(
                            fallback_kind,
                            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly),
                        );
                        fallback_reasons.extend(reasons);
                        moli_push_unique_roots(
                            &mut reasoned_fallback_roots,
                            &mut reasoned_fallback_seen,
                            selected_fallback_roots,
                        );
                    }
                },
                MoliDependencyFallbackHandling::ContextRoots(reasons)
                | MoliDependencyFallbackHandling::SourceFallback(reasons) => {
                    if selected_fallback_roots.is_empty() {
                        missing_fallback_root_reasons.extend(reasons);
                    } else {
                        fallback_kind = moli_merge_retained_source_invalidation_fallback_kind(
                            fallback_kind,
                            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly),
                        );
                        fallback_reasons.extend(reasons);
                        moli_push_unique_roots(
                            &mut reasoned_fallback_roots,
                            &mut reasoned_fallback_seen,
                            selected_fallback_roots,
                        );
                    }
                },
            }
            continue;
        }
        if let Some(context) = request.context() {
            let context_plan = MoliDependencyContextRootPlan::new(
                &dependency,
                request
                    .query()
                    .allows_direct_previous_following_sibling_fallback(),
            );
            let fallback = context_roots_provider.context_roots_for_source_dependency(
                request.query().root(),
                context_plan,
                context,
            );
            if fallback.requires_source_fallback() {
                let context_roots = fallback.roots();
                let reasons = dependency.source_invalidation_fallback_reasons();
                if dependency.tries_retained_query_before_source_fallback()
                    && !selected_fallback_roots.is_empty()
                {
                    // The context summary cannot express @scope boundaries,
                    // but the retained processor walks the exact scope
                    // dependency chain. Source roots are used only if that
                    // source-local query later proves unavailable or inexact.
                    fallback_kind = moli_merge_retained_source_invalidation_fallback_kind(
                        fallback_kind,
                        Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly),
                    );
                    moli_push_unique_roots(
                        &mut exact_safety_fallback_roots,
                        &mut exact_safety_fallback_seen,
                        selected_fallback_roots,
                    );
                    exact_queries.push((*request.query()).clone());
                } else if dependency.has_only_direct_relative_previous_sibling_dependency()
                    && !context_roots.is_empty()
                {
                    fallback_kind = moli_merge_retained_source_invalidation_fallback_kind(
                        fallback_kind,
                        Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback),
                    );
                    fallback_reasons.extend(reasons);
                    moli_push_unique_roots(
                        &mut reasoned_fallback_roots,
                        &mut reasoned_fallback_seen,
                        context_roots,
                    );
                } else if selected_fallback_roots.is_empty() {
                    missing_fallback_root_reasons.extend(reasons);
                } else {
                    fallback_kind = moli_merge_retained_source_invalidation_fallback_kind(
                        fallback_kind,
                        Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly),
                    );
                    fallback_reasons.extend(reasons);
                    moli_push_unique_roots(
                        &mut reasoned_fallback_roots,
                        &mut reasoned_fallback_seen,
                        selected_fallback_roots,
                    );
                }
                continue;
            }
            let context_roots = fallback.into_roots();
            if dependency.requires_structural_context_fallback_cleanup(
                request.requires_child_list_structural_dependency(),
                request.query().is_universal(),
            ) {
                if context_roots.is_empty() {
                    missing_fallback_root_reasons
                        .insert(MoliSourceInvalidationFallbackReason::InexactEmptyResult);
                    continue;
                }
                // This request is intentionally fallback-only: the context
                // roots are the cleanup target for structural relative
                // dependencies whose exact query would otherwise report an
                // inexact empty result. Other co-batched queries must remain
                // in the exact-query set because these fallback roots are not
                // required to subsume unrelated custom-state or media-state
                // query targets.
                fallback_kind = moli_merge_retained_source_invalidation_fallback_kind(
                    fallback_kind,
                    Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback),
                );
                fallback_reasons
                    .insert(MoliSourceInvalidationFallbackReason::InexactEmptyResult);
                moli_push_unique_roots(
                    &mut reasoned_fallback_roots,
                    &mut reasoned_fallback_seen,
                    &context_roots,
                );
                continue;
            }
            moli_push_unique_roots(
                &mut exact_safety_fallback_roots,
                &mut exact_safety_fallback_seen,
                &context_roots,
            );
        } else if !selected_fallback_roots.is_empty() {
            moli_push_unique_roots(
                &mut exact_safety_fallback_roots,
                &mut exact_safety_fallback_seen,
                selected_fallback_roots,
            );
        }
        exact_queries.push((*request.query()).clone());
    }
    if !missing_fallback_root_reasons.is_empty() {
        return moli_source_dependency_requires_source_fallback_plan(
            selected_fallback_roots,
            missing_fallback_root_reasons,
        );
    }
    MoliSourceDependencyInvalidationSourcePlan::work(
        MoliPlannedSourceDependencyInvalidationTarget::from_source_dependency_work_parts(
            exact_queries,
            fallback_kind,
            reasoned_fallback_roots,
            exact_safety_fallback_roots,
            fallback_reasons,
        ),
        exact_structural_cleanup_roots,
    )
}

/// Build source-local invalidation plans for all stylesheet sources that can be
/// affected by a Moli mutation.
///
/// The embedder owns source scopes and DOM traversal; this planner owns source
/// dependency interpretation, target normalization, empty-target fallback, and
/// structural-boundary cleanup selection.
pub fn moli_source_dependency_invalidation_batch_plan<
    Root: Copy + Eq + Hash,
    ContextRootsProvider,
>(
    sources: &[MoliSourceDependencyInvalidationBatchSource<'_, Root>],
    requests: &[MoliSourceDependencyInvalidationRequest<'_, Root>],
    boundary_roots: MoliSourceDependencyBoundaryRoots<'_, Root>,
    context_roots_provider: &mut ContextRootsProvider,
) -> MoliSourceDependencyInvalidationBatchPlan<Root>
where
    ContextRootsProvider: MoliSourceDependencyInvalidationContextRootsProvider<Root>,
{
    let mut planned_sources = Vec::new();
    let mut structural_boundary_fallback_source: Option<(usize, Vec<Root>)> = None;
    let mut nonstructural_empty_target_fallback_source: Option<(usize, Vec<Root>)> = None;
    for (source_index, source) in sources.iter().enumerate() {
        let selected_fallback_roots = source.selected_fallback_roots();
        if source
            .dependency_summary()
            .has_child_list_structural_dependency_for_requests(requests)
        {
            let has_fallback_roots = !selected_fallback_roots.is_empty();
            let should_replace_empty_target_source = match &structural_boundary_fallback_source {
                None => true,
                Some((_, roots)) => roots.is_empty() && has_fallback_roots,
            };
            if should_replace_empty_target_source {
                structural_boundary_fallback_source =
                    Some((source_index, selected_fallback_roots.to_vec()));
            }
        }
        if source
            .dependency_summary()
            .requires_nonstructural_empty_target_fallback_for_requests(requests)
        {
            let has_fallback_roots = !selected_fallback_roots.is_empty();
            let should_replace_empty_target_source =
                match &nonstructural_empty_target_fallback_source {
                    None => true,
                    Some((_, roots)) => roots.is_empty() && has_fallback_roots,
                };
            if should_replace_empty_target_source {
                nonstructural_empty_target_fallback_source =
                    Some((source_index, selected_fallback_roots.to_vec()));
            }
        }
        match moli_source_dependency_invalidation_plan(
            source.dependency_summary(),
            selected_fallback_roots,
            requests,
            context_roots_provider,
        ) {
            MoliSourceDependencyInvalidationSourcePlan::Work {
                target,
                exact_structural_cleanup_roots,
            } => {
                let Some(target) = target else {
                    continue;
                };
                let mut structural_boundary_cleanup_roots = source
                    .dependency_summary()
                    .structural_boundary_cleanup_roots_for_requests(
                        requests,
                        boundary_roots.relative_previous_sibling_cleanup_roots,
                    );
                let mut structural_boundary_cleanup_seen = structural_boundary_cleanup_roots
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>();
                moli_push_unique_roots(
                    &mut structural_boundary_cleanup_roots,
                    &mut structural_boundary_cleanup_seen,
                    &exact_structural_cleanup_roots,
                );
                let planned_source = MoliPlannedSourceDependencyInvalidation::from_target(
                    source_index,
                    target,
                    structural_boundary_cleanup_roots,
                );
                planned_sources.push(planned_source);
            },
            MoliSourceDependencyInvalidationSourcePlan::RequiresSourceFallback { target } => {
                return MoliSourceDependencyInvalidationBatchPlan::requires_source_fallback(
                    MoliPlannedSourceDependencyInvalidation::from_target(
                        source_index,
                        target,
                        Vec::new(),
                    ),
                );
            },
        }
    }
    let empty_target_fallback_source = structural_boundary_fallback_source.or_else(|| {
        planned_sources
            .is_empty()
            .then_some(nonstructural_empty_target_fallback_source)
            .flatten()
    });
    let boundary_fallback = match empty_target_fallback_source {
        Some((source_index, selected_fallback_roots)) => {
            if boundary_roots.empty_target_fallback_roots.is_empty() {
                return MoliSourceDependencyInvalidationBatchPlan::requires_source_fallback(
                        MoliPlannedSourceDependencyInvalidation::from_target(
                            source_index,
                            MoliPlannedSourceDependencyInvalidationTarget::source_dependency_fallback(
                                selected_fallback_roots,
                                [MoliSourceInvalidationFallbackReason::InexactEmptyResult],
                            ),
                            Vec::new(),
                        ),
                    );
            }
            Some(
                MoliPlannedFallbackRootInvalidationTarget::fallback_only(
                    boundary_roots.empty_target_fallback_roots.to_vec(),
                    [MoliSourceInvalidationFallbackReason::InexactEmptyResult],
                ),
            )
        },
        None => None,
    };
    MoliSourceDependencyInvalidationBatchPlan::work(planned_sources, boundary_fallback)
}

fn moli_source_dependency_requires_source_fallback_plan<Root: Copy>(
    selected_fallback_roots: &[Root],
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
) -> MoliSourceDependencyInvalidationSourcePlan<Root> {
    MoliSourceDependencyInvalidationSourcePlan::requires_source_fallback(
        MoliPlannedSourceDependencyInvalidationTarget::source_dependency_fallback(
            selected_fallback_roots.to_vec(),
            fallback_reasons,
        ),
    )
}

fn moli_push_unique_roots<Root: Copy + Eq + Hash>(
    roots: &mut Vec<Root>,
    seen: &mut HashSet<Root>,
    incoming: &[Root],
) {
    for &root in incoming {
        if seen.insert(root) {
            roots.push(root);
        }
    }
}

/// Result for one source-local Moli retained style invalidation query.
///
/// The DOM adapter still supplies concrete roots, but exactness, matched
/// dependency counts, fallback reasons, and merge behavior are Stylo-facing
/// query semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationQueryResult<Root> {
    affected_roots: Vec<Root>,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
}

/// Builder for one source-local retained style invalidation query result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationQueryResultBuilder<Root: Eq + Hash> {
    affected_roots: Vec<Root>,
    affected_root_set: HashSet<Root>,
    empty_result_is_exact: bool,
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
}

/// Snapshot-relative affected roots and verification state for one retained
/// query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSnapshotRelativeDependencyRoots<Root> {
    roots: Vec<Root>,
    verified_dependency_count: usize,
}

/// Policy for normal retained invalidation after snapshot-relative roots have
/// been collected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoliNormalStyleInvalidationDependencyPlan {
    drop_relative_dependencies: bool,
    empty_result_is_exact: bool,
}

/// Policy for relative retained invalidation after snapshot-relative roots have
/// been collected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MoliRelativeStyleInvalidationDependencyPlan {
    empty_result_is_exact: bool,
}

impl<Root> Default for MoliSnapshotRelativeDependencyRoots<Root> {
    #[inline]
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            verified_dependency_count: 0,
        }
    }
}

impl<Root> Default for MoliSourceStyleInvalidationQueryResult<Root> {
    #[inline]
    fn default() -> Self {
        Self {
            affected_roots: Vec::new(),
            empty_result_is_exact: false,
            matched_dependency_count: 0,
            fallback_reasons: IndexSet::new(),
        }
    }
}

impl<Root> Default for MoliSourceStyleInvalidationQueryResultBuilder<Root>
where
    Root: Eq + Hash,
{
    #[inline]
    fn default() -> Self {
        Self {
            affected_roots: Vec::new(),
            affected_root_set: HashSet::new(),
            empty_result_is_exact: true,
            fallback_reasons: IndexSet::new(),
        }
    }
}

/// Internal source-local invalidation result after a batch of Moli
/// retained dependency queries has been merged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationResult<Root> {
    affected_roots: Vec<Root>,
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    fallback_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
    fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
}

/// Drainable parts for one classified source-local invalidation result.
pub struct MoliSourceStyleInvalidationResultParts<Root> {
    affected_roots: Vec<Root>,
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    fallback_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
    fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
}

/// Sink used to drain source-local invalidation result policy into its owner.
pub trait MoliSourceStyleInvalidationResultSink<Root> {
    /// Record a fully classified source-local invalidation result artifact.
    fn set_source_style_invalidation_result(
        &mut self,
        parts: MoliSourceStyleInvalidationResultParts<Root>,
    );
}

/// Sink used by diagnostics and tests that need source-local result parts.
pub trait MoliSourceStyleInvalidationResultPartsSink<Root> {
    /// Record the classified source-local invalidation result parts.
    fn set_source_style_invalidation_result_parts(
        &mut self,
        affected_roots: Vec<Root>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        fallback_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
        fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
    );
}

impl<Root> MoliSourceStyleInvalidationQueryResult<Root> {
    /// Construct an exact empty result for a query whose matched dependencies
    /// have already been accounted for by the fork-owned dependency plan.
    #[inline]
    pub fn exact_empty(matched_dependency_count: usize) -> Self {
        Self {
            affected_roots: Vec::new(),
            empty_result_is_exact: true,
            matched_dependency_count,
            fallback_reasons: IndexSet::new(),
        }
    }

    /// Construct a single-query invalidation result from already-classified
    /// parts.
    #[inline]
    fn from_parts(
        affected_roots: Vec<Root>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        Self {
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons: fallback_reasons.into_iter().collect(),
        }
    }

    /// Consume this query result and drain affected roots into a runtime-owned
    /// root collection.
    #[inline]
    pub fn drain_affected_roots_into(self, target: &mut impl Extend<Root>) {
        target.extend(self.affected_roots);
    }
}

impl<Root> MoliSourceStyleInvalidationResultParts<Root> {
    #[inline]
    fn from_result(result: MoliSourceStyleInvalidationResult<Root>) -> Self {
        Self {
            affected_roots: result.affected_roots,
            fallback_reasons: result.fallback_reasons,
            fallback_kind: result.fallback_kind,
            fallback_root_availability: result.fallback_root_availability,
            empty_result_is_exact: result.empty_result_is_exact,
            matched_dependency_count: result.matched_dependency_count,
        }
    }

    /// Drain this artifact into a result-parts sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationResultPartsSink<Root>,
    ) {
        target.set_source_style_invalidation_result_parts(
            self.affected_roots,
            self.fallback_reasons,
            self.fallback_kind,
            self.fallback_root_availability,
            self.empty_result_is_exact,
            self.matched_dependency_count,
        );
    }
}

impl<Root> MoliSourceStyleInvalidationQueryResultBuilder<Root>
where
    Root: Copy + Eq + Hash,
{
    /// Create an empty query result builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an affected root while preserving first-seen order.
    #[inline]
    pub fn note_affected_root(&mut self, root: Root) {
        moli_push_unique_root(&mut self.affected_roots, &mut self.affected_root_set, root);
    }

    /// Record several affected roots while preserving first-seen order.
    #[inline]
    pub fn extend_affected_roots(&mut self, roots: impl IntoIterator<Item = Root>) {
        for root in roots {
            self.note_affected_root(root);
        }
    }

    /// Return whether any affected roots have been recorded.
    #[inline]
    pub fn has_affected_roots(&self) -> bool {
        !self.affected_roots.is_empty()
    }

    /// Record whether the current dependency can prove an empty result exact.
    #[inline]
    pub fn note_empty_result_supported(&mut self, supported: bool) {
        self.empty_result_is_exact &= supported;
    }

    /// Record a fallback reason for this query.
    #[inline]
    pub fn note_fallback_reason(&mut self, reason: MoliSourceInvalidationFallbackReason) {
        self.fallback_reasons.insert(reason);
    }

    /// Consume this builder into a typed single-query result.
    #[inline]
    pub fn into_query_result(
        self,
        matched_dependency_count: usize,
    ) -> MoliSourceStyleInvalidationQueryResult<Root> {
        MoliSourceStyleInvalidationQueryResult::from_parts(
            self.affected_roots,
            self.empty_result_is_exact,
            matched_dependency_count,
            self.fallback_reasons,
        )
    }
}

/// Runtime-provided mapping from a Stylo element to the retained invalidation
/// root stored in Moli's source-local result.
pub trait MoliStyleInvalidationElementRoot<E, Root>
where
    E: TElement + Copy,
{
    /// Return the Moli root represented by a Stylo invalidated element.
    fn root_for_style_invalidation_element(&self, element: E) -> Root;
}

/// Moli-facing invalidation processor for Servo's tree invalidator.
///
/// The embedder supplies concrete elements, snapshots, and a root mapper. This
/// processor owns selector dependency action application, retained-vs-fallback
/// effect classification, affected-root collection, and fallback reason recording.
pub struct MoliStyleInvalidationProcessor<'a, 'b, E, Root, RootMapper>
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    result_builder: MoliSourceStyleInvalidationQueryResultBuilder<Root>,
    matching_context: MatchingContext<'b, E::Impl>,
    traversal_map: SiblingTraversalMap<E>,
    dependencies: Vec<&'a Dependency>,
    snapshot_map: Option<&'b SnapshotMap>,
    root_mapper: RootMapper,
}

impl<'a, 'b, E, Root, RootMapper> MoliStyleInvalidationProcessor<'a, 'b, E, Root, RootMapper>
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    /// Create a Moli retained invalidation processor from already prepared
    /// Stylo invalidator inputs.
    #[inline]
    pub fn new(
        matching_context: MatchingContext<'b, E::Impl>,
        traversal_map: SiblingTraversalMap<E>,
        dependencies: Vec<&'a Dependency>,
        snapshot_map: Option<&'b SnapshotMap>,
        root_mapper: RootMapper,
    ) -> Self {
        Self {
            result_builder: MoliSourceStyleInvalidationQueryResultBuilder::new(),
            matching_context,
            traversal_map,
            dependencies,
            snapshot_map,
            root_mapper,
        }
    }

    /// Record an already computed affected root.
    #[inline]
    pub fn note_affected_root(&mut self, root: Root) {
        self.result_builder.note_affected_root(root);
    }

    /// Consume this processor into one source-local query result.
    #[inline]
    pub fn into_query_result(
        self,
        matched_dependency_count: usize,
    ) -> MoliSourceStyleInvalidationQueryResult<Root> {
        self.result_builder
            .into_query_result(matched_dependency_count)
    }

    fn note_affected_element(&mut self, element: E) {
        let root = self
            .root_mapper
            .root_for_style_invalidation_element(element);
        self.note_affected_root(root);
    }

    fn note_dependency(
        &mut self,
        element: E,
        dependency: &'a Dependency,
        descendant_invalidations: &mut DescendantInvalidationLists<'a>,
        sibling_invalidations: &mut InvalidationVector<'a>,
    ) -> bool {
        let mut application = MoliDependencyInvalidationActionApplication {
            processor: self,
            element,
            dependency,
            descendant_invalidations,
            sibling_invalidations,
            invalidates_self: false,
        };
        moli_dependency_invalidation_action(dependency).drain_into(&mut application);
        application.invalidates_self
    }
}

impl<'a, 'b, E, Root, RootMapper> Extend<Root>
    for MoliStyleInvalidationProcessor<'a, 'b, E, Root, RootMapper>
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    #[inline]
    fn extend<T>(&mut self, iter: T)
    where
        T: IntoIterator<Item = Root>,
    {
        self.result_builder.extend_affected_roots(iter);
    }
}

impl<'a, 'b, E, Root, RootMapper> InvalidationProcessor<'a, 'b, E>
    for MoliStyleInvalidationProcessor<'a, 'b, E, Root, RootMapper>
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    fn invalidates_on_pseudo_element(&self) -> bool {
        true
    }

    fn check_outer_dependency(
        &mut self,
        dependency: &Dependency,
        element: E,
        scope: Option<OpaqueElement>,
    ) -> bool {
        let Some(snapshot_map) = self.snapshot_map else {
            return true;
        };
        moli_dependency_changes_anchor_with_snapshot(
            dependency,
            element,
            snapshot_map,
            &mut self.matching_context,
            scope,
        )
    }

    fn matching_context(&mut self) -> &mut MatchingContext<'b, E::Impl> {
        &mut self.matching_context
    }

    fn sibling_traversal_map(&self) -> &SiblingTraversalMap<E> {
        &self.traversal_map
    }

    fn collect_invalidations(
        &mut self,
        element: E,
        _self_invalidations: &mut InvalidationVector<'a>,
        descendant_invalidations: &mut DescendantInvalidationLists<'a>,
        sibling_invalidations: &mut InvalidationVector<'a>,
    ) -> bool {
        let mut invalidates_self = false;
        for dependency in self.dependencies.clone() {
            let empty_result_is_exact =
                match moli_retained_processor_dependency_effect(dependency) {
                    MoliRetainedProcessorDependencyEffect::Retained {
                        empty_result_is_exact,
                    } => empty_result_is_exact,
                    MoliRetainedProcessorDependencyEffect::Fallback(reason) => {
                        self.result_builder.note_fallback_reason(reason);
                        continue;
                    },
                };
            self.result_builder
                .note_empty_result_supported(empty_result_is_exact);
            invalidates_self |= self.note_dependency(
                element,
                dependency,
                descendant_invalidations,
                sibling_invalidations,
            );
        }
        if invalidates_self {
            self.note_affected_element(element);
        }
        invalidates_self
    }

    fn should_process_descendants(&mut self, _element: E) -> bool {
        true
    }

    fn recursion_limit_exceeded(&mut self, element: E) {
        self.note_affected_element(element);
    }

    fn invalidated_self(&mut self, element: E) {
        self.note_affected_element(element);
    }

    fn invalidated_sibling(&mut self, sibling: E, _of: E) {
        self.note_affected_element(sibling);
    }

    fn invalidated_descendants(&mut self, _element: E, child: E) {
        self.note_affected_element(child);
    }

    fn found_relative_selector_invalidation(
        &mut self,
        _element: E,
        kind: RelativeDependencyInvalidationKind,
        relative_dependency: &'a Dependency,
    ) {
        self.result_builder.note_fallback_reason(
            moli_relative_selector_invalidation_fallback_reason(kind, relative_dependency),
        );
    }
}

struct MoliDependencyInvalidationActionApplication<
    'processor,
    'a,
    'b,
    'vectors,
    E,
    Root,
    RootMapper,
> where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    processor: &'processor mut MoliStyleInvalidationProcessor<'a, 'b, E, Root, RootMapper>,
    element: E,
    dependency: &'a Dependency,
    descendant_invalidations: &'vectors mut DescendantInvalidationLists<'a>,
    sibling_invalidations: &'vectors mut InvalidationVector<'a>,
    invalidates_self: bool,
}

impl<'processor, 'a, 'b, 'vectors, E, Root, RootMapper>
    MoliDependencyInvalidationActionApplication<
        'processor,
        'a,
        'b,
        'vectors,
        E,
        Root,
        RootMapper,
    >
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    fn invalidation(&self) -> Invalidation<'a> {
        Invalidation::new(
            self.dependency,
            self.processor.matching_context.current_host,
            self.processor.matching_context.scope_element,
        )
    }
}

impl<'processor, 'a, 'b, 'vectors, E, Root, RootMapper> MoliDependencyInvalidationActionSink
    for MoliDependencyInvalidationActionApplication<
        'processor,
        'a,
        'b,
        'vectors,
        E,
        Root,
        RootMapper,
    >
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    fn invalidate_element(&mut self) {
        self.invalidates_self = true;
    }

    fn invalidate_element_and_descendants(&mut self) {
        self.descendant_invalidations
            .dom_descendants
            .push(self.invalidation());
        self.invalidates_self = true;
    }

    fn invalidate_descendants(&mut self) {
        self.descendant_invalidations
            .dom_descendants
            .push(self.invalidation());
    }

    fn invalidate_siblings(&mut self) {
        self.sibling_invalidations.push(self.invalidation());
    }

    fn invalidate_slotted_elements(&mut self) {
        self.descendant_invalidations
            .slotted_descendants
            .push(self.invalidation());
    }

    fn invalidate_parts(&mut self) {
        self.descendant_invalidations
            .parts
            .push(self.invalidation());
    }

    fn invalidate_fallback(&mut self, reason: MoliSourceInvalidationFallbackReason) {
        self.processor.result_builder.note_fallback_reason(reason);
    }

    fn invalidate_scope(&mut self, action: MoliScopeDependencyInvalidationAction) {
        action.drain_into(self);
    }
}

impl<'processor, 'a, 'b, 'vectors, E, Root, RootMapper>
    MoliScopeDependencyInvalidationActionSink
    for MoliDependencyInvalidationActionApplication<
        'processor,
        'a,
        'b,
        'vectors,
        E,
        Root,
        RootMapper,
    >
where
    E: TElement + Copy,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
{
    fn invalidate_implicit_scope(&mut self) {
        if let Some(next) = self.dependency.next.as_ref() {
            for dep in next.as_ref().slice() {
                self.descendant_invalidations.dom_descendants.push(
                    Invalidation::new_always_effective_for_next_descendant(
                        dep,
                        self.processor.matching_context.current_host,
                        self.processor.matching_context.scope_element,
                    ),
                );
            }
        }
    }

    fn invalidate_scope_force_at_subject(&mut self, force_add: bool) {
        self.descendant_invalidations.dom_descendants.extend(
            note_scope_dependency_force_at_subject(
                self.dependency,
                self.processor.matching_context.current_host,
                self.processor.matching_context.scope_element,
                force_add,
            ),
        );
        self.invalidates_self = true;
    }

    fn invalidate_scope_check_next(&mut self) {
        let Some(next) = self.dependency.next.as_ref() else {
            return;
        };
        let scope = Some(self.element.opaque());
        for dep in next.as_ref().slice() {
            if self
                .processor
                .check_outer_dependency(dep, self.element, scope)
            {
                self.invalidates_self |= self.processor.note_dependency(
                    self.element,
                    dep,
                    self.descendant_invalidations,
                    self.sibling_invalidations,
                );
            }
        }
    }

    fn invalidate_scope_by_combinator(&mut self) {
        let invalidation = self.invalidation();
        if invalidation.combinator_to_right().is_sibling() {
            self.sibling_invalidations.push(invalidation);
        } else {
            self.descendant_invalidations
                .dom_descendants
                .push(invalidation);
        }
        self.invalidates_self = true;
    }
}

impl<Root> MoliSnapshotRelativeDependencyRoots<Root> {
    /// Create snapshot-relative roots from already-collected DOM roots and the
    /// number of relative dependencies that were verified by snapshots.
    #[inline]
    pub fn new(roots: Vec<Root>, verified_dependency_count: usize) -> Self {
        Self {
            roots,
            verified_dependency_count,
        }
    }

    /// Snapshot-relative roots collected for this query.
    #[inline]
    fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// Return whether every matched dependency was verified by snapshots.
    #[inline]
    fn verified_all_dependencies(
        &self,
        matched_dependency_count: usize,
        dependency_count: usize,
    ) -> bool {
        dependency_count != 0
            && self.verified_dependency_count == dependency_count
            && matched_dependency_count == dependency_count
    }

    /// Return whether every collected dependency in one dependency subset was
    /// verified by snapshots.
    #[inline]
    fn verified_all_collected_dependencies(&self, dependency_count: usize) -> bool {
        dependency_count != 0 && self.verified_dependency_count == dependency_count
    }

    /// Return whether a relative query's empty result can be treated as exact.
    #[inline]
    fn empty_result_is_exact(
        &self,
        matched_dependency_count: usize,
        dependency_count: usize,
        has_affected_roots: bool,
    ) -> bool {
        matched_dependency_count == 0
            || has_affected_roots
            || self.verified_all_dependencies(matched_dependency_count, dependency_count)
    }

    /// Consume this snapshot-relative result and drain affected roots into an
    /// invalidation root collection.
    #[inline]
    pub fn drain_affected_roots_into(self, target: &mut impl Extend<Root>) {
        target.extend(self.roots);
    }
}

impl MoliNormalStyleInvalidationDependencyPlan {
    /// Drain this plan into an adapter-owned normal invalidation action sink.
    #[inline]
    pub fn drain_into(self, target: &mut impl MoliNormalStyleInvalidationDependencyPlanSink) {
        if self.should_drop_relative_dependencies() {
            target.drop_collected_relative_dependencies();
        }
        if self.empty_result_is_exact() {
            target.record_exact_empty_result();
        }
    }

    /// Return whether normal invalidation should drop collected relative
    /// dependencies before running the normal invalidator.
    #[inline]
    fn should_drop_relative_dependencies(&self) -> bool {
        self.drop_relative_dependencies
    }

    /// Return whether the normal invalidation query is an exact empty result.
    #[inline]
    fn empty_result_is_exact(&self) -> bool {
        self.empty_result_is_exact
    }
}

impl MoliRelativeStyleInvalidationDependencyPlan {
    /// Return whether the relative invalidation query is an exact empty result.
    #[inline]
    fn empty_result_is_exact(&self) -> bool {
        self.empty_result_is_exact
    }
}

/// Sink for normal invalidation dependency plan actions.
pub trait MoliNormalStyleInvalidationDependencyPlanSink {
    /// Drop relative dependencies collected by the normal invalidator before
    /// running it.
    fn drop_collected_relative_dependencies(&mut self);

    /// Record that this normal invalidation query can return exact empty.
    fn record_exact_empty_result(&mut self);
}

/// Return normal invalidation dependency policy after snapshot-relative
/// dependency collection.
#[inline]
pub fn moli_normal_style_invalidation_dependency_plan<Root>(
    query: MoliStyleInvalidationQuery<'_>,
    matched_dependency_count: usize,
    relative_dependency_count: usize,
    snapshot_relative_roots: &MoliSnapshotRelativeDependencyRoots<Root>,
) -> MoliNormalStyleInvalidationDependencyPlan {
    let drop_relative_dependencies = query.drops_collected_relative_dependencies()
        || snapshot_relative_roots.verified_all_collected_dependencies(relative_dependency_count);
    let remaining_dependency_count = if drop_relative_dependencies {
        matched_dependency_count.saturating_sub(relative_dependency_count)
    } else {
        matched_dependency_count
    };
    MoliNormalStyleInvalidationDependencyPlan {
        drop_relative_dependencies,
        empty_result_is_exact: remaining_dependency_count == 0
            && snapshot_relative_roots.roots().is_empty(),
    }
}

/// Return relative invalidation dependency policy after the relative invalidator
/// and snapshot-relative dependency collection have both run.
#[inline]
fn moli_relative_style_invalidation_dependency_plan<Root>(
    matched_dependency_count: usize,
    relative_dependency_count: usize,
    has_affected_roots: bool,
    snapshot_relative_roots: &MoliSnapshotRelativeDependencyRoots<Root>,
) -> MoliRelativeStyleInvalidationDependencyPlan {
    MoliRelativeStyleInvalidationDependencyPlan {
        empty_result_is_exact: snapshot_relative_roots.empty_result_is_exact(
            matched_dependency_count,
            relative_dependency_count,
            has_affected_roots,
        ),
    }
}

/// Build one relative invalidation query result from Servo relative invalidator
/// affected roots and snapshot-relative roots.
#[inline]
fn moli_relative_style_invalidation_query_result<Root>(
    direct_affected_roots: impl IntoIterator<Item = Root>,
    snapshot_relative_roots: &MoliSnapshotRelativeDependencyRoots<Root>,
    matched_dependency_count: usize,
    relative_dependency_count: usize,
) -> MoliSourceStyleInvalidationQueryResult<Root>
where
    Root: Copy + Eq + Hash,
{
    let mut result_builder = MoliSourceStyleInvalidationQueryResultBuilder::new();
    result_builder.extend_affected_roots(direct_affected_roots);
    result_builder.extend_affected_roots(snapshot_relative_roots.roots().iter().copied());
    let dependency_plan = moli_relative_style_invalidation_dependency_plan(
        matched_dependency_count,
        relative_dependency_count,
        result_builder.has_affected_roots(),
        snapshot_relative_roots,
    );
    result_builder.note_empty_result_supported(dependency_plan.empty_result_is_exact());
    result_builder.into_query_result(matched_dependency_count)
}

/// Run Servo's relative selector invalidator for one source-local Moli
/// query and return the fork-owned query result.
#[inline]
pub fn moli_collect_relative_style_invalidation_query_result<
    'a,
    'b,
    E,
    Root,
    RootMapper,
    SnapshotRelativeRoots,
>(
    root_mapper: RootMapper,
    element: E,
    stylist: &'a Stylist,
    query: MoliStyleInvalidationQuery<'_>,
    quirks_mode: QuirksMode,
    snapshot_table: Option<&'b SnapshotMap>,
    sibling_traversal_map: SiblingTraversalMap<E>,
    collect_snapshot_relative_roots: SnapshotRelativeRoots,
) -> MoliSourceStyleInvalidationQueryResult<Root>
where
    E: TElement + Copy + 'a,
    Root: Copy + Eq + Hash,
    RootMapper: MoliStyleInvalidationElementRoot<E, Root>,
    SnapshotRelativeRoots: FnOnce(
        &[(Option<OpaqueElement>, &'a Dependency)],
    ) -> MoliSnapshotRelativeDependencyRoots<Root>,
{
    let mut matched_dependency_count = 0;
    let mut snapshot_relative_dependencies = Vec::new();
    let affected_roots = RefCell::new(Vec::new());

    {
        let collect_affected_root = |affected: E| {
            affected_roots
                .borrow_mut()
                .push(root_mapper.root_for_style_invalidation_element(affected));
        };

        let invalidator = RelativeSelectorInvalidator {
            element,
            quirks_mode,
            snapshot_table,
            invalidated: moli_ignore_relative_selector_invalidation::<E>,
            affected: Some(&collect_affected_root),
            sibling_traversal_map,
            _marker: std::marker::PhantomData,
        };
        invalidator.invalidate_relative_selectors_for_this(
            stylist,
            |candidate, scope, cascade_data, _quirks_mode, collector| {
                let mut dependencies = Vec::new();
                moli_collect_dependencies_from_invalidation_map(
                    cascade_data.relative_selector_invalidation_map(),
                    *candidate,
                    query,
                    &mut dependencies,
                );
                moli_collect_dependencies_from_additional_relative_invalidation_map(
                    cascade_data.relative_invalidation_map_attributes(),
                    query,
                    &mut dependencies,
                );
                matched_dependency_count += dependencies.len();
                for dependency in dependencies {
                    if moli_dependency_is_relative_selector(dependency) {
                        snapshot_relative_dependencies.push((scope, dependency));
                    }
                    collector.add_dependency(dependency, *candidate, scope);
                }
            },
        );
    }

    let snapshot_relative_roots = collect_snapshot_relative_roots(&snapshot_relative_dependencies);
    moli_relative_style_invalidation_query_result(
        affected_roots.into_inner(),
        &snapshot_relative_roots,
        matched_dependency_count,
        snapshot_relative_dependencies.len(),
    )
}

fn moli_ignore_relative_selector_invalidation<E>(_element: E, _result: &InvalidationResult) {}

impl MoliStyleInvalidationQuery<'_> {
    fn drops_collected_relative_dependencies(&self) -> bool {
        matches!(self, Self::CustomState(_))
    }
}

impl<Root> MoliSourceStyleInvalidationQueryResult<Root>
where
    Root: Eq + Hash,
{
    /// Merge two query results while preserving first-seen root and fallback
    /// reason order.
    #[inline]
    fn merged_with(self, other: Self) -> Self {
        let mut affected_roots = IndexSet::new();
        affected_roots.extend(self.affected_roots.into_iter().chain(other.affected_roots));

        let mut fallback_reasons = self.fallback_reasons;
        fallback_reasons.extend(other.fallback_reasons);

        Self {
            affected_roots: affected_roots.into_iter().collect(),
            empty_result_is_exact: self.empty_result_is_exact && other.empty_result_is_exact,
            matched_dependency_count: self.matched_dependency_count
                + other.matched_dependency_count,
            fallback_reasons,
        }
    }
}

/// Merge two source-local query results while preserving Stylo-owned fallback
/// and exactness semantics.
#[inline]
pub fn moli_merge_source_style_invalidation_query_results<Root>(
    existing: MoliSourceStyleInvalidationQueryResult<Root>,
    incoming: MoliSourceStyleInvalidationQueryResult<Root>,
) -> MoliSourceStyleInvalidationQueryResult<Root>
where
    Root: Eq + Hash,
{
    existing.merged_with(incoming)
}

impl<Root> MoliSourceStyleInvalidationResult<Root> {
    /// Construct a source-local invalidation result from already-classified
    /// parts.
    #[inline]
    fn from_parts(
        affected_roots: Vec<Root>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        fallback_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
        fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
    ) -> Self {
        Self {
            affected_roots,
            fallback_reasons,
            fallback_kind,
            fallback_root_availability,
            empty_result_is_exact,
            matched_dependency_count,
        }
    }

    /// Drain this result into the source-result policy owner.
    #[inline]
    pub fn drain_into(self, target: &mut impl MoliSourceStyleInvalidationResultSink<Root>) {
        target.set_source_style_invalidation_result(
            MoliSourceStyleInvalidationResultParts::from_result(self),
        );
    }
}

/// Accumulates query-local affected roots and fallback reasons for one retained
/// stylesheet source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationResultAccumulator<Root: Eq + Hash> {
    affected_roots: Vec<Root>,
    affected_root_set: HashSet<Root>,
    fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
}

impl<Root> MoliSourceStyleInvalidationResultAccumulator<Root>
where
    Root: Copy + Eq + Hash,
{
    /// Create an empty source-local result accumulator.
    #[inline]
    pub fn new() -> Self {
        Self {
            affected_roots: Vec::new(),
            affected_root_set: HashSet::new(),
            fallback_reasons: IndexSet::new(),
            empty_result_is_exact: true,
            matched_dependency_count: 0,
        }
    }

    /// Merge one retained dependency-query result into this source-local
    /// accumulator.
    #[inline]
    fn merge_query_result(
        &mut self,
        affected_roots: Vec<Root>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    ) {
        self.fallback_reasons.extend(fallback_reasons);
        self.empty_result_is_exact &= empty_result_is_exact;
        self.matched_dependency_count += matched_dependency_count;
        for root in affected_roots {
            moli_push_unique_root(
                &mut self.affected_roots,
                &mut self.affected_root_set,
                root,
            );
        }
    }

    /// Merge one typed retained dependency-query result into this source-local
    /// accumulator.
    #[inline]
    pub fn merge_invalidation_query_result(
        &mut self,
        result: MoliSourceStyleInvalidationQueryResult<Root>,
    ) {
        let MoliSourceStyleInvalidationQueryResult {
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons,
        } = result;
        self.merge_query_result(
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons,
        );
    }

    /// Convert the accumulated query results into the source-local result
    /// policy Moli should apply for this stylesheet source.
    #[inline]
    pub fn into_source_result(
        self,
        exact_safety_fallback_roots: &IndexSet<Root>,
    ) -> MoliSourceStyleInvalidationResult<Root> {
        self.into_source_result_with_zero_matched_dependency_exactness(
            exact_safety_fallback_roots,
            false,
        )
    }

    /// Convert accumulated query results using the source plan's proof for a
    /// zero matched-dependency result.
    ///
    /// `zero_matched_dependency_result_is_exact` must only be true when every
    /// retained query was selected from a fully supported source dependency
    /// branch. Optimistic retained queries that carry a fallback policy must
    /// keep this false so their exact-safety roots remain effective.
    #[inline]
    pub fn into_source_result_with_zero_matched_dependency_exactness(
        mut self,
        exact_safety_fallback_roots: &IndexSet<Root>,
        zero_matched_dependency_result_is_exact: bool,
    ) -> MoliSourceStyleInvalidationResult<Root> {
        let needs_source_fallback = !self.fallback_reasons.is_empty()
            || self.has_inexact_empty_result(zero_matched_dependency_result_is_exact);
        if needs_source_fallback && self.fallback_reasons.is_empty() {
            self.fallback_reasons
                .insert(MoliSourceInvalidationFallbackReason::InexactEmptyResult);
        }
        if needs_source_fallback && exact_safety_fallback_roots.is_empty() {
            self.fallback_reasons
                .insert(MoliSourceInvalidationFallbackReason::MissingFallbackRoots);
            return MoliSourceStyleInvalidationResult::from_parts(
                self.affected_roots,
                self.fallback_reasons,
                Some(MoliSourceStyleInvalidationSourceResultKind::MissingFallbackRoots),
                Some(MoliSourceFallbackRootAvailability::Missing),
                self.empty_result_is_exact,
                self.matched_dependency_count,
            );
        }
        if needs_source_fallback {
            self.affected_roots.clear();
            self.affected_root_set.clear();
            for &root in exact_safety_fallback_roots {
                moli_push_unique_root(
                    &mut self.affected_roots,
                    &mut self.affected_root_set,
                    root,
                );
            }
        }
        MoliSourceStyleInvalidationResult::from_parts(
            self.affected_roots,
            self.fallback_reasons,
            needs_source_fallback
                .then_some(MoliSourceStyleInvalidationSourceResultKind::Fallback),
            if needs_source_fallback {
                MoliSourceFallbackRootAvailability::for_root_count(
                    exact_safety_fallback_roots.len(),
                )
            } else {
                None
            },
            self.empty_result_is_exact,
            self.matched_dependency_count,
        )
    }

    fn has_inexact_empty_result(
        &self,
        zero_matched_dependency_result_is_exact: bool,
    ) -> bool {
        self.affected_roots.is_empty()
            && (!self.empty_result_is_exact
                || (self.matched_dependency_count == 0
                    && !zero_matched_dependency_result_is_exact))
    }
}

impl<Root> Default for MoliSourceStyleInvalidationResultAccumulator<Root>
where
    Root: Copy + Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

fn moli_push_unique_root<Root: Copy + Eq + Hash>(
    roots: &mut Vec<Root>,
    root_set: &mut HashSet<Root>,
    root: Root,
) {
    if root_set.insert(root) {
        roots.push(root);
    }
}

/// Moli-facing retained invalidation result for a source-aware batch.
///
/// The source result table is the stored fact table. Runtime-specific cleanup
/// owners drain the table through sink traits instead of reading fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliInvalidationResult<Root> {
    source_results: Vec<MoliSourceStyleInvalidationSourceResult<Root>>,
}

/// Builder for a Moli-facing retained invalidation source-result table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliInvalidationResultBuilder<Root> {
    source_results: Vec<MoliSourceStyleInvalidationSourceResult<Root>>,
}

/// Sink used by a runtime owner to consume a Moli retained invalidation
/// source result table.
pub trait MoliInvalidationSourceResultsSink<Root> {
    /// Record how many source-result rows will be drained.
    fn record_moli_invalidation_source_result_count(&mut self, count: usize);

    /// Record one retained source-result row.
    fn record_moli_invalidation_source_result(
        &mut self,
        result: MoliSourceStyleInvalidationSourceResult<Root>,
    );
}

impl<Root> MoliInvalidationResult<Root> {
    /// Create a result table from already classified source-result rows.
    #[inline]
    fn from_source_results(
        source_results: Vec<MoliSourceStyleInvalidationSourceResult<Root>>,
    ) -> Self {
        Self { source_results }
    }

    /// Drain source-result rows into a runtime-owned sink.
    #[inline]
    pub fn drain_source_results_into(
        self,
        target: &mut impl MoliInvalidationSourceResultsSink<Root>,
    ) {
        target.record_moli_invalidation_source_result_count(self.source_results.len());
        for result in self.source_results {
            target.record_moli_invalidation_source_result(result);
        }
    }
}

impl<Root> Default for MoliInvalidationResult<Root> {
    fn default() -> Self {
        Self {
            source_results: Vec::new(),
        }
    }
}

impl<Root> Default for MoliInvalidationResultBuilder<Root> {
    fn default() -> Self {
        Self {
            source_results: Vec::new(),
        }
    }
}

impl<Root> MoliInvalidationResultBuilder<Root> {
    /// Create an empty source-result table builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an already-classified source-result row.
    #[inline]
    fn push_source_result(&mut self, result: MoliSourceStyleInvalidationSourceResult<Root>) {
        self.source_results.push(result);
    }

    /// Push an exact source-result row.
    #[inline]
    pub fn push_exact_source_result(
        &mut self,
        source_index: usize,
        affected_roots: Vec<Root>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
    ) {
        self.push_source_result(MoliSourceStyleInvalidationSourceResult::exact_result(
            source_index,
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
        ));
    }

    /// Push a fallback source-result row.
    #[inline]
    pub fn push_fallback_source_result(
        &mut self,
        source_index: usize,
        kind: MoliSourceStyleInvalidationSourceResultKind,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: impl IntoIterator<Item = MoliSourceInvalidationFallbackReason>,
        fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        affected_roots: Vec<Root>,
    ) {
        self.push_source_result(MoliSourceStyleInvalidationSourceResult::fallback(
            source_index,
            kind,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons.into_iter().collect(),
            fallback_root_availability,
            affected_roots,
        ));
    }

    /// Finish and return the Moli-facing retained invalidation result.
    #[inline]
    pub fn finish(self) -> MoliInvalidationResult<Root> {
        MoliInvalidationResult::from_source_results(self.source_results)
    }
}

impl<Root> MoliInvalidationResultBuilder<Root>
where
    Root: Copy + Eq + Hash,
{
    /// Push a fallback-only source-result row.
    #[inline]
    pub fn push_fallback_only_source(
        &mut self,
        source_index: usize,
        kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
        fallback_roots: &IndexSet<Root>,
    ) {
        self.push_source_result(
            MoliSourceStyleInvalidationSourceResult::fallback_only(
                source_index,
                kind,
                fallback_reasons,
                fallback_roots,
            ),
        );
    }

    /// Push a final source-result row from source-local query result policy and
    /// a planned fallback policy.
    #[inline]
    pub fn push_source_result_from_planned_fallback(
        &mut self,
        source_index: usize,
        source_result: MoliSourceStyleInvalidationResult<Root>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: &IndexSet<Root>,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
    ) {
        self.push_source_result(
            MoliSourceStyleInvalidationSourceResult::from_source_result_and_planned_fallback(
                source_index,
                source_result,
                fallback_kind,
                reasoned_fallback_roots,
                fallback_reasons,
            ),
        );
    }

    /// Push a retained source-result row for an unavailable retained system or
    /// source cascade.
    #[inline]
    pub fn push_unavailable_retained_source(
        &mut self,
        source_index: usize,
        reason: MoliSourceInvalidationFallbackReason,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
        reasoned_fallback_roots: &IndexSet<Root>,
        exact_safety_fallback_roots: &IndexSet<Root>,
    ) {
        self.push_source_result(
            MoliSourceStyleInvalidationSourceResult::unavailable_retained_source(
                source_index,
                reason,
                fallback_reasons,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
            ),
        );
    }

    /// Push a source-result row for a missing retained style system.
    #[inline]
    pub fn push_missing_retained_style_system_source(
        &mut self,
        source_index: usize,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
        reasoned_fallback_roots: &IndexSet<Root>,
        exact_safety_fallback_roots: &IndexSet<Root>,
    ) {
        self.push_unavailable_retained_source(
            source_index,
            MoliSourceInvalidationFallbackReason::MissingRetainedStyleSystem,
            fallback_reasons,
            reasoned_fallback_roots,
            exact_safety_fallback_roots,
        );
    }

    /// Push a source-result row for missing retained source cascade data.
    #[inline]
    pub fn push_missing_retained_cascade_data_source(
        &mut self,
        source_index: usize,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
        reasoned_fallback_roots: &IndexSet<Root>,
        exact_safety_fallback_roots: &IndexSet<Root>,
    ) {
        self.push_unavailable_retained_source(
            source_index,
            MoliSourceInvalidationFallbackReason::MissingRetainedCascadeData,
            fallback_reasons,
            reasoned_fallback_roots,
            exact_safety_fallback_roots,
        );
    }
}

/// One source in a retained source invalidation batch and how it was resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationSourceResult<Root> {
    source_index: usize,
    kind: MoliSourceStyleInvalidationSourceResultKind,
    exact: bool,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
    fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
    fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
    affected_roots: Vec<Root>,
}

/// Diagnostic facts for one retained source-result row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationTargetResultDiagnosticFacts {
    kind: MoliSourceStyleInvalidationSourceResultKind,
    exact: bool,
    empty_result_is_exact: bool,
    matched_dependency_count: usize,
    fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
    fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
    affected_root_count: usize,
}

/// Cleanup facts for one retained source-result row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationTargetResultCleanupFacts {
    fallback_context_reasons: Vec<MoliSourceInvalidationFallbackReason>,
    clear_all_cleanup_reasons: Vec<MoliSourceInvalidationFallbackReason>,
    include_fallback_context_for_clear_all: bool,
    requires_fallback_handling: bool,
}

/// Cleanup and optional diagnostic facts for one retained source-result row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceStyleInvalidationTargetResultRecord {
    cleanup_facts: MoliSourceStyleInvalidationTargetResultCleanupFacts,
    diagnostic_facts: Option<MoliSourceStyleInvalidationTargetResultDiagnosticFacts>,
}

/// Drainable parts for one retained source-result row.
pub struct MoliSourceStyleInvalidationSourceResultParts<Root> {
    source_index: usize,
    affected_roots: MoliSourceAffectedRootsCleanup<Root>,
    target_result_record: MoliSourceStyleInvalidationTargetResultRecord,
}

/// Affected roots classified for exact cleanup or source-fallback cleanup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoliSourceAffectedRootsCleanup<Root> {
    kind: MoliSourceAffectedRootKind,
    roots: Vec<Root>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MoliSourceAffectedRootKind {
    Exact,
    SourceFallback,
}

/// Sink used to drain diagnostic facts for one retained source-result row.
pub trait MoliSourceStyleInvalidationTargetResultDiagnosticFactsSink {
    /// Set diagnostic facts for one retained source-result row.
    fn set_source_style_invalidation_target_result_diagnostic_facts(
        &mut self,
        facts: MoliSourceStyleInvalidationTargetResultDiagnosticFacts,
    );
}

/// Sink used by diagnostic owners that consume target-result diagnostic fields.
pub trait MoliSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink {
    /// Set diagnostic fact fields for one retained source-result row.
    fn set_source_style_invalidation_target_result_diagnostic_fact_parts(
        &mut self,
        kind: MoliSourceStyleInvalidationSourceResultKind,
        exact: bool,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        affected_root_count: usize,
    );
}

/// Sink used to drain cleanup facts for one retained source-result row.
pub trait MoliSourceStyleInvalidationTargetResultCleanupFactsSink {
    /// Set cleanup facts for one retained source-result row.
    fn set_source_style_invalidation_target_result_cleanup_facts(
        &mut self,
        facts: MoliSourceStyleInvalidationTargetResultCleanupFacts,
    );
}

/// Sink used by cleanup owners that consume target-result cleanup fields.
pub trait MoliSourceStyleInvalidationTargetResultCleanupFactsPartsSink {
    /// Set cleanup fact fields for one retained source-result row.
    fn set_source_style_invalidation_target_result_cleanup_fact_parts(
        &mut self,
        fallback_context_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        clear_all_cleanup_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        include_fallback_context_for_clear_all: bool,
        requires_fallback_handling: bool,
    );
}

/// Sink used to drain affected roots from one retained source-result row.
pub trait MoliSourceAffectedRootsCleanupSink<Root> {
    /// Extend exact affected roots.
    fn extend_exact_affected_roots(&mut self, roots: &[Root]);

    /// Extend source-fallback roots.
    fn extend_source_fallback_roots(&mut self, roots: &[Root]);
}

/// Sink used to consume retained source-result rows.
pub trait MoliSourceStyleInvalidationSourceResultSink<Root> {
    /// Return whether diagnostic target-result facts should be retained.
    fn retain_source_style_invalidation_target_result_diagnostics(&self) -> bool {
        true
    }

    /// Record one retained source-result row artifact.
    fn record_source_style_invalidation_source_result(
        &mut self,
        parts: MoliSourceStyleInvalidationSourceResultParts<Root>,
    );
}

/// Sink used by runtime owners that consume source-result row parts.
pub trait MoliSourceStyleInvalidationSourceResultPartsSink<Root> {
    /// Record one retained source-result row's classified parts.
    fn record_source_style_invalidation_source_result_parts(
        &mut self,
        source_index: usize,
        affected_roots: MoliSourceAffectedRootsCleanup<Root>,
        target_result_record: MoliSourceStyleInvalidationTargetResultRecord,
    );
}

impl MoliSourceStyleInvalidationTargetResultDiagnosticFacts {
    /// Drain this diagnostic row into a runtime-owned sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationTargetResultDiagnosticFactsSink,
    ) {
        target.set_source_style_invalidation_target_result_diagnostic_facts(self);
    }

    /// Drain this diagnostic row into a field-level sink.
    #[inline]
    pub fn drain_parts_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink,
    ) {
        target.set_source_style_invalidation_target_result_diagnostic_fact_parts(
            self.kind,
            self.exact,
            self.empty_result_is_exact,
            self.matched_dependency_count,
            self.fallback_reasons,
            self.fallback_root_availability,
            self.affected_root_count,
        );
    }
}

impl MoliSourceStyleInvalidationTargetResultCleanupFacts {
    /// Drain this cleanup row into a runtime-owned sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationTargetResultCleanupFactsSink,
    ) {
        target.set_source_style_invalidation_target_result_cleanup_facts(self);
    }

    /// Drain this cleanup row into a field-level sink.
    #[inline]
    pub fn drain_parts_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationTargetResultCleanupFactsPartsSink,
    ) {
        target.set_source_style_invalidation_target_result_cleanup_fact_parts(
            self.fallback_context_reasons,
            self.clear_all_cleanup_reasons,
            self.include_fallback_context_for_clear_all,
            self.requires_fallback_handling,
        );
    }
}

impl MoliSourceStyleInvalidationTargetResultRecord {
    fn with_diagnostic_facts(
        diagnostic_facts: MoliSourceStyleInvalidationTargetResultDiagnosticFacts,
        cleanup_facts: MoliSourceStyleInvalidationTargetResultCleanupFacts,
    ) -> Self {
        Self {
            cleanup_facts,
            diagnostic_facts: Some(diagnostic_facts),
        }
    }

    fn cleanup_only(
        cleanup_facts: MoliSourceStyleInvalidationTargetResultCleanupFacts,
    ) -> Self {
        Self {
            cleanup_facts,
            diagnostic_facts: None,
        }
    }

    /// Drain cleanup facts and return optional diagnostic facts.
    #[inline]
    pub fn drain_cleanup_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationTargetResultCleanupFactsSink,
    ) -> Option<MoliSourceStyleInvalidationTargetResultDiagnosticFacts> {
        self.cleanup_facts.drain_into(target);
        self.diagnostic_facts
    }
}

impl<Root> MoliSourceStyleInvalidationSourceResultParts<Root> {
    /// Drain this source-result row artifact into a parts sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationSourceResultPartsSink<Root>,
    ) {
        target.record_source_style_invalidation_source_result_parts(
            self.source_index,
            self.affected_roots,
            self.target_result_record,
        );
    }
}

impl<Root> MoliSourceAffectedRootsCleanup<Root> {
    fn new(kind: MoliSourceAffectedRootKind, roots: Vec<Root>) -> Self {
        Self { kind, roots }
    }

    /// Drain affected roots into a runtime-owned sink.
    #[inline]
    pub fn drain_into(self, target: &mut impl MoliSourceAffectedRootsCleanupSink<Root>) {
        match self.kind {
            MoliSourceAffectedRootKind::Exact => {
                target.extend_exact_affected_roots(&self.roots);
            },
            MoliSourceAffectedRootKind::SourceFallback => {
                target.extend_source_fallback_roots(&self.roots);
            },
        }
    }
}

impl<Root> MoliSourceStyleInvalidationSourceResult<Root>
where
    Root: Copy + Eq + Hash,
{
    /// Build a fallback-only source-result row.
    #[inline]
    fn fallback_only(
        source_index: usize,
        kind: MoliRetainedSourceStyleInvalidationKind,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
        fallback_roots: &IndexSet<Root>,
    ) -> Self {
        Self::fallback(
            source_index,
            kind.fallback_source_result_kind(!fallback_reasons.is_empty()),
            fallback_roots.is_empty(),
            0,
            fallback_reasons.iter().copied().collect(),
            kind.fallback_root_availability(fallback_roots.len()),
            fallback_roots.iter().copied().collect(),
        )
    }

    /// Build a source-result row for a retained source that could not be
    /// queried.
    #[inline]
    fn unavailable_retained_source(
        source_index: usize,
        reason: MoliSourceInvalidationFallbackReason,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
        reasoned_fallback_roots: &IndexSet<Root>,
        exact_safety_fallback_roots: &IndexSet<Root>,
    ) -> Self {
        let kind = match reason {
            MoliSourceInvalidationFallbackReason::MissingRetainedStyleSystem => {
                MoliSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem
            },
            MoliSourceInvalidationFallbackReason::MissingRetainedCascadeData => {
                MoliSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData
            },
            _ => MoliSourceStyleInvalidationSourceResultKind::Fallback,
        };
        let mut reasons = fallback_reasons.iter().copied().collect::<Vec<_>>();
        if !fallback_reasons.contains(&reason) {
            reasons.push(reason);
        }
        let fallback_roots =
            moli_union_fallback_roots(reasoned_fallback_roots, exact_safety_fallback_roots);
        Self::fallback(
            source_index,
            kind,
            false,
            0,
            reasons,
            MoliSourceFallbackRootAvailability::for_root_count(fallback_roots.len()),
            fallback_roots.iter().copied().collect(),
        )
    }

    /// Build a final source-result row from source-local query result policy
    /// and a planned fallback policy.
    #[inline]
    fn from_source_result_and_planned_fallback(
        source_index: usize,
        source_result: MoliSourceStyleInvalidationResult<Root>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        reasoned_fallback_roots: &IndexSet<Root>,
        fallback_reasons: &IndexSet<MoliSourceInvalidationFallbackReason>,
    ) -> Self {
        let MoliSourceStyleInvalidationResult {
            mut affected_roots,
            fallback_reasons: source_fallback_reasons,
            fallback_kind: source_fallback_kind,
            fallback_root_availability: source_fallback_root_availability,
            empty_result_is_exact,
            matched_dependency_count,
        } = source_result;
        if !fallback_reasons.is_empty() {
            let mut affected_root_set = affected_roots.iter().copied().collect();
            for &root in reasoned_fallback_roots {
                moli_push_unique_root(&mut affected_roots, &mut affected_root_set, root);
            }
        }
        let mut merged_fallback_reasons = fallback_reasons.iter().copied().collect::<IndexSet<_>>();
        merged_fallback_reasons.extend(source_fallback_reasons);
        if merged_fallback_reasons.is_empty() {
            return Self::exact_result(
                source_index,
                affected_roots,
                empty_result_is_exact,
                matched_dependency_count,
            );
        }
        let kind = source_fallback_kind
            .or_else(|| fallback_kind.map(|kind| kind.fallback_source_result_kind(true)))
            .unwrap_or(MoliSourceStyleInvalidationSourceResultKind::Fallback);
        let fallback_root_availability = source_fallback_root_availability.or_else(|| {
            fallback_kind
                .and_then(|kind| kind.fallback_root_availability(reasoned_fallback_roots.len()))
        });
        Self::fallback(
            source_index,
            kind,
            empty_result_is_exact,
            matched_dependency_count,
            merged_fallback_reasons.into_iter().collect(),
            fallback_root_availability,
            affected_roots,
        )
    }
}

impl<Root> MoliSourceStyleInvalidationSourceResult<Root> {
    /// Build an exact source-result row.
    #[inline]
    fn exact_result(
        source_index: usize,
        affected_roots: Vec<Root>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
    ) -> Self {
        Self {
            source_index,
            kind: MoliSourceStyleInvalidationSourceResultKind::Exact,
            exact: true,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons: Vec::new(),
            fallback_root_availability: None,
            affected_roots,
        }
    }

    /// Build a fallback source-result row.
    #[inline]
    fn fallback(
        source_index: usize,
        kind: MoliSourceStyleInvalidationSourceResultKind,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        affected_roots: Vec<Root>,
    ) -> Self {
        debug_assert_ne!(
            kind,
            MoliSourceStyleInvalidationSourceResultKind::Exact,
            "exact source results should use MoliSourceStyleInvalidationSourceResult::exact_result"
        );
        Self {
            source_index,
            kind,
            exact: false,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons,
            fallback_root_availability,
            affected_roots,
        }
    }

    /// Drain this source-result row into a runtime-owned sink.
    #[inline]
    pub fn drain_into(
        self,
        target: &mut impl MoliSourceStyleInvalidationSourceResultSink<Root>,
    ) {
        let retain_diagnostics =
            target.retain_source_style_invalidation_target_result_diagnostics();
        target.record_source_style_invalidation_source_result(
            self.into_source_result_cleanup_and_target_record_parts(retain_diagnostics),
        );
    }

    fn into_source_result_cleanup_and_target_record_parts(
        self,
        retain_diagnostics: bool,
    ) -> MoliSourceStyleInvalidationSourceResultParts<Root> {
        let source_index = self.source_index;
        let affected_root_kind = self.affected_root_kind();
        let affected_root_count = self.affected_root_count();
        let clear_all_cleanup_reasons = self.clear_all_cleanup_reasons();
        let include_fallback_context_for_clear_all =
            moli_source_result_kind_includes_fallback_context_for_clear_all(self.kind);
        let requires_fallback_handling = self.requires_fallback_handling();
        let affected_roots =
            MoliSourceAffectedRootsCleanup::new(affected_root_kind, self.affected_roots);
        let target_result_record = if retain_diagnostics {
            let fallback_context_reasons = self.fallback_reasons.clone();
            MoliSourceStyleInvalidationTargetResultRecord::with_diagnostic_facts(
                MoliSourceStyleInvalidationTargetResultDiagnosticFacts {
                    kind: self.kind,
                    exact: self.exact,
                    empty_result_is_exact: self.empty_result_is_exact,
                    matched_dependency_count: self.matched_dependency_count,
                    fallback_reasons: self.fallback_reasons,
                    fallback_root_availability: self.fallback_root_availability,
                    affected_root_count,
                },
                MoliSourceStyleInvalidationTargetResultCleanupFacts {
                    fallback_context_reasons,
                    clear_all_cleanup_reasons,
                    include_fallback_context_for_clear_all,
                    requires_fallback_handling,
                },
            )
        } else {
            MoliSourceStyleInvalidationTargetResultRecord::cleanup_only(
                MoliSourceStyleInvalidationTargetResultCleanupFacts {
                    fallback_context_reasons: self.fallback_reasons,
                    clear_all_cleanup_reasons,
                    include_fallback_context_for_clear_all,
                    requires_fallback_handling,
                },
            )
        };
        MoliSourceStyleInvalidationSourceResultParts {
            source_index,
            affected_roots,
            target_result_record,
        }
    }

    fn affected_root_count(&self) -> usize {
        self.affected_roots.len()
    }

    fn is_exact_source_result(&self) -> bool {
        self.exact
            && self.kind == MoliSourceStyleInvalidationSourceResultKind::Exact
            && self.fallback_reasons.is_empty()
    }

    fn requires_fallback_handling(&self) -> bool {
        !self.is_exact_source_result()
    }

    fn has_inexact_empty_result(&self) -> bool {
        !(self.is_exact_source_result() && self.empty_result_is_exact)
    }

    fn has_fallback_clear_all_cleanup(&self) -> bool {
        matches!(
            self.fallback_root_availability,
            Some(MoliSourceFallbackRootAvailability::Missing)
        ) || (!self.fallback_reasons.is_empty() && self.affected_roots.is_empty())
    }

    fn has_inexact_empty_clear_all_cleanup(&self) -> bool {
        self.affected_roots.is_empty() && self.has_inexact_empty_result()
    }

    fn clear_all_cleanup_reasons(&self) -> Vec<MoliSourceInvalidationFallbackReason> {
        if self.has_fallback_clear_all_cleanup() {
            let mut reasons = self
                .fallback_reasons
                .iter()
                .copied()
                .collect::<IndexSet<_>>();
            if matches!(
                self.fallback_root_availability,
                Some(MoliSourceFallbackRootAvailability::Missing)
            ) {
                reasons.insert(MoliSourceInvalidationFallbackReason::MissingFallbackRoots);
            }
            return reasons.into_iter().collect();
        }
        if self.has_inexact_empty_clear_all_cleanup() {
            return vec![MoliSourceInvalidationFallbackReason::InexactEmptyResult];
        }
        Vec::new()
    }

    fn affected_root_kind(&self) -> MoliSourceAffectedRootKind {
        if self.is_exact_source_result() {
            return MoliSourceAffectedRootKind::Exact;
        }
        MoliSourceAffectedRootKind::SourceFallback
    }
}

fn moli_union_fallback_roots<Root: Copy + Eq + Hash>(
    fallback_roots: &IndexSet<Root>,
    exact_safety_fallback_roots: &IndexSet<Root>,
) -> IndexSet<Root> {
    let mut roots = fallback_roots.clone();
    roots.extend(exact_safety_fallback_roots.iter().copied());
    roots
}

fn moli_source_result_kind_includes_fallback_context_for_clear_all(
    kind: MoliSourceStyleInvalidationSourceResultKind,
) -> bool {
    matches!(
        kind,
        MoliSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem
            | MoliSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData
    )
}

/// Return the Moli fallback reason represented by a raw Stylo dependency
/// kind.
#[inline]
fn moli_dependency_fallback_reason_for_dependency(
    dependency: &Dependency,
) -> MoliDependencyFallbackReason {
    match dependency.invalidation_kind() {
        DependencyInvalidationKind::FullSelector => {
            MoliDependencyFallbackReason::FullSelector
        },
        DependencyInvalidationKind::Relative(_) => {
            MoliDependencyFallbackReason::RelativeAnySelector
        },
        DependencyInvalidationKind::Scope(_) => MoliDependencyFallbackReason::ScopeDependency,
        DependencyInvalidationKind::Normal(_) => {
            MoliDependencyFallbackReason::UnsupportedDependency
        },
    }
}

/// Collect dependencies matching one Moli query from a Stylo invalidation
/// map.
#[inline]
pub fn moli_collect_dependencies_from_invalidation_map<'a, E>(
    map: &'a InvalidationMap,
    element: E,
    query: MoliStyleInvalidationQuery<'_>,
    dependencies: &mut Vec<&'a Dependency>,
) where
    E: TElement,
{
    let quirks_mode = element.as_node().owner_doc().quirks_mode();
    match query {
        MoliStyleInvalidationQuery::Universal => {
            dependencies.extend(map.any_to_selector.iter());
        },
        MoliStyleInvalidationQuery::Type(local_name) => {
            if let Some(items) = map.type_to_selector.get(&LocalName::from(local_name)) {
                dependencies.extend(items.iter());
            }
        },
        MoliStyleInvalidationQuery::Attribute(name) => {
            if let Some(items) = map
                .other_attribute_affecting_selectors
                .get(&LocalName::from(name))
            {
                dependencies.extend(items.iter());
            }
        },
        MoliStyleInvalidationQuery::Class(token) => {
            if let Some(items) = map.class_to_selector.get(&Atom::from(token), quirks_mode) {
                dependencies.extend(items.iter());
            }
        },
        MoliStyleInvalidationQuery::Id(value) => {
            if let Some(items) = map.id_to_selector.get(&Atom::from(value), quirks_mode) {
                dependencies.extend(items.iter());
            }
        },
        MoliStyleInvalidationQuery::State(state) => {
            map.state_affecting_selectors.lookup_with_additional(
                element,
                quirks_mode,
                None,
                &[],
                state,
                |dependency| {
                    if dependency.state.intersects(state) {
                        dependencies.push(&dependency.dep);
                    }
                    true
                },
            );
        },
        MoliStyleInvalidationQuery::CustomState(name) => {
            if let Some(items) = map
                .custom_state_affecting_selectors
                .get(&AtomIdent::from(name))
            {
                dependencies.extend(items.iter());
            }
        },
    }
}

/// Collect dependencies matching one Moli query from Stylo's additional
/// relative selector invalidation map.
#[inline]
fn moli_collect_dependencies_from_additional_relative_invalidation_map<'a>(
    map: &'a AdditionalRelativeSelectorInvalidationMap,
    query: MoliStyleInvalidationQuery<'_>,
    dependencies: &mut Vec<&'a Dependency>,
) {
    if query == MoliStyleInvalidationQuery::Universal {
        dependencies.extend(map.any_to_selector.iter());
    }
    if let MoliStyleInvalidationQuery::Type(local_name) = query {
        if let Some(items) = map.type_to_selector.get(&LocalName::from(local_name)) {
            dependencies.extend(items.iter());
        }
    }
}

/// Return the Moli retained invalidation action represented by a raw
/// Stylo dependency.
#[inline]
fn moli_dependency_invalidation_action(
    dependency: &Dependency,
) -> MoliDependencyInvalidationAction {
    match dependency.invalidation_kind() {
        DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Element) => {
            MoliDependencyInvalidationAction::Element
        },
        DependencyInvalidationKind::Normal(
            NormalDependencyInvalidationKind::ElementAndDescendants,
        ) => MoliDependencyInvalidationAction::ElementAndDescendants,
        DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Descendants) => {
            MoliDependencyInvalidationAction::Descendants
        },
        DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Siblings) => {
            MoliDependencyInvalidationAction::Siblings
        },
        DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::SlottedElements) => {
            MoliDependencyInvalidationAction::SlottedElements
        },
        DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Parts) => {
            MoliDependencyInvalidationAction::Parts
        },
        DependencyInvalidationKind::Scope(scope_kind) => {
            MoliDependencyInvalidationAction::Scope(
                moli_scope_dependency_invalidation_action(dependency, scope_kind),
            )
        },
        DependencyInvalidationKind::FullSelector | DependencyInvalidationKind::Relative(_) => {
            MoliDependencyInvalidationAction::Fallback(
                MoliSourceInvalidationFallbackReason::from(
                    moli_dependency_fallback_reason_for_dependency(dependency),
                ),
            )
        },
    }
}

/// Return the fallback reason for a Servo relative selector invalidation
/// callback that Moli cannot yet represent as exact affected roots.
#[inline]
fn moli_relative_selector_invalidation_fallback_reason(
    _kind: RelativeDependencyInvalidationKind,
    _dependency: &Dependency,
) -> MoliSourceInvalidationFallbackReason {
    MoliSourceInvalidationFallbackReason::RelativeAnySelector
}

/// Return the Moli candidate traversal action for a relative selector
/// dependency.
#[inline]
fn moli_relative_dependency_invalidation_action(
    dependency: &Dependency,
) -> Option<MoliRelativeDependencyInvalidationAction> {
    let DependencyInvalidationKind::Relative(kind) = dependency.invalidation_kind() else {
        return None;
    };
    Some(moli_relative_dependency_action(kind))
}

/// Return whether this dependency is a relative selector dependency.
#[inline]
pub fn moli_dependency_is_relative_selector(dependency: &Dependency) -> bool {
    moli_relative_dependency_invalidation_action(dependency).is_some()
}

/// Return whether this dependency can be used as a snapshot-relative outer
/// dependency by Moli.
#[inline]
pub fn moli_snapshot_relative_outer_dependency_supported(dependency: &Dependency) -> bool {
    matches!(
        dependency.invalidation_kind(),
        DependencyInvalidationKind::Normal(
            NormalDependencyInvalidationKind::Element
                | NormalDependencyInvalidationKind::ElementAndDescendants
                | NormalDependencyInvalidationKind::Descendants
                | NormalDependencyInvalidationKind::Siblings
                | NormalDependencyInvalidationKind::SlottedElements
                | NormalDependencyInvalidationKind::Parts
        )
    )
}

fn moli_scope_dependency_invalidation_action(
    dependency: &Dependency,
    scope_kind: ScopeDependencyInvalidationKind,
) -> MoliScopeDependencyInvalidationAction {
    if scope_kind == ScopeDependencyInvalidationKind::ImplicitScope {
        return MoliScopeDependencyInvalidationAction::ImplicitScope;
    }
    if dependency.selector.is_rightmost(dependency.selector_offset) {
        let force_add = any_next_has_scope_in_negation(dependency);
        if scope_kind == ScopeDependencyInvalidationKind::ScopeEnd || force_add {
            return MoliScopeDependencyInvalidationAction::ForceAtSubject { force_add };
        }
        return MoliScopeDependencyInvalidationAction::CheckNextInScope;
    }
    MoliScopeDependencyInvalidationAction::PushByCombinator
}

fn moli_relative_dependency_action(
    kind: RelativeDependencyInvalidationKind,
) -> MoliRelativeDependencyInvalidationAction {
    match kind {
        RelativeDependencyInvalidationKind::Ancestors => {
            MoliRelativeDependencyInvalidationAction::Ancestors
        },
        RelativeDependencyInvalidationKind::Parent => {
            MoliRelativeDependencyInvalidationAction::Parent
        },
        RelativeDependencyInvalidationKind::PrevSibling => {
            MoliRelativeDependencyInvalidationAction::PrevSibling
        },
        RelativeDependencyInvalidationKind::AncestorPrevSibling => {
            MoliRelativeDependencyInvalidationAction::AncestorPrevSibling
        },
        RelativeDependencyInvalidationKind::EarlierSibling => {
            MoliRelativeDependencyInvalidationAction::EarlierSibling
        },
        RelativeDependencyInvalidationKind::AncestorEarlierSibling => {
            MoliRelativeDependencyInvalidationAction::AncestorEarlierSibling
        },
    }
}

/// Return whether Moli's retained invalidation processor can represent
/// this dependency without source fallback.
#[inline]
fn moli_dependency_supported_by_retained_processor(dependency: &Dependency) -> bool {
    if !matches!(
        dependency.invalidation_kind(),
        DependencyInvalidationKind::Normal(_) | DependencyInvalidationKind::Scope(_)
    ) {
        return false;
    }
    dependency.next.as_ref().is_none_or(|dependencies| {
        dependencies
            .as_ref()
            .slice()
            .iter()
            .all(moli_dependency_supported_by_retained_processor)
    })
}

/// Return whether an empty result for this dependency can be treated as an exact
/// no-op by Moli's retained invalidation processor.
#[inline]
fn moli_dependency_empty_result_supported_by_retained_processor(
    dependency: &Dependency,
) -> bool {
    if !matches!(
        dependency.invalidation_kind(),
        DependencyInvalidationKind::Scope(_)
            | DependencyInvalidationKind::Normal(
                NormalDependencyInvalidationKind::Element
                    | NormalDependencyInvalidationKind::ElementAndDescendants
                    | NormalDependencyInvalidationKind::Descendants
                    | NormalDependencyInvalidationKind::Siblings
                    | NormalDependencyInvalidationKind::SlottedElements
                    | NormalDependencyInvalidationKind::Parts
            )
    ) {
        return false;
    }
    dependency.next.as_ref().is_none_or(|dependencies| {
        dependencies
            .as_ref()
            .slice()
            .iter()
            .all(moli_dependency_empty_result_supported_by_retained_processor)
    })
}

/// Classify one raw dependency for Moli's retained invalidation
/// processor.
#[inline]
fn moli_retained_processor_dependency_effect(
    dependency: &Dependency,
) -> MoliRetainedProcessorDependencyEffect {
    if !moli_dependency_supported_by_retained_processor(dependency) {
        return MoliRetainedProcessorDependencyEffect::Fallback(
            MoliSourceInvalidationFallbackReason::from(
                moli_dependency_fallback_reason_for_dependency(dependency),
            ),
        );
    }

    MoliRetainedProcessorDependencyEffect::Retained {
        empty_result_is_exact: moli_dependency_empty_result_supported_by_retained_processor(
            dependency,
        ),
    }
}

/// Which fallback roots may be used when a dependency query is not exact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MoliDependencyFallbackRootPolicy {
    /// Mutation-context roots are sufficient as the conservative cleanup target.
    ContextRoots,
    /// The caller must use source-local or source-scope fallback roots.
    SourceFallback,
}

/// Source dependency fallback handling chosen from one dependency query result.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MoliDependencyFallbackHandling {
    /// Mutation context roots can satisfy the fallback, when available.
    ContextRoots(IndexSet<MoliSourceInvalidationFallbackReason>),
    /// The source's fallback roots are required.
    SourceFallback(IndexSet<MoliSourceInvalidationFallbackReason>),
}

/// Dependency root categories needed by Moli's DOM-backed fallback-root
/// construction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MoliDependencyContextRootFlags {
    /// The query cannot be covered by context roots and needs source fallback.
    requires_source_fallback: bool,
    /// The changed element's local subtree can be affected.
    local_subtree: bool,
    /// Ancestors of the changed element can be affected.
    ancestor_chain: bool,
    /// Following siblings of the changed element can be affected.
    following_siblings: bool,
    /// The query includes a direct previous-sibling relative dependency.
    direct_previous_sibling_relative: bool,
    /// The previous element sibling can be affected.
    previous_sibling: bool,
    /// Earlier element siblings can be affected.
    earlier_siblings: bool,
    /// Previous siblings of ancestors can be affected.
    ancestor_previous_siblings: bool,
    /// Earlier siblings of ancestors can be affected.
    ancestor_earlier_siblings: bool,
    /// Assigned elements matched by `::slotted(...)` can be affected.
    slotted_elements: bool,
    /// Exposed part elements matched by `::part(...)` can be affected.
    parts: bool,
}

/// Sink for DOM-backed context-root categories derived from one dependency
/// query result.
pub trait MoliDependencyContextRootFlagsSink {
    /// Context roots are insufficient and source fallback is required.
    fn require_context_source_fallback(&mut self);

    /// Include the changed element's local subtree.
    fn include_context_local_subtree(&mut self);

    /// Include the changed element's ancestor chain.
    fn include_context_ancestor_chain(&mut self);

    /// Include following siblings from the changed element or next-sibling
    /// context.
    fn include_context_following_siblings(&mut self);

    /// Include following siblings of ancestors.
    fn include_context_ancestor_following_siblings(&mut self);

    /// Include the previous sibling from mutation context, or the changed root
    /// if no previous sibling is known.
    fn include_context_previous_sibling(&mut self);

    /// Include earlier siblings from mutation context, or from the changed root
    /// if no previous sibling is known.
    fn include_context_earlier_siblings(&mut self);

    /// Include previous siblings of the changed root's ancestors.
    fn include_context_ancestor_previous_siblings(&mut self);

    /// Include assigned/slotted elements.
    fn include_context_slotted_elements(&mut self);

    /// Include exposed part roots.
    fn include_context_parts(&mut self);
}

/// Sink that can materialize DOM-backed context roots after Stylo has drained
/// the dependency root categories.
pub trait MoliDependencyInvalidationContextRootsSink<Root>:
    MoliDependencyContextRootFlagsSink + Sized
{
    /// Drain collected context roots into a Stylo-owned typed roots builder.
    fn drain_collected_context_roots_into(
        self,
        target: &mut impl MoliDependencyInvalidationContextRootsPartsSink<Root>,
    );
}

/// Sink for the final DOM-backed context roots collected by an adapter.
pub trait MoliDependencyInvalidationContextRootsPartsSink<Root> {
    /// Context roots are insufficient and source fallback is required.
    fn record_context_source_fallback(&mut self);

    /// Extend the typed context-root result with collected DOM roots.
    fn extend_context_roots(&mut self, roots: Vec<Root>);
}

struct MoliDependencyInvalidationContextRootsBuilder<Root> {
    requires_source_fallback: bool,
    roots: Vec<Root>,
}

impl<Root> MoliDependencyInvalidationContextRootsBuilder<Root> {
    #[inline]
    fn finish(self) -> MoliDependencyInvalidationContextRoots<Root> {
        MoliDependencyInvalidationContextRoots::new(self.requires_source_fallback, self.roots)
    }
}

impl<Root> Default for MoliDependencyInvalidationContextRootsBuilder<Root> {
    #[inline]
    fn default() -> Self {
        Self {
            requires_source_fallback: false,
            roots: Vec::new(),
        }
    }
}

impl<Root> MoliDependencyInvalidationContextRootsPartsSink<Root>
    for MoliDependencyInvalidationContextRootsBuilder<Root>
{
    #[inline]
    fn record_context_source_fallback(&mut self) {
        self.requires_source_fallback = true;
    }

    #[inline]
    fn extend_context_roots(&mut self, roots: Vec<Root>) {
        self.roots.extend(roots);
    }
}

impl MoliDependencyContextRootFlags {
    /// Drain these context-root categories into a DOM-backed sink.
    #[inline]
    fn drain_into(
        self,
        allow_direct_previous_following_sibling_fallback: bool,
        target: &mut impl MoliDependencyContextRootFlagsSink,
    ) {
        if self.requires_source_fallback {
            target.require_context_source_fallback();
        }
        if self.local_subtree {
            target.include_context_local_subtree();
        }
        if self.ancestor_chain {
            target.include_context_ancestor_chain();
        }
        if self.includes_following_siblings(allow_direct_previous_following_sibling_fallback) {
            if self.ancestor_chain {
                target.include_context_ancestor_following_siblings();
            } else {
                target.include_context_following_siblings();
            }
        }
        if self.previous_sibling {
            target.include_context_previous_sibling();
        }
        if self.earlier_siblings {
            target.include_context_earlier_siblings();
        }
        if self.ancestor_previous_siblings || self.ancestor_earlier_siblings {
            target.include_context_ancestor_previous_siblings();
        }
        if self.slotted_elements {
            target.include_context_slotted_elements();
        }
        if self.parts {
            target.include_context_parts();
        }
    }

    #[inline]
    fn includes_following_siblings(
        self,
        allow_direct_previous_following_sibling_fallback: bool,
    ) -> bool {
        self.following_siblings
            && !(!allow_direct_previous_following_sibling_fallback
                && self.direct_previous_sibling_relative
                && !self.earlier_siblings
                && !self.ancestor_previous_siblings
                && !self.ancestor_earlier_siblings)
    }
}

impl MoliDependencyContextRootPlan {
    #[inline]
    fn new(
        query: &MoliDependencyQueryResult,
        allow_direct_previous_following_sibling_fallback: bool,
    ) -> Self {
        Self {
            flags: query.context_root_flags(),
            allow_direct_previous_following_sibling_fallback,
        }
    }

    /// Drain this plan into a DOM-backed root sink.
    #[inline]
    pub fn drain_into<Root, Sink>(
        self,
        mut sink: Sink,
    ) -> MoliDependencyInvalidationContextRoots<Root>
    where
        Sink: MoliDependencyInvalidationContextRootsSink<Root>,
    {
        self.flags.drain_into(
            self.allow_direct_previous_following_sibling_fallback,
            &mut sink,
        );
        let mut builder = MoliDependencyInvalidationContextRootsBuilder::default();
        sink.drain_collected_context_roots_into(&mut builder);
        builder.finish()
    }
}

/// Build typed dependency context roots by draining one dependency query's
/// root-category plan into an adapter-provided DOM sink.
#[cfg(test)]
fn moli_dependency_invalidation_context_roots<Root, Sink>(
    query: &MoliDependencyQueryResult,
    allow_direct_previous_following_sibling_fallback: bool,
    sink: Sink,
) -> MoliDependencyInvalidationContextRoots<Root>
where
    Sink: MoliDependencyInvalidationContextRootsSink<Root>,
{
    MoliDependencyContextRootPlan::new(
        query,
        allow_direct_previous_following_sibling_fallback,
    )
    .drain_into(sink)
}

/// Conservative query result for one changed class/id/attribute/state token.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct MoliDependencyQueryResult {
    kinds: Vec<MoliDependencyKind>,
    unknown_dependency: bool,
    fallback_reasons: Vec<MoliDependencyFallbackReason>,
}

impl MoliDependencyQueryResult {
    fn add_kind(&mut self, kind: MoliDependencyKind) {
        if !self.kinds.contains(&kind) {
            self.kinds.push(kind);
        }
    }

    fn add_fallback_reason(&mut self, reason: MoliDependencyFallbackReason) {
        if !self.fallback_reasons.contains(&reason) {
            self.fallback_reasons.push(reason);
        }
        if matches!(
            reason,
            MoliDependencyFallbackReason::UnknownDependency
        ) {
            self.unknown_dependency = true;
        }
    }

    fn extend(&mut self, other: Self) {
        for kind in other.kinds {
            self.add_kind(kind);
        }
        self.unknown_dependency |= other.unknown_dependency;
        for reason in other.fallback_reasons {
            self.add_fallback_reason(reason);
        }
        if other.unknown_dependency {
            self.add_fallback_reason(MoliDependencyFallbackReason::UnknownDependency);
        }
    }

    /// Returns whether any dependency matched the query.
    #[inline]
    pub fn has_any_dependency(&self) -> bool {
        self.unknown_dependency || !self.kinds.is_empty() || !self.fallback_reasons.is_empty()
    }

    /// Returns whether this query can invalidate descendants of the changed
    /// element.
    #[inline]
    pub fn has_descendants_dependency(&self) -> bool {
        self.kinds.contains(&MoliDependencyKind::Descendants)
    }

    /// Returns whether this query can invalidate `::slotted(...)` elements.
    #[inline]
    pub fn has_slotted_elements_dependency(&self) -> bool {
        self.kinds
            .contains(&MoliDependencyKind::SlottedElements)
    }

    /// Returns whether this query can invalidate `::part(...)` elements.
    #[inline]
    pub fn has_parts_dependency(&self) -> bool {
        self.kinds.contains(&MoliDependencyKind::Parts)
    }

    /// Returns whether this query can invalidate relative selector ancestor
    /// anchors.
    #[inline]
    pub fn has_relative_ancestors_dependency(&self) -> bool {
        self.kinds
            .contains(&MoliDependencyKind::RelativeAncestors)
    }

    /// Returns the concrete dependency kinds captured for this query.
    #[inline]
    #[cfg(test)]
    fn kinds(&self) -> &[MoliDependencyKind] {
        &self.kinds
    }

    /// Returns conservative fallback reasons captured for this query.
    #[inline]
    #[cfg(test)]
    fn fallback_reasons(&self) -> &[MoliDependencyFallbackReason] {
        &self.fallback_reasons
    }

    /// Returns whether this query requires conservative fallback handling.
    #[inline]
    pub fn requires_fallback(&self) -> bool {
        !self.fallback_reasons.is_empty()
    }

    /// Returns the fallback-root policy for this dependency query.
    #[inline]
    fn fallback_root_policy(&self) -> MoliDependencyFallbackRootPolicy {
        if !self.fallback_reasons.is_empty()
            && self.fallback_reasons.iter().all(|reason| {
                matches!(
                    reason,
                    MoliDependencyFallbackReason::NestedRelativeSelectorDependency
                        | MoliDependencyFallbackReason::NthOfDependency
                )
            })
        {
            MoliDependencyFallbackRootPolicy::ContextRoots
        } else {
            MoliDependencyFallbackRootPolicy::SourceFallback
        }
    }

    /// Return whether this summary-only fallback may first be attempted by
    /// the retained invalidator with mutation-context roots as a safety net.
    #[inline]
    fn tries_retained_query_before_context_fallback(&self) -> bool {
        !self.fallback_reasons.is_empty()
            && self.fallback_reasons.iter().all(|reason| {
                matches!(
                    reason,
                    MoliDependencyFallbackReason::NestedRelativeSelectorDependency
                )
            })
    }

    /// Return whether the retained dependency walker can answer a query whose
    /// coarse mutation-context plan had to request source fallback.
    ///
    /// Scope dependencies need their raw selector offset and next-dependency
    /// chain, so the summary cannot derive complete context roots. The retained
    /// walker still has that data and supports every scope action. Try it first
    /// while keeping the source roots solely as an inexact-result safety net.
    #[inline]
    fn tries_retained_query_before_source_fallback(&self) -> bool {
        !self.requires_fallback() && self.kinds.contains(&MoliDependencyKind::Scope)
    }

    /// Return whether mutation-local structural roots fully cover this
    /// query's otherwise unsupported `:nth-child(... of ...)` dependency.
    #[inline]
    fn can_use_exact_nth_of_structural_roots(&self) -> bool {
        !self.fallback_reasons.is_empty()
            && self
                .fallback_reasons
                .iter()
                .all(|reason| matches!(reason, MoliDependencyFallbackReason::NthOfDependency))
    }

    /// Returns explicit fallback reasons, or conservative shape-derived reasons
    /// when the caller has already determined this query needs fallback handling.
    #[inline]
    fn fallback_or_shape_reasons(&self) -> Vec<MoliDependencyFallbackReason> {
        if !self.fallback_reasons.is_empty() {
            return self.fallback_reasons.clone();
        }
        if self.kinds.contains(&MoliDependencyKind::Scope) {
            return vec![MoliDependencyFallbackReason::ScopeDependency];
        }
        vec![MoliDependencyFallbackReason::UnsupportedDependency]
    }

    /// Returns source invalidation fallback reasons for this query result.
    #[inline]
    fn source_invalidation_fallback_reasons(
        &self,
    ) -> IndexSet<MoliSourceInvalidationFallbackReason> {
        self.fallback_or_shape_reasons()
            .into_iter()
            .map(MoliSourceInvalidationFallbackReason::from)
            .collect()
    }

    /// Returns source dependency fallback handling for this query result.
    #[inline]
    fn source_dependency_fallback_handling(&self) -> MoliDependencyFallbackHandling {
        let reasons = self.source_invalidation_fallback_reasons();
        match self.fallback_root_policy() {
            MoliDependencyFallbackRootPolicy::ContextRoots => {
                MoliDependencyFallbackHandling::ContextRoots(reasons)
            },
            MoliDependencyFallbackRootPolicy::SourceFallback => {
                MoliDependencyFallbackHandling::SourceFallback(reasons)
            },
        }
    }

    /// Returns whether the query can affect following sibling invalidation.
    #[inline]
    pub fn has_sibling_dependency(&self) -> bool {
        self.unknown_dependency
            || self.kinds.iter().any(|kind| {
                matches!(
                    kind,
                    MoliDependencyKind::Siblings
                        | MoliDependencyKind::RelativePrevSibling
                        | MoliDependencyKind::RelativeAncestorPrevSibling
                        | MoliDependencyKind::RelativeEarlierSibling
                        | MoliDependencyKind::RelativeAncestorEarlierSibling
                )
            })
    }

    /// Returns whether this query can affect relative selector anchors.
    #[inline]
    fn has_relative_selector_dependency(&self) -> bool {
        self.kinds.iter().any(|kind| {
            matches!(
                kind,
                MoliDependencyKind::RelativeAncestors
                    | MoliDependencyKind::RelativeParent
                    | MoliDependencyKind::RelativePrevSibling
                    | MoliDependencyKind::RelativeEarlierSibling
                    | MoliDependencyKind::RelativeAncestorPrevSibling
                    | MoliDependencyKind::RelativeAncestorEarlierSibling
            )
        }) || self.fallback_reasons.iter().any(|reason| {
            matches!(
                reason,
                MoliDependencyFallbackReason::RelativeAnySelector
                    | MoliDependencyFallbackReason::NestedRelativeSelectorDependency
            )
        })
    }

    /// Returns whether this query can affect previous-sibling relative selector
    /// anchors.
    #[inline]
    fn has_relative_previous_sibling_dependency(&self) -> bool {
        self.kinds.iter().any(|kind| {
            matches!(
                kind,
                MoliDependencyKind::RelativePrevSibling
                    | MoliDependencyKind::RelativeEarlierSibling
                    | MoliDependencyKind::RelativeAncestorPrevSibling
                    | MoliDependencyKind::RelativeAncestorEarlierSibling
            )
        })
    }

    /// Returns whether this query is limited to a direct previous-sibling
    /// relative dependency plus the following-sibling dependency Stylo records
    /// for `+`.
    #[inline]
    fn has_only_direct_relative_previous_sibling_dependency(&self) -> bool {
        !self.kinds.is_empty()
            && self.kinds.iter().all(|kind| {
                matches!(
                    kind,
                    MoliDependencyKind::RelativePrevSibling
                        | MoliDependencyKind::Siblings
                )
            })
            && self
                .kinds
                .contains(&MoliDependencyKind::RelativePrevSibling)
    }

    /// Returns whether this dependency needs structural context fallback
    /// cleanup for a universal child-list structural request.
    #[inline]
    fn requires_structural_context_fallback_cleanup(
        &self,
        request_requires_child_list_structural_dependency: bool,
        query_is_universal: bool,
    ) -> bool {
        request_requires_child_list_structural_dependency
            && query_is_universal
            && self.has_relative_selector_dependency()
    }

    /// Returns whether this query can affect `::slotted(...)` invalidation.
    #[inline]
    fn has_slotted_dependency(&self) -> bool {
        self.unknown_dependency
            || self
                .kinds
                .iter()
                .any(|kind| matches!(kind, MoliDependencyKind::SlottedElements))
    }

    /// Returns the fallback-root categories this query can affect.
    #[inline]
    fn context_root_flags(&self) -> MoliDependencyContextRootFlags {
        let mut flags = MoliDependencyContextRootFlags {
            requires_source_fallback: self.requires_fallback(),
            ..MoliDependencyContextRootFlags::default()
        };
        for kind in &self.kinds {
            match kind {
                MoliDependencyKind::Element
                | MoliDependencyKind::ElementAndDescendants
                | MoliDependencyKind::Descendants => {
                    flags.local_subtree = true;
                },
                MoliDependencyKind::Siblings => {
                    flags.following_siblings = true;
                },
                MoliDependencyKind::SlottedElements => {
                    flags.slotted_elements = true;
                },
                MoliDependencyKind::Parts => {
                    flags.parts = true;
                },
                MoliDependencyKind::RelativeAncestors
                | MoliDependencyKind::RelativeParent => {
                    flags.ancestor_chain = true;
                },
                MoliDependencyKind::RelativePrevSibling => {
                    flags.direct_previous_sibling_relative = true;
                    flags.previous_sibling = true;
                },
                MoliDependencyKind::RelativeEarlierSibling => {
                    flags.earlier_siblings = true;
                },
                MoliDependencyKind::RelativeAncestorPrevSibling => {
                    flags.ancestor_chain = true;
                    flags.ancestor_previous_siblings = true;
                },
                MoliDependencyKind::RelativeAncestorEarlierSibling => {
                    flags.ancestor_chain = true;
                    flags.ancestor_earlier_siblings = true;
                },
                MoliDependencyKind::Scope => {
                    // Source-batch planning only has a coarse dependency
                    // summary. Scope dependencies need the retained scope
                    // action walker with selector-offset and next-dependency
                    // context, so mutation context roots are not sufficient.
                    flags.requires_source_fallback = true;
                },
            }
        }
        flags
    }
}

/// Keyed dependency query summary retained inside Moli source metadata.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MoliDependencyInvalidationSummary {
    class_dependencies: Vec<(Atom, MoliDependencyQueryResult)>,
    id_dependencies: Vec<(Atom, MoliDependencyQueryResult)>,
    type_dependencies: Vec<(LocalName, MoliDependencyQueryResult)>,
    universal_dependency: MoliDependencyQueryResult,
    attribute_dependencies: Vec<(LocalName, MoliDependencyQueryResult)>,
    custom_state_dependencies: Vec<(AtomIdent, MoliDependencyQueryResult)>,
    state_dependencies: Vec<(u64, MoliDependencyQueryResult)>,
    unknown_state_dependency_bits: u64,
    focus_dependency: MoliDependencyQueryResult,
    focus_within_dependency: MoliDependencyQueryResult,
    target_dependency: MoliDependencyQueryResult,
    unknown_dependency: bool,
}

impl MoliDependencyInvalidationSummary {
    fn note_class_dependency(&mut self, class: Atom, result: MoliDependencyQueryResult) {
        moli_note_keyed_dependency(&mut self.class_dependencies, class, result);
    }

    fn note_id_dependency(&mut self, id: Atom, result: MoliDependencyQueryResult) {
        moli_note_keyed_dependency(&mut self.id_dependencies, id, result);
    }

    fn note_attribute_dependency(
        &mut self,
        attribute: LocalName,
        result: MoliDependencyQueryResult,
    ) {
        moli_note_keyed_dependency(&mut self.attribute_dependencies, attribute, result);
    }

    fn note_type_dependency(
        &mut self,
        local_name: LocalName,
        result: MoliDependencyQueryResult,
    ) {
        moli_note_keyed_dependency(&mut self.type_dependencies, local_name, result);
    }

    fn note_universal_dependency(&mut self, result: MoliDependencyQueryResult) {
        self.universal_dependency.extend(result);
    }

    fn note_state_dependency(
        &mut self,
        state: ElementState,
        result: MoliDependencyQueryResult,
    ) {
        if state.is_empty() {
            self.unknown_dependency = true;
            return;
        }
        moli_note_keyed_dependency(
            &mut self.state_dependencies,
            state.bits(),
            result.clone(),
        );
        if state.intersects(ElementState::FOCUS | ElementState::FOCUSRING) {
            self.focus_dependency.extend(result.clone());
        }
        if state.intersects(ElementState::FOCUS_WITHIN) {
            self.focus_dependency.extend(result.clone());
            self.focus_within_dependency.extend(result.clone());
        }
        if state.intersects(ElementState::URLTARGET) {
            self.target_dependency.extend(result);
        }
    }

    fn note_custom_state_dependency(
        &mut self,
        state: AtomIdent,
        result: MoliDependencyQueryResult,
    ) {
        moli_note_keyed_dependency(&mut self.custom_state_dependencies, state, result);
    }

    pub(crate) fn note_nth_of_class_dependency(&mut self, class: Atom) {
        self.note_class_dependency(class, moli_nth_of_dependency_query_result());
    }

    pub(crate) fn note_nth_of_id_dependency(&mut self, id: Atom) {
        self.note_id_dependency(id, moli_nth_of_dependency_query_result());
    }

    pub(crate) fn note_nth_of_attribute_dependency(&mut self, attribute: LocalName) {
        self.note_attribute_dependency(attribute, moli_nth_of_dependency_query_result());
    }

    pub(crate) fn note_nth_of_custom_state_dependency(&mut self, state: AtomIdent) {
        self.note_custom_state_dependency(state, moli_nth_of_dependency_query_result());
    }

    pub(crate) fn note_nth_of_state_dependency(&mut self, state: ElementState) {
        if state.is_empty() {
            return;
        }
        self.note_state_dependency(state, moli_nth_of_dependency_query_result());
    }

    pub(crate) fn note_unrepresented_state_dependencies(&mut self, state: ElementState) {
        if state.is_empty() {
            return;
        }
        let represented_bits = self
            .state_dependencies
            .iter()
            .fold(0, |bits, (state_bits, _)| bits | *state_bits);
        self.unknown_state_dependency_bits |= state.bits() & !represented_bits;
    }

    fn mark_unknown_dependency(&mut self) {
        self.unknown_dependency = true;
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for (class, result) in other.class_dependencies {
            self.note_class_dependency(class, result);
        }
        for (id, result) in other.id_dependencies {
            self.note_id_dependency(id, result);
        }
        for (attribute, result) in other.attribute_dependencies {
            self.note_attribute_dependency(attribute, result);
        }
        for (local_name, result) in other.type_dependencies {
            self.note_type_dependency(local_name, result);
        }
        for (state, result) in other.custom_state_dependencies {
            self.note_custom_state_dependency(state, result);
        }
        self.universal_dependency.extend(other.universal_dependency);
        for (state_bits, result) in other.state_dependencies {
            moli_note_keyed_dependency(&mut self.state_dependencies, state_bits, result);
        }
        self.unknown_state_dependency_bits |= other.unknown_state_dependency_bits;
        self.focus_dependency.extend(other.focus_dependency);
        self.focus_within_dependency
            .extend(other.focus_within_dependency);
        self.target_dependency.extend(other.target_dependency);
        self.unknown_dependency |= other.unknown_dependency;
    }

    /// Query dependencies for a changed class token.
    pub fn query_class(&self, class: &Atom) -> MoliDependencyQueryResult {
        self.class_dependencies
            .iter()
            .find_map(|(candidate, result)| (candidate == class).then(|| result.clone()))
            .unwrap_or_default()
    }

    /// Query dependencies for a changed id.
    pub fn query_id(&self, id: &Atom) -> MoliDependencyQueryResult {
        self.id_dependencies
            .iter()
            .find_map(|(candidate, result)| (candidate == id).then(|| result.clone()))
            .unwrap_or_default()
    }

    /// Query dependencies for a changed attribute.
    pub fn query_attribute(&self, attribute: &LocalName) -> MoliDependencyQueryResult {
        self.attribute_dependencies
            .iter()
            .find_map(|(candidate, result)| (candidate == attribute).then(|| result.clone()))
            .unwrap_or_default()
    }

    /// Query dependencies for an inserted or removed element local name.
    pub fn query_type(&self, local_name: &LocalName) -> MoliDependencyQueryResult {
        self.type_dependencies
            .iter()
            .find_map(|(candidate, result)| (candidate == local_name).then(|| result.clone()))
            .unwrap_or_default()
    }

    /// Query dependencies for an inserted or removed element matching `*`.
    pub fn query_universal(&self) -> MoliDependencyQueryResult {
        self.universal_dependency.clone()
    }

    /// Query dependencies for a changed element state bitset.
    pub fn query_state(&self, state: ElementState) -> MoliDependencyQueryResult {
        let mut result = MoliDependencyQueryResult::default();
        let bits = state.bits();
        for (candidate_bits, candidate_result) in &self.state_dependencies {
            if candidate_bits & bits != 0 {
                result.extend(candidate_result.clone());
            }
        }
        if self.unknown_state_dependency_bits & bits != 0 {
            result.add_fallback_reason(
                MoliDependencyFallbackReason::UnsupportedStateDependency,
            );
        }
        result
    }

    /// Query dependencies for a changed CSS custom state.
    pub fn query_custom_state(&self, state: &AtomIdent) -> MoliDependencyQueryResult {
        self.custom_state_dependencies
            .iter()
            .find_map(|(candidate, result)| (candidate == state).then(|| result.clone()))
            .unwrap_or_default()
    }

    /// Query dependencies for focus-like state changes.
    pub fn query_focus(&self) -> MoliDependencyQueryResult {
        self.focus_dependency.clone()
    }

    /// Query dependencies for :focus-within state changes.
    pub fn query_focus_within(&self) -> MoliDependencyQueryResult {
        self.focus_within_dependency.clone()
    }

    /// Query dependencies for :target state changes.
    pub fn query_target(&self) -> MoliDependencyQueryResult {
        self.target_dependency.clone()
    }

    #[cfg(test)]
    #[inline]
    fn has_unknown_dependency(&self) -> bool {
        self.unknown_dependency
    }

    /// Returns whether any known dependency can affect following siblings.
    #[inline]
    pub fn has_sibling_dependency(&self) -> bool {
        self.unknown_dependency
            || self.unknown_state_dependency_bits != 0
            || self
                .class_dependencies
                .iter()
                .chain(self.id_dependencies.iter())
                .map(|(_, result)| result)
                .chain(self.attribute_dependencies.iter().map(|(_, result)| result))
                .chain(
                    self.custom_state_dependencies
                        .iter()
                        .map(|(_, result)| result),
                )
                .chain(self.state_dependencies.iter().map(|(_, result)| result))
                .chain(std::iter::once(&self.focus_dependency))
                .chain(std::iter::once(&self.focus_within_dependency))
                .chain(std::iter::once(&self.target_dependency))
                .chain(std::iter::once(&self.universal_dependency))
                .any(MoliDependencyQueryResult::has_sibling_dependency)
            || self
                .type_dependencies
                .iter()
                .map(|(_, result)| result)
                .any(MoliDependencyQueryResult::has_sibling_dependency)
    }

    /// Returns whether any known dependency can affect relative selector
    /// anchors.
    #[inline]
    pub fn has_relative_selector_dependency(&self) -> bool {
        self.class_dependencies
            .iter()
            .chain(self.id_dependencies.iter())
            .map(|(_, result)| result)
            .chain(self.attribute_dependencies.iter().map(|(_, result)| result))
            .chain(
                self.custom_state_dependencies
                    .iter()
                    .map(|(_, result)| result),
            )
            .chain(self.state_dependencies.iter().map(|(_, result)| result))
            .chain(std::iter::once(&self.focus_dependency))
            .chain(std::iter::once(&self.focus_within_dependency))
            .chain(std::iter::once(&self.target_dependency))
            .chain(std::iter::once(&self.universal_dependency))
            .any(MoliDependencyQueryResult::has_relative_selector_dependency)
            || self
                .type_dependencies
                .iter()
                .map(|(_, result)| result)
                .any(MoliDependencyQueryResult::has_relative_selector_dependency)
    }
}

fn moli_nth_of_dependency_query_result() -> MoliDependencyQueryResult {
    let mut result = MoliDependencyQueryResult::default();
    result.add_kind(MoliDependencyKind::Siblings);
    result.add_fallback_reason(MoliDependencyFallbackReason::NthOfDependency);
    result
}

fn moli_note_keyed_dependency<K: Eq>(
    dependencies: &mut Vec<(K, MoliDependencyQueryResult)>,
    key: K,
    result: MoliDependencyQueryResult,
) {
    if !result.has_any_dependency() {
        return;
    }
    if let Some((_, existing)) = dependencies
        .iter_mut()
        .find(|(candidate, _)| candidate == &key)
    {
        existing.extend(result);
        return;
    }
    dependencies.push((key, result));
}

pub(crate) fn moli_dependency_summary_for_invalidation_map(
    map: &InvalidationMap,
) -> MoliDependencyInvalidationSummary {
    let mut summary = MoliDependencyInvalidationSummary::default();
    if map.unkeyed_sibling_dependency {
        summary.mark_unknown_dependency();
    }
    summary.note_universal_dependency(moli_dependency_query_result_for_dependencies(
        &map.any_to_selector,
    ));
    for (class, dependencies) in map.class_to_selector.iter() {
        summary.note_class_dependency(
            class.clone(),
            moli_dependency_query_result_for_dependencies(dependencies),
        );
    }
    for (id, dependencies) in map.id_to_selector.iter() {
        summary.note_id_dependency(
            id.clone(),
            moli_dependency_query_result_for_dependencies(dependencies),
        );
    }
    for (attribute, dependencies) in map.other_attribute_affecting_selectors.iter() {
        summary.note_attribute_dependency(
            attribute.clone(),
            moli_dependency_query_result_for_dependencies(dependencies),
        );
    }
    for (local_name, dependencies) in map.type_to_selector.iter() {
        summary.note_type_dependency(
            local_name.clone(),
            moli_dependency_query_result_for_dependencies(dependencies),
        );
    }
    for (state, dependencies) in map.custom_state_affecting_selectors.iter() {
        summary.note_custom_state_dependency(
            state.clone(),
            moli_dependency_query_result_for_dependencies(dependencies),
        );
    }
    moli_collect_state_dependencies_from_selector_map(
        &map.state_affecting_selectors,
        &mut summary,
    );
    summary
}

pub(crate) fn moli_dependency_summary_for_relative_invalidation_map(
    map: &AdditionalRelativeSelectorInvalidationMap,
) -> MoliDependencyInvalidationSummary {
    let mut summary = MoliDependencyInvalidationSummary::default();
    if map.needs_ancestors_traversal {
        summary.mark_unknown_dependency();
    }
    summary.note_universal_dependency(moli_dependency_query_result_for_dependencies(
        &map.any_to_selector,
    ));
    for (local_name, dependencies) in map.type_to_selector.iter() {
        summary.note_type_dependency(
            local_name.clone(),
            moli_dependency_query_result_for_dependencies(dependencies),
        );
    }
    if !map.ts_state_to_selector.is_empty() {
        summary.mark_unknown_dependency();
    }
    summary
}

fn moli_dependency_query_result_for_dependencies(
    dependencies: &[Dependency],
) -> MoliDependencyQueryResult {
    let mut result = MoliDependencyQueryResult::default();
    for dependency in dependencies {
        moli_collect_dependency_query_result(dependency, &mut result);
    }
    result
}

fn moli_collect_dependency_query_result(
    dependency: &Dependency,
    result: &mut MoliDependencyQueryResult,
) {
    match dependency.invalidation_kind() {
        DependencyInvalidationKind::FullSelector => {
            result.add_fallback_reason(MoliDependencyFallbackReason::FullSelector);
        },
        DependencyInvalidationKind::Normal(kind) => {
            result.add_kind(match kind {
                NormalDependencyInvalidationKind::Element => MoliDependencyKind::Element,
                NormalDependencyInvalidationKind::ElementAndDescendants => {
                    MoliDependencyKind::ElementAndDescendants
                },
                NormalDependencyInvalidationKind::Descendants => {
                    MoliDependencyKind::Descendants
                },
                NormalDependencyInvalidationKind::Siblings => MoliDependencyKind::Siblings,
                NormalDependencyInvalidationKind::SlottedElements => {
                    MoliDependencyKind::SlottedElements
                },
                NormalDependencyInvalidationKind::Parts => MoliDependencyKind::Parts,
            });
        },
        DependencyInvalidationKind::Relative(kind) => {
            result.add_kind(match kind {
                RelativeDependencyInvalidationKind::Ancestors => {
                    MoliDependencyKind::RelativeAncestors
                },
                RelativeDependencyInvalidationKind::Parent => {
                    MoliDependencyKind::RelativeParent
                },
                RelativeDependencyInvalidationKind::PrevSibling => {
                    MoliDependencyKind::RelativePrevSibling
                },
                RelativeDependencyInvalidationKind::AncestorPrevSibling => {
                    MoliDependencyKind::RelativeAncestorPrevSibling
                },
                RelativeDependencyInvalidationKind::EarlierSibling => {
                    MoliDependencyKind::RelativeEarlierSibling
                },
                RelativeDependencyInvalidationKind::AncestorEarlierSibling => {
                    MoliDependencyKind::RelativeAncestorEarlierSibling
                },
            });
        },
        DependencyInvalidationKind::Scope(_) => {
            result.add_kind(MoliDependencyKind::Scope);
        },
    }
    if dependency.right_combinator_is_next_sibling()
        || dependency.dependency_is_relative_with_single_next_sibling()
    {
        result.add_kind(MoliDependencyKind::Siblings);
    }
    if moli_dependency_has_nested_relative_dependency(dependency) {
        result.add_fallback_reason(
            MoliDependencyFallbackReason::NestedRelativeSelectorDependency,
        );
    }
    if let Some(next) = dependency.next.as_ref() {
        for dependency in next.slice() {
            moli_collect_dependency_query_result(dependency, result);
        }
    }
}

fn moli_dependency_has_nested_relative_dependency(dependency: &Dependency) -> bool {
    if matches!(
        dependency.invalidation_kind(),
        DependencyInvalidationKind::Relative(_)
    ) {
        return false;
    }
    dependency
        .next
        .as_ref()
        .is_some_and(|next| moli_dependency_chain_contains_relative_dependency(next.slice()))
}

fn moli_dependency_chain_contains_relative_dependency(dependencies: &[Dependency]) -> bool {
    dependencies.iter().any(|dependency| {
        matches!(
            dependency.invalidation_kind(),
            DependencyInvalidationKind::Relative(_)
        ) || dependency.next.as_ref().is_some_and(|next| {
            moli_dependency_chain_contains_relative_dependency(next.slice())
        })
    })
}

fn moli_collect_state_dependencies_from_selector_map(
    map: &SelectorMap<StateDependency>,
    summary: &mut MoliDependencyInvalidationSummary,
) {
    for dependency in &map.root {
        moli_collect_state_dependency(dependency, summary);
    }
    for (_, dependencies) in map.id_hash.iter() {
        for dependency in dependencies {
            moli_collect_state_dependency(dependency, summary);
        }
    }
    for (_, dependencies) in map.class_hash.iter() {
        for dependency in dependencies {
            moli_collect_state_dependency(dependency, summary);
        }
    }
    for (_, dependencies) in map.attribute_hash.iter() {
        for dependency in dependencies {
            moli_collect_state_dependency(dependency, summary);
        }
    }
    for (_, dependencies) in map.local_name_hash.iter() {
        for dependency in dependencies {
            moli_collect_state_dependency(dependency, summary);
        }
    }
    for (_, dependencies) in map.namespace_hash.iter() {
        for dependency in dependencies {
            moli_collect_state_dependency(dependency, summary);
        }
    }
    for dependency in &map.rare_pseudo_classes {
        moli_collect_state_dependency(dependency, summary);
    }
    for dependency in &map.other {
        moli_collect_state_dependency(dependency, summary);
    }
}

fn moli_collect_state_dependency(
    dependency: &StateDependency,
    summary: &mut MoliDependencyInvalidationSummary,
) {
    summary.note_state_dependency(
        dependency.state,
        moli_dependency_query_result_for_dependencies(std::slice::from_ref(&dependency.dep)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::QuirksMode;
    use crate::invalidation::element::invalidation_map::note_selector_for_invalidation;
    use crate::selector_parser::SelectorParser;
    use crate::stylesheets::UrlExtraData;
    use servo_arc::ThinArc;

    fn moli_dependency_summary_for_selector(
        selector: &str,
    ) -> MoliDependencyInvalidationSummary {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let selectors = SelectorParser::parse_author_origin_no_namespace(selector, &url_data)
            .expect("selector should parse");
        let mut map = InvalidationMap::new();
        let mut relative_map = InvalidationMap::new();
        let mut additional_relative_map = AdditionalRelativeSelectorInvalidationMap::new();

        for selector in selectors.slice() {
            note_selector_for_invalidation(
                selector,
                QuirksMode::NoQuirks,
                &mut map,
                &mut relative_map,
                &mut additional_relative_map,
                None,
                None,
            )
            .expect("selector invalidation dependencies should collect");
        }

        let mut summary = moli_dependency_summary_for_invalidation_map(&map);
        summary.extend(moli_dependency_summary_for_invalidation_map(
            &relative_map,
        ));
        summary.extend(moli_dependency_summary_for_relative_invalidation_map(
            &additional_relative_map,
        ));
        summary
    }

    fn moli_structural_boundary_summary_for_type(
        local_name: &str,
    ) -> MoliChildListStructuralBoundaryDependencySummary {
        let mut summary = MoliChildListStructuralBoundaryDependencySummary::default();
        summary.note_type_dependency(LocalName::from(local_name));
        summary
    }

    fn moli_structural_boundary_summary_for_class(
        class: &str,
    ) -> MoliChildListStructuralBoundaryDependencySummary {
        let mut summary = MoliChildListStructuralBoundaryDependencySummary::default();
        summary.note_class_dependency(Atom::from(class));
        summary
    }

    fn moli_universal_structural_boundary_summary(
    ) -> MoliChildListStructuralBoundaryDependencySummary {
        let mut summary = MoliChildListStructuralBoundaryDependencySummary::default();
        summary.note_universal_dependency();
        summary
    }

    fn parse_moli_servo_selector(selector: &str) {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        SelectorParser::parse_author_origin_no_namespace(selector, &url_data)
            .unwrap_or_else(|error| panic!("selector should parse: {selector}: {error:?}"));
    }

    #[test]
    fn moli_servo_parser_accepts_migrated_selector_capabilities() {
        for selector in [
            "x-host::part(label):lang(en)",
            "x-host::part(label):dir(ltr)",
            "video:playing",
            "video:paused",
            "video:seeking",
            "video:muted",
            ":heading",
            ":heading(1, 2, 6)",
            "input:in-range",
            "input:out-of-range",
            "li:nth-child(odd of :not(.current))",
            "li:nth-last-child(2n+1 of .item)",
        ] {
            parse_moli_servo_selector(selector);
        }
    }

    #[test]
    fn moli_dependency_summary_collects_migrated_state_pseudos() {
        let summary = moli_dependency_summary_for_selector(
            "video:playing, video:paused, video:seeking, video:muted, \
             :heading, :heading(1, 2, 6), input:in-range, input:out-of-range",
        );

        for state in [
            ElementState::PAUSED,
            ElementState::SEEKING,
            ElementState::MUTED,
            ElementState::HEADING_LEVEL_BITS,
            ElementState::INRANGE,
            ElementState::OUTOFRANGE,
        ] {
            let result = summary.query_state(state);
            assert!(
                result.has_any_dependency(),
                "missing dependency for {state:?}"
            );
            assert!(
                result.fallback_reasons().is_empty(),
                "state dependency should not require fallback for {state:?}: {:?}",
                result.fallback_reasons()
            );
        }
    }

    #[test]
    fn moli_dependency_summary_collects_lang_and_dir_attribute_pseudos() {
        let summary = moli_dependency_summary_for_selector(
            "x-host::part(label):lang(fr), x-host::part(label):dir(rtl)",
        );

        for attribute in ["lang", "dir"] {
            let result = summary.query_attribute(&LocalName::from(attribute));
            assert!(
                result.has_any_dependency(),
                "missing dependency for {attribute}"
            );
            assert!(
                result.fallback_reasons().is_empty(),
                "attribute dependency should not require fallback for {attribute}: {:?}",
                result.fallback_reasons()
            );
        }
    }

    #[test]
    fn moli_dependency_query_result_keeps_fallback_reasons_out_of_kinds() {
        let mut result = MoliDependencyQueryResult::default();
        result.add_fallback_reason(MoliDependencyFallbackReason::FullSelector);

        assert!(result.has_any_dependency());
        assert!(result.requires_fallback());
        assert_eq!(
            result.fallback_reasons(),
            &[MoliDependencyFallbackReason::FullSelector]
        );
        assert_eq!(
            result.fallback_root_policy(),
            MoliDependencyFallbackRootPolicy::SourceFallback
        );
        assert!(result.kinds().is_empty());
    }

    #[test]
    fn moli_dependency_query_result_exposes_source_fallback_handling() {
        let mut nth_of = MoliDependencyQueryResult::default();
        nth_of.add_fallback_reason(MoliDependencyFallbackReason::NthOfDependency);
        let MoliDependencyFallbackHandling::ContextRoots(reasons) =
            nth_of.source_dependency_fallback_handling()
        else {
            panic!("nth-of dependency should use context fallback roots");
        };
        assert!(reasons.contains(&MoliSourceInvalidationFallbackReason::NthOfDependency));
        assert!(!nth_of.tries_retained_query_before_context_fallback());

        let mut nested_relative = MoliDependencyQueryResult::default();
        nested_relative.add_fallback_reason(
            MoliDependencyFallbackReason::NestedRelativeSelectorDependency,
        );
        assert!(nested_relative.tries_retained_query_before_context_fallback());
        nested_relative.add_fallback_reason(MoliDependencyFallbackReason::NthOfDependency);
        assert!(!nested_relative.tries_retained_query_before_context_fallback());

        let mut scope = MoliDependencyQueryResult::default();
        scope.add_kind(MoliDependencyKind::Scope);
        let MoliDependencyFallbackHandling::SourceFallback(reasons) =
            scope.source_dependency_fallback_handling()
        else {
            panic!("scope dependency should require source fallback roots");
        };
        assert!(reasons.contains(&MoliSourceInvalidationFallbackReason::ScopeDependency));
    }

    #[test]
    fn moli_dependency_query_result_dedupes_extended_fallback_reasons() {
        let mut first = MoliDependencyQueryResult::default();
        first.add_fallback_reason(MoliDependencyFallbackReason::UnknownDependency);
        let mut second = MoliDependencyQueryResult::default();
        second.add_fallback_reason(MoliDependencyFallbackReason::UnknownDependency);
        second.add_kind(MoliDependencyKind::Siblings);

        first.extend(second);

        assert!(first.has_any_dependency());
        assert!(first.requires_fallback());
        assert_eq!(
            first.fallback_reasons(),
            &[MoliDependencyFallbackReason::UnknownDependency]
        );
        assert_eq!(first.kinds(), &[MoliDependencyKind::Siblings]);
        assert!(first.has_sibling_dependency());
    }

    #[test]
    fn moli_dependency_query_result_derives_shape_fallback_reasons() {
        let mut scope = MoliDependencyQueryResult::default();
        scope.add_kind(MoliDependencyKind::Scope);
        assert_eq!(
            scope.fallback_or_shape_reasons(),
            vec![MoliDependencyFallbackReason::ScopeDependency]
        );

        let mut sibling = MoliDependencyQueryResult::default();
        sibling.add_kind(MoliDependencyKind::Siblings);
        assert_eq!(
            sibling.fallback_or_shape_reasons(),
            vec![MoliDependencyFallbackReason::UnsupportedDependency]
        );
    }

    #[test]
    fn moli_dependency_query_result_exposes_relative_shape_predicates() {
        let mut direct_previous = MoliDependencyQueryResult::default();
        direct_previous.add_kind(MoliDependencyKind::RelativePrevSibling);
        direct_previous.add_kind(MoliDependencyKind::Siblings);
        assert!(direct_previous.has_relative_selector_dependency());
        assert!(direct_previous.has_relative_previous_sibling_dependency());
        assert!(direct_previous.has_only_direct_relative_previous_sibling_dependency());

        let mut ancestor_previous = MoliDependencyQueryResult::default();
        ancestor_previous.add_kind(MoliDependencyKind::RelativeAncestorPrevSibling);
        assert!(ancestor_previous.has_relative_selector_dependency());
        assert!(ancestor_previous.has_relative_previous_sibling_dependency());
        assert!(!ancestor_previous.has_only_direct_relative_previous_sibling_dependency());

        let mut ancestor = MoliDependencyQueryResult::default();
        ancestor.add_kind(MoliDependencyKind::RelativeAncestors);
        assert!(ancestor.has_relative_selector_dependency());
        assert!(!ancestor.has_relative_previous_sibling_dependency());
        assert!(!ancestor.has_only_direct_relative_previous_sibling_dependency());
    }

    #[test]
    fn moli_dependency_query_result_exposes_context_root_flags() {
        #[derive(Default)]
        struct Sink {
            calls: Vec<&'static str>,
        }

        impl MoliDependencyContextRootFlagsSink for Sink {
            fn require_context_source_fallback(&mut self) {
                self.calls.push("source_fallback");
            }

            fn include_context_local_subtree(&mut self) {
                self.calls.push("local_subtree");
            }

            fn include_context_ancestor_chain(&mut self) {
                self.calls.push("ancestor_chain");
            }

            fn include_context_following_siblings(&mut self) {
                self.calls.push("following_siblings");
            }

            fn include_context_ancestor_following_siblings(&mut self) {
                self.calls.push("ancestor_following_siblings");
            }

            fn include_context_previous_sibling(&mut self) {
                self.calls.push("previous_sibling");
            }

            fn include_context_earlier_siblings(&mut self) {
                self.calls.push("earlier_siblings");
            }

            fn include_context_ancestor_previous_siblings(&mut self) {
                self.calls.push("ancestor_previous_siblings");
            }

            fn include_context_slotted_elements(&mut self) {
                self.calls.push("slotted_elements");
            }

            fn include_context_parts(&mut self) {
                self.calls.push("parts");
            }
        }

        let mut query = MoliDependencyQueryResult::default();
        query.add_kind(MoliDependencyKind::ElementAndDescendants);
        query.add_kind(MoliDependencyKind::Siblings);
        query.add_kind(MoliDependencyKind::SlottedElements);
        query.add_kind(MoliDependencyKind::Parts);
        query.add_kind(MoliDependencyKind::RelativePrevSibling);
        query.add_kind(MoliDependencyKind::RelativeAncestorEarlierSibling);
        let flags = query.context_root_flags();
        let mut sink = Sink::default();
        flags.drain_into(false, &mut sink);

        assert_eq!(
            sink.calls,
            vec![
                "local_subtree",
                "ancestor_chain",
                "ancestor_following_siblings",
                "previous_sibling",
                "ancestor_previous_siblings",
                "slotted_elements",
                "parts"
            ]
        );

        query.add_kind(MoliDependencyKind::Scope);
        let mut sink = Sink::default();
        query.context_root_flags().drain_into(false, &mut sink);
        assert!(sink.calls.contains(&"source_fallback"));
    }

    #[test]
    fn moli_dependency_invalidation_context_roots_drains_query_into_typed_roots() {
        #[derive(Default)]
        struct Sink {
            requires_source_fallback: bool,
            roots: Vec<u32>,
        }

        impl MoliDependencyContextRootFlagsSink for Sink {
            fn require_context_source_fallback(&mut self) {
                self.requires_source_fallback = true;
            }

            fn include_context_local_subtree(&mut self) {
                self.roots.push(1);
            }

            fn include_context_ancestor_chain(&mut self) {
                self.roots.push(2);
            }

            fn include_context_following_siblings(&mut self) {
                self.roots.push(3);
            }

            fn include_context_ancestor_following_siblings(&mut self) {
                self.roots.push(4);
            }

            fn include_context_previous_sibling(&mut self) {
                self.roots.push(5);
            }

            fn include_context_earlier_siblings(&mut self) {
                self.roots.push(6);
            }

            fn include_context_ancestor_previous_siblings(&mut self) {
                self.roots.push(7);
            }

            fn include_context_slotted_elements(&mut self) {
                self.roots.push(8);
            }

            fn include_context_parts(&mut self) {
                self.roots.push(9);
            }
        }

        impl MoliDependencyInvalidationContextRootsSink<u32> for Sink {
            fn drain_collected_context_roots_into(
                self,
                target: &mut impl MoliDependencyInvalidationContextRootsPartsSink<u32>,
            ) {
                if self.requires_source_fallback {
                    target.record_context_source_fallback();
                }
                target.extend_context_roots(self.roots);
            }
        }

        let mut query = MoliDependencyQueryResult::default();
        query.add_kind(MoliDependencyKind::Element);
        query.add_kind(MoliDependencyKind::Siblings);
        query.add_kind(MoliDependencyKind::Scope);

        let roots = moli_dependency_invalidation_context_roots(&query, true, Sink::default());

        assert!(roots.requires_source_fallback());
        assert_eq!(roots.roots(), &[1, 3]);
    }

    #[test]
    fn moli_dependency_query_result_exposes_structural_context_cleanup_policy() {
        let mut query = MoliDependencyQueryResult::default();
        query.add_kind(MoliDependencyKind::RelativePrevSibling);
        assert!(query.requires_structural_context_fallback_cleanup(true, true));
        assert!(!query.requires_structural_context_fallback_cleanup(false, true));
        assert!(!query.requires_structural_context_fallback_cleanup(true, false));

        let mut non_relative = MoliDependencyQueryResult::default();
        non_relative.add_kind(MoliDependencyKind::Element);
        assert!(!non_relative.requires_structural_context_fallback_cleanup(true, true));
    }

    #[test]
    fn moli_dependency_fallback_reason_maps_raw_dependency_kind() {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let selector = SelectorParser::parse_author_origin_no_namespace(".subject", &url_data)
            .expect("selector should parse")
            .slice()[0]
            .clone();
        let dependency_for_kind = |kind| Dependency::new(selector.clone(), 0, None, kind);

        assert_eq!(
            moli_dependency_fallback_reason_for_dependency(&dependency_for_kind(
                DependencyInvalidationKind::FullSelector
            )),
            MoliDependencyFallbackReason::FullSelector
        );
        assert_eq!(
            moli_dependency_fallback_reason_for_dependency(&dependency_for_kind(
                DependencyInvalidationKind::Relative(RelativeDependencyInvalidationKind::Ancestors)
            )),
            MoliDependencyFallbackReason::RelativeAnySelector
        );
        assert_eq!(
            moli_relative_selector_invalidation_fallback_reason(
                RelativeDependencyInvalidationKind::Ancestors,
                &dependency_for_kind(DependencyInvalidationKind::Relative(
                    RelativeDependencyInvalidationKind::Ancestors
                ))
            ),
            MoliSourceInvalidationFallbackReason::RelativeAnySelector
        );
        assert_eq!(
            moli_dependency_fallback_reason_for_dependency(&dependency_for_kind(
                DependencyInvalidationKind::Scope(ScopeDependencyInvalidationKind::ScopeEnd)
            )),
            MoliDependencyFallbackReason::ScopeDependency
        );
        assert_eq!(
            moli_dependency_fallback_reason_for_dependency(&dependency_for_kind(
                DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Element)
            )),
            MoliDependencyFallbackReason::UnsupportedDependency
        );
    }

    #[test]
    fn moli_dependency_invalidation_action_maps_raw_dependency_kind() {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let selector = SelectorParser::parse_author_origin_no_namespace(".subject", &url_data)
            .expect("selector should parse")
            .slice()[0]
            .clone();
        let dependency_for_kind = |kind| Dependency::new(selector.clone(), 0, None, kind);

        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Element)
            )),
            MoliDependencyInvalidationAction::Element
        );
        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Siblings)
            )),
            MoliDependencyInvalidationAction::Siblings
        );
        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::FullSelector
            )),
            MoliDependencyInvalidationAction::Fallback(
                MoliSourceInvalidationFallbackReason::FullSelector
            )
        );
        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::Relative(RelativeDependencyInvalidationKind::Ancestors)
            )),
            MoliDependencyInvalidationAction::Fallback(
                MoliSourceInvalidationFallbackReason::RelativeAnySelector
            )
        );
        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::Scope(ScopeDependencyInvalidationKind::ImplicitScope)
            )),
            MoliDependencyInvalidationAction::Scope(
                MoliScopeDependencyInvalidationAction::ImplicitScope
            )
        );
        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::Scope(ScopeDependencyInvalidationKind::ScopeEnd)
            )),
            MoliDependencyInvalidationAction::Scope(
                MoliScopeDependencyInvalidationAction::ForceAtSubject { force_add: false }
            )
        );
        assert_eq!(
            moli_dependency_invalidation_action(&dependency_for_kind(
                DependencyInvalidationKind::Scope(ScopeDependencyInvalidationKind::ExplicitScope)
            )),
            MoliDependencyInvalidationAction::Scope(
                MoliScopeDependencyInvalidationAction::CheckNextInScope
            )
        );
    }

    #[test]
    fn moli_dependency_invalidation_action_drains_into_sink() {
        #[derive(Default)]
        struct Sink {
            calls: Vec<&'static str>,
            fallback_reason: Option<MoliSourceInvalidationFallbackReason>,
            scope_action: Option<MoliScopeDependencyInvalidationAction>,
        }

        impl MoliDependencyInvalidationActionSink for Sink {
            fn invalidate_element(&mut self) {
                self.calls.push("element");
            }

            fn invalidate_element_and_descendants(&mut self) {
                self.calls.push("element_and_descendants");
            }

            fn invalidate_descendants(&mut self) {
                self.calls.push("descendants");
            }

            fn invalidate_siblings(&mut self) {
                self.calls.push("siblings");
            }

            fn invalidate_slotted_elements(&mut self) {
                self.calls.push("slotted");
            }

            fn invalidate_parts(&mut self) {
                self.calls.push("parts");
            }

            fn invalidate_fallback(&mut self, reason: MoliSourceInvalidationFallbackReason) {
                self.fallback_reason = Some(reason);
            }

            fn invalidate_scope(&mut self, action: MoliScopeDependencyInvalidationAction) {
                self.scope_action = Some(action);
            }
        }

        let mut sink = Sink::default();
        MoliDependencyInvalidationAction::ElementAndDescendants.drain_into(&mut sink);
        MoliDependencyInvalidationAction::Fallback(
            MoliSourceInvalidationFallbackReason::FullSelector,
        )
        .drain_into(&mut sink);
        MoliDependencyInvalidationAction::Scope(
            MoliScopeDependencyInvalidationAction::CheckNextInScope,
        )
        .drain_into(&mut sink);

        assert_eq!(sink.calls, vec!["element_and_descendants"]);
        assert_eq!(
            sink.fallback_reason,
            Some(MoliSourceInvalidationFallbackReason::FullSelector)
        );
        assert_eq!(
            sink.scope_action,
            Some(MoliScopeDependencyInvalidationAction::CheckNextInScope)
        );
    }

    #[test]
    fn moli_scope_dependency_invalidation_action_drains_into_sink() {
        #[derive(Default)]
        struct Sink {
            calls: Vec<&'static str>,
            force_add: Option<bool>,
        }

        impl MoliScopeDependencyInvalidationActionSink for Sink {
            fn invalidate_implicit_scope(&mut self) {
                self.calls.push("implicit_scope");
            }

            fn invalidate_scope_force_at_subject(&mut self, force_add: bool) {
                self.calls.push("force_at_subject");
                self.force_add = Some(force_add);
            }

            fn invalidate_scope_check_next(&mut self) {
                self.calls.push("check_next");
            }

            fn invalidate_scope_by_combinator(&mut self) {
                self.calls.push("by_combinator");
            }
        }

        let mut sink = Sink::default();
        MoliScopeDependencyInvalidationAction::ImplicitScope.drain_into(&mut sink);
        MoliScopeDependencyInvalidationAction::ForceAtSubject { force_add: true }
            .drain_into(&mut sink);
        MoliScopeDependencyInvalidationAction::CheckNextInScope.drain_into(&mut sink);
        MoliScopeDependencyInvalidationAction::PushByCombinator.drain_into(&mut sink);

        assert_eq!(
            sink.calls,
            vec![
                "implicit_scope",
                "force_at_subject",
                "check_next",
                "by_combinator"
            ]
        );
        assert_eq!(sink.force_add, Some(true));
    }

    #[test]
    fn moli_relative_dependency_invalidation_action_drains_into_sink() {
        #[derive(Default)]
        struct Sink {
            calls: Vec<&'static str>,
        }

        impl MoliRelativeDependencyInvalidationActionSink for Sink {
            fn visit_relative_ancestor_candidates(&mut self) {
                self.calls.push("ancestors");
            }

            fn visit_relative_parent_candidate(&mut self) {
                self.calls.push("parent");
            }

            fn visit_relative_previous_sibling_candidate(&mut self) {
                self.calls.push("prev_sibling");
            }

            fn visit_relative_earlier_sibling_candidates(&mut self) {
                self.calls.push("earlier_sibling");
            }

            fn visit_relative_ancestor_previous_sibling_candidates(&mut self) {
                self.calls.push("ancestor_prev_sibling");
            }

            fn visit_relative_ancestor_earlier_sibling_candidates(&mut self) {
                self.calls.push("ancestor_earlier_sibling");
            }
        }

        let mut sink = Sink::default();
        MoliRelativeDependencyInvalidationAction::Ancestors.drain_into(&mut sink);
        MoliRelativeDependencyInvalidationAction::Parent.drain_into(&mut sink);
        MoliRelativeDependencyInvalidationAction::PrevSibling.drain_into(&mut sink);
        MoliRelativeDependencyInvalidationAction::EarlierSibling.drain_into(&mut sink);
        MoliRelativeDependencyInvalidationAction::AncestorPrevSibling.drain_into(&mut sink);
        MoliRelativeDependencyInvalidationAction::AncestorEarlierSibling
            .drain_into(&mut sink);

        assert_eq!(
            sink.calls,
            vec![
                "ancestors",
                "parent",
                "prev_sibling",
                "earlier_sibling",
                "ancestor_prev_sibling",
                "ancestor_earlier_sibling"
            ]
        );
    }

    #[test]
    fn moli_relative_dependency_helpers_map_raw_dependency_kind() {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let selector = SelectorParser::parse_author_origin_no_namespace(".subject", &url_data)
            .expect("selector should parse")
            .slice()[0]
            .clone();
        let dependency_for_kind = |kind| Dependency::new(selector.clone(), 0, None, kind);

        let relative = dependency_for_kind(DependencyInvalidationKind::Relative(
            RelativeDependencyInvalidationKind::AncestorEarlierSibling,
        ));
        let normal = dependency_for_kind(DependencyInvalidationKind::Normal(
            NormalDependencyInvalidationKind::Descendants,
        ));
        let full = dependency_for_kind(DependencyInvalidationKind::FullSelector);

        assert_eq!(
            moli_relative_dependency_invalidation_action(&relative),
            Some(MoliRelativeDependencyInvalidationAction::AncestorEarlierSibling)
        );
        assert!(moli_dependency_is_relative_selector(&relative));
        assert!(!moli_dependency_is_relative_selector(&normal));
        assert!(moli_snapshot_relative_outer_dependency_supported(
            &normal
        ));
        assert!(!moli_snapshot_relative_outer_dependency_supported(
            &relative
        ));
        assert!(!moli_snapshot_relative_outer_dependency_supported(
            &full
        ));
    }

    #[test]
    fn moli_source_fallback_reason_preserves_dependency_detail() {
        let cases = [
            (
                MoliDependencyFallbackReason::UnknownDependency,
                MoliSourceInvalidationFallbackReason::UnknownDependency,
            ),
            (
                MoliDependencyFallbackReason::FullSelector,
                MoliSourceInvalidationFallbackReason::FullSelector,
            ),
            (
                MoliDependencyFallbackReason::RelativeAnySelector,
                MoliSourceInvalidationFallbackReason::RelativeAnySelector,
            ),
            (
                MoliDependencyFallbackReason::ScopeDependency,
                MoliSourceInvalidationFallbackReason::ScopeDependency,
            ),
            (
                MoliDependencyFallbackReason::UnsupportedStateDependency,
                MoliSourceInvalidationFallbackReason::UnsupportedStateDependency,
            ),
            (
                MoliDependencyFallbackReason::UnsupportedDependency,
                MoliSourceInvalidationFallbackReason::UnsupportedDependency,
            ),
            (
                MoliDependencyFallbackReason::NthOfDependency,
                MoliSourceInvalidationFallbackReason::NthOfDependency,
            ),
            (
                MoliDependencyFallbackReason::NestedRelativeSelectorDependency,
                MoliSourceInvalidationFallbackReason::NestedRelativeSelectorDependency,
            ),
        ];

        for (dependency_reason, source_reason) in cases {
            assert_eq!(
                MoliSourceInvalidationFallbackReason::from(dependency_reason),
                source_reason
            );
        }
    }

    #[test]
    fn moli_attribute_and_state_runtime_policy_is_fork_owned() {
        assert!(moli_attribute_change_can_use_retained_invalidator(
            "class", false
        ));
        assert!(moli_attribute_change_can_use_retained_invalidator(
            "style", false
        ));
        assert!(!moli_attribute_change_can_use_retained_invalidator(
            "width", true
        ));

        assert!(moli_attribute_change_can_skip_fallback_without_dependency("class"));
        assert!(moli_attribute_change_can_skip_fallback_without_dependency("data-state"));
        assert!(moli_attribute_change_can_skip_fallback_without_dependency("aria-expanded"));
        assert!(moli_attribute_change_can_skip_fallback_without_dependency("lang"));
        assert!(moli_attribute_change_can_skip_fallback_without_dependency("dir"));
        assert!(!moli_attribute_change_can_skip_fallback_without_dependency("DATA-State"));
        assert!(!moli_attribute_change_can_skip_fallback_without_dependency("href"));

        for state in [
            ElementState::CHECKED,
            ElementState::INDETERMINATE,
            ElementState::PLACEHOLDER_SHOWN,
            ElementState::DEFINED,
            ElementState::PAUSED,
            ElementState::MUTED,
            ElementState::SEEKING,
        ] {
            assert!(moli_state_change_can_use_retained_invalidator(
                state, None
            ));
            assert_eq!(
                moli_source_fallback_reason_for_unretained_state_change(state, None),
                None
            );
        }

        assert!(!moli_state_change_can_use_retained_invalidator(
            ElementState::HOVER,
            None
        ));
        assert_eq!(
            moli_source_fallback_reason_for_unretained_state_change(
                ElementState::HOVER,
                None
            ),
            Some(MoliSourceInvalidationFallbackReason::UnsupportedStateDependency)
        );
        assert!(moli_state_change_can_use_retained_invalidator(
            ElementState::HOVER,
            Some(ElementState::empty())
        ));
    }

    #[test]
    fn moli_runtime_fallback_roots_for_mutation_inputs_are_fork_planned() {
        struct Resolver;

        impl MoliRuntimeFallbackRootResolver<u32> for Resolver {
            fn unknown_slot_assignment_fallback_root(&self, slot: u32) -> u32 {
                slot + 100
            }
        }

        let added_nodes = [3, 5];
        let roots = moli_runtime_fallback_roots_for_mutation_inputs(
            [
                MoliRuntimeFallbackRootInput::Attribute {
                    element: 1,
                    attribute_name: "class",
                    has_dependency_change: true,
                    has_non_css_runtime_side_effect: false,
                },
                MoliRuntimeFallbackRootInput::Attribute {
                    element: 2,
                    attribute_name: "width",
                    has_dependency_change: true,
                    has_non_css_runtime_side_effect: true,
                },
                MoliRuntimeFallbackRootInput::ChildList {
                    added_nodes: &added_nodes,
                },
                MoliRuntimeFallbackRootInput::SlotAssignment {
                    slot: 4,
                    has_assignment_snapshot: false,
                },
                MoliRuntimeFallbackRootInput::ConnectedSubtree { root: 2 },
                MoliRuntimeFallbackRootInput::OtherMutation,
            ],
            &Resolver,
        );

        assert_eq!(roots, vec![2, 3, 5, 104]);

        let child_list_only = moli_runtime_fallback_roots_for_mutation_inputs(
            [MoliRuntimeFallbackRootInput::ChildList {
                added_nodes: &added_nodes,
            }],
            &Resolver,
        );
        assert!(child_list_only.is_empty());

        let known_slot = moli_runtime_fallback_roots_for_mutation_inputs(
            [MoliRuntimeFallbackRootInput::SlotAssignment {
                slot: 4,
                has_assignment_snapshot: true,
            }],
            &Resolver,
        );
        assert!(known_slot.is_empty());
    }

    #[test]
    fn moli_retained_source_invalidation_kind_exposes_result_policy() {
        use MoliRetainedSourceStyleInvalidationKind::{
            ContextFallback, FallbackOnly, MissingFallbackRoots, RetainedQueries,
            SourceScopeFallback,
        };

        assert_eq!(
            ContextFallback.merged_with(ContextFallback),
            ContextFallback
        );
        assert_eq!(
            moli_merge_retained_source_invalidation_kind(ContextFallback, ContextFallback),
            ContextFallback
        );
        assert_eq!(ContextFallback.merged_with(FallbackOnly), FallbackOnly);
        assert_eq!(
            moli_merge_retained_source_invalidation_fallback_kind(
                Some(ContextFallback),
                Some(FallbackOnly),
            ),
            Some(FallbackOnly)
        );
        assert_eq!(
            moli_merge_retained_source_invalidation_fallback_kind(None, Some(FallbackOnly)),
            Some(FallbackOnly)
        );
        assert_eq!(
            moli_merge_retained_source_invalidation_fallback_kind(
                Some(ContextFallback),
                None,
            ),
            Some(ContextFallback)
        );
        assert_eq!(
            FallbackOnly.merged_with(SourceScopeFallback),
            SourceScopeFallback
        );
        assert_eq!(
            SourceScopeFallback.merged_with(MissingFallbackRoots),
            MissingFallbackRoots
        );
        assert_eq!(
            MissingFallbackRoots.merged_with(RetainedQueries),
            RetainedQueries
        );
        assert!(RetainedQueries.carries_retained_queries());
        assert!(!FallbackOnly.carries_retained_queries());
        assert!(FallbackOnly.can_target_fallback_root());
        assert!(SourceScopeFallback.can_target_fallback_root());
        assert!(!ContextFallback.can_target_fallback_root());

        assert_eq!(
            ContextFallback.fallback_source_result_kind(true),
            MoliSourceStyleInvalidationSourceResultKind::ContextFallback
        );
        assert_eq!(
            FallbackOnly.fallback_source_result_kind(false),
            MoliSourceStyleInvalidationSourceResultKind::FallbackOnly
        );
        assert_eq!(
            FallbackOnly.fallback_source_result_kind(true),
            MoliSourceStyleInvalidationSourceResultKind::Fallback
        );
        assert_eq!(
            MoliSourceFallbackRootAvailability::for_root_count(0),
            None
        );
        assert_eq!(
            MoliSourceFallbackRootAvailability::for_root_count(2),
            Some(MoliSourceFallbackRootAvailability::Available { root_count: 2 })
        );
        assert_eq!(FallbackOnly.fallback_root_availability(0), None);
        assert_eq!(
            FallbackOnly.fallback_root_availability(2),
            Some(MoliSourceFallbackRootAvailability::Available { root_count: 2 })
        );
        assert_eq!(
            MissingFallbackRoots.fallback_root_availability(0),
            Some(MoliSourceFallbackRootAvailability::Missing)
        );
        assert_eq!(
            MissingFallbackRoots.fallback_root_availability(2),
            Some(MoliSourceFallbackRootAvailability::Missing)
        );
        assert_eq!(
            SourceScopeFallback.fallback_reason(),
            Some(MoliSourceInvalidationFallbackReason::SourceScopeFallback)
        );
        assert_eq!(
            MissingFallbackRoots.fallback_reason(),
            Some(MoliSourceInvalidationFallbackReason::MissingFallbackRoots)
        );
        assert_eq!(FallbackOnly.fallback_reason(), None);
    }

    #[test]
    fn moli_retained_style_query_maps_to_stylo_query_shape() {
        let traversal = MoliRetainedStyleSiblingTraversal::new(Some(1_u32), Some(3_u32));
        let class_query = MoliRetainedStyleInvalidationQuery::class(2_u32, "active".into())
            .with_sibling_traversal(Some(traversal));

        assert_eq!(class_query.root(), 2);
        assert_eq!(class_query.sibling_traversal(), Some(traversal));
        assert!(!class_query.is_universal());
        assert!(!class_query.allows_direct_previous_following_sibling_fallback());
        assert_eq!(
            class_query.as_stylo_query(),
            MoliStyleInvalidationQuery::Class("active")
        );
        let source_query = class_query.as_source_query();
        assert_eq!(source_query.root(), 2);
        assert_eq!(
            source_query.query(),
            MoliStyleInvalidationQuery::Class("active")
        );
        assert_eq!(source_query.previous_sibling(), Some(1));
        assert_eq!(source_query.next_sibling(), Some(3));
        assert_eq!(traversal.previous_sibling(), Some(1));
        assert_eq!(traversal.next_sibling(), Some(3));

        let universal_query = MoliRetainedStyleInvalidationQuery::universal(7_u32);
        assert!(universal_query.is_universal());
        assert_eq!(
            universal_query.as_stylo_query(),
            MoliStyleInvalidationQuery::Universal
        );
        assert_eq!(universal_query.as_source_query().previous_sibling(), None);
        assert_eq!(universal_query.as_source_query().next_sibling(), None);

        let heading_query = MoliRetainedStyleInvalidationQuery::state(
            9_u32,
            ElementState::HEADING_LEVEL_BITS,
        );
        assert!(heading_query.allows_direct_previous_following_sibling_fallback());
        assert_eq!(
            heading_query.as_stylo_query(),
            MoliStyleInvalidationQuery::State(ElementState::HEADING_LEVEL_BITS)
        );
    }

    #[test]
    fn moli_element_dependency_snapshot_builds_retained_queries() {
        let traversal = MoliRetainedStyleSiblingTraversal::new(Some(1_u32), Some(3_u32));
        let snapshot = MoliElementDependencySnapshot::new(
            2_u32,
            "article".into(),
            ElementState::CHECKED,
            vec!["class".into(), "data-state".into()],
            vec!["active".into()],
            vec!["expanded".into()],
            Some("main".into()),
        );

        let queries =
            moli_retained_queries_for_element_dependency_snapshot(&snapshot, Some(traversal));
        let query_shapes = queries
            .iter()
            .map(|query| query.as_stylo_query())
            .collect::<Vec<_>>();
        assert_eq!(
            query_shapes,
            vec![
                MoliStyleInvalidationQuery::Universal,
                MoliStyleInvalidationQuery::Type("article"),
                MoliStyleInvalidationQuery::State(ElementState::CHECKED),
                MoliStyleInvalidationQuery::Attribute("class"),
                MoliStyleInvalidationQuery::Attribute("data-state"),
                MoliStyleInvalidationQuery::Class("active"),
                MoliStyleInvalidationQuery::Id("main"),
                MoliStyleInvalidationQuery::CustomState("expanded"),
            ]
        );
        assert!(queries
            .iter()
            .all(|query| query.sibling_traversal() == Some(traversal)));
        assert_eq!(snapshot.handle(), 2);
        assert_eq!(snapshot.class_tokens(), &["active".to_string()]);

        let non_universal =
            moli_retained_non_universal_queries_for_element_dependency_snapshot(
                &snapshot, None,
            );
        assert!(non_universal.iter().all(|query| !query.is_universal()));
        assert_eq!(
            non_universal[0].as_stylo_query(),
            MoliStyleInvalidationQuery::Type("article")
        );
        assert!(non_universal
            .iter()
            .all(|query| query.sibling_traversal().is_none()));
    }

    #[test]
    fn moli_retained_source_invalidation_input_selects_typed_variant() {
        #[derive(Default)]
        struct Sink {
            retained_fallback_kind: Option<Option<MoliRetainedSourceStyleInvalidationKind>>,
            retained_shadow_root: Option<u32>,
            retained_query_count: usize,
            retained_zero_matched_dependency_result_is_exact: Option<bool>,
            retained_reasoned_roots: Vec<u32>,
            retained_exact_safety_roots: Vec<u32>,
            retained_fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
            retained_snapshot: Option<u8>,
            fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
            fallback_roots: Vec<u32>,
            fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        }

        impl<'a> MoliRetainedSourceStyleInvalidationSink<'a, u32, u8> for Sink {
            fn run_retained_source_style_invalidation_queries(
                &mut self,
                fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
                cascade_data: Option<&'a ServoArc<CascadeData>>,
                shadow_root: Option<u32>,
                queries: &'a IndexSet<MoliRetainedStyleInvalidationQuery<u32>>,
                zero_matched_dependency_result_is_exact: bool,
                reasoned_fallback_roots: &'a IndexSet<u32>,
                exact_safety_fallback_roots: &'a IndexSet<u32>,
                fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
                mutation_snapshot: &'a u8,
            ) {
                assert!(cascade_data.is_none());
                self.retained_fallback_kind = Some(fallback_kind);
                self.retained_shadow_root = shadow_root;
                self.retained_query_count = queries.len();
                self.retained_zero_matched_dependency_result_is_exact =
                    Some(zero_matched_dependency_result_is_exact);
                self.retained_reasoned_roots
                    .extend(reasoned_fallback_roots.iter().copied());
                self.retained_exact_safety_roots
                    .extend(exact_safety_fallback_roots.iter().copied());
                self.retained_fallback_reasons
                    .extend(fallback_reasons.iter().copied());
                self.retained_snapshot = Some(*mutation_snapshot);
            }

            fn run_fallback_source_style_invalidation(
                &mut self,
                kind: MoliRetainedSourceStyleInvalidationKind,
                fallback_roots: &'a IndexSet<u32>,
                fallback_reasons: &'a IndexSet<MoliSourceInvalidationFallbackReason>,
            ) {
                self.fallback_kind = Some(kind);
                self.fallback_roots.extend(fallback_roots.iter().copied());
                self.fallback_reasons
                    .extend(fallback_reasons.iter().copied());
            }
        }

        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "active".into());
        let queries = IndexSet::from([query]);
        let reasoned_roots = IndexSet::from([2_u32]);
        let exact_safety_roots = IndexSet::from([3_u32]);
        let fallback_reasons =
            IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector]);
        let snapshot = 7_u8;

        let retained = moli_retained_source_style_invalidation_from_parts(
            MoliRetainedSourceStyleInvalidationKind::RetainedQueries,
            Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback),
            None,
            Some(4_u32),
            Some(&queries),
            true,
            &reasoned_roots,
            &exact_safety_roots,
            &fallback_reasons,
            &snapshot,
        );

        let mut sink = Sink::default();
        retained.drain_into(&mut sink);
        assert_eq!(
            sink.retained_fallback_kind,
            Some(Some(
                MoliRetainedSourceStyleInvalidationKind::ContextFallback
            ))
        );
        assert_eq!(sink.retained_shadow_root, Some(4));
        assert_eq!(sink.retained_query_count, 1);
        assert_eq!(
            sink.retained_zero_matched_dependency_result_is_exact,
            Some(true)
        );
        assert_eq!(sink.retained_reasoned_roots, vec![2]);
        assert_eq!(sink.retained_exact_safety_roots, vec![3]);
        assert_eq!(
            sink.retained_fallback_reasons,
            vec![MoliSourceInvalidationFallbackReason::FullSelector]
        );
        assert_eq!(sink.retained_snapshot, Some(snapshot));

        let fallback = moli_retained_source_style_invalidation_from_parts(
            MoliRetainedSourceStyleInvalidationKind::FallbackOnly,
            None,
            None,
            None,
            None,
            false,
            &reasoned_roots,
            &exact_safety_roots,
            &fallback_reasons,
            &snapshot,
        );
        let mut sink = Sink::default();
        fallback.drain_into(&mut sink);
        assert_eq!(
            sink.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(sink.fallback_roots, vec![2]);
        assert_eq!(
            sink.fallback_reasons,
            vec![MoliSourceInvalidationFallbackReason::FullSelector]
        );
    }

    #[test]
    fn moli_source_dependency_request_requirement_merges_gates() {
        let exact = MoliSourceDependencyRequestRequirement::exact();
        let structural = MoliSourceDependencyRequestRequirement::child_list_structural();
        let relative = MoliSourceDependencyRequestRequirement::relative_previous_sibling();
        let both =
            MoliSourceDependencyRequestRequirement::child_list_structural_relative_previous_sibling();

        assert!(!exact.requires_child_list_structural_dependency());
        assert!(!exact.requires_relative_previous_sibling_dependency());
        assert!(structural.requires_child_list_structural_dependency());
        assert!(!structural.requires_relative_previous_sibling_dependency());
        assert!(!relative.requires_child_list_structural_dependency());
        assert!(relative.requires_relative_previous_sibling_dependency());
        assert!(both.requires_child_list_structural_dependency());
        assert!(both.requires_relative_previous_sibling_dependency());

        let merged = structural.merged_with(relative);
        assert!(!merged.requires_child_list_structural_dependency());
        assert!(merged.requires_relative_previous_sibling_dependency());

        let merged = both.merged_with(structural);
        assert!(merged.requires_child_list_structural_dependency());
        assert!(merged.requires_relative_previous_sibling_dependency());
    }

    #[test]
    fn moli_source_dependency_request_exposes_typed_context_and_gates() {
        let query = MoliRetainedStyleInvalidationQuery::id(1_u32, "target".into());
        let context = MoliDependencyInvalidationFallbackContext::from_mutation_relation(
            Some(2),
            Some(3),
            Some(4),
        );
        let request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            Some(context),
            MoliSourceDependencyRequestRequirement::child_list_structural_relative_previous_sibling(),
        );

        assert_eq!(request.query().root(), 1);
        assert!(request.requires_child_list_structural_dependency());
        assert!(request.requires_relative_previous_sibling_dependency());
        let context = request.context().expect("request should expose context");
        assert_eq!(context.parent(), Some(2));
        assert_eq!(context.previous_sibling(), Some(3));
        assert_eq!(context.next_sibling(), Some(4));

        let empty = MoliDependencyInvalidationFallbackContext::<u32>::default();
        assert_eq!(empty.parent(), None);
        assert_eq!(empty.previous_sibling(), None);
        assert_eq!(empty.next_sibling(), None);

        let exact_safety_roots = MoliDependencyInvalidationContextRoots::new(false, vec![5]);
        assert!(!exact_safety_roots.requires_source_fallback());
        assert_eq!(exact_safety_roots.roots(), &[5]);
        assert_eq!(exact_safety_roots.into_roots(), vec![5]);

        let source_fallback_roots =
            MoliDependencyInvalidationContextRoots::new(true, vec![6]);
        assert!(source_fallback_roots.requires_source_fallback());
        assert_eq!(source_fallback_roots.roots(), &[6]);
    }

    #[test]
    fn moli_source_dependency_summary_and_batch_source_expose_typed_inputs() {
        let dependency_summary = moli_dependency_summary_for_selector(".active");
        let source_summary = MoliSourceDependencySummary::from_parts(
            dependency_summary,
            true,
            moli_structural_boundary_summary_for_class("active"),
        );
        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "active".into());
        let request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            None,
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );

        assert!(source_summary
            .query_result(query.as_stylo_query())
            .has_any_dependency());
        assert!(source_summary
            .query_class(&Atom::from("active"))
            .has_any_dependency());
        assert!(source_summary.has_child_list_structural_dependency());
        assert!(source_summary.has_child_list_structural_dependency_for_requests(&[request]));
        assert!(!source_summary.has_relative_previous_sibling_dependency_for_requests(&[request]));
        assert!(!source_summary.has_slotted_dependency_for_requests(&[request]));
        assert!(source_summary.has_child_list_structural_dependency_for_requests(&[request]));
        assert!(
            !source_summary.requires_nonstructural_empty_target_fallback_for_requests(&[request])
        );
        assert!(source_summary
            .structural_boundary_cleanup_roots_for_requests(&[request], &[9])
            .is_empty());

        let mut relative_dependency = MoliDependencyQueryResult::default();
        relative_dependency.add_kind(MoliDependencyKind::RelativePrevSibling);
        let mut relative_dependency_summary = MoliDependencyInvalidationSummary::default();
        relative_dependency_summary
            .note_class_dependency(Atom::from("active"), relative_dependency);
        let relative_summary = MoliSourceDependencySummary::from_parts(
            relative_dependency_summary,
            false,
            MoliChildListStructuralBoundaryDependencySummary::default(),
        );
        let relative_request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            None,
            MoliSourceDependencyRequestRequirement::relative_previous_sibling(),
        );
        assert!(relative_summary
            .requires_nonstructural_empty_target_fallback_for_requests(&[relative_request]));
        assert_eq!(
            relative_summary
                .structural_boundary_cleanup_roots_for_requests(&[relative_request], &[9]),
            vec![9]
        );

        let source_roots = [2_u32];
        let cause_roots = [3_u32];
        let source = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &source_roots,
            &[],
        );
        assert!(source_summary
            .query_result(query.as_stylo_query())
            .has_any_dependency());
        assert_eq!(source.selected_fallback_roots(), &[2]);

        let source = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &source_roots,
            &cause_roots,
        );
        assert_eq!(source.selected_fallback_roots(), &[3]);
    }

    #[test]
    fn moli_structural_empty_target_gate_requires_a_keyed_dependency() {
        let source_summary = MoliSourceDependencySummary::from_parts(
            moli_dependency_summary_for_selector("details > summary:first-of-type"),
            true,
            moli_structural_boundary_summary_for_type("details"),
        );
        let details_query =
            MoliRetainedStyleInvalidationQuery::element_type(1_u32, "details".into());
        let details_request = MoliSourceDependencyInvalidationRequest::new(
            &details_query,
            None,
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );
        assert!(
            source_summary.has_child_list_structural_dependency_for_requests(&[details_request])
        );

        let unrelated_query =
            MoliRetainedStyleInvalidationQuery::element_type(2_u32, "em".into());
        let unrelated_request = MoliSourceDependencyInvalidationRequest::new(
            &unrelated_query,
            None,
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );
        assert!(
            !source_summary.has_child_list_structural_dependency_for_requests(&[unrelated_request])
        );
        assert!(!source_summary
            .requires_nonstructural_empty_target_fallback_for_requests(&[unrelated_request,]));

        let universal_summary = MoliSourceDependencySummary::from_parts(
            moli_dependency_summary_for_selector(":first-child"),
            true,
            moli_universal_structural_boundary_summary(),
        );
        let universal_query = MoliRetainedStyleInvalidationQuery::universal(3_u32);
        let universal_request = MoliSourceDependencyInvalidationRequest::new(
            &universal_query,
            None,
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );
        assert!(universal_summary
            .has_child_list_structural_dependency_for_requests(&[universal_request]));
        let universal_type_query =
            MoliRetainedStyleInvalidationQuery::element_type(4_u32, "article".into());
        let universal_type_request = MoliSourceDependencyInvalidationRequest::new(
            &universal_type_query,
            None,
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );
        assert!(universal_summary
            .has_child_list_structural_dependency_for_requests(&[universal_type_request]));

        let conservative_summary =
            MoliSourceDependencySummary::conservative_child_list_structural();
        assert!(conservative_summary
            .has_child_list_structural_dependency_for_requests(&[universal_type_request]));
    }

    #[test]
    fn moli_source_dependency_summary_exposes_aggregate_predicates() {
        let mut dependency_summary = MoliDependencyInvalidationSummary::default();

        let mut relative = MoliDependencyQueryResult::default();
        relative.add_fallback_reason(MoliDependencyFallbackReason::RelativeAnySelector);
        dependency_summary.note_class_dependency(Atom::from("anchor"), relative);

        let mut sibling = MoliDependencyQueryResult::default();
        sibling.add_kind(MoliDependencyKind::Siblings);
        dependency_summary.note_id_dependency(Atom::from("target"), sibling);

        let mut focus = MoliDependencyQueryResult::default();
        focus.add_kind(MoliDependencyKind::Element);
        dependency_summary
            .note_state_dependency(ElementState::FOCUS | ElementState::FOCUS_WITHIN, focus);

        let mut target = MoliDependencyQueryResult::default();
        target.add_kind(MoliDependencyKind::Element);
        dependency_summary.note_state_dependency(ElementState::URLTARGET, target);

        let source_summary = MoliSourceDependencySummary::from_parts(
            dependency_summary,
            true,
            MoliChildListStructuralBoundaryDependencySummary::default(),
        );

        assert!(source_summary.has_relative_selector_dependency());
        assert!(source_summary.has_focus_dependency());
        assert!(source_summary.has_focus_within_dependency());
        assert!(source_summary.has_target_dependency());
        assert!(source_summary.has_child_list_structural_dependency());
        assert!(source_summary.has_sibling_dependency());
    }

    #[test]
    fn moli_child_list_retained_query_batch_drains_typed_parts() {
        let requirement = MoliSourceDependencyRequestRequirement::child_list_structural();
        let query = MoliRetainedStyleInvalidationQuery::universal(1_u32);
        let row = MoliRetainedStyleChildListInvalidationQuery::new(query, requirement);
        let batch = MoliRetainedStyleChildListInvalidationQueries::new(
            vec![row],
            vec![1],
            vec![2],
            vec![3],
        );
        let mut sink = ChildListInvalidationBatchSinkForTest::default();

        batch.drain_into(&mut sink);

        assert_eq!(sink.rows.len(), 1);
        assert_eq!(sink.rows[0].0.root(), 1);
        assert_eq!(
            sink.rows[0].1,
            MoliSourceDependencyRequestRequirement::child_list_structural()
        );
        assert_eq!(sink.base_roots, vec![1]);
        assert_eq!(sink.empty_target_fallback_roots, vec![2]);
        assert_eq!(sink.relative_previous_sibling_cleanup_roots, vec![3]);
    }

    #[derive(Default)]
    struct ChildListInvalidationBatchSinkForTest {
        rows: Vec<(
            MoliRetainedStyleInvalidationQuery<u32>,
            MoliSourceDependencyRequestRequirement,
        )>,
        base_roots: Vec<u32>,
        empty_target_fallback_roots: Vec<u32>,
        relative_previous_sibling_cleanup_roots: Vec<u32>,
    }

    impl MoliRetainedStyleChildListInvalidationQueriesSink<u32>
        for ChildListInvalidationBatchSinkForTest
    {
        fn record_child_list_retained_query(
            &mut self,
            query: MoliRetainedStyleInvalidationQuery<u32>,
            requirement: MoliSourceDependencyRequestRequirement,
        ) {
            self.rows.push((query, requirement));
        }

        fn extend_child_list_base_roots(&mut self, roots: Vec<u32>) {
            self.base_roots.extend(roots);
        }

        fn extend_child_list_empty_target_fallback_roots(&mut self, roots: Vec<u32>) {
            self.empty_target_fallback_roots.extend(roots);
        }

        fn extend_child_list_relative_previous_sibling_cleanup_roots(&mut self, roots: Vec<u32>) {
            self.relative_previous_sibling_cleanup_roots.extend(roots);
        }
    }

    #[test]
    fn moli_child_list_retained_query_batch_drains_into_sink() {
        let requirement = MoliSourceDependencyRequestRequirement::relative_previous_sibling();
        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "active".into());
        let row = MoliRetainedStyleChildListInvalidationQuery::new(query, requirement);
        let batch = MoliRetainedStyleChildListInvalidationQueries::new(
            vec![row],
            vec![1],
            vec![2],
            vec![3],
        );
        let mut sink = ChildListInvalidationBatchSinkForTest::default();

        batch.drain_into(&mut sink);

        assert_eq!(sink.rows.len(), 1);
        assert_eq!(sink.rows[0].0.root(), 1);
        assert_eq!(sink.rows[0].1, requirement);
        assert_eq!(sink.base_roots, vec![1]);
        assert_eq!(sink.empty_target_fallback_roots, vec![2]);
        assert_eq!(sink.relative_previous_sibling_cleanup_roots, vec![3]);
    }

    #[test]
    fn moli_child_list_retained_query_builder_merges_rows_and_roots() {
        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "active".into());
        let mut builder = MoliRetainedStyleChildListInvalidationQueryBuilder::new();
        builder.insert_queries(
            [query.clone()],
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );
        builder.insert_queries(
            [query],
            MoliSourceDependencyRequestRequirement::exact(),
        );
        builder.insert_base_root(2);
        builder.insert_base_root(2);
        builder.insert_empty_target_fallback_root(3);
        builder.insert_empty_target_fallback_root(3);
        builder.insert_relative_previous_sibling_cleanup_root(4);
        builder.insert_relative_previous_sibling_cleanup_root(4);

        let batch = builder
            .into_queries()
            .expect("builder should emit query rows");
        let mut sink = ChildListInvalidationBatchSinkForTest::default();
        batch.drain_into(&mut sink);
        assert_eq!(sink.rows.len(), 1);
        assert_eq!(
            sink.rows[0].0.as_stylo_query(),
            MoliStyleInvalidationQuery::Class("active")
        );
        assert_eq!(
            sink.rows[0].1,
            MoliSourceDependencyRequestRequirement::exact()
        );
        assert_eq!(sink.base_roots, vec![2]);
        assert_eq!(sink.empty_target_fallback_roots, vec![3]);
        assert_eq!(sink.relative_previous_sibling_cleanup_roots, vec![4]);
        assert!(
            MoliRetainedStyleChildListInvalidationQueryBuilder::<u32>::new()
                .into_queries()
                .is_none()
        );
    }

    #[test]
    fn moli_child_list_sibling_boundary_plan_classifies_cleanup_buckets() {
        fn flags(plan: &MoliChildListSiblingBoundaryPlan<u32>) -> (bool, bool, bool) {
            (
                plan.includes_base_root(),
                plan.includes_empty_target_fallback_root(),
                plan.includes_relative_previous_sibling_cleanup_root(),
            )
        }

        let inserted_middle_previous = moli_child_list_sibling_boundary_plan(
            Some(1_u32),
            false,
            MoliChildListSiblingBoundaryKind::AddedPreviousSibling {
                inserted_at_end: false,
            },
        )
        .expect("unchanged previous sibling should produce a plan");
        assert_eq!(*inserted_middle_previous.root(), 1);
        assert_eq!(flags(&inserted_middle_previous), (false, true, true));

        let inserted_end_previous = moli_child_list_sibling_boundary_plan(
            Some(2_u32),
            false,
            MoliChildListSiblingBoundaryKind::AddedPreviousSibling {
                inserted_at_end: true,
            },
        )
        .expect("appended previous sibling should produce a plan");
        assert_eq!(flags(&inserted_end_previous), (true, true, true));

        let inserted_next = moli_child_list_sibling_boundary_plan(
            Some(3_u32),
            false,
            MoliChildListSiblingBoundaryKind::AddedNextSibling,
        )
        .expect("unchanged next sibling should produce a plan");
        assert_eq!(flags(&inserted_next), (true, true, false));

        let removed_previous = moli_child_list_sibling_boundary_plan(
            Some(4_u32),
            false,
            MoliChildListSiblingBoundaryKind::RemovedPreviousSibling,
        )
        .expect("unchanged previous sibling should produce a plan");
        assert_eq!(flags(&removed_previous), (true, true, true));

        let removed_next = moli_child_list_sibling_boundary_plan(
            Some(5_u32),
            false,
            MoliChildListSiblingBoundaryKind::RemovedNextSibling,
        )
        .expect("unchanged next sibling should produce a plan");
        assert_eq!(flags(&removed_next), (true, true, false));

        let removed_earlier = moli_child_list_sibling_boundary_plan(
            Some(6_u32),
            false,
            MoliChildListSiblingBoundaryKind::RemovedEarlierSibling,
        )
        .expect("unchanged earlier sibling should produce a plan");
        assert_eq!(flags(&removed_earlier), (false, false, true));

        assert!(moli_child_list_sibling_boundary_plan(
            Some(7_u32),
            true,
            MoliChildListSiblingBoundaryKind::AddedNextSibling,
        )
        .is_none());
        assert!(moli_child_list_sibling_boundary_plan::<u32>(
            None,
            false,
            MoliChildListSiblingBoundaryKind::RemovedNextSibling,
        )
        .is_none());

        let mut builder = MoliRetainedStyleChildListInvalidationQueryBuilder::new();
        builder.insert_queries(
            [MoliRetainedStyleInvalidationQuery::universal(10_u32)],
            MoliSourceDependencyRequestRequirement::exact(),
        );
        inserted_end_previous.apply_to_builder(&mut builder);
        inserted_next.apply_to_builder(&mut builder);
        removed_earlier.apply_to_builder(&mut builder);

        let batch = builder
            .into_queries()
            .expect("query row keeps roots visible");
        let mut sink = ChildListInvalidationBatchSinkForTest::default();
        batch.drain_into(&mut sink);
        assert_eq!(sink.base_roots, vec![2, 3]);
        assert_eq!(sink.empty_target_fallback_roots, vec![2, 3]);
        assert_eq!(sink.relative_previous_sibling_cleanup_roots, vec![2, 6]);
    }

    #[test]
    fn moli_child_list_dependency_fallback_context_matches_query_root() {
        let removed_snapshot = MoliElementDependencySnapshot::new(
            4_u32,
            "em".into(),
            ElementState::empty(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        let added_nodes = [2_u32];
        let removed_nodes = [3_u32];
        let removed_snapshots = [removed_snapshot];
        let context = MoliRetainedStyleChildListMutationContext::new(
            1,
            &added_nodes,
            &removed_nodes,
            &removed_snapshots,
            Some(5),
            Some(6),
        );

        let snapshot_query =
            MoliRetainedStyleInvalidationQuery::element_type(4_u32, "em".into())
                .with_sibling_traversal(Some(MoliRetainedStyleSiblingTraversal::new(
                    Some(7),
                    Some(8),
                )));
        let fallback =
            moli_child_list_dependency_fallback_context_for_query([context], &snapshot_query)
                .expect("removed snapshot root should match child-list context");
        assert_eq!(fallback.parent(), Some(1));
        assert_eq!(fallback.previous_sibling(), Some(7));
        assert_eq!(fallback.next_sibling(), Some(8));

        let added_query = MoliRetainedStyleInvalidationQuery::universal(2_u32);
        let fallback =
            moli_child_list_dependency_fallback_context_for_query([context], &added_query)
                .expect("added root should match child-list context");
        assert_eq!(fallback.previous_sibling(), Some(5));
        assert_eq!(fallback.next_sibling(), Some(6));

        let unrelated_query = MoliRetainedStyleInvalidationQuery::universal(9_u32);
        assert!(moli_child_list_dependency_fallback_context_for_query(
            [context],
            &unrelated_query,
        )
        .is_none());
    }

    #[test]
    fn moli_style_mutation_element_snapshot_preserves_first_old_values() {
        let mut first = MoliStyleMutationElementSnapshot::default();
        first.record_attribute_change("class", Some("initial".into()));
        first.record_attribute_change("class", Some("middle".into()));
        assert_eq!(first.try_record_old_state(ElementState::CHECKED), Some(()));
        assert_eq!(first.try_record_old_state(ElementState::FOCUS), None);
        first.record_old_custom_states(vec!["first".into()]);
        first.record_old_custom_states(vec!["second".into()]);

        let mut second = MoliStyleMutationElementSnapshot::default();
        second.record_attribute_change("class", Some("late".into()));
        second.record_attribute_change("id", Some("old-id".into()));
        second.record_attribute_change("data-state", None);
        second.try_record_old_state(ElementState::FOCUS);
        second.record_old_custom_states(vec!["incoming".into()]);

        first.merge_from(second);
        let changes = first.attribute_changes().collect::<Vec<_>>();

        assert_eq!(first.attribute_change_count(), 3);
        assert_eq!(changes[0].name(), "class");
        assert_eq!(changes[0].old_value(), Some("initial"));
        assert_eq!(changes[1].name(), "id");
        assert_eq!(changes[1].old_value(), Some("old-id"));
        assert_eq!(changes[2].name(), "data-state");
        assert_eq!(changes[2].old_value(), None);
        assert_eq!(first.old_state(), Some(ElementState::CHECKED));
        assert_eq!(
            first.old_custom_states(),
            Some(["first".to_string()].as_slice())
        );
    }

    #[derive(Default)]
    struct MoliPlannedFallbackRootTargetPartsForTest {
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        fallback_roots: Vec<u32>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    }

    #[derive(Default)]
    struct MoliPlannedSourceDependencyPartsForTest {
        source_index: Option<usize>,
        structural_boundary_cleanup_roots: Vec<u32>,
        target_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
        exact_queries: Vec<MoliRetainedStyleInvalidationQuery<u32>>,
        reasoned_fallback_roots: Vec<u32>,
        exact_safety_fallback_roots: Vec<u32>,
        fallback_roots: Vec<u32>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
    }

    impl MoliPlannedFallbackRootInvalidationTargetPartsSink<u32>
        for MoliPlannedFallbackRootTargetPartsForTest
    {
        fn set_planned_fallback_root_target_parts(
            &mut self,
            fallback_kind: MoliRetainedSourceStyleInvalidationKind,
            fallback_roots: Vec<u32>,
            fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        ) {
            self.fallback_kind = Some(fallback_kind);
            self.fallback_roots = fallback_roots;
            self.fallback_reasons = fallback_reasons;
        }
    }

    impl MoliPlannedSourceDependencyInvalidationPartsSink<u32>
        for MoliPlannedSourceDependencyPartsForTest
    {
        fn set_planned_source_dependency_source_index(&mut self, source_index: usize) {
            self.source_index = Some(source_index);
        }

        fn set_planned_source_dependency_structural_boundary_cleanup_roots(
            &mut self,
            structural_boundary_cleanup_roots: Vec<u32>,
        ) {
            self.structural_boundary_cleanup_roots = structural_boundary_cleanup_roots;
        }
    }

    impl MoliPlannedSourceDependencyInvalidationTargetPartsSink<u32>
        for MoliPlannedSourceDependencyPartsForTest
    {
        fn set_planned_retained_source_dependency_target_parts(
            &mut self,
            exact_queries: Vec<MoliRetainedStyleInvalidationQuery<u32>>,
            fallback_kind: Option<MoliRetainedSourceStyleInvalidationKind>,
            reasoned_fallback_roots: Vec<u32>,
            exact_safety_fallback_roots: Vec<u32>,
            fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        ) {
            self.target_kind = Some(MoliRetainedSourceStyleInvalidationKind::RetainedQueries);
            self.fallback_kind = fallback_kind;
            self.exact_queries = exact_queries;
            self.reasoned_fallback_roots = reasoned_fallback_roots;
            self.exact_safety_fallback_roots = exact_safety_fallback_roots;
            self.fallback_reasons = fallback_reasons;
        }

        fn set_planned_fallback_source_dependency_target_parts(
            &mut self,
            fallback_kind: MoliRetainedSourceStyleInvalidationKind,
            fallback_roots: Vec<u32>,
            fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        ) {
            self.target_kind = Some(fallback_kind);
            self.fallback_roots = fallback_roots;
            self.fallback_reasons = fallback_reasons;
        }

        fn set_planned_missing_fallback_roots_source_dependency_target_parts(
            &mut self,
            fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        ) {
            self.target_kind =
                Some(MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots);
            self.fallback_reasons = fallback_reasons;
        }
    }

    fn planned_source_dependency_parts_for_test(
        planned: MoliPlannedSourceDependencyInvalidation<u32>,
    ) -> MoliPlannedSourceDependencyPartsForTest {
        let mut sink = MoliPlannedSourceDependencyPartsForTest::default();
        planned.drain_into(&mut sink);
        sink
    }

    fn planned_source_dependency_target_parts_for_test(
        target: MoliPlannedSourceDependencyInvalidationTarget<u32>,
    ) -> MoliPlannedSourceDependencyPartsForTest {
        let mut sink = MoliPlannedSourceDependencyPartsForTest::default();
        target.drain_into(&mut sink);
        sink
    }

    #[derive(Default)]
    struct MoliSourceDependencyBatchPlanForTest {
        work_sources: Vec<MoliPlannedSourceDependencyInvalidation<u32>>,
        work_boundary_fallback: Option<MoliPlannedFallbackRootInvalidationTarget<u32>>,
        requires_source_fallback: Option<MoliPlannedSourceDependencyInvalidation<u32>>,
    }

    impl MoliSourceDependencyInvalidationBatchPlanSink<u32>
        for MoliSourceDependencyBatchPlanForTest
    {
        fn set_source_dependency_batch_work(
            &mut self,
            sources: Vec<MoliPlannedSourceDependencyInvalidation<u32>>,
            boundary_fallback: Option<MoliPlannedFallbackRootInvalidationTarget<u32>>,
        ) {
            self.work_sources = sources;
            self.work_boundary_fallback = boundary_fallback;
        }

        fn set_source_dependency_batch_requires_source_fallback(
            &mut self,
            source: MoliPlannedSourceDependencyInvalidation<u32>,
        ) {
            self.requires_source_fallback = Some(source);
        }
    }

    fn source_dependency_batch_plan_for_test(
        plan: MoliSourceDependencyInvalidationBatchPlan<u32>,
    ) -> MoliSourceDependencyBatchPlanForTest {
        let mut sink = MoliSourceDependencyBatchPlanForTest::default();
        plan.drain_into(&mut sink);
        sink
    }

    #[test]
    fn moli_planned_source_dependency_artifacts_drain_into_typed_sinks() {
        let empty_target_roots = [10_u32];
        let relative_cleanup_roots = [20_u32];
        let boundary_roots = MoliSourceDependencyBoundaryRoots::new(
            &empty_target_roots,
            &relative_cleanup_roots,
        );
        assert_eq!(boundary_roots.empty_target_fallback_roots, &[10]);
        assert_eq!(
            boundary_roots.relative_previous_sibling_cleanup_roots,
            &[20]
        );

        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "active".into());
        let planned =
            MoliPlannedSourceDependencyInvalidation::retained_queries_with_fallback_kind(
                3,
                vec![query],
                Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback),
                vec![4],
                vec![5],
                [MoliSourceInvalidationFallbackReason::FullSelector],
                vec![6],
            );
        let parts = planned_source_dependency_parts_for_test(planned);
        assert_eq!(parts.source_index, Some(3));
        assert_eq!(parts.structural_boundary_cleanup_roots, vec![6]);
        assert_eq!(
            parts.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::RetainedQueries)
        );
        assert_eq!(parts.exact_queries[0].root(), 1);
        assert_eq!(
            parts.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback)
        );
        assert_eq!(parts.reasoned_fallback_roots, vec![4]);
        assert_eq!(parts.exact_safety_fallback_roots, vec![5]);
        assert!(parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::FullSelector));

        let missing = planned_source_dependency_parts_for_test(
            MoliPlannedSourceDependencyInvalidation::<u32>::missing_fallback_roots(
                7,
                [],
                Vec::new(),
            ),
        );
        assert_eq!(missing.source_index, Some(7));
        assert_eq!(
            missing.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots)
        );
        assert!(missing
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::MissingFallbackRoots));

        let boundary_fallback =
            MoliPlannedFallbackRootInvalidationTarget::source_scope_fallback(vec![8], []);
        let mut fallback_parts = MoliPlannedFallbackRootTargetPartsForTest::default();
        boundary_fallback.drain_into(&mut fallback_parts);
        assert_eq!(
            fallback_parts.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::SourceScopeFallback)
        );
        assert_eq!(fallback_parts.fallback_roots, vec![8]);
        assert!(fallback_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::SourceScopeFallback));

        let promoted_safety_target =
            MoliPlannedSourceDependencyInvalidationTarget::from_source_dependency_work_parts(
                Vec::new(),
                None,
                Vec::new(),
                vec![9],
                [],
            )
            .expect("orphan exact-safety roots should become a fallback target");
        let promoted_safety_target =
            planned_source_dependency_target_parts_for_test(promoted_safety_target);
        assert_eq!(
            promoted_safety_target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(promoted_safety_target.fallback_roots, vec![9]);
        assert!(promoted_safety_target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::InexactEmptyResult));

        let missing_selected_roots =
            MoliPlannedSourceDependencyInvalidationTarget::<u32>::source_dependency_fallback(
                Vec::new(),
                [MoliSourceInvalidationFallbackReason::FullSelector],
            );
        let missing_selected_roots =
            planned_source_dependency_target_parts_for_test(missing_selected_roots);
        assert_eq!(
            missing_selected_roots.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots)
        );
        assert!(missing_selected_roots
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::FullSelector));
        assert!(missing_selected_roots
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::MissingFallbackRoots));

        let source_plan = MoliSourceDependencyInvalidationSourcePlan::work(
            Some(
                MoliPlannedSourceDependencyInvalidationTarget::source_dependency_fallback(
                    vec![11],
                    [MoliSourceInvalidationFallbackReason::FullSelector],
                ),
            ),
            vec![12],
        );
        let MoliSourceDependencyInvalidationSourcePlan::Work {
            target,
            exact_structural_cleanup_roots,
        } = source_plan
        else {
            panic!("source-local work plan should expose work target");
        };
        assert!(target.is_some());
        assert_eq!(exact_structural_cleanup_roots, vec![12]);

        let source_plan =
            MoliSourceDependencyInvalidationSourcePlan::requires_source_fallback(
                MoliPlannedSourceDependencyInvalidationTarget::source_dependency_fallback(
                    Vec::<u32>::new(),
                    [MoliSourceInvalidationFallbackReason::FullSelector],
                ),
            );
        let MoliSourceDependencyInvalidationSourcePlan::RequiresSourceFallback { target } =
            source_plan
        else {
            panic!("source-local fallback plan should expose fallback target");
        };
        let target = planned_source_dependency_target_parts_for_test(target);
        assert_eq!(
            target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots)
        );
        assert!(target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::FullSelector));

        let batch_plan_sink = source_dependency_batch_plan_for_test(
            MoliSourceDependencyInvalidationBatchPlan::work(
                vec![
                    MoliPlannedSourceDependencyInvalidation::fallback_only(
                        1,
                        vec![2],
                        [],
                        Vec::new(),
                    ),
                ],
                None,
            ),
        );
        assert_eq!(batch_plan_sink.work_sources.len(), 1);
        assert!(batch_plan_sink.work_boundary_fallback.is_none());
        assert!(batch_plan_sink.requires_source_fallback.is_none());

        let batch_plan_sink = source_dependency_batch_plan_for_test(
            MoliSourceDependencyInvalidationBatchPlan::requires_source_fallback(
                MoliPlannedSourceDependencyInvalidation::missing_fallback_roots(
                    4,
                    [MoliSourceInvalidationFallbackReason::FullSelector],
                    Vec::new(),
                ),
            ),
        );
        assert!(batch_plan_sink.work_sources.is_empty());
        assert!(batch_plan_sink.work_boundary_fallback.is_none());
        assert!(batch_plan_sink.requires_source_fallback.is_some());
    }

    #[test]
    fn moli_source_scope_and_fallback_roots_plans_label_targets() {
        let source_scope_target = moli_source_scope_fallback_plan(
            || vec![4_u32],
            [MoliSourceInvalidationFallbackReason::UnsupportedStateDependency],
        );
        let mut source_scope_parts = MoliPlannedFallbackRootTargetPartsForTest::default();
        source_scope_target.drain_into(&mut source_scope_parts);
        assert_eq!(
            source_scope_parts.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::SourceScopeFallback)
        );
        assert_eq!(source_scope_parts.fallback_roots, vec![4]);
        assert!(source_scope_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::SourceScopeFallback));
        assert!(source_scope_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::UnsupportedStateDependency));

        let fallback_target = moli_fallback_roots_plan(
            vec![5_u32],
            [MoliSourceInvalidationFallbackReason::UnsupportedStateDependency],
        );
        let mut fallback_parts = MoliPlannedFallbackRootTargetPartsForTest::default();
        fallback_target.drain_into(&mut fallback_parts);
        assert_eq!(
            fallback_parts.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(fallback_parts.fallback_roots, vec![5]);
        assert!(!fallback_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::SourceScopeFallback));
        assert!(fallback_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::UnsupportedStateDependency));
    }

    #[test]
    fn moli_runtime_or_source_scope_fallback_plan_prefers_runtime_roots() {
        let source_scope_target = moli_runtime_or_source_scope_fallback_plan(
            Vec::new(),
            || vec![1_u32, 2],
            [MoliSourceInvalidationFallbackReason::UnsupportedStateDependency],
        );
        let mut source_scope_parts = MoliPlannedFallbackRootTargetPartsForTest::default();
        source_scope_target.drain_into(&mut source_scope_parts);
        assert_eq!(
            source_scope_parts.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::SourceScopeFallback)
        );
        assert_eq!(source_scope_parts.fallback_roots, vec![1, 2]);
        assert!(source_scope_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::SourceScopeFallback));
        assert!(source_scope_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::UnsupportedStateDependency));

        let runtime_target = moli_runtime_or_source_scope_fallback_plan(
            vec![3_u32],
            || panic!("source-scope roots should not be resolved when runtime roots exist"),
            [MoliSourceInvalidationFallbackReason::UnsupportedStateDependency],
        );
        let mut runtime_parts = MoliPlannedFallbackRootTargetPartsForTest::default();
        runtime_target.drain_into(&mut runtime_parts);
        assert_eq!(
            runtime_parts.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(runtime_parts.fallback_roots, vec![3]);
        assert!(!runtime_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::SourceScopeFallback));
        assert!(runtime_parts
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::UnsupportedStateDependency));
    }

    #[test]
    fn moli_stylesheet_source_scope_fallback_roots_dispatches_input() {
        struct Resolver;

        impl MoliStylesheetSourceScopeFallbackRootsResolver<u32> for Resolver {
            fn stylesheet_owner_source_scope_fallback_roots(&self, owner: u32) -> Vec<u32> {
                assert_eq!(owner, 1);
                vec![10]
            }

            fn document_source_scope_fallback_roots(&self, document: u32) -> Vec<u32> {
                assert_eq!(document, 2);
                vec![20]
            }

            fn shadow_root_source_scope_fallback_roots(&self, root: u32) -> Vec<u32> {
                assert_eq!(root, 3);
                vec![30, 31]
            }
        }

        assert_eq!(
            moli_stylesheet_source_scope_fallback_roots(
                MoliStylesheetSourceScopeFallbackInput::StylesheetOwner { owner: 1 },
                &Resolver,
            ),
            vec![10]
        );
        assert_eq!(
            moli_stylesheet_source_scope_fallback_roots(
                MoliStylesheetSourceScopeFallbackInput::DocumentAdopted { document: 2 },
                &Resolver,
            ),
            vec![20]
        );
        assert_eq!(
            moli_stylesheet_source_scope_fallback_roots(
                MoliStylesheetSourceScopeFallbackInput::ShadowRootAdopted { root: 3 },
                &Resolver,
            ),
            vec![30, 31]
        );
        assert!(moli_stylesheet_source_scope_fallback_roots(
            MoliStylesheetSourceScopeFallbackInput::Unscoped,
            &Resolver,
        )
        .is_empty());
    }

    #[test]
    fn moli_source_dependency_batch_skips_unrelated_structural_source() {
        struct UnexpectedContextRootsProvider;

        impl MoliSourceDependencyInvalidationContextRootsProvider<u32>
            for UnexpectedContextRootsProvider
        {
            fn context_roots_for_source_dependency(
                &mut self,
                _root: u32,
                _plan: MoliDependencyContextRootPlan,
                _context: MoliDependencyInvalidationFallbackContext<u32>,
            ) -> MoliDependencyInvalidationContextRoots<u32> {
                panic!("an unrelated source query must not request context roots")
            }
        }

        let source_summary = MoliSourceDependencySummary::from_parts(
            moli_dependency_summary_for_selector("details > summary:first-of-type"),
            true,
            moli_structural_boundary_summary_for_type("details"),
        );
        let source_roots = [99_u32];
        let source = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &source_roots,
            &[],
        );
        let query = MoliRetainedStyleInvalidationQuery::element_type(1_u32, "em".into());
        let request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            None,
            MoliSourceDependencyRequestRequirement::child_list_structural(),
        );
        let empty_target_roots = [10_u32];
        let mut provider = UnexpectedContextRootsProvider;

        let plan = moli_source_dependency_invalidation_batch_plan(
            &[source],
            &[request],
            MoliSourceDependencyBoundaryRoots::new(&empty_target_roots, &[]),
            &mut provider,
        );

        let plan = source_dependency_batch_plan_for_test(plan);
        assert!(plan.work_sources.is_empty());
        assert!(plan.work_boundary_fallback.is_none());
        assert!(plan.requires_source_fallback.is_none());
    }

    #[test]
    fn moli_source_dependency_batch_plan_uses_context_roots_as_exact_query_safety() {
        #[derive(Default)]
        struct ContextRootsProviderForTest {
            calls: usize,
        }

        impl MoliSourceDependencyInvalidationContextRootsProvider<u32>
            for ContextRootsProviderForTest
        {
            fn context_roots_for_source_dependency(
                &mut self,
                root: u32,
                _plan: MoliDependencyContextRootPlan,
                context: MoliDependencyInvalidationFallbackContext<u32>,
            ) -> MoliDependencyInvalidationContextRoots<u32> {
                self.calls += 1;
                assert_eq!(root, 1);
                assert_eq!(context.parent(), Some(2));
                assert_eq!(context.previous_sibling(), Some(3));
                assert_eq!(context.next_sibling(), Some(4));
                MoliDependencyInvalidationContextRoots::new(false, vec![10])
            }
        }

        let mut dependency = MoliDependencyQueryResult::default();
        dependency
            .add_fallback_reason(MoliDependencyFallbackReason::NestedRelativeSelectorDependency);
        let mut dependency_summary = MoliDependencyInvalidationSummary::default();
        dependency_summary.note_class_dependency(Atom::from("active"), dependency);
        let source_summary = MoliSourceDependencySummary::from_parts(
            dependency_summary,
            false,
            MoliChildListStructuralBoundaryDependencySummary::default(),
        );
        let source_roots = [99_u32];
        let source = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &source_roots,
            &[],
        );
        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "active".into());
        let context = MoliDependencyInvalidationFallbackContext::from_mutation_relation(
            Some(2),
            Some(3),
            Some(4),
        );
        let request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            Some(context),
            MoliSourceDependencyRequestRequirement::exact(),
        );
        let mut provider = ContextRootsProviderForTest::default();

        let plan = moli_source_dependency_invalidation_batch_plan(
            &[source],
            &[request],
            MoliSourceDependencyBoundaryRoots::default(),
            &mut provider,
        );

        assert_eq!(provider.calls, 1);
        let mut plan = source_dependency_batch_plan_for_test(plan);
        assert!(plan.work_boundary_fallback.is_none());
        assert!(plan.requires_source_fallback.is_none());
        let sources = &mut plan.work_sources;
        assert_eq!(sources.len(), 1);
        let target = planned_source_dependency_parts_for_test(
            sources.pop().expect("source work should exist"),
        );
        assert_eq!(
            target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::RetainedQueries)
        );
        assert_eq!(target.exact_queries, vec![query]);
        assert_eq!(
            target.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::ContextFallback)
        );
        assert!(target.reasoned_fallback_roots.is_empty());
        assert_eq!(target.exact_safety_fallback_roots, vec![10]);
        assert!(target.fallback_reasons.is_empty());
    }

    #[test]
    fn moli_source_dependency_batch_plan_accumulates_missing_fallback_root_reasons() {
        struct ContextRootsProviderForTest;

        impl MoliSourceDependencyInvalidationContextRootsProvider<u32>
            for ContextRootsProviderForTest
        {
            fn context_roots_for_source_dependency(
                &mut self,
                _root: u32,
                _plan: MoliDependencyContextRootPlan,
                _context: MoliDependencyInvalidationFallbackContext<u32>,
            ) -> MoliDependencyInvalidationContextRoots<u32> {
                panic!("missing-root source fallback should not need context roots")
            }
        }

        let mut nth_dependency = MoliDependencyQueryResult::default();
        nth_dependency.add_fallback_reason(MoliDependencyFallbackReason::NthOfDependency);
        let mut full_dependency = MoliDependencyQueryResult::default();
        full_dependency.add_fallback_reason(MoliDependencyFallbackReason::FullSelector);
        let mut dependency_summary = MoliDependencyInvalidationSummary::default();
        dependency_summary.note_class_dependency(Atom::from("nth"), nth_dependency);
        dependency_summary.note_class_dependency(Atom::from("full"), full_dependency);
        let source_summary = MoliSourceDependencySummary::from_parts(
            dependency_summary,
            false,
            MoliChildListStructuralBoundaryDependencySummary::default(),
        );
        let source =
            MoliSourceDependencyInvalidationBatchSource::new(&source_summary, &[], &[]);
        let nth_query = MoliRetainedStyleInvalidationQuery::class(1_u32, "nth".into());
        let full_query = MoliRetainedStyleInvalidationQuery::class(1_u32, "full".into());
        let requests = [
            MoliSourceDependencyInvalidationRequest::new(
                &nth_query,
                None,
                MoliSourceDependencyRequestRequirement::exact(),
            ),
            MoliSourceDependencyInvalidationRequest::new(
                &full_query,
                None,
                MoliSourceDependencyRequestRequirement::exact(),
            ),
        ];
        let mut provider = ContextRootsProviderForTest;

        let plan = moli_source_dependency_invalidation_batch_plan(
            &[source],
            &requests,
            MoliSourceDependencyBoundaryRoots::default(),
            &mut provider,
        );

        let mut plan = source_dependency_batch_plan_for_test(plan);
        assert!(plan.work_sources.is_empty());
        assert!(plan.work_boundary_fallback.is_none());
        let target = planned_source_dependency_parts_for_test(
            plan.requires_source_fallback
                .take()
                .expect("missing fallback roots should force source fallback"),
        );
        assert_eq!(
            target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots)
        );
        assert!(target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::NthOfDependency));
        assert!(target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::FullSelector));
        assert!(target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::MissingFallbackRoots));
    }

    #[test]
    fn moli_source_dependency_batch_plan_uses_exact_structural_roots_for_custom_state_nth_of() {
        #[derive(Default)]
        struct ContextRootsProviderForTest {
            calls: usize,
        }

        impl MoliSourceDependencyInvalidationContextRootsProvider<u32>
            for ContextRootsProviderForTest
        {
            fn context_roots_for_source_dependency(
                &mut self,
                root: u32,
                _plan: MoliDependencyContextRootPlan,
                context: MoliDependencyInvalidationFallbackContext<u32>,
            ) -> MoliDependencyInvalidationContextRoots<u32> {
                self.calls += 1;
                assert_eq!(root, 1);
                assert_eq!(context.parent(), Some(2));
                assert_eq!(context.previous_sibling(), None);
                assert_eq!(context.next_sibling(), Some(3));
                // The generic context-root builder mirrors the nth fallback
                // classification in this bit. Nth-only planning must still
                // recognize that the sibling region is complete.
                MoliDependencyInvalidationContextRoots::new(true, vec![3, 4])
            }
        }

        let mut dependency = MoliDependencyQueryResult::default();
        dependency.add_kind(MoliDependencyKind::Siblings);
        dependency.add_fallback_reason(MoliDependencyFallbackReason::NthOfDependency);
        let mut dependency_summary = MoliDependencyInvalidationSummary::default();
        dependency_summary.note_custom_state_dependency(AtomIdent::from("--active"), dependency);
        let source_summary = MoliSourceDependencySummary::from_parts(
            dependency_summary,
            true,
            MoliChildListStructuralBoundaryDependencySummary::default(),
        );
        let source_roots = [99_u32];
        let source = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &source_roots,
            &[],
        );
        let query =
            MoliRetainedStyleInvalidationQuery::custom_state(1_u32, "--active".into());
        let context = MoliDependencyInvalidationFallbackContext::from_mutation_relation(
            Some(2),
            None,
            Some(3),
        );
        let request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            Some(context),
            MoliSourceDependencyRequestRequirement::exact(),
        );
        let mut provider = ContextRootsProviderForTest::default();

        let plan = moli_source_dependency_invalidation_batch_plan(
            &[source],
            &[request],
            MoliSourceDependencyBoundaryRoots::default(),
            &mut provider,
        );

        assert_eq!(provider.calls, 1);
        let mut plan = source_dependency_batch_plan_for_test(plan);
        assert!(plan.work_boundary_fallback.is_none());
        assert!(plan.requires_source_fallback.is_none());
        let sources = &mut plan.work_sources;
        assert_eq!(sources.len(), 1);
        let target = planned_source_dependency_parts_for_test(
            sources.pop().expect("source work should exist"),
        );
        assert_eq!(
            target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::RetainedQueries)
        );
        assert_eq!(target.exact_queries, vec![query]);
        assert_eq!(target.structural_boundary_cleanup_roots, vec![1, 3, 4]);
        assert_eq!(target.exact_safety_fallback_roots, vec![1, 3, 4]);
        assert!(target.reasoned_fallback_roots.is_empty());
        assert!(target.fallback_reasons.is_empty());
    }

    #[test]
    fn moli_source_dependency_batch_plan_tries_retained_scope_before_source_fallback() {
        #[derive(Default)]
        struct ContextRootsProviderForTest {
            calls: usize,
        }

        impl MoliSourceDependencyInvalidationContextRootsProvider<u32>
            for ContextRootsProviderForTest
        {
            fn context_roots_for_source_dependency(
                &mut self,
                root: u32,
                _plan: MoliDependencyContextRootPlan,
                context: MoliDependencyInvalidationFallbackContext<u32>,
            ) -> MoliDependencyInvalidationContextRoots<u32> {
                self.calls += 1;
                assert_eq!(root, 1);
                assert_eq!(context.parent(), Some(2));
                assert_eq!(context.previous_sibling(), Some(3));
                assert_eq!(context.next_sibling(), Some(4));
                MoliDependencyInvalidationContextRoots::new(true, vec![10])
            }
        }

        let mut dependency = MoliDependencyQueryResult::default();
        dependency.add_kind(MoliDependencyKind::Scope);
        let mut dependency_summary = MoliDependencyInvalidationSummary::default();
        dependency_summary.note_class_dependency(Atom::from("scoped"), dependency);
        let source_summary = MoliSourceDependencySummary::from_parts(
            dependency_summary,
            false,
            MoliChildListStructuralBoundaryDependencySummary::default(),
        );
        let source_roots = [99_u32];
        let source = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &source_roots,
            &[],
        );
        let query = MoliRetainedStyleInvalidationQuery::class(1_u32, "scoped".into());
        let context = MoliDependencyInvalidationFallbackContext::from_mutation_relation(
            Some(2),
            Some(3),
            Some(4),
        );
        let request = MoliSourceDependencyInvalidationRequest::new(
            &query,
            Some(context),
            MoliSourceDependencyRequestRequirement::exact(),
        );
        let mut provider = ContextRootsProviderForTest::default();

        let plan = moli_source_dependency_invalidation_batch_plan(
            &[source],
            &[request],
            MoliSourceDependencyBoundaryRoots::default(),
            &mut provider,
        );

        assert_eq!(provider.calls, 1);
        let mut plan = source_dependency_batch_plan_for_test(plan);
        assert!(plan.work_boundary_fallback.is_none());
        assert!(plan.requires_source_fallback.is_none());
        let sources = &mut plan.work_sources;
        assert_eq!(sources.len(), 1);
        let target = planned_source_dependency_parts_for_test(
            sources.pop().expect("source work should exist"),
        );
        assert_eq!(
            target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::RetainedQueries)
        );
        assert_eq!(
            target.fallback_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(target.exact_queries, vec![query.clone()]);
        assert_eq!(target.exact_safety_fallback_roots, vec![99]);
        assert!(target.reasoned_fallback_roots.is_empty());
        assert!(target.fallback_reasons.is_empty());

        let no_source_roots = [];
        let source_without_fallback_roots = MoliSourceDependencyInvalidationBatchSource::new(
            &source_summary,
            &no_source_roots,
            &[],
        );
        let request_without_fallback_roots = MoliSourceDependencyInvalidationRequest::new(
            &query,
            Some(MoliDependencyInvalidationFallbackContext::from_mutation_relation(
                Some(2),
                Some(3),
                Some(4),
            )),
            MoliSourceDependencyRequestRequirement::exact(),
        );
        let mut provider = ContextRootsProviderForTest::default();
        let plan = moli_source_dependency_invalidation_batch_plan(
            &[source_without_fallback_roots],
            &[request_without_fallback_roots],
            MoliSourceDependencyBoundaryRoots::default(),
            &mut provider,
        );

        assert_eq!(provider.calls, 1);
        let mut plan = source_dependency_batch_plan_for_test(plan);
        assert!(plan.work_sources.is_empty());
        let target = planned_source_dependency_parts_for_test(
            plan.requires_source_fallback
                .take()
                .expect("scope without safety roots must stay conservative"),
        );
        assert_eq!(
            target.target_kind,
            Some(MoliRetainedSourceStyleInvalidationKind::MissingFallbackRoots)
        );
        assert!(target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::ScopeDependency));
        assert!(target
            .fallback_reasons
            .contains(&MoliSourceInvalidationFallbackReason::MissingFallbackRoots));
    }

    #[derive(Default)]
    struct MoliSourceResultKindSummaryForTest {
        retained_source_unavailable_target_count: usize,
        source_scope_fallback_target_count: usize,
        context_fallback_target_count: usize,
    }

    impl MoliSourceStyleInvalidationSourceResultKindSummarySink
        for MoliSourceResultKindSummaryForTest
    {
        fn record_retained_source_unavailable_target(&mut self) {
            self.retained_source_unavailable_target_count += 1;
        }

        fn record_source_scope_fallback_target(&mut self) {
            self.source_scope_fallback_target_count += 1;
        }

        fn record_context_fallback_target(&mut self) {
            self.context_fallback_target_count += 1;
        }
    }

    #[test]
    fn moli_source_result_kind_records_summary_categories() {
        let mut summary = MoliSourceResultKindSummaryForTest::default();

        MoliSourceStyleInvalidationSourceResultKind::Exact.record_summary_into(&mut summary);
        MoliSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem
            .record_summary_into(&mut summary);
        MoliSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData
            .record_summary_into(&mut summary);
        MoliSourceStyleInvalidationSourceResultKind::SourceScopeFallback
            .record_summary_into(&mut summary);
        MoliSourceStyleInvalidationSourceResultKind::ContextFallback
            .record_summary_into(&mut summary);

        assert_eq!(summary.retained_source_unavailable_target_count, 2);
        assert_eq!(summary.source_scope_fallback_target_count, 1);
        assert_eq!(summary.context_fallback_target_count, 1);
    }

    #[derive(Default)]
    struct MoliFallbackRootAvailabilitySummaryForTest {
        missing_fallback_roots_target_count: usize,
    }

    impl MoliSourceFallbackRootAvailabilitySummarySink
        for MoliFallbackRootAvailabilitySummaryForTest
    {
        fn record_missing_fallback_roots_target(&mut self) {
            self.missing_fallback_roots_target_count += 1;
        }
    }

    #[test]
    fn moli_fallback_root_availability_records_missing_summary() {
        let mut summary = MoliFallbackRootAvailabilitySummaryForTest::default();

        MoliSourceFallbackRootAvailability::Available { root_count: 1 }
            .record_summary_into(&mut summary);
        MoliSourceFallbackRootAvailability::Missing.record_summary_into(&mut summary);

        assert_eq!(summary.missing_fallback_roots_target_count, 1);
    }

    #[derive(Default)]
    struct MoliSourceStyleInvalidationResultPartsForTest {
        affected_roots: Vec<u32>,
        fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
        fallback_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
        fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
    }

    impl MoliSourceStyleInvalidationResultSink<u32>
        for MoliSourceStyleInvalidationResultPartsForTest
    {
        fn set_source_style_invalidation_result(
            &mut self,
            parts: MoliSourceStyleInvalidationResultParts<u32>,
        ) {
            parts.drain_into(self);
        }
    }

    impl MoliSourceStyleInvalidationResultPartsSink<u32>
        for MoliSourceStyleInvalidationResultPartsForTest
    {
        fn set_source_style_invalidation_result_parts(
            &mut self,
            affected_roots: Vec<u32>,
            fallback_reasons: IndexSet<MoliSourceInvalidationFallbackReason>,
            fallback_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
            fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
            empty_result_is_exact: bool,
            matched_dependency_count: usize,
        ) {
            self.affected_roots = affected_roots;
            self.fallback_reasons = fallback_reasons;
            self.fallback_kind = fallback_kind;
            self.fallback_root_availability = fallback_root_availability;
            self.empty_result_is_exact = empty_result_is_exact;
            self.matched_dependency_count = matched_dependency_count;
        }
    }

    fn source_style_invalidation_result_parts_for_test(
        result: MoliSourceStyleInvalidationResult<u32>,
    ) -> MoliSourceStyleInvalidationResultPartsForTest {
        let mut sink = MoliSourceStyleInvalidationResultPartsForTest::default();
        result.drain_into(&mut sink);
        sink
    }

    #[test]
    fn moli_source_result_accumulator_reports_missing_fallback_roots() {
        let mut accumulated = MoliSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_query_result(
            Vec::<u32>::new(),
            true,
            1,
            IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector]),
        );

        let result = source_style_invalidation_result_parts_for_test(
            accumulated.into_source_result(&IndexSet::new()),
        );

        assert!(result.affected_roots.is_empty());
        assert_eq!(
            result.fallback_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::MissingFallbackRoots)
        );
        assert_eq!(
            result.fallback_root_availability,
            Some(MoliSourceFallbackRootAvailability::Missing)
        );
        assert!(result.empty_result_is_exact);
        assert_eq!(result.matched_dependency_count, 1);
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([
                MoliSourceInvalidationFallbackReason::FullSelector,
                MoliSourceInvalidationFallbackReason::MissingFallbackRoots,
            ])
        );
    }

    #[test]
    fn moli_query_result_merge_preserves_ordered_roots_and_reasons() {
        let first = MoliSourceStyleInvalidationQueryResult::from_parts(
            vec![1, 2],
            true,
            2,
            [MoliSourceInvalidationFallbackReason::FullSelector],
        );
        let second = MoliSourceStyleInvalidationQueryResult::from_parts(
            vec![2, 3],
            false,
            1,
            [
                MoliSourceInvalidationFallbackReason::FullSelector,
                MoliSourceInvalidationFallbackReason::RelativeAnySelector,
            ],
        );

        let merged = moli_merge_source_style_invalidation_query_results(first, second);
        let MoliSourceStyleInvalidationQueryResult {
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons,
        } = merged;

        assert_eq!(affected_roots, vec![1, 2, 3]);
        assert!(!empty_result_is_exact);
        assert_eq!(matched_dependency_count, 3);
        assert_eq!(
            fallback_reasons,
            IndexSet::from([
                MoliSourceInvalidationFallbackReason::FullSelector,
                MoliSourceInvalidationFallbackReason::RelativeAnySelector,
            ])
        );
    }

    #[test]
    fn moli_query_result_builder_preserves_roots_exactness_and_reasons() {
        let mut builder = MoliSourceStyleInvalidationQueryResultBuilder::new();
        builder.note_affected_root(1);
        builder.note_affected_root(2);
        builder.note_affected_root(1);
        builder.note_empty_result_supported(true);
        builder.note_empty_result_supported(false);
        builder.note_fallback_reason(MoliSourceInvalidationFallbackReason::FullSelector);
        builder.note_fallback_reason(MoliSourceInvalidationFallbackReason::FullSelector);

        let result = builder.into_query_result(3);
        let MoliSourceStyleInvalidationQueryResult {
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons,
        } = result;

        assert_eq!(affected_roots, vec![1, 2]);
        assert!(!empty_result_is_exact);
        assert_eq!(matched_dependency_count, 3);
        assert_eq!(
            fallback_reasons,
            IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector])
        );
    }

    #[test]
    fn moli_query_result_drains_affected_roots() {
        let result = MoliSourceStyleInvalidationQueryResult::from_parts(
            vec![1, 2, 1],
            true,
            2,
            [MoliSourceInvalidationFallbackReason::FullSelector],
        );
        let mut roots = IndexSet::new();

        result.drain_affected_roots_into(&mut roots);

        assert_eq!(roots, IndexSet::from([1, 2]));
    }

    #[test]
    fn moli_snapshot_relative_roots_classify_empty_exactness() {
        let verified = MoliSnapshotRelativeDependencyRoots::<u32>::new(Vec::new(), 2);

        assert!(verified.verified_all_dependencies(2, 2));
        assert!(verified.verified_all_collected_dependencies(2));
        assert!(verified.empty_result_is_exact(2, 2, false));
        assert!(!verified.empty_result_is_exact(3, 2, false));
        assert!(verified.empty_result_is_exact(0, 2, false));

        let rooted = MoliSnapshotRelativeDependencyRoots::new(vec![1], 0);
        assert_eq!(rooted.roots(), &[1]);
        assert!(rooted.empty_result_is_exact(1, 2, true));
    }

    #[test]
    fn moli_normal_invalidation_dependency_plan_classifies_relative_filtering() {
        let no_snapshot_roots = MoliSnapshotRelativeDependencyRoots::<u32>::default();

        let custom_state = moli_normal_style_invalidation_dependency_plan(
            MoliStyleInvalidationQuery::CustomState("expanded"),
            1,
            1,
            &no_snapshot_roots,
        );
        assert!(custom_state.should_drop_relative_dependencies());
        assert!(custom_state.empty_result_is_exact());

        let verified_relative_roots =
            MoliSnapshotRelativeDependencyRoots::<u32>::new(Vec::new(), 2);
        let mixed_dependencies = moli_normal_style_invalidation_dependency_plan(
            MoliStyleInvalidationQuery::Class("active"),
            3,
            2,
            &verified_relative_roots,
        );
        assert!(mixed_dependencies.should_drop_relative_dependencies());
        assert!(!mixed_dependencies.empty_result_is_exact());

        let rooted_snapshot = MoliSnapshotRelativeDependencyRoots::new(vec![1_u32], 1);
        let rooted_plan = moli_normal_style_invalidation_dependency_plan(
            MoliStyleInvalidationQuery::Class("active"),
            1,
            1,
            &rooted_snapshot,
        );
        assert!(rooted_plan.should_drop_relative_dependencies());
        assert!(!rooted_plan.empty_result_is_exact());

        let unsupported_relative = moli_normal_style_invalidation_dependency_plan(
            MoliStyleInvalidationQuery::Class("active"),
            1,
            1,
            &no_snapshot_roots,
        );
        assert!(!unsupported_relative.should_drop_relative_dependencies());
        assert!(!unsupported_relative.empty_result_is_exact());
    }

    #[test]
    fn moli_relative_invalidation_dependency_plan_classifies_empty_exactness() {
        let no_snapshot_roots = MoliSnapshotRelativeDependencyRoots::<u32>::default();

        let no_dependencies =
            moli_relative_style_invalidation_dependency_plan(0, 0, false, &no_snapshot_roots);
        assert!(no_dependencies.empty_result_is_exact());

        let affected_roots =
            moli_relative_style_invalidation_dependency_plan(2, 2, true, &no_snapshot_roots);
        assert!(affected_roots.empty_result_is_exact());

        let verified_snapshot_dependencies =
            MoliSnapshotRelativeDependencyRoots::<u32>::new(Vec::new(), 2);
        let verified = moli_relative_style_invalidation_dependency_plan(
            2,
            2,
            false,
            &verified_snapshot_dependencies,
        );
        assert!(verified.empty_result_is_exact());

        let unsupported_relative = moli_relative_style_invalidation_dependency_plan(
            2,
            1,
            false,
            &verified_snapshot_dependencies,
        );
        assert!(!unsupported_relative.empty_result_is_exact());
    }

    #[test]
    fn moli_relative_invalidation_query_result_merges_direct_and_snapshot_roots() {
        let snapshot_roots = MoliSnapshotRelativeDependencyRoots::new(vec![2_u32, 3], 1);

        let result =
            moli_relative_style_invalidation_query_result(vec![1, 2], &snapshot_roots, 2, 1);
        let MoliSourceStyleInvalidationQueryResult {
            affected_roots,
            empty_result_is_exact,
            matched_dependency_count,
            fallback_reasons,
        } = result;

        assert_eq!(affected_roots, vec![1, 2, 3]);
        assert!(empty_result_is_exact);
        assert_eq!(matched_dependency_count, 2);
        assert!(fallback_reasons.is_empty());

        let no_snapshot_roots = MoliSnapshotRelativeDependencyRoots::<u32>::default();
        let unsupported_empty =
            moli_relative_style_invalidation_query_result([], &no_snapshot_roots, 2, 1);
        assert!(!unsupported_empty.empty_result_is_exact);

        let no_dependencies =
            moli_relative_style_invalidation_query_result([], &no_snapshot_roots, 0, 0);
        assert!(no_dependencies.empty_result_is_exact);
    }

    #[test]
    fn moli_source_result_accumulator_consumes_typed_query_result() {
        let mut accumulated = MoliSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_invalidation_query_result(
            MoliSourceStyleInvalidationQueryResult::from_parts(
                vec![1],
                true,
                1,
                [MoliSourceInvalidationFallbackReason::FullSelector],
            ),
        );

        let result = source_style_invalidation_result_parts_for_test(
            accumulated.into_source_result(&IndexSet::from([2])),
        );

        assert_eq!(result.affected_roots, vec![2]);
        assert_eq!(
            result.fallback_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::Fallback)
        );
        assert_eq!(
            result.fallback_root_availability,
            Some(MoliSourceFallbackRootAvailability::Available { root_count: 1 })
        );
        assert!(result.empty_result_is_exact);
        assert_eq!(result.matched_dependency_count, 1);
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector])
        );
    }

    #[test]
    fn moli_source_result_accumulator_uses_exact_safety_roots_for_fallback() {
        let mut accumulated = MoliSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_query_result(
            vec![1],
            true,
            1,
            IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector]),
        );

        let result = source_style_invalidation_result_parts_for_test(
            accumulated.into_source_result(&IndexSet::from([2])),
        );

        assert_eq!(result.affected_roots, vec![2]);
        assert_eq!(
            result.fallback_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::Fallback)
        );
        assert_eq!(
            result.fallback_root_availability,
            Some(MoliSourceFallbackRootAvailability::Available { root_count: 1 })
        );
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector])
        );
    }

    #[test]
    fn moli_source_result_accumulator_accepts_proven_exact_zero_match_result() {
        let mut accumulated = MoliSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_query_result(Vec::<u32>::new(), true, 0, IndexSet::new());

        let result = source_style_invalidation_result_parts_for_test(
            accumulated.into_source_result_with_zero_matched_dependency_exactness(
                &IndexSet::from([1]),
                true,
            ),
        );

        assert!(result.affected_roots.is_empty());
        assert_eq!(result.fallback_kind, None);
        assert!(result.fallback_reasons.is_empty());
        assert!(result.empty_result_is_exact);
        assert_eq!(result.matched_dependency_count, 0);
    }

    #[test]
    fn moli_source_result_accumulator_keeps_zero_match_without_source_proof_as_fallback() {
        let mut accumulated = MoliSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_query_result(Vec::<u32>::new(), true, 0, IndexSet::new());

        let result = source_style_invalidation_result_parts_for_test(
            accumulated.into_source_result(&IndexSet::from([1])),
        );

        assert_eq!(result.affected_roots, vec![1]);
        assert_eq!(
            result.fallback_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::Fallback)
        );
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([MoliSourceInvalidationFallbackReason::InexactEmptyResult])
        );
    }

    #[test]
    fn moli_source_result_accumulator_keeps_unproven_empty_result_as_fallback() {
        let mut accumulated = MoliSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_query_result(Vec::<u32>::new(), false, 0, IndexSet::new());

        let result = source_style_invalidation_result_parts_for_test(
            accumulated.into_source_result(&IndexSet::from([1])),
        );

        assert_eq!(result.affected_roots, vec![1]);
        assert_eq!(
            result.fallback_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::Fallback)
        );
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([MoliSourceInvalidationFallbackReason::InexactEmptyResult])
        );
    }

    #[derive(Default)]
    struct MoliSourceResultDrainForTest {
        source_result_count: Option<usize>,
        source_index: Option<usize>,
        exact_roots: Vec<u32>,
        source_fallback_roots: Vec<u32>,
        diagnostic_kind: Option<MoliSourceStyleInvalidationSourceResultKind>,
        diagnostic_fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        diagnostic_fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
        cleanup_clear_all_reasons: Vec<MoliSourceInvalidationFallbackReason>,
        cleanup_includes_fallback_context_for_clear_all: bool,
    }

    impl MoliInvalidationSourceResultsSink<u32> for MoliSourceResultDrainForTest {
        fn record_moli_invalidation_source_result_count(&mut self, count: usize) {
            self.source_result_count = Some(count);
        }

        fn record_moli_invalidation_source_result(
            &mut self,
            result: MoliSourceStyleInvalidationSourceResult<u32>,
        ) {
            result.drain_into(self);
        }
    }

    impl MoliSourceStyleInvalidationSourceResultSink<u32> for MoliSourceResultDrainForTest {
        fn record_source_style_invalidation_source_result(
            &mut self,
            parts: MoliSourceStyleInvalidationSourceResultParts<u32>,
        ) {
            parts.drain_into(self);
        }
    }

    impl MoliSourceStyleInvalidationSourceResultPartsSink<u32>
        for MoliSourceResultDrainForTest
    {
        fn record_source_style_invalidation_source_result_parts(
            &mut self,
            source_index: usize,
            affected_roots: MoliSourceAffectedRootsCleanup<u32>,
            target_result_record: MoliSourceStyleInvalidationTargetResultRecord,
        ) {
            self.source_index = Some(source_index);
            affected_roots.drain_into(self);
            if let Some(diagnostic_facts) = target_result_record.drain_cleanup_into(self) {
                diagnostic_facts.drain_into(self);
            }
        }
    }

    impl MoliSourceAffectedRootsCleanupSink<u32> for MoliSourceResultDrainForTest {
        fn extend_exact_affected_roots(&mut self, roots: &[u32]) {
            self.exact_roots.extend(roots.iter().copied());
        }

        fn extend_source_fallback_roots(&mut self, roots: &[u32]) {
            self.source_fallback_roots.extend(roots.iter().copied());
        }
    }

    impl MoliSourceStyleInvalidationTargetResultCleanupFactsSink
        for MoliSourceResultDrainForTest
    {
        fn set_source_style_invalidation_target_result_cleanup_facts(
            &mut self,
            facts: MoliSourceStyleInvalidationTargetResultCleanupFacts,
        ) {
            facts.drain_parts_into(self);
        }
    }

    impl MoliSourceStyleInvalidationTargetResultCleanupFactsPartsSink
        for MoliSourceResultDrainForTest
    {
        fn set_source_style_invalidation_target_result_cleanup_fact_parts(
            &mut self,
            _fallback_context_reasons: Vec<MoliSourceInvalidationFallbackReason>,
            clear_all_cleanup_reasons: Vec<MoliSourceInvalidationFallbackReason>,
            include_fallback_context_for_clear_all: bool,
            _requires_fallback_handling: bool,
        ) {
            self.cleanup_clear_all_reasons = clear_all_cleanup_reasons;
            self.cleanup_includes_fallback_context_for_clear_all =
                include_fallback_context_for_clear_all;
        }
    }

    impl MoliSourceStyleInvalidationTargetResultDiagnosticFactsSink
        for MoliSourceResultDrainForTest
    {
        fn set_source_style_invalidation_target_result_diagnostic_facts(
            &mut self,
            facts: MoliSourceStyleInvalidationTargetResultDiagnosticFacts,
        ) {
            facts.drain_parts_into(self);
        }
    }

    impl MoliSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink
        for MoliSourceResultDrainForTest
    {
        fn set_source_style_invalidation_target_result_diagnostic_fact_parts(
            &mut self,
            kind: MoliSourceStyleInvalidationSourceResultKind,
            _exact: bool,
            _empty_result_is_exact: bool,
            _matched_dependency_count: usize,
            fallback_reasons: Vec<MoliSourceInvalidationFallbackReason>,
            fallback_root_availability: Option<MoliSourceFallbackRootAvailability>,
            _affected_root_count: usize,
        ) {
            self.diagnostic_kind = Some(kind);
            self.diagnostic_fallback_reasons = fallback_reasons;
            self.diagnostic_fallback_root_availability = fallback_root_availability;
        }
    }

    #[test]
    fn moli_source_result_drains_unavailable_retained_policy() {
        let result = MoliSourceStyleInvalidationSourceResult::unavailable_retained_source(
            3,
            MoliSourceInvalidationFallbackReason::MissingRetainedCascadeData,
            &IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector]),
            &IndexSet::from([1]),
            &IndexSet::from([2]),
        );
        let mut sink = MoliSourceResultDrainForTest::default();

        result.drain_into(&mut sink);

        assert_eq!(sink.source_index, Some(3));
        assert!(sink.exact_roots.is_empty());
        assert_eq!(sink.source_fallback_roots, vec![1, 2]);
        assert_eq!(
            sink.diagnostic_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData)
        );
        assert_eq!(
            sink.diagnostic_fallback_reasons,
            vec![
                MoliSourceInvalidationFallbackReason::FullSelector,
                MoliSourceInvalidationFallbackReason::MissingRetainedCascadeData,
            ]
        );
        assert_eq!(
            sink.diagnostic_fallback_root_availability,
            Some(MoliSourceFallbackRootAvailability::Available { root_count: 2 })
        );
        assert!(sink.cleanup_clear_all_reasons.is_empty());
        assert!(sink.cleanup_includes_fallback_context_for_clear_all);
    }

    #[test]
    fn moli_source_result_drains_missing_roots_clear_all_policy() {
        let result = MoliSourceStyleInvalidationSourceResult::fallback(
            0,
            MoliSourceStyleInvalidationSourceResultKind::MissingFallbackRoots,
            false,
            1,
            vec![MoliSourceInvalidationFallbackReason::FullSelector],
            Some(MoliSourceFallbackRootAvailability::Missing),
            Vec::<u32>::new(),
        );
        let mut sink = MoliSourceResultDrainForTest::default();

        result.drain_into(&mut sink);

        assert_eq!(
            sink.cleanup_clear_all_reasons,
            vec![
                MoliSourceInvalidationFallbackReason::FullSelector,
                MoliSourceInvalidationFallbackReason::MissingFallbackRoots,
            ]
        );
    }

    #[test]
    fn moli_invalidation_result_drains_source_result_table() {
        let result = MoliInvalidationResult::from_source_results(vec![
            MoliSourceStyleInvalidationSourceResult::exact_result(0, vec![7], true, 1),
        ]);
        let mut sink = MoliSourceResultDrainForTest::default();

        result.drain_source_results_into(&mut sink);

        assert_eq!(sink.source_result_count, Some(1));
        assert_eq!(sink.source_index, Some(0));
        assert_eq!(sink.exact_roots, vec![7]);
        assert!(sink.source_fallback_roots.is_empty());
    }

    #[test]
    fn moli_invalidation_result_builder_builds_source_result_table() {
        let mut builder = MoliInvalidationResultBuilder::new();
        builder.push_missing_retained_style_system_source(
            2,
            &IndexSet::from([MoliSourceInvalidationFallbackReason::FullSelector]),
            &IndexSet::from([3]),
            &IndexSet::from([4]),
        );
        let result = builder.finish();
        let mut sink = MoliSourceResultDrainForTest::default();

        result.drain_source_results_into(&mut sink);

        assert_eq!(sink.source_result_count, Some(1));
        assert_eq!(sink.source_index, Some(2));
        assert_eq!(sink.source_fallback_roots, vec![3, 4]);
        assert_eq!(
            sink.diagnostic_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem)
        );
        assert_eq!(
            sink.diagnostic_fallback_root_availability,
            Some(MoliSourceFallbackRootAvailability::Available { root_count: 2 })
        );

        let mut builder = MoliInvalidationResultBuilder::new();
        builder.push_missing_retained_cascade_data_source(
            5,
            &IndexSet::new(),
            &IndexSet::new(),
            &IndexSet::from([6]),
        );
        let result = builder.finish();
        let mut sink = MoliSourceResultDrainForTest::default();

        result.drain_source_results_into(&mut sink);

        assert_eq!(sink.source_result_count, Some(1));
        assert_eq!(sink.source_index, Some(5));
        assert_eq!(sink.source_fallback_roots, vec![6]);
        assert_eq!(
            sink.diagnostic_kind,
            Some(MoliSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData)
        );
    }

    #[test]
    fn moli_dependency_processor_support_rejects_unsupported_shapes() {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let selector = SelectorParser::parse_author_origin_no_namespace(".subject", &url_data)
            .expect("selector should parse")
            .slice()[0]
            .clone();
        let dependency_for_kind = |kind| Dependency::new(selector.clone(), 0, None, kind);

        let normal = dependency_for_kind(DependencyInvalidationKind::Normal(
            NormalDependencyInvalidationKind::ElementAndDescendants,
        ));
        assert!(moli_dependency_supported_by_retained_processor(
            &normal
        ));
        assert!(moli_dependency_empty_result_supported_by_retained_processor(&normal));

        let scope = dependency_for_kind(DependencyInvalidationKind::Scope(
            ScopeDependencyInvalidationKind::ScopeEnd,
        ));
        assert!(moli_dependency_supported_by_retained_processor(
            &scope
        ));
        assert!(moli_dependency_empty_result_supported_by_retained_processor(&scope));

        let full = dependency_for_kind(DependencyInvalidationKind::FullSelector);
        assert!(!moli_dependency_supported_by_retained_processor(
            &full
        ));
        assert!(!moli_dependency_empty_result_supported_by_retained_processor(&full));

        let relative = dependency_for_kind(DependencyInvalidationKind::Relative(
            RelativeDependencyInvalidationKind::Ancestors,
        ));
        assert!(!moli_dependency_supported_by_retained_processor(
            &relative
        ));
        assert!(!moli_dependency_empty_result_supported_by_retained_processor(&relative));

        let full_next = ThinArc::from_header_and_iter(
            (),
            [dependency_for_kind(
                DependencyInvalidationKind::FullSelector,
            )]
            .into_iter(),
        );
        let normal_with_unsupported_next = Dependency::new(
            selector,
            0,
            Some(full_next),
            DependencyInvalidationKind::Normal(NormalDependencyInvalidationKind::Element),
        );
        assert!(!moli_dependency_supported_by_retained_processor(
            &normal_with_unsupported_next
        ));
        assert!(
            !moli_dependency_empty_result_supported_by_retained_processor(
                &normal_with_unsupported_next
            )
        );
    }

    #[test]
    fn moli_retained_processor_dependency_effect_classifies_dependency() {
        let url_data = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let selector = SelectorParser::parse_author_origin_no_namespace(".subject", &url_data)
            .expect("selector should parse")
            .slice()[0]
            .clone();
        let dependency_for_kind = |kind| Dependency::new(selector.clone(), 0, None, kind);

        let normal = dependency_for_kind(DependencyInvalidationKind::Normal(
            NormalDependencyInvalidationKind::Element,
        ));
        assert_eq!(
            moli_retained_processor_dependency_effect(&normal),
            MoliRetainedProcessorDependencyEffect::Retained {
                empty_result_is_exact: true
            }
        );

        let full = dependency_for_kind(DependencyInvalidationKind::FullSelector);
        assert_eq!(
            moli_retained_processor_dependency_effect(&full),
            MoliRetainedProcessorDependencyEffect::Fallback(
                MoliSourceInvalidationFallbackReason::FullSelector
            )
        );
    }

    #[test]
    fn moli_relative_summary_does_not_treat_used_flag_as_unknown_dependency() {
        let mut map = AdditionalRelativeSelectorInvalidationMap::new();
        map.used = true;

        let summary = moli_dependency_summary_for_relative_invalidation_map(&map);

        assert!(!summary.has_unknown_dependency());
        assert!(!summary.query_universal().has_any_dependency());
    }

    #[test]
    fn moli_relative_summary_keeps_ancestor_traversal_unknown() {
        let mut map = AdditionalRelativeSelectorInvalidationMap::new();
        map.needs_ancestors_traversal = true;

        let summary = moli_dependency_summary_for_relative_invalidation_map(&map);

        assert!(summary.has_unknown_dependency());
    }

    #[test]
    fn moli_nth_of_dependencies_are_sibling_sensitive_by_key() {
        let mut summary = MoliDependencyInvalidationSummary::default();
        let class = Atom::from("c");
        let other_class = Atom::from("other");
        let id = Atom::from("target");
        let attribute = LocalName::from("data-active");
        let custom_state = AtomIdent::from("--active");

        summary.note_nth_of_class_dependency(class.clone());
        summary.note_nth_of_id_dependency(id.clone());
        summary.note_nth_of_attribute_dependency(attribute.clone());
        summary.note_nth_of_custom_state_dependency(custom_state.clone());
        summary.note_nth_of_state_dependency(ElementState::FOCUS);
        summary.note_nth_of_state_dependency(ElementState::empty());

        assert!(!summary.has_unknown_dependency());
        let class_result = summary.query_class(&class);
        assert!(class_result.requires_fallback());
        assert_eq!(class_result.kinds(), &[MoliDependencyKind::Siblings]);
        assert_eq!(
            class_result.fallback_reasons(),
            &[MoliDependencyFallbackReason::NthOfDependency]
        );
        assert_eq!(
            class_result.fallback_root_policy(),
            MoliDependencyFallbackRootPolicy::ContextRoots
        );
        assert!(!summary.query_class(&other_class).has_any_dependency());
        assert_eq!(
            summary.query_id(&id).kinds(),
            &[MoliDependencyKind::Siblings]
        );
        assert_eq!(
            summary.query_attribute(&attribute).kinds(),
            &[MoliDependencyKind::Siblings]
        );
        assert_eq!(
            summary.query_custom_state(&custom_state).fallback_reasons(),
            &[MoliDependencyFallbackReason::NthOfDependency]
        );
        assert_eq!(
            summary.query_focus().kinds(),
            &[MoliDependencyKind::Siblings]
        );
    }

    #[test]
    fn moli_summary_marks_nested_relative_selector_lists_for_fallback() {
        let summary = moli_dependency_summary_for_selector(
            "#target:has(:is(.item + .item + .item > .child + .child + .child))",
        );

        let item_result = summary.query_class(&Atom::from("item"));
        assert!(item_result.has_any_dependency());
        assert!(item_result.requires_fallback());
        assert!(item_result
            .fallback_reasons()
            .contains(&MoliDependencyFallbackReason::NestedRelativeSelectorDependency));
        assert_eq!(
            item_result.fallback_root_policy(),
            MoliDependencyFallbackRootPolicy::ContextRoots
        );

        let child_result = summary.query_class(&Atom::from("child"));
        assert!(child_result.has_any_dependency());
        assert!(child_result.requires_fallback());
        assert!(child_result
            .fallback_reasons()
            .contains(&MoliDependencyFallbackReason::NestedRelativeSelectorDependency));
        assert_eq!(
            child_result.fallback_root_policy(),
            MoliDependencyFallbackRootPolicy::ContextRoots
        );
    }

    #[test]
    fn moli_summary_exposes_link_pseudos_as_href_attribute_dependencies() {
        let summary = moli_dependency_summary_for_selector("#target:has(:any-link)");
        let href = LocalName::from("href");

        assert!(summary.query_attribute(&href).has_any_dependency());
        assert!(!summary
            .query_attribute(&LocalName::from("class"))
            .has_any_dependency());
    }
}
