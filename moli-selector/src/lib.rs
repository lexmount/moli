//! Selector engine integration for Moli documents.
//!
//! This crate wraps selector parsing, validation, and Stylo-backed querying on
//! top of the extracted DOM crate.

mod cssom_selector;
mod detached_stylo;
mod error;
mod stylo;
mod validation;

pub use cssom_selector::{
    CssDirection, GetComputedStylePseudoElement, first_strong_text_direction,
    get_computed_style_pseudo_element,
};
pub use detached_stylo::{
    DetachedStyloSelectorHost, detached_stylo_selector_matches,
    detached_stylo_selector_matches_if_uses_defined_pseudo, detached_stylo_selector_query_all,
};
pub use error::{SelectorError, SelectorErrorKind};
pub use moli_dom as dom;
pub use stylo::{
    MoliInvalidationResult, MoliInvalidationResultBuilder, MoliInvalidationSourceResultsSink,
    MoliStyleMutationElementSnapshot, MoliStyleMutationSnapshot,
    StyloDependencyInvalidationFallbackContext, StyloDocument, StyloDomHostBinding,
    StyloDomStyleAdapter, StyloElement, StyloElementDependencySnapshot, StyloNode,
    StyloPlannedFallbackRootInvalidationTarget,
    StyloPlannedFallbackRootInvalidationTargetPartsSink, StyloPlannedSourceDependencyInvalidation,
    StyloPlannedSourceDependencyInvalidationPartsSink,
    StyloPlannedSourceDependencyInvalidationTarget,
    StyloPlannedSourceDependencyInvalidationTargetPartsSink, StyloRetainedSourceStyleInvalidation,
    StyloRetainedSourceStyleInvalidationKind, StyloRetainedSourceStyleInvalidationSink,
    StyloRetainedStyleChildListInvalidationQueries,
    StyloRetainedStyleChildListInvalidationQueriesSink,
    StyloRetainedStyleChildListInvalidationQuery, StyloRetainedStyleInvalidationQuery,
    StyloRetainedStyleSiblingTraversal, StyloRuntimeFallbackRootInput, StyloShadowRoot,
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
    StyloStylesheetSourceScopeFallbackInput, dom_api_selector_uses_validity_pseudo,
    is_svg_presentation_attribute_name,
    stylo_attribute_change_can_skip_fallback_without_dependency,
    stylo_attribute_change_can_use_retained_invalidator, stylo_element_dependency_snapshot,
    stylo_fallback_roots_plan, stylo_flat_tree_heading_descendants,
    stylo_focus_change_invalidation_roots, stylo_focus_state_matches_handle,
    stylo_focus_within_state_matches_handle,
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

use moli_dom::native::DomHost;
use moli_dom::{NodeId, native::NativeDom};
use std::{collections::HashMap, sync::OnceLock};

// Temporary self-alias so the extracted module tree can keep compiling while
// `moli` keeps its historical `crate::selector::*` references.
mod selector {
    pub(crate) use crate::SelectorError;

    pub(crate) mod validation {
        pub(crate) use crate::validation::*;
    }
}

#[derive(Debug, Clone, Default)]
pub struct QueryEngine;

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct StyleRuleNamespaceContext {
    pub default_namespace_uri: Option<String>,
    pub namespace_prefixes: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum StyleRuleSelectorContext {
    #[default]
    TopLevel,
    Nested,
    Scope,
}

impl StyleRuleNamespaceContext {
    fn has_default_namespace(&self) -> bool {
        self.default_namespace_uri.is_some()
    }

    fn prefixes_matching_default_namespace(&self) -> Vec<String> {
        let Some(default_namespace_uri) = self.default_namespace_uri.as_deref() else {
            return Vec::new();
        };
        self.namespace_prefixes
            .iter()
            .filter(|(_, namespace_uri)| namespace_uri == default_namespace_uri)
            .map(|(prefix, _)| prefix.clone())
            .collect()
    }

    fn has_namespace_prefix(&self, prefix: &str) -> bool {
        self.namespace_prefixes
            .iter()
            .any(|(existing, _)| existing == prefix)
    }
}

impl QueryEngine {
    fn adapter(&self) -> &'static stylo::StyloDomApiAdapter {
        static ADAPTER: OnceLock<stylo::StyloDomApiAdapter> = OnceLock::new();
        ADAPTER.get_or_init(stylo::StyloDomApiAdapter::new)
    }

    fn host_for_document(document: &NativeDom) -> DomHost {
        DomHost::from_dom(document.clone())
    }

    pub fn query_selector_host(
        &self,
        host: &DomHost,
        selector: &str,
    ) -> Result<Option<NodeId>, SelectorError> {
        self.adapter().query_selector(host, selector, None)
    }

    pub fn query_selector_host_with_validity_states(
        &self,
        host: &DomHost,
        selector: &str,
        validity_states: &HashMap<NodeId, bool>,
    ) -> Result<Option<NodeId>, SelectorError> {
        self.adapter()
            .query_selector(host, selector, Some(validity_states))
    }

    pub fn query_selector_all_host(
        &self,
        host: &DomHost,
        selector: &str,
    ) -> Result<Vec<NodeId>, SelectorError> {
        self.adapter().query_selector_all(host, selector, None)
    }

    pub fn query_selector_all_host_with_validity_states(
        &self,
        host: &DomHost,
        selector: &str,
        validity_states: &HashMap<NodeId, bool>,
    ) -> Result<Vec<NodeId>, SelectorError> {
        self.adapter()
            .query_selector_all(host, selector, Some(validity_states))
    }

    pub fn query_selector_in_host(
        &self,
        host: &DomHost,
        root: NodeId,
        selector: &str,
    ) -> Result<Option<NodeId>, SelectorError> {
        self.adapter().query_selector_in(host, root, selector, None)
    }

    pub fn query_selector_in_host_with_validity_states(
        &self,
        host: &DomHost,
        root: NodeId,
        selector: &str,
        validity_states: &HashMap<NodeId, bool>,
    ) -> Result<Option<NodeId>, SelectorError> {
        self.adapter()
            .query_selector_in(host, root, selector, Some(validity_states))
    }

    pub fn query_selector_all_in_host(
        &self,
        host: &DomHost,
        root: NodeId,
        selector: &str,
    ) -> Result<Vec<NodeId>, SelectorError> {
        self.adapter()
            .query_selector_all_in(host, root, selector, None)
    }

    pub fn query_selector_all_in_host_with_validity_states(
        &self,
        host: &DomHost,
        root: NodeId,
        selector: &str,
        validity_states: &HashMap<NodeId, bool>,
    ) -> Result<Vec<NodeId>, SelectorError> {
        self.adapter()
            .query_selector_all_in(host, root, selector, Some(validity_states))
    }

    pub fn matches_host(
        &self,
        host: &DomHost,
        node_id: NodeId,
        selector: &str,
    ) -> Result<bool, SelectorError> {
        self.adapter().matches(host, node_id, selector, None)
    }

    pub fn matches_host_with_validity_states(
        &self,
        host: &DomHost,
        node_id: NodeId,
        selector: &str,
        validity_states: &HashMap<NodeId, bool>,
    ) -> Result<bool, SelectorError> {
        self.adapter()
            .matches(host, node_id, selector, Some(validity_states))
    }

    pub fn matches_with_scope_host(
        &self,
        host: &DomHost,
        node_id: NodeId,
        selector: &str,
        scope_root: NodeId,
    ) -> Result<bool, SelectorError> {
        self.adapter()
            .matches_with_scope(host, node_id, selector, scope_root, None)
    }

    pub fn closest_host(
        &self,
        host: &DomHost,
        start: NodeId,
        selector: &str,
    ) -> Result<Option<NodeId>, SelectorError> {
        self.adapter().closest(host, start, selector, None)
    }

    pub fn closest_host_with_validity_states(
        &self,
        host: &DomHost,
        start: NodeId,
        selector: &str,
        validity_states: &HashMap<NodeId, bool>,
    ) -> Result<Option<NodeId>, SelectorError> {
        self.adapter()
            .closest(host, start, selector, Some(validity_states))
    }

    pub fn query_selector(
        &self,
        document: &NativeDom,
        selector: &str,
    ) -> Result<Option<NodeId>, SelectorError> {
        let host = Self::host_for_document(document);
        self.query_selector_host(&host, selector)
    }

    pub fn query_selector_all(
        &self,
        document: &NativeDom,
        selector: &str,
    ) -> Result<Vec<NodeId>, SelectorError> {
        let host = Self::host_for_document(document);
        self.query_selector_all_host(&host, selector)
    }

    pub fn query_selector_in(
        &self,
        document: &NativeDom,
        root: NodeId,
        selector: &str,
    ) -> Result<Option<NodeId>, SelectorError> {
        let host = Self::host_for_document(document);
        self.query_selector_in_host(&host, root, selector)
    }

    pub fn query_selector_all_in(
        &self,
        document: &NativeDom,
        root: NodeId,
        selector: &str,
    ) -> Result<Vec<NodeId>, SelectorError> {
        let host = Self::host_for_document(document);
        self.query_selector_all_in_host(&host, root, selector)
    }

    pub fn matches(
        &self,
        document: &NativeDom,
        node_id: NodeId,
        selector: &str,
    ) -> Result<bool, SelectorError> {
        let host = Self::host_for_document(document);
        self.matches_host(&host, node_id, selector)
    }

    pub fn matches_with_scope(
        &self,
        document: &NativeDom,
        node_id: NodeId,
        selector: &str,
        scope_root: NodeId,
    ) -> Result<bool, SelectorError> {
        let host = Self::host_for_document(document);
        self.matches_with_scope_host(&host, node_id, selector, scope_root)
    }

    pub fn closest(
        &self,
        document: &NativeDom,
        start: NodeId,
        selector: &str,
    ) -> Result<Option<NodeId>, SelectorError> {
        let host = Self::host_for_document(document);
        self.closest_host(&host, start, selector)
    }
}

pub fn html_directionality(host: &DomHost, handle: NodeId) -> CssDirection {
    stylo::html_directionality(host, handle)
}

pub fn html_auto_directionality_invalidation_root(host: &DomHost, start: NodeId) -> Option<NodeId> {
    stylo::html_auto_directionality_invalidation_root(host, start)
}

pub fn validate_supports_selector_condition_argument(selector: &str) -> Result<(), SelectorError> {
    stylo::validate_supports_selector_condition_argument(selector)
}

pub fn normalize_scope_selector_list(selector: &str) -> Result<String, SelectorError> {
    stylo::normalize_scope_selector_list(selector)
}

pub fn normalize_scope_end_selector_list(selector: &str) -> Result<String, SelectorError> {
    stylo::normalize_scope_end_selector_list(selector)
}

pub fn canonicalize_cssom_style_rule_selector_text(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
    rule_context: StyleRuleSelectorContext,
) -> Result<String, SelectorError> {
    if cssom_selector::selector_list_has_invalid_terminal_pseudo_element_chain(selector) {
        return Err(SelectorError::syntax(
            "terminal pseudo-elements cannot be chained",
        ));
    }
    match rule_context {
        StyleRuleSelectorContext::Nested => {
            let selector = stylo::normalize_nested_style_rule_selector_list_with_namespaces(
                selector,
                namespace_context,
            )?;
            return Ok(serialize_cssom_style_rule_selector_text(
                &selector,
                namespace_context,
            ));
        }
        StyleRuleSelectorContext::Scope => {
            let selector = stylo::normalize_scope_style_rule_selector_list_with_namespaces(
                selector,
                namespace_context,
            )?;
            return Ok(serialize_cssom_style_rule_selector_text(
                &selector,
                namespace_context,
            ));
        }
        StyleRuleSelectorContext::TopLevel => {}
    };
    let validation = if cssom_selector::selector_list_has_namespace_separator(selector) {
        if let Some(prefix) =
            style_rule_selector_undeclared_namespace_prefix(selector, namespace_context)
        {
            return Err(SelectorError::syntax(format!(
                "undeclared namespace prefix `{prefix}`"
            )));
        }
        stylo::validate_style_rule_selector_list_with_namespaces(selector, namespace_context)
    } else {
        stylo::validate_style_rule_selector_list(selector)
    };
    if let Err(error) = validation
        && !cssom_selector::dom_api_selector_list_has_only_known_pseudo_elements(selector)
    {
        return Err(error);
    }
    Ok(serialize_cssom_style_rule_selector_text(
        selector,
        namespace_context,
    ))
}

fn style_rule_selector_undeclared_namespace_prefix(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
) -> Option<String> {
    cssom_selector::selector_list_namespace_prefixes(selector)
        .into_iter()
        .find(|prefix| !namespace_context.has_namespace_prefix(prefix))
}

fn serialize_cssom_style_rule_selector_text(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
) -> String {
    let prefixes_matching_default_namespace =
        namespace_context.prefixes_matching_default_namespace();
    cssom_selector::serialize_cssom_selector_text_preserving_invalid_forgiving_items(
        selector,
        namespace_context.has_default_namespace(),
        &prefixes_matching_default_namespace,
        |candidate| {
            stylo::validate_supports_selector_list_with_namespaces(candidate, namespace_context)
                .is_ok()
        },
    )
    .unwrap_or_else(|| selector.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        QueryEngine, StyleRuleNamespaceContext, StyleRuleSelectorContext, StyloDomStyleAdapter,
        canonicalize_cssom_style_rule_selector_text, html_auto_directionality_invalidation_root,
        normalize_scope_end_selector_list, normalize_scope_selector_list,
        validate_supports_selector_condition_argument,
    };
    use crate::dom::native::{DomHost, NativeDom};
    use crate::stylo::{
        validate_style_rule_selector_list, validate_style_rule_selector_list_with_namespaces,
        validate_supports_selector_list,
    };
    use dom::ElementState;
    use selectors::{
        matching::{MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags},
        parser::{ParseRelative, SelectorList},
    };
    use style::dom::{TDocument, TElement};
    use style::{
        context::QuirksMode,
        selector_parser::SelectorParser,
        stylesheets::{Namespaces, Origin, UrlExtraData},
    };

    #[test]
    fn style_rule_selector_validation_uses_css_rule_semantics() {
        assert!(validate_style_rule_selector_list("article > .card::before").is_ok());
        assert!(validate_style_rule_selector_list("#a#b").is_ok());
        assert!(validate_style_rule_selector_list(".one, main > .two").is_ok());
        assert!(validate_style_rule_selector_list("::part(mypart):lang(en)").is_ok());
        assert!(validate_style_rule_selector_list("::part(mypart):dir(ltr)").is_ok());
        assert!(
            validate_style_rule_selector_list("span::-webkit-something-invalid:active").is_ok()
        );
        assert!(
            validate_style_rule_selector_list("span::-webkit-something-invalid::before").is_err()
        );
        assert!(validate_style_rule_selector_list("").is_err());
        assert!(validate_style_rule_selector_list("div[").is_err());
        assert!(validate_style_rule_selector_list(".one,").is_err());
    }

    #[test]
    fn supports_selector_validation_disables_forgiving_selector_lists() {
        assert!(validate_supports_selector_list("::part(mypart):hover").is_ok());
        assert!(validate_supports_selector_list("::part(mypart):lang(en)").is_ok());
        assert!(validate_supports_selector_list("::part(mypart):dir(ltr)").is_ok());
        assert!(validate_supports_selector_list("::part(mypart):is(:hover)").is_ok());
        assert!(validate_supports_selector_list("::part(mypart):is(:first-child)").is_err());
        assert!(validate_supports_selector_list("::part(mypart):where(:first-child)").is_err());
        assert!(validate_supports_selector_list("span::-webkit-something-invalid").is_err());
    }

    #[test]
    fn supports_selector_condition_argument_requires_single_selector() {
        assert!(validate_supports_selector_condition_argument("::part(mypart):hover").is_ok());
        assert!(
            validate_supports_selector_condition_argument("div:is(.primary, .secondary)").is_ok()
        );
        assert!(validate_supports_selector_condition_argument("").is_err());
        assert!(validate_supports_selector_condition_argument("div, div").is_err());
        assert!(validate_supports_selector_condition_argument("div[attr=',']").is_ok());
        assert!(validate_supports_selector_condition_argument("div:is(.a, .b), span").is_err());
    }

    #[test]
    fn webkit_autofill_alias_uses_standard_autofill_selector_semantics() {
        for selector in ["input:-webkit-autofill", "input:-WeBkIt-AuToFiLl"] {
            assert!(validate_style_rule_selector_list(selector).is_ok());
            assert!(validate_supports_selector_list(selector).is_ok());
            assert!(validate_supports_selector_condition_argument(selector).is_ok());
        }
    }

    #[test]
    fn cssom_style_rule_selector_canonicalization_owns_namespace_validation() {
        let namespace_context = StyleRuleNamespaceContext {
            default_namespace_uri: Some("http://www.w3.org/1999/xhtml".to_owned()),
            namespace_prefixes: vec![
                ("html".to_owned(), "http://www.w3.org/1999/xhtml".to_owned()),
                ("svg".to_owned(), "http://www.w3.org/2000/svg".to_owned()),
            ],
        };

        assert_eq!(
            canonicalize_cssom_style_rule_selector_text(
                "html|div:is(.a, .b)",
                &namespace_context,
                StyleRuleSelectorContext::TopLevel,
            )
            .unwrap(),
            "div:is(.a, .b)"
        );
        assert!(
            canonicalize_cssom_style_rule_selector_text(
                "math|mi",
                &namespace_context,
                StyleRuleSelectorContext::TopLevel,
            )
            .is_err()
        );
    }

    #[test]
    fn cssom_style_rule_selector_canonicalization_uses_rule_context() {
        assert_eq!(
            canonicalize_cssom_style_rule_selector_text(
                "> .child",
                &StyleRuleNamespaceContext::default(),
                StyleRuleSelectorContext::Nested,
            )
            .unwrap(),
            "& > .child"
        );
        assert_eq!(
            canonicalize_cssom_style_rule_selector_text(
                "> .child",
                &StyleRuleNamespaceContext::default(),
                StyleRuleSelectorContext::Scope,
            )
            .unwrap(),
            "> .child"
        );
        assert!(
            canonicalize_cssom_style_rule_selector_text(
                "> .child",
                &StyleRuleNamespaceContext::default(),
                StyleRuleSelectorContext::TopLevel,
            )
            .is_err()
        );
    }

    #[test]
    fn scope_selector_normalization_uses_stylesheet_selector_parser() {
        assert_eq!(
            normalize_scope_selector_list(".a:hover, #b, div").unwrap(),
            ".a:hover, #b, div"
        );
        assert_eq!(normalize_scope_end_selector_list("> .b").unwrap(), "> .b");
        assert_eq!(normalize_scope_end_selector_list("& > &").unwrap(), "& > &");

        assert!(normalize_scope_selector_list("div::before").is_err());
        assert!(normalize_scope_end_selector_list(">>").is_err());
    }

    #[test]
    fn style_rule_selector_validation_uses_stylesheet_namespaces() {
        let context = StyleRuleNamespaceContext {
            default_namespace_uri: Some("http://www.w3.org/1999/xhtml".to_owned()),
            namespace_prefixes: vec![("svg".to_owned(), "http://www.w3.org/2000/svg".to_owned())],
        };
        assert!(validate_style_rule_selector_list_with_namespaces("svg|*.card", &context).is_ok());
        assert!(validate_style_rule_selector_list_with_namespaces("*|*.card", &context).is_ok());
        assert!(
            validate_style_rule_selector_list_with_namespaces("missing|*.card", &context).is_err()
        );
        assert!(validate_style_rule_selector_list_with_namespaces("svg|[", &context).is_err());
    }

    #[test]
    fn dom_api_selector_validation_rejects_digit_starting_class_names() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let host = DomHost::from_dom(NativeDom::new_html(url));
        let engine = QueryEngine;

        assert!(engine.query_selector_host(&host, ".5cm").is_err());
        assert!(engine.query_selector_host(&host, ".-5cm").is_err());
        assert!(engine.query_selector_host(&host, ".").is_err());
        assert!(engine.query_selector_host(&host, ". div").is_err());
        assert!(engine.query_selector_host(&host, "., div").is_err());
        assert!(engine.query_selector_host(&host, r#".\35 cm"#).is_ok());
        assert!(engine.query_selector_host(&host, r#".-\35 cm"#).is_ok());
        assert!(engine.query_selector_all_host(&host, "div, .5cm").is_err());
        assert!(
            engine
                .matches_host(&host, host.document_handle(), ".5cm")
                .is_err()
        );
    }

    #[test]
    fn dom_api_selectors_accept_duplicate_id_compounds() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "a"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, "div#a#a").unwrap(),
            Some(target)
        );
        assert_eq!(engine.query_selector_host(&host, "div#a#b").unwrap(), None);
    }

    #[test]
    fn dom_api_selectors_use_the_query_roots_owner_document_quirks_mode() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut dom = NativeDom::new_html(url);
        dom.node_mut(dom.document_node_id())
            .and_then(|node| node.data_mut().as_document_mut())
            .expect("HTML DOM should have a Document")
            .set_quirks_mode(QuirksMode::Quirks);
        let mut host = DomHost::from_dom(dom);
        host.reset_html_document_shell();
        let main_document = host.document_handle();
        assert_eq!(
            host.document_quirks_mode_for_handle(main_document),
            Some(QuirksMode::Quirks)
        );

        let body = host.document_body_handle().unwrap();
        let container = host.create_element("div");
        let adopted_button = host.create_element("button");
        let remaining_button = host.create_element("button");
        assert!(host.set_attribute(adopted_button, "class", "Foo"));
        assert!(host.set_attribute(remaining_button, "class", "Foo"));
        assert!(host.append_child(container, adopted_button));
        assert!(host.append_child(container, remaining_button));
        assert!(host.append_child(body, container));

        let detached_document = host.create_detached_html_document();
        assert!(host.append_child(detached_document, adopted_button));
        assert_eq!(
            host.owner_document_handle(adopted_button),
            Some(detached_document)
        );

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_in_host(&host, detached_document, ".Foo")
                .unwrap(),
            Some(adopted_button)
        );
        assert_eq!(
            engine
                .query_selector_in_host(&host, detached_document, ".foo")
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .query_selector_in_host(&host, container, ".foo")
                .unwrap(),
            Some(remaining_button)
        );
        assert!(!engine.matches_host(&host, adopted_button, ".foo").unwrap());
        assert!(
            engine
                .matches_host(&host, remaining_button, ".foo")
                .unwrap()
        );
    }

    #[test]
    fn stylo_style_adapter_exposes_dom_traits_and_document_bucketed_element_data() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("input");
        assert!(host.set_attribute(target, "id", "target"));
        assert!(host.set_attribute(target, "class", "card selected"));
        assert!(host.set_attribute(target, "disabled", ""));
        assert!(host.append_child(body, target));
        let shadow_host = host.create_element("div");
        assert!(host.append_child(body, shadow_host));
        let shadow_root = host.attach_shadow_root(shadow_host, "open").unwrap();
        let shadow_child = host.create_element("span");
        assert!(host.append_child(shadow_root, shadow_child));
        let detached_document = host.create_detached_html_document();
        let detached_target = host.create_element("section");
        assert!(host.append_child(detached_document, detached_target));

        let adapter = StyloDomStyleAdapter::new();
        adapter.with_bound_host(&host, |adapter| {
            let document = adapter.document(&host);
            assert!(document.is_html_document());
            assert!(document.as_node().as_document().is_some());

            let element = adapter.element(&host, target).unwrap();
            assert!(element.is_html_element());
            assert_eq!(element.id().map(AsRef::as_ref), Some("target"));
            let mut classes = Vec::new();
            element.each_class(|class| classes.push(class.as_ref().to_owned()));
            assert_eq!(classes, vec!["card", "selected"]);
            assert!(element.state().contains(ElementState::DISABLED));

            assert!(
                adapter
                    .element(&host, shadow_host)
                    .unwrap()
                    .shadow_root()
                    .is_some()
            );
            assert!(
                adapter
                    .element(&host, shadow_child)
                    .unwrap()
                    .containing_shadow()
                    .is_some()
            );

            assert!(!element.has_data());
            unsafe {
                let _data = element.ensure_data();
            }
            assert!(element.has_data());
            assert!(adapter.has_element_data(target));
            assert_eq!(adapter.element_data_count(), 1);
            assert_eq!(adapter.element_side_table_document_count_for_test(), 1);

            let detached_element = adapter.element(&host, detached_target).unwrap();
            assert!(!detached_element.has_data());
            unsafe {
                let _data = detached_element.ensure_data();
            }
            assert!(detached_element.has_data());
            assert!(adapter.has_element_data(detached_target));
            assert_eq!(adapter.element_data_count(), 2);
            assert_eq!(adapter.element_side_table_document_count_for_test(), 2);

            unsafe {
                element.clear_data();
            }
            assert!(!element.has_data());
            assert!(!adapter.has_element_data(target));
            assert!(detached_element.has_data());
            assert!(adapter.has_element_data(detached_target));
            assert_eq!(adapter.element_data_count(), 1);
            assert_eq!(adapter.element_side_table_document_count_for_test(), 1);
        });
    }

    #[test]
    #[should_panic(expected = "style adapter host access must use the currently bound DomHost")]
    fn stylo_style_adapter_rejects_cross_host_access_in_release() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let host = DomHost::from_dom(NativeDom::new_html(url.clone()));
        let other_host = DomHost::from_dom(NativeDom::new_html(url));
        let adapter = StyloDomStyleAdapter::new();

        adapter.with_bound_host(&host, |binding| {
            let _ = binding.document(&other_host);
        });
    }

    #[test]
    fn dom_api_selectors_apply_target_pseudo_class_from_document_url() {
        let url = url::Url::parse("https://example.test/page.html#foo%20bar").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "foo bar"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, ":target").unwrap(),
            Some(target)
        );
        assert!(engine.matches_host(&host, target, ":target").unwrap());
    }

    #[test]
    fn dom_api_selectors_match_nth_child_structural_pseudo_classes() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let first = host.create_element("div");
        let second = host.create_element("div");
        let target = host.create_element("div");
        assert!(host.append_child(body, first));
        assert!(host.append_child(body, second));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert!(
            engine
                .matches_host(&host, target, "div:nth-child(3)")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, target, "div:nth-child(odd of :not(.c))")
                .unwrap()
        );
        assert!(host.set_attribute(second, "class", "c"));
        assert!(
            engine
                .matches_host(&host, first, "body > div:nth-child(odd of :not(.c))")
                .unwrap()
        );
        assert!(
            !engine
                .matches_host(&host, target, "body > div:nth-child(odd of :not(.c))")
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_match_heading_pseudo_class() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let h1 = host.create_element("h1");
        let h2 = host.create_element("h2");
        let h7 = host.create_element("h7");
        let role_heading = host.create_element("p");
        let ancestor = host.create_element("section");
        let nested_h1 = host.create_element("h1");
        let subject = host.create_element("div");
        let offset_parent = host.create_element("section");
        let offset_h1 = host.create_element("h1");
        let reset_boundary = host.create_element("section");
        let nested_offset = host.create_element("div");
        let reset_h1 = host.create_element("h1");
        let shadow_host = host.create_element("section");
        let shadow_h1 = host.create_element("h1");
        let slot_container = host.create_element("div");
        let slot = host.create_element("slot");
        let slotted_h2 = host.create_element("h2");
        let modal_parent = host.create_element("div");
        let modal = host.create_element("dialog");
        let modal_h1 = host.create_element("h1");
        let aria_h1 = host.create_element("h1");
        assert!(host.set_attribute(role_heading, "role", "heading"));
        assert!(host.set_attribute(role_heading, "aria-level", "1"));
        assert!(host.set_attribute(ancestor, "id", "ancestor"));
        assert!(host.set_attribute(subject, "id", "subject"));
        assert!(host.set_attribute(offset_parent, "headingoffset", "3"));
        assert!(host.set_attribute(reset_boundary, "headingoffset", "2"));
        assert!(host.set_attribute(reset_boundary, "headingreset", ""));
        assert!(host.set_attribute(nested_offset, "headingoffset", "2"));
        assert!(host.set_attribute(shadow_host, "headingoffset", "1"));
        assert!(host.set_attribute(slot_container, "headingoffset", "1"));
        assert!(host.set_attribute(modal_parent, "headingoffset", "8"));
        assert!(host.set_attribute(aria_h1, "headingoffset", "8"));
        assert!(host.set_attribute(aria_h1, "aria-level", "3"));
        assert!(host.append_child(body, h1));
        assert!(host.append_child(body, h2));
        assert!(host.append_child(body, h7));
        assert!(host.append_child(body, role_heading));
        assert!(host.append_child(body, ancestor));
        assert!(host.append_child(ancestor, nested_h1));
        assert!(host.append_child(body, subject));
        assert!(host.append_child(body, offset_parent));
        assert!(host.append_child(offset_parent, offset_h1));
        assert!(host.append_child(body, reset_boundary));
        assert!(host.append_child(reset_boundary, nested_offset));
        assert!(host.append_child(nested_offset, reset_h1));
        assert!(host.append_child(body, shadow_host));
        let shadow_root = host.attach_shadow_root(shadow_host, "open").unwrap();
        assert!(host.append_child(shadow_root, shadow_h1));
        assert!(host.append_child(shadow_root, slot_container));
        assert!(host.append_child(slot_container, slot));
        assert!(host.append_child(shadow_host, slotted_h2));
        assert!(host.append_child(body, modal_parent));
        assert!(host.append_child(modal_parent, modal));
        assert!(host.append_child(modal, modal_h1));
        assert!(host.set_dialog_modal(modal, true));
        assert!(host.append_child(body, aria_h1));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, h1, ":heading").unwrap());
        assert!(engine.matches_host(&host, h1, ":heading(1)").unwrap());
        assert!(!engine.matches_host(&host, h1, ":heading(2)").unwrap());
        assert!(engine.matches_host(&host, h2, ":heading(1, 2)").unwrap());
        assert!(!engine.matches_host(&host, h7, ":heading").unwrap());
        assert!(
            !engine
                .matches_host(&host, role_heading, ":heading(1)")
                .unwrap()
        );
        assert_eq!(
            engine.query_selector_host(&host, ":heading(2)").unwrap(),
            Some(h2)
        );
        assert_eq!(
            engine.query_selector_host(&host, ":heading(0, 7)").unwrap(),
            None
        );
        assert!(
            engine
                .matches_host(&host, subject, "#ancestor:has(:heading(1)) ~ #subject")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, offset_h1, ":heading(4)")
                .unwrap()
        );
        assert!(host.set_attribute(offset_parent, "headingoffset", "5"));
        assert!(
            engine
                .matches_host(&host, offset_h1, ":heading(6)")
                .unwrap()
        );
        assert!(engine.matches_host(&host, reset_h1, ":heading(5)").unwrap());
        assert!(
            engine
                .matches_host(&host, shadow_h1, ":heading(2)")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, slotted_h2, ":heading(4)")
                .unwrap()
        );
        assert!(engine.matches_host(&host, modal_h1, ":heading(1)").unwrap());
        assert!(engine.matches_host(&host, aria_h1, ":heading(9)").unwrap());
        assert!(engine.query_selector_host(&host, ":heading()").is_err());
        assert!(engine.query_selector_host(&host, ":heading(2n)").is_err());
    }

    #[test]
    fn dom_api_selectors_match_custom_state_pseudo_class() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let subject = host.create_element("section");
        let target = host.create_element("x-stateful");
        assert!(host.append_child(body, subject));
        assert!(host.append_child(subject, target));

        let engine = QueryEngine;
        assert!(
            !engine
                .matches_host(&host, target, ":state(--active)")
                .unwrap()
        );
        assert!(
            !engine
                .matches_host(&host, subject, "section:has(:state(--active))")
                .unwrap()
        );

        assert!(host.insert_custom_state(target, "--active"));
        assert!(
            engine
                .matches_host(&host, target, ":state(--active)")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, subject, "section:has(:state(--active))")
                .unwrap()
        );
        assert_eq!(
            engine
                .query_selector_host(&host, ":state(--active)")
                .unwrap(),
            Some(target)
        );

        assert!(host.remove_custom_state(target, "--active"));
        assert!(
            !engine
                .matches_host(&host, target, ":state(--active)")
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_match_media_pseudo_classes() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let subject = host.create_element("section");
        let video = host.create_element("video");
        assert!(host.append_child(body, subject));
        assert!(host.append_child(subject, video));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, video, ":paused").unwrap());
        assert!(!engine.matches_host(&host, video, ":playing").unwrap());
        assert!(
            engine
                .matches_host(&host, subject, "section:has(:paused)")
                .unwrap()
        );

        assert!(host.set_media_paused(video, false));
        assert!(!engine.matches_host(&host, video, ":paused").unwrap());
        assert!(engine.matches_host(&host, video, ":playing").unwrap());
        assert!(
            engine
                .matches_host(&host, subject, "section:has(:playing)")
                .unwrap()
        );

        assert!(host.set_media_muted(video, true));
        assert!(engine.matches_host(&host, video, ":muted").unwrap());
        assert!(host.set_media_seeking(video, true));
        assert!(engine.matches_host(&host, video, ":seeking").unwrap());
    }

    #[test]
    fn stylo_style_element_matches_nth_child_of_selector_list() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url.clone()));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let section = host.create_element("section");
        let first = host.create_element("div");
        let middle = host.create_element("div");
        let target = host.create_element("div");
        assert!(host.set_attribute(middle, "class", "c"));
        assert!(host.append_child(body, section));
        assert!(host.append_child(section, first));
        assert!(host.append_child(section, middle));
        assert!(host.append_child(section, target));

        let selector = "section > div:nth-child(odd of :not(.c))";
        let namespaces = Namespaces::default();
        let url_data = UrlExtraData::from(url);
        let parser = SelectorParser {
            stylesheet_origin: Origin::Author,
            namespaces: &namespaces,
            url_data: &url_data,
            for_supports_rule: false,
        };
        let mut input = cssparser::ParserInput::new(selector);
        let selectors = SelectorList::parse(
            &parser,
            &mut cssparser::Parser::new(&mut input),
            ParseRelative::No,
        )
        .expect("style selector should parse");
        let adapter = StyloDomStyleAdapter::new();
        adapter.with_bound_host(&host, |adapter| {
            let first = adapter.element(&host, first).unwrap();
            let target = adapter.element(&host, target).unwrap();
            let mut selector_caches = selectors::matching::SelectorCaches::default();
            let mut context = MatchingContext::new(
                MatchingMode::Normal,
                None,
                &mut selector_caches,
                QuirksMode::NoQuirks,
                NeedsSelectorFlags::No,
                MatchingForInvalidation::No,
            );
            assert!(selectors::matching::matches_selector_list(
                &selectors,
                &first,
                &mut context
            ));
            assert!(!selectors::matching::matches_selector_list(
                &selectors,
                &target,
                &mut context
            ));
        });
    }

    #[test]
    fn dom_api_selectors_apply_target_pseudo_class_from_utf8_fragment() {
        let url = url::Url::parse("https://example.test/page.html#%E4%BD%A0").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "你"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, ":target").unwrap(),
            Some(target)
        );
    }

    #[test]
    fn dom_api_selectors_apply_target_state_for_complex_selectors() {
        let url = url::Url::parse("https://example.test/page.html#target").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("section");
        assert!(host.set_attribute(target, "id", "target"));
        assert!(host.set_attribute(target, "class", "hit"));
        assert!(host.set_attribute(target, "data-x", "ok"));
        let child = host.create_element("p");
        assert!(host.append_child(target, child));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_host(&host, "section.hit[data-x]:target")
                .unwrap(),
            Some(target)
        );
        assert_eq!(
            engine.query_selector_host(&host, ":target > p").unwrap(),
            Some(child)
        );
        assert!(!engine.matches_host(&host, target, ":not(:target)").unwrap());
    }

    #[test]
    fn dom_api_selectors_update_target_state_when_fragment_changes() {
        let url = url::Url::parse("https://example.test/page.html#first").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let first = host.create_element("div");
        let second = host.create_element("div");
        assert!(host.set_attribute(first, "id", "first"));
        assert!(host.set_attribute(second, "id", "second"));
        assert!(host.append_child(body, first));
        assert!(host.append_child(body, second));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, first, ":target").unwrap());
        assert!(!engine.matches_host(&host, second, ":target").unwrap());

        host.set_document_url(url::Url::parse("https://example.test/page.html#second").unwrap());

        assert!(!engine.matches_host(&host, first, ":target").unwrap());
        assert!(engine.matches_host(&host, second, ":target").unwrap());
    }

    #[test]
    fn dom_api_selectors_clear_target_state_when_target_stops_matching() {
        let url = url::Url::parse("https://example.test/page.html#target").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "target"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, target, ":target").unwrap());

        assert!(host.set_attribute(target, "id", "other"));
        assert!(!engine.matches_host(&host, target, ":target").unwrap());

        assert!(host.set_attribute(target, "id", "target"));
        assert!(engine.matches_host(&host, target, ":target").unwrap());

        assert!(host.remove_child(body, target));
        assert!(!engine.matches_host(&host, target, ":target").unwrap());
    }

    #[test]
    fn dom_api_selectors_match_persistent_hover_state() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let menu = host.create_element("nav");
        let item = host.create_element("button");
        let sibling = host.create_element("button");
        assert!(host.append_child(menu, item));
        assert!(host.append_child(body, menu));
        assert!(host.append_child(body, sibling));

        let engine = QueryEngine;
        assert!(!engine.matches_host(&host, item, ":hover").unwrap());
        assert!(!engine.matches_host(&host, menu, ":hover").unwrap());

        assert!(host.set_hovered_element_handles(vec![item, menu, body]));
        assert!(engine.matches_host(&host, item, ":hover").unwrap());
        assert!(engine.matches_host(&host, menu, ":hover").unwrap());
        assert!(
            engine
                .matches_host(&host, menu, ":hover:has(> button:hover)")
                .unwrap()
        );
        assert!(!engine.matches_host(&host, sibling, ":hover").unwrap());

        assert!(host.set_hovered_element_handles(vec![sibling, body]));
        assert!(!engine.matches_host(&host, item, ":hover").unwrap());
        assert!(!engine.matches_host(&host, menu, ":hover").unwrap());
        assert!(engine.matches_host(&host, sibling, ":hover").unwrap());

        assert!(host.remove_child(body, sibling));
        assert!(!engine.matches_host(&host, sibling, ":hover").unwrap());
        assert!(host.append_child(body, sibling));
        assert!(!engine.matches_host(&host, sibling, ":hover").unwrap());
    }

    #[test]
    fn dom_api_selectors_clear_target_state_for_empty_fragment_and_top() {
        let url = url::Url::parse("https://example.test/page.html#target").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "target"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, target, ":target").unwrap());

        host.set_document_url(url::Url::parse("https://example.test/page.html#").unwrap());
        assert_eq!(engine.query_selector_host(&host, ":target").unwrap(), None);
        assert!(!engine.matches_host(&host, target, ":target").unwrap());

        host.set_document_url(url::Url::parse("https://example.test/page.html#target").unwrap());
        assert!(engine.matches_host(&host, target, ":target").unwrap());

        host.set_document_url(url::Url::parse("https://example.test/page.html#top").unwrap());
        assert_eq!(engine.query_selector_host(&host, ":target").unwrap(), None);
        assert!(!engine.matches_host(&host, target, ":target").unwrap());
    }

    #[test]
    fn dom_api_selectors_resolve_target_raw_before_decoded_and_anchor_name() {
        let url = url::Url::parse("https://example.test/page.html#foo%20bar").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let raw = host.create_element("div");
        let decoded = host.create_element("div");
        assert!(host.set_attribute(raw, "id", "foo%20bar"));
        assert!(host.set_attribute(decoded, "id", "foo bar"));
        assert!(host.append_child(body, raw));
        assert!(host.append_child(body, decoded));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, ":target").unwrap(),
            Some(raw)
        );

        let mut name_host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/page.html#legacy").unwrap(),
        ));
        name_host.reset_html_document_shell();
        let body = name_host.document_body_handle().unwrap();
        let anchor = name_host.create_element("a");
        assert!(name_host.set_attribute(anchor, "name", "legacy"));
        assert!(name_host.append_child(body, anchor));

        assert_eq!(
            engine.query_selector_host(&name_host, ":target").unwrap(),
            Some(anchor)
        );
    }

    #[test]
    fn dom_api_selectors_treat_known_pseudo_elements_as_empty_results() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "target"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_all_host(&host, "#target::before")
                .unwrap(),
            Vec::new()
        );
        assert_eq!(
            engine
                .query_selector_host(&host, "#target:first-line")
                .unwrap(),
            None
        );
        assert_eq!(
            engine.query_selector_host(&host, "::slotted(foo").unwrap(),
            None
        );
        assert_eq!(
            engine
                .query_selector_host(&host, "::part(label):hover")
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .query_selector_host(&host, "::part(label):lang(en)")
                .unwrap(),
            None
        );
        for selector in [
            "::slotted(*)::checkmark",
            "::slotted(*)::picker-icon",
            "::slotted(*)::picker(select)",
        ] {
            assert_eq!(
                engine.query_selector_host(&host, selector).unwrap(),
                None,
                "valid pseudo-element selector `{selector}` should return no element"
            );
        }
        for selector in [
            "::slotted()",
            "::slotted(",
            "::slotted(0)",
            ":slotted(foo)",
            "::slotted(foo) + ::slotted(bar)",
            "::slotted(foo):hover",
            "::slotted(foo):focus",
            "::slotted(foo):lang(en)",
            "::slotted(foo):dir(ltr)",
            ":part()",
            "::part(",
            ":part(0)",
            ":part('foo')",
            ":part([foo])",
            "::part(foo) + ::part(bar)",
        ] {
            assert!(
                engine.query_selector_host(&host, selector).is_err(),
                "invalid pseudo-element selector `{selector}` should be rejected"
            );
        }
        assert!(
            engine
                .query_selector_host(&host, "::part(label):first-child")
                .is_err()
        );
        assert!(
            engine
                .query_selector_host(&host, "#target::example")
                .is_err()
        );
        assert!(
            engine
                .query_selector_host(&host, "#target::before[")
                .is_err()
        );
        assert!(
            engine
                .query_selector_host(&host, "#target::before, [")
                .is_err()
        );
        assert!(
            engine
                .query_selector_host(&host, "invalid# ::before")
                .is_err()
        );
        assert_eq!(
            engine
                .query_selector_host(&host, "span::-webkit-something-invalid")
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .query_selector_host(&host, "span::-webkit-something-invalid:active")
                .unwrap(),
            None
        );
        assert!(
            engine
                .query_selector_host(&host, "span::-webkit-something-invalid::before")
                .is_err()
        );
    }

    #[test]
    fn dom_api_selectors_recover_trailing_unclosed_attribute_selector() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "align", "center"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_host(&host, r#"[align="center""#)
                .unwrap(),
            Some(target)
        );
        assert!(
            engine
                .matches_host(&host, target, r#"[align="center""#)
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_empty_substring_attribute_operators_match_nothing() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "class", "hit"));
        assert!(host.set_attribute(target, "data-x", "value"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_host(&host, r#"[data-x^=""]"#)
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .query_selector_host(&host, r#"[data-x$=""]"#)
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .query_selector_host(&host, r#"[data-x*=""]"#)
                .unwrap(),
            None
        );
        assert!(
            !engine
                .matches_host(&host, target, r#"[class^=""]"#)
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_wildcard_namespace_attribute_scans_all_matching_local_names() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "foo", "x"));
        assert!(host.set_attribute_ns(target, Some("a"), Some("a"), "foo", "x"));
        assert!(host.set_attribute_ns(target, Some("b"), Some("b"), "foo", "BAR"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert!(
            engine
                .matches_host(&host, target, "[*|foo='bar' i]")
                .unwrap()
        );
        assert_eq!(
            engine
                .query_selector_host(&host, "[*|foo='bar' i]")
                .unwrap(),
            Some(target)
        );
        assert!(
            !engine
                .matches_host(&host, target, "[|foo='bar' i]")
                .unwrap()
        );
        assert!(!engine.matches_host(&host, target, "[foo='bar' i]").unwrap());
    }

    #[test]
    fn dom_api_selectors_contenteditable_false_blocks_read_write() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let editable = host.create_element("div");
        assert!(host.set_attribute(editable, "contenteditable", "true"));
        let inherited = host.create_element("p");
        assert!(host.append_child(editable, inherited));
        let blocked = host.create_element("section");
        assert!(host.set_attribute(blocked, "contenteditable", "false"));
        assert!(host.append_child(editable, blocked));
        let blocked_child = host.create_element("span");
        assert!(host.append_child(blocked, blocked_child));
        let invalid_inherits = host.create_element("em");
        assert!(host.set_attribute(invalid_inherits, "contenteditable", "invalid"));
        assert!(host.append_child(editable, invalid_inherits));
        let plaintext = host.create_element("article");
        assert!(host.set_attribute(plaintext, "contenteditable", "plaintext-only"));
        assert!(host.append_child(body, editable));
        assert!(host.append_child(body, plaintext));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, editable, ":read-write").unwrap());
        assert!(
            engine
                .matches_host(&host, inherited, ":read-write")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, invalid_inherits, ":read-write")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, plaintext, ":read-write")
                .unwrap()
        );
        assert!(!engine.matches_host(&host, blocked, ":read-write").unwrap());
        assert!(
            !engine
                .matches_host(&host, blocked_child, ":read-write")
                .unwrap()
        );
        assert!(engine.matches_host(&host, blocked, ":read-only").unwrap());
        assert!(
            engine
                .matches_host(&host, blocked_child, ":read-only")
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_design_mode_marks_connected_elements_read_write() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let paragraph = host.create_element("p");
        let blocked = host.create_element("div");
        assert!(host.set_attribute(blocked, "contenteditable", "false"));
        let detached = host.create_element("section");
        assert!(host.append_child(body, paragraph));
        assert!(host.append_child(body, blocked));
        assert!(host.set_document_design_mode_enabled_for_handle(host.document_handle(), true));

        let engine = QueryEngine;
        assert!(
            engine
                .matches_host(&host, paragraph, ":read-write")
                .unwrap()
        );
        assert!(!engine.matches_host(&host, paragraph, ":read-only").unwrap());
        assert!(!engine.matches_host(&host, blocked, ":read-write").unwrap());
        assert!(engine.matches_host(&host, blocked, ":read-only").unwrap());
        assert!(!engine.matches_host(&host, detached, ":read-write").unwrap());
        assert!(engine.matches_host(&host, detached, ":read-only").unwrap());
    }

    #[test]
    fn dom_api_selectors_lang_uses_nearest_language_attribute() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        assert!(host.set_attribute(body, "lang", "en"));
        let paragraph = host.create_element("p");
        assert!(host.set_attribute(paragraph, "lang", "de"));
        let child = host.create_element("span");
        assert!(host.append_child(paragraph, child));
        assert!(host.append_child(body, paragraph));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, paragraph, ":lang(de)").unwrap());
        assert!(!engine.matches_host(&host, paragraph, ":lang(en)").unwrap());
        assert!(engine.matches_host(&host, child, ":lang(de)").unwrap());
        assert!(!engine.matches_host(&host, child, ":lang(en)").unwrap());
        assert!(engine.matches_host(&host, body, ":lang(en)").unwrap());
    }

    #[test]
    fn dom_api_selectors_lang_uses_stylo_extended_filtering() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let es_mx = host.create_element("p");
        let es = host.create_element("p");
        let en_gb_scouse = host.create_element("p");
        let singleton = host.create_element("p");
        let singleton_skipped = host.create_element("p");
        assert!(host.set_attribute(es_mx, "lang", "es-MX"));
        assert!(host.set_attribute(es, "lang", "es"));
        assert!(host.set_attribute(en_gb_scouse, "lang", "en-GB-scouse"));
        assert!(host.set_attribute(singleton, "lang", "fr-x-standard"));
        assert!(host.set_attribute(singleton_skipped, "lang", "fr-x-standard"));
        assert!(host.append_child(body, es_mx));
        assert!(host.append_child(body, es));
        assert!(host.append_child(body, en_gb_scouse));
        assert!(host.append_child(body, singleton));
        assert!(host.append_child(body, singleton_skipped));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, es_mx, ":lang(es)").unwrap());
        assert!(
            !engine
                .matches_host(&host, es_mx, ":lang(es-MX-US)")
                .unwrap()
        );
        assert!(!engine.matches_host(&host, es, ":lang(es-MX)").unwrap());
        assert!(
            engine
                .matches_host(&host, en_gb_scouse, ":lang(en-GB)")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, singleton, ":lang(fr-x)")
                .unwrap()
        );
        assert!(
            !engine
                .matches_host(&host, singleton_skipped, ":lang(fr-standard)")
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_dir_auto_uses_first_strong_text() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let bdi = host.create_element("bdi");
        assert!(host.append_child(body, bdi));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, bdi, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, bdi, ":dir(rtl)").unwrap());

        assert!(host.set_text_content(bdi, "\u{05ea}"));
        assert!(!engine.matches_host(&host, bdi, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, bdi, ":dir(rtl)").unwrap());

        let parent = host.create_element("div");
        let isolated = host.create_element("div");
        let fallback = host.create_element("div");
        assert!(host.set_attribute(parent, "dir", "auto"));
        assert!(host.set_attribute(isolated, "dir", "rtl"));
        assert!(host.set_text_content(isolated, "\u{05ea}"));
        assert!(host.set_text_content(fallback, "a"));
        assert!(host.append_child(body, parent));
        assert!(host.append_child(parent, isolated));
        assert!(host.append_child(parent, fallback));

        assert!(engine.matches_host(&host, parent, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, parent, ":dir(rtl)").unwrap());

        assert!(host.set_attribute(isolated, "dir", ""));
        assert!(!engine.matches_host(&host, parent, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, parent, ":dir(rtl)").unwrap());
    }

    #[test]
    fn dir_auto_mutation_root_stops_at_contained_text_boundaries() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();

        let auto = host.create_element("div");
        let transparent = host.create_element("span");
        let explicit = host.create_element("section");
        let explicit_child = host.create_element("span");
        let bdi = host.create_element("bdi");
        let script = host.create_element("script");
        assert!(host.set_attribute(auto, "dir", "auto"));
        assert!(host.set_attribute(explicit, "dir", "rtl"));
        assert!(host.append_child(body, auto));
        assert!(host.append_child(auto, transparent));
        assert!(host.append_child(auto, explicit));
        assert!(host.append_child(explicit, explicit_child));
        assert!(host.append_child(auto, bdi));
        assert!(host.append_child(auto, script));

        assert_eq!(
            html_auto_directionality_invalidation_root(&host, transparent),
            Some(auto)
        );
        assert_eq!(
            html_auto_directionality_invalidation_root(&host, explicit_child),
            None
        );
        assert_eq!(
            html_auto_directionality_invalidation_root(&host, bdi),
            Some(bdi)
        );
        assert_eq!(
            html_auto_directionality_invalidation_root(&host, script),
            None
        );
    }

    #[test]
    fn dom_api_selectors_dir_on_input_uses_html_directionality_rules() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let engine = QueryEngine;

        let rtl_parent = host.create_element("div");
        assert!(host.set_attribute(rtl_parent, "dir", "rtl"));
        assert!(host.append_child(body, rtl_parent));

        let tel = host.create_element("input");
        assert!(host.set_attribute(tel, "type", "tel"));
        assert!(host.append_child(rtl_parent, tel));
        assert!(engine.matches_host(&host, tel, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, tel, ":dir(rtl)").unwrap());

        assert!(host.set_attribute(tel, "dir", "foo"));
        assert!(engine.matches_host(&host, tel, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, tel, ":dir(rtl)").unwrap());

        assert!(host.set_attribute(tel, "dir", "auto"));
        assert!(host.set_input_value(tel, "\u{05ea}"));
        assert!(!engine.matches_host(&host, tel, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, tel, ":dir(rtl)").unwrap());

        assert!(host.remove_attribute(tel, "dir"));
        assert!(host.set_attribute(tel, "type", "text"));
        assert!(!engine.matches_host(&host, tel, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, tel, ":dir(rtl)").unwrap());

        let auto = host.create_element("input");
        assert!(host.set_attribute(auto, "dir", "auto"));
        assert!(host.set_attribute(auto, "type", "text"));
        assert!(host.set_input_value(auto, "\u{05ea}"));
        assert!(host.append_child(body, auto));
        assert!(!engine.matches_host(&host, auto, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, auto, ":dir(rtl)").unwrap());

        assert!(host.set_attribute(auto, "type", "radio"));
        assert!(engine.matches_host(&host, auto, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, auto, ":dir(rtl)").unwrap());

        assert!(host.set_attribute(auto, "type", "hidden"));
        assert!(!engine.matches_host(&host, auto, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, auto, ":dir(rtl)").unwrap());

        let textarea = host.create_element("textarea");
        assert!(host.set_attribute(textarea, "dir", "auto"));
        let textarea_text = host.create_text_node("\u{05ea}");
        assert!(host.append_child(textarea, textarea_text));
        assert!(host.append_child(body, textarea));
        assert!(!engine.matches_host(&host, textarea, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, textarea, ":dir(rtl)").unwrap());

        assert!(host.set_text_content(textarea, "A"));
        assert!(engine.matches_host(&host, textarea, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, textarea, ":dir(rtl)").unwrap());

        assert!(host.set_input_value(textarea, "\u{05ea}"));
        assert!(host.set_text_content(textarea, "A"));
        assert!(!engine.matches_host(&host, textarea, ":dir(ltr)").unwrap());
        assert!(engine.matches_host(&host, textarea, ":dir(rtl)").unwrap());

        assert!(host.set_input_value(textarea, "A"));
        assert!(engine.matches_host(&host, textarea, ":dir(ltr)").unwrap());
        assert!(!engine.matches_host(&host, textarea, ":dir(rtl)").unwrap());
    }

    #[test]
    fn dom_api_selectors_dir_auto_resolves_slot_directionality_boundaries() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let engine = QueryEngine;

        let assigned_host = host.create_element("div");
        assert!(host.append_child(body, assigned_host));
        let assigned_text = host.create_text_node("\u{0627}\u{062e}\u{062a}\u{0628}\u{0631}");
        assert!(host.append_child(assigned_host, assigned_text));
        let assigned_shadow = host.attach_shadow_root(assigned_host, "open").unwrap();
        let auto_slot = host.create_element("slot");
        assert!(host.set_attribute(auto_slot, "dir", "auto"));
        assert!(host.append_child(assigned_shadow, auto_slot));

        assert!(engine.matches_host(&host, auto_slot, ":dir(rtl)").unwrap());
        assert!(!engine.matches_host(&host, auto_slot, ":dir(ltr)").unwrap());

        let inherited_host = host.create_element("div");
        assert!(host.set_attribute(inherited_host, "dir", "rtl"));
        assert!(host.append_child(body, inherited_host));
        let inherited_shadow = host.attach_shadow_root(inherited_host, "open").unwrap();
        let auto_container = host.create_element("div");
        assert!(host.set_attribute(auto_container, "dir", "auto"));
        let inherited_slot = host.create_element("slot");
        let later_ltr_text = host.create_text_node("A");
        assert!(host.append_child(auto_container, inherited_slot));
        assert!(host.append_child(auto_container, later_ltr_text));
        assert!(host.append_child(inherited_shadow, auto_container));

        assert!(
            engine
                .matches_host(&host, auto_container, ":dir(rtl)")
                .unwrap()
        );
        assert!(
            !engine
                .matches_host(&host, auto_container, ":dir(ltr)")
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_invalid_form_walks_deep_dom_iteratively() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let form = host.create_element("form");
        assert!(host.append_child(body, form));

        let mut parent = form;
        for _ in 0..4096 {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }

        let input = host.create_element("input");
        assert!(host.set_attribute(input, "required", ""));
        assert!(host.append_child(parent, input));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, input, ":invalid").unwrap());
        assert!(engine.matches_host(&host, form, ":invalid").unwrap());
    }

    #[test]
    fn dom_api_selectors_invalid_textarea_uses_required_branch() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let textarea = host.create_element("textarea");
        assert!(host.set_attribute(textarea, "required", ""));
        assert!(host.append_child(body, textarea));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, textarea, ":invalid").unwrap());
        assert!(!engine.matches_host(&host, textarea, ":valid").unwrap());
    }

    #[test]
    fn dom_api_selectors_optional_is_independent_of_required_support() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let engine = QueryEngine;

        let input = host.create_element("input");
        assert!(host.set_attribute(input, "required", ""));
        assert!(host.append_child(body, input));
        assert!(engine.matches_host(&host, input, ":required").unwrap());
        assert!(!engine.matches_host(&host, input, ":optional").unwrap());

        assert!(host.set_attribute(input, "type", "hidden"));
        assert!(!engine.matches_host(&host, input, ":required").unwrap());
        assert!(engine.matches_host(&host, input, ":optional").unwrap());

        assert!(host.set_attribute(input, "type", "button"));
        assert!(!engine.matches_host(&host, input, ":required").unwrap());
        assert!(engine.matches_host(&host, input, ":optional").unwrap());

        assert!(host.set_attribute(input, "type", "text"));
        assert!(engine.matches_host(&host, input, ":required").unwrap());
        assert!(!engine.matches_host(&host, input, ":optional").unwrap());
        assert!(host.remove_attribute(input, "required"));
        assert!(!engine.matches_host(&host, input, ":required").unwrap());
        assert!(engine.matches_host(&host, input, ":optional").unwrap());

        let button = host.create_element("button");
        assert!(host.set_attribute(button, "required", ""));
        assert!(host.append_child(body, button));
        assert!(!engine.matches_host(&host, button, ":required").unwrap());
        assert!(engine.matches_host(&host, button, ":optional").unwrap());

        for local_name in ["select", "textarea"] {
            let control = host.create_element(local_name);
            assert!(host.set_attribute(control, "required", ""));
            assert!(host.append_child(body, control));
            assert!(engine.matches_host(&host, control, ":required").unwrap());
            assert!(!engine.matches_host(&host, control, ":optional").unwrap());
            assert!(host.remove_attribute(control, "required"));
            assert!(!engine.matches_host(&host, control, ":required").unwrap());
            assert!(engine.matches_host(&host, control, ":optional").unwrap());
        }

        let div = host.create_element("div");
        assert!(host.set_attribute(div, "required", ""));
        assert!(host.append_child(body, div));
        assert!(!engine.matches_host(&host, div, ":required").unwrap());
        assert!(!engine.matches_host(&host, div, ":optional").unwrap());
    }

    #[test]
    fn dom_api_selectors_readonly_input_is_barred_from_range_pseudos() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let in_range = host.create_element("input");
        assert!(host.set_attribute(in_range, "type", "number"));
        assert!(host.set_attribute(in_range, "min", "1"));
        assert!(host.set_attribute(in_range, "max", "10"));
        assert!(host.set_attribute(in_range, "value", "5"));
        assert!(host.set_input_value(in_range, "5"));
        let out_of_range = host.create_element("input");
        assert!(host.set_attribute(out_of_range, "type", "number"));
        assert!(host.set_attribute(out_of_range, "min", "1"));
        assert!(host.set_attribute(out_of_range, "max", "10"));
        assert!(host.set_attribute(out_of_range, "value", "12"));
        assert!(host.set_input_value(out_of_range, "12"));
        assert!(host.append_child(body, in_range));
        assert!(host.append_child(body, out_of_range));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, in_range, ":in-range").unwrap());
        assert!(
            engine
                .matches_host(&host, out_of_range, ":out-of-range")
                .unwrap()
        );

        assert!(host.set_attribute(in_range, "readonly", ""));
        assert!(host.set_attribute(out_of_range, "readonly", ""));
        assert!(!engine.matches_host(&host, in_range, ":in-range").unwrap());
        assert!(
            !engine
                .matches_host(&host, out_of_range, ":out-of-range")
                .unwrap()
        );

        assert!(host.remove_attribute(in_range, "readonly"));
        assert!(host.remove_attribute(out_of_range, "readonly"));
        assert!(engine.matches_host(&host, in_range, ":in-range").unwrap());
        assert!(
            engine
                .matches_host(&host, out_of_range, ":out-of-range")
                .unwrap()
        );
    }

    #[test]
    fn dom_api_selectors_readonly_controls_are_barred_from_validity_pseudos() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let input = host.create_element("input");
        assert!(host.set_attribute(input, "type", "number"));
        assert!(host.set_attribute(input, "min", "1"));
        assert!(host.set_attribute(input, "max", "10"));
        assert!(host.set_attribute(input, "value", "12"));
        assert!(host.set_input_value(input, "12"));
        assert!(host.set_attribute(input, "readonly", ""));
        assert!(host.append_child(body, input));

        let textarea = host.create_element("textarea");
        assert!(host.set_attribute(textarea, "required", ""));
        assert!(host.set_attribute(textarea, "readonly", ""));
        assert!(host.append_child(body, textarea));

        let engine = QueryEngine;
        assert!(!engine.matches_host(&host, input, ":valid").unwrap());
        assert!(!engine.matches_host(&host, input, ":invalid").unwrap());
        assert!(!engine.matches_host(&host, input, ":in-range").unwrap());
        assert!(!engine.matches_host(&host, input, ":out-of-range").unwrap());
        assert!(!engine.matches_host(&host, textarea, ":valid").unwrap());
        assert!(!engine.matches_host(&host, textarea, ":invalid").unwrap());
    }

    #[test]
    fn dom_api_selectors_default_checkbox_uses_checked_content_attribute() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let input = host.create_element("input");
        assert!(host.set_attribute(input, "type", "checkbox"));
        assert!(host.append_child(body, input));

        let engine = QueryEngine;
        assert!(!engine.matches_host(&host, input, ":default").unwrap());
        assert!(host.set_checked_state(input, true));
        assert!(!engine.matches_host(&host, input, ":default").unwrap());
        assert!(host.set_attribute(input, "checked", ""));
        assert!(engine.matches_host(&host, input, ":default").unwrap());
        assert!(host.set_checked_state(input, false));
        assert!(engine.matches_host(&host, input, ":default").unwrap());
        assert!(host.remove_attribute(input, "checked"));
        assert!(!engine.matches_host(&host, input, ":default").unwrap());
    }

    #[test]
    fn dom_api_webkit_autofill_alias_matches_autofilled_state() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let input = host.create_element("input");
        assert!(host.append_child(body, input));

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_host(&host, ":-webkit-autofill")
                .unwrap(),
            None
        );

        assert!(host.set_autofilled_state(input, true));
        assert_eq!(
            engine
                .query_selector_host(&host, ":-webkit-autofill")
                .unwrap(),
            Some(input)
        );
        assert!(
            engine
                .matches_host(&host, input, ":-WeBkIt-AuToFiLl")
                .unwrap()
        );
        assert!(engine.matches_host(&host, input, ":autofill").unwrap());
    }

    #[test]
    fn dom_api_selectors_disabled_select_disables_option_descendants() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let select = host.create_element("select");
        let optgroup = host.create_element("optgroup");
        let option = host.create_element("option");
        assert!(host.append_child(body, select));
        assert!(host.append_child(select, optgroup));
        assert!(host.append_child(optgroup, option));

        let engine = QueryEngine;
        assert!(!engine.matches_host(&host, optgroup, ":disabled").unwrap());
        assert!(!engine.matches_host(&host, option, ":disabled").unwrap());
        assert!(
            !engine
                .matches_host(&host, select, ":has(option:disabled)")
                .unwrap()
        );

        assert!(host.set_attribute(select, "disabled", ""));
        assert!(engine.matches_host(&host, optgroup, ":disabled").unwrap());
        assert!(engine.matches_host(&host, option, ":disabled").unwrap());
        assert!(
            engine
                .matches_host(&host, select, ":has(option:disabled)")
                .unwrap()
        );

        assert!(host.remove_attribute(select, "disabled"));
        assert!(!engine.matches_host(&host, optgroup, ":disabled").unwrap());
        assert!(!engine.matches_host(&host, option, ":disabled").unwrap());
    }

    #[test]
    fn dom_api_selectors_option_disabledness_respects_association_boundaries() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let engine = QueryEngine;

        let select = host.create_element("select");
        let optgroup = host.create_element("optgroup");
        let div = host.create_element("div");
        let option = host.create_element("option");
        assert!(host.append_child(body, select));
        assert!(host.append_child(select, optgroup));
        assert!(host.append_child(optgroup, div));
        assert!(host.append_child(div, option));
        assert!(host.set_attribute(optgroup, "disabled", ""));
        assert!(engine.matches_host(&host, option, ":disabled").unwrap());

        let disabled_select = host.create_element("select");
        let valid_optgroup = host.create_element("optgroup");
        let valid_option = host.create_element("option");
        let nested_optgroup = host.create_element("optgroup");
        let nested_optgroup_option = host.create_element("option");
        let parent_option = host.create_element("option");
        let nested_option = host.create_element("option");
        let hr = host.create_element("hr");
        let hr_option = host.create_element("option");
        let datalist = host.create_element("datalist");
        let datalist_option = host.create_element("option");
        assert!(host.set_attribute(disabled_select, "disabled", ""));
        assert!(host.append_child(body, disabled_select));
        assert!(host.append_child(disabled_select, valid_optgroup));
        assert!(host.append_child(valid_optgroup, valid_option));
        assert!(host.append_child(valid_optgroup, nested_optgroup));
        assert!(host.append_child(nested_optgroup, nested_optgroup_option));
        assert!(host.append_child(disabled_select, parent_option));
        assert!(host.append_child(parent_option, nested_option));
        assert!(host.append_child(disabled_select, hr));
        assert!(host.append_child(hr, hr_option));
        assert!(host.append_child(disabled_select, datalist));
        assert!(host.append_child(datalist, datalist_option));

        assert!(
            engine
                .matches_host(&host, valid_optgroup, ":disabled")
                .unwrap()
        );
        assert!(
            engine
                .matches_host(&host, valid_option, ":disabled")
                .unwrap()
        );
        for handle in [
            nested_optgroup,
            nested_optgroup_option,
            nested_option,
            hr_option,
            datalist_option,
        ] {
            assert!(!engine.matches_host(&host, handle, ":disabled").unwrap());
        }
    }

    #[test]
    fn dom_api_selectors_disabled_fieldset_disables_option_descendants() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let fieldset = host.create_element("fieldset");
        let select = host.create_element("select");
        let optgroup = host.create_element("optgroup");
        let option = host.create_element("option");
        assert!(host.append_child(body, fieldset));
        assert!(host.append_child(fieldset, select));
        assert!(host.append_child(select, optgroup));
        assert!(host.append_child(optgroup, option));

        let engine = QueryEngine;
        assert!(!engine.matches_host(&host, optgroup, ":disabled").unwrap());
        assert!(!engine.matches_host(&host, option, ":disabled").unwrap());

        assert!(host.set_attribute(fieldset, "disabled", ""));
        assert!(engine.matches_host(&host, optgroup, ":disabled").unwrap());
        assert!(engine.matches_host(&host, option, ":disabled").unwrap());

        assert!(host.remove_attribute(fieldset, "disabled"));
        assert!(!engine.matches_host(&host, optgroup, ":disabled").unwrap());
        assert!(!engine.matches_host(&host, option, ":disabled").unwrap());
    }

    #[test]
    fn dom_api_selectors_radio_indeterminate_walks_deep_dom_iteratively() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("input");
        assert!(host.set_attribute(target, "type", "radio"));
        assert!(host.set_attribute(target, "name", "group"));
        assert!(host.append_child(body, target));

        let mut parent = body;
        for _ in 0..4096 {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }
        let checked = host.create_element("input");
        assert!(host.set_attribute(checked, "type", "radio"));
        assert!(host.set_attribute(checked, "name", "group"));
        assert!(host.set_attribute(checked, "checked", ""));
        assert!(host.append_child(parent, checked));

        let engine = QueryEngine;
        assert!(
            !engine
                .matches_host(&host, target, ":indeterminate")
                .unwrap()
        );
        assert!(engine.matches_host(&host, checked, ":checked").unwrap());
    }

    #[test]
    fn dom_api_selectors_default_submit_walks_deep_dom_iteratively() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let form = host.create_element("form");
        assert!(host.append_child(body, form));

        let mut parent = form;
        for _ in 0..4096 {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }
        let submit = host.create_element("button");
        assert!(host.append_child(parent, submit));

        let engine = QueryEngine;
        assert!(engine.matches_host(&host, submit, ":default").unwrap());
    }

    #[test]
    fn target_pseudo_class_matches_current_fragment_target() {
        let url = url::Url::parse("https://example.test/page.html#target").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "target"));
        let other = host.create_element("div");
        assert!(host.set_attribute(other, "id", "other"));
        assert!(host.append_child(body, target));
        assert!(host.append_child(body, other));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, ":target").unwrap(),
            Some(target)
        );
        assert!(engine.matches_host(&host, target, ":target").unwrap());
        assert!(!engine.matches_host(&host, other, ":target").unwrap());
    }

    #[test]
    fn target_pseudo_class_matches_percent_decoded_anchor_name() {
        let url = url::Url::parse("https://example.test/page.html#caf%C3%A9").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("a");
        assert!(host.set_attribute(target, "name", "café"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, "a:target").unwrap(),
            Some(target)
        );
    }

    #[test]
    fn target_pseudo_class_matches_percent_decoded_element_id() {
        let url = url::Url::parse("https://example.test/page.html#foo%20bar").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "id", "foo bar"));
        assert!(host.append_child(body, target));

        let engine = QueryEngine;
        assert_eq!(
            engine.query_selector_host(&host, ":target").unwrap(),
            Some(target)
        );
    }

    #[test]
    fn simple_class_descendant_selector_uses_stylo_semantics() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url));
        host.reset_html_document_shell();
        let body = host.document_body_handle().unwrap();
        let slide = host.create_element("div");
        assert!(host.set_attribute(slide, "class", "slick-slide active"));
        let image = host.create_element("img");
        let other = host.create_element("img");
        assert!(host.append_child(slide, image));
        assert!(host.append_child(body, slide));
        assert!(host.append_child(body, other));

        let engine = QueryEngine;
        assert_eq!(
            engine
                .query_selector_all_in_host(&host, body, ".slick-slide")
                .unwrap(),
            vec![slide]
        );
        assert_eq!(
            engine
                .query_selector_all_in_host(&host, body, ".slick-slide img")
                .unwrap(),
            vec![image]
        );
        assert!(
            engine
                .matches_host(&host, image, ".slick-slide img")
                .unwrap()
        );
        assert!(
            !engine
                .matches_host(&host, other, ".slick-slide img")
                .unwrap()
        );
        assert_eq!(
            engine
                .closest_host(&host, image, ".slick-slide img")
                .unwrap(),
            Some(image)
        );
    }
}
