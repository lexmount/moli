use std::collections::{HashMap, HashSet};

use moli_page_types::{DocumentNodeInspectorIdentity, DocumentNodeSnapshot};
use moli_selector::QueryEngine;
use url::Url;

use super::PageVm;
use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, NativeDom, NodeData, NodeType};
use crate::runtime::page_dom::live_document_node_snapshot;
use crate::runtime::page_generated_dom::user_agent_shadow_root_snapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum DocumentSearchMatch {
    Live(DomHandle),
    Generated {
        host: DomHandle,
        identity: DocumentNodeInspectorIdentity,
    },
}

impl PageVm {
    pub(crate) fn document_perform_search(
        &mut self,
        inspector_session_id: Option<&str>,
        query: &str,
        include_user_agent_shadow_dom: bool,
    ) -> crate::RendererDomSearchRegistration {
        let mut documents = vec![
            self.vm()
                .document_runtime
                .dom_host()
                .dom()
                .document_node_id(),
        ];
        documents.extend(
            self.inspector_child_document_handles()
                .into_iter()
                .map(|(_, document)| document),
        );
        let matches = search_document_matches(
            self.vm().document_runtime.dom_host(),
            &documents,
            query,
            include_user_agent_shadow_dom,
        );
        self.register_document_search_results(inspector_session_id, matches)
    }
}

fn search_document_matches(
    dom_host: &DomHost,
    documents: &[DomHandle],
    query: &str,
    include_user_agent_shadow_dom: bool,
) -> Vec<DocumentSearchMatch> {
    let query = query.trim();
    let mut results = OrderedSearchMatches::default();

    for document in documents {
        collect_selector_matches(
            dom_host,
            *document,
            query,
            include_user_agent_shadow_dom,
            &mut results,
        );
    }

    let manual_query = ManualSearchQuery::new(query);
    for document in documents {
        let Some(document_element) = dom_host
            .dom()
            .document_element_handle_for_document(*document)
        else {
            continue;
        };
        collect_manual_matches(
            dom_host,
            document_element,
            include_user_agent_shadow_dom,
            &manual_query,
            &mut results,
        );
    }

    for document in documents {
        for handle in crate::native_bridge::document::evaluate_live_xpath_search_node_handles(
            dom_host, query, *document,
        ) {
            results.insert(DocumentSearchMatch::Live(handle));
        }
    }

    results.into_vec()
}

fn collect_selector_matches(
    dom_host: &DomHost,
    document: DomHandle,
    query: &str,
    include_user_agent_shadow_dom: bool,
    results: &mut OrderedSearchMatches,
) {
    let engine = QueryEngine;
    let document_matches = if document == dom_host.dom().document_node_id() {
        engine.query_selector_all_host(dom_host, query)
    } else {
        engine.query_selector_all_in_host(dom_host, document, query)
    };
    if let Ok(matches) = document_matches {
        results.extend(matches.into_iter().map(DocumentSearchMatch::Live));
    }

    let Some(document_element) = dom_host
        .dom()
        .document_element_handle_for_document(document)
    else {
        return;
    };
    collect_shadow_selector_matches(
        dom_host,
        document_element,
        query,
        include_user_agent_shadow_dom,
        results,
    );
}

fn collect_shadow_selector_matches(
    dom_host: &DomHost,
    handle: DomHandle,
    query: &str,
    include_user_agent_shadow_dom: bool,
    results: &mut OrderedSearchMatches,
) {
    if let Some(shadow_root) = dom_host.shadow_root_handle(handle) {
        if let Ok(matches) = QueryEngine.query_selector_all_in_host(dom_host, shadow_root, query) {
            results.extend(matches.into_iter().map(DocumentSearchMatch::Live));
        }
        for child in dom_host.child_handles(shadow_root) {
            collect_shadow_selector_matches(
                dom_host,
                child,
                query,
                include_user_agent_shadow_dom,
                results,
            );
        }
    } else if include_user_agent_shadow_dom
        && let Some(root) = generated_user_agent_shadow_root(dom_host, handle)
    {
        results.extend(generated_selector_matches(dom_host, handle, &root, query));
    }

    for child in dom_host.child_handles(handle) {
        collect_shadow_selector_matches(
            dom_host,
            child,
            query,
            include_user_agent_shadow_dom,
            results,
        );
    }
}

fn collect_manual_matches(
    dom_host: &DomHost,
    handle: DomHandle,
    include_user_agent_shadow_dom: bool,
    query: &ManualSearchQuery,
    results: &mut OrderedSearchMatches,
) {
    if let Some(node) = dom_host.node(handle) {
        let matched = match node.kind() {
            NodeData::Text(text) => query.matches_node_value(text.data()),
            NodeData::CDataSection(cdata) => query.matches_node_value(cdata.data()),
            NodeData::Comment(comment) => query.matches_node_value(comment.data()),
            NodeData::Element(element) => {
                query.matches_element_name(element.local_name())
                    || element.attributes().iter().any(|attribute| {
                        query.matches_attribute_name(&attribute.name())
                            || query.matches_attribute_value(attribute.value())
                    })
            }
            _ => false,
        };
        if matched {
            results.insert(DocumentSearchMatch::Live(handle));
        }
    }

    if let Some(shadow_root) = dom_host.shadow_root_handle(handle) {
        collect_manual_matches(
            dom_host,
            shadow_root,
            include_user_agent_shadow_dom,
            query,
            results,
        );
    } else if include_user_agent_shadow_dom
        && let Some(root) = generated_user_agent_shadow_root(dom_host, handle)
    {
        collect_generated_manual_matches(handle, &root, query, results);
    }

    for child in dom_host.child_handles(handle) {
        collect_manual_matches(
            dom_host,
            child,
            include_user_agent_shadow_dom,
            query,
            results,
        );
    }
}

fn generated_user_agent_shadow_root(
    dom_host: &DomHost,
    host: DomHandle,
) -> Option<DocumentNodeSnapshot> {
    let originating_element = live_document_node_snapshot(dom_host, host, 0, None, false)?;
    user_agent_shadow_root_snapshot(dom_host, &originating_element, -1, true)
}

fn generated_search_match(
    host: DomHandle,
    snapshot: &DocumentNodeSnapshot,
) -> Option<DocumentSearchMatch> {
    Some(DocumentSearchMatch::Generated {
        host,
        identity: snapshot.inspector_identity?,
    })
}

fn collect_generated_manual_matches(
    host: DomHandle,
    snapshot: &DocumentNodeSnapshot,
    query: &ManualSearchQuery,
    results: &mut OrderedSearchMatches,
) {
    let matched = if snapshot.is_element {
        query.matches_element_name(&snapshot.local_name)
            || snapshot.attributes.iter().any(|attribute| {
                query.matches_attribute_name(&attribute.local_name)
                    || query.matches_attribute_value(&attribute.value)
            })
    } else if matches!(
        snapshot.node_type,
        value if value == NodeType::Text as u8
            || value == NodeType::CDataSection as u8
            || value == NodeType::Comment as u8
    ) {
        query.matches_node_value(&snapshot.node_value)
    } else {
        false
    };
    if matched && let Some(result) = generated_search_match(host, snapshot) {
        results.insert(result);
    }
    for child in &snapshot.children {
        collect_generated_manual_matches(host, child, query, results);
    }
}

fn generated_selector_matches(
    source_dom_host: &DomHost,
    host: DomHandle,
    root: &DocumentNodeSnapshot,
    query: &str,
) -> Vec<DocumentSearchMatch> {
    let url = source_dom_host
        .document_url_for_handle(
            source_dom_host
                .owner_document_handle(host)
                .unwrap_or_else(|| source_dom_host.document_handle()),
        )
        .cloned()
        .unwrap_or_else(|| Url::parse("about:blank").expect("about:blank must be a valid URL"));
    let mut projection = DomHost::from_dom(NativeDom::new_html(url));
    let projection_root = projection.create_document_fragment();
    let mut identities = HashMap::new();
    for child in &root.children {
        append_generated_selector_projection(
            &mut projection,
            projection_root,
            host,
            child,
            &mut identities,
        );
    }
    QueryEngine
        .query_selector_all_in_host(&projection, projection_root, query)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|handle| identities.get(&handle).copied())
        .collect()
}

fn append_generated_selector_projection(
    projection: &mut DomHost,
    parent: DomHandle,
    source_host: DomHandle,
    snapshot: &DocumentNodeSnapshot,
    identities: &mut HashMap<DomHandle, DocumentSearchMatch>,
) {
    let handle = if snapshot.is_element {
        projection.create_element_ns(snapshot.namespace_uri.as_deref(), &snapshot.local_name)
    } else if snapshot.node_type == NodeType::Text as u8 {
        Some(projection.create_text_node(&snapshot.node_value))
    } else if snapshot.node_type == NodeType::CDataSection as u8 {
        Some(projection.create_cdata_section(&snapshot.node_value))
    } else if snapshot.node_type == NodeType::Comment as u8 {
        Some(projection.create_comment(&snapshot.node_value))
    } else {
        None
    };
    let Some(handle) = handle else {
        return;
    };
    for attribute in &snapshot.attributes {
        let _ = projection.set_attribute(handle, &attribute.local_name, &attribute.value);
    }
    let _ = projection.append_child_without_mutation_effects(parent, handle);
    if let Some(search_match) = generated_search_match(source_host, snapshot) {
        identities.insert(handle, search_match);
    }
    for child in &snapshot.children {
        append_generated_selector_projection(projection, handle, source_host, child, identities);
    }
}

#[derive(Default)]
struct OrderedSearchMatches {
    seen: HashSet<DocumentSearchMatch>,
    values: Vec<DocumentSearchMatch>,
}

impl OrderedSearchMatches {
    fn insert(&mut self, search_match: DocumentSearchMatch) {
        if self.seen.insert(search_match) {
            self.values.push(search_match);
        }
    }

    fn extend(&mut self, matches: impl IntoIterator<Item = DocumentSearchMatch>) {
        for search_match in matches {
            self.insert(search_match);
        }
    }

    fn into_vec(self) -> Vec<DocumentSearchMatch> {
        self.values
    }
}

struct ManualSearchQuery {
    raw_lowercase: String,
    tag_lowercase: String,
    tag_match: TagNameMatch,
    attribute_value_lowercase: String,
    exact_attribute_value: bool,
}

impl ManualSearchQuery {
    fn new(query: &str) -> Self {
        let (tag_query, tag_match) = tag_name_query(query);
        let (attribute_value_query, exact_attribute_value) = attribute_value_query(query);
        Self {
            raw_lowercase: query.to_lowercase(),
            tag_lowercase: tag_query.to_lowercase(),
            tag_match,
            attribute_value_lowercase: attribute_value_query.to_lowercase(),
            exact_attribute_value,
        }
    }

    fn matches_node_value(&self, value: &str) -> bool {
        value.to_lowercase().contains(&self.raw_lowercase)
    }

    fn matches_element_name(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        match self.tag_match {
            TagNameMatch::Substring => name.contains(&self.tag_lowercase),
            TagNameMatch::Exact => name == self.tag_lowercase,
            TagNameMatch::Prefix => name.starts_with(&self.tag_lowercase),
            TagNameMatch::Suffix => name.ends_with(&self.tag_lowercase),
        }
    }

    fn matches_attribute_name(&self, name: &str) -> bool {
        name.to_lowercase().contains(&self.raw_lowercase)
    }

    fn matches_attribute_value(&self, value: &str) -> bool {
        let value = value.to_lowercase();
        if self.exact_attribute_value {
            value == self.attribute_value_lowercase
        } else {
            value.contains(&self.attribute_value_lowercase)
        }
    }
}

#[derive(Clone, Copy)]
enum TagNameMatch {
    Substring,
    Exact,
    Prefix,
    Suffix,
}

fn tag_name_query(query: &str) -> (&str, TagNameMatch) {
    let (query, has_start) = if let Some(query) = query.strip_prefix("</") {
        (query, true)
    } else if let Some(query) = query.strip_prefix('<') {
        (query, true)
    } else {
        (query, false)
    };
    let (query, has_end) = query
        .strip_suffix('>')
        .map_or((query, false), |query| (query, true));
    let match_type = match (has_start, has_end) {
        (false, false) => TagNameMatch::Substring,
        (true, true) => TagNameMatch::Exact,
        (true, false) => TagNameMatch::Prefix,
        (false, true) => TagNameMatch::Suffix,
    };
    (query, match_type)
}

fn attribute_value_query(query: &str) -> (&str, bool) {
    let has_start = query.starts_with('"');
    let has_end = query.ends_with('"');
    let query = query.strip_prefix('"').unwrap_or(query);
    let query = query.strip_suffix('"').unwrap_or(query);
    (query, has_start && has_end)
}

#[cfg(test)]
mod tests {
    use super::{TagNameMatch, attribute_value_query, tag_name_query};

    #[test]
    fn chromium_search_delimiters_select_tag_and_attribute_match_modes() {
        let (query, match_type) = tag_name_query("</article>");
        assert_eq!(query, "article");
        assert!(matches!(match_type, TagNameMatch::Exact));

        let (query, exact) = attribute_value_query("\"needle\"");
        assert_eq!(query, "needle");
        assert!(exact);
    }
}
