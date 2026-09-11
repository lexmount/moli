//! Renderer-independent stylesheet blocking discovery contract.
//!
//! The DOM owns candidate membership and order. This module only reads that
//! canonical registry, classifies candidates, and produces blocking inputs.

use std::collections::HashSet;

use url::Url;

use moli_dom::{
    NodeId,
    native::{DomHost, NativeDom, NativeNodeId, Node},
};

use crate::fetcher::StylesheetFetchOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StylesheetLinkDisposition {
    Blocking {
        url: Url,
        options: StylesheetFetchOptions,
    },
    LoadOnly {
        url: Url,
        options: StylesheetFetchOptions,
    },
}

impl StylesheetLinkDisposition {
    pub fn url(&self) -> &Url {
        match self {
            Self::Blocking { url, .. } | Self::LoadOnly { url, .. } => url,
        }
    }

    pub fn options(&self) -> &StylesheetFetchOptions {
        match self {
            Self::Blocking { options, .. } | Self::LoadOnly { options, .. } => options,
        }
    }

    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::Blocking { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StylesheetPreloadLinkRequest {
    url: Url,
    options: StylesheetFetchOptions,
}

impl StylesheetPreloadLinkRequest {
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn options(&self) -> &StylesheetFetchOptions {
        &self.options
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentOwnedBlockingStylesheetCandidate {
    Link {
        node_id: NodeId,
        url: Url,
        options: StylesheetFetchOptions,
    },
    ParserCreatedStyleImport {
        node_id: NodeId,
        urls: Vec<Url>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DocumentBlockingStylesheetSignature {
    Link {
        url: Url,
        options: StylesheetFetchOptions,
    },
    ParserCreatedStyleImport {
        urls: Vec<Url>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentOwnedBlockingStylesheet {
    node_id: NodeId,
    signature: DocumentBlockingStylesheetSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentOwnedBlockingStylesheetDiscoveryInput {
    node_id: NodeId,
    signature: DocumentBlockingStylesheetSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StylesheetElementRead {
    is_html_element: bool,
    local_name: String,
    parser_blocking_eligible: bool,
    rel: Option<String>,
    href: Option<String>,
    as_attr: Option<String>,
    type_attr: Option<String>,
    disabled: bool,
    media: Option<String>,
    cross_origin: Option<String>,
    referrer_policy: Option<String>,
    integrity: Option<String>,
    nonce: Option<String>,
    charset: Option<String>,
    fetch_priority: Option<String>,
}

impl StylesheetElementRead {
    pub fn from_node(node: &Node) -> Option<Self> {
        let element = node.as_element()?;
        Some(Self {
            is_html_element: element.namespace() == "http://www.w3.org/1999/xhtml",
            local_name: element.local_name().to_owned(),
            parser_blocking_eligible: if element.is_html_element("link") {
                element.link_created_by_parser()
            } else {
                node.flags().parser_created()
            },
            rel: element.attribute("rel").map(str::to_owned),
            href: element.attribute("href").map(str::to_owned),
            as_attr: element.attribute("as").map(str::to_owned),
            type_attr: element.attribute("type").map(str::to_owned),
            disabled: element.attribute("disabled").is_some(),
            media: element.attribute("media").map(str::to_owned),
            cross_origin: element.attribute("crossorigin").map(str::to_owned),
            referrer_policy: element.attribute("referrerpolicy").map(str::to_owned),
            integrity: element.attribute("integrity").map(str::to_owned),
            nonce: element
                .cryptographic_nonce()
                .or_else(|| element.attribute("nonce"))
                .map(str::to_owned),
            charset: element.attribute("charset").map(str::to_owned),
            fetch_priority: element.attribute("fetchpriority").map(str::to_owned),
        })
    }

    fn is_html_element(&self, local_name: &str) -> bool {
        self.is_html_element && self.local_name == local_name
    }

    #[cfg(test)]
    pub(crate) fn parser_created_html_link_for_test(href: &str, media: Option<&str>) -> Self {
        Self {
            is_html_element: true,
            local_name: "link".to_owned(),
            parser_blocking_eligible: true,
            rel: Some("stylesheet".to_owned()),
            href: Some(href.to_owned()),
            as_attr: None,
            type_attr: None,
            disabled: false,
            media: media.map(str::to_owned),
            cross_origin: None,
            referrer_policy: None,
            integrity: None,
            nonce: None,
            charset: None,
            fetch_priority: None,
        }
    }
}

impl DocumentOwnedBlockingStylesheet {
    pub fn from_candidate(candidate: &DocumentOwnedBlockingStylesheetCandidate) -> Self {
        Self {
            node_id: candidate.node_id(),
            signature: DocumentBlockingStylesheetSignature::from_candidate(candidate),
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn signature(&self) -> &DocumentBlockingStylesheetSignature {
        &self.signature
    }
}

impl DocumentOwnedBlockingStylesheetDiscoveryInput {
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = node_id;
        self
    }

    pub fn signature(&self) -> &DocumentBlockingStylesheetSignature {
        &self.signature
    }
}

impl From<&DocumentOwnedBlockingStylesheet> for DocumentOwnedBlockingStylesheetDiscoveryInput {
    fn from(blocker: &DocumentOwnedBlockingStylesheet) -> Self {
        Self {
            node_id: blocker.node_id(),
            signature: blocker.signature().clone(),
        }
    }
}

impl From<&DocumentOwnedBlockingStylesheetCandidate>
    for DocumentOwnedBlockingStylesheetDiscoveryInput
{
    fn from(candidate: &DocumentOwnedBlockingStylesheetCandidate) -> Self {
        Self {
            node_id: candidate.node_id(),
            signature: DocumentBlockingStylesheetSignature::from_candidate(candidate),
        }
    }
}

impl DocumentOwnedBlockingStylesheetCandidate {
    pub fn node_id(&self) -> NodeId {
        match self {
            Self::Link { node_id, .. } | Self::ParserCreatedStyleImport { node_id, .. } => *node_id,
        }
    }
}

impl DocumentBlockingStylesheetSignature {
    pub fn from_candidate(candidate: &DocumentOwnedBlockingStylesheetCandidate) -> Self {
        match candidate {
            DocumentOwnedBlockingStylesheetCandidate::Link { url, options, .. } => Self::Link {
                url: url.clone(),
                options: options.clone(),
            },
            DocumentOwnedBlockingStylesheetCandidate::ParserCreatedStyleImport { urls, .. } => {
                Self::ParserCreatedStyleImport { urls: urls.clone() }
            }
        }
    }
}

pub trait StylesheetBlockingReadView {
    fn stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead>;
    fn child_ids(&self, node_id: NativeNodeId) -> Vec<NativeNodeId>;
    fn text_content(&self, node_id: NativeNodeId) -> Option<String>;
    fn final_url_clone(&self) -> Option<Url>;
    fn document_base_url_clone(&self) -> Option<Url>;
    fn document_node_id(&self) -> NativeNodeId;
    fn document_is_quirks_mode(&self) -> bool;

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<NodeId>,
    ) -> Vec<NativeNodeId>;
}

impl StylesheetBlockingReadView for NativeDom {
    fn stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        self.node(node_id)
            .and_then(StylesheetElementRead::from_node)
    }

    fn child_ids(&self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        self.child_ids(node_id).collect()
    }

    fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.text_content(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.final_url().cloned()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        NativeDom::document(self).map(|document| document.base_url().clone())
    }

    fn document_node_id(&self) -> NativeNodeId {
        self.document_node_id()
    }

    fn document_is_quirks_mode(&self) -> bool {
        self.document()
            .is_some_and(|document| document.is_quirks_mode())
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<NodeId>,
    ) -> Vec<NativeNodeId> {
        self.stylesheet_candidate_handles_before_in_tree_scope(
            self.document_node_id(),
            target_node_id.map(|node_id| NativeNodeId::new(node_id.index())),
        )
    }
}

impl StylesheetBlockingReadView for DomHost {
    fn stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        self.node(node_id)
            .and_then(StylesheetElementRead::from_node)
    }

    fn child_ids(&self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        self.child_handles(node_id).collect()
    }

    fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.text_content(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.node(self.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        self.node(self.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.base_url().clone())
    }

    fn document_node_id(&self) -> NativeNodeId {
        self.document_handle()
    }

    fn document_is_quirks_mode(&self) -> bool {
        self.node(self.document_handle())
            .and_then(Node::as_document)
            .is_some_and(|document| document.is_quirks_mode())
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<NodeId>,
    ) -> Vec<NativeNodeId> {
        self.stylesheet_candidate_handles_before_in_tree_scope(
            self.document_handle(),
            target_node_id.map(|node_id| NativeNodeId::new(node_id.index())),
        )
    }
}

pub fn stylesheet_link_disposition(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NodeId,
) -> Option<StylesheetLinkDisposition> {
    stylesheet_link_disposition_in_view(document, node_id)
}

pub fn stylesheet_preload_link_request(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NodeId,
) -> Option<StylesheetPreloadLinkRequest> {
    let native_node_id = NativeNodeId::new(node_id.index());
    let element = document.stylesheet_element(native_node_id)?;
    if !element.is_html_element("link")
        || element.disabled
        || !stylesheet_type_is_supported(element.type_attr.as_deref())
    {
        return None;
    }
    let rel = element.rel.as_deref()?;
    if !link_rel_includes_token(rel, "preload")
        || !element
            .as_attr
            .as_deref()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("style"))
    {
        return None;
    }
    let href = element.href.as_deref()?.trim();
    if href.is_empty() {
        return None;
    }
    let url = document.document_base_url_clone()?.join(href).ok()?;
    let options = StylesheetFetchOptions::from_link_attributes(
        element.cross_origin.as_deref(),
        element.referrer_policy.as_deref(),
        element.integrity.as_deref(),
        element.nonce.as_deref(),
        element.charset.as_deref(),
        element.fetch_priority.as_deref(),
    );
    Some(StylesheetPreloadLinkRequest { url, options })
}

fn stylesheet_link_disposition_in_view(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NodeId,
) -> Option<StylesheetLinkDisposition> {
    let native_node_id = NativeNodeId::new(node_id.index());
    let element = document.stylesheet_element(native_node_id)?;
    if !element.is_html_element("link") {
        return None;
    }
    let rel = element.rel.as_deref()?;
    if !link_rel_includes_token(rel, "stylesheet") {
        return None;
    }
    let href = element.href.as_deref()?.trim();
    if href.is_empty()
        || element.disabled
        || !stylesheet_type_is_supported(element.type_attr.as_deref())
    {
        return None;
    }

    let url = document.document_base_url_clone()?.join(href).ok()?;
    let options = StylesheetFetchOptions::from_link_attributes(
        element.cross_origin.as_deref(),
        element.referrer_policy.as_deref(),
        element.integrity.as_deref(),
        element.nonce.as_deref(),
        element.charset.as_deref(),
        element.fetch_priority.as_deref(),
    )
    .with_quirks_mode_mime_compatibility(document.document_is_quirks_mode());
    let is_alternate = link_rel_includes_token(rel, "alternate");
    let blocking = !is_alternate && media_blocks_scripts(element.media.as_deref());
    Some(if blocking {
        StylesheetLinkDisposition::Blocking { url, options }
    } else {
        StylesheetLinkDisposition::LoadOnly { url, options }
    })
}

pub fn link_rel_includes_token(rel: &str, token: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

pub fn connected_preload_like_link_url(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NodeId,
) -> Option<Url> {
    let native_node_id = NativeNodeId::new(node_id.index());
    let element = document.stylesheet_element(native_node_id)?;
    if !element.is_html_element("link") {
        return None;
    }
    let rel = element.rel.as_deref()?;
    if !(link_rel_includes_token(rel, "preload")
        || link_rel_includes_token(rel, "modulepreload")
        || link_rel_includes_token(rel, "prefetch")
        || link_rel_includes_token(rel, "compression-dictionary"))
    {
        return None;
    }
    let href = element.href.as_deref()?.trim();
    if href.is_empty() {
        return None;
    }
    document.document_base_url_clone()?.join(href).ok()
}

pub fn preload_like_link_loads_stylesheet(rel: &str, as_attr: Option<&str>, href: &str) -> bool {
    if link_rel_includes_token(rel, "preload") {
        return as_attr
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("style"));
    }
    if !link_rel_includes_token(rel, "modulepreload") {
        return false;
    }
    match as_attr.map(str::trim) {
        Some(value) if !value.is_empty() => value.eq_ignore_ascii_case("style"),
        _ => href
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.ends_with(".css")),
    }
}

fn stylesheet_type_is_supported(value: Option<&str>) -> bool {
    value.is_none_or(|value| {
        let value = value.trim();
        value.is_empty() || value.eq_ignore_ascii_case("text/css")
    })
}

fn media_blocks_scripts(media: Option<&str>) -> bool {
    let Some(media) = media.map(str::trim) else {
        return true;
    };
    media.is_empty() || media.eq_ignore_ascii_case("all") || media.eq_ignore_ascii_case("screen")
}

pub fn collect_blocking_stylesheet_nodes_before(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    target_node_id: NodeId,
) -> Vec<NodeId> {
    document
        .document_order_stylesheet_candidate_ids_before(Some(target_node_id))
        .into_iter()
        .filter_map(|node_id| {
            let node_id = NodeId::new(node_id.index());
            stylesheet_link_disposition(document, node_id)
                .is_some_and(|disposition| disposition.is_blocking())
                .then_some(node_id)
        })
        .collect()
}

fn collect_document_owned_blocking_stylesheet_candidates_impl(
    document: &(impl StylesheetBlockingReadView + ?Sized),
) -> Vec<DocumentOwnedBlockingStylesheetCandidate> {
    collect_document_owned_blocking_stylesheet_candidates_before_impl(document, None)
}

pub fn collect_document_owned_blocking_stylesheet_candidates(
    document: &(impl StylesheetBlockingReadView + ?Sized),
) -> Vec<DocumentOwnedBlockingStylesheetCandidate> {
    collect_document_owned_blocking_stylesheet_candidates_impl(document)
}

pub fn collect_document_owned_blocking_stylesheets(
    document: &(impl StylesheetBlockingReadView + ?Sized),
) -> Vec<DocumentOwnedBlockingStylesheet> {
    collect_document_owned_blocking_stylesheet_candidates(document)
        .iter()
        .map(DocumentOwnedBlockingStylesheet::from_candidate)
        .collect()
}

pub fn collect_document_owned_blocking_stylesheet_nodes_before(
    document: &NativeDom,
    target_node_id: NodeId,
) -> Vec<NodeId> {
    collect_document_owned_blocking_stylesheet_candidates_before_impl(document, target_node_id)
        .into_iter()
        .map(|candidate| candidate.node_id())
        .collect()
}

fn collect_document_owned_blocking_stylesheet_candidates_before_impl(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    target_node_id: impl Into<Option<NodeId>>,
) -> Vec<DocumentOwnedBlockingStylesheetCandidate> {
    let mut out = Vec::new();
    for node_id in document.document_order_stylesheet_candidate_ids_before(target_node_id.into()) {
        let node_id = NodeId::new(node_id.index());
        if let Some(candidate) =
            document_owned_blocking_stylesheet_candidate_for_node(document, node_id)
        {
            out.push(candidate);
        }
    }
    out
}

pub fn document_owned_blocking_stylesheet_candidate_for_node(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NodeId,
) -> Option<DocumentOwnedBlockingStylesheetCandidate> {
    let native_node_id = NativeNodeId::new(node_id.index());
    let element = document.stylesheet_element(native_node_id)?;
    if element.parser_blocking_eligible
        && let Some(disposition) = stylesheet_link_disposition_in_view(document, node_id)
        && disposition.is_blocking()
    {
        return Some(DocumentOwnedBlockingStylesheetCandidate::Link {
            node_id,
            url: disposition.url().clone(),
            options: disposition.options().clone(),
        });
    }
    parser_created_style_import_urls(document, node_id).map(|urls| {
        DocumentOwnedBlockingStylesheetCandidate::ParserCreatedStyleImport { node_id, urls }
    })
}

pub fn collect_document_owned_blocking_stylesheets_before_in_view(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    target_node_id: NodeId,
) -> Vec<DocumentOwnedBlockingStylesheet> {
    collect_document_owned_blocking_stylesheet_candidates_before_impl(document, target_node_id)
        .iter()
        .map(DocumentOwnedBlockingStylesheet::from_candidate)
        .collect()
}

pub fn collect_document_owned_blocking_stylesheets_before(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    target_node_id: NodeId,
) -> Vec<DocumentOwnedBlockingStylesheet> {
    collect_document_owned_blocking_stylesheets_before_in_view(document, target_node_id)
}

pub fn collect_document_owned_blocking_stylesheet_discovery_inputs_before_in_view(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    target_node_ids: impl IntoIterator<Item = NodeId>,
) -> Vec<DocumentOwnedBlockingStylesheetDiscoveryInput> {
    let mut discovered_inputs = Vec::new();
    let mut discovered_blocker_node_ids = HashSet::new();
    for target_node_id in target_node_ids {
        for blocker in
            collect_document_owned_blocking_stylesheets_before_in_view(document, target_node_id)
        {
            if discovered_blocker_node_ids.insert(blocker.node_id()) {
                discovered_inputs.push(DocumentOwnedBlockingStylesheetDiscoveryInput::from(
                    &blocker,
                ));
            }
        }
    }
    discovered_inputs
}

pub fn document_node_precedes(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    candidate_node_id: NodeId,
    target_node_id: NodeId,
) -> bool {
    let mut found_candidate = false;
    let mut stop = false;
    document_node_precedes_until(
        document,
        document.document_node_id(),
        candidate_node_id,
        target_node_id,
        &mut found_candidate,
        &mut stop,
    );
    found_candidate
}

fn document_node_precedes_until(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NativeNodeId,
    candidate_node_id: NodeId,
    target_node_id: NodeId,
    found_candidate: &mut bool,
    stop: &mut bool,
) {
    if *stop {
        return;
    }
    let current_node_id = NodeId::new(node_id.index());
    if current_node_id == target_node_id {
        *stop = true;
        return;
    }
    if current_node_id == candidate_node_id {
        *found_candidate = true;
    }

    for child in document.child_ids(node_id) {
        document_node_precedes_until(
            document,
            child,
            candidate_node_id,
            target_node_id,
            found_candidate,
            stop,
        );
        if *stop {
            break;
        }
    }
}

fn parser_created_style_import_urls(
    document: &(impl StylesheetBlockingReadView + ?Sized),
    node_id: NodeId,
) -> Option<Vec<Url>> {
    let native_node_id = NativeNodeId::new(node_id.index());
    let element = document.stylesheet_element(native_node_id)?;
    if !element.is_html_element("style") || !element.parser_blocking_eligible {
        return None;
    }
    if !media_blocks_scripts(element.media.as_deref()) {
        return None;
    }
    let css_text = document.text_content(native_node_id)?;
    let base_url = document.document_base_url_clone()?;
    let urls = extract_css_import_urls(&base_url, &css_text);
    (!urls.is_empty()).then_some(urls)
}

fn extract_css_import_urls(base_url: &Url, css_text: &str) -> Vec<Url> {
    let mut out = Vec::new();
    for statement in css_text.split(';') {
        let statement = statement.trim();
        if !statement
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("@import"))
        {
            continue;
        }
        let remainder = statement[7..].trim_start();
        let Some(specifier) = extract_css_import_specifier(remainder) else {
            continue;
        };
        if let Ok(url) = base_url.join(&specifier) {
            out.push(url);
        }
    }
    out
}

fn extract_css_import_specifier(input: &str) -> Option<String> {
    let input = input.trim_start();
    if input.starts_with("url(") || input.starts_with("URL(") {
        let rest = &input[4..];
        let end = rest.find(')')?;
        let inner = rest[..end].trim();
        return trim_css_string(inner);
    }
    if let Some(quote) = input.chars().next().filter(|ch| matches!(ch, '"' | '\'')) {
        let remainder = &input[quote.len_utf8()..];
        let end = remainder.find(quote)?;
        let specifier = &remainder[..end];
        return Some(specifier.trim().to_owned());
    }
    trim_css_string(input)
}

fn trim_css_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.len() < 2 {
        return None;
    }
    let first = trimmed.as_bytes()[0] as char;
    let last = trimmed.as_bytes()[trimmed.len() - 1] as char;
    if !matches!(first, '"' | '\'') || first != last {
        return None;
    }
    Some(trimmed[1..trimmed.len() - 1].trim().to_owned())
}

#[cfg(test)]
mod tests {
    use moli_dom::native::{DomHost, NativeDom};

    use super::{
        document_owned_blocking_stylesheet_candidate_for_node, extract_css_import_urls,
        stylesheet_link_disposition, stylesheet_preload_link_request,
    };

    #[test]
    fn parser_created_style_imports_join_against_document_url() {
        let base_url = url::Url::parse("https://example.com/path/page.html").unwrap();
        let urls = extract_css_import_urls(
            &base_url,
            "@import url('/a.css'); @import \"b.css\" screen; body { color: red; }",
        );

        assert_eq!(
            urls,
            vec![
                url::Url::parse("https://example.com/a.css").unwrap(),
                url::Url::parse("https://example.com/path/b.css").unwrap(),
            ]
        );
    }

    #[test]
    fn style_disabled_content_attribute_does_not_suppress_parser_import_blocking() {
        let document_url = url::Url::parse("https://example.com/path/page.html").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(document_url));
        let style = host.create_parser_element_without_attributes(
            "style".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        let text = host.create_text_node("@import url('theme.css');");
        assert!(host.set_attribute(style, "disabled", ""));
        assert!(host.append_child(style, text));
        assert!(host.append_child(host.document_handle(), style));

        let candidate = document_owned_blocking_stylesheet_candidate_for_node(
            &host,
            moli_dom::NodeId::new(style.index()),
        )
        .expect("the unsupported style content attribute must not disable import blocking");
        let super::DocumentOwnedBlockingStylesheetCandidate::ParserCreatedStyleImport {
            urls, ..
        } = candidate
        else {
            panic!("expected a parser-created style import candidate");
        };
        assert_eq!(
            urls,
            [url::Url::parse("https://example.com/path/theme.css").unwrap()]
        );
    }

    #[test]
    fn hidden_nonce_remains_part_of_stylesheet_request_identity() {
        let document_url = url::Url::parse("https://example.com/page.html").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(document_url));
        let link = host.create_element("link");
        assert!(host.set_attribute(link, "rel", "stylesheet"));
        assert!(host.set_attribute(link, "href", "/style.css"));
        assert!(host.set_attribute(link, "nonce", ""));
        assert!(host.set_cryptographic_nonce(link, Some("secret".to_owned())));
        assert!(host.append_child(host.document_handle(), link));

        let disposition = stylesheet_link_disposition(&host, moli_dom::NodeId::new(link.index()))
            .expect("the connected link should remain a stylesheet candidate");

        assert_eq!(disposition.options().nonce(), Some("secret"));
    }

    #[test]
    fn dynamic_stylesheet_link_loads_without_becoming_a_parser_script_blocker() {
        let document_url = url::Url::parse("https://example.com/page.html").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(document_url));
        let link = host.create_element("link");
        assert!(host.set_attribute(link, "rel", "stylesheet"));
        assert!(host.set_attribute(link, "href", "/style.css"));
        assert!(host.append_child(host.document_handle(), link));
        let node_id = moli_dom::NodeId::new(link.index());

        assert!(
            stylesheet_link_disposition(&host, node_id)
                .is_some_and(|disposition| disposition.is_blocking()),
            "a dynamic stylesheet still owns its normal load operation"
        );
        assert!(
            document_owned_blocking_stylesheet_candidate_for_node(&host, node_id).is_none(),
            "Chromium classifies a non-parser-created sheet as non-script-blocking"
        );
    }

    #[test]
    fn style_preload_request_uses_stylesheet_identity_without_becoming_a_stylesheet_link() {
        let document_url = url::Url::parse("https://example.com/path/page.html").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(document_url));
        let link = host.create_element("link");
        assert!(host.set_attribute(link, "rel", "dns-prefetch PRELOAD"));
        assert!(host.set_attribute(link, "as", "STYLE"));
        assert!(host.set_attribute(link, "href", "theme.css#fragment"));
        assert!(host.set_attribute(link, "crossorigin", "anonymous"));
        assert!(host.set_attribute(link, "integrity", "sha256-test"));
        assert!(host.append_child(host.document_handle(), link));

        let node_id = moli_dom::NodeId::new(link.index());
        let request =
            stylesheet_preload_link_request(&host, node_id).expect("style preload request");

        assert_eq!(
            request.url().as_str(),
            "https://example.com/path/theme.css#fragment"
        );
        assert_eq!(request.options().cross_origin(), Some("anonymous"));
        assert_eq!(request.options().integrity(), Some("sha256-test"));
        assert!(
            stylesheet_link_disposition(&host, node_id).is_none(),
            "a preload client must not become a stylesheet install client"
        );
    }

    #[test]
    fn style_preload_request_rejects_disabled_or_unsupported_type_links() {
        let document_url = url::Url::parse("https://example.com/page.html").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(document_url));
        let link = host.create_element("link");
        assert!(host.set_attribute(link, "rel", "preload"));
        assert!(host.set_attribute(link, "as", "style"));
        assert!(host.set_attribute(link, "href", "/theme.css"));
        assert!(host.set_attribute(link, "disabled", ""));
        assert!(host.append_child(host.document_handle(), link));
        let node_id = moli_dom::NodeId::new(link.index());
        assert!(stylesheet_preload_link_request(&host, node_id).is_none());

        assert!(host.remove_attribute(link, "disabled"));
        assert!(host.set_attribute(link, "type", "text/plain"));
        assert!(stylesheet_preload_link_request(&host, node_id).is_none());
    }
}
