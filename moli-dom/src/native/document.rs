use html5ever::tree_builder::QuirksMode as HtmlQuirksMode;
use selectors::matching::QuirksMode;
use url::Url;

use super::{NativeDom, NativeNodeId};

mod base_url;

use base_url::DocumentBaseUrlState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTitleSetterTarget {
    ExistingTitle(NativeNodeId),
    AppendToHtmlHead(NativeNodeId),
    PrependToSvgRoot(NativeNodeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    Html,
    Xml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentReadyState {
    Loading,
    Interactive,
    Complete,
}

impl DocumentReadyState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Interactive => "interactive",
            Self::Complete => "complete",
        }
    }
}

impl std::fmt::Display for DocumentReadyState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    url: Url,
    content_type: Box<str>,
    ready_state: DocumentReadyState,
    quirks_mode: QuirksMode,
    kind: DocumentKind,
    css_target: Option<NativeNodeId>,
    default_language: Option<Box<str>>,
    source_last_modified_ms: Option<f64>,
    base_url_state: DocumentBaseUrlState,
}

impl Document {
    pub fn new(url: Url) -> Self {
        Self::new_html(url)
    }

    pub fn new_html(url: Url) -> Self {
        Self {
            base_url_state: DocumentBaseUrlState::new(&url),
            url,
            content_type: "text/html".into(),
            ready_state: DocumentReadyState::Complete,
            quirks_mode: QuirksMode::NoQuirks,
            kind: DocumentKind::Html,
            css_target: None,
            default_language: None,
            source_last_modified_ms: None,
        }
    }

    pub fn new_xml(url: Url) -> Self {
        Self {
            base_url_state: DocumentBaseUrlState::new(&url),
            url,
            content_type: "application/xml".into(),
            ready_state: DocumentReadyState::Complete,
            quirks_mode: QuirksMode::NoQuirks,
            kind: DocumentKind::Xml,
            css_target: None,
            default_language: None,
            source_last_modified_ms: None,
        }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn css_target(&self) -> Option<NativeNodeId> {
        self.css_target
    }

    pub fn ready_state(&self) -> DocumentReadyState {
        self.ready_state
    }

    pub fn default_language(&self) -> Option<&str> {
        self.default_language.as_deref()
    }

    pub fn source_last_modified_ms(&self) -> Option<f64> {
        self.source_last_modified_ms
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    pub fn kind(&self) -> DocumentKind {
        self.kind
    }

    pub fn is_html_document(&self) -> bool {
        self.kind == DocumentKind::Html
    }

    pub fn fallback_base_url(&self) -> &Url {
        self.base_url_state.fallback_base_url()
    }

    pub fn base_element_url(&self) -> Option<&Url> {
        self.base_url_state.base_element_url()
    }

    pub fn base_url(&self) -> &Url {
        self.base_url_state.base_url()
    }

    pub fn base_target(&self) -> Option<&str> {
        self.base_url_state.base_target()
    }

    pub fn set_url(&mut self, url: Url) {
        self.base_url_state.set_document_url(&url);
        self.url = url;
    }

    pub fn set_url_preserving_fallback_base(&mut self, url: Url) {
        self.url = url;
    }

    pub fn set_base_url_override(&mut self, url: Option<Url>) {
        self.base_url_state.set_base_url_override(url);
    }

    pub fn set_fallback_base_url(&mut self, fallback_base_url: Url) {
        self.base_url_state.set_fallback_base_url(fallback_base_url);
    }

    pub fn set_content_type(&mut self, content_type: impl Into<String>) {
        self.content_type = content_type.into().into_boxed_str();
    }

    pub fn set_css_target(&mut self, target: Option<NativeNodeId>) -> bool {
        if self.css_target == target {
            return false;
        }
        self.css_target = target;
        true
    }

    pub fn set_ready_state(&mut self, ready_state: DocumentReadyState) {
        self.ready_state = ready_state;
    }

    pub fn set_default_language(&mut self, language: Option<String>) {
        self.default_language = language.map(String::into_boxed_str);
    }

    pub fn set_source_last_modified_ms(&mut self, timestamp_ms: Option<f64>) {
        self.source_last_modified_ms = timestamp_ms;
    }

    pub fn set_quirks_mode(&mut self, quirks_mode: QuirksMode) {
        self.quirks_mode = quirks_mode;
    }

    pub fn set_html_quirks_mode(&mut self, quirks_mode: HtmlQuirksMode) {
        self.quirks_mode = match quirks_mode {
            HtmlQuirksMode::NoQuirks => QuirksMode::NoQuirks,
            HtmlQuirksMode::Quirks => QuirksMode::Quirks,
            HtmlQuirksMode::LimitedQuirks => QuirksMode::LimitedQuirks,
        };
    }

    pub fn document_element_handle(
        &self,
        dom: &NativeDom,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        dom.find_child(document_node_id, |handle| {
            dom.node(handle).is_some_and(|node| node.is_element())
        })
    }

    fn html_document_element_handle(
        &self,
        dom: &NativeDom,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        self.document_element_handle(dom, document_node_id)
            .filter(|handle| {
                dom.node(*handle)
                    .is_some_and(|node| node.is_html_element_named("html"))
            })
    }

    pub fn head_handle(
        &self,
        dom: &NativeDom,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        let document_element = self.html_document_element_handle(dom, document_node_id)?;
        dom.find_child(document_element, |handle| {
            dom.node(handle)
                .is_some_and(|node| node.is_html_element_named("head"))
        })
    }

    pub fn body_handle(
        &self,
        dom: &NativeDom,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        let document_element = self.html_document_element_handle(dom, document_node_id)?;
        dom.find_child(document_element, |handle| {
            dom.node(handle)
                .is_some_and(|node| node.is_html_element_named("body"))
        })
    }

    pub fn body_or_frameset_handle(
        &self,
        dom: &NativeDom,
        document_node_id: NativeNodeId,
    ) -> Option<NativeNodeId> {
        let document_element = self.html_document_element_handle(dom, document_node_id)?;
        dom.find_child(document_element, |handle| {
            dom.node(handle).is_some_and(|node| {
                node.is_html_element_named("body") || node.is_html_element_named("frameset")
            })
        })
    }
}

#[derive(Debug, Clone)]
pub struct DocumentType {
    name: Box<str>,
    public_id: Box<str>,
    system_id: Box<str>,
}

impl DocumentType {
    pub fn new(name: String, public_id: String, system_id: String) -> Self {
        Self {
            name: name.into_boxed_str(),
            public_id: public_id.into_boxed_str(),
            system_id: system_id.into_boxed_str(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn public_id(&self) -> &str {
        &self.public_id
    }

    pub fn system_id(&self) -> &str {
        &self.system_id
    }
}

#[derive(Debug, Clone, Default)]
pub struct DocumentFragment;
