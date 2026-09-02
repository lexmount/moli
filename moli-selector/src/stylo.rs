use std::collections::HashMap;

use selectors::{
    Element as SelectorsElement,
    matching::MatchingContext,
    parser::{Component, RelativeSelector},
    visitor::SelectorVisitor,
};
use style::{
    dom::TDocument,
    dom_apis::{
        MayUseInvalidation, QueryAll, QueryFirst, element_closest, element_matches, query_selector,
    },
    selector_parser::{NonTSPseudoClass, SelectorImpl},
    shared_lock::SharedRwLock,
};

use crate::{
    dom::{NodeId, native::DomHost},
    selector::SelectorError,
};

/// Returns heading elements below `root` in flat-tree order.
///
/// Renderer state invalidation uses the same traversal as `:heading()` so a
/// `headingoffset` or `headingreset` mutation cannot leave computed styles on
/// shadow/slotted descendants stale.
pub fn stylo_flat_tree_heading_descendants(host: &DomHost, root: NodeId) -> Vec<NodeId> {
    query::flat_tree_heading_descendants(host, root)
}

/// Reports whether a DOM API selector contains `:valid` or `:invalid`,
/// including escaped names and pseudos nested inside functional selectors.
pub fn dom_api_selector_uses_validity_pseudo(host: &DomHost, selector: &str) -> bool {
    let Ok(ParsedDomApiSelectorList::Parsed(selectors)) =
        parse_dom_api_selector_list(host, selector)
    else {
        return false;
    };
    let mut visitor = ValidityPseudoVisitor { found: false };
    for selector in selectors.slice() {
        if !selector.visit(&mut visitor) {
            break;
        }
    }
    visitor.found
}

struct ValidityPseudoVisitor {
    found: bool,
}

impl SelectorVisitor for ValidityPseudoVisitor {
    type Impl = SelectorImpl;

    fn visit_simple_selector(&mut self, component: &Component<Self::Impl>) -> bool {
        self.found = matches!(
            component,
            Component::NonTSPseudoClass(NonTSPseudoClass::Valid | NonTSPseudoClass::Invalid)
        );
        !self.found
    }

    fn visit_relative_selector_list(&mut self, selectors: &[RelativeSelector<Self::Impl>]) -> bool {
        selectors
            .iter()
            .all(|selector| selector.selector.visit(self))
    }
}

mod atoms;
mod invalidation;
mod presentation;
mod presentational_hints;
mod query;
mod selector_parse;
mod style_traversal;
use atoms::QueryAtomCache;
pub use invalidation::{
    MoliInvalidationResult, MoliInvalidationResultBuilder, MoliInvalidationSourceResultsSink,
    MoliStyleMutationElementSnapshot, MoliStyleMutationSnapshot,
    StyloDependencyInvalidationFallbackContext, StyloElementDependencySnapshot,
    StyloPlannedFallbackRootInvalidationTarget,
    StyloPlannedFallbackRootInvalidationTargetPartsSink, StyloPlannedSourceDependencyInvalidation,
    StyloPlannedSourceDependencyInvalidationPartsSink,
    StyloPlannedSourceDependencyInvalidationTarget,
    StyloPlannedSourceDependencyInvalidationTargetPartsSink, StyloRetainedSourceStyleInvalidation,
    StyloRetainedSourceStyleInvalidationKind, StyloRetainedSourceStyleInvalidationSink,
    StyloRetainedStyleChildListInvalidationQueries,
    StyloRetainedStyleChildListInvalidationQueriesSink,
    StyloRetainedStyleChildListInvalidationQuery, StyloRetainedStyleInvalidationQuery,
    StyloRetainedStyleSiblingTraversal, StyloRuntimeFallbackRootInput,
    StyloSourceAffectedRootsCleanup, StyloSourceAffectedRootsCleanupSink,
    StyloSourceDependencyBoundaryRoots, StyloSourceDependencyInvalidationBatchPlan,
    StyloSourceDependencyInvalidationBatchPlanSink, StyloSourceDependencyInvalidationBatchSource,
    StyloSourceDependencyInvalidationRequest, StyloSourceDependencyRequestRequirement,
    StyloSourceDependencySummary, StyloSourceFallbackRootAvailability,
    StyloSourceFallbackRootAvailabilitySummary, StyloSourceFallbackRootAvailabilitySummarySink,
    StyloSourceInvalidationFallbackReason, StyloSourceInvalidationFallbackReasonPlan,
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
    StyloSourceStyleInvalidationTargetResultRecord, StyloStateInvalidationRoot,
    StyloStyleInvalidationQuery, StyloStyleInvalidationSnapshot,
    StyloStyleInvalidationSnapshotAttribute, StyloStyleSourceScope,
    StyloStylesheetSourceScopeFallbackInput,
    stylo_attribute_change_can_skip_fallback_without_dependency,
    stylo_attribute_change_can_use_retained_invalidator, stylo_element_dependency_snapshot,
    stylo_fallback_roots_plan, stylo_focus_change_invalidation_roots,
    stylo_focus_state_matches_handle, stylo_focus_within_state_matches_handle,
    stylo_merge_retained_source_invalidation_fallback_kind,
    stylo_merge_retained_source_invalidation_kind,
    stylo_merge_source_dependency_request_requirement, stylo_removed_element_dependency_snapshots,
    stylo_retained_next_element_sibling, stylo_retained_previous_element_sibling,
    stylo_retained_queries_for_current_element, stylo_retained_queries_for_element_snapshot,
    stylo_retained_source_invalidation_kind_can_use_fallback_payload,
    stylo_retained_source_style_invalidation_from_parts,
    stylo_runtime_fallback_roots_for_mutation_inputs, stylo_runtime_or_source_scope_fallback_plan,
    stylo_shadow_root_host_participates_in_style_scope,
    stylo_source_dependency_invalidation_batch_plan,
    stylo_source_fallback_reason_for_unretained_state_change,
    stylo_source_invalidation_fallback_reason_plan, stylo_source_scope_fallback_plan,
    stylo_state_change_can_use_retained_invalidator, stylo_stylesheet_owner_is_in_source_scope,
    stylo_stylesheet_source_scope_fallback_roots,
};
pub use presentation::is_svg_presentation_attribute_name;
use query::{QueryDocument, QueryElement, QueryNode};
pub(crate) use query::{html_auto_directionality_invalidation_root, html_directionality};
#[cfg(test)]
pub(crate) use selector_parse::validate_supports_selector_list;
pub(crate) use selector_parse::{
    ParsedDomApiSelectorList, normalize_nested_style_rule_selector_list_with_namespaces,
    normalize_scope_end_selector_list, normalize_scope_selector_list,
    normalize_scope_style_rule_selector_list_with_namespaces, parse_dom_api_selector_list,
    parse_dom_api_selector_list_for_url, validate_style_rule_selector_list,
    validate_style_rule_selector_list_with_namespaces,
    validate_supports_selector_condition_argument, validate_supports_selector_list_with_namespaces,
};
pub use style_traversal::{
    StyloDocument, StyloDomHostBinding, StyloDomStyleAdapter, StyloElement, StyloElementDataStore,
    StyloNode, StyloShadowRoot,
};

/// Query-only `DomHost` wrapper around Stylo `dom_apis`.
///
/// This intentionally provides only the subset of `TNode` / `TElement` /
/// `TDocument` / `TShadowRoot` required to make DOM query APIs work. Style,
/// layout and animation-related hooks are implemented conservatively.
#[derive(Debug, Clone)]
pub(super) struct StyloDomApiAdapter {
    shared_lock: SharedRwLock,
}

impl Default for StyloDomApiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for StyloDomStyleAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl StyloDomApiAdapter {
    pub(super) fn new() -> Self {
        Self {
            shared_lock: SharedRwLock::new(),
        }
    }

    fn document<'a>(
        &'a self,
        host: &'a DomHost,
        atom_cache: &'a QueryAtomCache,
        validity_states: Option<&'a HashMap<NodeId, bool>>,
    ) -> QueryDocument<'a> {
        QueryDocument::new(
            host,
            host.document_handle(),
            &self.shared_lock,
            None,
            atom_cache,
            validity_states,
        )
    }

    fn node<'a>(
        &'a self,
        host: &'a DomHost,
        handle: NodeId,
        atom_cache: &'a QueryAtomCache,
        validity_states: Option<&'a HashMap<NodeId, bool>>,
    ) -> Option<QueryNode<'a>> {
        host.node(handle).map(|_| {
            QueryNode::new(
                host,
                handle,
                &self.shared_lock,
                None,
                atom_cache,
                validity_states,
            )
        })
    }

    pub(super) fn query_selector_all(
        &self,
        host: &DomHost,
        selector: &str,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<Vec<NodeId>, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(Vec::new()),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let mut results =
            <QueryAll as style::dom_apis::SelectorQuery<QueryElement<'_>>>::Output::default();
        query_selector::<QueryElement<'_>, QueryAll>(
            self.document(host, &atom_cache, validity_states).as_node(),
            &selector_list,
            &mut results,
            MayUseInvalidation::No,
        );
        Ok(results.into_iter().map(QueryElement::handle).collect())
    }

    pub(super) fn query_selector_all_in(
        &self,
        host: &DomHost,
        root: NodeId,
        selector: &str,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<Vec<NodeId>, SelectorError> {
        if host.node(root).is_none() {
            return Ok(Vec::new());
        }
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(Vec::new()),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(root) = self.node(host, root, &atom_cache, validity_states) else {
            return Ok(Vec::new());
        };
        let mut results =
            <QueryAll as style::dom_apis::SelectorQuery<QueryElement<'_>>>::Output::default();
        query_selector::<QueryElement<'_>, QueryAll>(
            root,
            &selector_list,
            &mut results,
            MayUseInvalidation::No,
        );
        Ok(results.into_iter().map(QueryElement::handle).collect())
    }

    pub(super) fn query_selector(
        &self,
        host: &DomHost,
        selector: &str,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<Option<NodeId>, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(None),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let mut result =
            <QueryFirst as style::dom_apis::SelectorQuery<QueryElement<'_>>>::Output::default();
        query_selector::<QueryElement<'_>, QueryFirst>(
            self.document(host, &atom_cache, validity_states).as_node(),
            &selector_list,
            &mut result,
            MayUseInvalidation::No,
        );
        Ok(result.map(QueryElement::handle))
    }

    pub(super) fn query_selector_in(
        &self,
        host: &DomHost,
        root: NodeId,
        selector: &str,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<Option<NodeId>, SelectorError> {
        if host.node(root).is_none() {
            return Ok(None);
        }
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(None),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(root) = self.node(host, root, &atom_cache, validity_states) else {
            return Ok(None);
        };
        let mut result =
            <QueryFirst as style::dom_apis::SelectorQuery<QueryElement<'_>>>::Output::default();
        query_selector::<QueryElement<'_>, QueryFirst>(
            root,
            &selector_list,
            &mut result,
            MayUseInvalidation::No,
        );
        Ok(result.map(QueryElement::handle))
    }

    pub(super) fn matches(
        &self,
        host: &DomHost,
        handle: NodeId,
        selector: &str,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<bool, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(false),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(element) = self
            .node(host, handle, &atom_cache, validity_states)
            .and_then(QueryNode::as_element)
        else {
            return Ok(false);
        };
        Ok(element_matches(
            &element,
            &selector_list,
            element.read_quirks_mode(),
        ))
    }

    pub(super) fn matches_with_scope(
        &self,
        host: &DomHost,
        handle: NodeId,
        selector: &str,
        scope_root: NodeId,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<bool, SelectorError> {
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(false),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let atom_cache = QueryAtomCache::default();
        let Some(element) = self
            .node(host, handle, &atom_cache, validity_states)
            .and_then(QueryNode::as_element)
        else {
            return Ok(false);
        };
        let Some(scope_element) = self
            .node(host, scope_root, &atom_cache, validity_states)
            .and_then(QueryNode::as_element)
        else {
            return Ok(false);
        };
        let mut selector_caches = selectors::matching::SelectorCaches::default();
        let mut context = MatchingContext::new(
            selectors::matching::MatchingMode::Normal,
            None,
            &mut selector_caches,
            element.read_quirks_mode(),
            selectors::matching::NeedsSelectorFlags::No,
            selectors::matching::MatchingForInvalidation::No,
        );
        context.scope_element = Some(scope_element.opaque());
        context.current_host = element.containing_shadow_host().map(|host| host.opaque());
        Ok(selectors::matching::matches_selector_list(
            &selector_list,
            &element,
            &mut context,
        ))
    }

    pub(super) fn closest(
        &self,
        host: &DomHost,
        handle: NodeId,
        selector: &str,
        validity_states: Option<&HashMap<NodeId, bool>>,
    ) -> Result<Option<NodeId>, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(None),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(element) = self
            .node(host, handle, &atom_cache, validity_states)
            .and_then(QueryNode::as_element)
        else {
            return Ok(None);
        };
        Ok(
            element_closest(element, &selector_list, element.read_quirks_mode())
                .map(QueryElement::handle),
        )
    }
}
