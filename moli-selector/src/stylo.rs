use selectors::{Element as SelectorsElement, matching::MatchingContext};
use style::{
    dom::TDocument,
    dom_apis::{
        MayUseInvalidation, QueryAll, QueryFirst, element_closest, element_matches, query_selector,
    },
    shared_lock::SharedRwLock,
};

use crate::{
    dom::{NodeId, native::DomHost},
    selector::SelectorError,
};

mod atoms;
mod invalidation;
mod presentation;
mod presentational_hints;
mod query;
mod selector_parse;
mod style_traversal;
use atoms::QueryAtomCache;
pub use invalidation::{
    StyleMutationSnapshots, stylo_focus_change_handles, stylo_focus_state_matches_handle,
    stylo_focus_within_state_matches_handle, stylo_shadow_root_host_participates_in_style_scope,
};
pub use presentation::attribute_affects_presentational_hints;
pub use query::html_auto_directionality_root;
pub(crate) use query::html_directionality;
use query::{QueryDocument, QueryElement, QueryNode};
#[cfg(test)]
pub(crate) use selector_parse::validate_supports_selector_list;
pub(crate) use selector_parse::{
    ParsedDomApiSelectorList, normalize_nested_style_rule_selector_list_with_namespaces,
    normalize_scope_end_selector_list, normalize_scope_selector_list,
    normalize_scope_style_rule_selector_list_with_namespaces, parse_dom_api_selector_list,
    parse_dom_api_selector_list_for_url, validate_style_rule_selector_list,
    validate_style_rule_selector_list_with_namespaces,
    validate_supports_selector_condition_argument,
};
pub use style_traversal::{
    InlineStyleSnapshot, QueryContainerSizes, StyloDocument, StyloDomHostBinding,
    StyloDomStyleAdapter, StyloElement, StyloElementDataStore, StyloNode, StyloShadowRoot,
    StyloStylePath,
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
    ) -> QueryDocument<'a> {
        QueryDocument::new(
            host,
            host.document_handle(),
            &self.shared_lock,
            None,
            atom_cache,
        )
    }

    fn node<'a>(
        &'a self,
        host: &'a DomHost,
        handle: NodeId,
        atom_cache: &'a QueryAtomCache,
    ) -> Option<QueryNode<'a>> {
        host.node(handle)
            .map(|_| QueryNode::new(host, handle, &self.shared_lock, None, atom_cache))
    }

    pub(super) fn query_selector_all(
        &self,
        host: &DomHost,
        selector: &str,
    ) -> Result<Vec<NodeId>, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(Vec::new()),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let mut results =
            <QueryAll as style::dom_apis::SelectorQuery<QueryElement<'_>>>::Output::default();
        query_selector::<QueryElement<'_>, QueryAll>(
            self.document(host, &atom_cache).as_node(),
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
    ) -> Result<Vec<NodeId>, SelectorError> {
        if host.node(root).is_none() {
            return Ok(Vec::new());
        }
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(Vec::new()),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(root) = self.node(host, root, &atom_cache) else {
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
    ) -> Result<Option<NodeId>, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(None),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let mut result =
            <QueryFirst as style::dom_apis::SelectorQuery<QueryElement<'_>>>::Output::default();
        query_selector::<QueryElement<'_>, QueryFirst>(
            self.document(host, &atom_cache).as_node(),
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
    ) -> Result<Option<NodeId>, SelectorError> {
        if host.node(root).is_none() {
            return Ok(None);
        }
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(None),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(root) = self.node(host, root, &atom_cache) else {
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
    ) -> Result<bool, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(false),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(element) = self
            .node(host, handle, &atom_cache)
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
    ) -> Result<bool, SelectorError> {
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(false),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let atom_cache = QueryAtomCache::default();
        let Some(element) = self
            .node(host, handle, &atom_cache)
            .and_then(QueryNode::as_element)
        else {
            return Ok(false);
        };
        let Some(scope_element) = self
            .node(host, scope_root, &atom_cache)
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
    ) -> Result<Option<NodeId>, SelectorError> {
        let atom_cache = QueryAtomCache::default();
        let selector_list = match parse_dom_api_selector_list(host, selector)? {
            ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(None),
            ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
        };
        let Some(element) = self
            .node(host, handle, &atom_cache)
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
