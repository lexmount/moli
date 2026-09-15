use url::Url;

use super::Document;
use crate::native::{NativeDom, NativeNodeId, Node};

/// A newly frozen base URL awaiting the embedding document's policy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentBaseUrlPolicyCheck {
    pub document: NativeNodeId,
    pub element: NativeNodeId,
    pub url: Url,
}

#[derive(Debug, Clone)]
pub(super) struct DocumentBaseUrlState {
    fallback_base_url: Url,
    base_url_override: Option<Url>,
    base_element_url: Option<Url>,
    base_element_input: Option<(NativeNodeId, String)>,
    pending_policy_check: Option<DocumentBaseUrlPolicyCheck>,
    base_url: Url,
    base_target: Option<Box<str>>,
}

impl DocumentBaseUrlState {
    pub(super) fn new(document_url: &Url) -> Self {
        Self {
            fallback_base_url: document_url.clone(),
            base_url_override: None,
            base_element_url: None,
            base_element_input: None,
            pending_policy_check: None,
            base_url: document_url.clone(),
            base_target: None,
        }
    }

    pub(super) fn fallback_base_url(&self) -> &Url {
        &self.fallback_base_url
    }

    pub(super) fn base_element_url(&self) -> Option<&Url> {
        self.base_element_url.as_ref()
    }

    pub(super) fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub(super) fn base_target(&self) -> Option<&str> {
        self.base_target.as_deref()
    }

    pub(super) fn set_document_url(&mut self, url: &Url) {
        self.fallback_base_url = url.clone();
    }

    pub(super) fn set_fallback_base_url(&mut self, url: Url) {
        self.fallback_base_url = url;
    }

    pub(super) fn set_base_url_override(&mut self, url: Option<Url>) {
        self.base_url_override = url;
        self.update_base_url();
    }

    fn process_base_element(
        &mut self,
        base_element_input: Option<(NativeNodeId, String)>,
        base_element_url: Option<Url>,
        policy_check: Option<DocumentBaseUrlPolicyCheck>,
        base_target: Option<String>,
        force_base_url_update: bool,
    ) -> bool {
        // A document URL change must preserve the element's frozen URL,
        // including the fallback frozen when CSP rejected its href.
        let changed = self.base_element_input != base_element_input
            || (force_base_url_update && self.base_element_url.is_none());
        if changed {
            self.base_element_input = base_element_input;
            self.base_element_url = base_element_url;
            self.pending_policy_check = policy_check;
            self.update_base_url();
        }
        self.base_target = base_target.map(String::into_boxed_str);
        changed
    }

    fn update_base_url(&mut self) {
        self.base_url = self
            .base_element_url
            .as_ref()
            .or(self.base_url_override.as_ref())
            .unwrap_or(&self.fallback_base_url)
            .clone();
    }
}

impl NativeDom {
    pub(crate) fn process_base_element_for_node(&mut self, node_id: NativeNodeId) {
        let Some(document) = self.document_tree_owner(node_id) else {
            return;
        };
        self.process_base_element(document, false);
    }

    pub(crate) fn process_base_element(
        &mut self,
        document_node_id: NativeNodeId,
        force_base_url_update: bool,
    ) {
        let Some(fallback_base_url) = self
            .node(document_node_id)
            .and_then(Node::as_document)
            .map(Document::fallback_base_url)
            .cloned()
        else {
            return;
        };
        let (base_element_input, target) = self.first_base_element_attributes(document_node_id);
        let mut policy_check = None;
        let base_element_url = base_element_input.as_ref().and_then(|(element, href)| {
            let href = trim_html_spaces(href);
            if href.is_empty() {
                return None;
            }
            let url = fallback_base_url
                .join(href)
                .ok()
                .filter(|url| !matches!(url.scheme(), "data" | "javascript"));
            if let Some(url) = &url {
                policy_check = Some(DocumentBaseUrlPolicyCheck {
                    document: document_node_id,
                    element: *element,
                    url: url.clone(),
                });
            }
            Some(url.unwrap_or_else(|| fallback_base_url.clone()))
        });
        if let Some(document) = self
            .node_mut(document_node_id)
            .and_then(|node| node.data_mut().as_document_mut())
        {
            let changed = document.base_url_state.process_base_element(
                base_element_input,
                base_element_url,
                policy_check,
                target,
                force_base_url_update,
            );
            if changed && !self.pending_base_url_documents.contains(&document_node_id) {
                self.pending_base_url_documents.push(document_node_id);
            }
        }
    }

    pub(crate) fn take_base_url_policy_checks(&mut self) -> Vec<DocumentBaseUrlPolicyCheck> {
        std::mem::take(&mut self.pending_base_url_documents)
            .into_iter()
            .filter_map(|handle| {
                self.node_mut(handle)
                    .and_then(|node| node.data_mut().as_document_mut())?
                    .base_url_state
                    .pending_policy_check
                    .take()
            })
            .collect()
    }

    pub(crate) fn reject_base_url_policy_check(&mut self, check: &DocumentBaseUrlPolicyCheck) {
        let Some(document) = self
            .node_mut(check.document)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return;
        };
        let state = &mut document.base_url_state;
        if state
            .base_element_input
            .as_ref()
            .map(|(element, _)| *element)
            == Some(check.element)
            && state.base_element_url.as_ref() == Some(&check.url)
        {
            state.base_element_url = Some(state.fallback_base_url.clone());
            state.update_base_url();
        }
    }

    pub(crate) fn document_tree_owner(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        let mut current = Some(node_id);
        while let Some(handle) = current {
            let node = self.node(handle)?;
            if node.is_document() {
                return Some(handle);
            }
            current = node.parent_node();
        }
        None
    }

    pub(crate) fn is_base_state_owner(&self, node_id: NativeNodeId) -> bool {
        self.node(node_id)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_element("base")
                    && (element.attribute_ns("", "href").is_some()
                        || element.attribute_ns("", "target").is_some())
            })
    }

    pub(crate) fn base_element_attribute_owner_document(
        &self,
        node_id: NativeNodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<NativeNodeId> {
        let is_base_state_attribute = if namespace.is_some() {
            namespace == Some("") && matches!(local_name, "href" | "target")
        } else {
            local_name.eq_ignore_ascii_case("href") || local_name.eq_ignore_ascii_case("target")
        };
        if !is_base_state_attribute
            || !self
                .node(node_id)
                .is_some_and(|node| node.is_html_element_named("base"))
        {
            return None;
        }
        self.document_tree_owner(node_id)
    }

    fn first_base_element_attributes(
        &self,
        document_node_id: NativeNodeId,
    ) -> (Option<(NativeNodeId, String)>, Option<String>) {
        let mut href = None;
        let mut target = None;
        let mut current = self.first_child(document_node_id);
        while let Some(node_id) = current {
            let Some(element) = self.node(node_id).and_then(Node::as_element) else {
                current = self.next_in_preorder(node_id, document_node_id);
                continue;
            };
            if element.is_html_element("base") {
                if href.is_none()
                    && let Some(value) = element.attribute_ns("", "href")
                {
                    href = Some((node_id, value.to_owned()));
                }
                if target.is_none()
                    && let Some(value) = element.attribute_ns("", "target")
                {
                    target = Some(value.to_owned());
                }
            }
            if href.is_some() && target.is_some() {
                break;
            }
            current = self.next_in_preorder(node_id, document_node_id);
        }
        (href, target)
    }

    fn next_in_preorder(&self, node_id: NativeNodeId, root: NativeNodeId) -> Option<NativeNodeId> {
        if let Some(first_child) = self.first_child(node_id) {
            return Some(first_child);
        }
        let mut current = node_id;
        loop {
            if let Some(next_sibling) = self.next_sibling(current) {
                return Some(next_sibling);
            }
            current = self.parent_node(current)?;
            if current == root {
                return None;
            }
        }
    }
}

fn trim_html_spaces(value: &str) -> &str {
    value.trim_matches(['\t', '\n', '\u{000c}', '\r', ' '])
}
