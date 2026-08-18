use url::Url;

use crate::ParserStreamDocumentSnapshot;
use moli_dom::{
    NodeId,
    native::{DomHost, NativeDom, NativeNodeId, Node},
};
use moli_fetch::FetchPriorityHint;
use moli_page_types::{ScriptKind, ScriptMode, ScriptSourceKind};
use moli_script::{
    ScriptElementClassificationInput, ScriptPreparationClassificationInput,
    ScriptPreparationDisposition, classify_script_preparation,
};

pub struct ParserScriptRead {
    pub parser_inserted: bool,
    pub parser_inserted_for_prepare: bool,
    pub already_started: bool,
    pub has_nomodule: bool,
    pub script_type: Option<String>,
    pub script_language: Option<String>,
    pub script_event: Option<String>,
    pub script_for: Option<String>,
    pub script_src: Option<String>,
    pub script_text: Option<String>,
    pub script_async: bool,
    pub async_attribute_present: bool,
    pub defer_attribute_present: bool,
    pub fetch_metadata: ScriptFetchMetadata,
}

fn parser_script_read_from_node(
    node: &Node,
    script_src: Option<String>,
    script_text: Option<String>,
) -> Option<ParserScriptRead> {
    let element = node.as_element()?;
    if !element.is_script_element() {
        return None;
    }

    // `type` and `async` belong to both HTMLScriptElement and
    // SVGScriptElement. The remaining classification/scheduling attributes
    // below are HTML-only; accepting them on SVG scripts would, for example,
    // make a harmless `nomodule` suppress execution or make `defer` change the
    // parser lane.
    let is_html_script = element.is_html_script();
    Some(ParserScriptRead {
        parser_inserted: node.flags().parser_created(),
        parser_inserted_for_prepare: element.script_parser_inserted_for_prepare(),
        already_started: element.script_already_started(),
        has_nomodule: is_html_script && element.attribute("nomodule").is_some(),
        script_type: element.attribute("type").map(str::to_owned),
        script_language: if is_html_script {
            element.attribute("language").map(str::to_owned)
        } else {
            None
        },
        script_event: if is_html_script {
            element.attribute("event").map(str::to_owned)
        } else {
            None
        },
        script_for: if is_html_script {
            element.attribute("for").map(str::to_owned)
        } else {
            None
        },
        script_src,
        script_text,
        script_async: element.script_async(),
        async_attribute_present: element.attribute("async").is_some(),
        defer_attribute_present: is_html_script && element.attribute("defer").is_some(),
        fetch_metadata: ScriptFetchMetadata::from_script_attributes(
            element.attribute("crossorigin"),
            element.attribute("referrerpolicy"),
            element.attribute("charset"),
            element.attribute("integrity"),
            element
                .cryptographic_nonce()
                .or_else(|| element.attribute("nonce")),
            element.attribute("fetchpriority"),
        )
        .with_parser_inserted(node.flags().parser_created()),
    })
}

pub trait ParserPlanningReadView {
    fn parser_script_read(&self, node_id: NativeNodeId) -> Option<ParserScriptRead>;
    fn is_connected(&self, node_id: NativeNodeId) -> bool;
    fn script_handles(&self) -> Vec<NativeNodeId>;
    fn document_order_script_handles(&self) -> Vec<NativeNodeId>;
    fn document_order_position(&self, node_id: NativeNodeId) -> Option<usize> {
        self.document_order_script_handles()
            .into_iter()
            .position(|handle| handle == node_id)
    }
    fn final_url_clone(&self) -> Option<Url>;
    fn document_base_url_clone(&self) -> Option<Url>;
    fn script_start_line(&self, _node_id: NativeNodeId) -> Option<u64> {
        None
    }
}

fn collect_script_handles_in_dom_host(
    handles: &mut Vec<NativeNodeId>,
    host: &DomHost,
    current: NativeNodeId,
) {
    let Some(node) = host.node(current) else {
        return;
    };
    if node.is_script_element() {
        handles.push(current);
    }
    if let Some(template_contents) = node
        .as_element()
        .and_then(|element| element.template_contents())
    {
        collect_script_handles_in_dom_host(handles, host, template_contents);
    }
    for child in host.child_handles(current) {
        collect_script_handles_in_dom_host(handles, host, child);
    }
}

impl ParserPlanningReadView for NativeDom {
    fn parser_script_read(&self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        let node = self.node(node_id)?;
        parser_script_read_from_node(
            node,
            self.script_src(node_id).map(str::to_owned),
            self.script_text(node_id),
        )
    }

    fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        NativeDom::document_order_script_handles(self)
    }

    fn is_connected(&self, node_id: NativeNodeId) -> bool {
        self.node(node_id).is_some_and(Node::is_connected)
    }

    fn script_handles(&self) -> Vec<NativeNodeId> {
        self.script_handles()
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.final_url().cloned()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        self.document().map(|document| document.base_url().clone())
    }
}

impl ParserPlanningReadView for DomHost {
    fn parser_script_read(&self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        let node = self.node(node_id)?;
        if !node.is_script_element() {
            return None;
        }

        let script_text = self
            .dom()
            .direct_text_content(node_id)
            .and_then(|text| (!text.is_empty()).then_some(text));

        parser_script_read_from_node(
            node,
            node.as_element()
                .and_then(|element| element.script_source_attribute())
                .map(str::to_owned),
            script_text,
        )
    }

    fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        self.script_handles_in_light_subtree(self.document_handle())
    }

    fn is_connected(&self, node_id: NativeNodeId) -> bool {
        DomHost::is_connected(self, node_id)
    }

    fn script_handles(&self) -> Vec<NativeNodeId> {
        let mut handles = Vec::new();
        collect_script_handles_in_dom_host(&mut handles, self, self.document_handle());
        handles
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.node(self.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        self.document_base_url()
    }
}

impl ParserPlanningReadView for ParserStreamDocumentSnapshot {
    fn parser_script_read(&self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        let node = self.node(node_id)?;
        parser_script_read_from_node(
            node,
            self.script_src(node_id).map(str::to_owned),
            self.script_text(node_id),
        )
    }

    fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        ParserStreamDocumentSnapshot::document_order_script_handles(self)
    }

    fn is_connected(&self, node_id: NativeNodeId) -> bool {
        self.node(node_id).is_some_and(Node::is_connected)
    }

    fn script_handles(&self) -> Vec<NativeNodeId> {
        self.script_handles()
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.final_url().cloned()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        self.document_base_url()
    }
}

#[derive(Debug, Clone)]
pub struct PreparedScript {
    pub position: usize,
    pub node_id: NodeId,
    pub kind: ScriptKind,
    pub mode: ScriptMode,
    pub source_kind: ScriptSourceKind,
    pub fetch_metadata: ScriptFetchMetadata,
    pub source: ScriptSource,
    pub url: Url,
    pub base_url: Url,
    pub initiator_url: Url,
    pub host_script_handle: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedImportMap {
    pub position: usize,
    pub node_id: NodeId,
    pub source: PreparedImportMapSource,
    pub base_url: Url,
    pub initiator_url: Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedImportMapSource {
    Inline(String),
    ExternalUnsupported,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ScriptFetchMetadata {
    pub cross_origin: Option<String>,
    pub referrer_policy: Option<String>,
    pub charset: Option<String>,
    pub integrity: Option<String>,
    pub nonce: Option<String>,
    pub fetch_priority: Option<FetchPriorityHint>,
    /// HTML's script fetch options capture parser metadata at prepare time.
    /// Module descendants and dynamic imports inherit this exact value; it is
    /// not equivalent to whether the eventual fetch runs on a parser-owned
    /// scheduler lane.
    pub parser_inserted: bool,
}

impl ScriptFetchMetadata {
    pub fn from_script_attributes(
        cross_origin: Option<&str>,
        referrer_policy: Option<&str>,
        charset: Option<&str>,
        integrity: Option<&str>,
        nonce: Option<&str>,
        fetch_priority: Option<&str>,
    ) -> Self {
        Self {
            cross_origin: normalize_cross_origin(cross_origin),
            referrer_policy: normalize_referrer_policy(referrer_policy),
            charset: normalize_attr_token(charset),
            integrity: normalize_non_empty_attr(integrity),
            nonce: normalize_non_empty_attr(nonce),
            fetch_priority: FetchPriorityHint::from_attribute(fetch_priority),
            parser_inserted: false,
        }
    }

    pub fn with_parser_inserted(mut self, parser_inserted: bool) -> Self {
        self.parser_inserted = parser_inserted;
        self
    }
}

fn normalize_cross_origin(value: Option<&str>) -> Option<String> {
    let value = value?;
    let normalized = value.trim().to_ascii_lowercase();
    if normalized == "use-credentials" {
        Some("use-credentials".to_owned())
    } else {
        Some("anonymous".to_owned())
    }
}

fn normalize_attr_token(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_referrer_policy(value: Option<&str>) -> Option<String> {
    let normalized = normalize_attr_token(value)?;
    match normalized.as_str() {
        "no-referrer"
        | "no-referrer-when-downgrade"
        | "origin"
        | "origin-when-cross-origin"
        | "same-origin"
        | "strict-origin"
        | "strict-origin-when-cross-origin"
        | "unsafe-url" => Some(normalized),
        _ => None,
    }
}

fn normalize_non_empty_attr(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

impl PreparedScript {
    pub fn with_loaded_source(mut self, source: String) -> Self {
        self.source = ScriptSource::Loaded(source);
        self
    }

    pub fn with_loaded_binary_source(mut self, source: String, bytes: Vec<u8>) -> Self {
        self.source = ScriptSource::LoadedBinary { source, bytes };
        self
    }

    pub fn waits_for_blocking_stylesheets(&self) -> bool {
        match self.kind {
            ScriptKind::Classic => {
                self.source_kind == ScriptSourceKind::External
                    && matches!(
                        self.mode,
                        ScriptMode::Normal
                            | ScriptMode::Defer
                            | ScriptMode::InOrder
                            | ScriptMode::ImportMapInOrder
                    )
            }
            ScriptKind::Module => matches!(
                self.mode,
                ScriptMode::ModuleDefer | ScriptMode::ModuleInOrder
            ),
            ScriptKind::ImportMap | ScriptKind::DataBlock => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScriptSource {
    Inline(String),
    Loaded(String),
    LoadedBinary { source: String, bytes: Vec<u8> },
    External,
}

/// Classification result from reading and classifying a parser-visible script.
pub struct ScriptClassification {
    pub script: ParserScriptRead,
    pub disposition: ScriptPreparationDisposition,
    pub source_kind: ScriptSourceKind,
    pub legacy_event_for_mismatch: bool,
}

/// Reason a classified script should be skipped rather than prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptFilterSkipReason {
    AlreadyStarted,
    NoModule,
    DataBlock,
    LegacyEventForMismatch,
}

impl ScriptClassification {
    pub fn kind(&self) -> ScriptKind {
        self.disposition.kind()
    }

    pub fn executable(&self) -> Option<(ScriptKind, ScriptMode)> {
        self.disposition.executable()
    }

    pub fn mode(&self) -> Option<ScriptMode> {
        self.executable().map(|(_, mode)| mode)
    }

    /// Returns `None` if this script should be prepared; returns a skip reason otherwise.
    /// Centralizes the shared filter: already_started → classic nomodule → DataBlock.
    pub fn skip_reason(&self) -> Option<ScriptFilterSkipReason> {
        if self.script.already_started {
            return Some(ScriptFilterSkipReason::AlreadyStarted);
        }
        if self.script.has_nomodule && self.kind() == ScriptKind::Classic {
            return Some(ScriptFilterSkipReason::NoModule);
        }
        if self.legacy_event_for_mismatch {
            return Some(ScriptFilterSkipReason::LegacyEventForMismatch);
        }
        match self.kind() {
            ScriptKind::DataBlock => Some(ScriptFilterSkipReason::DataBlock),
            ScriptKind::Classic | ScriptKind::Module | ScriptKind::ImportMap => None,
        }
    }
}

/// Reads a script node via `ParserPlanningReadView` and classifies its kind, mode,
/// and source kind. Returns `None` if the node is not a script element.
pub fn classify_parser_script(
    document: &impl ParserPlanningReadView,
    node_id: NativeNodeId,
) -> Option<ScriptClassification> {
    let script = document.parser_script_read(node_id)?;
    let source_kind = ScriptSourceKind::from_script_src(&script.script_src);
    let classification = classify_script_preparation(ScriptPreparationClassificationInput {
        element: ScriptElementClassificationInput {
            script_type: script.script_type.as_deref(),
            language: script.script_language.as_deref(),
            event: script.script_event.as_deref(),
            for_attribute: script.script_for.as_deref(),
        },
        parser_inserted: script.parser_inserted,
        allow_parser_blocking_modes: true,
        force_async: script.script_async && !script.async_attribute_present,
        async_attribute_present: script.async_attribute_present,
        defer_attribute_present: script.defer_attribute_present,
        source_kind,
    });
    Some(ScriptClassification {
        script,
        disposition: classification.disposition,
        source_kind,
        legacy_event_for_mismatch: classification.legacy_event_for_mismatch,
    })
}

/// Outcome of building a `PreparedScript` from a classified script node.
pub enum PrepareScriptOutcome {
    /// Script is ready to prepare.
    Prepared(Box<PreparedScript>),
    /// External script src URL could not be resolved.
    UrlResolutionFailed(String),
    /// External script src was present but empty, so it is not fetchable.
    EmptyExternalSource(String),
    /// Inline script had no text content or was empty.
    EmptyInlineSource,
    /// Import maps and data blocks do not produce executable script payloads.
    NonExecutableKind(ScriptKind),
}

/// Builds a `PreparedScript` from an already-classified script node.
///
/// `position` is caller-supplied because different contexts compute document
/// order differently (parser uses `document_order_position`, scheduler uses
/// enumeration index, written-document uses a separate hash map).
pub fn build_prepared_script(
    classification: &ScriptClassification,
    document_url: Url,
    document_base_url: Url,
    node_id: NativeNodeId,
    position: usize,
) -> PrepareScriptOutcome {
    let Some((kind, mode)) = classification.executable() else {
        return PrepareScriptOutcome::NonExecutableKind(classification.kind());
    };
    if let Some(src) = classification.script.script_src.as_deref() {
        if src.trim().is_empty() {
            return PrepareScriptOutcome::EmptyExternalSource(
                "empty script src is not fetchable".to_owned(),
            );
        }
        match document_base_url.join(src) {
            Ok(url) => PrepareScriptOutcome::Prepared(Box::new(PreparedScript {
                position,
                node_id: NodeId::new(node_id.index()),
                kind,
                mode,
                source_kind: classification.source_kind,
                fetch_metadata: classification.script.fetch_metadata.clone(),
                source: ScriptSource::External,
                base_url: url.clone(),
                url,
                initiator_url: document_url,
                host_script_handle: None,
            })),
            Err(error) => PrepareScriptOutcome::UrlResolutionFailed(format!(
                "failed to resolve script src `{src}`: {error}"
            )),
        }
    } else {
        match classification.script.script_text.clone() {
            Some(source) if !source.is_empty() => {
                PrepareScriptOutcome::Prepared(Box::new(PreparedScript {
                    position,
                    node_id: NodeId::new(node_id.index()),
                    kind,
                    mode,
                    source_kind: classification.source_kind,
                    fetch_metadata: classification.script.fetch_metadata.clone(),
                    source: ScriptSource::Inline(source),
                    url: document_url.clone(),
                    base_url: document_base_url,
                    initiator_url: document_url,
                    host_script_handle: None,
                }))
            }
            _ => PrepareScriptOutcome::EmptyInlineSource,
        }
    }
}

pub fn build_prepared_import_map(
    classification: &ScriptClassification,
    document_url: Url,
    document_base_url: Url,
    node_id: NativeNodeId,
    position: usize,
) -> Option<PreparedImportMap> {
    if classification.disposition != ScriptPreparationDisposition::ImportMap {
        return None;
    }
    let source = if classification.script.script_src.is_some() {
        PreparedImportMapSource::ExternalUnsupported
    } else {
        PreparedImportMapSource::Inline(
            classification
                .script
                .script_text
                .clone()
                .unwrap_or_default(),
        )
    };
    Some(PreparedImportMap {
        position,
        node_id: NodeId::new(node_id.index()),
        source,
        base_url: document_base_url,
        initiator_url: document_url,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ParserPlanningReadView, PrepareScriptOutcome, ScriptFetchMetadata, ScriptFilterSkipReason,
        build_prepared_script, classify_parser_script,
    };
    use crate::HtmlParser;
    use moli_dom::native::DomHost;
    use moli_page_types::{ScriptKind, ScriptMode, ScriptSourceKind};
    use url::Url;

    #[test]
    fn script_fetch_metadata_ignores_invalid_referrer_policy() {
        let metadata = ScriptFetchMetadata::from_script_attributes(
            None,
            Some(" always "),
            None,
            None,
            None,
            None,
        );

        assert_eq!(metadata.referrer_policy, None);
    }

    #[test]
    fn parser_script_classification_uses_language_when_type_is_absent() {
        let document = HtmlParser.parse(
            Url::parse("https://example.test/").unwrap(),
            "<!doctype html><script language='javascript'>ok()</script><script language=' javascript '>bad()</script>".to_owned(),
        );
        let handles = document.script_handles();

        let first =
            classify_parser_script(&document, handles[0]).expect("first script should classify");
        assert_eq!(first.kind(), ScriptKind::Classic);
        assert_eq!(first.skip_reason(), None);

        let second =
            classify_parser_script(&document, handles[1]).expect("second script should classify");
        assert_eq!(second.kind(), ScriptKind::DataBlock);
        assert!(matches!(
            second.skip_reason(),
            Some(ScriptFilterSkipReason::DataBlock)
        ));
    }

    #[test]
    fn parser_script_classification_applies_legacy_for_event_gate() {
        let document = HtmlParser.parse(
            Url::parse("https://example.test/").unwrap(),
            "<!doctype html><script for=' window ' event=' onload() '>ok()</script><script for='window' event='onclick'>bad()</script>".to_owned(),
        );
        let handles = document.script_handles();

        let matching =
            classify_parser_script(&document, handles[0]).expect("matching script should classify");
        assert_eq!(matching.kind(), ScriptKind::Classic);
        assert_eq!(matching.skip_reason(), None);

        let mismatching = classify_parser_script(&document, handles[1])
            .expect("mismatching script should classify");
        assert!(matches!(
            mismatching.skip_reason(),
            Some(ScriptFilterSkipReason::LegacyEventForMismatch)
        ));
    }

    #[test]
    fn parser_script_classification_applies_nomodule_only_to_classic_scripts() {
        let document = HtmlParser.parse(
            Url::parse("https://example.test/").unwrap(),
            "<!doctype html><script nomodule>legacy()</script><script nomodule type='module'>modern()</script>"
                .to_owned(),
        );
        let handles = document.script_handles();

        let classic =
            classify_parser_script(&document, handles[0]).expect("classic script should classify");
        assert_eq!(classic.kind(), ScriptKind::Classic);
        assert!(matches!(
            classic.skip_reason(),
            Some(ScriptFilterSkipReason::NoModule)
        ));

        let module =
            classify_parser_script(&document, handles[1]).expect("module script should classify");
        assert_eq!(module.kind(), ScriptKind::Module);
        assert_eq!(module.skip_reason(), None);
    }

    #[test]
    fn parser_svg_script_ignores_html_only_classification_and_scheduling_attributes() {
        let document = HtmlParser.parse(
            Url::parse("https://example.test/").unwrap(),
            "<!doctype html><svg><script nomodule defer language='application/json' for='not-window' event='onclick' href='/svg.js'></script></svg>"
                .to_owned(),
        );
        let handle = document.script_handles()[0];

        let classification =
            classify_parser_script(&document, handle).expect("SVG script should classify");

        assert_eq!(classification.kind(), ScriptKind::Classic);
        assert_eq!(classification.mode(), Some(ScriptMode::Normal));
        assert_eq!(classification.skip_reason(), None);
        assert!(!classification.script.has_nomodule);
        assert!(!classification.script.defer_attribute_present);
        assert_eq!(classification.script.script_language, None);
        assert_eq!(classification.script.script_event, None);
        assert_eq!(classification.script.script_for, None);
    }

    #[test]
    fn empty_external_script_src_is_not_resolved_to_document_url() {
        let final_url = Url::parse("https://example.test/page.html").unwrap();
        let document = HtmlParser.parse(
            final_url.clone(),
            "<!doctype html><script src=\"\"></script>".to_owned(),
        );
        let handle = document.script_handles()[0];
        let classification =
            classify_parser_script(&document, handle).expect("script should classify");

        assert_eq!(classification.source_kind, ScriptSourceKind::External);
        match build_prepared_script(&classification, final_url.clone(), final_url, handle, 0) {
            PrepareScriptOutcome::EmptyExternalSource(message) => {
                assert!(message.contains("empty script src"));
            }
            PrepareScriptOutcome::Prepared(script) => {
                panic!("empty src must not prepare a fetch for {}", script.url);
            }
            _ => panic!("empty external src should be reported distinctly"),
        }
    }

    #[test]
    fn parser_script_source_uses_direct_text_children_only() {
        let document = HtmlParser.parse(
            Url::parse("https://example.test/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut host = DomHost::from_dom(document.clone());
        let script = host.create_element("script");
        let span = host.create_element("span");
        let descendant_text = host.create_text_node("window.descendant = true;");
        let direct_text = host.create_text_node("window.direct = true;");
        assert!(host.append_child(span, descendant_text));
        assert!(host.append_child(script, span));
        assert!(host.append_child(script, direct_text));

        let classification = classify_parser_script(&host, script).expect("script should classify");

        assert_eq!(
            classification.script.script_text.as_deref(),
            Some("window.direct = true;")
        );
    }

    #[test]
    fn external_script_src_resolves_against_document_base_url() {
        let final_url = Url::parse("https://example.test/fetch-src/alpha/base.html").unwrap();
        let document = HtmlParser.parse(
            final_url.clone(),
            "<!doctype html><base href=\"../beta/\"><script src=\"test.js\"></script>".to_owned(),
        );
        let handle = document.script_handles()[0];
        let classification =
            classify_parser_script(&document, handle).expect("script should classify");
        let document_base_url = document
            .document_base_url_clone()
            .expect("parsed document should expose base URL");

        match build_prepared_script(
            &classification,
            final_url.clone(),
            document_base_url,
            handle,
            0,
        ) {
            PrepareScriptOutcome::Prepared(script) => {
                assert_eq!(
                    script.url.as_str(),
                    "https://example.test/fetch-src/beta/test.js"
                );
                assert_eq!(script.initiator_url, final_url);
                assert!(
                    script.fetch_metadata.parser_inserted,
                    "parser metadata must be captured when the script element is prepared"
                );
            }
            PrepareScriptOutcome::UrlResolutionFailed(error) => {
                panic!("script URL should resolve against document base URL: {error}");
            }
            _ => panic!("external src should prepare a fetch"),
        }
    }
}
