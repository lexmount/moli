use std::collections::{HashMap, HashSet};

use dom::{ElementState, HEADING_LEVEL_OFFSET};
use indexmap::{IndexMap, IndexSet};
use selectors::{
    OpaqueElement,
    matching::{
        MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, SelectorCaches,
    },
};
use style::{
    LocalName, Namespace, Prefix,
    context::QuirksMode,
    dom::{TDocument, TElement, TNode},
    invalidation::element::{
        invalidation_map::Dependency,
        invalidator::{SiblingTraversalMap, TreeStyleInvalidator},
    },
    moli_invalidation::{
        MoliChildListSiblingBoundaryKind, MoliDependencyContextRootFlagsSink,
        MoliDependencyContextRootPlan, MoliDependencyInvalidationContextRoots,
        MoliDependencyInvalidationContextRootsPartsSink,
        MoliDependencyInvalidationContextRootsSink, MoliDependencyInvalidationFallbackContext,
        MoliElementDependencySnapshot, MoliInvalidationResult as ForkMoliInvalidationResult,
        MoliInvalidationResultBuilder as ForkMoliInvalidationResultBuilder,
        MoliNormalStyleInvalidationDependencyPlanSink, MoliPlannedFallbackRootInvalidationTarget,
        MoliPlannedSourceDependencyInvalidation, MoliPlannedSourceDependencyInvalidationTarget,
        MoliRetainedSourceStyleInvalidation, MoliRetainedSourceStyleInvalidationKind,
        MoliRetainedSourceStyleInvalidationSink, MoliRetainedStyleChildListInvalidationQueries,
        MoliRetainedStyleChildListInvalidationQuery,
        MoliRetainedStyleChildListInvalidationQueryBuilder,
        MoliRetainedStyleChildListMutationContext, MoliRetainedStyleInvalidationQuery,
        MoliRetainedStyleSiblingTraversal, MoliRuntimeFallbackRootInput,
        MoliRuntimeFallbackRootResolver, MoliSnapshotRelativeDependencyRoots,
        MoliSourceAffectedRootsCleanup, MoliSourceDependencyBoundaryRoots,
        MoliSourceDependencyInvalidationBatchPlan, MoliSourceDependencyInvalidationBatchSource,
        MoliSourceDependencyInvalidationContextRootsProvider,
        MoliSourceDependencyInvalidationRequest, MoliSourceDependencyRequestRequirement,
        MoliSourceDependencySummary, MoliSourceFallbackRootAvailability,
        MoliSourceInvalidationFallbackReason, MoliSourceStyleInvalidationQuery,
        MoliSourceStyleInvalidationQueryResult, MoliSourceStyleInvalidationResult,
        MoliSourceStyleInvalidationResultAccumulator, MoliSourceStyleInvalidationSourceResult,
        MoliSourceStyleInvalidationSourceResultKind, MoliStateInvalidationRoot,
        MoliStyleInvalidationElementRoot,
        MoliStyleInvalidationProcessor as ForkMoliStyleInvalidationProcessor,
        MoliStyleInvalidationQuery, MoliStyleInvalidationSnapshot,
        MoliStyleInvalidationSnapshotAttribute,
        MoliStyleMutationElementSnapshot as ForkMoliStyleMutationElementSnapshot,
        MoliStylesheetSourceScopeFallbackInput, MoliStylesheetSourceScopeFallbackRootsResolver,
        moli_child_list_dependency_fallback_context_for_query,
        moli_child_list_sibling_boundary_plan, moli_collect_dependencies_from_invalidation_map,
        moli_collect_relative_style_invalidation_query_result,
        moli_dependency_is_relative_selector, moli_fallback_roots_plan,
        moli_normal_style_invalidation_dependency_plan, moli_relative_dependency_changes_anchor,
        moli_retained_non_universal_queries_for_element_dependency_snapshot,
        moli_retained_queries_for_element_dependency_snapshot,
        moli_runtime_fallback_roots_for_mutation_inputs,
        moli_runtime_or_source_scope_fallback_plan,
        moli_snapshot_relative_outer_dependency_supported,
        moli_source_dependency_invalidation_batch_plan, moli_source_scope_fallback_plan,
        moli_stylesheet_source_scope_fallback_roots, moli_visit_relative_dependency_candidates,
    },
    selector_parser::{Snapshot, SnapshotMap},
    servo::attr::{AttrIdentifier, AttrValue},
    servo_arc::Arc as ServoArc,
    stylist::{CascadeData, Stylist},
    values::AtomIdent,
};

use crate::{
    dom::{
        NodeId,
        native::{ConnectedShadowRootSnapshot, DomHost, Element},
    },
    stylo::style_traversal::{StyleDomHostBinding, StyleElement},
};

pub use style::moli_invalidation::{
    MoliInvalidationSourceResultsSink,
    MoliRetainedSourceStyleInvalidationSink as StyloRetainedSourceStyleInvalidationSink,
    MoliSourceAffectedRootsCleanupSink as StyloSourceAffectedRootsCleanupSink,
    MoliSourceFallbackRootAvailabilitySummary as StyloSourceFallbackRootAvailabilitySummary,
    MoliSourceFallbackRootAvailabilitySummarySink as StyloSourceFallbackRootAvailabilitySummarySink,
    MoliSourceStyleInvalidationSourceResultKindSummary as StyloSourceStyleInvalidationSourceResultKindSummary,
    MoliSourceStyleInvalidationSourceResultKindSummarySink as StyloSourceStyleInvalidationSourceResultKindSummarySink,
    MoliSourceStyleInvalidationSourceResultParts as StyloSourceStyleInvalidationSourceResultParts,
    MoliSourceStyleInvalidationSourceResultPartsSink as StyloSourceStyleInvalidationSourceResultPartsSink,
    MoliSourceStyleInvalidationSourceResultSink as StyloSourceStyleInvalidationSourceResultSink,
    MoliSourceStyleInvalidationTargetResultCleanupFacts as StyloSourceStyleInvalidationTargetResultCleanupFacts,
    MoliSourceStyleInvalidationTargetResultCleanupFactsPartsSink as StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink,
    MoliSourceStyleInvalidationTargetResultCleanupFactsSink as StyloSourceStyleInvalidationTargetResultCleanupFactsSink,
    MoliSourceStyleInvalidationTargetResultDiagnosticFacts as StyloSourceStyleInvalidationTargetResultDiagnosticFacts,
    MoliSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink as StyloSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink,
    MoliSourceStyleInvalidationTargetResultDiagnosticFactsSink as StyloSourceStyleInvalidationTargetResultDiagnosticFactsSink,
    MoliSourceStyleInvalidationTargetResultRecord as StyloSourceStyleInvalidationTargetResultRecord,
    moli_attribute_change_can_skip_fallback_without_dependency as stylo_attribute_change_can_skip_fallback_without_dependency,
    moli_attribute_change_can_use_retained_invalidator as stylo_attribute_change_can_use_retained_invalidator,
    moli_merge_retained_source_invalidation_fallback_kind as stylo_merge_retained_source_invalidation_fallback_kind,
    moli_merge_retained_source_invalidation_kind as stylo_merge_retained_source_invalidation_kind,
    moli_merge_source_dependency_request_requirement as stylo_merge_source_dependency_request_requirement,
    moli_merge_source_style_invalidation_query_results as stylo_merge_source_style_invalidation_query_results,
    moli_retained_source_invalidation_kind_can_use_fallback_payload as stylo_retained_source_invalidation_kind_can_use_fallback_payload,
    moli_retained_source_style_invalidation_from_parts as stylo_retained_source_style_invalidation_from_parts,
    moli_source_fallback_reason_for_unretained_state_change as stylo_source_fallback_reason_for_unretained_state_change,
    moli_state_change_can_use_retained_invalidator as stylo_state_change_can_use_retained_invalidator,
};

pub type MoliInvalidationResult = ForkMoliInvalidationResult<NodeId>;
pub type MoliInvalidationResultBuilder = ForkMoliInvalidationResultBuilder<NodeId>;
pub type StyloStyleInvalidationQuery<'a> = MoliStyleInvalidationQuery<'a>;
type StyloSourceStyleInvalidationQuery<'a> = MoliSourceStyleInvalidationQuery<'a, NodeId>;
pub type StyloSourceAffectedRootsCleanup = MoliSourceAffectedRootsCleanup<NodeId>;
type StyloStyleInvalidationResult = MoliSourceStyleInvalidationQueryResult<NodeId>;
type StyloStyleInvalidationProcessor<'a, 'b, 'element> = ForkMoliStyleInvalidationProcessor<
    'a,
    'b,
    StyleElement<'element>,
    NodeId,
    StyloStyleInvalidationRootMapper,
>;
type StyloSnapshotRelativeDependencyRoots = MoliSnapshotRelativeDependencyRoots<NodeId>;
type StyloSourceStyleInvalidationResult = MoliSourceStyleInvalidationResult<NodeId>;
type StyloSourceStyleInvalidationResultAccumulator =
    MoliSourceStyleInvalidationResultAccumulator<NodeId>;
pub type StyloSourceStyleInvalidationSourceResult = MoliSourceStyleInvalidationSourceResult<NodeId>;
pub type StyloStateInvalidationRoot = MoliStateInvalidationRoot<NodeId>;
pub type StyloRetainedStyleSiblingTraversal = MoliRetainedStyleSiblingTraversal<NodeId>;
pub type StyloRetainedStyleInvalidationQuery = MoliRetainedStyleInvalidationQuery<NodeId>;
pub type StyloElementDependencySnapshot = MoliElementDependencySnapshot<NodeId>;
pub type StyloRuntimeFallbackRootInput<'a> = MoliRuntimeFallbackRootInput<'a, NodeId>;
pub type StyloSourceDependencyRequestRequirement = MoliSourceDependencyRequestRequirement;
pub type StyloRetainedStyleChildListInvalidationQueries =
    MoliRetainedStyleChildListInvalidationQueries<NodeId>;
type StyloRetainedStyleChildListInvalidationQueryBuilder =
    MoliRetainedStyleChildListInvalidationQueryBuilder<NodeId>;
type StyloRetainedStyleChildListMutationContext<'a> =
    MoliRetainedStyleChildListMutationContext<'a, NodeId>;
pub type StyloRetainedStyleChildListInvalidationQuery =
    MoliRetainedStyleChildListInvalidationQuery<NodeId>;
pub type StyloSourceDependencyBoundaryRoots<'a> = MoliSourceDependencyBoundaryRoots<'a, NodeId>;
pub type StyloDependencyInvalidationFallbackContext =
    MoliDependencyInvalidationFallbackContext<NodeId>;
type StyloDependencyInvalidationContextRoots = MoliDependencyInvalidationContextRoots<NodeId>;
pub type StyloSourceDependencyInvalidationRequest<'a> =
    MoliSourceDependencyInvalidationRequest<'a, NodeId>;
pub type StyloSourceDependencySummary = MoliSourceDependencySummary;
pub type StyloSourceDependencyInvalidationBatchSource<'a> =
    MoliSourceDependencyInvalidationBatchSource<'a, NodeId>;
pub type StyloPlannedSourceDependencyInvalidation = MoliPlannedSourceDependencyInvalidation<NodeId>;
pub type StyloPlannedSourceDependencyInvalidationTarget =
    MoliPlannedSourceDependencyInvalidationTarget<NodeId>;
pub type StyloPlannedFallbackRootInvalidationTarget =
    MoliPlannedFallbackRootInvalidationTarget<NodeId>;
pub type StyloSourceDependencyInvalidationBatchPlan =
    MoliSourceDependencyInvalidationBatchPlan<NodeId>;
pub type StyloStylesheetSourceScopeFallbackInput = MoliStylesheetSourceScopeFallbackInput<NodeId>;

#[derive(Clone, Copy, Debug, Default)]
struct StyloStyleInvalidationRootMapper;

impl<'element> MoliStyleInvalidationElementRoot<StyleElement<'element>, NodeId>
    for StyloStyleInvalidationRootMapper
{
    fn root_for_style_invalidation_element(&self, element: StyleElement<'element>) -> NodeId {
        element.handle()
    }
}

#[derive(Default)]
struct StyloNormalStyleInvalidationDependencyPlanEffects {
    drop_relative_dependencies: bool,
    exact_empty_result: bool,
}

impl MoliNormalStyleInvalidationDependencyPlanSink
    for StyloNormalStyleInvalidationDependencyPlanEffects
{
    fn drop_collected_relative_dependencies(&mut self) {
        self.drop_relative_dependencies = true;
    }

    fn record_exact_empty_result(&mut self) {
        self.exact_empty_result = true;
    }
}

pub use style::moli_invalidation::{
    MoliPlannedFallbackRootInvalidationTargetPartsSink as StyloPlannedFallbackRootInvalidationTargetPartsSink,
    MoliPlannedSourceDependencyInvalidationPartsSink as StyloPlannedSourceDependencyInvalidationPartsSink,
    MoliPlannedSourceDependencyInvalidationTargetPartsSink as StyloPlannedSourceDependencyInvalidationTargetPartsSink,
    MoliRetainedStyleChildListInvalidationQueriesSink as StyloRetainedStyleChildListInvalidationQueriesSink,
    MoliSourceDependencyInvalidationBatchPlanSink as StyloSourceDependencyInvalidationBatchPlanSink,
};

/// Source/scope boundary used when deciding which stylesheet sources can be
/// affected by a Moli style invalidation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyloStyleSourceScope {
    documents: HashSet<NodeId>,
    shadow_roots: HashSet<NodeId>,
}

#[derive(Default)]
struct StyloConnectedShadowRootScopeSnapshot {
    members: Option<Vec<ConnectedShadowRootSnapshot>>,
    related_members_by_light_tree_handle: HashMap<NodeId, Vec<ConnectedShadowRootSnapshot>>,
}

impl StyloStyleSourceScope {
    pub fn for_handle(host: &DomHost, handle: NodeId) -> Self {
        let mut scope = Self::default();
        scope.include_handle(host, handle);
        scope
    }

    pub fn for_handles(host: &DomHost, handles: impl IntoIterator<Item = NodeId>) -> Self {
        let mut scope = Self::default();
        scope.include_handles(host, handles);
        scope
    }

    pub fn for_document(document: NodeId) -> Self {
        let mut scope = Self::default();
        scope.documents.insert(document);
        scope
    }

    pub fn for_document_and_connected_shadow_roots(host: &DomHost, document: NodeId) -> Self {
        let mut scope = Self::for_document(document);
        let mut connected_shadow_roots = StyloConnectedShadowRootScopeSnapshot::default();
        connected_shadow_roots.include_roots_for_document(host, document, &mut scope.shadow_roots);
        scope
    }

    fn include_handle(&mut self, host: &DomHost, handle: NodeId) {
        let mut connected_shadow_roots = StyloConnectedShadowRootScopeSnapshot::default();
        self.include_handle_with_connected_shadow_roots(host, handle, &mut connected_shadow_roots);
    }

    fn include_handles(&mut self, host: &DomHost, handles: impl IntoIterator<Item = NodeId>) {
        let mut connected_shadow_roots = StyloConnectedShadowRootScopeSnapshot::default();
        let mut seen_handles = HashSet::new();
        for handle in handles {
            if !seen_handles.insert(handle) {
                continue;
            }
            self.include_handle_with_connected_shadow_roots(
                host,
                handle,
                &mut connected_shadow_roots,
            );
        }
    }

    fn include_handle_with_connected_shadow_roots(
        &mut self,
        host: &DomHost,
        handle: NodeId,
        connected_shadow_roots: &mut StyloConnectedShadowRootScopeSnapshot,
    ) {
        if handle == host.document_handle() {
            self.documents.insert(handle);
            self.include_connected_shadow_roots_related_to_light_tree_handle(
                host,
                handle,
                connected_shadow_roots,
            );
        } else if host.is_connected(handle)
            && let Some(document) = host.owner_document_handle(handle)
        {
            self.documents.insert(document);
            if document == host.document_handle() && host.containing_shadow_root(handle).is_none() {
                self.include_connected_shadow_roots_related_to_light_tree_handle(
                    host,
                    handle,
                    connected_shadow_roots,
                );
            }
        } else if let Some(document) = host.owner_document_handle(handle)
            && document != host.document_handle()
        {
            self.documents.insert(document);
        }
        if host.is_shadow_root(handle)
            && stylo_shadow_root_host_participates_in_style_scope(host, handle)
        {
            self.shadow_roots.insert(handle);
        }
        if let Some(root) = host.containing_shadow_root(handle)
            && stylo_shadow_root_host_participates_in_style_scope(host, root)
        {
            self.shadow_roots.insert(root);
        }
        if let Some(slot) = host.assigned_slot_for_node(handle)
            && let Some(root) = host.containing_shadow_root(slot)
            && stylo_shadow_root_host_participates_in_style_scope(host, root)
        {
            self.shadow_roots.insert(root);
        }
        if let Some(root) = host.shadow_root_handle(handle)
            && stylo_shadow_root_host_participates_in_style_scope(host, root)
        {
            self.shadow_roots.insert(root);
        }
    }

    fn include_connected_shadow_roots_related_to_light_tree_handle(
        &mut self,
        host: &DomHost,
        handle: NodeId,
        connected_shadow_roots: &mut StyloConnectedShadowRootScopeSnapshot,
    ) {
        connected_shadow_roots.include_roots_related_to_light_tree_handle(
            host,
            handle,
            &mut self.shadow_roots,
        );
    }

    fn contains_document(&self, document: NodeId) -> bool {
        self.documents.contains(&document)
    }

    fn contains_shadow_root(&self, host: &DomHost, root: NodeId) -> bool {
        host.is_shadow_root(root) && self.shadow_roots.contains(&root)
    }

    fn conservative_fallback_roots(&self, host: &DomHost) -> Vec<NodeId> {
        let mut roots = Vec::new();
        let mut seen = HashSet::new();
        for &document in &self.documents {
            push_unique_root(&mut roots, &mut seen, document);
        }
        for &root in &self.shadow_roots {
            push_unique_root(&mut roots, &mut seen, root);
            if let Some(shadow_host) = host.shadow_root_host(root) {
                push_unique_root(&mut roots, &mut seen, shadow_host);
            }
        }
        roots
    }
}

impl StyloConnectedShadowRootScopeSnapshot {
    fn include_roots_for_document(
        &mut self,
        host: &DomHost,
        document: NodeId,
        target: &mut HashSet<NodeId>,
    ) {
        for member in self.members(host) {
            if host.owner_document_handle(member.root) == Some(document) {
                target.insert(member.root);
            }
        }
    }

    fn include_all_roots(&mut self, host: &DomHost, target: &mut HashSet<NodeId>) {
        for member in self.members(host) {
            target.insert(member.root);
        }
    }

    fn include_roots_related_to_light_tree_handle(
        &mut self,
        host: &DomHost,
        handle: NodeId,
        target: &mut HashSet<NodeId>,
    ) {
        if handle == host.document_handle() {
            self.include_all_roots(host, target);
            return;
        }
        for member in self.related_members_for_light_tree_handle(host, handle) {
            target.insert(member.root);
        }
    }

    fn members<'a>(&'a mut self, host: &DomHost) -> &'a [ConnectedShadowRootSnapshot] {
        self.members
            .get_or_insert_with(|| {
                host.snapshot_connected_shadow_root_bindings()
                    .into_iter()
                    .filter(|binding| {
                        stylo_shadow_host_participates_in_style_scope(host, binding.host)
                    })
                    .collect()
            })
            .as_slice()
    }

    fn related_members_for_light_tree_handle<'a>(
        &'a mut self,
        host: &DomHost,
        handle: NodeId,
    ) -> &'a [ConnectedShadowRootSnapshot] {
        self.related_members_by_light_tree_handle
            .entry(handle)
            .or_insert_with(|| {
                host.snapshot_connected_shadow_root_bindings_related_to_light_tree_handle(handle)
                    .into_iter()
                    .filter(|binding| {
                        stylo_shadow_host_participates_in_style_scope(host, binding.host)
                    })
                    .collect()
            })
            .as_slice()
    }
}

pub fn stylo_stylesheet_owner_is_in_source_scope(
    host: &DomHost,
    owner: NodeId,
    source_scope: &StyloStyleSourceScope,
) -> bool {
    if let Some(root) = host.containing_shadow_root(owner) {
        return source_scope.contains_shadow_root(host, root)
            && stylo_shadow_root_host_participates_in_style_scope(host, root);
    }
    host.owner_document_handle(owner).is_some_and(|document| {
        source_scope.contains_document(document)
            && (host.is_connected(owner) || document != host.document_handle())
    })
}

pub fn stylo_stylesheet_source_scope_fallback_roots(
    host: &DomHost,
    input: StyloStylesheetSourceScopeFallbackInput,
    source_scope: &StyloStyleSourceScope,
) -> Vec<NodeId> {
    moli_stylesheet_source_scope_fallback_roots(
        input,
        &StyloStylesheetSourceScopeFallbackRootsResolver { host, source_scope },
    )
}

struct StyloStylesheetSourceScopeFallbackRootsResolver<'a> {
    host: &'a DomHost,
    source_scope: &'a StyloStyleSourceScope,
}

impl MoliStylesheetSourceScopeFallbackRootsResolver<NodeId>
    for StyloStylesheetSourceScopeFallbackRootsResolver<'_>
{
    fn stylesheet_owner_source_scope_fallback_roots(&self, owner: NodeId) -> Vec<NodeId> {
        stylo_stylesheet_owner_source_scope_fallback_roots(self.host, owner, self.source_scope)
    }

    fn document_source_scope_fallback_roots(&self, document: NodeId) -> Vec<NodeId> {
        stylo_document_source_scope_fallback_roots(self.host, document, self.source_scope)
    }

    fn shadow_root_source_scope_fallback_roots(&self, root: NodeId) -> Vec<NodeId> {
        stylo_shadow_root_source_scope_fallback_roots(self.host, root, self.source_scope)
    }
}

pub fn stylo_source_scope_fallback_plan(
    host: &DomHost,
    source_scope: &StyloStyleSourceScope,
    fallback_reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
) -> StyloPlannedFallbackRootInvalidationTarget {
    moli_source_scope_fallback_plan(
        || source_scope.conservative_fallback_roots(host),
        fallback_reasons,
    )
}

pub fn stylo_runtime_or_source_scope_fallback_plan(
    host: &DomHost,
    source_scope: &StyloStyleSourceScope,
    runtime_fallback_roots: Vec<NodeId>,
    fallback_reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
) -> StyloPlannedFallbackRootInvalidationTarget {
    moli_runtime_or_source_scope_fallback_plan(
        runtime_fallback_roots,
        || source_scope.conservative_fallback_roots(host),
        fallback_reasons,
    )
}

pub fn stylo_fallback_roots_plan(
    fallback_roots: Vec<NodeId>,
    fallback_reasons: impl IntoIterator<Item = StyloSourceInvalidationFallbackReason>,
) -> StyloPlannedFallbackRootInvalidationTarget {
    moli_fallback_roots_plan(fallback_roots, fallback_reasons)
}

fn stylo_stylesheet_owner_source_scope_fallback_roots(
    host: &DomHost,
    owner: NodeId,
    source_scope: &StyloStyleSourceScope,
) -> Vec<NodeId> {
    if host.node(owner).is_none() {
        return Vec::new();
    }
    if let Some(root) = host.containing_shadow_root(owner) {
        return stylo_shadow_root_source_scope_fallback_roots(host, root, source_scope);
    }
    host.owner_document_handle(owner)
        .filter(|document| {
            source_scope.contains_document(*document)
                && (host.is_connected(owner) || *document != host.document_handle())
        })
        .into_iter()
        .collect()
}

fn stylo_document_source_scope_fallback_roots(
    host: &DomHost,
    document: NodeId,
    source_scope: &StyloStyleSourceScope,
) -> Vec<NodeId> {
    if host.node(document).is_some() && source_scope.contains_document(document) {
        vec![document]
    } else {
        Vec::new()
    }
}

fn stylo_shadow_root_source_scope_fallback_roots(
    host: &DomHost,
    root: NodeId,
    source_scope: &StyloStyleSourceScope,
) -> Vec<NodeId> {
    if !source_scope.contains_shadow_root(host, root)
        || !stylo_shadow_root_host_participates_in_style_scope(host, root)
    {
        return Vec::new();
    }
    let mut roots = vec![root];
    if let Some(shadow_host) = host.shadow_root_host(root) {
        roots.push(shadow_host);
    }
    roots
}

pub fn stylo_shadow_root_host_participates_in_style_scope(host: &DomHost, root: NodeId) -> bool {
    host.shadow_root_host(root)
        .is_some_and(|shadow_host| stylo_shadow_host_participates_in_style_scope(host, shadow_host))
}

fn stylo_shadow_host_participates_in_style_scope(host: &DomHost, shadow_host: NodeId) -> bool {
    // A stylesheet inside a shadow tree can style that tree when the host is in
    // the active document. Detached/child documents are also independent style
    // scopes, even though their nodes are not connected to the active document.
    host.is_connected(shadow_host)
        || host
            .owner_document_handle(shadow_host)
            .is_some_and(|document| document != host.document_handle())
}

/// Capture dependency keys for one live element before it leaves the tree.
pub fn stylo_element_dependency_snapshot(
    host: &DomHost,
    handle: NodeId,
) -> Option<StyloElementDependencySnapshot> {
    let element = host.node(handle)?.as_element()?;
    Some(StyloElementDependencySnapshot::new(
        handle,
        element.local_name().to_owned(),
        stylo_retained_dependency_state_for_element(element),
        element
            .attributes()
            .iter()
            .map(|attribute| attribute.local_name().to_owned())
            .collect(),
        ascii_whitespace_tokens(element.attribute("class").unwrap_or_default()),
        element.custom_states().iter().cloned().collect(),
        element.id().map(str::to_owned),
    ))
}

/// Capture dependency keys for the removed elements in a child-list mutation.
pub fn stylo_removed_element_dependency_snapshots(
    host: &DomHost,
    removed_nodes: &[NodeId],
) -> Vec<StyloElementDependencySnapshot> {
    removed_nodes
        .iter()
        .flat_map(|&handle| stylo_element_dependency_snapshots_for_current_subtree(host, handle))
        .collect()
}

/// Build retained dependency queries for a currently-live element.
pub fn stylo_retained_queries_for_current_element(
    host: &DomHost,
    root: NodeId,
    sibling_traversal: Option<StyloRetainedStyleSiblingTraversal>,
) -> Vec<StyloRetainedStyleInvalidationQuery> {
    let Some(snapshot) = stylo_element_dependency_snapshot(host, root) else {
        return Vec::new();
    };
    moli_retained_queries_for_element_dependency_snapshot(&snapshot, sibling_traversal)
}

fn stylo_retained_non_universal_queries_for_current_element(
    host: &DomHost,
    root: NodeId,
    sibling_traversal: Option<StyloRetainedStyleSiblingTraversal>,
) -> Vec<StyloRetainedStyleInvalidationQuery> {
    let Some(snapshot) = stylo_element_dependency_snapshot(host, root) else {
        return Vec::new();
    };
    moli_retained_non_universal_queries_for_element_dependency_snapshot(
        &snapshot,
        sibling_traversal,
    )
}

fn stylo_element_dependency_snapshots_for_current_subtree(
    host: &DomHost,
    root: NodeId,
) -> Vec<StyloElementDependencySnapshot> {
    let mut snapshots = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(handle) = stack.pop() {
        if !seen.insert(handle) {
            continue;
        }
        if let Some(snapshot) = stylo_element_dependency_snapshot(host, handle) {
            snapshots.push(snapshot);
        }
        let mut child = host.first_child(handle);
        while let Some(current) = child {
            stack.push(current);
            child = host.next_sibling(current);
        }
    }
    snapshots
}

/// Build retained dependency queries from keys captured before an element was
/// removed from the tree.
pub fn stylo_retained_queries_for_element_snapshot(
    snapshot: &StyloElementDependencySnapshot,
    sibling_traversal: Option<StyloRetainedStyleSiblingTraversal>,
) -> Vec<StyloRetainedStyleInvalidationQuery> {
    moli_retained_queries_for_element_dependency_snapshot(snapshot, sibling_traversal)
}

pub fn stylo_retained_previous_element_sibling(
    host: &DomHost,
    sibling: Option<NodeId>,
) -> Option<NodeId> {
    let mut current = sibling;
    while let Some(handle) = current {
        let node = host.node(handle)?;
        if node.as_element().is_some() {
            return Some(handle);
        }
        current = node.prev_sibling();
    }
    None
}

pub fn stylo_retained_next_element_sibling(
    host: &DomHost,
    sibling: Option<NodeId>,
) -> Option<NodeId> {
    let mut current = sibling;
    while let Some(handle) = current {
        let node = host.node(handle)?;
        if node.as_element().is_some() {
            return Some(handle);
        }
        current = host.next_sibling(handle);
    }
    None
}

pub type StyloSourceFallbackRootAvailability = MoliSourceFallbackRootAvailability;
pub type StyloSourceInvalidationFallbackReason = MoliSourceInvalidationFallbackReason;
pub type StyloSourceStyleInvalidationSourceResultKind = MoliSourceStyleInvalidationSourceResultKind;
pub type StyloRetainedSourceStyleInvalidationKind = MoliRetainedSourceStyleInvalidationKind;
pub type StyloRetainedSourceStyleInvalidation<'a> =
    MoliRetainedSourceStyleInvalidation<'a, NodeId, MoliStyleMutationSnapshot>;
pub type StyloStyleInvalidationSnapshot = MoliStyleInvalidationSnapshot<NodeId>;
pub type StyloStyleInvalidationSnapshotAttribute = MoliStyleInvalidationSnapshotAttribute;

/// Audit metadata for one retained source-invalidation fallback reason.
///
/// The reason enum itself is owned by the Stylo fork. Moli's selector
/// adapter keeps this exhaustive match beside the alias so every conservative
/// root still names the missing fact and the work needed to make it exact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyloSourceInvalidationFallbackReasonPlan {
    pub owner: &'static str,
    pub missing_fact: &'static str,
    pub next_work_item: &'static str,
}

pub fn stylo_source_invalidation_fallback_reason_plan(
    reason: StyloSourceInvalidationFallbackReason,
) -> StyloSourceInvalidationFallbackReasonPlan {
    match reason {
        StyloSourceInvalidationFallbackReason::UnknownDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo dependency extraction",
                missing_fact: "typed dependency provenance for an invalidation-map entry",
                next_work_item: "classify the dependency before it reaches retained source queries",
            }
        }
        StyloSourceInvalidationFallbackReason::FullSelector => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo invalidation-map planner",
                missing_fact: "source-local exact roots for full-selector invalidation",
                next_work_item: "decompose full-selector invalidation into retained source queries or exact fallback roots",
            }
        }
        StyloSourceInvalidationFallbackReason::RelativeAnySelector => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo relative-selector dependency API",
                missing_fact: "anchor and subject roots for relative-selector dependencies",
                next_work_item: "return cause-local roots for relative selectors such as :has()",
            }
        }
        StyloSourceInvalidationFallbackReason::ScopeDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo scope dependency API",
                missing_fact: "scope boundary roots for source-local retained invalidation",
                next_work_item: "expose exact @scope/:scope affected roots to the source query planner",
            }
        }
        StyloSourceInvalidationFallbackReason::UnsupportedStateDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo state dependency planner",
                missing_fact: "exact roots for state-dependent selectors",
                next_work_item: "map supported runtime state changes to retained source queries without source-scope fallback",
            }
        }
        StyloSourceInvalidationFallbackReason::UnsupportedShadowDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo shadow dependency planner",
                missing_fact: "exact roots for shadow, slot, and part dependencies",
                next_work_item: "return shadow-tree and host roots for supported shadow dependency kinds",
            }
        }
        StyloSourceInvalidationFallbackReason::SourceScopeFallback => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Moli selector source-scope adapter",
                missing_fact: "cause-local or source-local roots for this mutation",
                next_work_item: "replace source-scope fallback with exact roots from the Stylo query plan",
            }
        }
        StyloSourceInvalidationFallbackReason::UnsupportedDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo dependency metadata",
                missing_fact: "retained-query representation for the dependency kind",
                next_work_item: "add a typed retained query or exact fallback-root plan for the dependency",
            }
        }
        StyloSourceInvalidationFallbackReason::NthOfDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Moli mutation-context capture",
                missing_fact: "mutation-time sibling context for an nth-of query that reached the planner without it",
                next_work_item: "attach sibling context to the remaining context-free nth-of invalidation entry points",
            }
        }
        StyloSourceInvalidationFallbackReason::NestedRelativeSelectorDependency => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo nested relative-selector planner",
                missing_fact: "root mapping for selector lists nested inside relative selectors",
                next_work_item: "preserve nested relative-selector context in retained source queries",
            }
        }
        StyloSourceInvalidationFallbackReason::InexactEmptyResult => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Stylo source query result exactness",
                missing_fact: "complete verification for a matched dependency branch that returned no affected roots",
                next_work_item: "extend snapshot and relative-dependency verification; direct keyed document-scope zero matches are already exact",
            }
        }
        StyloSourceInvalidationFallbackReason::MissingFallbackRoots => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Moli source lifecycle adapter",
                missing_fact: "fallback roots for a source query that requested them",
                next_work_item: "provide source/scope roots or drop stale unavailable source targets before planning",
            }
        }
        StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Moli style-engine retained state",
                missing_fact: "retained style system availability when the source query is drained",
                next_work_item: "keep retained state available through source invalidation or emit an explicit lifecycle teardown",
            }
        }
        StyloSourceInvalidationFallbackReason::MissingRetainedCascadeData => {
            StyloSourceInvalidationFallbackReasonPlan {
                owner: "Moli style-engine source map",
                missing_fact: "per-source CascadeData for the stylesheet source",
                next_work_item: "preserve source-to-cascade-data mapping until pending retained invalidations are drained",
            }
        }
    }
}

pub type MoliStyleMutationElementSnapshot = ForkMoliStyleMutationElementSnapshot;

/// Child-list before-state captured while the mutation record still has its
/// mutation-time sibling relation and removed-element dependency keys.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MoliStyleChildListMutationSnapshot {
    parent: NodeId,
    added_nodes: Vec<NodeId>,
    removed_nodes: Vec<NodeId>,
    removed_element_snapshots: Vec<StyloElementDependencySnapshot>,
    previous_sibling: Option<NodeId>,
    next_sibling: Option<NodeId>,
}

/// Mutation-time before-state for one retained invalidation batch.
///
/// The renderer records old attribute values and old element state here while
/// it still owns the runtime mutation context. The selector crate materializes
/// the payload into Stylo invalidation snapshots when the retained invalidator
/// is actually run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MoliStyleMutationSnapshot {
    inputs: IndexMap<NodeId, MoliStyleMutationElementSnapshot>,
    child_list_mutations: Vec<MoliStyleChildListMutationSnapshot>,
}

impl MoliStyleMutationSnapshot {
    pub fn record_attribute_change(
        &mut self,
        element: NodeId,
        name: &str,
        old_value: Option<String>,
    ) {
        self.inputs
            .entry(element)
            .or_default()
            .record_attribute_change(name, old_value);
    }

    pub fn try_record_old_state(&mut self, element: NodeId, old_state: ElementState) -> Option<()> {
        self.inputs
            .entry(element)
            .or_default()
            .try_record_old_state(old_state)
    }

    pub fn record_old_custom_states(&mut self, element: NodeId, old_custom_states: Vec<String>) {
        self.inputs
            .entry(element)
            .or_default()
            .record_old_custom_states(old_custom_states);
    }

    pub fn record_child_list_mutation(
        &mut self,
        parent: NodeId,
        added_nodes: &[NodeId],
        removed_nodes: &[NodeId],
        removed_element_snapshots: &[StyloElementDependencySnapshot],
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    ) {
        self.child_list_mutations
            .push(MoliStyleChildListMutationSnapshot {
                parent,
                added_nodes: added_nodes.to_vec(),
                removed_nodes: removed_nodes.to_vec(),
                removed_element_snapshots: removed_element_snapshots.to_vec(),
                previous_sibling,
                next_sibling,
            });
    }

    pub fn merge_from(&mut self, incoming: Self) {
        for (handle, input) in incoming.inputs {
            self.inputs.entry(handle).or_default().merge_from(input);
        }
        self.child_list_mutations
            .extend(incoming.child_list_mutations);
    }

    pub fn len(&self) -> usize {
        self.inputs.len() + self.child_list_mutations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty() && self.child_list_mutations.is_empty()
    }

    fn to_stylo_snapshots(&self, host: &DomHost) -> Option<Vec<StyloStyleInvalidationSnapshot>> {
        self.iter()
            .map(|(handle, input)| stylo_retained_style_snapshot_for_input(host, handle, input))
            .collect()
    }

    pub fn child_list_invalidation_queries(
        &self,
        host: &DomHost,
    ) -> Option<StyloRetainedStyleChildListInvalidationQueries> {
        let mut builder = StyloRetainedStyleChildListInvalidationQueryBuilder::new();
        let mut changed_roots = IndexSet::new();
        for mutation in &self.child_list_mutations {
            changed_roots.extend(mutation.added_nodes.iter().copied());
            changed_roots.extend(mutation.removed_nodes.iter().copied());
        }
        for mutation in &self.child_list_mutations {
            if host
                .node(mutation.parent)
                .and_then(|node| node.as_element())
                .is_some()
            {
                builder.insert_queries(
                    stylo_retained_queries_for_current_element(host, mutation.parent, None),
                    StyloSourceDependencyRequestRequirement::child_list_structural(),
                );
                push_style_ancestor_chain_roots(host, mutation.parent, |root| {
                    builder.insert_empty_target_fallback_root(root)
                });
            }
            if !mutation.added_nodes.is_empty() {
                let added_sibling_traversal = Some(StyloRetainedStyleSiblingTraversal::new(
                    stylo_retained_previous_element_sibling(host, mutation.previous_sibling),
                    stylo_retained_next_element_sibling(host, mutation.next_sibling),
                ));
                for &root in &mutation.added_nodes {
                    builder.insert_queries(
                        stylo_retained_queries_for_current_subtree(host, root),
                        StyloSourceDependencyRequestRequirement::exact(),
                    );
                    builder.insert_queries(
                        stylo_retained_non_universal_queries_for_current_element(
                            host,
                            root,
                            added_sibling_traversal,
                        ),
                        StyloSourceDependencyRequestRequirement::exact(),
                    );
                }
                let previous_sibling =
                    stylo_retained_previous_element_sibling(host, mutation.previous_sibling);
                if let Some(plan) = moli_child_list_sibling_boundary_plan(
                    previous_sibling,
                    previous_sibling.is_some_and(|root| changed_roots.contains(&root)),
                    MoliChildListSiblingBoundaryKind::AddedPreviousSibling {
                        inserted_at_end: mutation.next_sibling.is_none(),
                    },
                ) {
                    plan.apply_to_builder(&mut builder);
                }
                let next_sibling = stylo_retained_next_element_sibling(host, mutation.next_sibling);
                if let Some(plan) = moli_child_list_sibling_boundary_plan(
                    next_sibling,
                    next_sibling.is_some_and(|root| changed_roots.contains(&root)),
                    MoliChildListSiblingBoundaryKind::AddedNextSibling,
                ) {
                    let next_sibling = next_sibling
                        .expect("planned added next-sibling boundary should carry a root");
                    plan.apply_to_builder(&mut builder);
                    builder.insert_queries(
                        stylo_retained_non_universal_queries_for_current_element(
                            host,
                            next_sibling,
                            Some(StyloRetainedStyleSiblingTraversal::new(
                                previous_sibling,
                                stylo_retained_next_element_sibling(
                                    host,
                                    host.next_sibling(next_sibling),
                                ),
                            )),
                        ),
                        StyloSourceDependencyRequestRequirement::exact(),
                    );
                    builder.insert_queries(
                        [StyloRetainedStyleInvalidationQuery::universal(next_sibling)
                            .with_sibling_traversal(Some(
                                StyloRetainedStyleSiblingTraversal::new(
                                    previous_sibling,
                                    stylo_retained_next_element_sibling(
                                        host,
                                        host.next_sibling(next_sibling),
                                    ),
                                ),
                            ))],
                        StyloSourceDependencyRequestRequirement::relative_previous_sibling(),
                    );
                }
            }

            if mutation.removed_nodes.is_empty() {
                continue;
            }
            let previous_sibling =
                stylo_retained_previous_element_sibling(host, mutation.previous_sibling);
            let next_sibling = stylo_retained_next_element_sibling(host, mutation.next_sibling);
            let traversal = StyloRetainedStyleSiblingTraversal::new(previous_sibling, next_sibling);
            if let Some(plan) = moli_child_list_sibling_boundary_plan(
                previous_sibling,
                previous_sibling.is_some_and(|root| changed_roots.contains(&root)),
                MoliChildListSiblingBoundaryKind::RemovedPreviousSibling,
            ) {
                let previous_sibling = previous_sibling
                    .expect("planned removed previous-sibling boundary should carry a root");
                plan.apply_to_builder(&mut builder);
                if let Some(earlier_sibling) = stylo_retained_previous_element_sibling(
                    host,
                    host.node(previous_sibling)?.prev_sibling(),
                ) && let Some(plan) = moli_child_list_sibling_boundary_plan(
                    Some(earlier_sibling),
                    changed_roots.contains(&earlier_sibling),
                    MoliChildListSiblingBoundaryKind::RemovedEarlierSibling,
                ) {
                    plan.apply_to_builder(&mut builder);
                }
            }
            if let Some(plan) = moli_child_list_sibling_boundary_plan(
                next_sibling,
                next_sibling.is_some_and(|root| changed_roots.contains(&root)),
                MoliChildListSiblingBoundaryKind::RemovedNextSibling,
            ) {
                plan.apply_to_builder(&mut builder);
            }
            let removed_roots_with_snapshots = mutation
                .removed_nodes
                .iter()
                .filter(|&&handle| {
                    host.node(handle)
                        .and_then(|node| node.as_element())
                        .is_some()
                })
                .count();
            if mutation.removed_element_snapshots.len() < removed_roots_with_snapshots {
                return None;
            }
            for snapshot in &mutation.removed_element_snapshots {
                builder.insert_queries(
                    stylo_retained_queries_for_element_snapshot(snapshot, Some(traversal)),
                    StyloSourceDependencyRequestRequirement::exact(),
                );
            }
            if let Some(previous_sibling) = previous_sibling {
                builder.insert_queries(
                    stylo_retained_queries_for_current_element(host, previous_sibling, None),
                    StyloSourceDependencyRequestRequirement::exact(),
                );
            }
        }
        builder.into_queries()
    }

    pub fn child_list_dependency_fallback_context(
        &self,
        query: &StyloRetainedStyleInvalidationQuery,
    ) -> Option<StyloDependencyInvalidationFallbackContext> {
        moli_child_list_dependency_fallback_context_for_query(
            self.child_list_mutations.iter().map(|mutation| {
                StyloRetainedStyleChildListMutationContext::new(
                    mutation.parent,
                    &mutation.added_nodes,
                    &mutation.removed_nodes,
                    &mutation.removed_element_snapshots,
                    mutation.previous_sibling,
                    mutation.next_sibling,
                )
            }),
            query,
        )
    }

    fn iter(&self) -> impl Iterator<Item = (NodeId, &MoliStyleMutationElementSnapshot)> {
        self.inputs.iter().map(|(&handle, input)| (handle, input))
    }
}

fn stylo_retained_queries_for_current_subtree(
    host: &DomHost,
    root: NodeId,
) -> Vec<StyloRetainedStyleInvalidationQuery> {
    let mut queries = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(handle) = stack.pop() {
        if !seen.insert(handle) {
            continue;
        }
        queries.extend(stylo_retained_queries_for_current_element(
            host, handle, None,
        ));
        let mut child = host.first_child(handle);
        while let Some(current) = child {
            stack.push(current);
            child = host.next_sibling(current);
        }
    }
    queries
}

fn stylo_retained_style_snapshot_for_input(
    host: &DomHost,
    handle: NodeId,
    input: &MoliStyleMutationElementSnapshot,
) -> Option<StyloStyleInvalidationSnapshot> {
    let element = host.node(handle)?.as_element()?;
    let mut attributes = element
        .attributes()
        .iter()
        .map(|attribute| {
            StyloStyleInvalidationSnapshotAttribute::new(
                attribute.local_name().to_owned(),
                attribute.name(),
                attribute.namespace().to_owned(),
                attribute.prefix().map(str::to_owned),
                attribute.value().to_owned(),
            )
        })
        .collect::<Vec<_>>();

    let mut changed_attributes = Vec::new();
    for change in input.attribute_changes() {
        let name = change.name();
        changed_attributes.push(name.to_owned());
        if let Some(old_value) = change.old_value() {
            if let Some(attribute) = attributes
                .iter_mut()
                .find(|attribute| attribute.is_no_namespace_local_name(name))
            {
                attribute.set_value(old_value.to_owned());
            } else {
                attributes.push(StyloStyleInvalidationSnapshotAttribute::new(
                    name.to_owned(),
                    name.to_owned(),
                    String::new(),
                    None,
                    old_value.to_owned(),
                ));
            }
        } else {
            attributes.retain(|attribute| !attribute.is_no_namespace_local_name(name));
        }
    }

    Some(StyloStyleInvalidationSnapshot::new(
        handle,
        input.old_state(),
        input.old_custom_states().map(<[_]>::to_vec),
        changed_attributes,
        attributes,
    ))
}

/// Build focus/focus-within invalidation roots, crossing shadow-root
/// boundaries the same way Stylo observes focus state.
pub fn stylo_focus_change_invalidation_roots(
    host: &DomHost,
    previous: Option<NodeId>,
    next: Option<NodeId>,
) -> Vec<StyloStateInvalidationRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for root in [previous, next].into_iter().flatten() {
        push_state_root(
            &mut roots,
            &mut seen,
            root,
            ElementState::FOCUS | ElementState::FOCUSRING,
        );
        let mut current = Some(root);
        while let Some(handle) = current {
            if host
                .node(handle)
                .is_some_and(|node| node.as_element().is_some())
            {
                push_state_root(&mut roots, &mut seen, handle, ElementState::FOCUS_WITHIN);
            }
            if let Some(shadow_root) = host.containing_shadow_root(handle)
                && let Some(shadow_host) = host.shadow_root_host(shadow_root)
            {
                push_state_root(
                    &mut roots,
                    &mut seen,
                    shadow_host,
                    ElementState::FOCUS | ElementState::FOCUSRING,
                );
            }
            current = style_parent_crossing_shadow_root(host, handle);
        }
    }
    roots
}

pub fn stylo_runtime_fallback_roots_for_mutation_inputs<'a>(
    host: &DomHost,
    inputs: impl IntoIterator<Item = StyloRuntimeFallbackRootInput<'a>>,
) -> Vec<NodeId> {
    moli_runtime_fallback_roots_for_mutation_inputs(
        inputs,
        &StyloRuntimeFallbackRootResolver { host },
    )
}

struct StyloRuntimeFallbackRootResolver<'a> {
    host: &'a DomHost,
}

impl MoliRuntimeFallbackRootResolver<NodeId> for StyloRuntimeFallbackRootResolver<'_> {
    fn unknown_slot_assignment_fallback_root(&self, slot: NodeId) -> NodeId {
        stylo_unknown_slot_assignment_fallback_root(self.host, slot)
    }
}

/// Return whether the focused element maps to `handle` for Stylo's `:focus`
/// state, including focus-delegating shadow hosts.
pub fn stylo_focus_state_matches_handle(host: &DomHost, focused: NodeId, handle: NodeId) -> bool {
    if focused == handle {
        return true;
    }
    let mut current = focused;
    while let Some(shadow_root) = host.containing_shadow_root(current) {
        let Some(shadow_host) = host.shadow_root_host(shadow_root) else {
            return false;
        };
        if shadow_host == handle {
            return true;
        }
        current = shadow_host;
    }
    false
}

/// Return whether the focused element is inside `handle` for Stylo's
/// `:focus-within` state, crossing shadow-root boundaries.
pub fn stylo_focus_within_state_matches_handle(
    host: &DomHost,
    focused: NodeId,
    handle: NodeId,
) -> bool {
    let mut current = Some(focused);
    while let Some(candidate) = current {
        if candidate == handle {
            return true;
        }
        current = style_parent_crossing_shadow_root(host, candidate);
    }
    false
}

/// Conservative root for an unknown manual slot-assignment mutation.
///
/// Exact assigned-node transitions are handled as source-local queries. When
/// the mutation payload cannot tell us previous/current assigned nodes, the
/// shadow host is the smallest Moli root that covers `::slotted(...)`
/// effects observable from the containing shadow tree.
fn stylo_unknown_slot_assignment_fallback_root(host: &DomHost, slot: NodeId) -> NodeId {
    host.containing_shadow_root(slot)
        .and_then(|shadow_root| host.shadow_root_host(shadow_root))
        .unwrap_or(slot)
}

/// Build conservative source-local fallback roots with mutation-time context.
fn stylo_dependency_invalidation_fallback_roots_with_context(
    host: &DomHost,
    root: NodeId,
    plan: MoliDependencyContextRootPlan,
    context: StyloDependencyInvalidationFallbackContext,
) -> StyloDependencyInvalidationContextRoots {
    let collector = StyloDependencyContextRootCollector::new(host, root, context);
    plan.drain_into(collector)
}

struct StyloDependencyContextRootCollector<'a> {
    host: &'a DomHost,
    root: NodeId,
    context: StyloDependencyInvalidationFallbackContext,
    roots: Vec<NodeId>,
    seen: HashSet<NodeId>,
    requires_source_fallback: bool,
}

impl<'a> StyloDependencyContextRootCollector<'a> {
    fn new(
        host: &'a DomHost,
        root: NodeId,
        context: StyloDependencyInvalidationFallbackContext,
    ) -> Self {
        Self {
            host,
            root,
            context,
            roots: Vec::new(),
            seen: HashSet::new(),
            requires_source_fallback: false,
        }
    }

    fn push_unique_root(&mut self, root: NodeId) {
        push_unique_root(&mut self.roots, &mut self.seen, root);
    }
}

impl MoliDependencyInvalidationContextRootsSink<NodeId>
    for StyloDependencyContextRootCollector<'_>
{
    fn drain_collected_context_roots_into(
        self,
        target: &mut impl MoliDependencyInvalidationContextRootsPartsSink<NodeId>,
    ) {
        if self.requires_source_fallback {
            target.record_context_source_fallback();
        }
        target.extend_context_roots(self.roots);
    }
}

impl MoliDependencyContextRootFlagsSink for StyloDependencyContextRootCollector<'_> {
    fn require_context_source_fallback(&mut self) {
        self.requires_source_fallback = true;
    }

    fn include_context_local_subtree(&mut self) {
        self.push_unique_root(self.root);
    }

    fn include_context_ancestor_chain(&mut self) {
        push_ancestor_chain_roots(self.host, self.root, &mut self.roots, &mut self.seen);
        if let Some(parent) = self.context.parent() {
            push_ancestor_chain_roots(self.host, parent, &mut self.roots, &mut self.seen);
        }
    }

    fn include_context_following_siblings(&mut self) {
        if let Some(next) = self.context.next_sibling() {
            push_following_sibling_roots(self.host, next, &mut self.roots, &mut self.seen);
        } else {
            push_following_sibling_roots(self.host, self.root, &mut self.roots, &mut self.seen);
        }
    }

    fn include_context_ancestor_following_siblings(&mut self) {
        push_ancestor_following_sibling_roots(
            self.host,
            self.root,
            &mut self.roots,
            &mut self.seen,
        );
        if let Some(parent) = self.context.parent() {
            push_ancestor_following_sibling_roots(
                self.host,
                parent,
                &mut self.roots,
                &mut self.seen,
            );
        }
    }

    fn include_context_previous_sibling(&mut self) {
        if let Some(previous) = self.context.previous_sibling() {
            self.push_unique_root(previous);
        } else {
            self.push_unique_root(self.root);
        }
    }

    fn include_context_earlier_siblings(&mut self) {
        if let Some(previous) = self.context.previous_sibling() {
            push_previous_sibling_roots(self.host, previous, &mut self.roots, &mut self.seen);
        } else {
            push_previous_sibling_roots(self.host, self.root, &mut self.roots, &mut self.seen);
        }
    }

    fn include_context_ancestor_previous_siblings(&mut self) {
        push_ancestor_previous_sibling_roots(self.host, self.root, &mut self.roots, &mut self.seen);
        if let Some(parent) = self.context.parent() {
            push_ancestor_previous_sibling_roots(
                self.host,
                parent,
                &mut self.roots,
                &mut self.seen,
            );
        }
    }

    fn include_context_slotted_elements(&mut self) {
        if self.host.assigned_slot_for_node(self.root).is_some() {
            self.push_unique_root(self.root);
        } else {
            for assigned in self
                .host
                .assigned_nodes_for_slot_with_options(self.root, false)
            {
                self.push_unique_root(assigned);
            }
        }
    }

    fn include_context_parts(&mut self) {
        if let Some(shadow_root) = self.host.shadow_root_handle(self.root) {
            self.push_unique_root(shadow_root);
        }
    }
}

struct StyloSourceDependencyContextRootsProvider<'a> {
    host: &'a DomHost,
}

impl MoliSourceDependencyInvalidationContextRootsProvider<NodeId>
    for StyloSourceDependencyContextRootsProvider<'_>
{
    fn context_roots_for_source_dependency(
        &mut self,
        root: NodeId,
        plan: MoliDependencyContextRootPlan,
        context: StyloDependencyInvalidationFallbackContext,
    ) -> StyloDependencyInvalidationContextRoots {
        stylo_dependency_invalidation_fallback_roots_with_context(self.host, root, plan, context)
    }
}

/// Build source-local invalidation plans for all stylesheet sources that can be
/// affected by a Moli mutation.
///
/// This keeps selector dependency interpretation inside the Stylo-facing
/// boundary. The renderer supplies source identity/scope and runtime fallback
/// root facts separately, while this planner chooses the concrete fallback
/// roots and returns exact queries or typed fallback targets.
pub fn stylo_source_dependency_invalidation_batch_plan(
    host: &DomHost,
    sources: &[StyloSourceDependencyInvalidationBatchSource<'_>],
    requests: &[StyloSourceDependencyInvalidationRequest<'_>],
    boundary_roots: StyloSourceDependencyBoundaryRoots<'_>,
) -> StyloSourceDependencyInvalidationBatchPlan {
    let mut context_roots_provider = StyloSourceDependencyContextRootsProvider { host };
    moli_source_dependency_invalidation_batch_plan(
        sources,
        requests,
        boundary_roots,
        &mut context_roots_provider,
    )
}

fn push_style_ancestor_chain_roots(
    host: &DomHost,
    start: NodeId,
    mut push_root: impl FnMut(NodeId),
) {
    let mut current = Some(start);
    while let Some(handle) = current {
        if host
            .node(handle)
            .is_some_and(|node| node.as_element().is_some())
        {
            push_root(handle);
        }
        current = style_parent_crossing_shadow_root(host, handle);
    }
}

fn collect_snapshot_relative_dependency_roots<'element>(
    root: StyleElement<'element>,
    dependencies: &[(Option<OpaqueElement>, &Dependency)],
    snapshot_map: &SnapshotMap,
    quirks_mode: QuirksMode,
    previous_sibling: Option<StyleElement<'element>>,
    next_sibling: Option<StyleElement<'element>>,
) -> StyloSnapshotRelativeDependencyRoots {
    let sibling_traversal = if previous_sibling.is_some() || next_sibling.is_some() {
        SiblingTraversalMap::new(root, previous_sibling, next_sibling)
    } else {
        SiblingTraversalMap::default()
    };
    let mut roots = IndexSet::new();
    let mut verified_dependency_count = 0;
    for (scope, dependency) in dependencies {
        if !moli_dependency_is_relative_selector(dependency) {
            continue;
        }
        let Some(outer_dependencies) = dependency
            .next
            .as_ref()
            .map(|dependencies| dependencies.as_ref().slice())
        else {
            continue;
        };
        if outer_dependencies.is_empty()
            || !outer_dependencies
                .iter()
                .all(moli_snapshot_relative_outer_dependency_supported)
        {
            continue;
        }
        verified_dependency_count += 1;
        moli_visit_relative_dependency_candidates(
            root,
            dependency,
            &sibling_traversal,
            |candidate| {
                for outer_dependency in outer_dependencies {
                    if moli_relative_dependency_changes_anchor(
                        outer_dependency,
                        candidate,
                        *scope,
                        snapshot_map,
                        quirks_mode,
                    ) {
                        push_snapshot_relative_outer_dependency_roots(
                            candidate,
                            outer_dependency,
                            *scope,
                            snapshot_map,
                            quirks_mode,
                            &mut roots,
                        );
                    }
                }
            },
        );
    }
    StyloSnapshotRelativeDependencyRoots::new(
        roots.into_iter().collect(),
        verified_dependency_count,
    )
}

fn push_snapshot_relative_outer_dependency_roots(
    candidate: StyleElement<'_>,
    dependency: &Dependency,
    scope: Option<OpaqueElement>,
    snapshot_map: &SnapshotMap,
    quirks_mode: QuirksMode,
    roots: &mut IndexSet<NodeId>,
) {
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
    let mut processor = StyloStyleInvalidationProcessor::new(
        matching_context,
        SiblingTraversalMap::default(),
        vec![dependency],
        Some(snapshot_map),
        StyloStyleInvalidationRootMapper,
    );
    TreeStyleInvalidator::new(candidate, None, &mut processor).invalidate();
    processor
        .into_query_result(0)
        .drain_affected_roots_into(roots);
}

fn ascii_whitespace_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut seen = HashSet::new();
    for token in value
        .split_ascii_whitespace()
        .filter(|token| !token.is_empty())
    {
        if seen.insert(token) {
            tokens.push(token.to_owned());
        }
    }
    tokens
}

fn stylo_retained_dependency_state_for_element(element: &Element) -> ElementState {
    let mut state = ElementState::empty();
    const HEADING_NAMES: [(&str, u64); 6] = [
        ("h1", 1),
        ("h2", 2),
        ("h3", 3),
        ("h4", 4),
        ("h5", 5),
        ("h6", 6),
    ];
    if let Some(level) = HEADING_NAMES
        .iter()
        .find_map(|(name, level)| element.is_html_element(name).then_some(*level))
    {
        state |= ElementState::from_bits_retain(level << HEADING_LEVEL_OFFSET);
    }
    if element.is_html_media() {
        state |= ElementState::PAUSED;
        if element.media_muted() {
            state |= ElementState::MUTED;
        }
        if element.media_seeking() {
            state |= ElementState::SEEKING;
        }
    }
    if element.popover_open() {
        state |= ElementState::POPOVER_OPEN;
    }
    state
}

impl StyleDomHostBinding<'_> {
    /// Return the current Stylo-visible state for an element.
    ///
    /// Renderer builds old-state snapshots by starting from this value and
    /// replacing only the state bits represented by a pending mutation.
    pub fn computed_element_state(&self, host: &DomHost, handle: NodeId) -> Option<ElementState> {
        Some(self.element(host, handle)?.as_query().computed_state())
    }

    /// Collect roots for one mutation query using Stylo's normal dependency
    /// invalidator only.
    ///
    /// Relative selector dependencies are intentionally excluded here so the
    /// renderer can combine this result with the dedicated Servo relative
    /// selector invalidator instead of treating all `:has(...)` dependencies as
    /// unsupported fallback.
    fn collect_normal_style_invalidation_roots_with_sibling_traversal_and_snapshots(
        &self,
        host: &DomHost,
        cascade_data: &CascadeData,
        root: NodeId,
        query: StyloStyleInvalidationQuery<'_>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
        snapshots: &[StyloStyleInvalidationSnapshot],
    ) -> StyloStyleInvalidationResult {
        let Some(element) = self.element(host, root) else {
            return StyloStyleInvalidationResult::default();
        };
        let previous_sibling = previous_sibling.and_then(|handle| self.element(host, handle));
        let next_sibling = next_sibling.and_then(|handle| self.element(host, handle));

        let mut dependencies = Vec::new();
        moli_collect_dependencies_from_invalidation_map(
            cascade_data.invalidation_map(),
            element,
            query,
            &mut dependencies,
        );
        let matched_dependency_count = dependencies.len();
        let (snapshot_map, snapshot_handles) =
            snapshot_map_for_moli_snapshots(self, host, snapshots);
        let snapshot_map = (!snapshot_handles.is_empty()).then_some(&snapshot_map);
        let quirks_mode = element.as_node().owner_doc().quirks_mode();
        let relative_dependencies = dependencies
            .iter()
            .filter(|dependency| moli_dependency_is_relative_selector(dependency))
            .map(|dependency| (None, *dependency))
            .collect::<Vec<_>>();
        let snapshot_relative_roots = snapshot_map
            .map(|snapshot_map| {
                collect_snapshot_relative_dependency_roots(
                    element,
                    &relative_dependencies,
                    snapshot_map,
                    quirks_mode,
                    previous_sibling,
                    next_sibling,
                )
            })
            .unwrap_or_default();
        let mut dependency_plan_effects =
            StyloNormalStyleInvalidationDependencyPlanEffects::default();
        moli_normal_style_invalidation_dependency_plan(
            query,
            matched_dependency_count,
            relative_dependencies.len(),
            &snapshot_relative_roots,
        )
        .drain_into(&mut dependency_plan_effects);
        if dependency_plan_effects.drop_relative_dependencies {
            dependencies.retain(|dependency| !moli_dependency_is_relative_selector(dependency));
        }
        if dependency_plan_effects.exact_empty_result {
            return StyloStyleInvalidationResult::exact_empty(matched_dependency_count);
        }

        let mut selector_caches = SelectorCaches::default();
        let matching_context = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut selector_caches,
            quirks_mode,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::Yes,
        );
        let traversal_map = if previous_sibling.is_some() || next_sibling.is_some() {
            SiblingTraversalMap::new(element, previous_sibling, next_sibling)
        } else {
            SiblingTraversalMap::default()
        };
        let mut processor = StyloStyleInvalidationProcessor::new(
            matching_context,
            traversal_map,
            dependencies,
            snapshot_map,
            StyloStyleInvalidationRootMapper,
        );
        snapshot_relative_roots.drain_affected_roots_into(&mut processor);
        self.with_snapshot_handles(snapshot_handles, || {
            TreeStyleInvalidator::new(element, None, &mut processor).invalidate();
        });

        processor.into_query_result(matched_dependency_count)
    }

    /// Collect roots for one mutation query using Servo's relative selector
    /// invalidator.
    fn collect_relative_style_invalidation_roots_with_sibling_traversal_and_snapshots(
        &self,
        host: &DomHost,
        stylist: &Stylist,
        root: NodeId,
        query: StyloStyleInvalidationQuery<'_>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
        snapshots: &[StyloStyleInvalidationSnapshot],
    ) -> StyloStyleInvalidationResult {
        let Some(element) = self.element(host, root) else {
            return StyloStyleInvalidationResult::default();
        };
        let previous_sibling = previous_sibling.and_then(|handle| self.element(host, handle));
        let next_sibling = next_sibling.and_then(|handle| self.element(host, handle));
        let quirks_mode = element.as_node().owner_doc().quirks_mode();
        let (snapshot_map, snapshot_handles) =
            snapshot_map_for_moli_snapshots(self, host, snapshots);
        let snapshot_map = (!snapshot_handles.is_empty()).then_some(&snapshot_map);
        let traversal_map = if previous_sibling.is_some() || next_sibling.is_some() {
            SiblingTraversalMap::new(element, previous_sibling, next_sibling)
        } else {
            SiblingTraversalMap::default()
        };

        self.with_snapshot_handles(snapshot_handles, || {
            moli_collect_relative_style_invalidation_query_result(
                StyloStyleInvalidationRootMapper,
                element,
                stylist,
                query,
                quirks_mode,
                snapshot_map,
                traversal_map,
                |snapshot_relative_dependencies| {
                    snapshot_map
                        .map(|snapshot_map| {
                            collect_snapshot_relative_dependency_roots(
                                element,
                                snapshot_relative_dependencies,
                                snapshot_map,
                                quirks_mode,
                                previous_sibling,
                                next_sibling,
                            )
                        })
                        .unwrap_or_default()
                },
            )
        })
    }

    /// Collect roots for one source-local invalidation query, merging Stylo's
    /// normal dependency invalidator and Servo's relative selector invalidator
    /// behind one Moli-facing result.
    fn collect_source_style_invalidation_roots_with_sibling_traversal_and_snapshots(
        &self,
        host: &DomHost,
        cascade_data: &CascadeData,
        stylist: &Stylist,
        root: NodeId,
        query: StyloStyleInvalidationQuery<'_>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
        snapshots: &[StyloStyleInvalidationSnapshot],
    ) -> StyloStyleInvalidationResult {
        let normal = self
            .collect_normal_style_invalidation_roots_with_sibling_traversal_and_snapshots(
                host,
                cascade_data,
                root,
                query,
                previous_sibling,
                next_sibling,
                snapshots,
            );
        let relative = self
            .collect_relative_style_invalidation_roots_with_sibling_traversal_and_snapshots(
                host,
                stylist,
                root,
                query,
                previous_sibling,
                next_sibling,
                snapshots,
            );
        stylo_merge_source_style_invalidation_query_results(normal, relative)
    }

    /// Collect roots for a batch of source-local invalidation queries.
    ///
    /// This keeps the exactness / empty-result / fallback interpretation beside
    /// the Stylo-facing invalidator instead of making renderer merge those
    /// booleans for every source.
    fn collect_source_style_invalidation_roots_for_queries_with_snapshots(
        &self,
        host: &DomHost,
        cascade_data: &CascadeData,
        stylist: &Stylist,
        queries: &IndexSet<StyloRetainedStyleInvalidationQuery>,
        exact_safety_fallback_roots: &IndexSet<NodeId>,
        zero_matched_dependency_result_is_exact: bool,
        snapshots: &[StyloStyleInvalidationSnapshot],
    ) -> StyloSourceStyleInvalidationResult {
        let mut accumulated = StyloSourceStyleInvalidationResultAccumulator::new();
        for retained_query in queries {
            let query: StyloSourceStyleInvalidationQuery<'_> = retained_query.as_source_query();
            let result = self
                .collect_source_style_invalidation_roots_with_sibling_traversal_and_snapshots(
                    host,
                    cascade_data,
                    stylist,
                    query.root(),
                    query.query(),
                    query.previous_sibling(),
                    query.next_sibling(),
                    snapshots,
                );
            accumulated.merge_invalidation_query_result(result);
        }
        accumulated.into_source_result_with_zero_matched_dependency_exactness(
            exact_safety_fallback_roots,
            zero_matched_dependency_result_is_exact,
        )
    }

    /// Collect roots for a retained source-aware invalidation batch.
    ///
    /// This is the renderer-facing boundary for retained invalidation. It keeps
    /// source cascade availability, shadow cascade setup, fallback-root
    /// application, and retained query conversion beside the Stylo-facing
    /// invalidator.
    pub fn collect_retained_source_style_invalidation_result<'a>(
        &self,
        host: &DomHost,
        stylist: Option<&Stylist>,
        shadow_cascade_data: &[(NodeId, ServoArc<CascadeData>)],
        sources: impl IntoIterator<Item = StyloRetainedSourceStyleInvalidation<'a>>,
    ) -> MoliInvalidationResult {
        let mut result = MoliInvalidationResultBuilder::new();
        if stylist.is_some() {
            for (root, cascade_data) in shadow_cascade_data {
                self.set_shadow_cascade_data(*root, cascade_data.clone());
            }
        }
        for (source_index, source) in sources.into_iter().enumerate() {
            let mut collector = StyloRetainedSourceStyleInvalidationCollector {
                binding: self,
                host,
                stylist,
                source_index,
                result: &mut result,
            };
            source.drain_into(&mut collector);
        }
        result.finish()
    }
}

struct StyloRetainedSourceStyleInvalidationCollector<
    'binding,
    'dom_binding,
    'host,
    'stylist,
    'result,
> {
    binding: &'binding StyleDomHostBinding<'dom_binding>,
    host: &'host DomHost,
    stylist: Option<&'stylist Stylist>,
    source_index: usize,
    result: &'result mut MoliInvalidationResultBuilder,
}

impl<'a, 'binding, 'dom_binding, 'host, 'stylist, 'result>
    MoliRetainedSourceStyleInvalidationSink<'a, NodeId, MoliStyleMutationSnapshot>
    for StyloRetainedSourceStyleInvalidationCollector<
        'binding,
        'dom_binding,
        'host,
        'stylist,
        'result,
    >
{
    fn run_retained_source_style_invalidation_queries(
        &mut self,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        cascade_data: Option<&'a ServoArc<CascadeData>>,
        shadow_root: Option<NodeId>,
        queries: &'a IndexSet<StyloRetainedStyleInvalidationQuery>,
        zero_matched_dependency_result_is_exact: bool,
        reasoned_fallback_roots: &'a IndexSet<NodeId>,
        exact_safety_fallback_roots: &'a IndexSet<NodeId>,
        fallback_reasons: &'a IndexSet<StyloSourceInvalidationFallbackReason>,
        mutation_snapshot: &'a MoliStyleMutationSnapshot,
    ) {
        let Some(stylist) = self.stylist else {
            self.result.push_missing_retained_style_system_source(
                self.source_index,
                fallback_reasons,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
            );
            return;
        };
        let Some(cascade_data) = cascade_data else {
            self.result.push_missing_retained_cascade_data_source(
                self.source_index,
                fallback_reasons,
                reasoned_fallback_roots,
                exact_safety_fallback_roots,
            );
            return;
        };
        if let Some(root) = shadow_root {
            self.binding
                .set_shadow_cascade_data(root, (*cascade_data).clone());
        }
        let snapshots = mutation_snapshot
            .to_stylo_snapshots(self.host)
            .unwrap_or_default();
        let source_result = self
            .binding
            .collect_source_style_invalidation_roots_for_queries_with_snapshots(
                self.host,
                cascade_data,
                stylist,
                queries,
                exact_safety_fallback_roots,
                zero_matched_dependency_result_is_exact,
                &snapshots,
            );
        self.result.push_source_result_from_planned_fallback(
            self.source_index,
            source_result,
            fallback_kind,
            reasoned_fallback_roots,
            fallback_reasons,
        );
    }

    fn run_fallback_source_style_invalidation(
        &mut self,
        kind: StyloRetainedSourceStyleInvalidationKind,
        fallback_roots: &'a IndexSet<NodeId>,
        fallback_reasons: &'a IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        self.result.push_fallback_only_source(
            self.source_index,
            kind,
            fallback_reasons,
            fallback_roots,
        );
    }
}

fn push_unique_root(roots: &mut Vec<NodeId>, seen: &mut HashSet<NodeId>, handle: NodeId) {
    if seen.insert(handle) {
        roots.push(handle);
    }
}

fn push_state_root(
    roots: &mut Vec<StyloStateInvalidationRoot>,
    seen: &mut HashSet<(NodeId, u64)>,
    root: NodeId,
    state: ElementState,
) {
    if seen.insert((root, state.bits())) {
        roots.push(StyloStateInvalidationRoot::new(root, state));
    }
}

fn push_ancestor_chain_roots(
    host: &DomHost,
    start: NodeId,
    roots: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    let mut current = Some(start);
    while let Some(handle) = current {
        if host
            .node(handle)
            .is_some_and(|node| node.as_element().is_some())
        {
            push_unique_root(roots, seen, handle);
        }
        current = style_parent_crossing_shadow_root(host, handle);
    }
}

fn push_following_sibling_roots(
    host: &DomHost,
    element: NodeId,
    roots: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    push_unique_root(roots, seen, element);
    let mut sibling = host.next_sibling(element);
    while let Some(handle) = sibling {
        push_unique_root(roots, seen, handle);
        sibling = host.next_sibling(handle);
    }
}

fn push_previous_sibling_roots(
    host: &DomHost,
    element: NodeId,
    roots: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    push_unique_root(roots, seen, element);
    let mut sibling = host.node(element).and_then(|node| node.prev_sibling());
    while let Some(handle) = sibling {
        push_unique_root(roots, seen, handle);
        sibling = host.node(handle).and_then(|node| node.prev_sibling());
    }
}

fn push_ancestor_previous_sibling_roots(
    host: &DomHost,
    element: NodeId,
    roots: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    let mut current = Some(element);
    while let Some(handle) = current {
        if host
            .node(handle)
            .is_some_and(|node| node.as_element().is_some())
        {
            push_previous_sibling_roots(host, handle, roots, seen);
        }
        current = style_parent_crossing_shadow_root(host, handle);
    }
}

fn push_ancestor_following_sibling_roots(
    host: &DomHost,
    element: NodeId,
    roots: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    let mut current = Some(element);
    while let Some(handle) = current {
        if host
            .node(handle)
            .is_some_and(|node| node.as_element().is_some())
        {
            push_following_sibling_roots(host, handle, roots, seen);
        }
        current = style_parent_crossing_shadow_root(host, handle);
    }
}

fn style_parent_crossing_shadow_root(host: &DomHost, handle: NodeId) -> Option<NodeId> {
    host.parent_node(handle).or_else(|| {
        if host.is_shadow_root(handle) {
            host.shadow_root_host(handle)
        } else {
            None
        }
    })
}

fn snapshot_map_for_moli_snapshots(
    binding: &StyleDomHostBinding<'_>,
    host: &DomHost,
    snapshots: &[StyloStyleInvalidationSnapshot],
) -> (SnapshotMap, Vec<NodeId>) {
    let mut snapshot_map = SnapshotMap::new();
    let mut handles = Vec::new();
    let mut seen_handles = HashSet::new();
    for snapshot in snapshots {
        let snapshot_element = snapshot.element();
        let Some(element) = binding.element(host, snapshot_element) else {
            continue;
        };
        snapshot_map.insert(
            element.as_node().opaque(),
            servo_snapshot_for_moli_snapshot(snapshot),
        );
        if seen_handles.insert(snapshot_element) {
            handles.push(snapshot_element);
        }
    }
    (snapshot_map, handles)
}

fn servo_snapshot_for_moli_snapshot(snapshot: &StyloStyleInvalidationSnapshot) -> Snapshot {
    Snapshot {
        state: snapshot.state(),
        custom_states: snapshot.custom_states().map(|states| {
            states
                .iter()
                .map(|state| AtomIdent::from(state.as_str()))
                .collect()
        }),
        attrs: Some(
            snapshot
                .attributes()
                .iter()
                .map(|attribute| {
                    (
                        AttrIdentifier {
                            local_name: LocalName::from(attribute.local_name()),
                            name: LocalName::from(attribute.name()),
                            namespace: Namespace::from(attribute.namespace()),
                            prefix: attribute.prefix().map(Prefix::from),
                        },
                        snapshot_attribute_value(attribute),
                    )
                })
                .collect(),
        ),
        changed_attrs: snapshot
            .changed_attributes()
            .iter()
            .map(|name| LocalName::from(name.as_str()))
            .collect(),
        class_changed: snapshot
            .changed_attributes()
            .iter()
            .any(|name| name == "class"),
        id_changed: snapshot
            .changed_attributes()
            .iter()
            .any(|name| name == "id"),
        other_attributes_changed: snapshot
            .changed_attributes()
            .iter()
            .any(|name| name != "class" && name != "id"),
    }
}

fn snapshot_attribute_value(attribute: &StyloStyleInvalidationSnapshotAttribute) -> AttrValue {
    if attribute.local_name() == "class" {
        return AttrValue::from_serialized_tokenlist(attribute.value().to_owned());
    }
    if attribute.local_name() == "id" {
        return AttrValue::from_atomic(attribute.value().to_owned());
    }
    if attribute.local_name() == "part" {
        return AttrValue::from_shadow_parts(attribute.value().to_owned());
    }
    AttrValue::from(attribute.value().to_owned())
}

#[cfg(test)]
mod tests {
    use dom::ElementState;

    use crate::StyloDomStyleAdapter;
    use crate::dom::{
        NodeId,
        native::{DomHost, NativeDom},
    };
    use indexmap::IndexSet;
    use style::moli_invalidation::{
        MoliSourceStyleInvalidationQueryResultBuilder as StyloStyleInvalidationResultBuilder,
        MoliSourceStyleInvalidationResultParts as StyloSourceStyleInvalidationResultParts,
        MoliSourceStyleInvalidationResultPartsSink as StyloSourceStyleInvalidationResultPartsSink,
        MoliSourceStyleInvalidationResultSink as StyloSourceStyleInvalidationResultSink,
    };

    use super::{
        CascadeData, MoliInvalidationResult, MoliInvalidationResultBuilder,
        MoliInvalidationSourceResultsSink, MoliRetainedSourceStyleInvalidationSink,
        MoliStyleMutationSnapshot, ServoArc, StyloPlannedFallbackRootInvalidationTarget,
        StyloPlannedFallbackRootInvalidationTargetPartsSink,
        StyloPlannedSourceDependencyInvalidation,
        StyloPlannedSourceDependencyInvalidationPartsSink,
        StyloPlannedSourceDependencyInvalidationTargetPartsSink,
        StyloRetainedSourceStyleInvalidation, StyloRetainedSourceStyleInvalidationKind,
        StyloRetainedStyleChildListInvalidationQueries,
        StyloRetainedStyleChildListInvalidationQueriesSink, StyloRetainedStyleInvalidationQuery,
        StyloRuntimeFallbackRootInput, StyloSourceAffectedRootsCleanup,
        StyloSourceAffectedRootsCleanupSink, StyloSourceDependencyBoundaryRoots,
        StyloSourceDependencyInvalidationBatchPlan, StyloSourceDependencyInvalidationBatchPlanSink,
        StyloSourceDependencyInvalidationBatchSource, StyloSourceDependencyInvalidationRequest,
        StyloSourceDependencyRequestRequirement, StyloSourceDependencySummary,
        StyloSourceFallbackRootAvailability, StyloSourceFallbackRootAvailabilitySummary,
        StyloSourceFallbackRootAvailabilitySummarySink, StyloSourceInvalidationFallbackReason,
        StyloSourceStyleInvalidationResult, StyloSourceStyleInvalidationResultAccumulator,
        StyloSourceStyleInvalidationSourceResult, StyloSourceStyleInvalidationSourceResultKind,
        StyloSourceStyleInvalidationSourceResultKindSummary,
        StyloSourceStyleInvalidationSourceResultKindSummarySink,
        StyloSourceStyleInvalidationSourceResultParts,
        StyloSourceStyleInvalidationSourceResultPartsSink,
        StyloSourceStyleInvalidationSourceResultSink,
        StyloSourceStyleInvalidationTargetResultCleanupFacts,
        StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink,
        StyloSourceStyleInvalidationTargetResultCleanupFactsSink,
        StyloSourceStyleInvalidationTargetResultDiagnosticFacts,
        StyloSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink,
        StyloSourceStyleInvalidationTargetResultDiagnosticFactsSink,
        StyloSourceStyleInvalidationTargetResultRecord, StyloStyleInvalidationQuery,
        StyloStyleSourceScope, StyloStylesheetSourceScopeFallbackInput,
        stylo_attribute_change_can_skip_fallback_without_dependency,
        stylo_attribute_change_can_use_retained_invalidator, stylo_element_dependency_snapshot,
        stylo_focus_change_invalidation_roots, stylo_merge_source_style_invalidation_query_results,
        stylo_removed_element_dependency_snapshots,
        stylo_retained_source_style_invalidation_from_parts,
        stylo_runtime_fallback_roots_for_mutation_inputs,
        stylo_runtime_or_source_scope_fallback_plan,
        stylo_source_dependency_invalidation_batch_plan,
        stylo_source_fallback_reason_for_unretained_state_change,
        stylo_source_invalidation_fallback_reason_plan,
        stylo_state_change_can_use_retained_invalidator,
        stylo_stylesheet_source_scope_fallback_roots,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct DiagnosticFactsForTest {
        kind: StyloSourceStyleInvalidationSourceResultKind,
        exact: bool,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
        fallback_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        fallback_root_availability: Option<StyloSourceFallbackRootAvailability>,
        affected_root_count: usize,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct CleanupFactsForTest {
        fallback_context_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        clear_all_cleanup_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        include_fallback_context_for_clear_all: bool,
        requires_fallback_handling: bool,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    struct RetainedSourceInputPartsForTest {
        retained_fallback_kind: Option<Option<StyloRetainedSourceStyleInvalidationKind>>,
        retained_query_count: usize,
        retained_zero_matched_dependency_result_is_exact: Option<bool>,
        retained_reasoned_fallback_roots: Vec<NodeId>,
        retained_exact_safety_fallback_roots: Vec<NodeId>,
        retained_fallback_reasons: Vec<StyloSourceInvalidationFallbackReason>,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        fallback_roots: Vec<NodeId>,
        fallback_reasons: Vec<StyloSourceInvalidationFallbackReason>,
    }

    impl<'a> MoliRetainedSourceStyleInvalidationSink<'a, NodeId, MoliStyleMutationSnapshot>
        for RetainedSourceInputPartsForTest
    {
        fn run_retained_source_style_invalidation_queries(
            &mut self,
            fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
            _cascade_data: Option<&'a ServoArc<CascadeData>>,
            _shadow_root: Option<NodeId>,
            queries: &'a IndexSet<StyloRetainedStyleInvalidationQuery>,
            zero_matched_dependency_result_is_exact: bool,
            reasoned_fallback_roots: &'a IndexSet<NodeId>,
            exact_safety_fallback_roots: &'a IndexSet<NodeId>,
            fallback_reasons: &'a IndexSet<StyloSourceInvalidationFallbackReason>,
            _mutation_snapshot: &'a MoliStyleMutationSnapshot,
        ) {
            self.retained_fallback_kind = Some(fallback_kind);
            self.retained_query_count = queries.len();
            self.retained_zero_matched_dependency_result_is_exact =
                Some(zero_matched_dependency_result_is_exact);
            self.retained_reasoned_fallback_roots
                .extend(reasoned_fallback_roots.iter().copied());
            self.retained_exact_safety_fallback_roots
                .extend(exact_safety_fallback_roots.iter().copied());
            self.retained_fallback_reasons
                .extend(fallback_reasons.iter().copied());
        }

        fn run_fallback_source_style_invalidation(
            &mut self,
            kind: StyloRetainedSourceStyleInvalidationKind,
            fallback_roots: &'a IndexSet<NodeId>,
            fallback_reasons: &'a IndexSet<StyloSourceInvalidationFallbackReason>,
        ) {
            self.fallback_kind = Some(kind);
            self.fallback_roots.extend(fallback_roots.iter().copied());
            self.fallback_reasons
                .extend(fallback_reasons.iter().copied());
        }
    }

    fn retained_source_input_parts_for_test(
        input: StyloRetainedSourceStyleInvalidation<'_>,
    ) -> RetainedSourceInputPartsForTest {
        let mut parts = RetainedSourceInputPartsForTest::default();
        input.drain_into(&mut parts);
        parts
    }

    #[derive(Default)]
    struct SourceResultDrainForTest {
        source_result_count: Option<usize>,
        source_indices: Vec<usize>,
        exact_roots: Vec<Vec<NodeId>>,
        source_fallback_roots: Vec<Vec<NodeId>>,
        diagnostic_facts: Vec<DiagnosticFactsForTest>,
        cleanup_facts: Vec<CleanupFactsForTest>,
        current_exact_roots: Vec<NodeId>,
        current_source_fallback_roots: Vec<NodeId>,
    }

    #[derive(Default)]
    struct SourceStyleInvalidationResultPartsForTest {
        affected_roots: Vec<NodeId>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
        fallback_kind: Option<StyloSourceStyleInvalidationSourceResultKind>,
        fallback_root_availability: Option<StyloSourceFallbackRootAvailability>,
        empty_result_is_exact: bool,
        matched_dependency_count: usize,
    }

    impl StyloSourceStyleInvalidationResultSink<NodeId> for SourceStyleInvalidationResultPartsForTest {
        fn set_source_style_invalidation_result(
            &mut self,
            parts: StyloSourceStyleInvalidationResultParts<NodeId>,
        ) {
            parts.drain_into(self);
        }
    }

    impl StyloSourceStyleInvalidationResultPartsSink<NodeId>
        for SourceStyleInvalidationResultPartsForTest
    {
        fn set_source_style_invalidation_result_parts(
            &mut self,
            affected_roots: Vec<NodeId>,
            fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
            fallback_kind: Option<StyloSourceStyleInvalidationSourceResultKind>,
            fallback_root_availability: Option<StyloSourceFallbackRootAvailability>,
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

    impl SourceResultDrainForTest {
        fn drain_source_style_invalidation_result(
            result: StyloSourceStyleInvalidationResult,
        ) -> SourceStyleInvalidationResultPartsForTest {
            let mut sink = SourceStyleInvalidationResultPartsForTest::default();
            result.drain_into(&mut sink);
            sink
        }

        fn drain_moli_result(result: MoliInvalidationResult) -> Self {
            let mut sink = Self::default();
            result.drain_source_results_into(&mut sink);
            sink
        }

        fn drain_builder(builder: MoliInvalidationResultBuilder) -> Self {
            Self::drain_moli_result(builder.finish())
        }

        fn drain_builder_source_result(
            build: impl FnOnce(&mut MoliInvalidationResultBuilder),
        ) -> Self {
            let mut builder = MoliInvalidationResultBuilder::new();
            build(&mut builder);
            Self::drain_builder(builder)
        }
    }

    impl MoliInvalidationSourceResultsSink<NodeId> for SourceResultDrainForTest {
        fn record_moli_invalidation_source_result_count(&mut self, count: usize) {
            self.source_result_count = Some(count);
        }

        fn record_moli_invalidation_source_result(
            &mut self,
            result: StyloSourceStyleInvalidationSourceResult,
        ) {
            result.drain_into(self);
        }
    }

    impl StyloSourceStyleInvalidationSourceResultSink<NodeId> for SourceResultDrainForTest {
        fn record_source_style_invalidation_source_result(
            &mut self,
            parts: StyloSourceStyleInvalidationSourceResultParts<NodeId>,
        ) {
            parts.drain_into(self);
        }
    }

    impl StyloSourceStyleInvalidationSourceResultPartsSink<NodeId> for SourceResultDrainForTest {
        fn record_source_style_invalidation_source_result_parts(
            &mut self,
            source_index: usize,
            affected_roots: StyloSourceAffectedRootsCleanup,
            target_result_record: StyloSourceStyleInvalidationTargetResultRecord,
        ) {
            self.source_indices.push(source_index);
            self.current_exact_roots.clear();
            self.current_source_fallback_roots.clear();
            affected_roots.drain_into(self);
            self.exact_roots.push(self.current_exact_roots.clone());
            self.source_fallback_roots
                .push(self.current_source_fallback_roots.clone());
            if let Some(diagnostic_facts) = target_result_record.drain_cleanup_into(self) {
                diagnostic_facts.drain_into(self);
            }
        }
    }

    impl StyloSourceAffectedRootsCleanupSink<NodeId> for SourceResultDrainForTest {
        fn extend_exact_affected_roots(&mut self, roots: &[NodeId]) {
            self.current_exact_roots.extend(roots.iter().copied());
        }

        fn extend_source_fallback_roots(&mut self, roots: &[NodeId]) {
            self.current_source_fallback_roots
                .extend(roots.iter().copied());
        }
    }

    impl StyloSourceStyleInvalidationTargetResultDiagnosticFactsSink for SourceResultDrainForTest {
        fn set_source_style_invalidation_target_result_diagnostic_facts(
            &mut self,
            facts: StyloSourceStyleInvalidationTargetResultDiagnosticFacts,
        ) {
            facts.drain_parts_into(self);
        }
    }

    impl StyloSourceStyleInvalidationTargetResultDiagnosticFactsPartsSink for SourceResultDrainForTest {
        fn set_source_style_invalidation_target_result_diagnostic_fact_parts(
            &mut self,
            kind: StyloSourceStyleInvalidationSourceResultKind,
            exact: bool,
            empty_result_is_exact: bool,
            matched_dependency_count: usize,
            fallback_reasons: Vec<StyloSourceInvalidationFallbackReason>,
            fallback_root_availability: Option<StyloSourceFallbackRootAvailability>,
            affected_root_count: usize,
        ) {
            self.diagnostic_facts.push(DiagnosticFactsForTest {
                kind,
                exact,
                empty_result_is_exact,
                matched_dependency_count,
                fallback_reasons,
                fallback_root_availability,
                affected_root_count,
            });
        }
    }

    impl StyloSourceStyleInvalidationTargetResultCleanupFactsSink for SourceResultDrainForTest {
        fn set_source_style_invalidation_target_result_cleanup_facts(
            &mut self,
            facts: StyloSourceStyleInvalidationTargetResultCleanupFacts,
        ) {
            facts.drain_parts_into(self);
        }
    }

    impl StyloSourceStyleInvalidationTargetResultCleanupFactsPartsSink for SourceResultDrainForTest {
        fn set_source_style_invalidation_target_result_cleanup_fact_parts(
            &mut self,
            fallback_context_reasons: Vec<StyloSourceInvalidationFallbackReason>,
            clear_all_cleanup_reasons: Vec<StyloSourceInvalidationFallbackReason>,
            include_fallback_context_for_clear_all: bool,
            requires_fallback_handling: bool,
        ) {
            self.cleanup_facts.push(CleanupFactsForTest {
                fallback_context_reasons,
                clear_all_cleanup_reasons,
                include_fallback_context_for_clear_all,
                requires_fallback_handling,
            });
        }
    }

    #[derive(Default)]
    struct PlannedFallbackRootTargetPartsForTest {
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        fallback_roots: Vec<NodeId>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    }

    #[derive(Default)]
    struct PlannedSourceDependencyPartsForTest {
        source_index: Option<usize>,
        structural_boundary_cleanup_roots: Vec<NodeId>,
        target_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
        exact_queries: Vec<StyloRetainedStyleInvalidationQuery>,
        reasoned_fallback_roots: Vec<NodeId>,
        exact_safety_fallback_roots: Vec<NodeId>,
        fallback_roots: Vec<NodeId>,
        fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
    }

    impl StyloPlannedFallbackRootInvalidationTargetPartsSink<NodeId>
        for PlannedFallbackRootTargetPartsForTest
    {
        fn set_planned_fallback_root_target_parts(
            &mut self,
            fallback_kind: StyloRetainedSourceStyleInvalidationKind,
            fallback_roots: Vec<NodeId>,
            fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
        ) {
            self.fallback_kind = Some(fallback_kind);
            self.fallback_roots = fallback_roots;
            self.fallback_reasons = fallback_reasons;
        }
    }

    impl StyloPlannedSourceDependencyInvalidationPartsSink<NodeId>
        for PlannedSourceDependencyPartsForTest
    {
        fn set_planned_source_dependency_source_index(&mut self, source_index: usize) {
            self.source_index = Some(source_index);
        }

        fn set_planned_source_dependency_structural_boundary_cleanup_roots(
            &mut self,
            structural_boundary_cleanup_roots: Vec<NodeId>,
        ) {
            self.structural_boundary_cleanup_roots = structural_boundary_cleanup_roots;
        }
    }

    impl StyloPlannedSourceDependencyInvalidationTargetPartsSink<NodeId>
        for PlannedSourceDependencyPartsForTest
    {
        fn set_planned_retained_source_dependency_target_parts(
            &mut self,
            exact_queries: Vec<StyloRetainedStyleInvalidationQuery>,
            fallback_kind: Option<StyloRetainedSourceStyleInvalidationKind>,
            reasoned_fallback_roots: Vec<NodeId>,
            exact_safety_fallback_roots: Vec<NodeId>,
            fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
        ) {
            self.target_kind = Some(StyloRetainedSourceStyleInvalidationKind::RetainedQueries);
            self.fallback_kind = fallback_kind;
            self.exact_queries = exact_queries;
            self.reasoned_fallback_roots = reasoned_fallback_roots;
            self.exact_safety_fallback_roots = exact_safety_fallback_roots;
            self.fallback_reasons = fallback_reasons;
        }

        fn set_planned_fallback_source_dependency_target_parts(
            &mut self,
            fallback_kind: StyloRetainedSourceStyleInvalidationKind,
            fallback_roots: Vec<NodeId>,
            fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
        ) {
            self.target_kind = Some(fallback_kind);
            self.fallback_roots = fallback_roots;
            self.fallback_reasons = fallback_reasons;
        }

        fn set_planned_missing_fallback_roots_source_dependency_target_parts(
            &mut self,
            fallback_reasons: IndexSet<StyloSourceInvalidationFallbackReason>,
        ) {
            self.target_kind = Some(StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots);
            self.fallback_reasons = fallback_reasons;
        }
    }

    fn planned_fallback_root_target_parts_for_test(
        target: StyloPlannedFallbackRootInvalidationTarget,
    ) -> (
        StyloRetainedSourceStyleInvalidationKind,
        Vec<NodeId>,
        IndexSet<StyloSourceInvalidationFallbackReason>,
    ) {
        let mut sink = PlannedFallbackRootTargetPartsForTest::default();
        target.drain_into(&mut sink);
        (
            sink.fallback_kind
                .expect("planned fallback-root target kind"),
            sink.fallback_roots,
            sink.fallback_reasons,
        )
    }

    fn planned_source_dependency_parts_for_test(
        planned: StyloPlannedSourceDependencyInvalidation,
    ) -> PlannedSourceDependencyPartsForTest {
        let mut sink = PlannedSourceDependencyPartsForTest::default();
        planned.drain_into(&mut sink);
        sink
    }

    #[derive(Default)]
    struct SourceDependencyBatchPlanPartsForTest {
        work_sources: Vec<StyloPlannedSourceDependencyInvalidation>,
        work_boundary_fallback: Option<StyloPlannedFallbackRootInvalidationTarget>,
        requires_source_fallback: Option<StyloPlannedSourceDependencyInvalidation>,
    }

    impl StyloSourceDependencyInvalidationBatchPlanSink<NodeId>
        for SourceDependencyBatchPlanPartsForTest
    {
        fn set_source_dependency_batch_work(
            &mut self,
            sources: Vec<StyloPlannedSourceDependencyInvalidation>,
            boundary_fallback: Option<StyloPlannedFallbackRootInvalidationTarget>,
        ) {
            self.work_sources = sources;
            self.work_boundary_fallback = boundary_fallback;
        }

        fn set_source_dependency_batch_requires_source_fallback(
            &mut self,
            source: StyloPlannedSourceDependencyInvalidation,
        ) {
            self.requires_source_fallback = Some(source);
        }
    }

    fn source_dependency_batch_plan_parts_for_test(
        plan: StyloSourceDependencyInvalidationBatchPlan,
    ) -> SourceDependencyBatchPlanPartsForTest {
        let mut sink = SourceDependencyBatchPlanPartsForTest::default();
        plan.drain_into(&mut sink);
        sink
    }

    #[derive(Default)]
    struct ChildListInvalidationBatchPartsForTest {
        rows: Vec<(
            StyloRetainedStyleInvalidationQuery,
            StyloSourceDependencyRequestRequirement,
        )>,
        base_roots: Vec<NodeId>,
        empty_target_fallback_roots: Vec<NodeId>,
        relative_previous_sibling_cleanup_roots: Vec<NodeId>,
    }

    impl StyloRetainedStyleChildListInvalidationQueriesSink<NodeId>
        for ChildListInvalidationBatchPartsForTest
    {
        fn record_child_list_retained_query(
            &mut self,
            query: StyloRetainedStyleInvalidationQuery,
            requirement: StyloSourceDependencyRequestRequirement,
        ) {
            self.rows.push((query, requirement));
        }

        fn extend_child_list_base_roots(&mut self, roots: Vec<NodeId>) {
            self.base_roots.extend(roots);
        }

        fn extend_child_list_empty_target_fallback_roots(&mut self, roots: Vec<NodeId>) {
            self.empty_target_fallback_roots.extend(roots);
        }

        fn extend_child_list_relative_previous_sibling_cleanup_roots(
            &mut self,
            roots: Vec<NodeId>,
        ) {
            self.relative_previous_sibling_cleanup_roots.extend(roots);
        }
    }

    fn child_list_batch_parts_for_test(
        queries: StyloRetainedStyleChildListInvalidationQueries,
    ) -> ChildListInvalidationBatchPartsForTest {
        let mut sink = ChildListInvalidationBatchPartsForTest::default();
        queries.drain_into(&mut sink);
        sink
    }

    #[test]
    fn moli_invalidation_result_default_has_empty_source_table() {
        let result = MoliInvalidationResult::default();
        let drained = SourceResultDrainForTest::drain_moli_result(result);

        assert_eq!(drained.source_result_count, Some(0));
        assert!(drained.source_indices.is_empty());
    }

    #[test]
    fn source_dependency_fallback_roots_select_cause_roots_inside_selector_plan() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let cause_root = host.create_element("section");
        assert!(host.append_child(document, cause_root));

        let source_roots = [document];
        let cause_roots = [cause_root];

        let summary = StyloSourceDependencySummary::conservative_child_list_structural();
        let query = StyloRetainedStyleInvalidationQuery::class(document, "active".into());
        let request = StyloSourceDependencyInvalidationRequest::new(
            &query,
            None,
            StyloSourceDependencyRequestRequirement::child_list_structural(),
        );

        let source_only =
            StyloSourceDependencyInvalidationBatchSource::new(&summary, &source_roots, &[]);
        let plan = stylo_source_dependency_invalidation_batch_plan(
            &host,
            &[source_only],
            std::slice::from_ref(&request),
            StyloSourceDependencyBoundaryRoots::default(),
        );
        let mut plan = source_dependency_batch_plan_parts_for_test(plan);
        assert!(plan.work_sources.is_empty());
        assert!(plan.work_boundary_fallback.is_none());
        let source_only_parts = planned_source_dependency_parts_for_test(
            plan.requires_source_fallback
                .take()
                .expect("missing boundary roots should force source fallback"),
        );
        assert_eq!(
            source_only_parts.target_kind,
            Some(StyloRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(source_only_parts.fallback_roots, source_roots.to_vec());

        let cause_scoped = StyloSourceDependencyInvalidationBatchSource::new(
            &summary,
            &source_roots,
            &cause_roots,
        );
        let plan = stylo_source_dependency_invalidation_batch_plan(
            &host,
            &[cause_scoped],
            std::slice::from_ref(&request),
            StyloSourceDependencyBoundaryRoots::default(),
        );
        let mut plan = source_dependency_batch_plan_parts_for_test(plan);
        assert!(plan.work_sources.is_empty());
        assert!(plan.work_boundary_fallback.is_none());
        let cause_scoped_parts = planned_source_dependency_parts_for_test(
            plan.requires_source_fallback
                .take()
                .expect("missing boundary roots should force source fallback"),
        );
        assert_eq!(
            cause_scoped_parts.target_kind,
            Some(StyloRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(cause_scoped_parts.fallback_roots, cause_roots.to_vec());
    }

    #[test]
    fn attribute_retained_invalidator_capability_comes_from_fork_policy() {
        assert!(stylo_attribute_change_can_use_retained_invalidator(
            "class", false
        ));
        assert!(stylo_attribute_change_can_use_retained_invalidator(
            "data-state",
            false
        ));
        assert!(stylo_attribute_change_can_use_retained_invalidator(
            "style", false
        ));
        assert!(!stylo_attribute_change_can_use_retained_invalidator(
            "width", true
        ));
        assert!(!stylo_attribute_change_can_use_retained_invalidator(
            "href", true
        ));

        assert!(stylo_attribute_change_can_skip_fallback_without_dependency(
            "class"
        ));
        assert!(stylo_attribute_change_can_skip_fallback_without_dependency(
            "data-state"
        ));
        assert!(stylo_attribute_change_can_skip_fallback_without_dependency(
            "aria-expanded"
        ));
        assert!(stylo_attribute_change_can_skip_fallback_without_dependency(
            "lang"
        ));
        assert!(stylo_attribute_change_can_skip_fallback_without_dependency(
            "dir"
        ));
        assert!(!stylo_attribute_change_can_skip_fallback_without_dependency("DATA-State"));
        assert!(!stylo_attribute_change_can_skip_fallback_without_dependency("href"));
    }

    #[test]
    fn runtime_fallback_roots_use_fork_policy_with_dom_resolver() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let skipped_attribute = host.create_element("article");
        let runtime_attribute = host.create_element("section");
        let added_child = host.create_element("span");
        let connected = host.create_element("main");
        let slot = host.create_element("slot");
        for root in [
            skipped_attribute,
            runtime_attribute,
            added_child,
            connected,
            slot,
        ] {
            assert!(host.append_child(document, root));
        }
        let added_nodes = [added_child];

        let roots = stylo_runtime_fallback_roots_for_mutation_inputs(
            &host,
            [
                StyloRuntimeFallbackRootInput::Attribute {
                    element: skipped_attribute,
                    attribute_name: "class",
                    has_dependency_change: true,
                    has_non_css_runtime_side_effect: false,
                },
                StyloRuntimeFallbackRootInput::Attribute {
                    element: runtime_attribute,
                    attribute_name: "width",
                    has_dependency_change: true,
                    has_non_css_runtime_side_effect: true,
                },
                StyloRuntimeFallbackRootInput::ChildList {
                    added_nodes: &added_nodes,
                },
                StyloRuntimeFallbackRootInput::OtherMutation,
                StyloRuntimeFallbackRootInput::SlotAssignment {
                    slot,
                    has_assignment_snapshot: false,
                },
                StyloRuntimeFallbackRootInput::ConnectedSubtree { root: connected },
            ],
        );

        assert!(!roots.contains(&skipped_attribute));
        assert!(roots.contains(&runtime_attribute));
        assert!(roots.contains(&added_child));
        assert!(roots.contains(&connected));
        assert!(roots.contains(&slot));

        let all_child_list_roots = stylo_runtime_fallback_roots_for_mutation_inputs(
            &host,
            [StyloRuntimeFallbackRootInput::ChildList {
                added_nodes: &added_nodes,
            }],
        );
        assert!(all_child_list_roots.is_empty());

        let known_slot_roots = stylo_runtime_fallback_roots_for_mutation_inputs(
            &host,
            [StyloRuntimeFallbackRootInput::SlotAssignment {
                slot,
                has_assignment_snapshot: true,
            }],
        );
        assert!(known_slot_roots.is_empty());
    }

    #[test]
    fn state_change_retained_invalidator_capability_comes_from_fork_policy() {
        for state in [
            ElementState::CHECKED,
            ElementState::INDETERMINATE,
            ElementState::PLACEHOLDER_SHOWN,
            ElementState::DEFINED,
            ElementState::PAUSED,
            ElementState::MUTED,
            ElementState::SEEKING,
        ] {
            assert!(stylo_state_change_can_use_retained_invalidator(state, None));
            assert_eq!(
                stylo_source_fallback_reason_for_unretained_state_change(state, None),
                None
            );
        }

        assert!(!stylo_state_change_can_use_retained_invalidator(
            ElementState::HOVER,
            None
        ));
        assert_eq!(
            stylo_source_fallback_reason_for_unretained_state_change(ElementState::HOVER, None),
            Some(StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
        );
        assert!(stylo_state_change_can_use_retained_invalidator(
            ElementState::HOVER,
            Some(ElementState::empty())
        ));
        assert_eq!(
            stylo_source_fallback_reason_for_unretained_state_change(
                ElementState::HOVER,
                Some(ElementState::empty())
            ),
            None
        );
    }

    #[test]
    fn runtime_or_source_scope_fallback_plan_preserves_kind_and_reasons() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let runtime_root = host.create_element("section");
        assert!(host.append_child(document, runtime_root));
        let source_scope = StyloStyleSourceScope::for_document(document);

        let source_scope_plan = stylo_runtime_or_source_scope_fallback_plan(
            &host,
            &source_scope,
            Vec::new(),
            [StyloSourceInvalidationFallbackReason::UnsupportedStateDependency],
        );
        let (source_scope_kind, source_scope_roots, source_scope_reasons) =
            planned_fallback_root_target_parts_for_test(source_scope_plan);
        assert_eq!(
            source_scope_kind,
            StyloRetainedSourceStyleInvalidationKind::SourceScopeFallback
        );
        assert!(source_scope_roots.contains(&document));
        assert!(
            source_scope_reasons
                .contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
        );
        assert!(
            source_scope_reasons
                .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
        );

        let runtime_plan = stylo_runtime_or_source_scope_fallback_plan(
            &host,
            &source_scope,
            vec![runtime_root],
            [StyloSourceInvalidationFallbackReason::UnsupportedStateDependency],
        );
        let (runtime_kind, runtime_roots, runtime_reasons) =
            planned_fallback_root_target_parts_for_test(runtime_plan);
        assert_eq!(
            runtime_kind,
            StyloRetainedSourceStyleInvalidationKind::FallbackOnly
        );
        assert_eq!(runtime_roots, vec![runtime_root]);
        assert!(
            !runtime_reasons.contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
        );
        assert!(
            runtime_reasons
                .contains(&StyloSourceInvalidationFallbackReason::UnsupportedStateDependency)
        );
    }

    #[test]
    fn stylesheet_source_scope_fallback_input_selects_owner_document_and_shadow_roots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let owner = host.create_element("style");
        let shadow_host = host.create_element("section");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("section should host a shadow root");
        assert!(host.append_child(document, owner));
        assert!(host.append_child(document, shadow_host));
        let source_scope =
            StyloStyleSourceScope::for_document_and_connected_shadow_roots(&host, document);

        assert_eq!(
            stylo_stylesheet_source_scope_fallback_roots(
                &host,
                StyloStylesheetSourceScopeFallbackInput::StylesheetOwner { owner },
                &source_scope,
            ),
            vec![document]
        );
        assert_eq!(
            stylo_stylesheet_source_scope_fallback_roots(
                &host,
                StyloStylesheetSourceScopeFallbackInput::DocumentAdopted { document },
                &source_scope,
            ),
            vec![document]
        );
        let shadow_roots = stylo_stylesheet_source_scope_fallback_roots(
            &host,
            StyloStylesheetSourceScopeFallbackInput::ShadowRootAdopted { root: shadow_root },
            &source_scope,
        );
        assert!(shadow_roots.contains(&shadow_root));
        assert!(shadow_roots.contains(&shadow_host));
        assert_eq!(
            stylo_stylesheet_source_scope_fallback_roots(
                &host,
                StyloStylesheetSourceScopeFallbackInput::Unscoped,
                &source_scope,
            ),
            Vec::<NodeId>::new()
        );
    }

    #[test]
    fn document_connected_shadow_scope_filters_roots_by_owner_document() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let document_host = host.create_element("section");
        assert!(host.append_child(document, document_host));
        let document_shadow_root = host
            .attach_shadow_root(document_host, "open")
            .expect("document host should attach a shadow root");

        let child_document = host.create_detached_html_document();
        let child_host = host.create_parser_element_without_attributes_for_document(
            child_document,
            "article".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(host.append_child(child_document, child_host));
        let child_shadow_root = host
            .attach_shadow_root(child_host, "open")
            .expect("child host should attach a shadow root");
        host.mark_subtree_connected_preserving_owner_document(child_document);

        let document_scope =
            StyloStyleSourceScope::for_document_and_connected_shadow_roots(&host, document);
        let child_scope =
            StyloStyleSourceScope::for_document_and_connected_shadow_roots(&host, child_document);

        assert!(document_scope.contains_shadow_root(&host, document_shadow_root));
        assert!(!document_scope.contains_shadow_root(&host, child_shadow_root));
        assert!(!child_scope.contains_shadow_root(&host, document_shadow_root));
        assert!(child_scope.contains_shadow_root(&host, child_shadow_root));
    }

    #[test]
    fn style_source_scope_for_handles_matches_incremental_include_handle() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let descendant_host = host.create_element("section");
        let descendant_child = host.create_element("span");
        let descendant_shadow_root = host
            .attach_shadow_root(descendant_host, "open")
            .expect("section should host a shadow root");
        let sibling_host = host.create_element("aside");
        let sibling_shadow_root = host
            .attach_shadow_root(sibling_host, "open")
            .expect("aside should host a shadow root");
        assert!(host.append_child(document, descendant_host));
        assert!(host.append_child(descendant_host, descendant_child));
        assert!(host.append_child(document, sibling_host));

        let batched = StyloStyleSourceScope::for_handles(&host, [descendant_child]);
        let mut incremental = StyloStyleSourceScope::default();
        incremental.include_handle(&host, descendant_child);

        assert_eq!(batched, incremental);
        assert!(batched.contains_document(document));
        assert!(batched.contains_shadow_root(&host, descendant_shadow_root));
        assert!(!batched.contains_shadow_root(&host, sibling_shadow_root));
    }

    #[test]
    fn moli_invalidation_result_builder_preserves_source_results() {
        let mut builder = MoliInvalidationResultBuilder::new();
        builder.push_fallback_source_result(
            0,
            StyloSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem,
            false,
            0,
            vec![StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem],
            None,
            Vec::new(),
        );

        let result = builder.finish();
        let drained = SourceResultDrainForTest::drain_moli_result(result);

        assert_eq!(drained.source_result_count, Some(1));
        assert_eq!(drained.diagnostic_facts.len(), 1);
        let facts = &drained.diagnostic_facts[0];
        assert_eq!(
            facts.kind,
            StyloSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem
        );
        assert_eq!(
            facts.fallback_reasons,
            vec![StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem]
        );
    }

    #[test]
    fn source_result_owner_builds_fallback_only_policy() {
        let root = NodeId::new(1);
        let drained = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_fallback_only_source(
                3,
                StyloRetainedSourceStyleInvalidationKind::SourceScopeFallback,
                &IndexSet::from([StyloSourceInvalidationFallbackReason::SourceScopeFallback]),
                &IndexSet::from([root]),
            );
        });

        assert_eq!(drained.source_indices, vec![3]);
        assert!(drained.exact_roots[0].is_empty());
        assert_eq!(drained.source_fallback_roots[0], vec![root]);
        let facts = &drained.diagnostic_facts[0];
        assert_eq!(
            facts.kind,
            StyloSourceStyleInvalidationSourceResultKind::SourceScopeFallback
        );
        assert_eq!(
            facts.fallback_reasons,
            vec![StyloSourceInvalidationFallbackReason::SourceScopeFallback]
        );
        assert_eq!(
            facts.fallback_root_availability,
            Some(StyloSourceFallbackRootAvailability::Available { root_count: 1 })
        );
    }

    #[test]
    fn source_result_owner_builds_unavailable_retained_policy() {
        let reasoned_root = NodeId::new(1);
        let exact_safety_root = NodeId::new(2);
        let drained = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_missing_retained_cascade_data_source(
                3,
                &IndexSet::from([StyloSourceInvalidationFallbackReason::FullSelector]),
                &IndexSet::from([reasoned_root]),
                &IndexSet::from([exact_safety_root]),
            );
        });

        assert_eq!(drained.source_indices, vec![3]);
        assert!(drained.exact_roots[0].is_empty());
        assert_eq!(
            drained.source_fallback_roots[0],
            vec![reasoned_root, exact_safety_root]
        );
        let facts = &drained.diagnostic_facts[0];
        assert_eq!(
            facts.kind,
            StyloSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData
        );
        assert_eq!(
            facts.fallback_reasons,
            vec![
                StyloSourceInvalidationFallbackReason::FullSelector,
                StyloSourceInvalidationFallbackReason::MissingRetainedCascadeData,
            ]
        );
        assert_eq!(
            facts.fallback_root_availability,
            Some(StyloSourceFallbackRootAvailability::Available { root_count: 2 })
        );
    }

    #[test]
    fn source_result_accumulator_reports_missing_fallback_roots() {
        let mut accumulated = StyloSourceStyleInvalidationResultAccumulator::new();
        let mut query_result = StyloStyleInvalidationResultBuilder::<NodeId>::new();
        query_result.note_fallback_reason(StyloSourceInvalidationFallbackReason::FullSelector);
        accumulated.merge_invalidation_query_result(query_result.into_query_result(1));

        let result = SourceResultDrainForTest::drain_source_style_invalidation_result(
            accumulated.into_source_result(&IndexSet::new()),
        );

        assert!(result.affected_roots.is_empty());
        assert_eq!(
            result.fallback_kind,
            Some(StyloSourceStyleInvalidationSourceResultKind::MissingFallbackRoots)
        );
        assert_eq!(
            result.fallback_root_availability,
            Some(StyloSourceFallbackRootAvailability::Missing)
        );
        assert!(result.empty_result_is_exact);
        assert_eq!(result.matched_dependency_count, 1);
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([
                StyloSourceInvalidationFallbackReason::FullSelector,
                StyloSourceInvalidationFallbackReason::MissingFallbackRoots,
            ])
        );
    }

    #[test]
    fn source_result_accumulator_uses_exact_safety_roots_for_source_fallback() {
        let exact_root = NodeId::new(1);
        let fallback_root = NodeId::new(2);
        let mut accumulated = StyloSourceStyleInvalidationResultAccumulator::new();
        let mut query_result = StyloStyleInvalidationResultBuilder::<NodeId>::new();
        query_result.note_affected_root(exact_root);
        query_result.note_fallback_reason(StyloSourceInvalidationFallbackReason::FullSelector);
        accumulated.merge_invalidation_query_result(query_result.into_query_result(1));

        let result = SourceResultDrainForTest::drain_source_style_invalidation_result(
            accumulated.into_source_result(&IndexSet::from([fallback_root])),
        );

        assert_eq!(result.affected_roots, vec![fallback_root]);
        assert_eq!(
            result.fallback_kind,
            Some(StyloSourceStyleInvalidationSourceResultKind::Fallback)
        );
        assert_eq!(
            result.fallback_root_availability,
            Some(StyloSourceFallbackRootAvailability::Available { root_count: 1 })
        );
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([StyloSourceInvalidationFallbackReason::FullSelector])
        );
    }

    #[test]
    fn source_result_accumulator_converts_empty_inexact_result_to_fallback_reason() {
        let fallback_root = NodeId::new(1);
        let mut accumulated = StyloSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_invalidation_query_result(
            StyloStyleInvalidationResultBuilder::<NodeId>::new().into_query_result(0),
        );

        let result = SourceResultDrainForTest::drain_source_style_invalidation_result(
            accumulated.into_source_result(&IndexSet::from([fallback_root])),
        );

        assert_eq!(result.affected_roots, vec![fallback_root]);
        assert_eq!(
            result.fallback_kind,
            Some(StyloSourceStyleInvalidationSourceResultKind::Fallback)
        );
        assert_eq!(
            result.fallback_reasons,
            IndexSet::from([StyloSourceInvalidationFallbackReason::InexactEmptyResult])
        );
    }

    #[test]
    fn source_result_owner_merges_planned_fallback_policy() {
        let exact_root = NodeId::new(1);
        let fallback_root = NodeId::new(2);
        let mut accumulated = StyloSourceStyleInvalidationResultAccumulator::new();
        let mut query_result = StyloStyleInvalidationResultBuilder::<NodeId>::new();
        query_result.note_affected_root(exact_root);
        accumulated.merge_invalidation_query_result(query_result.into_query_result(1));
        let source_result = accumulated.into_source_result(&IndexSet::new());
        let drained = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_source_result_from_planned_fallback(
                7,
                source_result,
                Some(StyloRetainedSourceStyleInvalidationKind::ContextFallback),
                &IndexSet::from([fallback_root]),
                &IndexSet::from([
                    StyloSourceInvalidationFallbackReason::NestedRelativeSelectorDependency,
                ]),
            );
        });

        assert_eq!(drained.source_indices, vec![7]);
        assert!(drained.exact_roots[0].is_empty());
        assert_eq!(
            drained.source_fallback_roots[0],
            vec![exact_root, fallback_root]
        );
        let facts = &drained.diagnostic_facts[0];
        assert_eq!(
            facts.kind,
            StyloSourceStyleInvalidationSourceResultKind::ContextFallback
        );
        assert!(!facts.exact);
        assert_eq!(
            facts.fallback_reasons,
            vec![StyloSourceInvalidationFallbackReason::NestedRelativeSelectorDependency]
        );
        assert_eq!(
            facts.fallback_root_availability,
            Some(StyloSourceFallbackRootAvailability::Available { root_count: 1 })
        );
    }

    #[derive(Default)]
    struct MissingFallbackRootAvailabilitySummaryForTest {
        count: usize,
    }

    impl StyloSourceFallbackRootAvailabilitySummarySink
        for MissingFallbackRootAvailabilitySummaryForTest
    {
        fn record_missing_fallback_roots_target(&mut self) {
            self.count += 1;
        }
    }

    #[test]
    fn fallback_root_availability_records_missing_summary_through_sink() {
        let mut summary = MissingFallbackRootAvailabilitySummaryForTest::default();

        StyloSourceFallbackRootAvailability::Available { root_count: 2 }
            .record_summary_into(&mut summary);
        assert_eq!(summary.count, 0);

        StyloSourceFallbackRootAvailability::Missing.record_summary_into(&mut summary);
        assert_eq!(summary.count, 1);
    }

    #[derive(Default)]
    struct SourceResultKindSummaryForTest {
        retained_unavailable: usize,
        source_scope_fallback: usize,
        context_fallback: usize,
    }

    impl StyloSourceStyleInvalidationSourceResultKindSummarySink for SourceResultKindSummaryForTest {
        fn record_retained_source_unavailable_target(&mut self) {
            self.retained_unavailable += 1;
        }

        fn record_source_scope_fallback_target(&mut self) {
            self.source_scope_fallback += 1;
        }

        fn record_context_fallback_target(&mut self) {
            self.context_fallback += 1;
        }
    }

    #[test]
    fn source_result_kind_records_summary_through_sink() {
        let mut summary = SourceResultKindSummaryForTest::default();

        StyloSourceStyleInvalidationSourceResultKind::Exact.record_summary_into(&mut summary);
        assert_eq!(summary.retained_unavailable, 0);
        assert_eq!(summary.source_scope_fallback, 0);
        assert_eq!(summary.context_fallback, 0);

        StyloSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem
            .record_summary_into(&mut summary);
        StyloSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData
            .record_summary_into(&mut summary);
        StyloSourceStyleInvalidationSourceResultKind::SourceScopeFallback
            .record_summary_into(&mut summary);
        StyloSourceStyleInvalidationSourceResultKind::ContextFallback
            .record_summary_into(&mut summary);

        assert_eq!(summary.retained_unavailable, 2);
        assert_eq!(summary.source_scope_fallback, 1);
        assert_eq!(summary.context_fallback, 1);
    }

    #[test]
    fn planned_fallback_targets_normalize_kind_specific_reasons() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let summary = StyloSourceDependencySummary::conservative_child_list_structural();
        let query = StyloRetainedStyleInvalidationQuery::class(document, "active".into());
        let request = StyloSourceDependencyInvalidationRequest::new(
            &query,
            None,
            StyloSourceDependencyRequestRequirement::child_list_structural(),
        );
        let source = StyloSourceDependencyInvalidationBatchSource::new(&summary, &[], &[]);
        let mut plan = source_dependency_batch_plan_parts_for_test(
            stylo_source_dependency_invalidation_batch_plan(
                &host,
                &[source],
                std::slice::from_ref(&request),
                StyloSourceDependencyBoundaryRoots::default(),
            ),
        );
        let missing_source_target = planned_source_dependency_parts_for_test(
            plan.requires_source_fallback
                .take()
                .expect("missing fallback roots should force source fallback"),
        );
        assert_eq!(
            missing_source_target.target_kind,
            Some(StyloRetainedSourceStyleInvalidationKind::MissingFallbackRoots)
        );
        assert!(
            missing_source_target
                .fallback_reasons
                .contains(&StyloSourceInvalidationFallbackReason::MissingFallbackRoots)
        );

        let source_scope = StyloStyleSourceScope::for_document(host.document_handle());
        let (_, _, fallback_reasons) = planned_fallback_root_target_parts_for_test(
            crate::stylo_source_scope_fallback_plan(&host, &source_scope, []),
        );
        assert!(
            fallback_reasons.contains(&StyloSourceInvalidationFallbackReason::SourceScopeFallback)
        );
    }

    #[test]
    fn retained_source_style_invalidation_from_parts_selects_selector_variant() {
        let root = NodeId::new(1);
        let mut queries = IndexSet::new();
        queries.insert(StyloRetainedStyleInvalidationQuery::universal(root));
        let reasoned_fallback_roots = IndexSet::from([root]);
        let exact_safety_fallback_roots = IndexSet::new();
        let fallback_reasons =
            IndexSet::from([StyloSourceInvalidationFallbackReason::FullSelector]);
        let mutation_snapshot = MoliStyleMutationSnapshot::default();

        let retained = stylo_retained_source_style_invalidation_from_parts(
            StyloRetainedSourceStyleInvalidationKind::RetainedQueries,
            Some(StyloRetainedSourceStyleInvalidationKind::ContextFallback),
            None,
            None,
            Some(&queries),
            true,
            &reasoned_fallback_roots,
            &exact_safety_fallback_roots,
            &fallback_reasons,
            &mutation_snapshot,
        );
        let retained = retained_source_input_parts_for_test(retained);
        assert_eq!(
            retained.retained_fallback_kind,
            Some(Some(
                StyloRetainedSourceStyleInvalidationKind::ContextFallback
            ))
        );
        assert_eq!(retained.retained_query_count, 1);
        assert_eq!(
            retained.retained_zero_matched_dependency_result_is_exact,
            Some(true)
        );
        assert_eq!(retained.retained_reasoned_fallback_roots, vec![root]);
        assert!(retained.retained_exact_safety_fallback_roots.is_empty());
        assert_eq!(
            retained.retained_fallback_reasons,
            vec![StyloSourceInvalidationFallbackReason::FullSelector]
        );

        let fallback = stylo_retained_source_style_invalidation_from_parts(
            StyloRetainedSourceStyleInvalidationKind::FallbackOnly,
            None,
            None,
            None,
            None,
            false,
            &reasoned_fallback_roots,
            &exact_safety_fallback_roots,
            &fallback_reasons,
            &mutation_snapshot,
        );
        let fallback = retained_source_input_parts_for_test(fallback);
        assert_eq!(
            fallback.fallback_kind,
            Some(StyloRetainedSourceStyleInvalidationKind::FallbackOnly)
        );
        assert_eq!(fallback.fallback_roots, vec![root]);
        assert_eq!(
            fallback.fallback_reasons,
            vec![StyloSourceInvalidationFallbackReason::FullSelector]
        );
    }

    #[test]
    fn source_result_reports_cleanup_policy() {
        let root = NodeId::new(1);
        let exact = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_exact_source_result(0, vec![root], true, 1);
        });
        assert_eq!(exact.exact_roots[0], vec![root]);
        assert!(exact.source_fallback_roots[0].is_empty());
        assert!(exact.cleanup_facts[0].clear_all_cleanup_reasons.is_empty());
        assert!(!exact.cleanup_facts[0].requires_fallback_handling);

        let fallback_only = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_fallback_source_result(
                0,
                StyloSourceStyleInvalidationSourceResultKind::FallbackOnly,
                false,
                0,
                Vec::new(),
                None,
                Vec::new(),
            );
        });
        assert!(fallback_only.exact_roots[0].is_empty());
        assert!(fallback_only.source_fallback_roots[0].is_empty());
        assert!(fallback_only.cleanup_facts[0].requires_fallback_handling);
        assert_eq!(
            fallback_only.cleanup_facts[0].clear_all_cleanup_reasons,
            vec![StyloSourceInvalidationFallbackReason::InexactEmptyResult]
        );

        let reasoned_fallback = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_fallback_source_result(
                0,
                StyloSourceStyleInvalidationSourceResultKind::Fallback,
                false,
                1,
                vec![StyloSourceInvalidationFallbackReason::FullSelector],
                None,
                Vec::new(),
            );
        });
        assert!(reasoned_fallback.source_fallback_roots[0].is_empty());
        assert_eq!(
            reasoned_fallback.cleanup_facts[0].clear_all_cleanup_reasons,
            vec![StyloSourceInvalidationFallbackReason::FullSelector]
        );

        let rooted_fallback = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_fallback_source_result(
                0,
                StyloSourceStyleInvalidationSourceResultKind::Fallback,
                false,
                1,
                vec![StyloSourceInvalidationFallbackReason::FullSelector],
                None,
                vec![root],
            );
        });
        assert_eq!(rooted_fallback.source_fallback_roots[0], vec![root]);
        assert!(
            rooted_fallback.cleanup_facts[0]
                .clear_all_cleanup_reasons
                .is_empty()
        );

        let context_fallback = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_fallback_source_result(
                0,
                StyloSourceStyleInvalidationSourceResultKind::ContextFallback,
                false,
                1,
                vec![StyloSourceInvalidationFallbackReason::NthOfDependency],
                Some(StyloSourceFallbackRootAvailability::Available { root_count: 1 }),
                vec![root],
            );
        });
        assert_eq!(context_fallback.source_fallback_roots[0], vec![root]);
        assert!(
            context_fallback.cleanup_facts[0]
                .clear_all_cleanup_reasons
                .is_empty()
        );
        assert!(!context_fallback.cleanup_facts[0].include_fallback_context_for_clear_all);

        let missing_retained_style_system =
            SourceResultDrainForTest::drain_builder_source_result(|builder| {
                builder.push_fallback_source_result(
                    0,
                    StyloSourceStyleInvalidationSourceResultKind::MissingRetainedStyleSystem,
                    false,
                    0,
                    vec![StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem],
                    Some(StyloSourceFallbackRootAvailability::Available { root_count: 1 }),
                    vec![root],
                );
            });
        assert!(
            missing_retained_style_system.cleanup_facts[0].include_fallback_context_for_clear_all
        );

        let missing_retained_cascade_data =
            SourceResultDrainForTest::drain_builder_source_result(|builder| {
                builder.push_fallback_source_result(
                    0,
                    StyloSourceStyleInvalidationSourceResultKind::MissingRetainedCascadeData,
                    false,
                    0,
                    vec![StyloSourceInvalidationFallbackReason::MissingRetainedCascadeData],
                    Some(StyloSourceFallbackRootAvailability::Available { root_count: 1 }),
                    vec![root],
                );
            });
        assert!(
            missing_retained_cascade_data.cleanup_facts[0].include_fallback_context_for_clear_all
        );

        let missing_fallback_roots =
            SourceResultDrainForTest::drain_builder_source_result(|builder| {
                builder.push_fallback_source_result(
                    0,
                    StyloSourceStyleInvalidationSourceResultKind::MissingFallbackRoots,
                    false,
                    1,
                    vec![StyloSourceInvalidationFallbackReason::FullSelector],
                    Some(StyloSourceFallbackRootAvailability::Missing),
                    Vec::new(),
                );
            });
        assert_eq!(
            missing_fallback_roots.cleanup_facts[0].clear_all_cleanup_reasons,
            vec![
                StyloSourceInvalidationFallbackReason::FullSelector,
                StyloSourceInvalidationFallbackReason::MissingFallbackRoots
            ]
        );
    }

    #[test]
    fn retained_source_batch_returns_one_result_per_source_input() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let detached_document = host.create_detached_html_document();
        let adapter = StyloDomStyleAdapter::new();

        adapter.with_bound_host(&host, |binding| {
            let first_fallback_roots = IndexSet::from([document]);
            let first_fallback_reasons =
                IndexSet::from([StyloSourceInvalidationFallbackReason::UnsupportedDependency]);
            let second_fallback_roots = IndexSet::from([detached_document]);
            let second_fallback_reasons =
                IndexSet::from([StyloSourceInvalidationFallbackReason::FullSelector]);
            let exact_safety_fallback_roots = IndexSet::new();
            let mutation_snapshot = MoliStyleMutationSnapshot::default();
            let sources = [
                stylo_retained_source_style_invalidation_from_parts(
                    StyloRetainedSourceStyleInvalidationKind::FallbackOnly,
                    None,
                    None,
                    None,
                    None,
                    false,
                    &first_fallback_roots,
                    &exact_safety_fallback_roots,
                    &first_fallback_reasons,
                    &mutation_snapshot,
                ),
                stylo_retained_source_style_invalidation_from_parts(
                    StyloRetainedSourceStyleInvalidationKind::FallbackOnly,
                    None,
                    None,
                    None,
                    None,
                    false,
                    &second_fallback_roots,
                    &exact_safety_fallback_roots,
                    &second_fallback_reasons,
                    &mutation_snapshot,
                ),
            ];

            let result = binding.collect_retained_source_style_invalidation_result(
                &host,
                None,
                &[],
                sources.iter().copied(),
            );
            let drained = SourceResultDrainForTest::drain_moli_result(result);

            assert_eq!(drained.source_result_count, Some(sources.len()));
            assert_eq!(drained.source_indices, vec![0, 1]);
            assert!(drained.exact_roots[0].is_empty());
            assert_eq!(drained.source_fallback_roots[0], vec![document]);
            assert!(drained.exact_roots[1].is_empty());
            assert_eq!(drained.source_fallback_roots[1], vec![detached_document]);
            assert_eq!(
                drained.diagnostic_facts[0].fallback_reasons,
                vec![StyloSourceInvalidationFallbackReason::UnsupportedDependency]
            );
            assert_eq!(
                drained.diagnostic_facts[1].fallback_reasons,
                vec![StyloSourceInvalidationFallbackReason::FullSelector]
            );
        });
    }

    #[test]
    fn merged_invalidation_result_preserves_fallback_reasons() {
        let mut first = StyloStyleInvalidationResultBuilder::<NodeId>::new();
        first.note_fallback_reason(StyloSourceInvalidationFallbackReason::FullSelector);
        let first = first.into_query_result(2);
        let mut second = StyloStyleInvalidationResultBuilder::<NodeId>::new();
        second.note_fallback_reason(StyloSourceInvalidationFallbackReason::RelativeAnySelector);
        let second = second.into_query_result(0);

        let merged = stylo_merge_source_style_invalidation_query_results(first, second);
        let mut accumulated = StyloSourceStyleInvalidationResultAccumulator::new();
        accumulated.merge_invalidation_query_result(merged);
        let source_result = SourceResultDrainForTest::drain_source_style_invalidation_result(
            accumulated.into_source_result(&IndexSet::from([NodeId::new(1)])),
        );

        assert_eq!(
            source_result.fallback_reasons,
            IndexSet::from([
                StyloSourceInvalidationFallbackReason::FullSelector,
                StyloSourceInvalidationFallbackReason::RelativeAnySelector
            ])
        );
        assert_eq!(source_result.matched_dependency_count, 2);
    }

    #[test]
    fn source_result_missing_roots_preserves_dependency_fallback_reasons() {
        let drained = SourceResultDrainForTest::drain_builder_source_result(|builder| {
            builder.push_fallback_source_result(
                0,
                StyloSourceStyleInvalidationSourceResultKind::MissingFallbackRoots,
                false,
                1,
                vec![
                    StyloSourceInvalidationFallbackReason::FullSelector,
                    StyloSourceInvalidationFallbackReason::MissingFallbackRoots,
                ],
                Some(StyloSourceFallbackRootAvailability::Missing),
                Vec::new(),
            );
        });

        assert_eq!(
            drained.diagnostic_facts[0].fallback_reasons,
            vec![
                StyloSourceInvalidationFallbackReason::FullSelector,
                StyloSourceInvalidationFallbackReason::MissingFallbackRoots,
            ]
        );
    }

    #[test]
    fn source_fallback_reason_audit_covers_current_stylo_reasons() {
        let reasons = [
            StyloSourceInvalidationFallbackReason::UnknownDependency,
            StyloSourceInvalidationFallbackReason::FullSelector,
            StyloSourceInvalidationFallbackReason::RelativeAnySelector,
            StyloSourceInvalidationFallbackReason::ScopeDependency,
            StyloSourceInvalidationFallbackReason::UnsupportedStateDependency,
            StyloSourceInvalidationFallbackReason::UnsupportedShadowDependency,
            StyloSourceInvalidationFallbackReason::SourceScopeFallback,
            StyloSourceInvalidationFallbackReason::UnsupportedDependency,
            StyloSourceInvalidationFallbackReason::NthOfDependency,
            StyloSourceInvalidationFallbackReason::NestedRelativeSelectorDependency,
            StyloSourceInvalidationFallbackReason::InexactEmptyResult,
            StyloSourceInvalidationFallbackReason::MissingFallbackRoots,
            StyloSourceInvalidationFallbackReason::MissingRetainedStyleSystem,
            StyloSourceInvalidationFallbackReason::MissingRetainedCascadeData,
        ];
        let mut seen = IndexSet::new();

        for reason in reasons {
            assert!(seen.insert(reason), "duplicate fallback reason in audit");
            let plan = stylo_source_invalidation_fallback_reason_plan(reason);
            assert!(!plan.owner.is_empty(), "missing owner for {reason:?}");
            assert!(
                !plan.missing_fact.is_empty(),
                "missing exactness gap for {reason:?}"
            );
            assert!(
                !plan.next_work_item.is_empty(),
                "missing next work item for {reason:?}"
            );
        }
    }

    #[test]
    fn active_light_tree_handle_scope_includes_related_connected_shadow_roots() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let document = host.document_handle();
        let body = host.document_body_handle().expect("document body");
        let ancestor = host.create_element("section");
        let descendant_host = host.create_element("article");
        let descendant_shadow_root = host
            .attach_shadow_root(descendant_host, "open")
            .expect("descendant host should support shadow root");
        let light_child = host.create_element("span");
        let sibling_host = host.create_element("aside");
        let sibling_shadow_root = host
            .attach_shadow_root(sibling_host, "open")
            .expect("sibling host should support shadow root");
        let detached_document = host.create_detached_html_document();
        let detached_host = host.create_element("nav");
        let detached_shadow_root = host
            .attach_shadow_root(detached_host, "open")
            .expect("detached host should support shadow root");

        assert!(host.append_child(body, ancestor));
        assert!(host.append_child(ancestor, descendant_host));
        assert!(host.append_child(descendant_host, light_child));
        assert!(host.append_child(body, sibling_host));
        assert!(host.append_child(detached_document, detached_host));

        let ancestor_scope = StyloStyleSourceScope::for_handle(&host, ancestor);

        assert!(ancestor_scope.contains_document(document));
        assert!(ancestor_scope.contains_shadow_root(&host, descendant_shadow_root));
        assert!(!ancestor_scope.contains_shadow_root(&host, sibling_shadow_root));
        assert!(!ancestor_scope.contains_shadow_root(&host, detached_shadow_root));

        let child_scope = StyloStyleSourceScope::for_handle(&host, light_child);
        assert!(child_scope.contains_document(document));
        assert!(child_scope.contains_shadow_root(&host, descendant_shadow_root));
        assert!(!child_scope.contains_shadow_root(&host, sibling_shadow_root));
        assert!(!child_scope.contains_shadow_root(&host, detached_shadow_root));
    }

    #[test]
    fn retained_mutation_snapshot_merge_by_handle_and_attribute() {
        let element = NodeId::new(1);
        let mut first = MoliStyleMutationSnapshot::default();
        first.record_attribute_change(element, "class", Some("initial".into()));
        first.record_attribute_change(element, "class", Some("middle".into()));

        let mut second = MoliStyleMutationSnapshot::default();
        second.record_attribute_change(element, "class", Some("late".into()));
        second.record_attribute_change(element, "id", Some("old-id".into()));

        first.merge_from(second);

        let input = first
            .inputs
            .get(&element)
            .expect("merged element input should exist");
        let attribute_changes = input.attribute_changes().collect::<Vec<_>>();
        assert_eq!(first.len(), 1);
        assert_eq!(input.attribute_change_count(), 2);
        assert_eq!(attribute_changes.len(), 2);
        assert_eq!(
            attribute_changes
                .iter()
                .find(|change| change.name() == "class")
                .and_then(|change| change.old_value()),
            Some("initial")
        );
        assert_eq!(
            attribute_changes
                .iter()
                .find(|change| change.name() == "id")
                .and_then(|change| change.old_value()),
            Some("old-id")
        );
    }

    #[test]
    fn retained_mutation_snapshot_materialization_keeps_first_old_attribute_value() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        let element = host.create_element("div");
        assert!(host.set_attribute(element, "class", "final"));

        let mut snapshot = MoliStyleMutationSnapshot::default();
        snapshot.record_attribute_change(element, "class", Some("initial".into()));
        snapshot.record_attribute_change(element, "class", Some("middle".into()));

        let materialized = snapshot
            .to_stylo_snapshots(&host)
            .expect("element should have snapshot")
            .into_iter()
            .next()
            .expect("one snapshot should be produced");
        let class_attribute = materialized
            .attributes()
            .iter()
            .find(|attribute| attribute.local_name().eq_ignore_ascii_case("class"))
            .expect("class attribute should be represented");

        assert_eq!(materialized.changed_attributes(), ["class"]);
        assert_eq!(class_attribute.value(), "initial");
    }

    #[test]
    fn retained_mutation_snapshot_merge_materialization_preserves_first_old_attribute_value() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        let element = host.create_element("div");
        assert!(host.set_attribute(element, "class", "final"));

        let mut first = MoliStyleMutationSnapshot::default();
        first.record_attribute_change(element, "class", Some("initial".into()));
        let mut second = MoliStyleMutationSnapshot::default();
        second.record_attribute_change(element, "class", Some("middle".into()));

        first.merge_from(second);

        assert_eq!(first.len(), 1);
        let materialized = first
            .to_stylo_snapshots(&host)
            .expect("element should have snapshot")
            .into_iter()
            .next()
            .expect("one snapshot should be produced");
        let class_attribute = materialized
            .attributes()
            .iter()
            .find(|attribute| attribute.local_name().eq_ignore_ascii_case("class"))
            .expect("class attribute should be represented");

        assert_eq!(materialized.changed_attributes(), ["class"]);
        assert_eq!(class_attribute.value(), "initial");
    }

    #[test]
    fn retained_mutation_snapshot_materialization_preserves_case_sensitive_attribute_names() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        let element = host
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg:rect")
            .expect("svg element should be created");
        assert!(host.set_attribute(element, "data-state", "lower-final"));
        assert!(host.set_attribute(element, "DATA-State", "upper-final"));

        let mut snapshot = MoliStyleMutationSnapshot::default();
        snapshot.record_attribute_change(element, "data-state", Some("lower-initial".into()));
        snapshot.record_attribute_change(element, "DATA-State", Some("upper-initial".into()));

        let materialized = snapshot
            .to_stylo_snapshots(&host)
            .expect("element should have snapshot")
            .into_iter()
            .next()
            .expect("one snapshot should be produced");
        let lower_attribute = materialized
            .attributes()
            .iter()
            .find(|attribute| attribute.local_name() == "data-state")
            .expect("lowercase attribute should be represented");
        let upper_attribute = materialized
            .attributes()
            .iter()
            .find(|attribute| attribute.local_name() == "DATA-State")
            .expect("uppercase attribute should be represented");

        assert_eq!(
            materialized.changed_attributes(),
            ["data-state", "DATA-State"]
        );
        assert_eq!(lower_attribute.value(), "lower-initial");
        assert_eq!(upper_attribute.value(), "upper-initial");
    }

    #[test]
    fn retained_mutation_snapshot_owns_child_list_before_state() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let parent = host.create_element("div");
        let previous = host.create_element("span");
        let removed = host.create_element("em");
        let next = host.create_element("strong");
        assert!(host.set_attribute(removed, "class", "removed"));
        assert!(host.append_child(body, parent));
        assert!(host.append_child(parent, previous));
        assert!(host.append_child(parent, removed));
        assert!(host.append_child(parent, next));

        let removed_snapshot =
            stylo_element_dependency_snapshot(&host, removed).expect("removed element snapshot");
        let mut snapshot = MoliStyleMutationSnapshot::default();
        snapshot.record_child_list_mutation(
            parent,
            &[],
            &[removed],
            &[removed_snapshot],
            Some(previous),
            Some(next),
        );

        let queries = snapshot
            .child_list_invalidation_queries(&host)
            .expect("child-list mutation should build retained queries");
        let parts = child_list_batch_parts_for_test(queries);
        let removed_class_query = parts
            .rows
            .iter()
            .find(|query| {
                query.0.root() == removed
                    && matches!(
                        query.0.as_stylo_query(),
                        StyloStyleInvalidationQuery::Class(token) if token == "removed"
                    )
            })
            .expect("removed element class query should come from mutation snapshot");
        let context = snapshot
            .child_list_dependency_fallback_context(&removed_class_query.0)
            .expect("fallback context should come from mutation snapshot");

        assert_eq!(snapshot.len(), 1);
        assert!(parts.base_roots.contains(&next));
        assert_eq!(context.parent(), Some(parent));
        assert_eq!(context.previous_sibling(), Some(previous));
        assert_eq!(context.next_sibling(), Some(next));
    }

    #[test]
    fn child_list_invalidation_queries_include_inserted_subtree_descendants() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let parent = host.create_element("div");
        let wrapper = host.create_element("section");
        let descendant = host.create_element("span");
        assert!(host.set_attribute(descendant, "class", "descendant"));
        assert!(host.append_child(body, parent));
        assert!(host.append_child(wrapper, descendant));
        assert!(host.append_child(parent, wrapper));

        let mut snapshot = MoliStyleMutationSnapshot::default();
        snapshot.record_child_list_mutation(parent, &[wrapper], &[], &[], None, None);

        let queries = snapshot
            .child_list_invalidation_queries(&host)
            .expect("child-list mutation should build retained queries");
        let parts = child_list_batch_parts_for_test(queries);

        assert!(!parts.base_roots.contains(&wrapper));
        assert!(parts.rows.iter().any(|query| {
            query.0.root() == descendant
                && query.1 == StyloSourceDependencyRequestRequirement::exact()
                && matches!(
                    query.0.as_stylo_query(),
                    StyloStyleInvalidationQuery::Class(token) if token == "descendant"
                )
        }));
    }

    #[test]
    fn child_list_invalidation_context_covers_inserted_previous_sibling() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let parent = host.create_element("div");
        let previous = host.create_element("span");
        let inserted = host.create_element("span");
        assert!(host.append_child(body, parent));
        assert!(host.append_child(parent, previous));
        assert!(host.append_child(parent, inserted));

        let mut snapshot = MoliStyleMutationSnapshot::default();
        snapshot.record_child_list_mutation(parent, &[inserted], &[], &[], Some(previous), None);

        let queries = snapshot
            .child_list_invalidation_queries(&host)
            .expect("child-list mutation should build retained queries");
        let parts = child_list_batch_parts_for_test(queries);
        assert!(parts.base_roots.contains(&previous));
        assert!(parts.empty_target_fallback_roots.contains(&previous));
        assert!(
            parts.rows.iter().all(|query| query.0.root() != previous),
            "previous sibling should be a direct cleanup root, not a source dependency query"
        );
        let inserted_query = parts
            .rows
            .iter()
            .find(|query| query.0.root() == inserted)
            .expect("inserted node query should keep child-list fallback context");
        let context = snapshot
            .child_list_dependency_fallback_context(&inserted_query.0)
            .expect("inserted query should keep child-list fallback context");

        assert_eq!(context.parent(), Some(parent));
        assert_eq!(context.previous_sibling(), Some(previous));
        assert_eq!(context.next_sibling(), None);
    }

    #[test]
    fn removed_element_dependency_snapshots_include_removed_subtree_descendants() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let parent = host.create_element("div");
        let wrapper = host.create_element("section");
        let descendant = host.create_element("span");
        assert!(host.set_attribute(descendant, "class", "descendant"));
        assert!(host.append_child(body, parent));
        assert!(host.append_child(parent, wrapper));
        assert!(host.append_child(wrapper, descendant));

        let snapshots = stylo_removed_element_dependency_snapshots(&host, &[wrapper]);

        assert!(
            snapshots
                .iter()
                .any(|snapshot| snapshot.handle() == wrapper)
        );
        assert!(snapshots.iter().any(|snapshot| {
            snapshot.handle() == descendant
                && snapshot
                    .class_tokens()
                    .iter()
                    .any(|token| token == "descendant")
        }));
    }

    #[test]
    fn focus_change_invalidation_roots_include_shadow_hosts_matching_focus_state() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        host.reset_html_document_shell();
        let body = host.document_body_handle().expect("document body");
        let shadow_host = host.create_element("x-menu");
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow host should support shadow root");
        let focused = host.create_element("button");

        assert!(host.append_child(body, shadow_host));
        assert!(host.append_child(shadow_root, focused));

        let roots = stylo_focus_change_invalidation_roots(&host, None, Some(focused));

        assert!(roots.iter().any(|root| {
            root.root() == focused
                && root
                    .state()
                    .contains(ElementState::FOCUS | ElementState::FOCUSRING)
        }));
        assert!(roots.iter().any(|root| {
            root.root() == shadow_host
                && root
                    .state()
                    .contains(ElementState::FOCUS | ElementState::FOCUSRING)
        }));
    }
}
