use html5ever::tree_builder::QuirksMode as HtmlQuirksMode;
use selectors::matching::QuirksMode;
use url::Url;

use super::{DomStringValue, NativeDom, NativeNodeId, SvgUserTransform};

mod base_url;
mod referrer_policy;

#[cfg(test)]
mod tests;

pub use base_url::DocumentBaseUrlPolicyCheck;
use base_url::DocumentBaseUrlState;
pub use referrer_policy::DocumentMetaReferrerPolicySource;
use referrer_policy::MetaReferrerPolicy;

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

#[derive(Debug, Clone, Copy)]
enum DocumentInterface {
    Document,
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
    metadata: DocumentMetadata,
    arena_state: DocumentArenaState,
}

// Portable document data contains no references into a NativeDom arena.
#[derive(Debug, Clone)]
struct DocumentMetadata {
    url: Url,
    content_type: Box<str>,
    character_set: Box<str>,
    allow_declarative_shadow_roots: bool,
    ready_state: DocumentReadyState,
    // Retained with the Document when its Window is replaced during navigation.
    visibility_hidden: bool,
    quirks_mode: QuirksMode,
    kind: DocumentKind,
    interface: DocumentInterface,
    design_mode_enabled: bool,
    // Portable parser policy; inert/template documents keep scripting disabled.
    scripting_enabled: bool,
    default_language: Option<Box<str>>,
    source_last_modified_ms: Option<f64>,
    base_url_state: DocumentBaseUrlState,
}

// Node relationships and the associated template-owner role belong to one
// Document identity. Only a whole-arena snapshot may copy them unchanged.
#[derive(Debug, Clone, Default)]
struct DocumentArenaState {
    // Meta delivery updates policy in time order; removing the element does
    // not undo the policy it delivered to this Document.
    meta_referrer_policy: MetaReferrerPolicy,
    // Sticky for this Document; aborting a parser disables document.open/write.
    active_parser_was_aborted: bool,
    is_inert_template_document: bool,
    associated_inert_template_document: Option<NativeNodeId>,
    css_target: Option<NativeNodeId>,
    // SVG document-root replacement retains magnification and panning.
    svg_user_transform: Option<Box<SvgUserTransform>>,
}

impl Document {
    pub fn new(url: Url) -> Self {
        Self::new_html(url)
    }

    pub fn new_html(url: Url) -> Self {
        Self::new_html_with_scripting(url, true)
    }

    pub fn new_html_with_scripting(url: Url, scripting_enabled: bool) -> Self {
        Self::new_with_kind(url, DocumentKind::Html, scripting_enabled)
    }

    pub fn new_xml(url: Url) -> Self {
        Self::new_with_kind(url, DocumentKind::Xml, true)
    }

    fn new_with_kind(url: Url, kind: DocumentKind, scripting_enabled: bool) -> Self {
        let (content_type, interface) = match kind {
            DocumentKind::Html => ("text/html", DocumentInterface::Html),
            DocumentKind::Xml => ("application/xml", DocumentInterface::Xml),
        };
        Self {
            metadata: DocumentMetadata {
                base_url_state: DocumentBaseUrlState::new(&url),
                url,
                content_type: content_type.into(),
                character_set: "UTF-8".into(),
                allow_declarative_shadow_roots: false,
                ready_state: DocumentReadyState::Complete,
                visibility_hidden: false,
                quirks_mode: QuirksMode::NoQuirks,
                kind,
                interface,
                design_mode_enabled: false,
                scripting_enabled,
                default_language: None,
                source_last_modified_ms: None,
            },
            arena_state: DocumentArenaState::default(),
        }
    }

    pub(super) fn new_inert_template(kind: DocumentKind) -> Self {
        let url = Url::parse("about:blank").expect("about:blank is valid");
        let mut document = Self::new_with_kind(url, kind, false);
        document.metadata.interface = DocumentInterface::Document;
        document.arena_state.is_inert_template_document = true;
        document
    }

    /// Copy an independent Document's portable data without tree-derived caches,
    /// even across DOM arenas.
    /// `Clone` instead preserves relationships for whole-arena snapshots and
    /// copy-on-write node storage; it must not create a new Document identity.
    pub(super) fn clone_metadata_for_new_arena(&self) -> Self {
        Self {
            metadata: DocumentMetadata {
                base_url_state: self
                    .metadata
                    .base_url_state
                    .clone_without_tree_caches(self.url()),
                ..self.metadata.clone()
            },
            arena_state: DocumentArenaState::default(),
        }
    }

    pub(super) fn clone_for_node(&self) -> Self {
        let mut document = self.clone_metadata_for_new_arena();
        document.set_design_mode_enabled(false);
        // DOM additionally freezes the computed base URL; HTML separately
        // retains the about base URL used by a future relative <base href>.
        // https://dom.spec.whatwg.org/#concept-node-clone
        // https://html.spec.whatwg.org/multipage/dom.html#the-document-object
        document.set_base_url_override(Some(self.base_url().clone()));
        document
    }

    pub fn url(&self) -> &Url {
        &self.metadata.url
    }

    pub fn content_type(&self) -> &str {
        &self.metadata.content_type
    }

    pub fn character_set(&self) -> &str {
        &self.metadata.character_set
    }

    pub fn allow_declarative_shadow_roots(&self) -> bool {
        self.metadata.allow_declarative_shadow_roots
    }

    pub fn set_allow_declarative_shadow_roots(&mut self, allow: bool) {
        self.metadata.allow_declarative_shadow_roots = allow;
    }

    pub fn set_character_set(&mut self, character_set: impl Into<String>) {
        self.metadata.character_set = character_set.into().into_boxed_str();
    }

    pub fn css_target(&self) -> Option<NativeNodeId> {
        self.arena_state.css_target
    }

    pub fn ready_state(&self) -> DocumentReadyState {
        self.metadata.ready_state
    }

    pub fn active_parser_was_aborted(&self) -> bool {
        self.arena_state.active_parser_was_aborted
    }

    pub fn mark_active_parser_aborted(&mut self) {
        self.arena_state.active_parser_was_aborted = true;
    }

    pub(super) fn svg_user_transform(&self) -> SvgUserTransform {
        self.arena_state.svg_user_transform
            .as_deref()
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn set_svg_user_transform(&mut self, value: SvgUserTransform) {
        // Only x/y participate in the document's panning transform. The
        // DOMPoint's z/w belong to its element, including during replacement.
        let value = SvgUserTransform {
            point: [value.point[0], value.point[1], 0.0, 1.0],
            ..value
        };
        if value.is_default() {
            self.arena_state.svg_user_transform = None;
        } else if let Some(current) = self.arena_state.svg_user_transform.as_deref_mut() {
            *current = value;
        } else {
            self.arena_state.svg_user_transform = Some(Box::new(value));
        }
    }

    pub fn meta_referrer_policy(&self) -> Option<&str> {
        self.arena_state.meta_referrer_policy.get()
    }

    pub fn meta_referrer_policy_source(&self) -> DocumentMetaReferrerPolicySource {
        self.arena_state.meta_referrer_policy.source()
    }

    pub fn visibility_hidden(&self) -> bool {
        self.metadata.visibility_hidden
    }

    pub fn default_language(&self) -> Option<&str> {
        self.metadata.default_language.as_deref()
    }

    pub fn source_last_modified_ms(&self) -> Option<f64> {
        self.metadata.source_last_modified_ms
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.metadata.quirks_mode
    }

    pub fn is_quirks_mode(&self) -> bool {
        self.metadata.quirks_mode == QuirksMode::Quirks
    }

    pub fn kind(&self) -> DocumentKind {
        self.metadata.kind
    }

    pub fn is_html_document(&self) -> bool {
        self.metadata.kind == DocumentKind::Html
    }

    pub fn interface_name(&self) -> &'static str {
        match self.metadata.interface {
            DocumentInterface::Document => "Document",
            DocumentInterface::Html => "HTMLDocument",
            DocumentInterface::Xml => "XMLDocument",
        }
    }

    pub fn is_inert_template_document(&self) -> bool {
        self.arena_state.is_inert_template_document
    }

    pub(super) fn associated_inert_template_document(&self) -> Option<NativeNodeId> {
        self.arena_state.associated_inert_template_document
    }

    pub(super) fn set_associated_inert_template_document(&mut self, document: NativeNodeId) {
        self.arena_state.associated_inert_template_document = Some(document);
    }

    pub fn design_mode_enabled(&self) -> bool {
        self.metadata.design_mode_enabled
    }

    pub fn scripting_enabled(&self) -> bool {
        self.metadata.scripting_enabled
    }

    pub fn fallback_base_url(&self) -> &Url {
        self.metadata.base_url_state.fallback_base_url()
    }

    pub fn base_element_url(&self) -> Option<&Url> {
        self.metadata.base_url_state.base_element_url()
    }

    pub fn base_url(&self) -> &Url {
        self.metadata.base_url_state.base_url()
    }

    pub fn base_target(&self) -> Option<&str> {
        self.metadata.base_url_state.base_target()
    }

    pub fn set_url(&mut self, url: Url) {
        self.metadata.base_url_state.set_document_url(&url);
        self.metadata.url = url;
    }

    pub fn set_base_url_override(&mut self, url: Option<Url>) {
        self.metadata.base_url_state.set_base_url_override(url);
    }

    pub fn set_fallback_base_url(&mut self, fallback_base_url: Option<Url>) -> bool {
        self.metadata
            .base_url_state
            .set_fallback_base_url(&self.metadata.url, fallback_base_url)
    }

    pub fn set_content_type(&mut self, content_type: impl Into<String>) {
        self.metadata.content_type = content_type.into().into_boxed_str();
    }

    pub fn set_design_mode_enabled(&mut self, design_mode_enabled: bool) {
        self.metadata.design_mode_enabled = design_mode_enabled;
    }

    pub fn set_scripting_enabled(&mut self, scripting_enabled: bool) {
        self.metadata.scripting_enabled = scripting_enabled;
    }

    pub fn set_css_target(&mut self, target: Option<NativeNodeId>) -> bool {
        if self.arena_state.css_target == target {
            return false;
        }
        self.arena_state.css_target = target;
        true
    }

    pub fn set_ready_state(&mut self, ready_state: DocumentReadyState) {
        self.metadata.ready_state = ready_state;
    }

    pub fn set_visibility_hidden(&mut self, hidden: bool) {
        self.metadata.visibility_hidden = hidden;
    }

    pub fn set_default_language(&mut self, language: Option<String>) {
        self.metadata.default_language = language.map(String::into_boxed_str);
    }

    pub fn set_source_last_modified_ms(&mut self, timestamp_ms: Option<f64>) {
        self.metadata.source_last_modified_ms = timestamp_ms;
    }

    pub fn set_quirks_mode(&mut self, quirks_mode: QuirksMode) {
        self.metadata.quirks_mode = quirks_mode;
    }

    pub fn set_html_quirks_mode(&mut self, quirks_mode: HtmlQuirksMode) {
        self.metadata.quirks_mode = match quirks_mode {
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
    // Doctypes are rare; their lossless strings must not enlarge common node records.
    strings: Box<DocumentTypeStrings>,
}

#[derive(Debug, Clone)]
struct DocumentTypeStrings {
    name: DomStringValue,
    public_id: DomStringValue,
    system_id: DomStringValue,
}

impl DocumentType {
    pub fn new(
        name: impl Into<DomStringValue>,
        public_id: impl Into<DomStringValue>,
        system_id: impl Into<DomStringValue>,
    ) -> Self {
        Self {
            strings: Box::new(DocumentTypeStrings {
                name: name.into(),
                public_id: public_id.into(),
                system_id: system_id.into(),
            }),
        }
    }

    pub fn name(&self) -> &str {
        self.strings.name.as_str_lossy()
    }

    pub fn public_id(&self) -> &str {
        self.strings.public_id.as_str_lossy()
    }

    pub fn system_id(&self) -> &str {
        self.strings.system_id.as_str_lossy()
    }

    pub fn name_value(&self) -> &DomStringValue {
        &self.strings.name
    }

    pub fn public_id_value(&self) -> &DomStringValue {
        &self.strings.public_id
    }

    pub fn system_id_value(&self) -> &DomStringValue {
        &self.strings.system_id
    }
}

#[derive(Debug, Clone, Default)]
pub struct DocumentFragment {
    host: Option<NativeNodeId>,
}

impl DocumentFragment {
    pub fn new(host: Option<NativeNodeId>) -> Self {
        Self { host }
    }

    pub fn host(&self) -> Option<NativeNodeId> {
        self.host
    }

    pub(crate) fn set_host(&mut self, host: NativeNodeId) {
        self.host = Some(host);
    }
}
